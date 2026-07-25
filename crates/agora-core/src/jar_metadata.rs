use crate::dependency_ops::{IncompatibilityDecl, IncompatibilitySource, JarDeps, ProvidedMod};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Seek};
use std::path::Path;

/// Whether an artifact was fully inspected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParseStatus {
    #[default]
    Complete,
    Partial,
    Failed,
}

impl ParseStatus {
    fn record_partial(&mut self) {
        if *self == Self::Complete {
            *self = Self::Partial;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseDiagnosticKind {
    ArchiveOpenFailed,
    MetadataMalformed,
    MetadataTooLarge,
    NestedPathUnsafe,
    NestedArchiveMalformed,
    NestedLimitExceeded,
    IoFailure,
    UnsupportedMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub kind: ParseDiagnosticKind,
    pub entry_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct JarInventoryResult {
    pub metadata: JarDeps,
    pub status: ParseStatus,
    pub diagnostics: Vec<ParseDiagnostic>,
}

/// Loader/framework mod IDs that are part of the ecosystem but never present as
/// installable mod JARs. Declaring any of these as a dependency would produce a
/// false `MissingRequiredDependency` blocker, so they are filtered out.
const DEPENDENCY_IGNORE_LIST: &[&str] = &[
    "minecraft",
    "fabric",
    "fabricloader",
    "quilt_loader",
    "java",
    "forge",
    "neoforge",
];

/// Maximum nesting depth for explicitly declared nested JARs.
const MAX_NESTING_DEPTH: u32 = 4;

/// Maximum total number of nested JARs across all nesting levels.
const MAX_TOTAL_NESTED_JARS: u32 = 128;

/// Maximum decompressed bytes for a single nested JAR entry.
const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;

/// Maximum total decompressed bytes across all nested JARs.
const MAX_TOTAL_NESTED_BYTES: u64 = 128 * 1024 * 1024;

/// Maximum bytes read from a single metadata text entry.
const MAX_METADATA_TEXT_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetadataScope {
    Any,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl MetadataScope {
    fn from_loader(loader: &str) -> Self {
        match loader.trim().to_ascii_lowercase().as_str() {
            "fabric" => Self::Fabric,
            "quilt" => Self::Quilt,
            "forge" => Self::Forge,
            "neoforge" => Self::NeoForge,
            _ => Self::Any,
        }
    }

    fn includes_fabric(self, has_quilt_metadata: bool) -> bool {
        matches!(self, Self::Any | Self::Fabric)
            || matches!(self, Self::Quilt) && !has_quilt_metadata
    }

    fn includes_quilt(self) -> bool {
        matches!(self, Self::Any | Self::Quilt)
    }

    fn includes_forge(self) -> bool {
        matches!(self, Self::Any | Self::Forge)
    }

    fn includes_neoforge(self) -> bool {
        matches!(self, Self::Any | Self::NeoForge)
    }
}

/// Metadata parsed for a specific active loader.
///
/// `has_native_metadata` distinguishes a native loader implementation from a
/// compatibility artifact that only carries metadata for another loader. This
/// lets Modrinth dependency resolution retain compatibility-route dependencies
/// only when the selected JAR has no native metadata for the active loader.
#[derive(Debug, Clone, Default)]
pub struct LoaderJarMetadata {
    pub metadata: JarDeps,
    pub has_native_metadata: bool,
    pub status: ParseStatus,
    pub diagnostics: Vec<ParseDiagnostic>,
}

/// Shared budget tracker for nested JAR parsing limits.
#[derive(Default)]
struct NestedJarBudget {
    total_count: u32,
    total_bytes: u64,
}

/// Parse a `.jar` file to extract Java packages, mod ID, mod version, and
/// declared dependencies from `fabric.mod.json`, `quilt.mod.json`,
/// `META-INF/mods.toml` / `META-INF/neoforge.mods.toml`, and
/// `META-INF/MANIFEST.MF` (for `Implementation-Version`).
///
/// Also parses Fabric/Quilt `provides` aliases and explicitly declared nested
/// JAR entries (via `jars`), respecting depth/count/size bounds.
///
/// The compatibility wrapper returns only metadata. New callers should use
/// [`parse_jar_metadata_result`] so an unreadable artifact is not confused
/// with a valid metadata-free JAR.
pub fn parse_jar_metadata(jar_path: &Path) -> JarDeps {
    parse_jar_metadata_result(jar_path).metadata
}

/// Parse a JAR and retain inspection status and safe diagnostics.
pub fn parse_jar_metadata_result(jar_path: &Path) -> JarInventoryResult {
    let file = match std::fs::File::open(jar_path) {
        Ok(f) => f,
        Err(_) => {
            return JarInventoryResult {
                metadata: JarDeps::default(),
                status: ParseStatus::Failed,
                diagnostics: vec![ParseDiagnostic {
                    kind: ParseDiagnosticKind::ArchiveOpenFailed,
                    entry_path: None,
                    message: "The JAR could not be opened.".into(),
                }],
            }
        }
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => {
            return JarInventoryResult {
                metadata: JarDeps::default(),
                status: ParseStatus::Failed,
                diagnostics: vec![ParseDiagnostic {
                    kind: ParseDiagnosticKind::ArchiveOpenFailed,
                    entry_path: None,
                    message: "The JAR archive is malformed.".into(),
                }],
            }
        }
    };
    let mut budget = NestedJarBudget::default();
    let parsed = parse_from_archive(&mut archive, 0, &mut budget, MetadataScope::Any);
    JarInventoryResult {
        metadata: parsed.metadata,
        status: parsed.status,
        diagnostics: parsed.diagnostics,
    }
}

/// Parse a JAR using only metadata for the active loader.
///
/// Fabric reads `fabric.mod.json`; Quilt prefers `quilt.mod.json` and falls
/// back to Fabric metadata; Forge and NeoForge read only their respective TOML
/// manifests. Unknown loader names retain the legacy all-manifest behavior.
pub fn parse_jar_metadata_for_loader(jar_path: &Path, loader: &str) -> JarDeps {
    parse_jar_metadata_for_loader_with_status(jar_path, loader).metadata
}

/// Parse loader-specific metadata and report whether the JAR contains native
/// metadata for the requested loader.
pub fn parse_jar_metadata_for_loader_with_status(
    jar_path: &Path,
    loader: &str,
) -> LoaderJarMetadata {
    let file = match std::fs::File::open(jar_path) {
        Ok(f) => f,
        Err(_) => {
            return LoaderJarMetadata {
                metadata: JarDeps::default(),
                has_native_metadata: false,
                status: ParseStatus::Failed,
                diagnostics: vec![ParseDiagnostic {
                    kind: ParseDiagnosticKind::ArchiveOpenFailed,
                    entry_path: None,
                    message: "The JAR could not be opened.".into(),
                }],
            }
        }
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => {
            return LoaderJarMetadata {
                metadata: JarDeps::default(),
                has_native_metadata: false,
                status: ParseStatus::Failed,
                diagnostics: vec![ParseDiagnostic {
                    kind: ParseDiagnosticKind::ArchiveOpenFailed,
                    entry_path: None,
                    message: "The JAR archive is malformed.".into(),
                }],
            }
        }
    };
    let mut budget = NestedJarBudget::default();
    parse_from_archive(
        &mut archive,
        0,
        &mut budget,
        MetadataScope::from_loader(loader),
    )
}

/// Parse loader-specific metadata directly from verified JAR bytes.
pub fn parse_jar_metadata_bytes_for_loader(bytes: &[u8], loader: &str) -> LoaderJarMetadata {
    let cursor = Cursor::new(bytes);
    let mut archive = match zip::ZipArchive::new(cursor) {
        Ok(a) => a,
        Err(_) => {
            return LoaderJarMetadata {
                metadata: JarDeps::default(),
                has_native_metadata: false,
                status: ParseStatus::Failed,
                diagnostics: vec![ParseDiagnostic {
                    kind: ParseDiagnosticKind::ArchiveOpenFailed,
                    entry_path: None,
                    message: "The JAR archive is malformed.".into(),
                }],
            }
        }
    };
    let mut budget = NestedJarBudget::default();
    parse_from_archive(
        &mut archive,
        0,
        &mut budget,
        MetadataScope::from_loader(loader),
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_fabric_metadata(
    value: &serde_json::Value,
    mod_jar_id: &mut Option<String>,
    mod_version: &mut Option<String>,
    fabric_version: &mut Option<String>,
    depends_on: &mut BTreeSet<String>,
    optional_deps: &mut BTreeSet<String>,
    incompatible_ids: &mut BTreeSet<String>,
    incompatibility_decls: &mut Vec<IncompatibilityDecl>,
    provides: &mut Vec<String>,
    jars: &mut Vec<String>,
) {
    if let Some(id) = value.get("id").and_then(|v| v.as_str()) {
        if !id.is_empty() {
            *mod_jar_id = Some(id.to_string());
        }
    }
    if let Some(version) = value.get("version").and_then(|v| v.as_str()) {
        if !version.is_empty() {
            *mod_version = Some(version.to_string());
            *fabric_version = Some(version.to_string());
        }
    }
    if let Some(value) = value.get("depends") {
        extract_fabric_deps(value, depends_on, None);
    }
    for key in ["recommends", "suggests"] {
        if let Some(value) = value.get(key) {
            extract_fabric_deps(value, optional_deps, None);
        }
    }
    if let Some(value) = value.get("breaks") {
        extract_fabric_deps(
            value,
            incompatible_ids,
            Some((IncompatibilitySource::FabricBreaks, incompatibility_decls)),
        );
    }
    if let Some(value) = value.get("conflicts") {
        extract_fabric_deps(
            value,
            incompatible_ids,
            Some((
                IncompatibilitySource::FabricConflicts,
                incompatibility_decls,
            )),
        );
    }
    if let Some(values) = value.get("provides").and_then(|v| v.as_array()) {
        provides.extend(
            values
                .iter()
                .filter_map(|value| value.as_str())
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        );
    }
    if let Some(values) = value.get("jars").and_then(|v| v.as_array()) {
        jars.extend(values.iter().filter_map(|value| {
            value
                .get("file")
                .and_then(|file| file.as_str())
                .filter(|file| !file.is_empty())
                .map(str::to_string)
        }));
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_quilt_metadata(
    value: &serde_json::Value,
    mod_jar_id: &mut Option<String>,
    mod_version: &mut Option<String>,
    quilt_version: &mut Option<String>,
    depends_on: &mut BTreeSet<String>,
    incompatible_ids: &mut BTreeSet<String>,
    incompatibility_decls: &mut Vec<IncompatibilityDecl>,
    provides: &mut Vec<(String, Option<String>)>,
    jars: &mut Vec<String>,
) {
    let Some(loader) = value.get("quilt_loader").or(value.get("quiltLoader")) else {
        return;
    };
    if let Some(id) = loader.get("id").and_then(|v| v.as_str()) {
        if !id.is_empty() && mod_jar_id.is_none() {
            *mod_jar_id = Some(id.to_string());
        }
    }
    if let Some(version) = loader.get("version").and_then(|v| v.as_str()) {
        if !version.is_empty() {
            *mod_version = Some(version.to_string());
            *quilt_version = Some(version.to_string());
        }
    }
    if let Some(value) = loader.get("depends") {
        extract_fabric_deps(value, depends_on, None);
    }
    if let Some(value) = loader.get("breaks") {
        extract_fabric_deps(
            value,
            incompatible_ids,
            Some((IncompatibilitySource::QuiltBreaks, incompatibility_decls)),
        );
    }
    if let Some(value) = loader.get("conflicts") {
        extract_fabric_deps(
            value,
            incompatible_ids,
            Some((IncompatibilitySource::QuiltConflicts, incompatibility_decls)),
        );
    }
    if let Some(values) = loader.get("provides").and_then(|v| v.as_array()) {
        for value in values {
            if let Some(id) = value.as_str().filter(|id| !id.is_empty()) {
                provides.push((id.to_string(), None));
            } else if let Some(object) = value.as_object() {
                let Some(id) = object
                    .get("id")
                    .and_then(|value| value.as_str())
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                let version = object
                    .get("version")
                    .and_then(|value| value.as_str())
                    .filter(|version| !version.is_empty())
                    .map(str::to_string);
                provides.push((id.to_string(), version));
            }
        }
    }
    if let Some(values) = loader.get("jars").and_then(|v| v.as_array()) {
        jars.extend(
            values
                .iter()
                .filter_map(|value| value.as_str())
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        );
    }
}

/// Generic recursive JAR metadata parser.
///
/// Works over any `R: Read + Seek` so it can be called with
/// `ZipArchive<File>` for the top-level JAR and `ZipArchive<Cursor<Vec<u8>>>`
/// for nested JARs extracted from parent entries.
fn parse_from_archive<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    depth: u32,
    budget: &mut NestedJarBudget,
    scope: MetadataScope,
) -> LoaderJarMetadata {
    if depth > MAX_NESTING_DEPTH {
        return LoaderJarMetadata {
            metadata: JarDeps::default(),
            has_native_metadata: false,
            status: ParseStatus::Partial,
            diagnostics: vec![ParseDiagnostic {
                kind: ParseDiagnosticKind::NestedLimitExceeded,
                entry_path: None,
                message: "Nested JAR depth limit exceeded.".into(),
            }],
        };
    }

    let total_len = archive.len();
    let has_quilt_metadata = (0..total_len).any(|i| {
        archive
            .by_index(i)
            .ok()
            .is_some_and(|entry| entry.name() == "quilt.mod.json")
    });
    let has_neoforge_metadata = (0..total_len).any(|i| {
        archive
            .by_index(i)
            .ok()
            .is_some_and(|entry| entry.name() == "META-INF/neoforge.mods.toml")
    });
    let include_fabric = scope.includes_fabric(has_quilt_metadata);
    let include_quilt = scope.includes_quilt();
    let include_forge = scope.includes_forge();
    let include_neoforge = scope.includes_neoforge();
    let mut packages: BTreeSet<String> = BTreeSet::new();
    let mut mod_jar_id: Option<String> = None;
    let mut mod_version: Option<String> = None;
    let mut depends_on: BTreeSet<String> = BTreeSet::new();
    let mut optional_deps: BTreeSet<String> = BTreeSet::new();
    let mut incompatible_ids: BTreeSet<String> = BTreeSet::new();
    let mut incompatibility_decls: Vec<IncompatibilityDecl> = Vec::new();
    let mut forge_mod_id: Option<String> = None;
    let mut forge_provides_strs: BTreeSet<String> = BTreeSet::new();
    let mut forge_version: Option<String> = None;
    let mut manifest_impl_version: Option<String> = None;
    let mut saw_neoforge_toml = false;
    let mut has_native_metadata = false;
    let mut status = ParseStatus::Complete;
    let mut diagnostics = Vec::new();

    // Fabric/Quilt specific accumulators.
    let mut fabric_version: Option<String> = None;
    let mut fabric_provides_strs: Vec<String> = Vec::new();
    let mut fabric_jars_strs: Vec<String> = Vec::new();
    let mut quilt_version: Option<String> = None;
    let mut quilt_provides: Vec<(String, Option<String>)> = Vec::new();
    let mut quilt_jars_strs: Vec<String> = Vec::new();
    let mut forge_jarjar_paths: Vec<String> = Vec::new();

    // ---- First pass: collect metadata and record declared nested paths ----
    for i in 0..total_len {
        let name = match archive.by_index(i) {
            Ok(e) => e.name().to_string(),
            Err(_) => continue,
        };
        if name.ends_with(".class") {
            let stem = match name.strip_suffix(".class") {
                Some(s) => s,
                None => continue,
            };
            let replaced = stem.replace('\\', "/");
            let segments: Vec<&str> = replaced.split('/').collect();
            if segments.len() < 3 {
                continue;
            }
            let dir_segments: Vec<&str> = segments[..segments.len() - 1].to_vec();
            packages.insert(dir_segments.join("."));
            continue;
        }
        if name == "fabric.mod.json" && include_fabric {
            match read_entry_utf8_bounded(archive, i, MAX_METADATA_TEXT_BYTES) {
                Some(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(value) => {
                        has_native_metadata = true;
                        collect_fabric_metadata(
                            &value,
                            &mut mod_jar_id,
                            &mut mod_version,
                            &mut fabric_version,
                            &mut depends_on,
                            &mut optional_deps,
                            &mut incompatible_ids,
                            &mut incompatibility_decls,
                            &mut fabric_provides_strs,
                            &mut fabric_jars_strs,
                        );
                    }
                    Err(_) => {
                        status.record_partial();
                        diagnostics.push(ParseDiagnostic {
                            kind: ParseDiagnosticKind::MetadataMalformed,
                            entry_path: Some(name.clone()),
                            message: "Fabric metadata is malformed JSON.".into(),
                        });
                    }
                },
                None => {
                    status.record_partial();
                    diagnostics.push(ParseDiagnostic {
                        kind: ParseDiagnosticKind::MetadataTooLarge,
                        entry_path: Some(name.clone()),
                        message: "Fabric metadata could not be read within the metadata limit."
                            .into(),
                    });
                }
            }
            continue;
        }

        if name == "quilt.mod.json" && include_quilt {
            match read_entry_utf8_bounded(archive, i, MAX_METADATA_TEXT_BYTES) {
                Some(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(value) => {
                        has_native_metadata = true;
                        collect_quilt_metadata(
                            &value,
                            &mut mod_jar_id,
                            &mut mod_version,
                            &mut quilt_version,
                            &mut depends_on,
                            &mut incompatible_ids,
                            &mut incompatibility_decls,
                            &mut quilt_provides,
                            &mut quilt_jars_strs,
                        );
                    }
                    Err(_) => {
                        status.record_partial();
                        diagnostics.push(ParseDiagnostic {
                            kind: ParseDiagnosticKind::MetadataMalformed,
                            entry_path: Some(name.clone()),
                            message: "Quilt metadata is malformed JSON.".into(),
                        });
                    }
                },
                None => {
                    status.record_partial();
                    diagnostics.push(ParseDiagnostic {
                        kind: ParseDiagnosticKind::MetadataTooLarge,
                        entry_path: Some(name.clone()),
                        message: "Quilt metadata could not be read within the metadata limit."
                            .into(),
                    });
                }
            }
            continue;
        }

        /*
        if name == "fabric.mod.json" && include_fabric {
            match read_entry_utf8_bounded(archive, i, MAX_METADATA_TEXT_BYTES) {
                None => {
                    status.record_partial();
                    diagnostics.push(ParseDiagnostic {
                        kind: ParseDiagnosticKind::MetadataTooLarge,
                        entry_path: Some(name.clone()),
                        message: "Fabric metadata could not be read within the metadata limit.".into(),
                    });
                }
                Some(content_str) => {
                    match serde_json::from_str::<serde_json::Value>(&content_str) {
                        Err(_) => {
                            status.record_partial();
                            diagnostics.push(ParseDiagnostic {
                                kind: ParseDiagnosticKind::MetadataMalformed,
                                entry_path: Some(name.clone()),
                                message: "Fabric metadata is malformed JSON.".into(),
                            });
                        }
                        Ok(value) => {
                    has_native_metadata = true;
                    if let Some(id_str) = value.get("id").and_then(|v| v.as_str()) {
                        if !id_str.is_empty() {
                            mod_jar_id = Some(id_str.to_string());
                        }
                    }
                    if let Some(ver) = value.get("version").and_then(|v| v.as_str()) {
                        if !ver.is_empty() {
                            mod_version = Some(ver.to_string());
                            fabric_version = Some(ver.to_string());
                        }
                    }
                    // Required deps.
                    if let Some(val) = value.get("depends") {
                        extract_fabric_deps(val, &mut depends_on, None);
                    }
                    // Optional deps (recommends + suggests both soft).
                    for key in ["recommends", "suggests"] {
                        if let Some(val) = value.get(key) {
                            extract_fabric_deps(val, &mut optional_deps, None);
                        }
                    }
                    // breaks -> hard incompat; conflicts -> soft incompat.
                    if let Some(val) = value.get("breaks") {
                        extract_fabric_deps(
                            val,
                            &mut incompatible_ids,
                            Some((
                                IncompatibilitySource::FabricBreaks,
                                &mut incompatibility_decls,
                            )),
                        );
                    }
                    if let Some(val) = value.get("conflicts") {
                        extract_fabric_deps(
                            val,
                            &mut incompatible_ids,
                            Some((
                                IncompatibilitySource::FabricConflicts,
                                &mut incompatibility_decls,
                            )),
                        );
                    }
                    // Fabric `provides` array of strings.
                    if let Some(provides) = value.get("provides").and_then(|v| v.as_array()) {
                        for elem in provides {
                            if let Some(s) = elem.as_str() {
                                if !s.is_empty() {
                                    fabric_provides_strs.push(s.to_string());
                                }
                            }
                        }
                    }
                    // Fabric `jars` array of objects with `file` string.
                    if let Some(jars) = value.get("jars").and_then(|v| v.as_array()) {
                        for elem in jars {
                            if let Some(file) = elem.get("file").and_then(|v| v.as_str()) {
                                if !file.is_empty() {
                                    fabric_jars_strs.push(file.to_string());
                                }
                            }
                        }
                         }
                         }
                     }
                  }
                  }
                  }
            continue;
        if name == "quilt.mod.json" && include_quilt {
            match read_entry_utf8_bounded(archive, i, MAX_METADATA_TEXT_BYTES) {
                None => {
                    status.record_partial();
                    diagnostics.push(ParseDiagnostic {
                        kind: ParseDiagnosticKind::MetadataTooLarge,
                        entry_path: Some(name.clone()),
                        message: "Quilt metadata could not be read within the metadata limit.".into(),
                    });
                }
                Some(content_str) => {
                    match serde_json::from_str::<serde_json::Value>(&content_str) {
                        Err(_) => {
                            status.record_partial();
                            diagnostics.push(ParseDiagnostic {
                                kind: ParseDiagnosticKind::MetadataMalformed,
                                entry_path: Some(name.clone()),
                                message: "Quilt metadata is malformed JSON.".into(),
                            });
                        }
                        Ok(value) => {
                    has_native_metadata = true;
                    if let Some(ql) = value.get("quilt_loader").or(value.get("quiltLoader")) {
                        if let Some(id_str) = ql.get("id").and_then(|v| v.as_str()) {
                            if !id_str.is_empty() && mod_jar_id.is_none() {
                                mod_jar_id = Some(id_str.to_string());
                            }
                        }
                        if let Some(ver) = ql.get("version").and_then(|v| v.as_str()) {
                            if !ver.is_empty() {
                                mod_version = Some(ver.to_string());
                                quilt_version = Some(ver.to_string());
                            }
                        }
                        // Quilt deps/breaks/conflicts use the same grammar as Fabric.
                        if let Some(val) = ql.get("depends") {
                            extract_fabric_deps(val, &mut depends_on, None);
                        }
                        if let Some(val) = ql.get("breaks") {
                            extract_fabric_deps(
                                val,
                                &mut incompatible_ids,
                                Some((
                                    IncompatibilitySource::QuiltBreaks,
                                    &mut incompatibility_decls,
                                )),
                            );
                        }
                        if let Some(val) = ql.get("conflicts") {
                            extract_fabric_deps(
                                val,
                                &mut incompatible_ids,
                                Some((
                                    IncompatibilitySource::QuiltConflicts,
                                    &mut incompatibility_decls,
                                )),
                            );
                        }
                        // Quilt `quilt_loader.provides`: array of strings or objects.
                        if let Some(provides) = ql.get("provides").and_then(|v| v.as_array()) {
                            for elem in provides {
                                if let Some(s) = elem.as_str() {
                                    if !s.is_empty() {
                                        quilt_provides.push((s.to_string(), None));
                                    }
                                } else if let Some(obj) = elem.as_object() {
                                    let pid = obj.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                    if !pid.is_empty() {
                                        let pver = obj
                                            .get("version")
                                            .and_then(|v| v.as_str())
                                            .filter(|s| !s.is_empty())
                                            .map(|s| s.to_string());
                                        quilt_provides.push((pid.to_string(), pver));
                                    }
                                }
                            }
                        }
                        // Quilt `quilt_loader.jars`: array of string paths.
                        if let Some(jars) = ql.get("jars").and_then(|v| v.as_array()) {
                            for elem in jars {
                                if let Some(s) = elem.as_str() {
                                    if !s.is_empty() {
                                        quilt_jars_strs.push(s.to_string());
                                    }
                                }
                            }
                        }
                        }
                        }
                    }
                }
                 }
                 }
                 }
            continue;
        */
        // NeoForge ships neoforge.mods.toml; Forge ships mods.toml. Same TOML
        // schema, so both go through the same parser. Prefer neoforge when both
        // are present (a NeoForge mod's mods.toml, if also shipped, is usually
        // a stub).
        if name == "META-INF/neoforge.mods.toml" && include_neoforge {
            saw_neoforge_toml = true;
            match read_entry_utf8_bounded(archive, i, MAX_METADATA_TEXT_BYTES) {
                Some(content) => match parse_forge_metadata(&content) {
                    Ok(parsed) => {
                        has_native_metadata = true;
                        forge_provides_strs.extend(parsed.mod_ids.clone());
                        merge_forge_metadata(
                            parsed,
                            &mut depends_on,
                            &mut optional_deps,
                            &mut incompatible_ids,
                            &mut incompatibility_decls,
                            &mut forge_mod_id,
                            &mut forge_version,
                        );
                    }
                    Err(message) => {
                        status.record_partial();
                        diagnostics.push(ParseDiagnostic {
                            kind: ParseDiagnosticKind::MetadataMalformed,
                            entry_path: Some(name.clone()),
                            message,
                        });
                    }
                },
                None => {
                    status.record_partial();
                    diagnostics.push(ParseDiagnostic {
                        kind: ParseDiagnosticKind::MetadataTooLarge,
                        entry_path: Some(name.clone()),
                        message: "NeoForge metadata could not be read within the metadata limit."
                            .into(),
                    });
                }
            }
            continue;
        }
        if name == "META-INF/mods.toml"
            && include_forge
            && !(matches!(scope, MetadataScope::Any) && has_neoforge_metadata)
            && !saw_neoforge_toml
        {
            match read_entry_utf8_bounded(archive, i, MAX_METADATA_TEXT_BYTES) {
                Some(content) => match parse_forge_metadata(&content) {
                    Ok(parsed) => {
                        has_native_metadata = true;
                        forge_provides_strs.extend(parsed.mod_ids.clone());
                        merge_forge_metadata(
                            parsed,
                            &mut depends_on,
                            &mut optional_deps,
                            &mut incompatible_ids,
                            &mut incompatibility_decls,
                            &mut forge_mod_id,
                            &mut forge_version,
                        );
                    }
                    Err(message) => {
                        status.record_partial();
                        diagnostics.push(ParseDiagnostic {
                            kind: ParseDiagnosticKind::MetadataMalformed,
                            entry_path: Some(name.clone()),
                            message,
                        });
                    }
                },
                None => {
                    status.record_partial();
                    diagnostics.push(ParseDiagnostic {
                        kind: ParseDiagnosticKind::MetadataTooLarge,
                        entry_path: Some(name.clone()),
                        message: "Forge metadata could not be read within the metadata limit."
                            .into(),
                    });
                }
            }
            continue;
        }
        if name == "META-INF/jarjar/metadata.json" && (include_forge || include_neoforge) {
            match read_entry_utf8_bounded(archive, i, MAX_METADATA_TEXT_BYTES) {
                Some(content) => match extract_jarjar_paths(&content) {
                    Ok(paths) => forge_jarjar_paths.extend(paths),
                    Err(message) => {
                        status.record_partial();
                        diagnostics.push(ParseDiagnostic {
                            kind: ParseDiagnosticKind::MetadataMalformed,
                            entry_path: Some(name.clone()),
                            message,
                        });
                    }
                },
                None => {
                    status.record_partial();
                    diagnostics.push(ParseDiagnostic {
                        kind: ParseDiagnosticKind::MetadataTooLarge,
                        entry_path: Some(name.clone()),
                        message: "Jar-in-Jar metadata could not be read within the metadata limit."
                            .into(),
                    });
                }
            }
            continue;
        }
        if name == "META-INF/MANIFEST.MF" {
            if let Some(content) = read_entry_utf8_bounded(archive, i, MAX_METADATA_TEXT_BYTES) {
                manifest_impl_version = parse_manifest_version(&content);
            }
            continue;
        }
    }

    // Resolve mod_jar_id and mod_version (same logic as original).
    if mod_jar_id.is_none() {
        mod_jar_id = forge_mod_id;
    }
    if mod_version.is_none() {
        mod_version = forge_version.take();
    }
    if mod_version.as_deref() == Some("${file.jarVersion}") {
        mod_version = manifest_impl_version.take();
    }
    if mod_version.is_none() {
        mod_version = manifest_impl_version;
    }

    // ---- Build initial provided_mods from Fabric/Quilt provides ----
    let mut provided_mods_map: BTreeMap<String, Option<String>> = BTreeMap::new();

    for id in forge_provides_strs {
        if mod_jar_id.as_deref() != Some(id.as_str()) {
            provided_mods_map.insert(id, mod_version.clone());
        }
    }

    for alias in fabric_provides_strs {
        let ver = fabric_version.as_ref().cloned();
        provided_mods_map
            .entry(alias)
            .and_modify(|e| {
                if e.is_none() && ver.is_some() {
                    *e = ver.clone();
                }
            })
            .or_insert(ver);
    }

    for (pid, explicit_ver) in quilt_provides {
        let ver = explicit_ver.or_else(|| quilt_version.clone());
        provided_mods_map
            .entry(pid)
            .and_modify(|e| {
                if e.is_none() && ver.is_some() {
                    *e = ver.clone();
                }
            })
            .or_insert(ver);
    }

    // ---- Validate and dedupe declared nested JAR paths ----
    let mut invalid_nested_paths = Vec::new();
    let nested_paths: BTreeSet<String> = fabric_jars_strs
        .into_iter()
        .chain(quilt_jars_strs)
        .chain(forge_jarjar_paths)
        .filter(|p| {
            if !is_safe_nested_path(p) {
                invalid_nested_paths.push(p.clone());
                false
            } else {
                true
            }
        })
        .map(|p| p.replace('\\', "/"))
        .collect();
    if !invalid_nested_paths.is_empty() {
        status.record_partial();
        for path in invalid_nested_paths {
            diagnostics.push(ParseDiagnostic {
                kind: ParseDiagnosticKind::NestedPathUnsafe,
                entry_path: Some(path),
                message: "A declared nested JAR path was rejected as unsafe.".into(),
            });
        }
    }

    // ---- Second pass: read and parse declared nested JARs ----
    let mut processed_nested_paths: BTreeSet<String> = BTreeSet::new();
    for i in 0..total_len {
        let entry_name = match archive.by_index(i) {
            Ok(e) => e.name().to_string(),
            Err(_) => continue,
        };
        let normalized = entry_name.replace('\\', "/");
        if !nested_paths.contains(&normalized) || !processed_nested_paths.insert(normalized.clone())
        {
            continue;
        }

        if budget.total_count >= MAX_TOTAL_NESTED_JARS {
            status.record_partial();
            diagnostics.push(ParseDiagnostic {
                kind: ParseDiagnosticKind::NestedLimitExceeded,
                entry_path: None,
                message: "Nested JAR count limit exceeded.".into(),
            });
            break;
        }

        let bytes = read_entry_bytes_bounded(archive, i, MAX_ENTRY_BYTES);
        let bytes = match bytes {
            Some(b) if !b.is_empty() => b,
            _ => {
                status.record_partial();
                diagnostics.push(ParseDiagnostic {
                    kind: ParseDiagnosticKind::IoFailure,
                    entry_path: Some(normalized.clone()),
                    message: "A declared nested JAR could not be read.".into(),
                });
                continue;
            }
        };

        let entry_size = bytes.len() as u64;
        if budget.total_bytes + entry_size > MAX_TOTAL_NESTED_BYTES {
            status.record_partial();
            diagnostics.push(ParseDiagnostic {
                kind: ParseDiagnosticKind::NestedLimitExceeded,
                entry_path: Some(normalized.clone()),
                message: "Nested JAR decompressed byte limit exceeded.".into(),
            });
            continue;
        }
        budget.total_bytes += entry_size;
        budget.total_count += 1;

        let cursor = Cursor::new(bytes);
        let mut nested_archive = match zip::ZipArchive::new(cursor) {
            Ok(a) => a,
            Err(_) => {
                status.record_partial();
                diagnostics.push(ParseDiagnostic {
                    kind: ParseDiagnosticKind::NestedArchiveMalformed,
                    entry_path: Some(normalized.clone()),
                    message: "A declared nested JAR is malformed.".into(),
                });
                continue;
            }
        };
        let nested = parse_from_archive(&mut nested_archive, depth + 1, budget, scope);
        let nested_metadata = nested.metadata;
        if nested.status != ParseStatus::Complete {
            status.record_partial();
        }
        diagnostics.extend(nested.diagnostics.into_iter().map(|mut diagnostic| {
            diagnostic.entry_path = Some(match diagnostic.entry_path {
                Some(path) => format!("{normalized}!/{path}"),
                None => normalized.clone(),
            });
            diagnostic
        }));

        // Add nested primary ID as ProvidedMod.
        if let Some(ref nested_id) = nested_metadata.mod_jar_id {
            let nv = nested_metadata.mod_version.clone();
            provided_mods_map
                .entry(nested_id.clone())
                .and_modify(|e| {
                    if e.is_none() && nv.is_some() {
                        *e = nv.clone();
                    }
                })
                .or_insert(nv);
        }

        // Aggregate nested provided_mods.
        for pm in &nested_metadata.provided_mods {
            let pv = pm.version.clone();
            provided_mods_map
                .entry(pm.mod_id.clone())
                .and_modify(|e| {
                    if e.is_none() && pv.is_some() {
                        *e = pv.clone();
                    }
                })
                .or_insert(pv);
        }

        // Aggregate dependencies (except intra-JAR deps filtered later).
        depends_on.extend(nested_metadata.depends_on);
        optional_deps.extend(nested_metadata.optional_deps);
        incompatible_ids.extend(nested_metadata.incompatible_deps);
        incompatibility_decls.extend(nested_metadata.incompatibility_decls);
        packages.extend(nested_metadata.java_packages);
    }

    for path in nested_paths {
        if !processed_nested_paths.contains(&path) {
            status.record_partial();
            diagnostics.push(ParseDiagnostic {
                kind: ParseDiagnosticKind::IoFailure,
                entry_path: Some(path),
                message: "A declared nested JAR path was not found in the archive.".into(),
            });
        }
    }

    // ---- After all nesting, filter intra-JAR dependencies ----
    let supplied_ids: BTreeSet<&str> = {
        let mut ids = BTreeSet::new();
        if let Some(ref id) = mod_jar_id {
            ids.insert(id.as_str());
        }
        for id in provided_mods_map.keys() {
            ids.insert(id.as_str());
        }
        ids
    };

    depends_on.retain(|dep| !supplied_ids.contains(dep.as_str()));
    optional_deps.retain(|dep| !supplied_ids.contains(dep.as_str()));
    // Do NOT filter incompatible_deps or incompatibility_decls — an internally
    // bundled incompatibility is a real loader failure.

    // Apply DEPENDENCY_IGNORE_LIST.
    depends_on.retain(|dep| !DEPENDENCY_IGNORE_LIST.contains(&dep.as_str()));
    optional_deps.retain(|dep| !DEPENDENCY_IGNORE_LIST.contains(&dep.as_str()));
    incompatible_ids.retain(|dep| !DEPENDENCY_IGNORE_LIST.contains(&dep.as_str()));
    incompatibility_decls.retain(|d| !DEPENDENCY_IGNORE_LIST.contains(&d.mod_id.as_str()));

    // Build final ProvidedMod vec from map (already sorted by BTreeMap).
    let provided_mods: Vec<ProvidedMod> = provided_mods_map
        .into_iter()
        .map(|(mod_id, version)| ProvidedMod { mod_id, version })
        .collect();

    LoaderJarMetadata {
        metadata: JarDeps {
            java_packages: packages.into_iter().collect(),
            mod_jar_id,
            mod_version,
            depends_on: depends_on.into_iter().collect(),
            optional_deps: optional_deps.into_iter().collect(),
            incompatible_deps: incompatible_ids.into_iter().collect(),
            incompatibility_decls,
            provided_mods,
        },
        has_native_metadata,
        status,
        diagnostics,
    }
}

/// Validate that a declared nested JAR path is safe to read.
///
/// Rejects absolute paths, drive-prefixed paths, backslash-traversal patterns,
/// and any `..` path segment. Backslashes are normalized to forward slashes
/// for validation.
fn is_safe_nested_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");

    if normalized.is_empty() {
        return false;
    }

    // Reject absolute paths (start with /).
    if normalized.starts_with('/') {
        return false;
    }

    // Reject Windows drive-prefixed paths (e.g., "C:" or "c:").
    if normalized.len() >= 2 {
        let bytes = normalized.as_bytes();
        if bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return false;
        }
    }

    // Reject any path segment that is ".."
    for segment in normalized.split('/') {
        if segment == ".." {
            return false;
        }
    }

    true
}

/// Read a text entry from the archive with a byte-size bound.
/// Returns `None` if the entry exceeds `max_bytes` or is not valid UTF-8.
fn read_entry_utf8_bounded<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    index: usize,
    max_bytes: u64,
) -> Option<String> {
    let entry = archive.by_index(index).ok()?;
    let mut buf = Vec::new();
    entry.take(max_bytes + 1).read_to_end(&mut buf).ok()?;
    if buf.len() > max_bytes as usize {
        return None;
    }
    String::from_utf8(buf).ok()
}

/// Read a binary entry from the archive with a byte-size bound.
/// Returns `None` if the entry exceeds `max_bytes`.
fn read_entry_bytes_bounded<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    index: usize,
    max_bytes: u64,
) -> Option<Vec<u8>> {
    let entry = archive.by_index(index).ok()?;
    let mut buf = Vec::new();
    entry.take(max_bytes + 1).read_to_end(&mut buf).ok()?;
    if buf.len() > max_bytes as usize {
        return None;
    }
    Some(buf)
}

/// Extract Fabric dependency ids (and, for `breaks`/`conflicts`, structured
/// `IncompatibilityDecl`s carrying version-range predicates + severity).
///
/// - `out` receives the dep id strings (shared across depends/optional/incompat
///   flat lists depending on the caller).
/// - `incompat`: when `Some((severity, decls))`, also emit structured decls
///   capturing each version predicate. `None` for depends/recommends/suggests.
///
/// Fabric semantics:
/// - Object form `{"modid": "<2.0"}` → single AND predicate string.
/// - Object form `{"modid": ["<2.0", ">=3.0"]}` → OR array of predicate strings.
/// - Object form `{"modid": "*"}` → unconditional (any version).
/// - Array form `[{"id":..,"version":..}, {"identifier":..,"version":..}]` →
///   each object's `version` (may be absent) becomes a single-element range.
fn extract_fabric_deps(
    depends: &serde_json::Value,
    out: &mut BTreeSet<String>,
    mut incompat: Option<(IncompatibilitySource, &mut Vec<IncompatibilityDecl>)>,
) {
    match depends {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let ranges = fabric_version_ranges(val);
                out.insert(key.clone());
                if let Some((sev, decls)) = incompat.as_mut() {
                    decls.push(IncompatibilityDecl {
                        mod_id: key.clone(),
                        version_ranges: ranges,
                        source: *sev,
                    });
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for elem in arr {
                let id = elem
                    .get("id")
                    .and_then(|v| v.as_str())
                    .or_else(|| elem.get("identifier").and_then(|v| v.as_str()));
                if let Some(id) = id {
                    out.insert(id.to_string());
                    if let Some((sev, decls)) = incompat.as_mut() {
                        let ranges = match elem.get("version") {
                            Some(v) => fabric_version_ranges(v),
                            None => Vec::new(),
                        };
                        decls.push(IncompatibilityDecl {
                            mod_id: id.to_string(),
                            version_ranges: ranges,
                            source: *sev,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

/// Normalize a Fabric version value into a list of OR-joined predicate strings.
/// - String → single predicate (may contain space-separated AND predicates).
/// - Array of strings → OR list.
/// - Anything else → empty (unconditional).
fn fabric_version_ranges(val: &serde_json::Value) -> Vec<String> {
    match val {
        serde_json::Value::String(s) => {
            let t = s.trim();
            if t == "*" || t.is_empty() {
                Vec::new()
            } else {
                vec![s.clone()]
            }
        }
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|e| match e {
                serde_json::Value::String(s) => {
                    let t = s.trim();
                    if t == "*" || t.is_empty() {
                        None
                    } else {
                        Some(s.clone())
                    }
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// In-flight Forge dependency block state.
#[derive(Clone, Default)]
struct PendingForgeDep {
    /// The TARGET mod id (the dependency), read from the inner `modId` line.
    mod_id: Option<String>,
    /// NeoForge `type`.
    dep_type: Option<String>,
    /// Traditional Forge `mandatory` (true=required, false=optional).
    mandatory: Option<bool>,
    /// `versionRange` (Maven range; empty string = any version).
    version_range: Option<String>,
}

#[derive(Default)]
struct ParsedForgeMetadata {
    mod_ids: BTreeSet<String>,
    version: Option<String>,
    dependencies: Vec<PendingForgeDep>,
}

/// Parse the Forge/NeoForge TOML document with a real TOML parser.
///
/// Forge dependency table keys identify the declaring mod. The dependency
/// target is always the `modId` field inside each array element.
fn parse_forge_metadata(content: &str) -> Result<ParsedForgeMetadata, String> {
    let document = content
        .parse::<toml::Value>()
        .map_err(|_| "Forge metadata is malformed TOML.".to_string())?;
    let mut parsed = ParsedForgeMetadata::default();
    if let Some(mods) = document.get("mods") {
        let mods = mods
            .as_array()
            .ok_or_else(|| "Forge metadata contains an invalid [[mods]] array.".to_string())?;
        for entry in mods {
            let table = entry
                .as_table()
                .ok_or_else(|| "Forge metadata contains an invalid [[mods]] entry.".to_string())?;
            if let Some(id) = table.get("modId").and_then(toml::Value::as_str) {
                if !id.trim().is_empty() {
                    parsed.mod_ids.insert(id.to_string());
                }
            }
            if parsed.version.is_none() {
                parsed.version = table
                    .get("version")
                    .and_then(toml::Value::as_str)
                    .filter(|version| !version.trim().is_empty())
                    .map(str::to_string);
            }
        }
    } else if let Some(id) = document.get("modId").and_then(toml::Value::as_str) {
        if !id.trim().is_empty() {
            parsed.mod_ids.insert(id.to_string());
        }
        parsed.version = document
            .get("version")
            .and_then(toml::Value::as_str)
            .filter(|version| !version.trim().is_empty())
            .map(str::to_string);
    }

    if let Some(dependencies) = document.get("dependencies") {
        let owners = dependencies
            .as_table()
            .ok_or_else(|| "Forge metadata has an invalid dependencies table.".to_string())?;
        for blocks in owners.values() {
            let blocks = blocks.as_array().ok_or_else(|| {
                "Forge metadata has a dependency owner that is not an array.".to_string()
            })?;
            for block in blocks {
                let table = block.as_table().ok_or_else(|| {
                    "Forge metadata contains an invalid dependency entry.".to_string()
                })?;
                parsed.dependencies.push(PendingForgeDep {
                    mod_id: table
                        .get("modId")
                        .and_then(toml::Value::as_str)
                        .filter(|id| !id.trim().is_empty())
                        .map(str::to_string),
                    dep_type: table
                        .get("type")
                        .and_then(toml::Value::as_str)
                        .map(str::to_string),
                    mandatory: table.get("mandatory").and_then(toml::Value::as_bool),
                    version_range: table
                        .get("versionRange")
                        .and_then(toml::Value::as_str)
                        .map(str::to_string),
                });
            }
        }
    }

    Ok(parsed)
}

fn merge_forge_metadata(
    parsed: ParsedForgeMetadata,
    required_out: &mut BTreeSet<String>,
    optional_out: &mut BTreeSet<String>,
    incompatible_ids_out: &mut BTreeSet<String>,
    incompatibility_decls_out: &mut Vec<IncompatibilityDecl>,
    mod_id_out: &mut Option<String>,
    mod_version_out: &mut Option<String>,
) {
    if mod_id_out.is_none() {
        *mod_id_out = parsed.mod_ids.iter().next().cloned();
    }
    if mod_version_out.is_none() {
        *mod_version_out = parsed.version;
    }
    for dependency in parsed.dependencies {
        flush_forge_dep(
            &dependency,
            required_out,
            optional_out,
            incompatible_ids_out,
            incompatibility_decls_out,
        );
    }
}

#[cfg(test)]
fn extract_forge_mod_ids(content: &str) -> BTreeSet<String> {
    parse_forge_metadata(content)
        .map(|parsed| parsed.mod_ids)
        .unwrap_or_default()
}

/// Parse a Forge/NeoForge `mods.toml`/`neoforge.mods.toml` manifest.
///
/// Key fix: the section header `[[dependencies.<owner>]]` names the OWNER mod,
/// NOT the dependency. The dependency id is the `modId` line INSIDE the block.
/// Previously the parser stored the owner id as the dependency, which caused a
/// mod to appear to depend on / conflict with itself.
#[cfg(test)]
fn extract_forge_deps(
    content: &str,
    required_out: &mut BTreeSet<String>,
    optional_out: &mut BTreeSet<String>,
    incompatible_ids_out: &mut BTreeSet<String>,
    incompatibility_decls_out: &mut Vec<IncompatibilityDecl>,
    mod_id_out: &mut Option<String>,
    mod_version_out: &mut Option<String>,
) {
    if let Ok(parsed) = parse_forge_metadata(content) {
        merge_forge_metadata(
            parsed,
            required_out,
            optional_out,
            incompatible_ids_out,
            incompatibility_decls_out,
            mod_id_out,
            mod_version_out,
        );
    }
}

/// Finalize a pending Forge dependency block, routing it to the right buckets.
fn flush_forge_dep(
    pending: &PendingForgeDep,
    required_out: &mut BTreeSet<String>,
    optional_out: &mut BTreeSet<String>,
    incompatible_ids_out: &mut BTreeSet<String>,
    incompatibility_decls_out: &mut Vec<IncompatibilityDecl>,
) {
    let dep_id = match &pending.mod_id {
        Some(id) if !id.is_empty() => id.clone(),
        _ => return,
    };

    let ranges = match &pending.version_range {
        Some(r) => {
            let t = r.trim();
            if t.is_empty() || t == "*" {
                Vec::new()
            } else {
                vec![r.clone()]
            }
        }
        None => Vec::new(),
    };

    let effective = pending.dep_type.as_deref();
    match effective {
        Some("incompatible") => {
            incompatible_ids_out.insert(dep_id.clone());
            incompatibility_decls_out.push(IncompatibilityDecl {
                mod_id: dep_id,
                version_ranges: ranges,
                source: IncompatibilitySource::ForgeIncompatible,
            });
        }
        Some("discouraged") => {
            incompatible_ids_out.insert(dep_id.clone());
            incompatibility_decls_out.push(IncompatibilityDecl {
                mod_id: dep_id,
                version_ranges: ranges,
                source: IncompatibilitySource::ForgeDiscouraged,
            });
        }
        Some("optional") => {
            optional_out.insert(dep_id);
        }
        Some("required") | Some(_) => {
            required_out.insert(dep_id);
        }
        None => match pending.mandatory {
            Some(false) => {
                optional_out.insert(dep_id);
            }
            _ => {
                required_out.insert(dep_id);
            }
        },
    }
}

/// Read the official NeoForge/JarJar metadata and return only declared paths.
fn extract_jarjar_paths(content: &str) -> Result<Vec<String>, String> {
    let value = serde_json::from_str::<serde_json::Value>(content)
        .map_err(|_| "Jar-in-Jar metadata is malformed JSON.".to_string())?;
    let jars = value
        .get("jars")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "Jar-in-Jar metadata does not contain a jars array.".to_string())?;
    let mut paths = Vec::new();
    for jar in jars {
        let path = jar
            .get("path")
            .and_then(serde_json::Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| "Jar-in-Jar metadata contains an entry without a path.".to_string())?;
        paths.push(path.to_string());
    }
    Ok(paths)
}

/// Parse a `META-INF/MANIFEST.MF` file and extract the
/// `Implementation-Version` attribute value (if present).
fn parse_manifest_version(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let colon = line.find(':')?;
        let key = line[..colon].trim();
        if key.eq_ignore_ascii_case("Implementation-Version") {
            let val = line[colon + 1..].trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_multiple_mod_blocks_are_loader_visible() {
        let ids = extract_forge_mod_ids(
            "[[mods]]\nmodId=\"primary\"\n[[mods]]\nmodId=\"provided\"\n[[dependencies.primary]]\nmodId=\"dep\"\n",
        );
        assert_eq!(ids.into_iter().collect::<Vec<_>>(), ["primary", "provided"]);
    }

    /// Build an in-memory `.jar` (zip) with the given `(entry_name, content)`
    /// pairs and write it to a unique temp file, returning the path.
    fn build_test_jar(entries: &[(&str, &str)]) -> std::path::PathBuf {
        use std::io::{Seek, Write};
        let mut file = tempfile::NamedTempFile::new().expect("create temp file");
        {
            let mut zip = zip::ZipWriter::new(&file);
            let opts = zip::write::FileOptions::default();
            for (name, content) in entries {
                zip.start_file(*name, opts).expect("start_file");
                zip.write_all(content.as_bytes()).expect("write_all");
            }
            zip.finish().expect("finish zip");
        }
        file.seek(std::io::SeekFrom::Start(0)).expect("rewind");
        let (_file, path) = file.keep().expect("keep temp file");
        path
    }

    /// Build an in-memory `.jar` (zip) with binary entries and write to a
    /// unique temp file, returning the path.
    fn build_test_jar_binary(entries: &[(&str, &[u8])]) -> std::path::PathBuf {
        use std::io::{Seek, Write};
        let mut file = tempfile::NamedTempFile::new().expect("create temp file");
        {
            let mut zip = zip::ZipWriter::new(&file);
            let opts = zip::write::FileOptions::default();
            for (name, content) in entries {
                zip.start_file(*name, opts).expect("start_file");
                zip.write_all(content).expect("write_all");
            }
            zip.finish().expect("finish zip");
        }
        file.seek(std::io::SeekFrom::Start(0)).expect("rewind");
        let (_file, path) = file.keep().expect("keep temp file");
        path
    }

    /// Build a JAR (zip) entirely in memory and return its raw bytes.
    fn build_jar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::FileOptions::default();
            for (name, content) in entries {
                zip.start_file(*name, opts).expect("start_file");
                zip.write_all(content).expect("write_all");
            }
            zip.finish().expect("finish zip");
        }
        buf.into_inner()
    }

    #[test]
    fn parse_jar_metadata_missing_file_returns_default() {
        let meta = parse_jar_metadata(std::path::Path::new("/nonexistent/jar.jar"));
        assert!(meta.java_packages.is_empty());
        assert!(meta.mod_jar_id.is_none());
        assert!(meta.depends_on.is_empty());
        assert!(meta.incompatibility_decls.is_empty());
        assert!(meta.provided_mods.is_empty());
    }

    // -------------------------------------------------------------------
    // Fabric provides alias tests
    // -------------------------------------------------------------------

    #[test]
    fn fabric_provides_string_aliases_captured_with_outer_version() {
        let jar = build_test_jar(&[(
            "fabric.mod.json",
            r#"{"id":"a","version":"1.0","provides":["b","c"]}"#,
        )]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);

        assert_eq!(meta.mod_jar_id.as_deref(), Some("a"));
        assert_eq!(meta.mod_version.as_deref(), Some("1.0"));
        // Both aliases with the outer version.
        let b = meta
            .provided_mods
            .iter()
            .find(|p| p.mod_id == "b")
            .expect("b should be provided");
        assert_eq!(b.version.as_deref(), Some("1.0"));
        let c = meta
            .provided_mods
            .iter()
            .find(|p| p.mod_id == "c")
            .expect("c should be provided");
        assert_eq!(c.version.as_deref(), Some("1.0"));
    }

    // -------------------------------------------------------------------
    // Quilt provides tests
    // -------------------------------------------------------------------

    #[test]
    fn quilt_provides_string_and_object_forms() {
        let jar = build_test_jar(&[(
            "quilt.mod.json",
            r#"{"quilt_loader":{"id":"q","version":"2.0","provides":["a",{"id":"b","version":"3.0"},{"id":"c"}]}}"#,
        )]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);

        assert_eq!(meta.mod_jar_id.as_deref(), Some("q"));
        // 'a' is a plain string → defaults to outer version.
        let a = meta
            .provided_mods
            .iter()
            .find(|p| p.mod_id == "a")
            .expect("a should be provided");
        assert_eq!(a.version.as_deref(), Some("2.0"));
        // 'b' has explicit version.
        let b = meta
            .provided_mods
            .iter()
            .find(|p| p.mod_id == "b")
            .expect("b should be provided");
        assert_eq!(b.version.as_deref(), Some("3.0"));
        // 'c' has no explicit version → defaults to outer version.
        let c = meta
            .provided_mods
            .iter()
            .find(|p| p.mod_id == "c")
            .expect("c should be provided");
        assert_eq!(c.version.as_deref(), Some("2.0"));
    }

    // -------------------------------------------------------------------
    // Nested JAR tests
    // -------------------------------------------------------------------

    #[test]
    fn fabric_nested_jar_primary_id_and_deps_aggregate() {
        // Build inner JAR bytes.
        let inner_bytes = build_jar_bytes(&[(
            "fabric.mod.json",
            br#"{"id":"inner","version":"2.0","depends":{"req":"1.0"}}"# as &[u8],
        )]);

        let jar = build_test_jar_binary(&[
            (
                "fabric.mod.json",
                br#"{"id":"outer","version":"1.0","jars":[{"file":"nested.jar"}]}"# as &[u8],
            ),
            ("nested.jar", &inner_bytes),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);

        assert_eq!(meta.mod_jar_id.as_deref(), Some("outer"));
        // Nested primary ID appears in provided_mods.
        let inner = meta
            .provided_mods
            .iter()
            .find(|p| p.mod_id == "inner")
            .expect("inner should be provided");
        assert_eq!(inner.version.as_deref(), Some("2.0"));
        // Nested dep "req" should be in depends_on.
        assert!(meta.depends_on.contains(&"req".to_string()));
    }

    #[test]
    fn fabric_nested_jar_intra_jar_dep_removed() {
        // Outer depends on "inner" but inner supplies that ID.
        let inner_bytes = build_jar_bytes(&[(
            "fabric.mod.json",
            br#"{"id":"inner","version":"1.0"}"# as &[u8],
        )]);

        let jar = build_test_jar_binary(&[
            (
                "fabric.mod.json",
                br#"{"id":"outer","version":"1.0","depends":{"inner":"1.0"},"jars":[{"file":"nested.jar"}]}"#
                    as &[u8],
            ),
            ("nested.jar", &inner_bytes),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);

        // "inner" is supplied by the same physical JAR (via the nested JAR),
        // so it should NOT appear in depends_on.
        assert!(
            !meta.depends_on.contains(&"inner".to_string()),
            "intra-JAR dep must be filtered out"
        );
    }

    #[test]
    fn nested_jar_provided_mods_propagate() {
        // Inner JAR has its own provides aliases.
        let inner_bytes = build_jar_bytes(&[(
            "fabric.mod.json",
            br#"{"id":"core","version":"2.0","provides":["core_api","core_utils"]}"# as &[u8],
        )]);

        let jar = build_test_jar_binary(&[
            (
                "fabric.mod.json",
                br#"{"id":"wrapper","version":"1.0","jars":[{"file":"inner.jar"}]}"# as &[u8],
            ),
            ("inner.jar", &inner_bytes),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);

        assert_eq!(meta.mod_jar_id.as_deref(), Some("wrapper"));
        // Inner's primary ID propagates.
        assert!(meta.provided_mods.iter().any(|p| p.mod_id == "core"));
        // Inner's provides aliases propagate.
        assert!(meta.provided_mods.iter().any(|p| p.mod_id == "core_api"));
        assert!(meta.provided_mods.iter().any(|p| p.mod_id == "core_utils"));
    }

    #[test]
    fn quilt_string_jars_path_works() {
        let inner_bytes = build_jar_bytes(&[(
            "quilt.mod.json",
            br#"{"quilt_loader":{"id":"qchild","version":"1.5"}}"# as &[u8],
        )]);

        let jar = build_test_jar_binary(&[
            (
                "quilt.mod.json",
                br#"{"quilt_loader":{"id":"qparent","version":"1.0","jars":["child.jar"]}}"#
                    as &[u8],
            ),
            ("child.jar", &inner_bytes),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);

        assert_eq!(meta.mod_jar_id.as_deref(), Some("qparent"));
        assert!(meta.provided_mods.iter().any(|p| p.mod_id == "qchild"));
        let qc = meta
            .provided_mods
            .iter()
            .find(|p| p.mod_id == "qchild")
            .expect("qchild should be provided");
        assert_eq!(qc.version.as_deref(), Some("1.5"));
    }

    // -------------------------------------------------------------------
    // Path security tests
    // -------------------------------------------------------------------

    #[test]
    fn undeclared_embedded_jar_ignored() {
        // A .jar entry exists but is NOT declared in `jars` → must be ignored.
        let undeclared_bytes = build_jar_bytes(&[(
            "fabric.mod.json",
            br#"{"id":"sneaky","version":"9.9"}"# as &[u8],
        )]);

        let jar = build_test_jar_binary(&[
            (
                "fabric.mod.json",
                br#"{"id":"main","version":"1.0"}"# as &[u8],
            ),
            ("hidden.jar", &undeclared_bytes),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);

        assert_eq!(meta.mod_jar_id.as_deref(), Some("main"));
        assert!(
            !meta.provided_mods.iter().any(|p| p.mod_id == "sneaky"),
            "undeclared nested JAR must not be parsed"
        );
    }

    #[test]
    fn unsafe_dotdot_nested_path_ignored() {
        let resolved_bytes = build_jar_bytes(&[(
            "fabric.mod.json",
            br#"{"id":"resolved","version":"1.0"}"# as &[u8],
        )]);

        let jar = build_test_jar_binary(&[
            (
                "fabric.mod.json",
                br#"{"id":"main","version":"1.0","jars":[{"file":"../nested.jar"}]}"# as &[u8],
            ),
            // The entry exists at the unsafe path — it should still be rejected.
            ("../nested.jar", &resolved_bytes),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);

        assert_eq!(meta.mod_jar_id.as_deref(), Some("main"));
        assert!(
            !meta.provided_mods.iter().any(|p| p.mod_id == "resolved"),
            "path with '..' segment must be rejected"
        );
    }

    // -------------------------------------------------------------------
    // Depth guard test
    // -------------------------------------------------------------------

    #[test]
    fn nested_depth_greater_than_4_terminates() {
        // Build a chain: level0.jar -> level1.jar -> level2.jar -> level3.jar
        // -> level4.jar -> level5.jar (exceeds max depth 4).
        // Only levels 0-4 should be exposed.
        let level5_bytes = build_jar_bytes(&[(
            "fabric.mod.json",
            br#"{"id":"level5","version":"5.0"}"# as &[u8],
        )]);
        let level4_bytes = build_jar_bytes(&[
            (
                "fabric.mod.json",
                br#"{"id":"level4","version":"4.0","jars":[{"file":"dirty.jar"}]}"# as &[u8],
            ),
            ("dirty.jar", &level5_bytes),
        ]);
        let level3_bytes = build_jar_bytes(&[
            (
                "fabric.mod.json",
                br#"{"id":"level3","version":"3.0","jars":[{"file":"l4.jar"}]}"# as &[u8],
            ),
            ("l4.jar", &level4_bytes),
        ]);
        let level2_bytes = build_jar_bytes(&[
            (
                "fabric.mod.json",
                br#"{"id":"level2","version":"2.0","jars":[{"file":"l3.jar"}]}"# as &[u8],
            ),
            ("l3.jar", &level3_bytes),
        ]);
        let level1_bytes = build_jar_bytes(&[
            (
                "fabric.mod.json",
                br#"{"id":"level1","version":"1.0","jars":[{"file":"l2.jar"}]}"# as &[u8],
            ),
            ("l2.jar", &level2_bytes),
        ]);

        let jar = build_test_jar_binary(&[
            (
                "fabric.mod.json",
                br#"{"id":"level0","version":"0.0","jars":[{"file":"l1.jar"}]}"# as &[u8],
            ),
            ("l1.jar", &level1_bytes),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);

        assert_eq!(meta.mod_jar_id.as_deref(), Some("level0"));
        // Levels 1-4 should be visible.
        assert!(meta.provided_mods.iter().any(|p| p.mod_id == "level1"));
        assert!(meta.provided_mods.iter().any(|p| p.mod_id == "level2"));
        assert!(meta.provided_mods.iter().any(|p| p.mod_id == "level3"));
        assert!(meta.provided_mods.iter().any(|p| p.mod_id == "level4"));
        // Level 5 exceeds max depth and must NOT appear.
        assert!(
            !meta.provided_mods.iter().any(|p| p.mod_id == "level5"),
            "depth >4 must not expose deepest ID"
        );
    }

    // -------------------------------------------------------------------
    // JarDeps::default serde test for provided_mods
    // -------------------------------------------------------------------

    #[test]
    fn jar_deps_default_serde_roundtrip() {
        // Default should serialize and deserialize correctly with provided_mods.
        let default = JarDeps::default();
        let json = serde_json::to_string(&default).expect("serialize");
        let restored: JarDeps = serde_json::from_str(&json).expect("deserialize");
        assert!(restored.provided_mods.is_empty());
        assert!(restored.mod_jar_id.is_none());
        assert!(restored.depends_on.is_empty());
    }

    #[test]
    fn jar_deps_deserialize_missing_provided_mods_defaults_empty() {
        // JSON without provided_mods field should work (serde default).
        let json = r#"{"java_packages":[],"mod_jar_id":null,"depends_on":[],"optional_deps":[],"incompatible_deps":[],"incompatibility_decls":[],"mod_version":null}"#;
        let restored: JarDeps = serde_json::from_str(json).expect("deserialize");
        assert!(restored.provided_mods.is_empty());
    }

    // -------------------------------------------------------------------
    // Existing tests below — kept unchanged
    // -------------------------------------------------------------------

    #[test]
    fn extract_fabric_deps_object_form() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"fabric-api": ">=0.40.0", "minecraft": ">=1.20"}"#).unwrap();
        let mut out = BTreeSet::new();
        extract_fabric_deps(&v, &mut out, None);
        assert!(out.contains("fabric-api"));
        assert!(out.contains("minecraft"));
    }

    #[test]
    fn extract_fabric_deps_array_form() {
        let v: serde_json::Value =
            serde_json::from_str(r#"[{"id": "sodium"}, {"identifier": "lithium"}]"#).unwrap();
        let mut out = BTreeSet::new();
        extract_fabric_deps(&v, &mut out, None);
        assert!(out.contains("sodium"));
        assert!(out.contains("lithium"));
    }

    #[test]
    fn fabric_breaks_captured_as_hard_with_predicate() {
        let v: serde_json::Value = serde_json::from_str(r#"{"optifine": "<2.0"}"#).unwrap();
        let mut ids = BTreeSet::new();
        let mut decls = Vec::new();
        extract_fabric_deps(
            &v,
            &mut ids,
            Some((IncompatibilitySource::FabricBreaks, &mut decls)),
        );
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].mod_id, "optifine");
        assert_eq!(decls[0].version_ranges, vec!["<2.0".to_string()]);
        assert_eq!(decls[0].source, IncompatibilitySource::FabricBreaks);
        assert!(decls[0].source.is_hard());
    }

    #[test]
    fn fabric_conflicts_is_soft() {
        let v: serde_json::Value = serde_json::from_str(r#"{"foo": "*"}"#).unwrap();
        let mut decls = Vec::new();
        let mut ids = BTreeSet::new();
        extract_fabric_deps(
            &v,
            &mut ids,
            Some((IncompatibilitySource::FabricConflicts, &mut decls)),
        );
        assert_eq!(decls.len(), 1);
        assert!(!decls[0].source.is_hard());
        assert!(decls[0].version_ranges.is_empty());
    }

    #[test]
    fn fabric_array_predicates_captured_as_or() {
        let v: serde_json::Value = serde_json::from_str(r#"{"foo": ["<2.0", ">=3.0"]}"#).unwrap();
        let mut decls = Vec::new();
        let mut ids = BTreeSet::new();
        extract_fabric_deps(
            &v,
            &mut ids,
            Some((IncompatibilitySource::FabricBreaks, &mut decls)),
        );
        assert_eq!(decls[0].version_ranges, vec!["<2.0", ">=3.0"]);
    }

    #[test]
    fn fabric_array_of_objects_form() {
        let v: serde_json::Value =
            serde_json::from_str(r#"[{"id":"foo","version":"<2.0"},{"id":"bar"}]"#).unwrap();
        let mut decls = Vec::new();
        let mut ids = BTreeSet::new();
        extract_fabric_deps(
            &v,
            &mut ids,
            Some((IncompatibilitySource::FabricBreaks, &mut decls)),
        );
        let foo = decls.iter().find(|d| d.mod_id == "foo").unwrap();
        assert_eq!(foo.version_ranges, vec!["<2.0"]);
        let bar = decls.iter().find(|d| d.mod_id == "bar").unwrap();
        assert!(bar.version_ranges.is_empty());
    }

    // -------------------------------------------------------------------
    // Forge/NeoForge parsing
    // -------------------------------------------------------------------

    #[test]
    fn forge_dep_block_reads_inner_modid_not_owner() {
        let toml = r#"modId="mymod"
version="1.0"

[[dependencies.mymod]]
    modId="fabric-api"
    type="required"

[[dependencies.mymod]]
    modId="sodium"
    type="optional"
"#;
        let mut required = BTreeSet::new();
        let mut optional = BTreeSet::new();
        let mut incompat_ids = BTreeSet::new();
        let mut decls = Vec::new();
        let mut mod_id = None;
        let mut mod_version = None;
        extract_forge_deps(
            toml,
            &mut required,
            &mut optional,
            &mut incompat_ids,
            &mut decls,
            &mut mod_id,
            &mut mod_version,
        );
        assert_eq!(mod_id, Some("mymod".to_string()));
        assert!(required.contains("fabric-api"));
        assert!(!required.contains("mymod"), "owner must NOT be its own dep");
        assert!(optional.contains("sodium"));
    }

    #[test]
    fn forge_mandatory_false_is_optional() {
        let toml = r#"[[dependencies.foo]]
    modId="bar"
    mandatory=false
"#;
        let mut required = BTreeSet::new();
        let mut optional = BTreeSet::new();
        let mut incompat_ids = BTreeSet::new();
        let mut decls = Vec::new();
        let mut mod_id = None;
        let mut mod_version = None;
        extract_forge_deps(
            toml,
            &mut required,
            &mut optional,
            &mut incompat_ids,
            &mut decls,
            &mut mod_id,
            &mut mod_version,
        );
        assert!(optional.contains("bar"));
        assert!(!required.contains("bar"));
    }

    #[test]
    fn forge_mandatory_true_is_required() {
        let toml = r#"[[dependencies.foo]]
    modId="bar"
    mandatory=true
"#;
        let mut required = BTreeSet::new();
        let mut optional = BTreeSet::new();
        let mut incompat_ids = BTreeSet::new();
        let mut decls = Vec::new();
        let mut mod_id = None;
        let mut mod_version = None;
        extract_forge_deps(
            toml,
            &mut required,
            &mut optional,
            &mut incompat_ids,
            &mut decls,
            &mut mod_id,
            &mut mod_version,
        );
        assert!(required.contains("bar"));
    }

    #[test]
    fn forge_incompatible_uses_target_id_and_captures_version_range() {
        let toml = r#"[[dependencies.mymod]]
    modId="optifine"
    type="incompatible"
    versionRange="[1.0,2.0)"
"#;
        let mut required = BTreeSet::new();
        let mut optional = BTreeSet::new();
        let mut incompat_ids = BTreeSet::new();
        let mut decls = Vec::new();
        let mut mod_id = None;
        let mut mod_version = None;
        extract_forge_deps(
            toml,
            &mut required,
            &mut optional,
            &mut incompat_ids,
            &mut decls,
            &mut mod_id,
            &mut mod_version,
        );
        assert!(incompat_ids.contains("optifine"));
        assert!(
            !incompat_ids.contains("mymod"),
            "owner must not self-conflict"
        );
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].mod_id, "optifine");
        assert_eq!(decls[0].version_ranges, vec!["[1.0,2.0)".to_string()]);
        assert_eq!(decls[0].source, IncompatibilitySource::ForgeIncompatible);
    }

    #[test]
    fn forge_discouraged_is_soft() {
        let toml = r#"[[dependencies.mymod]]
    modId="bar"
    type="discouraged"
"#;
        let mut required = BTreeSet::new();
        let mut optional = BTreeSet::new();
        let mut incompat_ids = BTreeSet::new();
        let mut decls = Vec::new();
        let mut mod_id = None;
        let mut mod_version = None;
        extract_forge_deps(
            toml,
            &mut required,
            &mut optional,
            &mut incompat_ids,
            &mut decls,
            &mut mod_id,
            &mut mod_version,
        );
        assert_eq!(decls.len(), 1);
        assert!(!decls[0].source.is_hard());
    }

    #[test]
    fn forge_empty_version_range_is_unconditional() {
        let toml = r#"[[dependencies.mymod]]
    modId="bar"
    type="incompatible"
    versionRange=""
"#;
        let mut required = BTreeSet::new();
        let mut optional = BTreeSet::new();
        let mut incompat_ids = BTreeSet::new();
        let mut decls = Vec::new();
        let mut mod_id = None;
        let mut mod_version = None;
        extract_forge_deps(
            toml,
            &mut required,
            &mut optional,
            &mut incompat_ids,
            &mut decls,
            &mut mod_id,
            &mut mod_version,
        );
        assert!(decls[0].version_ranges.is_empty());
    }

    #[test]
    fn forge_dep_block_without_modid_is_skipped() {
        let toml = r#"[[dependencies.someowner]]
    type="required"
"#;
        let mut required = BTreeSet::new();
        let mut optional = BTreeSet::new();
        let mut incompat_ids = BTreeSet::new();
        let mut decls = Vec::new();
        let mut mod_id = None;
        let mut mod_version = None;
        extract_forge_deps(
            toml,
            &mut required,
            &mut optional,
            &mut incompat_ids,
            &mut decls,
            &mut mod_id,
            &mut mod_version,
        );
        assert!(required.is_empty());
        assert!(incompat_ids.is_empty());
        assert!(decls.is_empty());
    }

    // -------------------------------------------------------------------
    // Full JAR parse (zip fixture) end-to-end
    // -------------------------------------------------------------------

    #[test]
    fn parse_jar_fabric_breaks_and_conflicts() {
        let jar = build_test_jar(&[(
            "fabric.mod.json",
            r#"{"id":"mod_a","breaks":{"bad":"<2.0"},"conflicts":{"iffy":"*"}, "depends":{"fabric-api":">=0.40"}}"#,
        )]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);
        assert_eq!(meta.mod_jar_id.as_deref(), Some("mod_a"));
        assert!(meta.depends_on.contains(&"fabric-api".to_string()));
        assert!(meta.incompatible_deps.contains(&"bad".to_string()));
        assert!(meta.incompatible_deps.contains(&"iffy".to_string()));
        let bad = meta
            .incompatibility_decls
            .iter()
            .find(|d| d.mod_id == "bad")
            .unwrap();
        assert_eq!(bad.source, IncompatibilitySource::FabricBreaks);
        assert_eq!(bad.version_ranges, vec!["<2.0".to_string()]);
        let iffy = meta
            .incompatibility_decls
            .iter()
            .find(|d| d.mod_id == "iffy")
            .unwrap();
        assert_eq!(iffy.source, IncompatibilitySource::FabricConflicts);
        assert!(iffy.version_ranges.is_empty());
    }

    #[test]
    fn parse_jar_neoforge_mods_toml_parsed_and_inner_modid_read() {
        let jar = build_test_jar(&[
            (
                "META-INF/neoforge.mods.toml",
                "modId=\"neomod\"\n\n[[dependencies.neomod]]\n    modId=\"optifine\"\n    type=\"incompatible\"\n",
            ),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);
        assert_eq!(meta.mod_jar_id.as_deref(), Some("neomod"));
        assert!(meta.incompatible_deps.contains(&"optifine".to_string()));
        assert!(
            !meta.incompatible_deps.contains(&"neomod".to_string()),
            "owner must not self-conflict"
        );
        let optifine = meta
            .incompatibility_decls
            .iter()
            .find(|d| d.mod_id == "optifine")
            .expect("optifine decl");
        assert_eq!(optifine.source, IncompatibilitySource::ForgeIncompatible);
    }

    #[test]
    fn parse_jar_forge_self_conflict_bug_fixed() {
        let jar = build_test_jar(&[
            (
                "META-INF/mods.toml",
                "modId=\"examplemod\"\n[[dependencies.examplemod]]\n    modId=\"othermod\"\n    type=\"incompatible\"\n",
            ),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);
        assert!(meta.incompatible_deps.contains(&"othermod".to_string()));
        assert!(
            !meta.incompatible_deps.contains(&"examplemod".to_string()),
            "examplemod must not appear incompatible with itself"
        );
    }

    #[test]
    fn parse_jar_forge_neoforge_loader_deps_ignored() {
        let jar = build_test_jar(&[
            (
                "META-INF/neoforge.mods.toml",
                "modId=\"m\"\n[[dependencies.m]]\n    modId=\"neoforge\"\n    type=\"required\"\n[[dependencies.m]]\n    modId=\"realdep\"\n    type=\"required\"\n",
            ),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);
        assert!(!meta.depends_on.contains(&"neoforge".to_string()));
        assert!(meta.depends_on.contains(&"realdep".to_string()));
    }

    #[test]
    fn parse_fabric_version_is_extracted() {
        let jar = build_test_jar(&[("fabric.mod.json", r#"{"id":"a","version":"1.2.3-build.4"}"#)]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);
        assert_eq!(meta.mod_version.as_deref(), Some("1.2.3-build.4"));
    }

    #[test]
    fn parse_forge_version_placeholder_resolves_to_manifest() {
        let jar = build_test_jar(&[
            (
                "META-INF/mods.toml",
                "modId=\"m\"\nversion=\"${file.jarVersion}\"\n",
            ),
            (
                "META-INF/MANIFEST.MF",
                "Manifest-Version: 1.0\nImplementation-Version: 3.7.2\n",
            ),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);
        assert_eq!(meta.mod_version.as_deref(), Some("3.7.2"));
    }

    #[test]
    fn parse_forge_explicit_version_used_directly() {
        let jar = build_test_jar(&[
            ("META-INF/mods.toml", "modId=\"m\"\nversion=\"2.1.0\"\n"),
            (
                "META-INF/MANIFEST.MF",
                "Manifest-Version: 1.0\nImplementation-Version: 9.9.9\n",
            ),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);
        assert_eq!(meta.mod_version.as_deref(), Some("2.1.0"));
    }

    #[test]
    fn parse_forge_no_version_falls_back_to_manifest_only() {
        let jar = build_test_jar(&[
            ("META-INF/mods.toml", "modId=\"m\"\n"),
            (
                "META-INF/MANIFEST.MF",
                "Manifest-Version: 1.0\nImplementation-Version: 0.4.1\n",
            ),
        ]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);
        assert_eq!(meta.mod_version.as_deref(), Some("0.4.1"));
    }

    #[test]
    fn parse_quilt_mod_id_and_version_extracted() {
        let jar = build_test_jar(&[(
            "quilt.mod.json",
            r#"{"quilt_loader":{"id":"qmod","version":"5.6.7"}}"#,
        )]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);
        assert_eq!(meta.mod_jar_id.as_deref(), Some("qmod"));
        assert_eq!(meta.mod_version.as_deref(), Some("5.6.7"));
    }

    #[test]
    fn parse_quilt_breaks_and_conflicts_captured() {
        let jar = build_test_jar(&[(
            "quilt.mod.json",
            r#"{"quilt_loader":{"id":"qmod","version":"1.0","breaks":{"target_a":"<2.0"},"conflicts":{"target_b":"*"}}}"#,
        )]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);
        assert!(meta.incompatible_deps.contains(&"target_a".to_string()));
        assert!(meta.incompatible_deps.contains(&"target_b".to_string()));
        let a_decl = meta
            .incompatibility_decls
            .iter()
            .find(|d| d.mod_id == "target_a")
            .expect("expected target_a decl");
        assert_eq!(a_decl.source, IncompatibilitySource::QuiltBreaks);
        let b_decl = meta
            .incompatibility_decls
            .iter()
            .find(|d| d.mod_id == "target_b")
            .expect("expected target_b decl");
        assert_eq!(b_decl.source, IncompatibilitySource::QuiltConflicts);
        assert!(a_decl.source.is_hard());
        assert!(!b_decl.source.is_hard());
        assert!(a_decl.source.is_fabric_grammar());
        assert!(b_decl.source.is_fabric_grammar());
    }

    #[test]
    fn parse_quilt_depends_collected_as_required() {
        let jar = build_test_jar(&[(
            "quilt.mod.json",
            r#"{"quilt_loader":{"id":"qmod","version":"1.0","depends":{"needed_dep":">=1.0"}}}"#,
        )]);
        let meta = parse_jar_metadata(&jar);
        let _ = std::fs::remove_file(&jar);
        assert!(meta.depends_on.contains(&"needed_dep".to_string()));
    }

    #[test]
    fn active_loader_metadata_does_not_merge_other_loader_dependencies() {
        let jar = build_test_jar(&[
            (
                "fabric.mod.json",
                r#"{"id":"swingthrough","version":"1.0"}"#,
            ),
            (
                "META-INF/neoforge.mods.toml",
                "modId=\"swingthrough\"\n[[dependencies.swingthrough]]\nmodId=\"connector\"\ntype=\"required\"\n",
            ),
        ]);

        let fabric = parse_jar_metadata_for_loader_with_status(&jar, "fabric");
        let neoforge = parse_jar_metadata_for_loader_with_status(&jar, "neoforge");
        let _ = std::fs::remove_file(&jar);

        assert!(fabric.has_native_metadata);
        assert_eq!(fabric.metadata.mod_jar_id.as_deref(), Some("swingthrough"));
        assert!(!fabric
            .metadata
            .depends_on
            .contains(&"connector".to_string()));
        assert!(neoforge.has_native_metadata);
        assert!(neoforge
            .metadata
            .depends_on
            .contains(&"connector".to_string()));
    }

    #[test]
    fn quilt_prefers_quilt_metadata_over_fabric_metadata() {
        let jar = build_test_jar(&[
            (
                "fabric.mod.json",
                r#"{"id":"fabric-id","version":"1.0","depends":{"fabric-only":"*"}}"#,
            ),
            (
                "quilt.mod.json",
                r#"{"quilt_loader":{"id":"quilt-id","version":"2.0","depends":{"quilt-only":"*"}}}"#,
            ),
        ]);

        let metadata = parse_jar_metadata_for_loader(&jar, "quilt");
        let _ = std::fs::remove_file(&jar);

        assert_eq!(metadata.mod_jar_id.as_deref(), Some("quilt-id"));
        assert!(metadata.depends_on.contains(&"quilt-only".to_string()));
        assert!(!metadata.depends_on.contains(&"fabric-only".to_string()));
    }

    #[test]
    fn neoforge_jarjar_metadata_exposes_nested_capability() {
        let nested = build_jar_bytes(&[(
            "META-INF/neoforge.mods.toml",
            br#"modId="statuemenus"
version="1.0.0"
"#,
        )]);
        let jar = build_test_jar_binary(&[
            (
                "META-INF/neoforge.mods.toml",
                br#"modId="armorstatues"
version="1.0.0"
[[dependencies.armorstatues]]
modId="statuemenus"
type="required"
"#,
            ),
            (
                "META-INF/jarjar/metadata.json",
                br#"{"jars":[{"identifier":{"group":"example","artifact":"statuemenus"},"version":{"range":"[1,2)","artifactVersion":"1.0.0"},"path":"META-INF/jarjar/statuemenus.jar"}]}"#,
            ),
            ("META-INF/jarjar/statuemenus.jar", &nested),
        ]);

        let parsed = parse_jar_metadata_for_loader_with_status(&jar, "neoforge");
        let _ = std::fs::remove_file(&jar);

        assert_eq!(parsed.status, ParseStatus::Complete);
        assert_eq!(parsed.metadata.mod_jar_id.as_deref(), Some("armorstatues"));
        assert!(parsed
            .metadata
            .provided_mods
            .iter()
            .any(|provided| provided.mod_id == "statuemenus"));
        assert!(!parsed
            .metadata
            .depends_on
            .iter()
            .any(|dependency| dependency == "statuemenus"));
    }

    #[test]
    fn malformed_metadata_is_partial_and_valid_metadata_free_jar_is_complete() {
        let malformed = build_test_jar(&[("fabric.mod.json", "{not-json")]);
        let malformed_result = parse_jar_metadata_result(&malformed);
        let _ = std::fs::remove_file(&malformed);
        assert_eq!(malformed_result.status, ParseStatus::Partial);
        assert!(malformed_result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == ParseDiagnosticKind::MetadataMalformed));

        let empty = build_test_jar(&[("assets/example/test.txt", "content")]);
        let empty_result = parse_jar_metadata_result(&empty);
        let _ = std::fs::remove_file(&empty);
        assert_eq!(empty_result.status, ParseStatus::Complete);
        assert!(empty_result.diagnostics.is_empty());
    }

    #[test]
    fn malformed_archive_is_failed_not_metadata_free() {
        let result = parse_jar_metadata_bytes_for_loader(b"not-a-zip", "fabric");
        assert_eq!(result.status, ParseStatus::Failed);
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == ParseDiagnosticKind::ArchiveOpenFailed));
    }
}
