//! Canonical, privacy-preserving instance lockfiles and drift comparison.

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const LOCKFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstanceLockfile {
    pub schema_version: u32,
    pub created_at: String,
    pub instance: LockedInstance,
    pub artifacts: Vec<LockedArtifact>,
    pub loader: LockedLoader,
    pub manifest_sha256: String,
    pub config_policy: LockfileConfigPolicy,
    pub content_hash: String,
    #[serde(default)]
    pub signature: Option<LockfileSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LockedInstance {
    pub name: String,
    pub minecraft_version: String,
    pub loader: String,
    pub loader_version: String,
    pub is_locked: bool,
    #[serde(default)]
    pub user_preferences: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockedLoader {
    pub source_url: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockedArtifact {
    pub filename: String,
    pub content_type: String,
    pub registry_id: Option<String>,
    pub modrinth_id: Option<String>,
    pub source: String,
    pub source_url: Option<String>,
    pub version: Option<String>,
    pub sha256: String,
    pub enabled: bool,
    #[serde(default)]
    pub unresolved_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockfileConfigPolicy {
    pub included: bool,
    pub config_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockfileSignature {
    pub algorithm: String,
    pub public_key: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DriftReport {
    pub status: DriftStatus,
    pub differences: Vec<DriftDifference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DriftStatus {
    InSync,
    Drifted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DriftDifference {
    pub path: String,
    pub kind: DriftKind,
    pub expected_sha256: Option<String>,
    pub actual_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DriftKind {
    Added,
    Removed,
    Modified,
    Enabled,
    Disabled,
    ConfigModified,
}

impl InstanceLockfile {
    pub fn new(
        instance: LockedInstance,
        mut artifacts: Vec<LockedArtifact>,
        loader: LockedLoader,
        manifest_sha256: String,
        config_hash: Option<String>,
    ) -> Result<Self, String> {
        artifacts.sort_by(|a, b| {
            a.content_type
                .cmp(&b.content_type)
                .then(a.filename.cmp(&b.filename))
        });
        let mut lockfile = Self {
            schema_version: LOCKFILE_SCHEMA_VERSION,
            created_at: chrono::Utc::now().to_rfc3339(),
            instance,
            artifacts,
            loader,
            manifest_sha256,
            config_policy: LockfileConfigPolicy {
                included: false,
                config_hash,
            },
            content_hash: String::new(),
            signature: None,
        };
        lockfile.content_hash = lockfile.recompute_content_hash()?;
        Ok(lockfile)
    }

    pub fn parse_and_validate(json: &str) -> Result<Self, String> {
        let lockfile: Self = serde_json::from_str(json)
            .map_err(|error| format!("Invalid lockfile JSON: {error}"))?;
        lockfile.validate()?;
        Ok(lockfile)
    }

    pub fn validate(&self) -> Result<(), String> {
        use std::collections::BTreeSet;
        if self.schema_version != LOCKFILE_SCHEMA_VERSION {
            return Err(format!(
                "Unsupported lockfile schema version {} (supported: {}).",
                self.schema_version, LOCKFILE_SCHEMA_VERSION
            ));
        }
        if self.instance.name.trim().is_empty()
            || self.instance.minecraft_version.trim().is_empty()
            || self.instance.loader.trim().is_empty()
            || (!matches!(self.instance.loader.as_str(), "vanilla")
                && self.instance.loader_version.trim().is_empty())
        {
            return Err("Lockfile instance identity is incomplete.".into());
        }
        if self.config_policy.included {
            return Err("Lockfiles must not include private configuration contents.".into());
        }
        let mut paths = BTreeSet::new();
        for artifact in &self.artifacts {
            validate_artifact(artifact)?;
            let path = artifact_path(artifact);
            if !paths.insert(path.clone()) {
                return Err(format!("Lockfile contains duplicate artifact path: {path}"));
            }
        }
        let computed = self.recompute_content_hash()?;
        if !computed.eq_ignore_ascii_case(&self.content_hash) {
            return Err(format!(
                "Lockfile content hash mismatch: expected {}, computed {}.",
                self.content_hash, computed
            ));
        }
        if let Some(signature) = &self.signature {
            verify_signature(&self.content_hash, signature)?;
        }
        Ok(())
    }

    /// Hashes a canonical representation with `contentHash` blank and the
    /// optional signature removed. The hash therefore never contains itself.
    pub fn recompute_content_hash(&self) -> Result<String, String> {
        let mut unsigned = self.clone();
        unsigned.content_hash.clear();
        unsigned.signature = None;
        let value = serde_json::to_value(unsigned)
            .map_err(|error| format!("Could not canonicalize lockfile: {error}"))?;
        let canonical = canonical_json(&value)?;
        Ok(hex::encode(Sha256::digest(canonical.as_bytes())))
    }

    pub fn to_pretty_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|error| format!("Could not serialize lockfile: {error}"))
    }
}

pub fn detect_drift(
    lockfile: &InstanceLockfile,
    live_files: &BTreeMap<String, String>,
    live_config_hash: Option<&str>,
) -> DriftReport {
    let mut expected = BTreeMap::new();
    for artifact in &lockfile.artifacts {
        expected.insert(
            artifact_path(artifact),
            artifact.sha256.to_ascii_lowercase(),
        );
    }

    let mut differences = Vec::new();
    let mut status_counterparts = std::collections::BTreeSet::new();
    for (path, hash) in &expected {
        match live_files.get(path) {
            None => {
                let (alternate, kind) = if let Some(enabled_path) = path.strip_suffix(".disabled") {
                    (enabled_path.to_string(), DriftKind::Enabled)
                } else {
                    (format!("{path}.disabled"), DriftKind::Disabled)
                };
                if let Some(actual) = live_files.get(&alternate) {
                    status_counterparts.insert(alternate);
                    differences.push(DriftDifference {
                        path: path.clone(),
                        kind,
                        expected_sha256: Some(hash.clone()),
                        actual_sha256: Some(actual.clone()),
                    });
                } else {
                    differences.push(DriftDifference {
                        path: path.clone(),
                        kind: DriftKind::Removed,
                        expected_sha256: Some(hash.clone()),
                        actual_sha256: None,
                    });
                }
            }
            Some(actual) if !actual.eq_ignore_ascii_case(hash) => {
                differences.push(DriftDifference {
                    path: path.clone(),
                    kind: DriftKind::Modified,
                    expected_sha256: Some(hash.clone()),
                    actual_sha256: Some(actual.clone()),
                });
            }
            Some(_) => {}
        }
    }
    for (path, hash) in live_files {
        if !expected.contains_key(path) && !status_counterparts.contains(path) {
            differences.push(DriftDifference {
                path: path.clone(),
                kind: DriftKind::Added,
                expected_sha256: None,
                actual_sha256: Some(hash.clone()),
            });
        }
    }
    if let Some(expected_config) = lockfile.config_policy.config_hash.as_deref() {
        if live_config_hash.map(|actual| actual.eq_ignore_ascii_case(expected_config)) != Some(true)
        {
            differences.push(DriftDifference {
                path: "config/".into(),
                kind: DriftKind::ConfigModified,
                expected_sha256: Some(expected_config.into()),
                actual_sha256: live_config_hash.map(str::to_string),
            });
        }
    }
    differences.sort_by(|a, b| a.path.cmp(&b.path));
    DriftReport {
        status: if differences.is_empty() {
            DriftStatus::InSync
        } else {
            DriftStatus::Drifted
        },
        differences,
    }
}

pub fn artifact_path(artifact: &LockedArtifact) -> String {
    let directory = match artifact.content_type.as_str() {
        "resourcepack" | "resourcepacks" => "resourcepacks",
        "shader" | "shaderpack" | "shaderpacks" => "shaderpacks",
        "datapack" | "datapacks" => "datapacks",
        "world" | "worlds" => "saves",
        _ => "mods",
    };
    let suffix = if artifact.enabled { "" } else { ".disabled" };
    format!("{directory}/{}{suffix}", artifact.filename)
}

fn validate_artifact(artifact: &LockedArtifact) -> Result<(), String> {
    let filename = artifact.filename.trim();
    if filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0')
    {
        return Err(format!(
            "Unsafe lockfile artifact filename: {}",
            artifact.filename
        ));
    }
    let hash = artifact.sha256.trim();
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("Invalid SHA-256 for {}.", artifact.filename));
    }
    if let Some(url) = artifact.source_url.as_deref() {
        let parsed = reqwest::Url::parse(url)
            .map_err(|error| format!("Invalid source URL for {}: {error}", artifact.filename))?;
        if parsed.scheme() != "https" {
            return Err(format!("Artifact source must use HTTPS: {url}"));
        }
    }
    Ok(())
}

fn verify_signature(content_hash: &str, signature: &LockfileSignature) -> Result<(), String> {
    if !signature.algorithm.eq_ignore_ascii_case("ed25519") {
        return Err(format!(
            "Unsupported lockfile signature algorithm: {}",
            signature.algorithm
        ));
    }
    let engine = base64::engine::general_purpose::STANDARD;
    let public_key = engine
        .decode(&signature.public_key)
        .map_err(|_| "Lockfile public key is not valid base64.".to_string())?;
    let public_key: [u8; 32] = public_key
        .try_into()
        .map_err(|_| "Lockfile Ed25519 public key must be 32 bytes.".to_string())?;
    let verifying_key = VerifyingKey::from_bytes(&public_key)
        .map_err(|_| "Lockfile Ed25519 public key is invalid.".to_string())?;
    let signature_bytes = engine
        .decode(&signature.signature)
        .map_err(|_| "Lockfile signature is not valid base64.".to_string())?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| "Lockfile Ed25519 signature must be 64 bytes.".to_string())?;
    verifying_key
        .verify(content_hash.as_bytes(), &signature)
        .map_err(|_| "Lockfile signature verification failed.".to_string())
}

/// Build an [`InstanceLockfile`] from an instance directory on disk.
///
/// Reads the instance manifest, walks content directories (mods, resourcepacks,
/// shaderpacks, datapacks, saves/worlds), computes SHA-256 for every artifact,
/// and produces a privacy-preserving lockfile (no config file contents).
pub fn build_from_instance(instance_dir: &std::path::Path) -> Result<InstanceLockfile, String> {
    use std::collections::BTreeMap;
    use std::fs;

    let manifest_path = instance_dir.join("instance_manifest.json");
    let manifest = crate::helpers::read_manifest(&manifest_path)
        .map_err(|e| format!("Cannot read instance manifest: {e}"))?;

    let content_dirs: BTreeMap<&str, &[crate::models::InstalledMod]> = BTreeMap::from([
        ("mods", manifest.mods.as_slice()),
        ("resourcepacks", manifest.resourcepacks.as_slice()),
        ("shaderpacks", manifest.shaders.as_slice()),
        ("datapacks", manifest.datapacks.as_slice()),
        ("saves", manifest.worlds.as_slice()),
    ]);

    let mut artifacts = Vec::new();
    for (dir_name, items) in &content_dirs {
        for item in *items {
            let file_path = instance_dir.join(dir_name).join(&item.filename);
            let disabled_path = format!("{}.disabled", file_path.display());
            let actual_path = if file_path.exists() {
                file_path
            } else if std::path::Path::new(&disabled_path).exists() {
                std::path::PathBuf::from(&disabled_path)
            } else {
                continue;
            };
            let data = fs::read(&actual_path)
                .map_err(|e| format!("Cannot read {}: {e}", item.filename))?;
            let sha256 = crate::download::sha256_hex(&data);

            let content_type = match *dir_name {
                "resourcepacks" => "resourcepack",
                "shaderpacks" => "shader",
                "saves" => "world",
                _ => *dir_name,
            };

            artifacts.push(LockedArtifact {
                filename: item.filename.clone(),
                content_type: content_type.to_string(),
                registry_id: item.registry_id.clone(),
                modrinth_id: item.modrinth_id.clone(),
                source: item.source.clone(),
                source_url: item.source_url.clone(),
                version: item.version.clone(),
                sha256,
                enabled: item.enabled,
                unresolved_reason: None,
            });
        }
    }

    let loader_entries =
        crate::loader_manifests::list_versions(&manifest.loader, &manifest.minecraft_version);
    let loader_entry = loader_entries
        .iter()
        .find(|e| e.loader_version == manifest.loader_version);

    let lockfile = InstanceLockfile::new(
        LockedInstance {
            name: manifest.name.clone(),
            minecraft_version: manifest.minecraft_version.clone(),
            loader: manifest.loader.clone(),
            loader_version: manifest.loader_version.clone(),
            is_locked: manifest.is_locked,
            user_preferences: manifest.user_preferences.clone(),
        },
        artifacts,
        LockedLoader {
            source_url: loader_entry.map(|e| e.source_url.clone()),
            sha256: loader_entry.map(|e| e.sha256.clone()),
        },
        crate::download::sha256_hex(&serde_json::to_vec(&manifest).unwrap_or_default()),
        None,
    )?;
    Ok(lockfile)
}

fn canonical_json(value: &serde_json::Value) -> Result<String, String> {
    fn sorted(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut entries = map.iter().collect::<Vec<_>>();
                entries.sort_by_key(|(left, _)| *left);
                serde_json::Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.clone(), sorted(value)))
                        .collect(),
                )
            }
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.iter().map(sorted).collect())
            }
            _ => value.clone(),
        }
    }
    serde_json::to_string(&sorted(value))
        .map_err(|error| format!("Could not serialize canonical JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn lockfile() -> InstanceLockfile {
        InstanceLockfile::new(
            LockedInstance {
                name: "Test".into(),
                minecraft_version: "1.21.1".into(),
                loader: "fabric".into(),
                loader_version: "0.16.0".into(),
                is_locked: false,
                user_preferences: serde_json::json!({"memoryMb": 4096}),
            },
            vec![LockedArtifact {
                filename: "example.jar".into(),
                content_type: "mod".into(),
                registry_id: Some("example".into()),
                modrinth_id: None,
                source: "registry".into(),
                source_url: Some("https://example.com/example.jar".into()),
                version: Some("1.0.0".into()),
                sha256: "ab".repeat(32),
                enabled: true,
                unresolved_reason: None,
            }],
            LockedLoader {
                source_url: Some("https://example.com/loader.jar".into()),
                sha256: Some("cd".repeat(32)),
            },
            "ef".repeat(32),
            Some("12".repeat(32)),
        )
        .unwrap()
    }

    #[test]
    fn canonical_round_trip_and_self_hash_are_stable() {
        let original = lockfile();
        let json = original.to_pretty_json().unwrap();
        let parsed = InstanceLockfile::parse_and_validate(&json).unwrap();
        assert_eq!(parsed, original);
        assert_eq!(
            parsed.recompute_content_hash().unwrap(),
            original.content_hash
        );
    }

    #[test]
    fn tampering_is_rejected() {
        let mut changed = lockfile();
        changed.artifacts[0].filename = "changed.jar".into();
        assert!(changed
            .validate()
            .unwrap_err()
            .contains("content hash mismatch"));
    }

    #[test]
    fn drift_detects_modified_missing_added_and_config() {
        let reference = lockfile();
        let live = BTreeMap::from([
            ("mods/example.jar".into(), "ff".repeat(32)),
            ("mods/extra.jar".into(), "ee".repeat(32)),
        ]);
        let report = detect_drift(&reference, &live, Some(&"34".repeat(32)));
        assert_eq!(report.status, DriftStatus::Drifted);
        assert_eq!(report.differences.len(), 3);
    }

    #[test]
    fn drift_reports_enabled_state_change_once() {
        let mut reference = lockfile();
        reference.artifacts[0].enabled = false;
        reference.content_hash = reference.recompute_content_hash().unwrap();
        let live = BTreeMap::from([("mods/example.jar".into(), "ab".repeat(32))]);
        let report = detect_drift(&reference, &live, Some(&"12".repeat(32)));
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.differences[0].kind, DriftKind::Enabled);
        assert_eq!(report.differences[0].path, "mods/example.jar.disabled");
    }

    #[test]
    fn artifact_paths_cover_every_supported_content_type() {
        let mut artifact = lockfile().artifacts.remove(0);
        for (content_type, expected_directory) in [
            ("mod", "mods"),
            ("resourcepack", "resourcepacks"),
            ("shader", "shaderpacks"),
            ("datapack", "datapacks"),
            ("world", "saves"),
        ] {
            artifact.content_type = content_type.into();
            assert_eq!(
                artifact_path(&artifact),
                format!("{expected_directory}/example.jar")
            );
        }
    }

    #[test]
    fn future_schema_fails_safely() {
        let mut future = lockfile();
        future.schema_version += 1;
        assert!(future.validate().unwrap_err().contains("Unsupported"));
    }

    #[test]
    fn vanilla_lockfile_with_empty_loader_version_validates() {
        let mut vanilla = lockfile();
        vanilla.instance.loader = "vanilla".into();
        vanilla.instance.loader_version = String::new();
        vanilla.loader = LockedLoader {
            source_url: None,
            sha256: None,
        };
        vanilla.content_hash = vanilla.recompute_content_hash().unwrap();
        vanilla.validate().unwrap();

        let json = vanilla.to_pretty_json().unwrap();
        InstanceLockfile::parse_and_validate(&json).unwrap();
    }

    #[test]
    fn non_vanilla_lockfile_with_empty_loader_version_is_rejected() {
        let mut missing = lockfile();
        missing.instance.loader_version = String::new();
        missing.content_hash = missing.recompute_content_hash().unwrap();
        assert!(missing
            .validate()
            .unwrap_err()
            .contains("identity is incomplete"));
    }

    #[test]
    fn valid_signature_is_accepted_and_tampering_is_rejected() {
        let mut signed = lockfile();
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let signature = key.sign(signed.content_hash.as_bytes());
        let engine = base64::engine::general_purpose::STANDARD;
        signed.signature = Some(LockfileSignature {
            algorithm: "ed25519".into(),
            public_key: engine.encode(key.verifying_key().to_bytes()),
            signature: engine.encode(signature.to_bytes()),
        });
        signed.validate().unwrap();

        signed.signature.as_mut().unwrap().signature = engine.encode([0u8; 64]);
        assert!(signed
            .validate()
            .unwrap_err()
            .contains("signature verification failed"));
    }
}
