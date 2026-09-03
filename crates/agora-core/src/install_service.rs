//! Core-owned InstallService — resolves and executes install plans.
//!
//! Owns: intent validation, instance/manifest loading, registry revision,
//! removal reverse-dependency planning, source-specific artifact resolution
//! (curated, Modrinth, manual), and InstallPipeline delegation.

use crate::ctx::Ctx;
use crate::dependency_ops::AliasMap;
use crate::error::{LauncherError, LauncherResult};
use crate::install_pipeline::{
    CancellationToken, InstallIntent, InstallOutcome, InstallPipeline, PreparedPlan,
    ProgressReporter, ResolvedInstallPlan, ResolvedOperation, ReverseDepInfo,
};
use crate::models::{InstalledMod, InstanceManifest};
use crate::resolver::Resolver;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// InstanceLoadResult
// ---------------------------------------------------------------------------

/// Result of loading instance state from the filesystem.
pub struct InstanceLoadResult {
    pub instance_dir: PathBuf,
    pub manifest: InstanceManifest,
    pub registry_revision: String,
}

// ---------------------------------------------------------------------------
// InstallService
// ---------------------------------------------------------------------------

/// Core-owned install service.
///
/// Create via [`InstallService::new`] with a [`Ctx`].
#[derive(Clone)]
pub struct InstallService {
    ctx: Ctx,
}

impl InstallService {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// Validate instance ID and load its manifest + registry revision.
    pub fn load_instance(&self, instance_id: &str) -> LauncherResult<InstanceLoadResult> {
        let sanitized = crate::paths::sanitize_id(instance_id);
        crate::app_paths::validate_path_component(&sanitized).map_err(|_| {
            LauncherError::Generic {
                code: "ERR_INVALID_INSTANCE".into(),
                message: "Invalid instance ID.".into(),
            }
        })?;
        let instance_dir = self.ctx.paths.instance_dir(&sanitized)?;
        let manifest_path = self.ctx.paths.instance_manifest(&sanitized)?;
        if !instance_dir.exists() || !manifest_path.exists() {
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_NOT_FOUND".into(),
                message: format!("Instance '{instance_id}' not found."),
            });
        }
        let manifest = crate::helpers::read_manifest(&manifest_path)?;
        let registry_revision = self.compute_registry_revision()?;
        Ok(InstanceLoadResult {
            instance_dir,
            manifest,
            registry_revision,
        })
    }

    /// Prepare a removal `PreparedPlan` (pure logic, no network needed).
    ///
    /// Reverse-dependency planning is computed here rather than left to the
    /// caller. This is the core-owned replacement for the desktop adapter's
    /// `prepare_remove` and the CLI's inline removal code.
    pub fn prepare_removal(
        manifest: &InstanceManifest,
        filename: &str,
        registry_revision: String,
    ) -> PreparedPlan {
        let target = all_installed(manifest).find(|item| {
            item.filename == filename
                || effective_filename(item) == filename
                || item
                    .registry_id
                    .as_deref()
                    .map(|id| id.eq_ignore_ascii_case(filename))
                    .unwrap_or(false)
                || item
                    .modrinth_id
                    .as_deref()
                    .map(|id| id.eq_ignore_ascii_case(filename))
                    .unwrap_or(false)
        });

        let (target_filename, reverse_dependents) = match target {
            Some(target) => {
                let aliases = AliasMap::from_pairs(&[]);
                let installed: Vec<InstalledMod> = all_installed(manifest).cloned().collect();
                let removal = crate::dependency_ops::build_removal_plan_with_aliases(
                    &installed, target, &aliases,
                );
                (
                    effective_filename(target),
                    removal
                        .dependents
                        .into_iter()
                        .map(|d| ReverseDepInfo {
                            mod_jar_id: d.mod_id,
                            filename: d.filename,
                            requirement: d.requirement,
                            impact: Some("Would lose a required dependency".into()),
                        })
                        .collect(),
                )
            }
            None => (filename.to_string(), vec![]),
        };

        PreparedPlan {
            operation: ResolvedOperation::Remove {
                target_filename,
                reverse_dependents,
                content_type: None,
            },
            dependencies: vec![],
            conflicts: vec![],
            registry_revision,
        }
    }

    /// Resolve an install intent into a read-only plan.
    ///
    /// Uses the core-owned [`Resolver`] to prepare curated, Modrinth, or
    /// manual artifacts and dependency dispositions, then normalizes through
    /// [`InstallPipeline`] to produce the final plan.
    pub async fn resolve(
        &self,
        intent: InstallIntent,
        reporter: &dyn ProgressReporter,
    ) -> LauncherResult<ResolvedInstallPlan> {
        let load = self.load_instance(&intent.target_instance)?;
        let mut resolver = Resolver::new(self.ctx.clone());
        if let Some(token) = std::env::var("GITHUB_TOKEN")
            .ok()
            .filter(|value| !value.is_empty())
        {
            resolver = resolver.with_github_token(token);
        } else if let Some(token) = crate::auth::get_token() {
            resolver = resolver.with_stored_github_token(token);
        }
        let prepared = resolver.resolve(&intent, &load.manifest).await?;
        let mut plan = InstallPipeline
            .resolve_plan(intent, &load.instance_dir, prepared, reporter)
            .await
            .map_err(|e| LauncherError::Generic {
                code: "ERR_RESOLVE".into(),
                message: e,
            })?;
        self.append_curated_conflict_warnings(&mut plan, &load.manifest);
        Ok(plan)
    }

    /// Execute a fully-resolved plan with freshness checks.
    ///
    /// Re-validates instance state and registry revision before delegating to
    /// `InstallPipeline::execute_plan`.
    pub async fn execute(
        &self,
        plan: &ResolvedInstallPlan,
        reporter: &dyn ProgressReporter,
        cancel: &CancellationToken,
    ) -> InstallOutcome {
        let load = match self.load_instance(&plan.intent.target_instance) {
            Ok(load) => load,
            Err(error) => {
                return InstallOutcome::Failed {
                    error: format!("Instance not accessible before execution: {error}"),
                    rollback_performed: false,
                    snapshot_id: None,
                };
            }
        };
        let registry_db = self
            .ctx
            .paths
            .registry_db()
            .is_file()
            .then(|| self.ctx.paths.registry_db());
        let loader_service = crate::loader_service::LoaderService::new(self.ctx.clone());
        InstallPipeline
            .execute_plan_with_scheduler(
                plan,
                &load.instance_dir,
                &load.registry_revision,
                reporter,
                cancel,
                crate::install_pipeline::InstallExecutionResources {
                    scheduler: Some(&self.ctx.task_scheduler),
                    registry_db_path: registry_db.as_deref(),
                    loader_service: Some(&loader_service),
                },
            )
            .await
    }

    /// Resolve a caller-prepared reconciliation plan through the pipeline.
    ///
    /// Used by trusted adapters (e.g., desktop lockfile repair/import) that
    /// have already computed the backend-resolved operations. The service
    /// handles instance loading, registry revision, and pipeline normalization.
    pub async fn resolve_prepared(
        &self,
        intent: InstallIntent,
        mut prepared: PreparedPlan,
        reporter: &dyn ProgressReporter,
    ) -> LauncherResult<ResolvedInstallPlan> {
        let load = self.load_instance(&intent.target_instance)?;
        prepared.registry_revision = load.registry_revision;
        let mut plan = InstallPipeline
            .resolve_plan(intent, &load.instance_dir, prepared, reporter)
            .await
            .map_err(|e| LauncherError::Generic {
                code: "ERR_RESOLVE".into(),
                message: e,
            })?;
        self.append_curated_conflict_warnings(&mut plan, &load.manifest);
        Ok(plan)
    }

    /// Warn when this plan would create a curator-flagged bad pairing.
    ///
    /// Best-effort by design: a missing or unreadable `registry.db` yields no
    /// warnings rather than an error. The registry is optional, and failing an
    /// install because the advisory data could not be read would be worse than
    /// the advice being absent.
    fn append_curated_conflict_warnings(
        &self,
        plan: &mut ResolvedInstallPlan,
        manifest: &InstanceManifest,
    ) {
        let conflicts =
            match crate::registry::RegistryService::new(self.ctx.clone()).known_conflicts() {
                Ok(conflicts) if !conflicts.is_empty() => conflicts,
                _ => return,
            };
        plan.warnings
            .extend(curated_conflict_warnings(plan, manifest, &conflicts));
    }

    /// Check that the instance is not locked.
    /// Pin or unpin an installed entry against updates.
    ///
    /// A pinned entry is skipped by "Update All" and shows no update badge, but
    /// stays installed and can still be updated deliberately. Matching is by
    /// filename across every content array, the same identity the enable/disable
    /// path uses.
    ///
    /// Returns `Ok(false)` when no entry matches, so a caller can distinguish
    /// "nothing to do" from a write failure.
    pub fn set_update_pinned(
        &self,
        instance_id: &str,
        filename: &str,
        pinned: bool,
    ) -> LauncherResult<bool> {
        let sanitized = crate::paths::sanitize_id(instance_id);
        let _guard = self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(sanitized.clone()),
            "set_update_pinned",
        )?;
        let manifest_path = self.ctx.paths.instance_manifest(&sanitized)?;
        let mut manifest = crate::helpers::read_manifest(&manifest_path)?;

        let mut changed = false;
        for entry in manifest
            .mods
            .iter_mut()
            .chain(manifest.resourcepacks.iter_mut())
            .chain(manifest.shaders.iter_mut())
            .chain(manifest.datapacks.iter_mut())
            .chain(manifest.worlds.iter_mut())
        {
            if entry.filename == filename && entry.update_pinned != pinned {
                entry.update_pinned = pinned;
                changed = true;
            }
        }
        if changed {
            crate::helpers::atomic_write_manifest(&manifest_path, &manifest)?;
        }
        Ok(changed)
    }

    pub fn check_not_locked(&self, instance_id: &str) -> LauncherResult<()> {
        let manifest_path = self.ctx.paths.instance_manifest(instance_id)?;
        if !manifest_path.exists() {
            return Ok(());
        }
        let manifest = crate::helpers::read_manifest(&manifest_path)?;
        if manifest.is_locked {
            return Err(LauncherError::InstanceLocked);
        }
        Ok(())
    }

    /// Download, verify, and install a single artifact into an instance.
    ///
    /// This is a convenience method for direct installs that bypass the full
    /// pipeline (snapshot, staging, health scan). For production use, prefer
    /// `resolve_and_execute` through the pipeline.
    #[allow(clippy::too_many_arguments)]
    pub async fn install_artifact(
        &self,
        instance_id: &str,
        filename: &str,
        content_type: &str,
        download_url: &str,
        registry_id: Option<&str>,
        modrinth_id: Option<&str>,
        source: &str,
        version: Option<&str>,
        expected_sha1: Option<&str>,
        expected_sha256: Option<&str>,
    ) -> LauncherResult<InstalledMod> {
        self.check_not_locked(instance_id)?;

        let dir = self.ctx.paths.instance_dir(instance_id)?;
        crate::helpers::check_disk_space(&dir)?;

        let bytes =
            crate::download::download_mod_bytes(&self.ctx.http_clients, download_url).await?;

        // Verification is mandatory and fails closed, matching the install
        // pipeline's `verify_bytes`. A candidate whose source published no
        // usable hash (e.g. an older GitHub release with no asset digest, where
        // the registry's pinned hash describes a different file) must not be
        // silently installed unverified.
        let candidate_sha1 = expected_sha1.unwrap_or("").trim().to_lowercase();
        let candidate_sha256 = expected_sha256.unwrap_or("").trim().to_lowercase();
        if !candidate_sha1.is_empty() {
            let actual_sha1 = crate::download::sha1_hex(&bytes);
            if actual_sha1 != candidate_sha1 {
                return Err(LauncherError::HashMismatch);
            }
        } else if !candidate_sha256.is_empty() {
            let actual_sha = crate::download::sha256_hex(&bytes);
            if actual_sha != candidate_sha256 {
                return Err(LauncherError::HashMismatch);
            }
        } else {
            return Err(LauncherError::Generic {
                code: "ERR_NO_PUBLISHED_HASH".into(),
                message: format!(
                    "{filename} cannot be installed: its source published no SHA-1 or SHA-256 \
                     for this version, so Agora cannot verify the download."
                ),
            });
        }

        let installed_sha256 = crate::download::sha256_hex(&bytes);
        let target_dir = dir.join(crate::helpers::content_subdir(content_type));
        std::fs::create_dir_all(&target_dir).map_err(|_| LauncherError::InstanceCreateFailed)?;
        let item_path = target_dir.join(filename);
        std::fs::write(&item_path, &bytes).map_err(|_| LauncherError::InstanceCreateFailed)?;

        let manifest_path = self.ctx.paths.instance_manifest(instance_id)?;
        let mut manifest = crate::helpers::read_manifest(&manifest_path)?;
        let metadata =
            crate::jar_metadata::parse_jar_metadata_for_loader(&item_path, &manifest.loader);

        let installed_mod = InstalledMod {
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            filename: filename.to_string(),
            registry_id: registry_id.map(|s| s.to_string()),
            modrinth_id: modrinth_id.map(|s| s.to_string()),
            source: source.to_string(),
            source_url: Some(download_url.to_string()),
            version: version.map(|s| s.to_string()),
            sha256: installed_sha256,
            installed_at: chrono::Utc::now().to_rfc3339(),
            java_packages: metadata.java_packages,
            mod_jar_id: metadata.mod_jar_id,
            depends_on: metadata.depends_on,
            optional_deps: metadata.optional_deps,
            incompatible_deps: metadata.incompatible_deps,
            provided_mod_ids: metadata
                .provided_mods
                .into_iter()
                .map(|provided| provided.mod_id)
                .collect(),
            enabled: true,
            content_type: if content_type.is_empty() {
                "mod".to_string()
            } else {
                content_type.to_string()
            },
        };

        crate::helpers::push_to_content_array(&mut manifest, &installed_mod);
        crate::helpers::atomic_write_manifest(&manifest_path, &manifest)?;
        let _ = crate::snapshot::mark_instance_mutated(&dir);

        Ok(installed_mod)
    }

    /// Remove an artifact from an instance by filename.
    ///
    /// Deletes the file from whichever content subdirectory it resides in and
    /// updates the manifest atomically.
    pub fn remove_artifact(&self, instance_id: &str, filename: &str) -> LauncherResult<bool> {
        self.check_not_locked(instance_id)?;

        if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
            return Err(LauncherError::Generic {
                code: "ERR_INVALID_FILENAME".to_string(),
                message: "Filename contains invalid characters.".to_string(),
            });
        }

        let dir = self.ctx.paths.instance_dir(instance_id)?;
        let removed = crate::helpers::find_and_delete_file(&dir, filename);

        let manifest_path = self.ctx.paths.instance_manifest(instance_id)?;
        if manifest_path.exists() {
            let mut manifest = crate::helpers::read_manifest(&manifest_path)?;

            if crate::helpers::remove_from_content_array(&mut manifest, filename) {
                crate::helpers::atomic_write_manifest(&manifest_path, &manifest)?;
            }
        }

        let _ = crate::snapshot::mark_instance_mutated(&dir);

        Ok(removed)
    }

    /// Add a manually-dropped .jar file into an instance's `mods/` folder.
    ///
    /// Security: the source path must resolve to one of the user's allowlisted
    /// drop directories (Downloads, Desktop, Documents, or system temp).
    pub fn add_manual_artifact(
        &self,
        instance_id: &str,
        source_path: &str,
    ) -> LauncherResult<InstalledMod> {
        self.check_not_locked(instance_id)?;

        let src = std::path::Path::new(source_path);
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        if ext.as_deref() != Some("jar") {
            return Err(LauncherError::Generic {
                code: "ERR_INVALID_FILENAME".to_string(),
                message: "Only .jar files can be added manually.".to_string(),
            });
        }
        let file_name =
            src.file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| LauncherError::Generic {
                    code: "ERR_INVALID_FILENAME".to_string(),
                    message: "Could not determine a valid file name.".to_string(),
                })?;
        if file_name.contains("..") || file_name.contains('/') || file_name.contains('\\') {
            return Err(LauncherError::Generic {
                code: "ERR_INVALID_FILENAME".to_string(),
                message: "Filename contains invalid characters.".to_string(),
            });
        }

        let dir = self.ctx.paths.instance_dir(instance_id)?;
        let mods_dir = dir.join("mods");
        let dest = mods_dir.join(file_name);
        let manifest_path = self.ctx.paths.instance_manifest(instance_id)?;

        let canonical = std::fs::canonicalize(source_path).map_err(|_| LauncherError::Generic {
            code: "ERR_INVALID_SOURCE".to_string(),
            message: "Source file does not exist or cannot be resolved.".to_string(),
        })?;

        let mut roots: Vec<std::path::PathBuf> = Vec::new();
        for r in [
            dirs::download_dir(),
            dirs::desktop_dir(),
            dirs::document_dir(),
            Some(std::env::temp_dir()),
        ]
        .into_iter()
        .flatten()
        {
            if let Ok(c) = std::fs::canonicalize(&r) {
                roots.push(c);
            }
        }
        let allowed = roots.iter().any(|root| canonical.starts_with(root));
        if !allowed {
            return Err(LauncherError::Generic {
                code: "ERR_SOURCE_NOT_ALLOWED".to_string(),
                message: "Source file is outside the allowed drop directories \
                          (Downloads, Desktop, Documents, or system temp)."
                    .to_string(),
            });
        }

        let bytes = std::fs::read(&canonical).map_err(|_| LauncherError::Generic {
            code: "ERR_READ_FAILED".to_string(),
            message: "Failed to read the dropped file.".to_string(),
        })?;

        std::fs::create_dir_all(&mods_dir).map_err(|_| LauncherError::InstanceCreateFailed)?;
        std::fs::write(&dest, &bytes).map_err(|_| LauncherError::InstanceCreateFailed)?;
        let sha256 = crate::download::sha256_hex(&bytes);

        if !manifest_path.exists() {
            return Err(LauncherError::Generic {
                code: "ERR_MANIFEST_MISSING".to_string(),
                message: "Instance manifest not found. Create the instance first.".to_string(),
            });
        }
        let mut manifest = crate::helpers::read_manifest(&manifest_path)?;

        let metadata = crate::jar_metadata::parse_jar_metadata_for_loader(&dest, &manifest.loader);
        let installed_mod = InstalledMod {
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            filename: file_name.to_string(),
            registry_id: None,
            modrinth_id: None,
            source: "manual_drag_drop".to_string(),
            source_url: None,
            version: None,
            sha256,
            installed_at: chrono::Utc::now().to_rfc3339(),
            java_packages: metadata.java_packages,
            mod_jar_id: metadata.mod_jar_id,
            depends_on: metadata.depends_on,
            optional_deps: metadata.optional_deps,
            incompatible_deps: metadata.incompatible_deps,
            provided_mod_ids: metadata
                .provided_mods
                .into_iter()
                .map(|provided| provided.mod_id)
                .collect(),
            enabled: true,
            content_type: "mod".to_string(),
        };
        crate::helpers::push_to_content_array(&mut manifest, &installed_mod);
        crate::helpers::atomic_write_manifest(&manifest_path, &manifest)?;
        let _ = crate::snapshot::mark_instance_mutated(&dir);

        Ok(installed_mod)
    }

    /// Convenience: resolve and immediately execute (one-shot).
    ///
    /// Useful for the CLI and non-interactive callers.
    pub async fn resolve_and_execute(
        &self,
        intent: InstallIntent,
        reporter: &dyn ProgressReporter,
        cancel: &CancellationToken,
    ) -> LauncherResult<InstallOutcome> {
        let plan = self.resolve(intent, reporter).await?;
        Ok(self.execute(&plan, reporter, cancel).await)
    }

    fn compute_registry_revision(&self) -> LauncherResult<String> {
        let path = self.ctx.paths.registry_db();
        if !path.is_file() {
            return Ok("registry-unavailable".into());
        }
        let bytes = std::fs::read(&path).map_err(|e| LauncherError::Generic {
            code: "ERR_REGISTRY_READ".into(),
            message: format!("Could not read registry: {e}"),
        })?;
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        Ok(format!("{:x}", hasher.finalize()))
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn all_installed(manifest: &InstanceManifest) -> impl Iterator<Item = &InstalledMod> {
    manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
        .chain(manifest.worlds.iter())
}

fn effective_filename(item: &InstalledMod) -> String {
    if item.enabled || item.filename.ends_with(".disabled") {
        item.filename.clone()
    } else {
        format!("{}.disabled", item.filename)
    }
}

/// Curated-conflict warnings for the content set `plan` would leave behind.
///
/// Only pairs that this plan *creates* are reported: at least one side has to
/// be something the plan is adding. A conflict already sitting in the instance
/// is pre-launch health's job — repeating it on an unrelated install would be
/// noise attached to the wrong action.
///
/// Version-windowed facts are evaluated against the versions that will actually
/// be present, so installing 0.4 of a mod flagged from 0.5 stays quiet.
fn curated_conflict_warnings(
    plan: &ResolvedInstallPlan,
    manifest: &InstanceManifest,
    conflicts: &[crate::registry::KnownConflict],
) -> Vec<crate::install_pipeline::PlanWarning> {
    use crate::install_pipeline::{PlanWarning, ResolvedArtifact};
    use std::collections::HashMap;

    let removed: std::collections::HashSet<&str> = plan
        .files_to_remove
        .iter()
        .map(|f| f.filename.as_str())
        .collect();

    // id -> (version, is_incoming). An id that is both installed and being
    // updated counts as incoming: the plan is what put the new version there.
    let mut present: HashMap<String, (Option<String>, bool)> = HashMap::new();
    for item in all_installed(manifest) {
        if removed.contains(item.filename.as_str()) {
            continue;
        }
        for id in [item.registry_id.as_deref(), item.modrinth_id.as_deref()]
            .into_iter()
            .flatten()
        {
            present.insert(id.to_ascii_lowercase(), (item.version.clone(), false));
        }
    }
    for add in &plan.files_to_add {
        let metadata = match &add.artifact {
            ResolvedArtifact::Download(download) => &download.metadata,
            ResolvedArtifact::LocalFile(local) => &local.metadata,
        };
        for id in [
            metadata.registry_id.as_deref(),
            metadata.modrinth_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            present.insert(id.to_ascii_lowercase(), (metadata.version.clone(), true));
        }
    }

    let mut warnings = Vec::new();
    for conflict in conflicts {
        let Some((a_version, a_incoming)) = present.get(&conflict.mod_a_id.to_ascii_lowercase())
        else {
            continue;
        };
        let Some((b_version, b_incoming)) = present.get(&conflict.mod_b_id.to_ascii_lowercase())
        else {
            continue;
        };
        if !a_incoming && !b_incoming {
            continue;
        }
        if !conflict.applies_to_versions(a_version.as_deref(), b_version.as_deref()) {
            continue;
        }
        let mitigation = if conflict.mitigated_by.is_empty() {
            String::new()
        } else {
            format!(" Known mitigation: {}.", conflict.mitigated_by.join(", "))
        };
        warnings.push(PlanWarning {
            code: "WARN_KNOWN_CONFLICT".into(),
            message: format!(
                "'{}' and '{}' are a known conflict (severity: {}).{}{}",
                conflict.mod_a_id,
                conflict.mod_b_id,
                conflict.severity,
                conflict
                    .notes
                    .as_deref()
                    .map(|n| format!(" {n}"))
                    .unwrap_or_default(),
                mitigation
            ),
        });
    }
    warnings
}

#[cfg(test)]
mod curated_conflict_tests {
    use super::*;
    use crate::dependency_ops::VersionGrammar;
    use crate::install_pipeline::{
        ArtifactMetadata, ArtifactSource, DiskSpaceEstimate, FileAdd, FileRemove, HashSpec,
        InstallAction, OptionalDepsPolicy, PlanOverrides, RequestSource, ResolvedArtifact,
        ResolvedDownload, ResolvedOperation, SnapshotPlan, SourceType,
    };
    use crate::registry::KnownConflict;

    fn conflict(a: &str, b: &str, a_versions: &[&str]) -> KnownConflict {
        KnownConflict {
            mod_a_id: a.into(),
            mod_b_id: b.into(),
            severity: "hard".into(),
            mitigated_by: vec![],
            notes: None,
            mod_a_versions: a_versions.iter().map(|s| s.to_string()).collect(),
            mod_b_versions: vec![],
            version_grammar: VersionGrammar::Fabric,
        }
    }

    fn installed(registry_id: &str, filename: &str, version: Option<&str>) -> InstalledMod {
        InstalledMod {
            filename: filename.into(),
            registry_id: Some(registry_id.into()),
            modrinth_id: None,
            source: "registry".into(),
            source_url: None,
            version: version.map(str::to_string),
            sha256: String::new(),
            installed_at: String::new(),
            java_packages: vec![],
            mod_jar_id: Some(registry_id.into()),
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        }
    }

    fn manifest_with(mods: Vec<InstalledMod>) -> InstanceManifest {
        InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: "inst".into(),
            name: "Inst".into(),
            created_from_pack: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16.0".into(),
            is_locked: false,
            mods,
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        }
    }

    fn adding(registry_id: &str, version: Option<&str>) -> FileAdd {
        FileAdd {
            target_filename: format!("{registry_id}.jar"),
            staging_filename: format!("{registry_id}.jar.part"),
            artifact: ResolvedArtifact::Download(ResolvedDownload {
                item_id: registry_id.into(),
                version_id: "v".into(),
                source: ArtifactSource::Download {
                    url: "https://example.com/x.jar".into(),
                },
                hashes: HashSpec { values: vec![] },
                size: 0,
                filename: format!("{registry_id}.jar"),
                metadata: ArtifactMetadata {
                    source_type: SourceType::Curated,
                    registry_id: Some(registry_id.into()),
                    modrinth_id: None,
                    content_type: "mod".into(),
                    version: version.map(str::to_string),
                    download_strategy: None,
                    pinned_host: None,
                },
            }),
            hashes: HashSpec { values: vec![] },
            size: 0,
            installed_as_dependency: false,
        }
    }

    fn plan_adding(adds: Vec<FileAdd>, removes: Vec<FileRemove>) -> ResolvedInstallPlan {
        ResolvedInstallPlan {
            fingerprint: String::new(),
            intent: InstallIntent {
                action: InstallAction::Install {
                    source_type: SourceType::Curated,
                    item_id: "x".into(),
                    candidate_version: None,
                },
                target_instance: "inst".into(),
                optional_deps: OptionalDepsPolicy::ExcludeAll,
                requested_by: RequestSource::Interactive,
                overrides: PlanOverrides::default(),
            },
            operation: ResolvedOperation::Remove {
                target_filename: "noop.jar".into(),
                reverse_dependents: vec![],
                content_type: None,
            },
            dependencies: vec![],
            conflicts: vec![],
            files_to_add: adds,
            files_to_remove: removes,
            files_to_disable: vec![],
            files_to_promote: vec![],
            snapshot: SnapshotPlan {
                label: String::new(),
                estimated_bytes: 0,
            },
            disk_estimate: DiskSpaceEstimate::zero(),
            warnings: vec![],
            blocking_errors: vec![],
            pending_choices: vec![],
            loader_change: None,
            created_at: String::new(),
            instance_state_hash: String::new(),
            registry_revision: String::new(),
        }
    }

    #[test]
    fn installing_the_second_half_of_a_known_pair_warns() {
        let manifest = manifest_with(vec![installed("sodium", "sodium.jar", Some("0.5.3"))]);
        let plan = plan_adding(vec![adding("optifine", Some("1.0"))], vec![]);
        let warnings =
            curated_conflict_warnings(&plan, &manifest, &[conflict("optifine", "sodium", &[])]);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "WARN_KNOWN_CONFLICT");
    }

    #[test]
    fn a_conflict_already_present_is_not_re_reported_on_an_unrelated_install() {
        // Pre-launch health owns this one. Attaching it to an unrelated
        // install would blame the wrong action.
        let manifest = manifest_with(vec![
            installed("sodium", "sodium.jar", Some("0.5.3")),
            installed("optifine", "optifine.jar", Some("1.0")),
        ]);
        let plan = plan_adding(vec![adding("lithium", Some("0.12"))], vec![]);
        assert!(curated_conflict_warnings(
            &plan,
            &manifest,
            &[conflict("optifine", "sodium", &[])]
        )
        .is_empty());
    }

    #[test]
    fn a_version_window_that_excludes_the_incoming_version_stays_quiet() {
        let manifest = manifest_with(vec![installed("sodium", "sodium.jar", Some("0.5.3"))]);
        let plan = plan_adding(vec![adding("optifine", Some("0.9"))], vec![]);
        // Flagged only from 1.0 onward; 0.9 is going in.
        let facts = [conflict("optifine", "sodium", &[">=1.0"])];
        assert!(curated_conflict_warnings(&plan, &manifest, &facts).is_empty());

        let newer = plan_adding(vec![adding("optifine", Some("1.2"))], vec![]);
        assert_eq!(
            curated_conflict_warnings(&newer, &manifest, &facts).len(),
            1
        );
    }

    #[test]
    fn removing_the_other_half_in_the_same_plan_clears_the_warning() {
        let manifest = manifest_with(vec![installed("sodium", "sodium.jar", Some("0.5.3"))]);
        let plan = plan_adding(
            vec![adding("optifine", Some("1.0"))],
            vec![FileRemove {
                filename: "sodium.jar".into(),
                content_type: Some("mod".into()),
            }],
        );
        assert!(curated_conflict_warnings(
            &plan,
            &manifest,
            &[conflict("optifine", "sodium", &[])]
        )
        .is_empty());
    }

    #[test]
    fn an_empty_plan_and_an_empty_fact_set_produce_nothing() {
        let manifest = manifest_with(vec![installed("sodium", "sodium.jar", Some("0.5.3"))]);
        let empty_plan = plan_adding(vec![], vec![]);
        assert!(curated_conflict_warnings(
            &empty_plan,
            &manifest,
            &[conflict("optifine", "sodium", &[])]
        )
        .is_empty());
        let plan = plan_adding(vec![adding("optifine", Some("1.0"))], vec![]);
        assert!(curated_conflict_warnings(&plan, &manifest_with(vec![]), &[]).is_empty());
    }

    #[test]
    fn ids_match_case_insensitively() {
        let manifest = manifest_with(vec![installed("Sodium", "sodium.jar", Some("0.5.3"))]);
        let plan = plan_adding(vec![adding("OptiFine", Some("1.0"))], vec![]);
        assert_eq!(
            curated_conflict_warnings(&plan, &manifest, &[conflict("optifine", "sodium", &[])])
                .len(),
            1
        );
    }
}
