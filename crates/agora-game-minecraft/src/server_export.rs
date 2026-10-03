use crate::gc::{generate_args, GcProfile};
use agora_core::error::{LauncherError, LauncherResult};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub total_mods: usize,
    pub server_mods: usize,
    pub removed_client_only: Vec<String>,
    pub server_jar_downloaded: bool,
    pub start_scripts_created: bool,
}

pub fn export_server_environment(
    instance_dir: &Path,
    dest_dir: &Path,
    _loader: &str,
    _mc_version: &str,
) -> LauncherResult<ExportResult> {
    let mods_dir = instance_dir.join("mods");
    let dest_mods = dest_dir.join("mods");
    std::fs::create_dir_all(&dest_mods).map_err(|e| LauncherError::Generic {
        code: "ERR_EXPORT_MKDIR".into(),
        message: format!("Failed to create dest mods dir: {e}"),
    })?;

    let mut total_mods = 0;
    let mut server_mods = 0;
    let mut removed_client_only = Vec::new();

    if mods_dir.is_dir() {
        for entry in std::fs::read_dir(&mods_dir).map_err(|e| LauncherError::Generic {
            code: "ERR_EXPORT_READDIR".into(),
            message: format!("Failed to read mods dir: {e}"),
        })? {
            let entry = entry.map_err(|e| LauncherError::Generic {
                code: "ERR_EXPORT_ENTRY".into(),
                message: format!("Failed to read entry: {e}"),
            })?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jar") {
                continue;
            }
            total_mods += 1;

            let _meta = crate::jar_metadata::parse_jar_metadata(&path);

            if is_client_only(&path) {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    removed_client_only.push(name.to_string());
                }
                continue;
            }

            let fname = path.file_name().ok_or_else(|| LauncherError::Generic {
                code: "ERR_EXPORT_FILENAME".into(),
                message: "Missing file name".into(),
            })?;
            std::fs::copy(&path, dest_mods.join(fname)).map_err(|e| LauncherError::Generic {
                code: "ERR_EXPORT_COPY".into(),
                message: format!("Failed to copy mod: {e}"),
            })?;
            server_mods += 1;
        }
    }

    let config_src = instance_dir.join("config");
    if config_src.is_dir() {
        copy_dir_recursive(&config_src, &dest_dir.join("config"))?;
    }

    let world_src = instance_dir.join("world");
    if world_src.is_dir() {
        copy_dir_recursive(&world_src, &dest_dir.join("world"))?;
    } else {
        let saves = instance_dir.join("saves");
        if saves.is_dir() {
            if let Some(first) = std::fs::read_dir(&saves)
                .map_err(|e| LauncherError::Generic {
                    code: "ERR_EXPORT_READDIR".into(),
                    message: format!("Failed to read saves: {e}"),
                })?
                .filter_map(|e| e.ok())
                .find(|e| e.path().is_dir())
            {
                let name = first.file_name();
                copy_dir_recursive(&saves.join(&name), &dest_dir.join(&name))?;
            }
        }
    }

    Ok(ExportResult {
        total_mods,
        server_mods,
        removed_client_only,
        server_jar_downloaded: false,
        start_scripts_created: false,
    })
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> LauncherResult<()> {
    std::fs::create_dir_all(dst).map_err(|e| LauncherError::Generic {
        code: "ERR_EXPORT_MKDIR".into(),
        message: format!("mkdir {dst:?}: {e}"),
    })?;
    for entry in std::fs::read_dir(src).map_err(|e| LauncherError::Generic {
        code: "ERR_EXPORT_READDIR".into(),
        message: format!("readdir {src:?}: {e}"),
    })? {
        let entry = entry.map_err(|e| LauncherError::Generic {
            code: "ERR_EXPORT_ENTRY".into(),
            message: format!("entry: {e}"),
        })?;
        let ty = entry.file_type().map_err(|e| LauncherError::Generic {
            code: "ERR_EXPORT_FTYPE".into(),
            message: format!("ftype: {e}"),
        })?;
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name())).map_err(|e| {
                LauncherError::Generic {
                    code: "ERR_EXPORT_COPY".into(),
                    message: format!("copy {}: {e}", entry.path().display()),
                }
            })?;
        }
    }
    Ok(())
}

fn is_client_only(jar_path: &Path) -> bool {
    let file = match std::fs::File::open(jar_path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => return false,
    };

    for i in 0..archive.len() {
        let name = match archive.by_index(i) {
            Ok(e) => e.name().to_string(),
            Err(_) => continue,
        };
        if name == "fabric.mod.json" {
            if let Some(content) = read_entry_utf8(&mut archive, i) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                    return matches!(
                        value.get("environment").and_then(|v| v.as_str()),
                        Some("client")
                    );
                }
            }
            return false;
        }
        if name == "META-INF/mods.toml" {
            if let Some(content) = read_entry_utf8(&mut archive, i) {
                for line in content.lines() {
                    let t = line.trim();
                    if let Some(rest) = t.strip_prefix("side") {
                        let rest = rest.trim_start();
                        if let Some(stripped) = rest.strip_prefix('=') {
                            let val = stripped.trim().trim_matches('"').trim_matches('\'');
                            if val.eq_ignore_ascii_case("CLIENT") {
                                return true;
                            }
                        }
                    }
                }
            }
            return false;
        }
    }

    false
}

fn read_entry_utf8(archive: &mut zip::ZipArchive<std::fs::File>, index: usize) -> Option<String> {
    let mut file = archive.by_index(index).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

pub fn generate_start_script(loader: &str, mc_version: &str, heap_mb: i64) -> (String, String) {
    let gc_args = generate_args(GcProfile::HighEfficiency, heap_mb, "");

    let _ = mc_version;
    let jar_file = match loader.to_lowercase().as_str() {
        "fabric" => "fabric-server-launch.jar",
        "quilt" => "quilt-server-launch.jar",
        "forge" => "forge-server.jar",
        "neoforge" => "neoforge-server.jar",
        _ => "server.jar",
    };

    let cmd = format!("java {gc_args} -jar {jar_file} nogui");

    let sh = format!("#!/bin/bash\n{cmd}\n");
    let bat = format!("@echo off\n{cmd}\n");

    (sh, bat)
}

pub fn download_server_loader(
    _client: &reqwest::Client,
    dest_dir: &Path,
    loader: &str,
    mc_version: &str,
) -> LauncherResult<PathBuf> {
    let url: String = match loader.to_lowercase().as_str() {
        "fabric" => format!("https://meta.fabricmc.net/v2/versions/loader/{mc_version}/server/jar"),
        "quilt" => format!("https://meta.quiltmc.org/v3/versions/loader/{mc_version}/server/jar"),
        "forge" | "neoforge" => {
            let entries = crate::loader_manifests::list_versions(loader, mc_version);
            let latest = entries.last().ok_or_else(|| LauncherError::Generic {
                code: "ERR_EXPORT_NO_PINNED".into(),
                message: format!("No pinned {loader} version for MC {mc_version}"),
            })?;
            latest.source_url.clone()
        }
        _ => {
            return Err(LauncherError::Generic {
                code: "ERR_EXPORT_UNKNOWN_LOADER".into(),
                message: format!("Unknown loader: {loader}"),
            })
        }
    };

    let jar_name = match loader.to_lowercase().as_str() {
        "fabric" => "fabric-server-launch.jar",
        "quilt" => "quilt-server-launch.jar",
        "forge" => "forge-server.jar",
        "neoforge" => "neoforge-server.jar",
        _ => "server.jar",
    };

    let dest_path = dest_dir.join(jar_name);

    let clients = agora_core::http_client::HttpClients::new()?;
    let bytes = agora_core::http_client::blocking_checked_get_bytes(
        &clients,
        agora_core::http_client::ClientCategory::Loader,
        &url,
    )?;

    std::fs::write(&dest_path, &bytes).map_err(|e| LauncherError::Generic {
        code: "ERR_EXPORT_WRITE".into(),
        message: format!("Failed to write {dest_path:?}: {e}"),
    })?;

    Ok(dest_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn export_with_empty_instance() {
        let tmp = TempDir::new().unwrap();
        let instance = tmp.path().join("instance");
        let dest = tmp.path().join("server");
        std::fs::create_dir_all(instance.join("mods")).unwrap();

        let result = export_server_environment(&instance, &dest, "fabric", "1.21").unwrap();
        assert_eq!(result.total_mods, 0);
        assert_eq!(result.server_mods, 0);
        assert!(result.removed_client_only.is_empty());
    }

    #[test]
    fn export_with_fake_jars() {
        let tmp = TempDir::new().unwrap();
        let instance = tmp.path().join("instance");
        let dest = tmp.path().join("server");
        std::fs::create_dir_all(instance.join("mods")).unwrap();

        std::fs::write(instance.join("mods").join("example.jar"), b"not a real zip").unwrap();
        std::fs::write(instance.join("mods").join("other.jar"), b"also fake").unwrap();

        let result = export_server_environment(&instance, &dest, "fabric", "1.21").unwrap();
        assert_eq!(result.total_mods, 2);
        assert_eq!(result.server_mods, 2);
        assert!(result.removed_client_only.is_empty());
        assert!(dest.join("mods").join("example.jar").exists());
        assert!(dest.join("mods").join("other.jar").exists());
    }

    #[test]
    fn start_scripts_non_empty_for_all_loaders() {
        for loader in &["fabric", "quilt", "forge", "neoforge"] {
            let (sh, bat) = generate_start_script(loader, "1.21", 4096);
            assert!(!sh.is_empty(), "{loader} sh is empty");
            assert!(!bat.is_empty(), "{loader} bat is empty");
            assert!(sh.contains("java "), "{loader} sh missing java");
            assert!(bat.contains("java "), "{loader} bat missing java");
        }
    }

    #[test]
    fn start_scripts_include_gc_args() {
        let (sh, _bat) = generate_start_script("fabric", "1.21", 4096);
        assert!(sh.contains("-Xmx"), "should have -Xmx");
        assert!(sh.contains("-Xms"), "should have -Xms");
        assert!(sh.contains("-XX:"), "should have GC flags");
        assert!(
            sh.contains("fabric-server-launch.jar"),
            "should have correct jar"
        );
    }

    #[test]
    fn start_scripts_loader_specific_jar() {
        let (sh, _) = generate_start_script("forge", "1.20.1", 4096);
        assert!(sh.contains("forge-server.jar"));
        let (sh, _) = generate_start_script("neoforge", "1.21", 4096);
        assert!(sh.contains("neoforge-server.jar"));
        let (sh, _) = generate_start_script("quilt", "1.20.1", 4096);
        assert!(sh.contains("quilt-server-launch.jar"));
    }
}
