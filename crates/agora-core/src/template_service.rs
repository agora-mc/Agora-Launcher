//! Instance templates — reusable starting points for new instances.
//!
//! A template captures two independent things:
//!
//! * **JVM settings** (`TemplateJvm`) — memory, GC, custom args, Java path.
//! * **Config files** (`files/`) — `options.txt` and any mod config the user
//!   explicitly selected.
//!
//! Either half may be empty, which is what makes a single store serve two user-
//! facing features: a template with only JVM settings is a *named Java profile*,
//! a template with only files is a *config preset*, and one with both is a full
//! instance template. Keeping them in one store means "apply this" is a single
//! code path regardless of which the user thinks they are using.
//!
//! # Storage
//!
//! Templates live on the filesystem rather than in `local_state.db`:
//!
//! ```text
//! <app_data>/templates/<template_id>/template.json
//! <app_data>/templates/<template_id>/files/config/sodium-options.json
//! ```
//!
//! The payload is file *content*, which does not belong in a row, and a plain
//! directory is portable — a user can back it up, sync it, or hand it to a
//! friend without an export step. This also avoids a schema migration.
//!
//! # Safety
//!
//! Every relative path is validated on both capture and apply: only `Normal`
//! components, only under a known capturable root, no symlinks, and bounded in
//! size. A template directory is untrusted input on apply because the user may
//! have edited or received it out of band.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::helpers;

/// On-disk format version for `template.json`.
pub const TEMPLATE_SCHEMA_VERSION: u32 = 1;

/// Largest single file a template may capture (8 MiB).
///
/// Mod configs are text; anything larger is a data file that was never meant to
/// be a reusable starting point.
pub const MAX_TEMPLATE_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Largest total payload for one template (64 MiB).
pub const MAX_TEMPLATE_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// Directory roots whose contents may be captured into a template.
///
/// Deliberately excludes `mods/`, `saves/`, `resourcepacks/` and friends: a
/// template is *configuration*, not content. Content comes from the pack or the
/// user's explicit installs, and copying jars into every new instance would
/// silently duplicate gigabytes.
const CAPTURABLE_DIRS: &[&str] = &["config", "defaultconfigs", "kubejs"];

/// Individual files at the instance root that may be captured.
const CAPTURABLE_ROOT_FILES: &[&str] = &[
    "options.txt",
    "optionsof.txt",
    "optionsshaders.txt",
    "servers.dat",
];

/// JVM settings carried by a template.
///
/// Every field is optional and `None` means "leave the instance's own value
/// alone", so a Java profile that only pins heap size does not clobber a custom
/// GC choice the user made per instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateJvm {
    #[serde(default)]
    pub java_path: Option<String>,
    #[serde(default)]
    pub jvm_memory_mb: Option<i64>,
    #[serde(default)]
    pub jvm_memory_mode: Option<String>,
    #[serde(default)]
    pub jvm_gc: Option<String>,
    #[serde(default)]
    pub jvm_custom_args: Option<String>,
    #[serde(default)]
    pub jvm_always_pre_touch: Option<bool>,
}

impl TemplateJvm {
    /// Whether this template carries any JVM setting at all.
    pub fn is_empty(&self) -> bool {
        *self == TemplateJvm::default()
    }
}

/// One captured file recorded in `template.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateFile {
    /// Forward-slash relative path, rooted at the instance directory.
    pub relative_path: String,
    pub sha256: String,
    pub size: u64,
}

/// A stored template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceTemplate {
    #[serde(default = "default_template_version")]
    pub template_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub jvm: TemplateJvm,
    #[serde(default)]
    pub files: Vec<TemplateFile>,
}

fn default_template_version() -> u32 {
    1
}

impl InstanceTemplate {
    /// Total captured payload in bytes.
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// A file offered to the user in the "which configs should this template keep?"
/// picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapturableFile {
    pub relative_path: String,
    pub size: u64,
    /// Top-level bucket the file belongs to (`options`, `config`, `kubejs`, …)
    /// so the UI can group without re-parsing paths.
    pub category: String,
    /// Whether the file exceeds [`MAX_TEMPLATE_FILE_BYTES`]. Oversized files are
    /// still listed — silently omitting a config the user is looking for reads
    /// as a bug — but they cannot be selected.
    pub too_large: bool,
}

/// Root directory holding every template.
pub fn templates_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("templates")
}

fn template_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    let sanitized = crate::paths::sanitize_id(id);
    if sanitized.is_empty() || sanitized != id {
        return Err(format!("Invalid template id '{id}'."));
    }
    Ok(root.join(sanitized))
}

/// Validate a capture-relative path and return its category.
///
/// Fails closed: anything that is not a plain relative path under a known
/// capturable root is rejected, whether it came from the frontend or from a
/// hand-edited `template.json`.
fn classify_relative_path(relative_path: &str) -> Result<String, String> {
    if relative_path.is_empty() {
        return Err("Empty template path.".to_string());
    }
    if relative_path.contains('\\') {
        return Err(format!(
            "Template path '{relative_path}' must use forward slashes."
        ));
    }
    let candidate = Path::new(relative_path);
    let mut components = Vec::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => {
                let text = part
                    .to_str()
                    .ok_or_else(|| format!("Template path '{relative_path}' is not UTF-8."))?;
                components.push(text.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("Unsafe template path '{relative_path}'."));
            }
        }
    }
    let Some(first) = components.first() else {
        return Err(format!("Template path '{relative_path}' names nothing."));
    };
    if components.len() == 1 {
        if CAPTURABLE_ROOT_FILES.contains(&first.as_str()) {
            return Ok(root_file_category(first));
        }
        return Err(format!(
            "'{relative_path}' is not a file a template may capture."
        ));
    }
    if CAPTURABLE_DIRS.contains(&first.as_str()) {
        return Ok(first.clone());
    }
    Err(format!(
        "'{relative_path}' is not inside a directory a template may capture."
    ))
}

fn root_file_category(name: &str) -> String {
    if name == "servers.dat" {
        "servers".to_string()
    } else {
        "options".to_string()
    }
}

/// Join a validated relative path onto a base directory.
fn join_validated(base: &Path, relative_path: &str) -> Result<PathBuf, String> {
    classify_relative_path(relative_path)?;
    let mut out = base.to_path_buf();
    for part in relative_path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        out.push(part);
    }
    Ok(out)
}

/// Every file in `instance_dir` a template is allowed to capture.
///
/// Sorted by path so the picker is stable between calls.
pub fn list_capturable_files(instance_dir: &Path) -> Result<Vec<CapturableFile>, String> {
    let mut out: Vec<CapturableFile> = Vec::new();

    for name in CAPTURABLE_ROOT_FILES {
        let path = instance_dir.join(name);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if helpers::unsafe_filesystem_entry(&path, &metadata.file_type()) || !metadata.is_file() {
            continue;
        }
        out.push(CapturableFile {
            relative_path: (*name).to_string(),
            size: metadata.len(),
            category: root_file_category(name),
            too_large: metadata.len() > MAX_TEMPLATE_FILE_BYTES,
        });
    }

    for dir in CAPTURABLE_DIRS {
        let root = instance_dir.join(dir);
        if !root.is_dir() {
            continue;
        }
        walk_capturable(&root, dir, dir, &mut out)?;
    }

    out.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(out)
}

fn walk_capturable(
    dir: &Path,
    prefix: &str,
    category: &str,
    out: &mut Vec<CapturableFile>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("Cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Cannot read entry in {}: {e}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Cannot stat {}: {e}", path.display()))?;
        if helpers::unsafe_filesystem_entry(&path, &file_type) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let relative_path = format!("{prefix}/{name}");
        if file_type.is_dir() {
            walk_capturable(&path, &relative_path, category, out)?;
        } else if file_type.is_file() {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(CapturableFile {
                relative_path,
                size,
                category: category.to_string(),
                too_large: size > MAX_TEMPLATE_FILE_BYTES,
            });
        }
    }
    Ok(())
}

/// Read every stored template, newest first.
///
/// A single unreadable template directory is skipped rather than failing the
/// whole listing: one corrupt template must not hide the rest.
pub fn list_templates(root: &Path) -> Result<Vec<InstanceTemplate>, String> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(root).map_err(|e| format!("Cannot read templates: {e}"))?;
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let manifest = entry.path().join("template.json");
        let Ok(text) = fs::read_to_string(&manifest) else {
            continue;
        };
        let Ok(template) = serde_json::from_str::<InstanceTemplate>(&text) else {
            continue;
        };
        out.push(template);
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    Ok(out)
}

/// Read a single template by id.
pub fn get_template(root: &Path, id: &str) -> Result<InstanceTemplate, String> {
    let dir = template_dir(root, id)?;
    let text = fs::read_to_string(dir.join("template.json"))
        .map_err(|e| format!("Cannot read template '{id}': {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("Cannot parse template '{id}': {e}"))
}

fn write_template(root: &Path, template: &InstanceTemplate) -> Result<(), String> {
    let dir = template_dir(root, &template.id)?;
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create template dir: {e}"))?;
    let json = serde_json::to_string_pretty(template)
        .map_err(|e| format!("Cannot serialize template: {e}"))?;
    let final_path = dir.join("template.json");
    let tmp = dir.join("template.json.tmp");
    fs::write(&tmp, json.as_bytes()).map_err(|e| format!("Cannot write template: {e}"))?;
    fs::rename(&tmp, &final_path).map_err(|e| format!("Cannot finalize template: {e}"))?;
    Ok(())
}

/// Everything needed to capture one template.
///
/// A struct rather than a parameter list because the pieces are independently
/// optional and a positional call site of `Option`s reads as a puzzle.
pub struct CreateTemplateRequest<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub description: Option<String>,
    pub jvm: TemplateJvm,
    /// May be `None` for a JVM-only template (a Java profile), in which case
    /// `selected_paths` must be empty.
    pub source_instance_dir: Option<&'a Path>,
    pub selected_paths: &'a [String],
    /// RFC 3339 timestamp recorded as both `created_at` and `updated_at`.
    pub now: &'a str,
}

/// Capture a new template.
pub fn create_template(
    root: &Path,
    request: CreateTemplateRequest<'_>,
) -> Result<InstanceTemplate, String> {
    let CreateTemplateRequest {
        id,
        name,
        description,
        jvm,
        source_instance_dir,
        selected_paths,
        now,
    } = request;
    let name = name.trim();
    if name.is_empty() {
        return Err("A template needs a name.".to_string());
    }
    let dir = template_dir(root, id)?;
    if dir.exists() {
        return Err(format!("A template with id '{id}' already exists."));
    }
    if !selected_paths.is_empty() && source_instance_dir.is_none() {
        return Err("Cannot capture files without a source instance.".to_string());
    }

    // De-duplicate, because the picker sends whatever the user checked and a
    // repeated path would be copied (and counted against the budget) twice.
    let mut unique: BTreeMap<String, ()> = BTreeMap::new();
    for path in selected_paths {
        unique.insert(path.clone(), ());
    }

    let files_root = dir.join("files");
    let mut files: Vec<TemplateFile> = Vec::new();
    let mut total: u64 = 0;

    let mut capture = || -> Result<(), String> {
        for relative_path in unique.keys() {
            classify_relative_path(relative_path)?;
            let source_root = source_instance_dir.expect("checked above");
            let src = join_validated(source_root, relative_path)?;
            let metadata = fs::symlink_metadata(&src)
                .map_err(|e| format!("Cannot read '{relative_path}': {e}"))?;
            if helpers::unsafe_filesystem_entry(&src, &metadata.file_type()) {
                return Err(format!("'{relative_path}' is not a regular file."));
            }
            if !metadata.is_file() {
                return Err(format!("'{relative_path}' is not a regular file."));
            }
            if metadata.len() > MAX_TEMPLATE_FILE_BYTES {
                return Err(format!(
                    "'{relative_path}' is too large for a template ({} bytes).",
                    metadata.len()
                ));
            }
            total = total.saturating_add(metadata.len());
            if total > MAX_TEMPLATE_TOTAL_BYTES {
                return Err("The selected files exceed the template size limit.".to_string());
            }
            let dst = join_validated(&files_root, relative_path)?;
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Cannot create '{}': {e}", parent.display()))?;
            }
            fs::copy(&src, &dst).map_err(|e| format!("Cannot copy '{relative_path}': {e}"))?;
            let sha256 = helpers::hash_file_sha256(&dst)
                .map_err(|e| format!("Cannot hash '{relative_path}': {e}"))?;
            files.push(TemplateFile {
                relative_path: relative_path.clone(),
                sha256,
                size: metadata.len(),
            });
        }
        Ok(())
    };

    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create template dir: {e}"))?;
    if let Err(error) = capture() {
        // A half-captured template is worse than none: the user would apply it
        // and get an arbitrary subset of their configs.
        let _ = fs::remove_dir_all(&dir);
        return Err(error);
    }

    let template = InstanceTemplate {
        template_version: TEMPLATE_SCHEMA_VERSION,
        id: id.to_string(),
        name: name.to_string(),
        description: description.filter(|d| !d.trim().is_empty()),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        jvm,
        files,
    };
    if let Err(error) = write_template(root, &template) {
        let _ = fs::remove_dir_all(&dir);
        return Err(error);
    }
    Ok(template)
}

/// Rename a template or change its description / JVM settings in place.
///
/// Captured files are untouched — re-capturing is a separate, explicit action.
pub fn update_template(
    root: &Path,
    id: &str,
    name: Option<&str>,
    description: Option<Option<String>>,
    jvm: Option<TemplateJvm>,
    now: &str,
) -> Result<InstanceTemplate, String> {
    let mut template = get_template(root, id)?;
    if let Some(name) = name {
        let name = name.trim();
        if name.is_empty() {
            return Err("A template needs a name.".to_string());
        }
        template.name = name.to_string();
    }
    if let Some(description) = description {
        template.description = description.filter(|d| !d.trim().is_empty());
    }
    if let Some(jvm) = jvm {
        template.jvm = jvm;
    }
    template.updated_at = now.to_string();
    write_template(root, &template)?;
    Ok(template)
}

/// Delete a template and everything it captured.
pub fn delete_template(root: &Path, id: &str) -> Result<(), String> {
    let dir = template_dir(root, id)?;
    if !dir.exists() {
        return Ok(());
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("Cannot delete template '{id}': {e}"))
}

/// Copy a template's captured files into `dest_dir`, returning how many landed.
///
/// Existing files are overwritten: the caller applies a template to a freshly
/// staged instance, and where it does not (a pack install that already wrote
/// `config/`), the template is the more specific user intent.
///
/// Paths are re-validated here rather than trusted from `template.json`, since
/// a template directory can be edited or copied in from elsewhere.
pub fn apply_template_files(root: &Path, id: &str, dest_dir: &Path) -> Result<usize, String> {
    let template = get_template(root, id)?;
    let files_root = template_dir(root, id)?.join("files");
    let mut applied = 0usize;
    for file in &template.files {
        let src = join_validated(&files_root, &file.relative_path)?;
        let metadata = match fs::symlink_metadata(&src) {
            Ok(metadata) => metadata,
            // A file recorded but missing on disk is a damaged template, not a
            // reason to abort: apply what is there and report the count.
            Err(_) => continue,
        };
        if helpers::unsafe_filesystem_entry(&src, &metadata.file_type()) || !metadata.is_file() {
            continue;
        }
        let dst = join_validated(dest_dir, &file.relative_path)?;
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create '{}': {e}", parent.display()))?;
        }
        fs::copy(&src, &dst).map_err(|e| format!("Cannot apply '{}': {e}", file.relative_path))?;
        applied += 1;
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_instance(dir: &Path) {
        fs::create_dir_all(dir.join("config/sodium")).unwrap();
        fs::create_dir_all(dir.join("mods")).unwrap();
        fs::write(dir.join("options.txt"), b"fov:80").unwrap();
        fs::write(dir.join("config/sodium/opts.json"), b"{}").unwrap();
        fs::write(dir.join("config/top.cfg"), b"x=1").unwrap();
        fs::write(dir.join("mods/a.jar"), b"jar").unwrap();
    }

    #[test]
    fn capturable_listing_covers_configs_and_skips_mods() {
        let tmp = tempfile::tempdir().unwrap();
        seed_instance(tmp.path());
        let files = list_capturable_files(tmp.path()).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["config/sodium/opts.json", "config/top.cfg", "options.txt"]
        );
        assert!(files.iter().all(|f| !f.too_large));
    }

    #[test]
    fn capture_then_apply_round_trips_selected_files_only() {
        let tmp = tempfile::tempdir().unwrap();
        let instance = tmp.path().join("instance");
        fs::create_dir_all(&instance).unwrap();
        seed_instance(&instance);
        let root = tmp.path().join("templates");

        let template = create_template(
            &root,
            CreateTemplateRequest {
                id: "starter",
                name: "Starter",
                description: None,
                jvm: TemplateJvm {
                    jvm_memory_mb: Some(4096),
                    ..TemplateJvm::default()
                },
                source_instance_dir: Some(&instance),
                selected_paths: &["options.txt".into(), "config/sodium/opts.json".into()],
                now: "2026-01-01T00:00:00Z",
            },
        )
        .unwrap();
        assert_eq!(template.files.len(), 2);
        assert_eq!(template.total_bytes(), 6 + 2);

        let dest = tmp.path().join("new-instance");
        fs::create_dir_all(&dest).unwrap();
        let applied = apply_template_files(&root, "starter", &dest).unwrap();
        assert_eq!(applied, 2);
        assert_eq!(
            fs::read_to_string(dest.join("options.txt")).unwrap(),
            "fov:80"
        );
        assert!(dest.join("config/sodium/opts.json").exists());
        // Not selected, so it must not appear.
        assert!(!dest.join("config/top.cfg").exists());
    }

    #[test]
    fn jvm_only_template_needs_no_source_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("templates");
        let template = create_template(
            &root,
            CreateTemplateRequest {
                id: "perf",
                name: "Performance",
                description: Some("  ".into()),
                jvm: TemplateJvm {
                    jvm_gc: Some("zgc".into()),
                    ..TemplateJvm::default()
                },
                source_instance_dir: None,
                selected_paths: &[],
                now: "2026-01-01T00:00:00Z",
            },
        )
        .unwrap();
        assert!(template.files.is_empty());
        // Whitespace-only descriptions normalize away rather than rendering as
        // an empty line in the UI.
        assert!(template.description.is_none());
        assert!(!template.jvm.is_empty());
    }

    #[test]
    fn traversal_and_disallowed_roots_are_rejected() {
        for path in [
            "../secrets.txt",
            "config/../../escape.txt",
            "mods/a.jar",
            "saves/world/level.dat",
            "/etc/passwd",
            "config\\windows.cfg",
            "",
        ] {
            assert!(
                classify_relative_path(path).is_err(),
                "expected '{path}' to be rejected"
            );
        }
        assert_eq!(classify_relative_path("options.txt").unwrap(), "options");
        assert_eq!(classify_relative_path("config/a/b.json").unwrap(), "config");
        assert_eq!(classify_relative_path("servers.dat").unwrap(), "servers");
    }

    #[test]
    fn a_failed_capture_leaves_no_partial_template() {
        let tmp = tempfile::tempdir().unwrap();
        let instance = tmp.path().join("instance");
        fs::create_dir_all(&instance).unwrap();
        seed_instance(&instance);
        let root = tmp.path().join("templates");

        let error = create_template(
            &root,
            CreateTemplateRequest {
                id: "broken",
                name: "Broken",
                description: None,
                jvm: TemplateJvm::default(),
                source_instance_dir: Some(&instance),
                selected_paths: &["options.txt".into(), "config/missing.cfg".into()],
                now: "2026-01-01T00:00:00Z",
            },
        )
        .unwrap_err();
        assert!(error.contains("config/missing.cfg"), "{error}");
        assert!(!root.join("broken").exists());
        assert!(list_templates(&root).unwrap().is_empty());
    }

    #[test]
    fn apply_re_validates_paths_from_a_tampered_template_json() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("templates");
        create_template(
            &root,
            CreateTemplateRequest {
                id: "evil",
                name: "Evil",
                description: None,
                jvm: TemplateJvm::default(),
                source_instance_dir: None,
                selected_paths: &[],
                now: "2026-01-01T00:00:00Z",
            },
        )
        .unwrap();
        let manifest_path = root.join("evil/template.json");
        let mut template: InstanceTemplate =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        template.files.push(TemplateFile {
            relative_path: "../../pwned.txt".into(),
            sha256: "0".repeat(64),
            size: 1,
        });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&template).unwrap(),
        )
        .unwrap();

        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();
        assert!(apply_template_files(&root, "evil", &dest).is_err());
        assert!(!tmp.path().join("pwned.txt").exists());
    }

    #[test]
    fn update_renames_without_touching_captured_files() {
        let tmp = tempfile::tempdir().unwrap();
        let instance = tmp.path().join("instance");
        fs::create_dir_all(&instance).unwrap();
        seed_instance(&instance);
        let root = tmp.path().join("templates");
        create_template(
            &root,
            CreateTemplateRequest {
                id: "t1",
                name: "Old",
                description: None,
                jvm: TemplateJvm::default(),
                source_instance_dir: Some(&instance),
                selected_paths: &["options.txt".into()],
                now: "2026-01-01T00:00:00Z",
            },
        )
        .unwrap();

        let updated = update_template(
            &root,
            "t1",
            Some("New"),
            Some(Some("notes".into())),
            None,
            "2026-02-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(updated.name, "New");
        assert_eq!(updated.description.as_deref(), Some("notes"));
        assert_eq!(updated.files.len(), 1);
        assert_eq!(updated.created_at, "2026-01-01T00:00:00Z");
        assert_eq!(updated.updated_at, "2026-02-01T00:00:00Z");
    }

    #[test]
    fn delete_is_idempotent_and_listing_survives_a_corrupt_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("templates");
        for (id, created) in [("a", "2026-01-01T00:00:00Z"), ("b", "2026-03-01T00:00:00Z")] {
            create_template(
                &root,
                CreateTemplateRequest {
                    id,
                    name: id,
                    description: None,
                    jvm: TemplateJvm::default(),
                    source_instance_dir: None,
                    selected_paths: &[],
                    now: created,
                },
            )
            .unwrap();
        }
        fs::create_dir_all(root.join("junk")).unwrap();
        fs::write(root.join("junk/template.json"), b"not json").unwrap();

        let listed = list_templates(&root).unwrap();
        let ids: Vec<&str> = listed.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a"], "newest first, corrupt entry skipped");

        delete_template(&root, "a").unwrap();
        delete_template(&root, "a").unwrap();
        assert_eq!(list_templates(&root).unwrap().len(), 1);
    }
}
