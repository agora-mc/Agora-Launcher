//! Minecraft-version migration *execution* — the counterpart to the read-only
//! [`crate::migration_report`].
//!
//! The report answers "can this instance move to a newer Minecraft version?".
//! This module answers "move it" — and refuses, loudly, when it cannot.
//!
//! # Why this is not an `InstallAction::BatchUpdate`
//!
//! Reusing [`crate::install_pipeline`] for the content half was considered and
//! rejected. The pipeline's single commit point is the atomic manifest rename
//! ([`crate::install_pipeline::ResolvedInstallPlan`], module docs), and the
//! future manifest it builds can change `loader_version` via
//! `LoaderChangePlan` but *never* `minecraft_version` — there is no field to
//! carry it. A migration whose files land through the pipeline and whose
//! version fields flip in a second write is a migration that can half-commit.
//! Worse, the pipeline's loader-change machinery resolves everything against
//! the instance's *current* Minecraft tuple: `execute_plan` calls
//! `ensure_installed(loader, manifest.minecraft_version /* old */, …)` and the
//! staged-loader validation evaluates the signed catalog for the old version.
//! Threading a changing Minecraft version through would mean bending the
//! pipeline at exactly the points where it is load-bearing.
//!
//! So this module owns a purpose-built transaction that follows the
//! pipeline's *rules* rather than its code:
//!
//! 1. **Plan** (read-only): classification via the shared report, per-entry
//!    target-build resolution carried by
//!    [`TargetBuildInfo`](crate::migration_report::TargetBuildInfo) (no second
//!    lookup pass), loader target chosen from the signed catalog.
//! 2. **Provision the loader** into the shared cache *before* any instance
//!    mutation (same ordering argument as `install_pipeline`: a stray
//!    installed profile is harmless cache material).
//! 3. **Stage** every replacement artifact and verify it against the
//!    published hashes (SHA-512 and/or SHA-1) and size.
//! 4. **Build the future manifest** in full — swapped content entries *and*
//!    `minecraft_version` *and* `loader_version` — and serialize it to disk
//!    before touching live state, so serialization failure is pre-mutation.
//! 5. **Snapshot** (mandatory) the pre-migration instance.
//! 6. **Apply** with a reversible journal; the atomic manifest rename is the
//!    single commit point, so version fields and content flip together.
//! 7. **Commit the DB row** with a guarded compare-and-set (mirroring
//!    `db::update_instance_loader_version`), and if that fails, undo the
//!    committed manifest and every file move.
//! 8. **Health-gate** the committed instance: blockers trigger a full
//!    rollback (snapshot restore + DB revert), because a half-migrated
//!    instance that crash-loops is worse than a refusal.
//!
//! # Failure semantics
//!
//! Every failure path ends in one of two outcomes:
//!
//! * [`MigrationOutcome::Blocked`] / pre-mutation [`MigrationOutcome::Failed`]
//!   — the instance was never touched; it launches exactly as before.
//! * [`MigrationOutcome::RolledBack`] — the instance was mutated and the undo
//!   verifiably restored the pre-migration state (files, manifest, DB row).
//!   A post-commit health failure lands here.
//!
//! Only [`MigrationOutcome::Failed`] with `rolled_back == false` means "state
//! may need manual recovery"; the recovery snapshot id is always surfaced in
//! that case.
//!
//! # Refusal rules (no silent skips)
//!
//! * Any content entry that is not `ready` (not-yet, abandoned, superseded,
//!   unknown, unclassifiable, needs-review) blocks the whole migration.
//! * A `ready` entry whose checker could not name a concrete
//!   [`TargetBuildInfo`] blocks the migration — "has a build" is not
//!   "can install the build".
//! * Update-pinned, pack-managed, modpack, and locked instances block.
//! * `saves/` worlds are deliberately *left behind* (with a warning): they
//!   upgrade in place on first launch and are not single-file artifacts, so
//!   they are neither classified nor replaced.
//!
//! # Seams (all offline-testable)
//!
//! Classification reuses [`ModrinthChecker`] exactly like the report does;
//! loader installation goes through [`LoaderProvisioner`] (production:
//! [`LiveLoaderProvisioner`] over [`crate::loader_service::LoaderService`]);
//! artifact bytes come through [`ArtifactFetcher`] (production:
//! [`HttpArtifactFetcher`] over the host-allowlisted Modrinth downloader).

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::health::HealthReport;
use crate::migration_report::{
    generate_migration_report, MigrationReport, MigrationStatus, ModrinthChecker, SuccessorLookup,
    TargetBuildInfo,
};
use crate::models::{InstalledMod, InstanceManifest};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Seams
// ---------------------------------------------------------------------------

/// Installs loader material for a `(loader, minecraft_version, loader_version)`
/// tuple into the shared cache. Production wraps
/// [`crate::loader_service::LoaderService::ensure_installed`]; tests inject a
/// recorder that can be made to fail at will.
#[async_trait]
pub trait LoaderProvisioner: Send + Sync {
    async fn ensure_loader(
        &self,
        loader: &str,
        minecraft_version: &str,
        loader_version: &str,
    ) -> Result<(), LauncherError>;
}

/// Live provisioner over the core loader service.
pub struct LiveLoaderProvisioner {
    service: crate::loader_service::LoaderService,
}

impl LiveLoaderProvisioner {
    pub fn new(service: crate::loader_service::LoaderService) -> Self {
        Self { service }
    }
}

#[async_trait]
impl LoaderProvisioner for LiveLoaderProvisioner {
    async fn ensure_loader(
        &self,
        loader: &str,
        minecraft_version: &str,
        loader_version: &str,
    ) -> Result<(), LauncherError> {
        self.service
            .ensure_installed(loader, minecraft_version, loader_version, false)
            .await
            .map(|_receipt| ())
    }
}

/// Fetches raw artifact bytes from a URL. Production wraps the allowlisted
/// download helpers; tests inject canned bytes with matching hashes so staging
/// verification is exercised without a network.
#[async_trait]
pub trait ArtifactFetcher: Send + Sync {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, LauncherError>;
}

/// Live fetcher through the shared Modrinth-CDN download path.
pub struct HttpArtifactFetcher;

#[async_trait]
impl ArtifactFetcher for HttpArtifactFetcher {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, LauncherError> {
        crate::download::download_mod_bytes_standalone(url).await
    }
}

/// The injected dependency bag for [`migrate_instance`].
pub struct MigrationServices<'a> {
    pub checker: &'a dyn ModrinthChecker,
    pub successors: &'a dyn SuccessorLookup,
    pub provisioner: &'a dyn LoaderProvisioner,
    pub fetcher: &'a dyn ArtifactFetcher,
    /// Skip the post-commit health scan (mirrors the install pipeline's
    /// override). The health scan defaults to on.
    pub skip_health_scan: bool,
}

// ---------------------------------------------------------------------------
// Plan types
// ---------------------------------------------------------------------------

/// One installed entry → its concrete replacement build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedSwap {
    /// Filename recorded in the manifest today.
    pub old_filename: String,
    /// Content type of the old entry (`mod`, `resourcepack`, …).
    pub content_type: String,
    /// Whether the old entry was enabled. Disabled entries are replaced
    /// under a `.disabled` live name so the loader keeps ignoring them.
    pub old_enabled: bool,
    /// Filename of the replacement artifact on disk (may carry `.disabled`).
    pub new_filename: String,
    /// The concrete target build (id, filename, URL, published hashes).
    pub target: TargetBuildInfo,
}

/// A fully resolved, read-only migration plan. Building one mutates nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlan {
    pub instance_id: String,
    pub source_version: String,
    pub target_version: String,
    pub loader: String,
    pub source_loader_version: String,
    /// Loader build to reinstall for the target version. `None` for vanilla
    /// (no loader step, `loader_version` kept as-is).
    pub target_loader_version: Option<String>,
    pub swaps: Vec<PlannedSwap>,
    /// Entries that will be left at their current version — not ready, pinned,
    /// or unresolvable.
    ///
    /// Reported rather than fatal: one abandoned mod out of forty should not
    /// hold an instance on an old Minecraft version. Execution still refuses
    /// unless the caller acknowledges these, so nothing is skipped silently.
    #[serde(default)]
    pub blockers: Vec<RejectionReason>,
    pub warnings: Vec<String>,
    /// Deterministic fingerprint over the plan content; also names the
    /// staging directory and the recovery snapshot.
    pub fingerprint: String,
    /// Hash of the instance's live file index at plan time. Execution
    /// re-computes it (same rule as the install pipeline's
    /// `instance_state_hash`) so any drift — including files that are not
    /// tracked by the manifest — aborts the migration.
    pub instance_state_hash: String,
    /// The full readiness report the plan was derived from.
    pub report: MigrationReport,
}

/// Why a migration was refused. `filename` scopes content-related reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectionReason {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

/// A read-only planning refusal: nothing was changed, and the reasons (plus
/// the report, once classification had run) explain what would have to move.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationRejection {
    pub reasons: Vec<RejectionReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<MigrationReport>,
}

impl MigrationRejection {
    fn infra(error: LauncherError) -> Self {
        Self {
            reasons: vec![RejectionReason {
                code: error.code(),
                message: error.to_string(),
                filename: None,
            }],
            report: None,
        }
    }
}

/// Typed outcome of a migration attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum MigrationOutcome {
    /// Content, loader, manifest and DB row all moved to the target version.
    Migrated {
        instance_id: String,
        from_version: String,
        to_version: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        loader_version: Option<String>,
        replaced: Vec<String>,
        snapshot_id: String,
        warnings: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        health: Option<HealthReport>,
    },
    /// Refused before touching anything. The instance is untouched.
    Blocked {
        reasons: Vec<RejectionReason>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        report: Option<MigrationReport>,
    },
    /// Mutated mid-way and verifiably restored to the pre-migration state.
    RolledBack {
        phase: String,
        error: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snapshot_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        health_report: Option<HealthReport>,
    },
    /// Failure. `rolled_back == true` means the undo completed; `false` means
    /// the instance may be mid-state and the `snapshot_id` (when present) is
    /// the recovery point.
    Failed {
        phase: String,
        error: String,
        rolled_back: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snapshot_id: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Planning (read-only)
// ---------------------------------------------------------------------------

/// Plan a version migration for an existing instance. Performs zero mutation.
///
/// Fails closed: every migratable entry must classify `ready` *and* carry a
/// concrete [`TargetBuildInfo`], the loader target must exist in the signed
/// catalog, and the instance must not be locked/pack-managed/busy-with-pins.
pub async fn plan_migration(
    ctx: &Ctx,
    instance_id: &str,
    target_version: &str,
    checker: &dyn ModrinthChecker,
    successors: &dyn SuccessorLookup,
) -> Result<MigrationPlan, MigrationRejection> {
    let instance_id = sanitize_instance_id(instance_id).map_err(MigrationRejection::infra)?;
    let target_version = target_version.trim().to_string();
    if target_version.is_empty() {
        return Err(reject(
            "ERR_MIGRATION_NOOP",
            "Target Minecraft version must not be empty.",
            None,
        ));
    }

    let (manifest, row) = match load_instance(ctx, &instance_id) {
        Ok(pair) => pair,
        Err(error) => return Err(MigrationRejection::infra(error)),
    };
    if target_version == manifest.minecraft_version {
        return Err(reject(
            "ERR_MIGRATION_NOOP",
            format!("Instance is already on Minecraft {target_version}; nothing to migrate."),
            None,
        ));
    }

    let mut warnings = Vec::new();

    // Instance-level refusal. The lock is the *only* consent gate, and it is
    // deliberately the same one for every kind of deviation: modpack instances
    // are locked on install, so unlocking one is already an explicit "I accept
    // that this will diverge from the pack author's build". Refusing packs a
    // second time here would just block the case this feature exists for.
    if manifest.is_locked || row.is_locked {
        return Err(reject(
            "ERR_INSTANCE_LOCKED",
            "This instance is locked. Unlock it explicitly before migrating.".to_string(),
            None,
        ));
    }
    if row.is_modpack || manifest.pack_origin.is_some() || manifest.created_from_pack.is_some() {
        warnings.push(
            "This instance came from a modpack. Migrating it moves away from the version the pack \
             author published, which most authors do not support; a later pack update may overwrite \
             mod versions and configuration changes."
                .to_string(),
        );
    }

    // Worlds are left behind: saves upgrade in place on first launch and are
    // directories (or archive+folder pairs), not single-file content
    // artifacts the swap machinery understands.
    let migratable: Vec<InstalledMod> = manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
        .cloned()
        .collect();
    if !manifest.worlds.is_empty() {
        warnings.push(format!(
            "{} installed world(s) are left behind and will upgrade in place on first launch.",
            manifest.worlds.len()
        ));
    }
    // Pinning is durable user intent, so a pinned entry is never swapped — but
    // it is reported rather than fatal, so one pinned mod cannot hold the whole
    // instance on an old Minecraft version.
    let mut blockers: Vec<RejectionReason> = Vec::new();
    let pinned: BTreeSet<String> = migratable
        .iter()
        .filter(|entry| entry.update_pinned)
        .map(|entry| entry.filename.clone())
        .collect();
    for filename in &pinned {
        blockers.push(RejectionReason {
            code: "ERR_MIGRATION_PINNED".into(),
            message: format!(
                "{filename} is pinned to its installed version and will be left as it is."
            ),
            filename: Some(filename.clone()),
        });
    }

    let mut entries = migratable;
    crate::migration_report::enrich_with_registry(ctx, &mut entries);

    let report = generate_migration_report(
        &instance_id,
        &manifest.minecraft_version,
        &target_version,
        &manifest.loader,
        &entries,
        checker,
        successors,
    )
    .await;
    blockers.extend(classify_rejection(&report));
    // Integrity problems that no user decision can make safe stay fatal.
    let mut rejection: Vec<RejectionReason> = Vec::new();

    // Build swaps from the ready entries; any ready-but-unresolvable entry is
    // a hard stop, never a silent skip.
    let mut swaps: Vec<PlannedSwap> = Vec::new();
    let mut seen_targets: BTreeSet<(String, String)> = BTreeSet::new();
    for entry in &report.mods {
        if entry.status != MigrationStatus::Ready {
            continue;
        }
        if pinned.contains(&entry.filename) {
            continue;
        }
        let Some(target) = entry.target_build.clone() else {
            blockers.push(RejectionReason {
                code: "ERR_MIGRATION_UNRESOLVABLE".into(),
                message: format!(
                    "{} has a build for the target version but Agora could not resolve which one; it will be left as it is.",
                    entry.filename
                ),
                filename: Some(entry.filename.clone()),
            });
            continue;
        };
        if validate_filename(&target.filename).is_err()
            || target.download_url.trim().is_empty()
            || !has_verifiable_hash(&target)
        {
            // Unsafe filename / no URL / no hash is an integrity problem, not
            // a preference — there is no version of this the user can consent
            // to, so it stays fatal.
            rejection.push(RejectionReason {
                code: "ERR_MIGRATION_UNRESOLVABLE".into(),
                message: format!(
                    "{} resolves to a target build with an unsafe filename, no download URL, or no published hash to verify against.",
                    entry.filename
                ),
                filename: Some(entry.filename.clone()),
            });
            continue;
        }
        let old = entries.iter().find(|m| m.filename == entry.filename);
        let Some(old) = old else { continue };
        let old_enabled = old.enabled;
        let new_filename = if old_enabled {
            target.filename.clone()
        } else {
            format!("{}.disabled", target.filename)
        };
        let bucket = normalized_content_type(&old.content_type).to_string();
        if !seen_targets.insert((bucket.clone(), new_filename.clone())) {
            rejection.push(RejectionReason {
                code: "ERR_MIGRATION_DUPLICATE_TARGET".into(),
                message: format!(
                    "{} and another entry both resolve to {new_filename}; refusing an ambiguous swap.",
                    old.filename
                ),
                filename: Some(old.filename.clone()),
            });
            continue;
        }
        if target.is_prerelease {
            warnings.push(format!(
                "{} will migrate to prerelease build {} ({}).",
                entry.filename, target.version_number, target.filename
            ));
        }
        swaps.push(PlannedSwap {
            old_filename: old.filename.clone(),
            content_type: bucket,
            old_enabled,
            new_filename,
            target,
        });
    }
    swaps.sort_by(|a, b| {
        (&a.content_type, &a.old_filename).cmp(&(&b.content_type, &b.old_filename))
    });

    if !rejection.is_empty() {
        return Err(MigrationRejection {
            reasons: rejection,
            report: Some(report),
        });
    }

    // Loader target from the signed catalog (never from the network).
    let target_loader_version = if is_vanilla(&manifest.loader) {
        None
    } else {
        let catalog = crate::loader_manifests::active_catalog();
        let candidates = catalog.list_versions(&manifest.loader, &target_version);
        let best = candidates
            .iter()
            .map(|entry| entry.loader_version.as_str())
            .max_by(|a, b| crate::version_match::compare_versions(a, b));
        match best {
            Some(version) => Some(version.to_string()),
            None => {
                return Err(reject(
                    "ERR_MIGRATION_NO_LOADER",
                    format!(
                        "The signed loader catalog has no {} build for Minecraft {target_version}; the loader cannot be reinstalled.",
                        manifest.loader
                    ),
                    None,
                ))
            }
        }
    };

    let mut plan = MigrationPlan {
        instance_id,
        source_version: manifest.minecraft_version,
        target_version,
        loader: manifest.loader,
        source_loader_version: manifest.loader_version,
        target_loader_version,
        swaps,
        blockers,
        warnings,
        fingerprint: String::new(),
        instance_state_hash: String::new(),
        report,
    };
    plan.warnings.extend(plan.report.warnings.iter().cloned());
    let instance_dir = ctx
        .paths
        .instance_dir(&plan.instance_id)
        .map_err(MigrationRejection::infra)?;
    let live_index = crate::snapshot::live_file_index(&instance_dir)
        .map_err(|error| reject("ERR_MIGRATION_STATE", error, None))?;
    plan.instance_state_hash = hash_serializable(&live_index);
    plan.fingerprint = plan_fingerprint(&plan);
    Ok(plan)
}

fn classify_rejection(report: &MigrationReport) -> Vec<RejectionReason> {
    let mut reasons = Vec::new();
    for entry in &report.mods {
        let (code, detail) = match entry.status {
            MigrationStatus::Ready => continue,
            MigrationStatus::NotYet => (
                "ERR_MIGRATION_NOT_READY",
                "has no build for the target version yet",
            ),
            MigrationStatus::Abandoned => (
                "ERR_MIGRATION_NOT_READY",
                "appears abandoned upstream and has no target build",
            ),
            MigrationStatus::Superseded => (
                "ERR_MIGRATION_NOT_READY",
                "is superseded and will not move on its own",
            ),
            MigrationStatus::Unknown => (
                "ERR_MIGRATION_NOT_READY",
                "could not be checked (network or privacy gate); the report is incomplete",
            ),
            MigrationStatus::Unclassifiable => (
                "ERR_MIGRATION_NEEDS_REVIEW",
                "has no Modrinth identity and cannot be checked automatically",
            ),
        };
        reasons.push(RejectionReason {
            code: code.into(),
            message: format!("{} {}.", entry.filename, detail),
            filename: Some(entry.filename.clone()),
        });
    }
    reasons
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// Execute a previously planned migration. All-or-nothing with the rollback
/// semantics documented at the module head.
pub async fn execute_migration(
    ctx: &Ctx,
    plan: &MigrationPlan,
    provisioner: &dyn LoaderProvisioner,
    fetcher: &dyn ArtifactFetcher,
    skip_health_scan: bool,
    accept_blockers: bool,
) -> MigrationOutcome {
    // The plan lists what will be left behind; proceeding past it has to be a
    // decision the user actually made, not a default.
    if !plan.blockers.is_empty() && !accept_blockers {
        return MigrationOutcome::Blocked {
            reasons: plan.blockers.clone(),
            report: Some(plan.report.clone()),
        };
    }
    let failed = |phase: &str, error: String| MigrationOutcome::Failed {
        phase: phase.to_string(),
        error,
        rolled_back: false,
        snapshot_id: None,
    };

    let instance_id = match sanitize_instance_id(&plan.instance_id) {
        Ok(id) => id,
        Err(error) => return failed("precondition", error.to_string()),
    };
    let _guard = match ctx.lock_manager.acquire(
        crate::lock_manager::LockResource::Instance(instance_id.clone()),
        "version-migration",
    ) {
        Ok(guard) => guard,
        Err(error) => return failed("lock", error.to_string()),
    };
    if ctx
        .process_session_manager
        .list()
        .iter()
        .any(|session| session.instance_id == instance_id)
    {
        return failed(
            "precondition",
            format!("Instance '{instance_id}' is running; stop it before migrating."),
        );
    }

    let instance_dir = match ctx.paths.instance_dir(&instance_id) {
        Ok(dir) => dir,
        Err(error) => return failed("precondition", error.to_string()),
    };

    match crate::snapshot::snapshot_readiness(&instance_dir) {
        crate::snapshot::SnapshotReadiness::Ready => {}
        crate::snapshot::SnapshotReadiness::Pending => {
            return failed(
                "precondition",
                "The instance is still finalizing its initial recovery snapshot. Try again shortly."
                    .into(),
            );
        }
        crate::snapshot::SnapshotReadiness::Failed => {
            return failed(
                "precondition",
                crate::snapshot::snapshot_readiness_error(&instance_dir)
                    .unwrap_or_else(|| "The initial recovery snapshot failed.".into()),
            );
        }
    }

    // Precondition revalidation: the plan must still describe reality.
    let (manifest, row) = match load_instance(ctx, &instance_id) {
        Ok(pair) => pair,
        Err(error) => return failed("precondition", error.to_string()),
    };
    if manifest.is_locked || row.is_locked || row.is_modpack {
        return failed(
            "precondition",
            "The instance became locked or pack-managed after planning.".into(),
        );
    }
    if let Some(reason) = plan_is_stale(plan, &manifest) {
        return failed("precondition", reason);
    }
    // Same freshness rule the install pipeline applies: the live file index
    // must match what the plan reviewed, so untracked drift (a file dropped
    // into mods/ without a manifest entry) aborts the migration too.
    let live_index = match crate::snapshot::live_file_index(&instance_dir) {
        Ok(index) => index,
        Err(error) => return failed("precondition", error),
    };
    if hash_serializable(&live_index) != plan.instance_state_hash {
        return failed(
            "precondition",
            "The instance changed after planning. Re-plan the migration.".into(),
        );
    }

    // 1. Loader material first: shared cache, harmless residue, zero instance
    //    risk (same ordering rule the install pipeline uses).
    if let Some(loader_version) = &plan.target_loader_version {
        if let Err(error) = provisioner
            .ensure_loader(&plan.loader, &plan.target_version, loader_version)
            .await
        {
            return failed("loader-provision", error.to_string());
        }
    }

    let staging_dir = instance_dir
        .join(".agora")
        .join("staging")
        .join(format!("migration-{}", plan.fingerprint));
    let artifacts_dir = staging_dir.join("artifacts");

    // 2. Stage + verify every replacement artifact.
    if let Err(error) = std::fs::create_dir_all(&artifacts_dir) {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return failed(
            "staging",
            format!("failed to create staging directory: {error}"),
        );
    }
    for swap in &plan.swaps {
        let bytes = match fetcher.fetch(&swap.target.download_url).await {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging_dir);
                return failed(
                    "staging",
                    format!("failed to download {}: {error}", swap.old_filename),
                );
            }
        };
        if let Err(error) = verify_artifact_bytes(&bytes, &swap.target) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            return failed(
                "staging",
                format!("verification failed for {}: {error}", swap.old_filename),
            );
        }
        if let Err(error) = write_staged(&artifacts_dir.join(staging_artifact_name(swap)), &bytes) {
            let _ = std::fs::remove_dir_all(&staging_dir);
            return failed("staging", error);
        }
    }

    // 3. Build the future manifest (content + version fields) and serialize
    //    it before any mutation, so serialization/disk failures are free.
    let (future_manifest, future_manifest_bytes) =
        match build_future_manifest(plan, &manifest, &artifacts_dir) {
            Ok(pair) => pair,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging_dir);
                return failed("prepare", error);
            }
        };
    let prepared_path = staging_dir.join("instance_manifest.next.json");
    if let Err(error) = write_staged(&prepared_path, &future_manifest_bytes) {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return failed("prepare", error);
    }

    // 4. Mandatory recovery snapshot of the pre-migration state.
    let snapshot = match crate::snapshot::create_snapshot(
        &instance_dir,
        Some(&format!(
            "migration-{}",
            &plan.fingerprint[..16.min(plan.fingerprint.len())]
        )),
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging_dir);
            return failed("snapshot", format!("Snapshot failed before apply: {error}"));
        }
    };
    let snapshot_id = snapshot.id.clone();

    // 5. Apply: reversible journal, atomic manifest rename as the commit.
    let manifest_path = instance_dir.join("instance_manifest.json");
    let original_manifest = match std::fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging_dir);
            return failed(
                "apply",
                format!("failed to read current manifest during apply: {error}"),
            );
        }
    };
    let mut journal = ApplyJournal::default();
    let commit_temp_path = instance_dir.join(format!(
        "instance_manifest.json.tmp.migration-{}",
        &plan.fingerprint[..16.min(plan.fingerprint.len())]
    ));
    let applied = apply_migration_moves(
        plan,
        &instance_dir,
        &staging_dir,
        &commit_temp_path,
        &mut journal,
    );
    if let Err(error) = applied {
        let undo = undo_migration(
            &journal,
            &manifest_path,
            &original_manifest,
            &commit_temp_path,
        );
        let _ = std::fs::remove_dir_all(&staging_dir);
        return match undo {
            Ok(()) => MigrationOutcome::RolledBack {
                phase: "apply".into(),
                error: format!("Apply failed before commit; every change was reversed: {error}"),
                snapshot_id: Some(snapshot_id),
                health_report: None,
            },
            Err(undo_error) => MigrationOutcome::Failed {
                phase: "apply".into(),
                error: format!("Apply failed ({error}) and undo did not complete cleanly ({undo_error}); restore snapshot {snapshot_id} to recover"),
                rolled_back: false,
                snapshot_id: Some(snapshot_id),
            },
        };
    }

    // 6. DB row commit with guarded compare-and-set. On failure, undo the
    //    committed manifest and every file move.
    let db_result = commit_migration_db(
        ctx,
        &instance_id,
        &plan.loader,
        &plan.source_version,
        &plan.source_loader_version,
        &plan.target_version,
        &new_loader_version(plan),
    );
    if let Err(error) = db_result {
        let undo = undo_migration(
            &journal,
            &manifest_path,
            &original_manifest,
            &commit_temp_path,
        );
        let _ = std::fs::remove_dir_all(&staging_dir);
        return match undo {
            Ok(()) => MigrationOutcome::RolledBack {
                phase: "db-commit".into(),
                error: format!("The instance row could not be committed; files and manifest were restored: {error}"),
                snapshot_id: Some(snapshot_id),
                health_report: None,
            },
            Err(undo_error) => MigrationOutcome::Failed {
                phase: "db-commit".into(),
                error: format!("DB commit failed ({error}) and undo did not complete cleanly ({undo_error}); restore snapshot {snapshot_id} to recover"),
                rolled_back: false,
                snapshot_id: Some(snapshot_id),
            },
        };
    }

    // 7. Post-commit cache invalidation (advisory), then the health gate.
    let _ = crate::snapshot::mark_instance_mutated(&instance_dir);
    let _ = crate::health::invalidate_health_cache(&instance_dir);
    let _ = std::fs::remove_dir_all(&staging_dir);

    let health = if skip_health_scan {
        None
    } else {
        let registry_db = ctx
            .paths
            .registry_db()
            .exists()
            .then(|| ctx.paths.registry_db());
        Some(crate::health::cached_health(
            &instance_dir,
            &future_manifest,
            registry_db.as_deref(),
            None,
        ))
    };
    if let Some(report) = &health {
        if !report.blockers.is_empty() {
            // A committed migration that health-checks red is exactly the
            // crash-loop case: undo it completely, snapshot restore + DB
            // revert, rather than hand the user a broken instance.
            let restore = crate::snapshot::restore_snapshot(&instance_dir, &snapshot_id);
            if let Err(error) = restore {
                return MigrationOutcome::Failed {
                    phase: "health".into(),
                    error: format!(
                        "Post-migration health found blockers ({}) but automatic restore failed ({error}); restore snapshot {snapshot_id} to recover",
                        report.blockers.iter().map(|b| b.message.clone()).collect::<Vec<_>>().join("; ")
                    ),
                    rolled_back: false,
                    snapshot_id: Some(snapshot_id),
                };
            }
            let revert = commit_migration_db(
                ctx,
                &instance_id,
                &plan.loader,
                &plan.target_version,
                &new_loader_version(plan),
                &plan.source_version,
                &plan.source_loader_version,
            );
            let _ = crate::snapshot::mark_instance_mutated(&instance_dir);
            let _ = crate::health::invalidate_health_cache(&instance_dir);
            return match revert {
                Ok(1) => MigrationOutcome::RolledBack {
                    phase: "health".into(),
                    error: format!(
                        "Post-migration health found blockers; the instance was fully restored to {}: {}",
                        plan.source_version,
                        report.blockers.iter().map(|b| b.message.clone()).collect::<Vec<_>>().join("; ")
                    ),
                    snapshot_id: Some(snapshot_id),
                    health_report: Some(report.clone()),
                },
                other => MigrationOutcome::Failed {
                    phase: "health-db-revert".into(),
                    error: format!(
                        "Instance files were restored but the database row could not be reverted ({other:?}); repair the instance tuple before launching"
                    ),
                    rolled_back: true,
                    snapshot_id: Some(snapshot_id),
                },
            };
        }
    }

    MigrationOutcome::Migrated {
        instance_id,
        from_version: plan.source_version.clone(),
        to_version: plan.target_version.clone(),
        loader_version: plan.target_loader_version.clone(),
        replaced: plan
            .swaps
            .iter()
            .map(|swap| swap.old_filename.clone())
            .collect(),
        snapshot_id,
        warnings: plan.warnings.clone(),
        health,
    }
}

/// Plan + execute in one call — the entry point adapters reach for.
pub async fn migrate_instance(
    ctx: &Ctx,
    instance_id: &str,
    target_version: &str,
    services: &MigrationServices<'_>,
    accept_blockers: bool,
) -> MigrationOutcome {
    let plan = match plan_migration(
        ctx,
        instance_id,
        target_version,
        services.checker,
        services.successors,
    )
    .await
    {
        Ok(plan) => plan,
        Err(rejection) => {
            return MigrationOutcome::Blocked {
                reasons: rejection.reasons,
                report: rejection.report,
            }
        }
    };
    execute_migration(
        ctx,
        &plan,
        services.provisioner,
        services.fetcher,
        services.skip_health_scan,
        accept_blockers,
    )
    .await
}

// ---------------------------------------------------------------------------
// Ctx-owned facade for the desktop/CLI adapters
// ---------------------------------------------------------------------------

/// Convenience wrapper wiring the live seams. The pure entry points above
/// remain available for hosts that want their own provisioner/fetcher.
#[derive(Clone)]
pub struct VersionMigrationService {
    ctx: Ctx,
    successors: std::sync::Arc<dyn SuccessorLookup>,
}

impl VersionMigrationService {
    pub fn new(ctx: Ctx) -> Self {
        Self {
            ctx,
            successors: std::sync::Arc::new(crate::migration_report::NoopSuccessorLookup),
        }
    }

    pub fn with_successor_lookup(mut self, lookup: std::sync::Arc<dyn SuccessorLookup>) -> Self {
        self.successors = lookup;
        self
    }

    pub async fn plan(
        &self,
        instance_id: &str,
        target_version: &str,
    ) -> Result<MigrationPlan, MigrationRejection> {
        let checker = crate::migration_report::LiveModrinthChecker::new(self.ctx.clone());
        plan_migration(
            &self.ctx,
            instance_id,
            target_version,
            &checker,
            self.successors.as_ref(),
        )
        .await
    }

    /// `accept_blockers` is the user's answer to the plan's `blockers` list:
    /// proceed and leave those entries at their current version, or stop.
    pub async fn migrate(
        &self,
        instance_id: &str,
        target_version: &str,
        accept_blockers: bool,
    ) -> MigrationOutcome {
        let checker = crate::migration_report::LiveModrinthChecker::new(self.ctx.clone());
        let provisioner =
            LiveLoaderProvisioner::new(crate::loader_service::LoaderService::new(self.ctx.clone()));
        let services = MigrationServices {
            checker: &checker,
            successors: self.successors.as_ref(),
            provisioner: &provisioner,
            fetcher: &HttpArtifactFetcher,
            skip_health_scan: false,
        };
        migrate_instance(
            &self.ctx,
            instance_id,
            target_version,
            &services,
            accept_blockers,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn reject(code: &str, message: impl Into<String>, filename: Option<String>) -> MigrationRejection {
    MigrationRejection {
        reasons: vec![RejectionReason {
            code: code.into(),
            message: message.into(),
            filename,
        }],
        report: None,
    }
}

fn sanitize_instance_id(instance_id: &str) -> LauncherResult<String> {
    let sanitized = crate::paths::sanitize_id(instance_id);
    crate::app_paths::validate_path_component(&sanitized)?;
    Ok(sanitized)
}

/// Manifest + DB row, with the same tuple-consistency rule the loader service
/// enforces: a mismatched DB/manifest pair must be repaired before we move it.
fn load_instance(
    ctx: &Ctx,
    instance_id: &str,
) -> LauncherResult<(InstanceManifest, crate::models::InstanceRow)> {
    let manifest_path = ctx.paths.instance_manifest(instance_id)?;
    let manifest = crate::helpers::read_manifest(&manifest_path)?;
    let conn = crate::db::local_state_connection(&ctx.paths.local_state_db())
        .map_err(|_| LauncherError::LocalStateFailed)?;
    let row = crate::db::get_instance(&conn, instance_id)
        .map_err(|_| LauncherError::LocalStateFailed)?
        .ok_or_else(|| LauncherError::Generic {
            code: "ERR_INSTANCE_NOT_FOUND".into(),
            message: format!("Instance '{instance_id}' not found"),
        })?;
    if row.loader != manifest.loader
        || row.minecraft_version != manifest.minecraft_version
        || row.loader_version != manifest.loader_version
    {
        return Err(LauncherError::Generic {
            code: "ERR_LOADER_TUPLE_MISMATCH".into(),
            message: format!(
                "Instance '{instance_id}' has a mismatched loader tuple between the database and its manifest; repair the instance before migrating."
            ),
        });
    }
    Ok((manifest, row))
}

fn is_vanilla(loader: &str) -> bool {
    matches!(loader.trim().to_ascii_lowercase().as_str(), "" | "vanilla")
}

/// Whether the plan still describes the instance on disk.
fn plan_is_stale(plan: &MigrationPlan, manifest: &InstanceManifest) -> Option<String> {
    if manifest.minecraft_version != plan.source_version
        || manifest.loader != plan.loader
        || manifest.loader_version != plan.source_loader_version
    {
        return Some(
            "The instance's version/loader tuple changed after planning. Re-plan the migration."
                .into(),
        );
    }
    let current: BTreeSet<(String, String)> = manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
        .map(|entry| {
            (
                normalized_content_type(&entry.content_type).to_string(),
                entry.filename.clone(),
            )
        })
        .collect();
    let planned: BTreeSet<(String, String)> = plan
        .swaps
        .iter()
        .map(|swap| (swap.content_type.clone(), swap.old_filename.clone()))
        .collect();
    if current != planned {
        return Some(
            "The instance's installed content changed after planning. Re-plan the migration."
                .into(),
        );
    }
    None
}

fn new_loader_version(plan: &MigrationPlan) -> String {
    plan.target_loader_version
        .clone()
        .unwrap_or_else(|| plan.source_loader_version.clone())
}

fn staging_artifact_name(swap: &PlannedSwap) -> String {
    // Prefix with the identity so same-named artifacts from different buckets
    // cannot collide inside the flat staging/artifacts directory.
    format!(
        "{}--{}--{}",
        normalized_content_type(&swap.content_type),
        swap.old_filename,
        base_filename(&swap.new_filename)
    )
}

fn base_filename(name: &str) -> &str {
    name.strip_suffix(".disabled").unwrap_or(name)
}

fn effective_installed_filename(entry: &InstalledMod) -> String {
    if entry.enabled || entry.filename.ends_with(".disabled") {
        entry.filename.clone()
    } else {
        format!("{}.disabled", entry.filename)
    }
}

fn installed_identity(item: &InstalledMod) -> Option<String> {
    item.registry_id
        .clone()
        .or_else(|| item.modrinth_id.clone())
        .or_else(|| item.mod_jar_id.clone())
}

/// Content-type normalization and bucket accessors. These intentionally
/// mirror the private helpers in `install_pipeline`; keeping the migration
/// out of the pipeline's internals is the whole point of this module, and a
/// drift here would show up immediately in the swap tests.
fn normalized_content_type(content_type: &str) -> &str {
    match content_type {
        "resourcepack" | "resourcepacks" => "resourcepack",
        "shader" | "shaderpack" | "shaderpacks" => "shader",
        "datapack" | "datapacks" => "datapack",
        "world" | "worlds" => "world",
        _ => "mod",
    }
}

fn content_subdir(content_type: &str) -> &'static str {
    match normalized_content_type(content_type) {
        "resourcepack" => "resourcepacks",
        "shader" => "shaderpacks",
        "datapack" => "datapacks",
        "world" => "saves",
        _ => "mods",
    }
}

fn content_entries_mut<'a>(
    manifest: &'a mut InstanceManifest,
    content_type: &str,
) -> &'a mut Vec<InstalledMod> {
    match normalized_content_type(content_type) {
        "resourcepack" => &mut manifest.resourcepacks,
        "shader" => &mut manifest.shaders,
        "datapack" => &mut manifest.datapacks,
        "world" => &mut manifest.worlds,
        _ => &mut manifest.mods,
    }
}

fn validate_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename.contains('/')
        || filename.contains('\\')
        || Path::new(filename)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(filename)
    {
        Err(format!("artifact filename {filename:?} is unsafe"))
    } else {
        Ok(())
    }
}

fn has_verifiable_hash(target: &TargetBuildInfo) -> bool {
    let sha512_ok = target
        .sha512
        .as_deref()
        .is_some_and(|hash| hash.trim().len() == 128);
    let sha1_ok = target
        .sha1
        .as_deref()
        .is_some_and(|hash| hash.trim().len() == 40);
    sha512_ok || sha1_ok
}

fn verify_artifact_bytes(bytes: &[u8], target: &TargetBuildInfo) -> Result<(), String> {
    if let Some(size) = target.size {
        if size != 0 && bytes.len() as u64 != size {
            return Err(format!(
                "size mismatch: expected {size}, received {}",
                bytes.len()
            ));
        }
    }
    let mut verified = false;
    if let Some(expected) = target.sha512.as_deref().map(str::trim) {
        if !expected.is_empty() {
            use sha2::Digest;
            let mut hasher = sha2::Sha512::new();
            hasher.update(bytes);
            let actual = format!("{:x}", hasher.finalize());
            if !actual.eq_ignore_ascii_case(expected) {
                return Err("SHA-512 mismatch".into());
            }
            verified = true;
        }
    }
    if let Some(expected) = target.sha1.as_deref().map(str::trim) {
        if !expected.is_empty() {
            use sha1::Digest;
            let mut hasher = sha1::Sha1::new();
            hasher.update(bytes);
            let actual = format!("{:x}", hasher.finalize());
            if !actual.eq_ignore_ascii_case(expected) {
                return Err("SHA-1 mismatch".into());
            }
            verified = true;
        }
    }
    if verified {
        Ok(())
    } else {
        Err("no published hash was available to verify against".into())
    }
}

fn write_staged(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let partial = path.with_extension("part");
    let mut file = std::fs::File::create(&partial)
        .map_err(|e| format!("failed to create staged {}: {e}", path.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("failed to write staged {}: {e}", path.display()))?;
    file.sync_all()
        .map_err(|e| format!("failed to sync staged {}: {e}", path.display()))?;
    std::fs::rename(&partial, path)
        .map_err(|e| format!("failed to finalize staged {}: {e}", path.display()))
}

/// Build the complete future manifest: every swap applied *and* the version
/// fields moved, so the single manifest rename commits the whole migration.
/// Returns the manifest plus the exact serialized bytes that will be committed.
fn build_future_manifest(
    plan: &MigrationPlan,
    current: &InstanceManifest,
    artifacts_dir: &Path,
) -> Result<(InstanceManifest, Vec<u8>), String> {
    check_failpoint("prepare-manifest")?;
    let mut future = current.clone();
    for swap in &plan.swaps {
        let staged = artifacts_dir.join(staging_artifact_name(swap));
        let contents = std::fs::read(&staged)
            .map_err(|e| format!("failed to read staged {}: {e}", swap.new_filename))?;
        verify_artifact_bytes(&contents, &swap.target)
            .map_err(|e| format!("staged {} failed re-verification: {e}", swap.new_filename))?;
        let old = content_entries_mut(&mut future, &swap.content_type)
            .iter()
            .find(|entry| entry.filename == swap.old_filename)
            .cloned()
            .ok_or_else(|| format!("{} vanished during planning", swap.old_filename))?;
        let jar = if normalized_content_type(&swap.content_type) == "mod" {
            crate::jar_metadata::parse_jar_metadata_for_loader(&staged, &future.loader)
        } else {
            crate::dependency_ops::JarDeps::default()
        };
        let entry_filename = base_filename(&swap.new_filename).to_string();
        let installed = InstalledMod {
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: old.installed_as_dependency,
            filename: entry_filename.clone(),
            registry_id: old.registry_id.clone(),
            modrinth_id: old.modrinth_id.clone(),
            // Provenance note: the replacement bytes come from the same
            // upstream that classified this entry as ready, so the recorded
            // source family is preserved rather than rewritten.
            source: old.source.clone(),
            source_url: Some(swap.target.download_url.clone()),
            version: Some(swap.target.version_number.clone()),
            sha256: crate::download::sha256_hex(&contents),
            installed_at: chrono::Utc::now().to_rfc3339(),
            java_packages: jar.java_packages,
            mod_jar_id: jar.mod_jar_id.or_else(|| installed_identity(&old)),
            depends_on: jar.depends_on,
            optional_deps: jar.optional_deps,
            incompatible_deps: jar.incompatible_deps,
            provided_mod_ids: jar
                .provided_mods
                .iter()
                .map(|provided| provided.mod_id.clone())
                .collect(),
            enabled: old.enabled,
            content_type: normalized_content_type(&swap.content_type).to_string(),
        };
        // Drop every stale record of this identity across all buckets (the
        // pipeline applies the same rule at `prepare_manifest`).
        let identity = installed_identity(&installed);
        for entries in [
            &mut future.mods,
            &mut future.resourcepacks,
            &mut future.shaders,
            &mut future.datapacks,
            &mut future.worlds,
        ] {
            entries.retain(|entry| {
                entry.filename != installed.filename
                    && identity
                        .as_ref()
                        .zip(installed_identity(entry).as_ref())
                        .map(|(a, b)| !a.eq_ignore_ascii_case(b))
                        .unwrap_or(true)
            });
        }
        content_entries_mut(&mut future, &installed.content_type).push(installed);
    }
    future.minecraft_version = plan.target_version.clone();
    if let Some(loader_version) = &plan.target_loader_version {
        future.loader_version = loader_version.clone();
    }
    let bytes = serde_json::to_vec_pretty(&future)
        .map_err(|e| format!("failed to serialize future manifest: {e}"))?;
    Ok((future, bytes))
}

#[derive(Default)]
struct ApplyJournal {
    /// (trash backup path, live original path)
    removed: Vec<(PathBuf, PathBuf)>,
    /// (live destination, staged source)
    added: Vec<(PathBuf, PathBuf)>,
}

fn apply_migration_moves(
    plan: &MigrationPlan,
    instance_dir: &Path,
    staging_dir: &Path,
    commit_temp_path: &Path,
    journal: &mut ApplyJournal,
) -> Result<(), String> {
    let manifest_path = instance_dir.join("instance_manifest.json");
    let trash_dir = staging_dir.join("trash");
    let artifacts_dir = staging_dir.join("artifacts");

    // Read the live manifest to locate each old entry's real file (disabled
    // entries may live under a `.disabled` name).
    let current = crate::helpers::read_manifest(&manifest_path)
        .map_err(|e| format!("failed to read manifest during apply: {e}"))?;

    for swap in &plan.swaps {
        let live = current
            .mods
            .iter()
            .chain(current.resourcepacks.iter())
            .chain(current.shaders.iter())
            .chain(current.datapacks.iter())
            .find(|entry| {
                entry.filename == swap.old_filename
                    && normalized_content_type(&entry.content_type)
                        == normalized_content_type(&swap.content_type)
            })
            .map(|entry| {
                instance_dir
                    .join(content_subdir(&swap.content_type))
                    .join(effective_installed_filename(entry))
            })
            .unwrap_or_else(|| {
                instance_dir
                    .join(content_subdir(&swap.content_type))
                    .join(&swap.old_filename)
            });
        if !live.exists() {
            continue;
        }
        let relative = live
            .strip_prefix(instance_dir)
            .map_err(|_| format!("removal path escaped instance: {}", live.display()))?;
        let backup = trash_dir.join(relative);
        if let Some(parent) = backup.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create transaction trash: {e}"))?;
        }
        std::fs::rename(&live, &backup)
            .map_err(|e| format!("failed to stage removal of {}: {e}", swap.old_filename))?;
        journal.removed.push((backup, live));
    }

    for swap in &plan.swaps {
        let staged = artifacts_dir.join(staging_artifact_name(swap));
        if !staged.is_file() {
            return Err(format!(
                "required staged artifact vanished: {}",
                swap.new_filename
            ));
        }
        let live = instance_dir
            .join(content_subdir(&swap.content_type))
            .join(&swap.new_filename);
        if live.exists() {
            return Err(format!("target file already exists: {}", live.display()));
        }
        if let Some(parent) = live.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create content directory: {e}"))?;
        }
        check_failpoint("artifact-move")?;
        std::fs::rename(&staged, &live)
            .map_err(|e| format!("failed to promote {}: {e}", swap.new_filename))?;
        journal.added.push((live, staged));
    }

    // Commit: two renames so the visible swap is the atomic manifest move.
    let prepared = staging_dir.join("instance_manifest.next.json");
    if !prepared.is_file() {
        return Err("prepared manifest vanished before commit".into());
    }
    check_failpoint("manifest-commit")?;
    std::fs::rename(prepared, commit_temp_path)
        .map_err(|e| format!("failed to move prepared manifest to commit location: {e}"))?;
    std::fs::rename(commit_temp_path, &manifest_path)
        .map_err(|e| format!("failed to commit instance manifest: {e}"))
}

/// Reverse every journal entry and restore the pre-commit manifest bytes.
/// Only valid while the DB row still describes the original state.
fn undo_migration(
    journal: &ApplyJournal,
    manifest_path: &Path,
    original_manifest: &[u8],
    commit_temp_path: &Path,
) -> Result<(), String> {
    let mut errors = Vec::new();
    for (live, staged) in journal.added.iter().rev() {
        if live.exists() {
            if let Some(parent) = staged.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(error) = std::fs::rename(live, staged) {
                errors.push(format!("added {}: {error}", live.display()));
            }
        }
    }
    for (backup, original) in journal.removed.iter().rev() {
        if backup.exists() && !original.exists() {
            if let Some(parent) = original.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(error) = std::fs::rename(backup, original) {
                errors.push(format!("removed {}: {error}", original.display()));
            }
        }
    }
    // If the commit rename pair was interrupted, a tmp manifest may be parked
    // beside the live one; discard it before restoring the original bytes.
    if commit_temp_path.exists() {
        let _ = std::fs::remove_file(commit_temp_path);
    }
    // Restore the original bytes atomically (write-then-rename), whichever
    // point of the commit sequence failed.
    if let Err(error) = atomic_write_bytes(manifest_path, original_manifest) {
        errors.push(format!("manifest restore: {error}"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let temp = path.with_extension(format!("restoring-{}", std::process::id()));
    let mut file = std::fs::File::create(&temp)
        .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    file.sync_all()
        .map_err(|e| format!("failed to sync {}: {e}", path.display()))?;
    drop(file);
    std::fs::rename(&temp, path).map_err(|e| format!("failed to restore {}: {e}", path.display()))
}

/// Guarded compare-and-set on the instance row, mirroring
/// `db::update_instance_loader_version`: the `WHERE` clause pins the expected
/// tuple so a concurrent change matches zero rows instead of being clobbered.
fn commit_migration_db(
    ctx: &Ctx,
    instance_id: &str,
    expected_loader: &str,
    expected_mc: &str,
    expected_loader_version: &str,
    new_mc: &str,
    new_loader_version: &str,
) -> Result<u64, String> {
    check_failpoint("db-commit")
        .map_err(|error| format!("failed to persist the migration instance row: {error}"))?;
    let conn = crate::db::local_state_connection(&ctx.paths.local_state_db())
        .map_err(|e| format!("local state failed: {e}"))?;
    let affected = conn
        .execute(
            "UPDATE user_instances
             SET minecraft_version = ?1,
                 loader_version = ?2
             WHERE instance_id = ?3
               AND loader = ?4
               AND minecraft_version = ?5
               AND loader_version = ?6",
            rusqlite::params![
                new_mc,
                new_loader_version,
                instance_id,
                expected_loader,
                expected_mc,
                expected_loader_version
            ],
        )
        .map_err(|e| format!("failed to persist the migration instance row: {e}"))?;
    if affected != 1 {
        return Err(format!(
            "the instance row changed concurrently (matched {affected} rows)"
        ));
    }
    Ok(affected as u64)
}

fn hash_serializable<T: Serialize>(value: &T) -> String {
    use sha2::Digest;
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn plan_fingerprint(plan: &MigrationPlan) -> String {
    use sha2::Digest;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Fingerprint<'a> {
        schema_version: u32,
        instance_id: &'a str,
        source_version: &'a str,
        target_version: &'a str,
        loader: &'a str,
        source_loader_version: &'a str,
        target_loader_version: &'a Option<String>,
        swaps: &'a [PlannedSwap],
        instance_state_hash: &'a str,
    }
    let material = Fingerprint {
        schema_version: 1,
        instance_id: &plan.instance_id,
        source_version: &plan.source_version,
        target_version: &plan.target_version,
        loader: &plan.loader,
        source_loader_version: &plan.source_loader_version,
        target_loader_version: &plan.target_loader_version,
        swaps: &plan.swaps,
        instance_state_hash: &plan.instance_state_hash,
    };
    let bytes = serde_json::to_vec(&material).unwrap_or_default();
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

// Test-only failpoints, mirroring the install pipeline's mechanism so
// mid-transaction failures (commit rename, DB write) are reproducible.
#[cfg(test)]
thread_local! {
    static MIGRATION_TEST_FAILPOINT: std::cell::RefCell<Option<&'static str>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_migration_failpoint(name: Option<&'static str>) {
    MIGRATION_TEST_FAILPOINT.with(|slot| *slot.borrow_mut() = name);
}

fn check_failpoint(name: &'static str) -> Result<(), String> {
    #[cfg(test)]
    {
        let should_fail = MIGRATION_TEST_FAILPOINT.with(|slot| *slot.borrow() == Some(name));
        if should_fail {
            return Err(format!("injected migration failpoint: {name}"));
        }
    }
    #[cfg(not(test))]
    let _ = name;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests (offline — checker, provisioner, and fetcher are all injected, the
// same way migration_report injects ModrinthChecker)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration_report::{NoopSuccessorLookup, SupportInfo};
    use crate::models::InstanceRow;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::sync::Mutex;

    const SOURCE_MC: &str = "1.21.1";
    const TARGET_MC: &str = "1.21.4";
    const SOURCE_LOADER_VERSION: &str = "0.18.6";

    fn context() -> (Ctx, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "agora-version-migration-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let ctx = Ctx::for_testing(root.clone());
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        (ctx, root)
    }

    /// A minimal but *parseable* Fabric mod jar so health scans and JAR
    /// metadata extraction behave like they would on real artifacts.
    fn fabric_jar(id: &str, version: &str, depends_json: Option<&str>) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::FileOptions::default();
            zip.start_file("fabric.mod.json", options).unwrap();
            let depends = depends_json.unwrap_or("{}");
            write!(
                zip,
                "{{\"schemaVersion\":1,\"id\":\"{id}\",\"version\":\"{version}\",\"depends\":{depends}}}"
            )
            .unwrap();
            zip.finish().unwrap();
        }
        buffer.into_inner()
    }

    fn sha512_hex(bytes: &[u8]) -> String {
        use sha2::Digest;
        let mut hasher = sha2::Sha512::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    fn target_build(filename: &str, version_number: &str, bytes: &[u8]) -> TargetBuildInfo {
        TargetBuildInfo {
            version_id: format!("vid-{filename}"),
            version_number: version_number.to_string(),
            filename: filename.to_string(),
            download_url: format!("https://cdn.modrinth.test/data/{filename}"),
            sha1: None,
            sha512: Some(sha512_hex(bytes)),
            size: Some(bytes.len() as u64),
            is_prerelease: false,
        }
    }

    /// project id -> checker answer.
    struct MockChecker {
        answers: BTreeMap<String, Result<SupportInfo, String>>,
        calls: Mutex<Vec<String>>,
    }

    impl MockChecker {
        fn new() -> Self {
            Self {
                answers: BTreeMap::new(),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn ready_with_build(mut self, project: &str, build: TargetBuildInfo) -> Self {
            self.answers.insert(
                project.to_string(),
                Ok(SupportInfo {
                    has_target_build: true,
                    last_updated: None,
                    target_build: Some(build),
                }),
            );
            self
        }
        fn ready_without_build(mut self, project: &str) -> Self {
            self.answers.insert(
                project.to_string(),
                Ok(SupportInfo {
                    has_target_build: true,
                    last_updated: None,
                    target_build: None,
                }),
            );
            self
        }
        fn not_yet(mut self, project: &str) -> Self {
            self.answers.insert(
                project.to_string(),
                Ok(SupportInfo {
                    has_target_build: false,
                    last_updated: None,
                    target_build: None,
                }),
            );
            self
        }
        fn failure(mut self, project: &str) -> Self {
            self.answers
                .insert(project.to_string(), Err("ERR_NETWORK".to_string()));
            self
        }
    }

    #[async_trait]
    impl ModrinthChecker for MockChecker {
        async fn check_target_support(
            &self,
            project_id: &str,
            _target_version: &str,
            _loader: &str,
            _content_type: &str,
        ) -> Result<SupportInfo, LauncherError> {
            self.calls.lock().unwrap().push(project_id.to_string());
            match self.answers.get(project_id) {
                Some(Ok(info)) => Ok(info.clone()),
                Some(Err(code)) => Err(LauncherError::Generic {
                    code: code.clone(),
                    message: "injected checker failure".into(),
                }),
                None => Err(LauncherError::Generic {
                    code: "ERR_NOT_MOCKED".into(),
                    message: format!("no mock for {project_id}"),
                }),
            }
        }
    }

    struct MockProvisioner {
        fail: bool,
        calls: Mutex<Vec<(String, String, String)>>,
    }

    impl MockProvisioner {
        fn ok() -> Self {
            Self {
                fail: false,
                calls: Mutex::new(Vec::new()),
            }
        }
        fn failing() -> Self {
            Self {
                fail: true,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl LoaderProvisioner for MockProvisioner {
        async fn ensure_loader(
            &self,
            loader: &str,
            minecraft_version: &str,
            loader_version: &str,
        ) -> Result<(), LauncherError> {
            self.calls.lock().unwrap().push((
                loader.to_string(),
                minecraft_version.to_string(),
                loader_version.to_string(),
            ));
            if self.fail {
                return Err(LauncherError::Generic {
                    code: "ERR_LOADER_PROVISION".into(),
                    message: "injected loader provision failure".into(),
                });
            }
            Ok(())
        }
    }

    struct MockFetcher {
        responses: BTreeMap<String, Result<Vec<u8>, String>>,
        calls: Mutex<Vec<String>>,
    }

    impl MockFetcher {
        fn new() -> Self {
            Self {
                responses: BTreeMap::new(),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn with(mut self, build: &TargetBuildInfo, bytes: Vec<u8>) -> Self {
            self.responses.insert(build.download_url.clone(), Ok(bytes));
            self
        }
        fn failing(mut self, url: &str) -> Self {
            self.responses
                .insert(url.to_string(), Err("injected fetch failure".to_string()));
            self
        }
    }

    #[async_trait]
    impl ArtifactFetcher for MockFetcher {
        async fn fetch(&self, url: &str) -> Result<Vec<u8>, LauncherError> {
            self.calls.lock().unwrap().push(url.to_string());
            match self.responses.get(url) {
                Some(Ok(bytes)) => Ok(bytes.clone()),
                Some(Err(message)) => Err(LauncherError::Generic {
                    code: "ERR_FETCH".into(),
                    message: message.clone(),
                }),
                None => Err(LauncherError::Generic {
                    code: "ERR_FETCH_UNMOCKED".into(),
                    message: format!("no fetch mock for {url}"),
                }),
            }
        }
    }

    struct Seed {
        filename: &'static str,
        modrinth_id: Option<&'static str>,
        content_type: &'static str,
        bytes: Vec<u8>,
    }

    fn seed_instance(
        ctx: &Ctx,
        instance_id: &str,
        seeds: &[Seed],
        row_overrides: impl FnOnce(&mut InstanceRow, &mut InstanceManifest),
    ) {
        let dir = ctx.paths.instance_dir(instance_id).unwrap();
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        let mut manifest = InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: instance_id.into(),
            name: "Mig".into(),
            created_from_pack: None,
            minecraft_version: SOURCE_MC.into(),
            loader: "fabric".into(),
            loader_version: SOURCE_LOADER_VERSION.into(),
            is_locked: false,
            mods: Vec::new(),
            resourcepacks: Vec::new(),
            shaders: Vec::new(),
            datapacks: Vec::new(),
            worlds: Vec::new(),
            user_preferences: serde_json::json!({}),
        };
        for seed in seeds {
            let subdir = content_subdir(seed.content_type);
            std::fs::create_dir_all(dir.join(subdir)).unwrap();
            if !seed.bytes.is_empty() {
                std::fs::write(dir.join(subdir).join(seed.filename), &seed.bytes).unwrap();
            }
            let entry = InstalledMod {
                update_pinned: false,
                pack_managed: false,
                installed_as_dependency: false,
                filename: seed.filename.to_string(),
                registry_id: None,
                modrinth_id: seed.modrinth_id.map(str::to_string),
                source: "modrinth_raw".into(),
                source_url: None,
                version: Some("1.0".into()),
                sha256: crate::download::sha256_hex(&seed.bytes),
                installed_at: chrono::Utc::now().to_rfc3339(),
                java_packages: Vec::new(),
                mod_jar_id: None,
                depends_on: Vec::new(),
                optional_deps: Vec::new(),
                incompatible_deps: Vec::new(),
                provided_mod_ids: Vec::new(),
                enabled: true,
                content_type: seed.content_type.into(),
            };
            match normalized_content_type(seed.content_type) {
                "resourcepack" => manifest.resourcepacks.push(entry),
                "shader" => manifest.shaders.push(entry),
                "datapack" => manifest.datapacks.push(entry),
                "world" => manifest.worlds.push(entry),
                _ => manifest.mods.push(entry),
            }
        }
        let mut row = InstanceRow {
            instance_id: instance_id.into(),
            name: "Mig".into(),
            minecraft_version: SOURCE_MC.into(),
            loader: manifest.loader.clone(),
            loader_version: manifest.loader_version.clone(),
            is_modpack: false,
            is_locked: false,
            last_launched_at: None,
            jvm_memory_mb: 4096,
            jvm_memory_mode: "auto".into(),
            jvm_gc: "auto".into(),
            jvm_custom_args: String::new(),
            jvm_always_pre_touch: false,
            created_at: chrono::Utc::now().to_rfc3339(),
            java_path: None,
            java_incompatible_override: false,
            icon_path: None,
            launch_mode_override: "auto".into(),
            import_source: None,
        };
        row_overrides(&mut row, &mut manifest);
        std::fs::write(
            ctx.paths.instance_manifest(instance_id).unwrap(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(&conn, &row).unwrap();
    }

    fn read_row(ctx: &Ctx, id: &str) -> InstanceRow {
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::get_instance(&conn, id).unwrap().unwrap()
    }

    fn read_manifest_of(ctx: &Ctx, id: &str) -> InstanceManifest {
        crate::helpers::read_manifest(&ctx.paths.instance_manifest(id).unwrap()).unwrap()
    }

    fn expected_latest_loader(loader: &str, mc: &str) -> String {
        crate::loader_manifests::list_versions(loader, mc)
            .into_iter()
            .map(|entry| entry.loader_version)
            .max_by(|a, b| crate::version_match::compare_versions(a, b))
            .expect("test catalog must carry fabric builds for the test target")
    }

    fn services<'a>(
        checker: &'a MockChecker,
        provisioner: &'a MockProvisioner,
        fetcher: &'a MockFetcher,
    ) -> MigrationServices<'a> {
        MigrationServices {
            checker,
            successors: &NoopSuccessorLookup,
            provisioner,
            fetcher,
            skip_health_scan: false,
        }
    }

    fn assert_untouched(ctx: &Ctx, id: &str) {
        let manifest = read_manifest_of(ctx, id);
        assert_eq!(manifest.minecraft_version, SOURCE_MC);
        assert_eq!(manifest.loader_version, SOURCE_LOADER_VERSION);
        let row = read_row(ctx, id);
        assert_eq!(row.minecraft_version, SOURCE_MC);
        assert_eq!(row.loader_version, SOURCE_LOADER_VERSION);
    }

    #[tokio::test]
    async fn an_instance_with_no_content_still_migrates() {
        // Degenerate but entirely legitimate: a fresh instance with no mods
        // yet. There is nothing to resolve, so the migration is purely the
        // version and loader change — it must not be mistaken for "nothing to
        // do" and silently no-op, nor blocked for lack of content.
        let (ctx, _root) = context();
        seed_instance(&ctx, "bare", &[], |_, _| {});

        let plan = plan_migration(
            &ctx,
            "bare",
            TARGET_MC,
            &MockChecker::new(),
            &NoopSuccessorLookup,
        )
        .await
        .expect("a contentless instance is migratable, not blocked");
        assert!(plan.swaps.is_empty());
        assert_eq!(plan.target_version, TARGET_MC);
    }

    #[tokio::test]
    async fn happy_path_changes_manifest_row_content_and_loader() {
        let (ctx, root) = context();
        let old_a = fabric_jar("moda", "1.0", None);
        let old_b = fabric_jar("modb", "1.0", None);
        let old_rp: Vec<u8> = b"old resource pack zip".to_vec();
        seed_instance(
            &ctx,
            "happy",
            &[
                Seed {
                    filename: "moda-1.0.jar",
                    modrinth_id: Some("proj-a"),
                    content_type: "mod",
                    bytes: old_a.clone(),
                },
                Seed {
                    filename: "modb-1.0.jar",
                    modrinth_id: Some("proj-b"),
                    content_type: "mod",
                    bytes: old_b.clone(),
                },
                Seed {
                    filename: "rp-1.0.zip",
                    modrinth_id: Some("proj-rp"),
                    content_type: "resourcepack",
                    bytes: old_rp.clone(),
                },
                Seed {
                    filename: "MyWorld",
                    modrinth_id: None,
                    content_type: "world",
                    bytes: Vec::new(),
                },
            ],
            |_, _| {},
        );
        let new_a = fabric_jar("moda", "2.0", None);
        let new_b = fabric_jar("modb", "2.0", None);
        let new_rp: Vec<u8> = b"new resource pack zip".to_vec();
        let build_a = target_build("moda-2.0.jar", "2.0", &new_a);
        let build_b = target_build("modb-2.0.jar", "2.0", &new_b);
        let build_rp = target_build("rp-2.0.zip", "2.0", &new_rp);
        let checker = MockChecker::new()
            .ready_with_build("proj-a", build_a.clone())
            .ready_with_build("proj-b", build_b.clone())
            .ready_with_build("proj-rp", build_rp.clone());
        let provisioner = MockProvisioner::ok();
        let fetcher = MockFetcher::new()
            .with(&build_a, new_a.clone())
            .with(&build_b, new_b.clone())
            .with(&build_rp, new_rp.clone());
        let outcome = migrate_instance(
            &ctx,
            "happy",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        let expected_loader = expected_latest_loader("fabric", TARGET_MC);
        let MigrationOutcome::Migrated {
            snapshot_id,
            warnings,
            health,
            loader_version,
            replaced,
            ..
        } = outcome
        else {
            panic!("expected Migrated, got {outcome:?}");
        };
        assert_eq!(loader_version.as_deref(), Some(expected_loader.as_str()));
        assert_eq!(replaced.len(), 3);
        assert!(health.as_ref().unwrap().blockers.is_empty());
        assert!(!snapshot_id.is_empty());
        assert!(warnings.iter().any(|w| w.contains("world")));

        let manifest = read_manifest_of(&ctx, "happy");
        assert_eq!(manifest.minecraft_version, TARGET_MC);
        assert_eq!(manifest.loader_version, expected_loader);
        let row = read_row(&ctx, "happy");
        assert_eq!(row.minecraft_version, TARGET_MC);
        assert_eq!(row.loader_version, expected_loader);
        assert_eq!(row.loader, "fabric");

        let dir = ctx.paths.instance_dir("happy").unwrap();
        assert_eq!(
            std::fs::read(dir.join("mods").join("moda-2.0.jar")).unwrap(),
            new_a
        );
        assert_eq!(
            std::fs::read(dir.join("mods").join("modb-2.0.jar")).unwrap(),
            new_b
        );
        assert_eq!(
            std::fs::read(dir.join("resourcepacks").join("rp-2.0.zip")).unwrap(),
            new_rp
        );
        assert!(!dir.join("mods").join("moda-1.0.jar").exists());
        assert!(!dir.join("mods").join("modb-1.0.jar").exists());
        assert_eq!(manifest.mods.len(), 2);
        for entry in &manifest.mods {
            assert_eq!(entry.version.as_deref(), Some("2.0"));
            assert!(entry.source_url.is_some());
        }
        // Worlds are left behind untouched.
        assert_eq!(manifest.worlds.len(), 1);
        // Provisioner was asked for the *target* tuple.
        assert_eq!(
            provisioner.calls.lock().unwrap().as_slice(),
            &[(
                "fabric".to_string(),
                TARGET_MC.to_string(),
                expected_loader.clone()
            )]
        );
        // Migration staging was cleaned up after success.
        let staging_root = dir.join(".agora").join("staging");
        if staging_root.exists() {
            assert_eq!(std::fs::read_dir(&staging_root).unwrap().count(), 0);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn ready_mod_without_named_build_stops_the_migration() {
        let (ctx, root) = context();
        seed_instance(
            &ctx,
            "unresolvable",
            &[
                Seed {
                    filename: "moda-1.0.jar",
                    modrinth_id: Some("proj-a"),
                    content_type: "mod",
                    bytes: fabric_jar("moda", "1.0", None),
                },
                Seed {
                    filename: "modb-1.0.jar",
                    modrinth_id: Some("proj-b"),
                    content_type: "mod",
                    bytes: fabric_jar("modb", "1.0", None),
                },
            ],
            |_, _| {},
        );
        let build_b = target_build("modb-2.0.jar", "2.0", &fabric_jar("modb", "2.0", None));
        let checker = MockChecker::new()
            .ready_without_build("proj-a")
            .ready_with_build("proj-b", build_b);
        let provisioner = MockProvisioner::ok();
        let fetcher = MockFetcher::new();
        let outcome = migrate_instance(
            &ctx,
            "unresolvable",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        let MigrationOutcome::Blocked { reasons, report } = outcome else {
            panic!("expected Blocked, got {outcome:?}");
        };
        assert!(reasons
            .iter()
            .any(|r| r.code == "ERR_MIGRATION_UNRESOLVABLE"
                && r.filename.as_deref() == Some("moda-1.0.jar")));
        assert!(report.is_some());
        assert!(provisioner.calls.lock().unwrap().is_empty());
        assert!(fetcher.calls.lock().unwrap().is_empty());
        assert_untouched(&ctx, "unresolvable");
        assert!(
            crate::snapshot::list_snapshots(&ctx.paths.instance_dir("unresolvable").unwrap())
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn not_ready_and_unknown_entries_stop_the_migration() {
        let (ctx, root) = context();
        seed_instance(
            &ctx,
            "notready",
            &[
                Seed {
                    filename: "moda-1.0.jar",
                    modrinth_id: Some("proj-a"),
                    content_type: "mod",
                    bytes: fabric_jar("moda", "1.0", None),
                },
                Seed {
                    filename: "modb-1.0.jar",
                    modrinth_id: Some("proj-b"),
                    content_type: "mod",
                    bytes: fabric_jar("modb", "1.0", None),
                },
            ],
            |_, _| {},
        );
        let build_a = target_build("moda-2.0.jar", "2.0", &fabric_jar("moda", "2.0", None));
        let provisioner = MockProvisioner::ok();
        let fetcher = MockFetcher::new();
        let checker = MockChecker::new()
            .ready_with_build("proj-a", build_a)
            .not_yet("proj-b");
        let outcome = migrate_instance(
            &ctx,
            "notready",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        let MigrationOutcome::Blocked { reasons, .. } = outcome else {
            panic!("expected Blocked, got {outcome:?}");
        };
        assert!(reasons.iter().any(|r| r.code == "ERR_MIGRATION_NOT_READY"
            && r.filename.as_deref() == Some("modb-1.0.jar")));
        assert_untouched(&ctx, "notready");

        // A lookup failure must block too — never treated as "nothing to do".
        let checker = MockChecker::new().failure("proj-a").failure("proj-b");
        let outcome = migrate_instance(
            &ctx,
            "notready",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        assert!(matches!(outcome, MigrationOutcome::Blocked { .. }));
        assert_untouched(&ctx, "notready");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn loader_provision_failure_leaves_the_instance_untouched() {
        let (ctx, root) = context();
        seed_instance(
            &ctx,
            "provfail",
            &[Seed {
                filename: "moda-1.0.jar",
                modrinth_id: Some("proj-a"),
                content_type: "mod",
                bytes: fabric_jar("moda", "1.0", None),
            }],
            |_, _| {},
        );
        let build_a = target_build("moda-2.0.jar", "2.0", &fabric_jar("moda", "2.0", None));
        let checker = MockChecker::new().ready_with_build("proj-a", build_a.clone());
        let provisioner = MockProvisioner::failing();
        let fetcher = MockFetcher::new().with(&build_a, fabric_jar("moda", "2.0", None));
        let outcome = migrate_instance(
            &ctx,
            "provfail",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        let MigrationOutcome::Failed {
            phase, rolled_back, ..
        } = outcome
        else {
            panic!("expected Failed, got {outcome:?}");
        };
        assert_eq!(phase, "loader-provision");
        assert!(!rolled_back);
        assert!(fetcher.calls.lock().unwrap().is_empty());
        assert_untouched(&ctx, "provfail");
        assert_eq!(
            read_manifest_of(&ctx, "provfail").mods[0].filename,
            "moda-1.0.jar"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn staging_failure_partway_changes_nothing() {
        let (ctx, root) = context();
        seed_instance(
            &ctx,
            "stagefail",
            &[
                Seed {
                    filename: "moda-1.0.jar",
                    modrinth_id: Some("proj-a"),
                    content_type: "mod",
                    bytes: fabric_jar("moda", "1.0", None),
                },
                Seed {
                    filename: "modb-1.0.jar",
                    modrinth_id: Some("proj-b"),
                    content_type: "mod",
                    bytes: fabric_jar("modb", "1.0", None),
                },
            ],
            |_, _| {},
        );
        let build_a = target_build("moda-2.0.jar", "2.0", &fabric_jar("moda", "2.0", None));
        let build_b = target_build("modb-2.0.jar", "2.0", &fabric_jar("modb", "2.0", None));
        let checker = MockChecker::new()
            .ready_with_build("proj-a", build_a.clone())
            .ready_with_build("proj-b", build_b.clone());
        let provisioner = MockProvisioner::ok();
        // The first artifact stages fine; the second cannot be fetched.
        let fetcher = MockFetcher::new()
            .with(&build_a, fabric_jar("moda", "2.0", None))
            .failing(&build_b.download_url);
        let outcome = migrate_instance(
            &ctx,
            "stagefail",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        let MigrationOutcome::Failed { phase, .. } = outcome else {
            panic!("expected Failed, got {outcome:?}");
        };
        assert_eq!(phase, "staging");
        assert_untouched(&ctx, "stagefail");
        let dir = ctx.paths.instance_dir("stagefail").unwrap();
        assert!(dir.join("mods").join("moda-1.0.jar").exists());
        assert!(dir.join("mods").join("modb-1.0.jar").exists());
        assert!(!dir.join("mods").join("moda-2.0.jar").exists());
        assert!(crate::snapshot::list_snapshots(&dir).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn manifest_commit_failure_rolls_back_every_file_move() {
        let (ctx, root) = context();
        let old_a = fabric_jar("moda", "1.0", None);
        seed_instance(
            &ctx,
            "commitfail",
            &[Seed {
                filename: "moda-1.0.jar",
                modrinth_id: Some("proj-a"),
                content_type: "mod",
                bytes: old_a.clone(),
            }],
            |_, _| {},
        );
        let new_a = fabric_jar("moda", "2.0", None);
        let build_a = target_build("moda-2.0.jar", "2.0", &new_a);
        let checker = MockChecker::new().ready_with_build("proj-a", build_a.clone());
        let provisioner = MockProvisioner::ok();
        let fetcher = MockFetcher::new().with(&build_a, new_a.clone());
        set_migration_failpoint(Some("manifest-commit"));
        let outcome = migrate_instance(
            &ctx,
            "commitfail",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        set_migration_failpoint(None);
        let MigrationOutcome::RolledBack { phase, .. } = outcome else {
            panic!("expected RolledBack, got {outcome:?}");
        };
        assert_eq!(phase, "apply");
        // The user's instance still launches: old files back, old tuple in
        // both manifest and DB row, no stray commit temporaries.
        assert_untouched(&ctx, "commitfail");
        let dir = ctx.paths.instance_dir("commitfail").unwrap();
        assert_eq!(
            std::fs::read(dir.join("mods").join("moda-1.0.jar")).unwrap(),
            old_a
        );
        assert!(!dir.join("mods").join("moda-2.0.jar").exists());
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("instance_manifest.json.tmp")
            })
            .collect();
        assert!(leftovers.is_empty(), "no tmp manifests: {leftovers:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn db_commit_failure_rolls_back_the_committed_transaction() {
        let (ctx, root) = context();
        let old_a = fabric_jar("moda", "1.0", None);
        seed_instance(
            &ctx,
            "dbfail",
            &[Seed {
                filename: "moda-1.0.jar",
                modrinth_id: Some("proj-a"),
                content_type: "mod",
                bytes: old_a.clone(),
            }],
            |_, _| {},
        );
        let build_a = target_build("moda-2.0.jar", "2.0", &fabric_jar("moda", "2.0", None));
        let checker = MockChecker::new().ready_with_build("proj-a", build_a.clone());
        let provisioner = MockProvisioner::ok();
        let fetcher = MockFetcher::new().with(&build_a, fabric_jar("moda", "2.0", None));
        // The manifest commit has *already happened* when the DB write fails —
        // this is the hardest rollback case: files AND manifest are moved.
        set_migration_failpoint(Some("db-commit"));
        let outcome = migrate_instance(
            &ctx,
            "dbfail",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        set_migration_failpoint(None);
        let MigrationOutcome::RolledBack { phase, .. } = outcome else {
            panic!("expected RolledBack, got {outcome:?}");
        };
        assert_eq!(phase, "db-commit");
        assert_untouched(&ctx, "dbfail");
        let dir = ctx.paths.instance_dir("dbfail").unwrap();
        assert_eq!(
            std::fs::read(dir.join("mods").join("moda-1.0.jar")).unwrap(),
            old_a
        );
        assert!(!dir.join("mods").join("moda-2.0.jar").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn post_commit_health_blockers_trigger_full_rollback() {
        let (ctx, root) = context();
        let old_a = fabric_jar("moda", "1.0", None);
        seed_instance(
            &ctx,
            "healthfail",
            &[Seed {
                filename: "moda-1.0.jar",
                modrinth_id: Some("proj-a"),
                content_type: "mod",
                bytes: old_a.clone(),
            }],
            |_, _| {},
        );
        // The replacement mod requires a library that nothing provides: health
        // must report a blocker and the migration must undo itself entirely.
        let new_a = fabric_jar("moda", "2.0", Some("{\"nowhere-lib\":\">=1.0.0\"}"));
        let build_a = target_build("moda-2.0.jar", "2.0", &new_a);
        let checker = MockChecker::new().ready_with_build("proj-a", build_a.clone());
        let provisioner = MockProvisioner::ok();
        let fetcher = MockFetcher::new().with(&build_a, new_a.clone());
        let outcome = migrate_instance(
            &ctx,
            "healthfail",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        let MigrationOutcome::RolledBack {
            phase,
            health_report,
            ..
        } = outcome
        else {
            panic!("expected RolledBack, got {outcome:?}");
        };
        assert_eq!(phase, "health");
        assert!(!health_report.unwrap().blockers.is_empty());
        // Fully restored: old files with old bytes, old tuple, and — crucially
        // — the DB row and manifest still *agree*, so the instance launches.
        let dir = ctx.paths.instance_dir("healthfail").unwrap();
        assert_eq!(
            std::fs::read(dir.join("mods").join("moda-1.0.jar")).unwrap(),
            old_a
        );
        assert_untouched(&ctx, "healthfail");
        assert!(
            !crate::snapshot::list_snapshots(&dir).unwrap().is_empty(),
            "the recovery snapshot survives the rollback"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn vanilla_instance_skips_the_loader_step() {
        let (ctx, root) = context();
        seed_instance(&ctx, "vanilla", &[], |row, manifest| {
            row.loader = "vanilla".into();
            row.loader_version = String::new();
            manifest.loader = "vanilla".into();
            manifest.loader_version = String::new();
        });
        let checker = MockChecker::new();
        let provisioner = MockProvisioner::ok();
        let fetcher = MockFetcher::new();
        let outcome = migrate_instance(
            &ctx,
            "vanilla",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        assert!(
            matches!(outcome, MigrationOutcome::Migrated { .. }),
            "expected Migrated, got {outcome:?}"
        );
        assert!(provisioner.calls.lock().unwrap().is_empty());
        let manifest = read_manifest_of(&ctx, "vanilla");
        assert_eq!(manifest.minecraft_version, TARGET_MC);
        let row = read_row(&ctx, "vanilla");
        assert_eq!(row.minecraft_version, TARGET_MC);
        assert_eq!(row.loader_version, manifest.loader_version);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn missing_signed_loader_target_blocks_before_any_mutation() {
        let (ctx, root) = context();
        seed_instance(
            &ctx,
            "noloader",
            &[Seed {
                filename: "moda-1.0.jar",
                modrinth_id: Some("proj-a"),
                content_type: "mod",
                bytes: fabric_jar("moda", "1.0", None),
            }],
            |_, _| {},
        );
        let build_a = target_build("moda-2.0.jar", "2.0", &fabric_jar("moda", "2.0", None));
        let checker = MockChecker::new().ready_with_build("proj-a", build_a);
        let provisioner = MockProvisioner::ok();
        let fetcher = MockFetcher::new();
        let outcome = migrate_instance(
            &ctx,
            "noloader",
            "9.9.99-phantom",
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        let MigrationOutcome::Blocked { reasons, .. } = outcome else {
            panic!("expected Blocked, got {outcome:?}");
        };
        assert!(reasons.iter().any(|r| r.code == "ERR_MIGRATION_NO_LOADER"));
        assert!(provisioner.calls.lock().unwrap().is_empty());
        assert_untouched(&ctx, "noloader");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn the_lock_is_the_only_hard_gate_and_the_rest_are_the_users_call() {
        // Policy, deliberately: unlocking is the single consent gate for every
        // kind of deviation, and everything else is reported so the user
        // decides. A modpack instance is locked on install, so unlocking one is
        // already "I accept this diverges from the pack" — refusing it twice
        // would block the case the feature exists for.
        let (ctx, root) = context();

        // A locked instance is refused outright.
        seed_instance(&ctx, "locked", &[], |row, manifest| {
            row.is_locked = true;
            manifest.is_locked = true;
        });
        let checker = MockChecker::new();
        let provisioner = MockProvisioner::ok();
        let fetcher = MockFetcher::new();
        let outcome = migrate_instance(
            &ctx,
            "locked",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        let MigrationOutcome::Blocked { reasons, .. } = outcome else {
            panic!("expected Blocked, got {outcome:?}");
        };
        assert!(reasons.iter().any(|r| r.code == "ERR_INSTANCE_LOCKED"));
        assert_untouched(&ctx, "locked");

        // An unlocked pack instance is allowed, and warned about.
        seed_instance(&ctx, "packed", &[], |_, manifest| {
            manifest.created_from_pack = Some("Some Pack".into());
        });
        let plan = plan_migration(&ctx, "packed", TARGET_MC, &checker, &NoopSuccessorLookup)
            .await
            .expect("an unlocked pack instance is migratable");
        assert!(
            plan.warnings.iter().any(|w| w.contains("modpack")),
            "the user has to be told it diverges: {:?}",
            plan.warnings
        );

        // A pinned mod is reported, not fatal — one pinned mod must not hold
        // the instance on an old Minecraft version forever.
        seed_instance(
            &ctx,
            "pinned",
            &[Seed {
                filename: "moda-1.0.jar",
                modrinth_id: Some("proj-a"),
                content_type: "mod",
                bytes: fabric_jar("moda", "1.0", None),
            }],
            |_, manifest| {
                manifest.mods[0].update_pinned = true;
            },
        );
        let plan = plan_migration(&ctx, "pinned", TARGET_MC, &checker, &NoopSuccessorLookup)
            .await
            .expect("a pinned entry is a blocker, not a planning refusal");
        assert!(plan
            .blockers
            .iter()
            .any(|r| r.code == "ERR_MIGRATION_PINNED"));
        assert!(plan.swaps.is_empty(), "a pinned entry is never swapped");

        // ...but proceeding past it still has to be an explicit decision.
        let outcome = execute_migration(&ctx, &plan, &provisioner, &fetcher, true, false).await;
        assert!(
            matches!(outcome, MigrationOutcome::Blocked { .. }),
            "unacknowledged blockers must stop execution: {outcome:?}"
        );
        assert_untouched(&ctx, "pinned");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn stale_plan_is_refused_at_execution_without_mutation() {
        let (ctx, root) = context();
        seed_instance(
            &ctx,
            "stale",
            &[Seed {
                filename: "moda-1.0.jar",
                modrinth_id: Some("proj-a"),
                content_type: "mod",
                bytes: fabric_jar("moda", "1.0", None),
            }],
            |_, _| {},
        );
        let build_a = target_build("moda-2.0.jar", "2.0", &fabric_jar("moda", "2.0", None));
        let checker = MockChecker::new().ready_with_build("proj-a", build_a);
        let plan = plan_migration(&ctx, "stale", TARGET_MC, &checker, &NoopSuccessorLookup)
            .await
            .unwrap();
        // The user installs a new mod between review and execution.
        let dir = ctx.paths.instance_dir("stale").unwrap();
        std::fs::write(dir.join("mods").join("surprise.jar"), b"surprise").unwrap();
        let provisioner = MockProvisioner::ok();
        let fetcher = MockFetcher::new();
        let outcome = execute_migration(&ctx, &plan, &provisioner, &fetcher, false, false).await;
        let MigrationOutcome::Failed { phase, error, .. } = outcome else {
            panic!("expected Failed, got {outcome:?}");
        };
        assert_eq!(phase, "precondition");
        assert!(error.contains("changed after planning"));
        assert!(provisioner.calls.lock().unwrap().is_empty());
        assert!(fetcher.calls.lock().unwrap().is_empty());
        assert_untouched(&ctx, "stale");
        assert!(dir.join("mods").join("surprise.jar").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn plan_is_deterministic_and_read_only() {
        let (ctx, root) = context();
        seed_instance(
            &ctx,
            "det",
            &[Seed {
                filename: "moda-1.0.jar",
                modrinth_id: Some("proj-a"),
                content_type: "mod",
                bytes: fabric_jar("moda", "1.0", None),
            }],
            |_, _| {},
        );
        let build_a = target_build("moda-2.0.jar", "2.0", &fabric_jar("moda", "2.0", None));
        let dir = ctx.paths.instance_dir("det").unwrap();
        let before = crate::snapshot::live_file_index(&dir).unwrap();
        let first = plan_migration(
            &ctx,
            "det",
            TARGET_MC,
            &MockChecker::new().ready_with_build("proj-a", build_a.clone()),
            &NoopSuccessorLookup,
        )
        .await
        .unwrap();
        let second = plan_migration(
            &ctx,
            "det",
            TARGET_MC,
            &MockChecker::new().ready_with_build("proj-a", build_a),
            &NoopSuccessorLookup,
        )
        .await
        .unwrap();
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(first.swaps.len(), 1);
        assert_eq!(first.swaps[0].new_filename, "moda-2.0.jar");
        assert_eq!(
            crate::snapshot::live_file_index(&dir).unwrap(),
            before,
            "planning must not touch the instance"
        );
        assert!(!dir.join(".agora").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn tampered_hash_fails_staging_before_mutation() {
        let (ctx, root) = context();
        seed_instance(
            &ctx,
            "tamper",
            &[Seed {
                filename: "moda-1.0.jar",
                modrinth_id: Some("proj-a"),
                content_type: "mod",
                bytes: fabric_jar("moda", "1.0", None),
            }],
            |_, _| {},
        );
        let good = fabric_jar("moda", "2.0", None);
        // Publish the hash but withhold the size so the *hash* check — not
        // the size shortcut — is what rejects the tampered bytes.
        let build_a = TargetBuildInfo {
            size: None,
            ..target_build("moda-2.0.jar", "2.0", &good)
        };
        let checker = MockChecker::new().ready_with_build("proj-a", build_a.clone());
        let provisioner = MockProvisioner::ok();
        // The fetcher returns bytes that do not match the published hash.
        let fetcher =
            MockFetcher::new().with(&build_a, fabric_jar("moda", "2.0-eviltampered", None));
        let outcome = migrate_instance(
            &ctx,
            "tamper",
            TARGET_MC,
            &services(&checker, &provisioner, &fetcher),
            false,
        )
        .await;
        let MigrationOutcome::Failed { phase, error, .. } = outcome else {
            panic!("expected Failed, got {outcome:?}");
        };
        assert_eq!(phase, "staging");
        assert!(error.contains("SHA-512 mismatch"));
        assert_untouched(&ctx, "tamper");
        let _ = std::fs::remove_dir_all(root);
    }
}
