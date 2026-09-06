//! Inventory of files contributed by an instance's pack.
//!
//! Stored durably in `instance_pack_files` (full per-file list) while only
//! [`crate::models::PackOrigin::pack_content_hash`] lives in the manifest so
//! manifests stay small yet drift remains detectable if the DB is lost.
//!
//! Scope is intentionally narrow: only paths a pack could have contributed are
//! inventoried, not the whole instance. Motivation in `import.rs:75`
//! `ALLOWED_OVERRIDE_PREFIXES` plus `mods/` and `options.txt`.
//!
//! Hashing is order-independent (sort by `relative_path` before hashing) and
//! changes if any file's path or content hash changes.

use crate::db::InstancePackFile;
use sha2::Digest;
use std::path::Path;

/// Roots that a pack may have contributed.
///
/// `mods/` plus the six override prefixes from `import::ALLOWED_OVERRIDE_PREFIXES`
/// plus the single root file `options.txt`.
const PACK_ROOT_FILE: &str = "options.txt";

/// Collect the current pack-contributable inventory from `instance_dir`.
///
/// Walks each allowed root, hashes every regular file found (skipping
/// symlinks/reparse points via [`crate::helpers::unsafe_filesystem_entry`]),
/// and returns the list sorted by `relative_path`. Errors only if the instance
/// directory itself is not a directory; per-file read/hash failures are skipped
/// (a file that cannot be read cannot be part of a stable content hash).
pub fn collect_pack_inventory(instance_dir: &Path) -> Result<Vec<InstancePackFile>, String> {
    if !instance_dir.is_dir() {
        return Err(format!(
            "Instance directory is not a directory: {}",
            instance_dir.display()
        ));
    }
    let mut files = Vec::new();

    // mods/
    collect_under(instance_dir, "mods", &mut files);

    // override prefixes from import.rs
    for prefix in crate::import::ALLOWED_OVERRIDE_PREFIXES {
        let dir = prefix.trim_end_matches('/');
        if dir.is_empty() {
            continue;
        }
        collect_under(instance_dir, dir, &mut files);
    }

    // single file at root
    let options_path = instance_dir.join(PACK_ROOT_FILE);
    if let Ok(meta) = std::fs::symlink_metadata(&options_path) {
        if meta.is_file()
            && !crate::helpers::unsafe_filesystem_entry(&options_path, &meta.file_type())
        {
            if let Ok(sha256) = crate::helpers::hash_file_sha256(&options_path) {
                files.push(InstancePackFile {
                    relative_path: PACK_ROOT_FILE.to_string(),
                    sha256,
                    size: meta.len(),
                });
            }
        }
    }

    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

/// Stable, order-independent hash over the sorted inventory.
///
/// Feeds each entry's `relative_path` and `sha256` (and `size` for extra
/// collision resistance) into a single SHA-256. Sorted before hashing so
/// directory traversal order does not affect the result.
pub fn pack_content_hash(files: &[InstancePackFile]) -> String {
    let mut sorted = files.to_vec();
    sorted.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let mut hasher = sha2::Sha256::new();
    for file in &sorted {
        hasher.update(file.relative_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(file.sha256.as_bytes());
        hasher.update(b"\0");
        hasher.update(file.size.to_le_bytes());
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

fn collect_under(instance_dir: &Path, subdir: &str, out: &mut Vec<InstancePackFile>) {
    let root = instance_dir.join(subdir);
    if !root.is_dir() {
        return;
    }
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            let path = entry.path();
            if crate::helpers::unsafe_filesystem_entry(&path, &ft) {
                continue;
            }
            // We don't apply is_excluded_source_name here: pack-contributable
            // roots are narrow (mods/, config/...) and .agora internals are not
            // under them. Skipping launcher excluded names would be misleading
            // for pack drift detection (a pack could legitimately contribute a
            // file named `profile.json` under `config/`? Not currently, but we
            // don't want to silently drop it).
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let relative_path = path
                    .strip_prefix(instance_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let sha256 = match crate::helpers::hash_file_sha256(&path) {
                    Ok(h) => h,
                    Err(_) => continue,
                };
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                out.push(InstancePackFile {
                    relative_path,
                    sha256,
                    size,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel: &str, contents: &[u8]) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, contents).unwrap();
    }

    fn temp_instance() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn collect_empty_when_no_pack_roots() {
        let tmp = temp_instance();
        let inst = tmp.path().join("inst");
        fs::create_dir_all(&inst).unwrap();
        // create a file outside pack scope, should be ignored
        write_file(&inst, "saves/world/level.dat", b"data");
        write_file(&inst, "logs/latest.log", b"log");
        let inv = collect_pack_inventory(&inst).unwrap();
        assert!(
            inv.is_empty(),
            "outside-scope files must be ignored: {inv:?}"
        );
    }

    #[test]
    fn collect_picks_mods_and_overrides_and_options() {
        let tmp = temp_instance();
        let inst = tmp.path().join("inst");
        fs::create_dir_all(&inst).unwrap();
        write_file(&inst, "mods/a.jar", b"a");
        write_file(&inst, "config/foo/bar.toml", b"cfg");
        write_file(&inst, "kubejs/server_scripts/x.js", b"kube");
        write_file(&inst, "options.txt", b"options");
        // outside scope
        write_file(&inst, "saves/ignored", b"no");
        let inv = collect_pack_inventory(&inst).unwrap();
        let paths: Vec<&str> = inv.iter().map(|f| f.relative_path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "config/foo/bar.toml",
                "kubejs/server_scripts/x.js",
                "mods/a.jar",
                "options.txt"
            ]
        );
        // ensure hashes present
        for f in &inv {
            assert_eq!(f.sha256.len(), 64);
        }
    }

    #[test]
    fn hash_is_order_independent() {
        let a = InstancePackFile {
            relative_path: "mods/a.jar".into(),
            sha256: "a".repeat(64),
            size: 1,
        };
        let b = InstancePackFile {
            relative_path: "config/b.toml".into(),
            sha256: "b".repeat(64),
            size: 2,
        };
        let h1 = pack_content_hash(&[a.clone(), b.clone()]);
        let h2 = pack_content_hash(&[b, a]);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_changes_when_content_changes() {
        let base = vec![InstancePackFile {
            relative_path: "mods/a.jar".into(),
            sha256: "a".repeat(64),
            size: 1,
        }];
        let h1 = pack_content_hash(&base);
        let mut mutated = base.clone();
        mutated[0].sha256 = "b".repeat(64);
        let h2 = pack_content_hash(&mutated);
        assert_ne!(h1, h2);
        // path change also changes hash
        let mut mutated2 = base;
        mutated2[0].relative_path = "mods/b.jar".into();
        let h3 = pack_content_hash(&mutated2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn hash_changes_when_file_added_or_removed() {
        let one = vec![InstancePackFile {
            relative_path: "mods/a.jar".into(),
            sha256: "a".repeat(64),
            size: 1,
        }];
        let two = vec![
            InstancePackFile {
                relative_path: "mods/a.jar".into(),
                sha256: "a".repeat(64),
                size: 1,
            },
            InstancePackFile {
                relative_path: "mods/b.jar".into(),
                sha256: "b".repeat(64),
                size: 2,
            },
        ];
        assert_ne!(pack_content_hash(&one), pack_content_hash(&two));
        assert_ne!(pack_content_hash(&[]), pack_content_hash(&one));
    }

    #[test]
    fn collect_captures_all_six_override_prefixes() {
        let tmp = temp_instance();
        let inst = tmp.path().join("inst");
        fs::create_dir_all(&inst).unwrap();
        for prefix in crate::import::ALLOWED_OVERRIDE_PREFIXES {
            let dir = prefix.trim_end_matches('/');
            write_file(&inst, &format!("{dir}/file.txt"), b"data");
        }
        let inv = collect_pack_inventory(&inst).unwrap();
        // mods/ not created, so only 6 override dirs
        assert_eq!(inv.len(), 6);
        for file in &inv {
            assert!(crate::import::ALLOWED_OVERRIDE_PREFIXES
                .iter()
                .any(|p| file.relative_path.starts_with(p)));
        }
    }

    #[test]
    fn collect_skips_symlinks() {
        let tmp = temp_instance();
        let inst = tmp.path().join("inst");
        fs::create_dir_all(inst.join("mods")).unwrap();
        write_file(&inst, "mods/real.jar", b"real");
        // Try to create a symlink; if platform denies, skip test.
        #[cfg(unix)]
        {
            let link = inst.join("mods/link.jar");
            let target = inst.join("mods/real.jar");
            if std::os::unix::fs::symlink(&target, &link).is_ok() {
                let inv = collect_pack_inventory(&inst).unwrap();
                let paths: Vec<&str> = inv.iter().map(|f| f.relative_path.as_str()).collect();
                assert_eq!(paths, vec!["mods/real.jar"], "symlink must be skipped");
            }
        }
        #[cfg(windows)]
        {
            // Windows symlink often requires privilege; just ensure collect doesn't panic.
            let inv = collect_pack_inventory(&inst).unwrap();
            assert!(inv.iter().any(|f| f.relative_path == "mods/real.jar"));
        }
    }
}
