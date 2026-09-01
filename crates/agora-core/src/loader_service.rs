//! Core-owned loader installation, repair, and version switching.

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::event_sink::{CoreEvent, EventStatus, ProgressEvent, ProgressPhase};
use crate::health::HealthReport;
use crate::installed_profile::{self, InstallReceiptSummary, LoaderTuple};
use crate::java::JavaInstallation;
use crate::loader_compatibility::{
    evaluate_loader_compatibility, LoaderCompatibilityReport, LoaderCompatibilityRequest,
};
use crate::loader_manifests::{self, LoaderEntry};
use crate::models::{InstanceManifest, InstanceRow};
use crate::network::{NetworkCategory, NetworkPolicy};
use crate::operation_manager::OpHandle;
use std::path::Path;
use std::time::Duration;

const BACKUP_SUFFIX: &str = ".bak-reinstall";
const INSTALLER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_INSTALLER_OUTPUT_BYTES: u64 = 1024 * 1024;

/// Core-owned loader installation service.
#[derive(Clone)]
pub struct LoaderService {
    ctx: Ctx,
}

impl LoaderService {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    pub fn list_versions(
        &self,
        loader: &str,
        minecraft_version: &str,
    ) -> Vec<LoaderVersionSummary> {
        loader_manifests::list_versions(loader, minecraft_version)
            .into_iter()
            .map(|entry| LoaderVersionSummary {
                loader: loader.to_owned(),
                mc_version: entry.mc_version,
                loader_version: entry.loader_version,
                file_type: entry.file_type,
            })
            .collect()
    }

    pub async fn repair(&self, instance_id: &str) -> LauncherResult<InstallReceiptSummary> {
        let conn = crate::db::local_state_connection(&self.ctx.paths.local_state_db())
            .map_err(|_| LauncherError::LocalStateFailed)?;
        let row = crate::db::get_instance(&conn, instance_id)
            .map_err(|_| LauncherError::LocalStateFailed)?
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_INSTANCE_NOT_FOUND".into(),
                message: format!("Instance '{instance_id}' not found"),
            })?;
        self.ensure_installed(
            &row.loader,
            &row.minecraft_version,
            &row.loader_version,
            true,
        )
        .await
    }

    /// Plan a loader version switch without mutating anything.
    ///
    /// The instance is locked for the duration, an active core-managed
    /// process is rejected, the DB/manifest loader tuples must match, and the
    /// enabled mods are re-inventoried and evaluated against the active
    /// signed loader catalog. The returned plan carries the current
    /// compatibility report; committing it is a separate
    /// [`Self::change_loader_version`] call.
    pub fn plan_loader_change(&self, instance_id: &str) -> LauncherResult<LoaderChangePlan> {
        let instance_id = self.sanitize_instance_id(instance_id)?;
        let _lock = self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(instance_id.clone()),
            "plan-loader-change",
        )?;
        self.reject_active_process(&instance_id)?;
        let (row, manifest) = self.load_instance_tuple(&instance_id)?;
        let report = self
            .evaluate_loader_compatibility(&instance_id, &manifest)
            .ok_or_else(|| no_loader_requirements_error(&instance_id))?;
        Ok(LoaderChangePlan {
            instance_id,
            loader: row.loader,
            minecraft_version: row.minecraft_version,
            current_loader_version: row.loader_version,
            recommended_loader_version: report
                .recommended_version
                .as_ref()
                .map(|candidate| candidate.loader_version.clone()),
            current_report: report,
        })
    }

    /// Switch an instance's loader version in one transactional operation.
    ///
    /// Steps: validate and lock the instance (rejecting an active
    /// core-managed process), require DB/manifest loader-tuple agreement,
    /// re-inventory the enabled mods and evaluate the active signed catalog,
    /// verify the target is an exact signed catalog tuple that satisfies
    /// every understood hard requirement (Unsupported hard requirements also
    /// reject the automatic change), install the target first, then commit
    /// manifest-first with DB rollback on failure. After commit the health
    /// and launch-plan caches are invalidated, and a fresh health report is
    /// returned with the result.
    pub async fn change_loader_version(
        &self,
        instance_id: &str,
        target_version: &str,
    ) -> LauncherResult<LoaderChangeResult> {
        let instance_id = self.sanitize_instance_id(instance_id)?;
        let _lock = self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(instance_id.clone()),
            "change-loader",
        )?;
        self.change_loader_version_locked(&instance_id, target_version, false)
            .await
    }

    /// Explicit manual-selection variant. An indeterminate target is allowed
    /// only after the caller has shown the user the signed candidate and asked
    /// for advanced confirmation. Known-incompatible targets remain rejected.
    pub async fn change_loader_version_with_confirmation(
        &self,
        instance_id: &str,
        target_version: &str,
        allow_indeterminate: bool,
    ) -> LauncherResult<LoaderChangeResult> {
        let instance_id = self.sanitize_instance_id(instance_id)?;
        let _lock = self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(instance_id.clone()),
            "change-loader",
        )?;
        self.change_loader_version_locked(&instance_id, target_version, allow_indeterminate)
            .await
    }

    /// Perform a loader switch while the caller owns the instance lock. This
    /// keeps switch-and-launch atomic with respect to other instance changes.
    pub(crate) async fn change_loader_version_locked(
        &self,
        instance_id: &str,
        target_version: &str,
        allow_indeterminate: bool,
    ) -> LauncherResult<LoaderChangeResult> {
        let target_version = target_version.trim();
        if target_version.is_empty() {
            return Err(LauncherError::Generic {
                code: "ERR_LOADER_CHANGE_INVALID_TARGET".into(),
                message: "Target loader version must not be empty.".into(),
            });
        }
        self.reject_active_process(instance_id)?;
        let (row, manifest) = self.load_instance_tuple(instance_id)?;
        if row.is_locked {
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_LOCKED".into(),
                message: format!("Instance '{instance_id}' is locked."),
            });
        }
        if row.loader_version == target_version {
            return Err(LauncherError::Generic {
                code: "ERR_LOADER_CHANGE_NOOP".into(),
                message: format!(
                    "Instance '{instance_id}' already uses {} {}.",
                    row.loader, target_version
                ),
            });
        }

        // The target must exist exactly in the active signed catalog for the
        // same loader + Minecraft tuple.
        let catalog = loader_manifests::active_catalog();
        if catalog
            .find_entry(&row.loader, &row.minecraft_version, target_version)
            .is_none()
        {
            return Err(LauncherError::Generic {
                code: "ERR_UNSUPPORTED_LOADER".into(),
                message: format!(
                    "{} {} {} is not a pinned signed catalog tuple.",
                    row.loader, row.minecraft_version, target_version
                ),
            });
        }

        let report = self
            .evaluate_loader_compatibility(instance_id, &manifest)
            .ok_or_else(|| no_loader_requirements_error(instance_id))?;
        // Compatible candidates satisfy every hard requirement; a candidate
        // with any Unsupported hard requirement is never listed. The target
        // must be one of them, so neither an Unsatisfied nor an Unsupported
        // hard requirement can pass an automatic switch (manual advanced
        // selection is a future explicit API).
        let target_satisfies_all = report
            .compatible_versions
            .iter()
            .any(|candidate| candidate.loader_version == target_version);
        let target_is_indeterminate = report
            .indeterminate_versions
            .iter()
            .any(|candidate| candidate.loader_version == target_version);
        if !(target_satisfies_all || allow_indeterminate && target_is_indeterminate) {
            return Err(LauncherError::Generic {
                code: "ERR_LOADER_CHANGE_REJECTED".into(),
                message: format!(
                    "{} {} does not satisfy every hard loader requirement of the enabled mods.",
                    row.loader, target_version
                ),
            });
        }

        // Install the target first. A successfully installed profile may
        // remain in the shared cache after a subsequent metadata failure.
        self.ensure_installed(&row.loader, &row.minecraft_version, target_version, false)
            .await?;

        let manifest_path = self.ctx.paths.instance_manifest(instance_id)?;
        let conn = crate::db::local_state_connection(&self.ctx.paths.local_state_db())
            .map_err(|_| LauncherError::LocalStateFailed)?;
        commit_loader_version_change(
            &manifest_path,
            &manifest,
            &conn,
            instance_id,
            &row.loader,
            &row.minecraft_version,
            &row.loader_version,
            target_version,
        )?;

        // Invalidate health and launch-plan caches after the commit so the
        // next scan and handoff observe the new tuple.
        let instance_dir = self.ctx.paths.instance_dir(instance_id)?;
        // Tracked content changed; the next launch must not reuse a recovery
        // snapshot taken under the previous loader tuple.
        let _ = crate::snapshot::mark_instance_mutated(&instance_dir);
        // The metadata commit is complete at this point. Cache cleanup is
        // advisory because both health and launch-plan keys include the new
        // manifest/loader tuple; never report a completed switch as failed
        // merely because stale cache material could not be deleted.
        let _ = crate::health::invalidate_health_cache(&instance_dir);
        invalidate_launch_plan_cache(&self.ctx.paths.minecraft_runtime_root(), instance_id);

        // Delegated launches regenerate the official-launcher profile from
        // the DB row at every handoff (`prepare_delegated_launch`), so the
        // new tuple is picked up before the next handoff without a separate
        // cache.
        let mut updated_manifest = manifest;
        updated_manifest.loader_version = target_version.to_string();
        let registry_db = self
            .ctx
            .paths
            .registry_db()
            .exists()
            .then(|| self.ctx.paths.registry_db());
        let health = crate::health::cached_health(
            &instance_dir,
            &updated_manifest,
            registry_db.as_deref(),
            None,
        );
        Ok(LoaderChangeResult {
            instance_id: instance_id.to_string(),
            previous_loader_version: row.loader_version,
            loader_version: target_version.to_string(),
            health,
        })
    }

    fn sanitize_instance_id(&self, instance_id: &str) -> LauncherResult<String> {
        let sanitized = crate::paths::sanitize_id(instance_id);
        crate::app_paths::validate_path_component(&sanitized)?;
        Ok(sanitized)
    }

    fn reject_active_process(&self, instance_id: &str) -> LauncherResult<()> {
        if self
            .ctx
            .process_session_manager
            .list()
            .iter()
            .any(|session| session.instance_id == instance_id)
        {
            return Err(LauncherError::Generic {
                code: "ERR_INSTANCE_BUSY".into(),
                message: format!(
                    "Instance '{instance_id}' is running; stop it before switching the loader."
                ),
            });
        }
        Ok(())
    }

    fn load_instance_tuple(
        &self,
        instance_id: &str,
    ) -> LauncherResult<(InstanceRow, InstanceManifest)> {
        let conn = crate::db::local_state_connection(&self.ctx.paths.local_state_db())
            .map_err(|_| LauncherError::LocalStateFailed)?;
        let row = crate::db::get_instance(&conn, instance_id)
            .map_err(|_| LauncherError::LocalStateFailed)?
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_INSTANCE_NOT_FOUND".into(),
                message: format!("Instance '{instance_id}' not found"),
            })?;
        let manifest_path = self.ctx.paths.instance_manifest(instance_id)?;
        let manifest = crate::helpers::read_manifest(&manifest_path)?;
        if row.loader != manifest.loader
            || row.minecraft_version != manifest.minecraft_version
            || row.loader_version != manifest.loader_version
        {
            return Err(LauncherError::Generic {
                code: "ERR_LOADER_TUPLE_MISMATCH".into(),
                message: format!(
                    "Instance '{instance_id}' has a mismatched loader tuple between the database and its manifest; repair the instance before switching."
                ),
            });
        }
        Ok((row, manifest))
    }

    fn evaluate_loader_compatibility(
        &self,
        instance_id: &str,
        manifest: &InstanceManifest,
    ) -> Option<LoaderCompatibilityReport> {
        let instance_dir = self.ctx.paths.instance_dir(instance_id).ok()?;
        if matches!(manifest.loader.as_str(), "" | "vanilla") {
            return None;
        }
        let requirements = crate::health::inventory(&instance_dir, manifest)
            .artifacts
            .into_iter()
            .flat_map(|artifact| artifact.metadata.dependency_decls)
            .collect::<Vec<_>>();
        let catalog = loader_manifests::active_catalog();
        Some(evaluate_loader_compatibility(&LoaderCompatibilityRequest {
            loader: &manifest.loader,
            minecraft_version: &manifest.minecraft_version,
            current_loader_version: (!manifest.loader_version.is_empty())
                .then_some(manifest.loader_version.as_str()),
            requirements: &requirements,
            catalog: &catalog,
        }))
    }

    pub async fn ensure_installed(
        &self,
        loader: &str,
        minecraft_version: &str,
        loader_version: &str,
        force_reinstall: bool,
    ) -> LauncherResult<InstallReceiptSummary> {
        let entry = loader_manifests::find_entry(loader, minecraft_version, loader_version)
            .ok_or(LauncherError::UnsupportedLoader)?;
        self.ensure_entry(loader.to_owned(), entry, force_reinstall)
            .await
    }

    async fn ensure_entry(
        &self,
        loader: String,
        entry: LoaderEntry,
        force_reinstall: bool,
    ) -> LauncherResult<InstallReceiptSummary> {
        let tuple = LoaderTuple {
            loader: loader.clone(),
            minecraft_version: entry.mc_version.clone(),
            loader_version: entry.loader_version.clone(),
        };
        let label = format!("Install {} {}", tuple.loader, tuple.loader_version);
        let op = self
            .ctx
            .operation_manager
            .register_for_instance(&label, &tuple.loader);

        let minecraft_root = self.ctx.paths.minecraft_runtime_root();
        let receipts_root = self.ctx.paths.loader_receipts();
        let cache_dir = self
            .ctx
            .paths
            .loader_cache()
            .join(&tuple.loader)
            .join(&tuple.minecraft_version)
            .join(&tuple.loader_version);
        let operation_id = op.id().clone();
        if let Err(e) = std::fs::create_dir_all(&cache_dir) {
            op.fail(e.to_string());
            return Err(LauncherError::InstanceCreateFailed);
        }
        self.ctx.progress_sink.report(ProgressEvent::new(
            operation_id.clone(),
            ProgressPhase::Installing,
            format!("Ensuring {} {}", tuple.loader, tuple.loader_version),
        ));
        let _lock = self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::LoaderInstall,
            "loader-install",
        )?;

        let expected_sha = loader_manifests::strip_sha_prefix(&entry.sha256);
        if !force_reinstall {
            if let Ok(adopted) = installed_profile::adopt_installed_profile(
                &minecraft_root,
                &receipts_root,
                &tuple,
                expected_sha,
            ) {
                op.complete();
                return Ok(summary_from_adoption(tuple, adopted, true));
            }
        }

        let conn = match crate::db::local_state_connection(&self.ctx.paths.local_state_db()) {
            Ok(c) => c,
            Err(_) => {
                op.fail("local state failed");
                return Err(LauncherError::LocalStateFailed);
            }
        };
        let policy = NetworkPolicy::from_db(&conn);
        if let Err(e) = policy.check(NetworkCategory::LoaderMetadataAndContent) {
            op.fail(e.to_string());
            return Err(e);
        }
        let file_path = cache_dir.join(&entry.file_name);
        let data = match verified_cache_hit(&file_path, &loader, &entry) {
            Some(bytes) => bytes,
            None => {
                self.ctx.progress_sink.report(ProgressEvent::new(
                    operation_id.clone(),
                    ProgressPhase::Downloading,
                    format!("Downloading {}", entry.file_name),
                ));
                match crate::download::download_verified_with_clients(
                    &self.ctx.http_clients,
                    &loader,
                    &entry.file_name,
                    &entry.file_type,
                    &entry.source_url,
                    &entry.sha256,
                )
                .await
                {
                    Ok(bytes) => {
                        if let Err(e) = atomic_write(&file_path, &bytes) {
                            op.fail(e.to_string());
                            return Err(e);
                        }
                        bytes
                    }
                    Err(e) => {
                        op.fail(e.to_string());
                        return Err(e);
                    }
                }
            }
        };

        let result = match entry.file_type.as_str() {
            "profile_json" => {
                install_profile_json(&minecraft_root, &receipts_root, &tuple, &entry, &data)
            }
            "installer_jar" => {
                self.install_forge_profile(
                    &minecraft_root,
                    &receipts_root,
                    &tuple,
                    &entry,
                    &data,
                    &policy,
                    force_reinstall,
                    &op,
                )
                .await
            }
            _ => {
                op.fail("unsupported loader");
                return Err(LauncherError::UnsupportedLoader);
            }
        };
        match result {
            Ok(summary) => {
                op.complete();
                self.ctx.event_sink.emit(CoreEvent::ModOperation {
                    operation_id,
                    instance_id: tuple.minecraft_version.clone(),
                    action: crate::event_sink::ModAction::Install,
                    status: EventStatus::Completed,
                    message: format!("Installed {} {}", tuple.loader, tuple.loader_version),
                });
                Ok(summary)
            }
            Err(e) => {
                op.fail(e.to_string());
                Err(e)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn install_forge_profile(
        &self,
        minecraft_root: &Path,
        receipts_root: &Path,
        tuple: &LoaderTuple,
        entry: &LoaderEntry,
        data: &[u8],
        policy: &NetworkPolicy,
        force_reinstall: bool,
        op: &OpHandle,
    ) -> LauncherResult<InstallReceiptSummary> {
        let backup = if force_reinstall {
            Some(backup_profile(minecraft_root, receipts_root, tuple)?)
        } else {
            None
        };
        let result = async {
            policy.check(NetworkCategory::LoaderMetadataAndContent)?;
            let java_path = self
                .resolve_installer_java(minecraft_root, &tuple.minecraft_version, policy, op)
                .await?;
            let staged = self
                .ctx
                .paths
                .staging_dir(&format!("loader-installer-{}", uuid::Uuid::new_v4()))?;
            std::fs::create_dir_all(&staged).map_err(|_| LauncherError::InstanceCreateFailed)?;
            let installer = staged.join(&entry.file_name);
            atomic_write(&installer, data)?;
            ensure_launcher_profile_stub(minecraft_root)?;
            let installer_result =
                run_installer_process(&java_path.path, &installer, &tuple.loader, minecraft_root)
                    .await?;
            if installer_result.exit_code != 0 {
                let detail = installer_result.failure_detail();
                return Err(LauncherError::Generic {
                    code: "ERR_INSTALLER_FAILED".into(),
                    message: format!(
                        "{} installer exited with status {}: {detail}",
                        tuple.loader, installer_result.exit_code
                    ),
                });
            }
            normalize_forge_profile(minecraft_root, tuple)?;
            let receipt = installed_profile::create_receipt_for_installed_profile(
                minecraft_root,
                receipts_root,
                tuple,
                loader_manifests::strip_sha_prefix(&entry.sha256),
                &entry.source_url,
                installer_result.exit_code,
            )
            .map_err(|issue| LauncherError::Generic {
                code: "ERR_PROFILE_CORRUPT".into(),
                message: issue.reasons.join("; "),
            })?;
            let _ = std::fs::remove_dir_all(&staged);
            Ok(InstallReceiptSummary {
                tuple: tuple.clone(),
                profile_id: receipt.profile_id,
                cache_hit: false,
                profile_stable_hash: receipt.profile_stable_hash,
                receipt_schema_version: receipt.schema_version,
                installer_exit_status: receipt.installer_exit_status,
            })
        }
        .await;
        match result {
            Ok(summary) => {
                if let Some(backup) = backup {
                    delete_backup(minecraft_root, &backup.profile_id);
                }
                Ok(summary)
            }
            Err(error) => {
                if let Some(backup) = backup {
                    restore_backup(minecraft_root, receipts_root, &backup);
                }
                Err(error)
            }
        }
    }

    async fn resolve_installer_java(
        &self,
        minecraft_root: &Path,
        minecraft_version: &str,
        policy: &NetworkPolicy,
        _op: &OpHandle,
    ) -> LauncherResult<JavaInstallation> {
        let version = crate::minecraft_metadata::ensure_base_version_metadata(
            minecraft_root,
            minecraft_version,
            policy,
        )
        .await?;
        let required = crate::java::java_requirement_from_version(&version).major;
        let runtimes_root = self.ctx.paths.java_runtimes_root();
        let candidates = tokio::task::spawn_blocking(move || {
            crate::java::detect_java_candidates(Some(&runtimes_root), None)
        })
        .await
        .map_err(|error| LauncherError::Generic {
            code: "ERR_JAVA_DISCOVERY".into(),
            message: error.to_string(),
        })?;
        if let Some(candidate) = candidates
            .into_iter()
            .find(|candidate| candidate.version == required)
        {
            return Ok(candidate);
        }
        let runtime_root = self.ctx.paths.java_runtimes_root();
        let catalog = self.ctx.runtime_catalog.snapshot();
        let policy = policy.clone();
        let lock_manager = self.ctx.lock_manager().clone();
        tokio::task::spawn_blocking(move || {
            crate::runtime_manager::ensure_runtime(
                &runtime_root,
                required,
                &catalog,
                &policy,
                None,
                Some(&lock_manager),
            )
        })
        .await
        .map_err(|error| LauncherError::Generic {
            code: "ERR_JAVA_PROVISION".into(),
            message: error.to_string(),
        })?
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LoaderVersionSummary {
    pub loader: String,
    pub mc_version: String,
    pub loader_version: String,
    pub file_type: String,
}

/// A preview of a loader version switch. Planning performs no mutation; the
/// caller reviews [`Self::current_report`] and then invokes
/// [`LoaderService::change_loader_version`] with an explicit target.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoaderChangePlan {
    pub instance_id: String,
    pub loader: String,
    pub minecraft_version: String,
    pub current_loader_version: String,
    /// The signed catalog version recommended for the current state, when one
    /// can be proven.
    pub recommended_loader_version: Option<String>,
    /// The compatibility report evaluated against the current loader tuple.
    pub current_report: LoaderCompatibilityReport,
}

/// The committed result of a loader version switch.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoaderChangeResult {
    pub instance_id: String,
    pub previous_loader_version: String,
    pub loader_version: String,
    /// Post-switch health report computed from a fresh scan.
    pub health: HealthReport,
}

fn summary_from_adoption(
    tuple: LoaderTuple,
    adopted: installed_profile::AdoptedProfile,
    cache_hit: bool,
) -> InstallReceiptSummary {
    InstallReceiptSummary {
        tuple,
        profile_id: adopted.profile_id,
        cache_hit,
        profile_stable_hash: adopted.profile_stable_hash,
        receipt_schema_version: adopted
            .receipt
            .as_ref()
            .map(|receipt| receipt.schema_version)
            .unwrap_or_default(),
        installer_exit_status: 0,
    }
}

fn no_loader_requirements_error(instance_id: &str) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_LOADER_CHANGE_NO_REQUIREMENTS".into(),
        message: format!(
            "Instance '{instance_id}' has no enabled mods declaring loader requirements."
        ),
    }
}

/// Commit a loader version switch manifest-first, then update the DB.
///
/// The instance manifest is written with the new version before the DB row is
/// touched, so a manifest write failure leaves the DB untouched. When the DB
/// update matches no row (instance absent or tuple changed concurrently), the
/// original manifest is restored through the same atomic writer and a clear
/// error is surfaced — including when the rollback itself fails.
#[allow(clippy::too_many_arguments)]
fn commit_loader_version_change(
    manifest_path: &Path,
    manifest: &InstanceManifest,
    conn: &rusqlite::Connection,
    instance_id: &str,
    loader: &str,
    minecraft_version: &str,
    previous_loader_version: &str,
    new_loader_version: &str,
) -> LauncherResult<()> {
    let mut updated = manifest.clone();
    updated.loader_version = new_loader_version.to_string();
    crate::helpers::atomic_write_manifest(manifest_path, &updated).map_err(|error| {
        LauncherError::Generic {
            code: "ERR_LOADER_CHANGE_MANIFEST".into(),
            message: format!("Failed to update the instance manifest: {error}"),
        }
    })?;

    let rollback = |manifest: &InstanceManifest| -> Result<(), String> {
        let mut rolled_back = manifest.clone();
        rolled_back.loader_version = previous_loader_version.to_string();
        crate::helpers::atomic_write_manifest(manifest_path, &rolled_back)
            .map_err(|error| format!("manifest rollback failed: {error}"))
    };

    let affected = crate::db::update_instance_loader_version(
        conn,
        instance_id,
        loader,
        minecraft_version,
        previous_loader_version,
        new_loader_version,
    )
    .map_err(|error| {
        if let Err(rollback_error) = rollback(&updated) {
            return LauncherError::Generic {
                code: "ERR_LOADER_CHANGE_ROLLBACK_FAILED".into(),
                message: format!(
                    "Loader switch could not be persisted ({error}) and {rollback_error}."
                ),
            };
        }
        LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: format!("Failed to persist the loader switch: {error}"),
        }
    })?;

    if affected != 1 {
        if let Err(rollback_error) = rollback(&updated) {
            return Err(LauncherError::Generic {
                code: "ERR_LOADER_CHANGE_ROLLBACK_FAILED".into(),
                message: format!(
                    "Loader switch could not be persisted (affected {affected} rows) and {rollback_error}."
                ),
            });
        }
        return Err(LauncherError::Generic {
            code: "ERR_LOADER_CHANGE_CONFLICT".into(),
            message: format!(
                "Loader switch aborted: the instance's loader tuple changed concurrently (affected {affected} rows)."
            ),
        });
    }
    Ok(())
}

/// Remove the durable launch-plan cache entries resolved for one instance.
///
/// The cache is advisory and keyed by its full resolution inputs, so a stale
/// entry would be revalidated anyway; this cleanup additionally prevents a
/// plan resolved against the old loader tuple from being reused before that
/// revalidation.
fn invalidate_launch_plan_cache(minecraft_root: &Path, instance_id: &str) {
    let plans_dir = minecraft_root.join("launch-plans");
    let Ok(entries) = std::fs::read_dir(&plans_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let belongs_to_instance = value
            .get("plan")
            .and_then(|plan| plan.get("instance_id"))
            .and_then(|value| value.as_str())
            == Some(instance_id);
        if belongs_to_instance {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn verified_cache_hit(path: &Path, loader: &str, entry: &LoaderEntry) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    let actual =
        crate::download::compute_loader_hash(loader, &entry.file_name, &entry.file_type, &data);
    (actual == loader_manifests::strip_sha_prefix(&entry.sha256)).then_some(data)
}

fn install_profile_json(
    minecraft_root: &Path,
    receipts_root: &Path,
    tuple: &LoaderTuple,
    entry: &LoaderEntry,
    data: &[u8],
) -> LauncherResult<InstallReceiptSummary> {
    let version_id = entry.file_name.trim_end_matches(".json");
    let target = minecraft_root
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));
    atomic_write(&target, data)?;
    installed_profile::create_receipt_for_profile_json(
        minecraft_root,
        receipts_root,
        tuple,
        loader_manifests::strip_sha_prefix(&entry.sha256),
        &entry.source_url,
        std::collections::BTreeMap::new(),
    )
    .map_err(|issue| LauncherError::Generic {
        code: "ERR_PROFILE_CORRUPT".into(),
        message: issue.reasons.join("; "),
    })?;
    let profile: serde_json::Value =
        serde_json::from_slice(data).map_err(|error| LauncherError::Generic {
            code: "ERR_PROFILE_CORRUPT".into(),
            message: error.to_string(),
        })?;
    Ok(InstallReceiptSummary {
        tuple: tuple.clone(),
        profile_id: installed_profile::derive_profile_id(tuple),
        cache_hit: false,
        profile_stable_hash: installed_profile::stable_profile_hash(&profile),
        receipt_schema_version: installed_profile::RECEIPT_SCHEMA_VERSION,
        installer_exit_status: 0,
    })
}

fn atomic_write(path: &Path, bytes: &[u8]) -> LauncherResult<()> {
    let parent = path.parent().ok_or(LauncherError::InstanceCreateFailed)?;
    std::fs::create_dir_all(parent).map_err(|_| LauncherError::InstanceCreateFailed)?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temp, bytes).map_err(|_| LauncherError::InstanceCreateFailed)?;
    std::fs::rename(&temp, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        LauncherError::Generic {
            code: "ERR_ATOMIC_WRITE".into(),
            message: error.to_string(),
        }
    })
}

async fn run_installer_process(
    java_path: &Path,
    installer_path: &Path,
    loader: &str,
    minecraft_root: &Path,
) -> LauncherResult<InstallerProcessResult> {
    let mut command = tokio::process::Command::new(java_path);
    command
        .args([
            std::ffi::OsString::from("-jar"),
            installer_path.as_os_str().to_owned(),
            std::ffi::OsString::from("--installClient"),
            minecraft_root.as_os_str().to_owned(),
        ])
        .current_dir(installer_path.parent().unwrap_or(minecraft_root))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Prevent an empty command prompt window from flashing while the Java
    // installer runs (the GUI app has no console of its own).
    crate::helpers::hide_console_window_async(&mut command);
    let mut child = command.spawn().map_err(|error| LauncherError::Generic {
        code: "ERR_INSTALLER_FAILED".into(),
        message: format!("Failed to spawn {loader} installer: {error}"),
    })?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_task = tokio::spawn(read_pipe_bounded(stdout));
    let stderr_task = tokio::spawn(read_pipe_bounded(stderr));
    let status = tokio::time::timeout(INSTALLER_TIMEOUT, child.wait()).await;
    let stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();
    match status {
        Ok(Ok(status)) => Ok(InstallerProcessResult {
            exit_code: status.code().unwrap_or(1),
            stdout,
            stderr,
        }),
        Ok(Err(error)) => Err(LauncherError::Generic {
            code: "ERR_INSTALLER_FAILED".into(),
            message: error.to_string(),
        }),
        Err(_) => Err(LauncherError::Generic {
            code: "ERR_INSTALLER_TIMEOUT".into(),
            message: format!("{loader} installer timed out"),
        }),
    }
}

fn ensure_launcher_profile_stub(minecraft_root: &Path) -> LauncherResult<()> {
    let profile = minecraft_root.join("launcher_profiles.json");
    if profile.exists() {
        return Ok(());
    }
    atomic_write(&profile, br#"{"profiles":{},"settings":{},"version":3}"#)
}

fn normalize_forge_profile(minecraft_root: &Path, tuple: &LoaderTuple) -> LauncherResult<()> {
    if tuple.loader != "forge" {
        return Ok(());
    }
    let expected_id = installed_profile::derive_profile_id(tuple);
    let expected_path = minecraft_root
        .join("versions")
        .join(&expected_id)
        .join(format!("{expected_id}.json"));
    if expected_path.exists() {
        return Ok(());
    }
    let generated_id = format!("{}-forge-{}", tuple.minecraft_version, tuple.loader_version);
    let generated_dir = minecraft_root.join("versions").join(&generated_id);
    let generated_path = generated_dir.join(format!("{generated_id}.json"));
    let raw = std::fs::read(&generated_path).map_err(|error| LauncherError::Generic {
        code: "ERR_PROFILE_MISSING".into(),
        message: format!("Forge generated profile was not found: {error}"),
    })?;
    let mut profile: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|error| LauncherError::Generic {
            code: "ERR_PROFILE_CORRUPT".into(),
            message: format!("Forge generated invalid profile JSON: {error}"),
        })?;
    profile["id"] = serde_json::Value::String(expected_id.clone());
    let normalized =
        serde_json::to_vec_pretty(&profile).map_err(|error| LauncherError::Generic {
            code: "ERR_PROFILE_CORRUPT".into(),
            message: error.to_string(),
        })?;
    atomic_write(&expected_path, &normalized)?;
    std::fs::remove_dir_all(generated_dir).map_err(|error| LauncherError::Generic {
        code: "ERR_PROFILE_PROMOTION".into(),
        message: format!("Could not remove the Forge installer profile after promotion: {error}"),
    })
}

struct InstallerProcessResult {
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl InstallerProcessResult {
    fn failure_detail(&self) -> String {
        let output = if self.stderr.is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        let text = String::from_utf8_lossy(output);
        let text = text.trim();
        if text.is_empty() {
            return "installer produced no diagnostic output".into();
        }
        let start = text
            .char_indices()
            .rev()
            .nth(8191)
            .map(|(index, _)| index)
            .unwrap_or(0);
        text[start..].to_string()
    }
}

async fn read_pipe_bounded<R>(pipe: Option<R>) -> Vec<u8>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use tokio::io::AsyncReadExt;
    let Some(mut pipe) = pipe else {
        return Vec::new();
    };
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    while let Ok(count) = pipe.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        let remaining = MAX_INSTALLER_OUTPUT_BYTES.saturating_sub(output.len() as u64);
        output.extend_from_slice(&buffer[..(count as u64).min(remaining) as usize]);
    }
    output
}

struct BackupState {
    tuple: LoaderTuple,
    profile_id: String,
    old_receipt_json: Option<String>,
}

fn backup_profile(
    minecraft_root: &Path,
    receipts_root: &Path,
    tuple: &LoaderTuple,
) -> LauncherResult<BackupState> {
    let profile_id = installed_profile::derive_profile_id(tuple);
    let version_dir = minecraft_root.join("versions").join(&profile_id);
    let backup_dir = minecraft_root
        .join("versions")
        .join(format!("{profile_id}{BACKUP_SUFFIX}"));
    if version_dir.exists() {
        if backup_dir.exists() {
            std::fs::remove_dir_all(&backup_dir)
                .map_err(|_| LauncherError::InstanceCreateFailed)?;
        }
        std::fs::rename(&version_dir, &backup_dir)
            .map_err(|_| LauncherError::InstanceCreateFailed)?;
    }
    let receipt_path = installed_profile::receipt_path(receipts_root, tuple);
    let old_receipt_json = if receipt_path.exists() {
        let content = std::fs::read_to_string(&receipt_path).ok();
        let _ = installed_profile::remove_receipt(receipts_root, tuple);
        content
    } else {
        None
    };
    Ok(BackupState {
        tuple: tuple.clone(),
        profile_id,
        old_receipt_json,
    })
}

fn restore_backup(minecraft_root: &Path, receipts_root: &Path, state: &BackupState) {
    let version_dir = minecraft_root.join("versions").join(&state.profile_id);
    let backup_dir = minecraft_root
        .join("versions")
        .join(format!("{}{BACKUP_SUFFIX}", state.profile_id));
    if backup_dir.exists() {
        let _ = std::fs::remove_dir_all(&version_dir);
        let _ = std::fs::rename(&backup_dir, &version_dir);
    }
    if let Some(json) = &state.old_receipt_json {
        let path = installed_profile::receipt_path(receipts_root, &state.tuple);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, json);
    }
}

fn delete_backup(minecraft_root: &Path, profile_id: &str) {
    let path = minecraft_root
        .join("versions")
        .join(format!("{profile_id}{BACKUP_SUFFIX}"));
    let _ = std::fs::remove_dir_all(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::Ctx;
    use crate::loader_compatibility::CurrentLoaderStatus;
    use crate::models::InstalledMod;
    use std::io::Write;

    fn context() -> (Ctx, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "agora-loader-service-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let ctx = Ctx::for_testing(root.clone());
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        (ctx, root)
    }

    fn write_jar(mods_dir: &std::path::Path, filename: &str, entries: &[(&str, &str)]) {
        let path = mods_dir.join(filename);
        let file = std::fs::File::create(&path).expect("create jar file");
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::FileOptions::default();
        for (name, content) in entries {
            zip.start_file(*name, opts).expect("start_file");
            zip.write_all(content.as_bytes()).expect("write_all");
        }
        zip.finish().expect("finish zip");
    }

    /// Seed an instance row, manifest, and mods directory. `mods` maps
    /// filename -> raw file content; use `write_jar` output or `write_jar`
    /// after seeding for parseable JARs.
    fn seed_instance(
        ctx: &Ctx,
        instance_id: &str,
        loader: &str,
        minecraft_version: &str,
        loader_version: &str,
        mods: &[(&str, Vec<u8>)],
    ) {
        let dir = ctx.paths.instance_dir(instance_id).unwrap();
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        for (filename, content) in mods {
            std::fs::write(dir.join("mods").join(filename), content).unwrap();
        }
        let manifest = InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: instance_id.into(),
            name: "Test".into(),
            created_from_pack: None,
            minecraft_version: minecraft_version.into(),
            loader: loader.into(),
            loader_version: loader_version.into(),
            is_locked: false,
            mods: mods
                .iter()
                .map(|(filename, _)| InstalledMod {
                    pack_managed: false,
                    filename: filename.to_string(),
                    registry_id: None,
                    modrinth_id: None,
                    source: "manual".into(),
                    source_url: None,
                    version: None,
                    sha256: String::new(),
                    installed_at: String::new(),
                    java_packages: vec![],
                    mod_jar_id: Some("moda".into()),
                    depends_on: vec![],
                    optional_deps: vec![],
                    incompatible_deps: vec![],
                    provided_mod_ids: vec![],
                    enabled: true,
                    content_type: "mod".into(),
                })
                .collect(),
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };
        std::fs::write(
            ctx.paths.instance_manifest(instance_id).unwrap(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let row = InstanceRow {
            instance_id: instance_id.into(),
            name: "Test".into(),
            minecraft_version: minecraft_version.into(),
            loader: loader.into(),
            loader_version: loader_version.into(),
            is_modpack: false,
            is_locked: false,
            last_launched_at: None,
            jvm_memory_mb: 4096,
            jvm_memory_mode: "auto".into(),
            jvm_gc: "auto".into(),
            jvm_custom_args: String::new(),
            jvm_always_pre_touch: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            java_path: None,
            java_incompatible_override: false,
            icon_path: None,
            launch_mode_override: "auto".into(),
            import_source: None,
        };
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
    }

    fn fabric_requirement_jar() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
            let opts = zip::write::FileOptions::default();
            zip.start_file("fabric.mod.json", opts).unwrap();
            zip.write_all(
                br#"{"id":"moda","version":"1.0","depends":{"fabricloader":">=0.19.0"}}"#,
            )
            .unwrap();
            zip.finish().unwrap();
        }
        bytes
    }

    #[test]
    fn plan_loader_change_reports_current_state_without_mutation() {
        let (ctx, root) = context();
        seed_instance(
            &ctx,
            "plan-test",
            "fabric",
            "1.21",
            "0.18.6",
            &[("moda.jar", fabric_requirement_jar())],
        );
        let service = LoaderService::new(ctx.clone());
        let plan = service.plan_loader_change("plan-test").unwrap();
        assert_eq!(plan.current_loader_version, "0.18.6");
        assert_eq!(plan.loader, "fabric");
        assert_eq!(plan.minecraft_version, "1.21");
        assert_eq!(
            plan.current_report.current_status,
            CurrentLoaderStatus::Incompatible
        );
        assert!(plan.recommended_loader_version.is_some());

        // No mutation: manifest and DB still carry the old tuple.
        let manifest: InstanceManifest = serde_json::from_slice(
            // allow-raw-instance-manifest
            &std::fs::read(ctx.paths.instance_manifest("plan-test").unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest.loader_version, "0.18.6");
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let row = crate::db::get_instance(&conn, "plan-test")
            .unwrap()
            .unwrap();
        assert_eq!(row.loader_version, "0.18.6");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn change_loader_version_rejects_target_absent_from_signed_catalog() {
        let (ctx, root) = context();
        seed_instance(
            &ctx,
            "change-unknown",
            "fabric",
            "1.21",
            "0.18.6",
            &[("moda.jar", fabric_requirement_jar())],
        );
        let service = LoaderService::new(ctx.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let error = rt
            .block_on(service.change_loader_version("change-unknown", "9.9.9"))
            .unwrap_err();
        assert_eq!(error.code(), "ERR_UNSUPPORTED_LOADER");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn change_loader_version_rejects_target_with_unsatisfied_hard_requirement() {
        let (ctx, root) = context();
        seed_instance(&ctx, "change-reject", "fabric", "1.21", "0.19.0", &[]);
        let dir = ctx.paths.instance_dir("change-reject").unwrap();
        write_jar(
            &dir.join("mods"),
            "moda.jar",
            &[(
                "fabric.mod.json",
                r#"{"id":"moda","version":"1.0","depends":{"fabricloader":">=0.20.0"}}"#,
            )],
        );
        let service = LoaderService::new(ctx.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let error = rt
            .block_on(service.change_loader_version("change-reject", "0.18.6"))
            .unwrap_err();
        assert_eq!(error.code(), "ERR_LOADER_CHANGE_REJECTED");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn commit_loader_version_change_persists_manifest_then_db() {
        let (ctx, root) = context();
        seed_instance(&ctx, "commit-ok", "fabric", "1.21", "0.18.6", &[]);
        let manifest_path = ctx.paths.instance_manifest("commit-ok").unwrap();
        let manifest = crate::helpers::read_manifest(&manifest_path).unwrap();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        commit_loader_version_change(
            &manifest_path,
            &manifest,
            &conn,
            "commit-ok",
            "fabric",
            "1.21",
            "0.18.6",
            "0.19.0",
        )
        .unwrap();
        let updated: InstanceManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(updated.loader_version, "0.19.0");
        let row = crate::db::get_instance(&conn, "commit-ok")
            .unwrap()
            .unwrap();
        assert_eq!(row.loader_version, "0.19.0");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn commit_loader_version_change_rolls_back_manifest_when_db_row_missing() {
        let (ctx, root) = context();
        let manifest_path = ctx.paths.instance_manifest("ghost").unwrap();
        std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
        let manifest = InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: "ghost".into(),
            name: "Ghost".into(),
            created_from_pack: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.18.6".into(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let error = commit_loader_version_change(
            &manifest_path,
            &manifest,
            &conn,
            "ghost",
            "fabric",
            "1.21",
            "0.18.6",
            "0.19.0",
        )
        .unwrap_err();
        assert_eq!(error.code(), "ERR_LOADER_CHANGE_CONFLICT");
        let restored: InstanceManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(restored.loader_version, "0.18.6");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn commit_loader_version_change_rolls_back_manifest_when_tuple_changed() {
        let (ctx, root) = context();
        seed_instance(&ctx, "commit-race", "fabric", "1.21", "0.18.6", &[]);
        // Simulate a concurrent change: the DB row already moved to 0.19.0.
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let affected = crate::db::update_instance_loader_version(
            &conn,
            "commit-race",
            "fabric",
            "1.21",
            "0.18.6",
            "0.19.0",
        )
        .unwrap();
        assert_eq!(affected, 1);
        let manifest_path = ctx.paths.instance_manifest("commit-race").unwrap();
        let manifest = crate::helpers::read_manifest(&manifest_path).unwrap();
        let error = commit_loader_version_change(
            &manifest_path,
            &manifest,
            &conn,
            "commit-race",
            "fabric",
            "1.21",
            "0.18.6",
            "0.19.2",
        )
        .unwrap_err();
        assert_eq!(error.code(), "ERR_LOADER_CHANGE_CONFLICT");
        let restored: InstanceManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(restored.loader_version, "0.18.6");
        // The DB keeps its newer tuple.
        let row = crate::db::get_instance(&conn, "commit-race")
            .unwrap()
            .unwrap();
        assert_eq!(row.loader_version, "0.19.0");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invalidate_launch_plan_cache_removes_only_this_instances_plans() {
        let minecraft_root = std::env::temp_dir().join(format!(
            "agora-launch-plans-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let plans_dir = minecraft_root.join("launch-plans");
        std::fs::create_dir_all(&plans_dir).unwrap();
        std::fs::write(
            plans_dir.join("mine.json"),
            br#"{"schema_version":1,"key":"k","plan":{"instance_id":"mine","loader_type":"fabric"}}"#,
        )
        .unwrap();
        std::fs::write(
            plans_dir.join("theirs.json"),
            br#"{"schema_version":1,"key":"k","plan":{"instance_id":"theirs","loader_type":"fabric"}}"#,
        )
        .unwrap();
        std::fs::write(plans_dir.join("notes.txt"), b"unrelated").unwrap();

        invalidate_launch_plan_cache(&minecraft_root, "mine");
        assert!(!plans_dir.join("mine.json").exists());
        assert!(plans_dir.join("theirs.json").exists());
        assert!(plans_dir.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&minecraft_root);
    }
}
