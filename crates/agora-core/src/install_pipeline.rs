//! Canonical install-transaction pipeline.
//!
//! All install/update/remove entry points flow through `InstallPipeline`:
//!
//! 1. **Resolve** — pure data, no instance changes. Returns a `ResolvedInstallPlan`.
//! 2. **Stage** — download + verify artifacts.
//! 3. **Snapshot** — create recovery zip.
//! 4. **Apply** — atomic file moves + manifest commit.
//! 5. **Health scan** — post-apply verification.
//!
//! This module owns the pipeline types and the resolve phase. Staging, application,
//! and health scanning are added in C2.
//!
//! ```text
//! ┌─ Intent ─▶ Resolve ─▶ Plan (read-only) ─▶ Stage ─▶ Snapshot ─▶ Apply ─▶ Health ─▶ Result
//! ```
//!
//! Key invariants:
//! - Planning makes zero instance changes.
//! - Snapshot is taken BEFORE any instance mutation and is mandatory.
//! - The manifest atomic rename is the single commit point.
//! - Post-apply health failure keeps the installed files, preserves the
//!   recovery snapshot for manual rollback, and surfaces the health report
//!   so the user can inspect and repair.
//!
//! Loader-mismatch gate (Work Package 8): when a plan adds local-file
//! artifacts whose loader requirements the current loader does not satisfy,
//! the resolve phase surfaces a structured `PendingChoice::LoaderChange`.
//! Approval is a backend override (`PlanOverrides::approve_loader_version`
//! naming a signed-catalog version that satisfies every hard requirement —
//! usually the recommendation); the accepted switch is committed inside the
//! same manifest transaction, so the loader version and the mod files change
//! atomically and roll back together.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::task_scheduler::{BlockingPriority, TaskScheduler};

// Reuse types from dependency_ops to avoid duplication.
pub use crate::dependency_ops::{DepSource, Requirement};

/// Optional host-owned resources used while executing an install. Core-only
/// callers can use the default; application hosts supply their bounded worker
/// scheduler and current registry so post-install caches match launch inputs.
#[derive(Clone, Copy, Default)]
pub struct InstallExecutionResources<'a> {
    pub scheduler: Option<&'a TaskScheduler>,
    pub registry_db_path: Option<&'a Path>,
    /// Production callers supply the core loader service so an approved loader
    /// switch installs its signed profile before the manifest commit point.
    pub loader_service: Option<&'a crate::loader_service::LoaderService>,
}

// ---------------------------------------------------------------------------
// 1. Operation type
// ---------------------------------------------------------------------------

/// What the user wants to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallOperation {
    Install,
    Update,
    Remove,
}

// ---------------------------------------------------------------------------
// 2. Source type
// ---------------------------------------------------------------------------

/// Where the item comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceType {
    /// Curated registry item (GitHub Release).
    Curated,
    /// Raw Modrinth project.
    Modrinth,
    /// Local file path.
    Manual,
}

// ---------------------------------------------------------------------------
// 3. Dependency policy
// ---------------------------------------------------------------------------

/// How optional dependencies are handled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum OptionalDepsPolicy {
    /// Include only these specific optional deps (empty = none).
    Include { deps: Vec<String> },
    /// Skip all optional deps.
    ExcludeAll,
    /// Prompt the user (returns choices as pending_choices).
    Prompt,
}

// ---------------------------------------------------------------------------
// 4. Request context
// ---------------------------------------------------------------------------

/// Who or what initiated this install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequestSource {
    Interactive,
    CLI,
    AutoUpdate,
}

// ---------------------------------------------------------------------------
// 5. Artifact locator
// ---------------------------------------------------------------------------

/// Describes where to obtain the artifact content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ArtifactSource {
    /// Download from a URL.
    Download { url: String },
    /// Use a local file directly (manual mod install).
    LocalFile { path: String },
}

// ---------------------------------------------------------------------------
// 6. Hash spec — stores multiple algorithms for defense in depth
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HashSpec {
    /// Ordered by preference (strongest first). At least one entry required.
    /// SHA-256 is mandatory for curated items; SHA-1 accepted for Modrinth
    /// backward compatibility only if accompanied by a stronger hash.
    pub values: Vec<HashedValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HashedValue {
    pub algorithm: HashAlgorithm,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HashAlgorithm {
    Sha256,
    Sha512,
    Sha1,
}

// ---------------------------------------------------------------------------
// 7. Resolved item — typed by source
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedDownload {
    pub item_id: String,
    pub version_id: String,
    pub source: ArtifactSource,
    pub hashes: HashSpec,
    pub size: u64,
    /// The filename that will be written to mods/.
    pub filename: String,
    pub metadata: ArtifactMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedLocal {
    pub item_id: String,
    pub source_path: String,
    pub hashes: HashSpec, // computed at staging time
    pub size: u64,
    pub filename: String,
    pub metadata: ArtifactMetadata,
}

/// Manifest identity carried with a backend-resolved artifact. This data is
/// never trusted from the frontend; the Tauri facade resolves it from the
/// signed registry or Modrinth response before the core builds a plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMetadata {
    pub source_type: SourceType,
    pub registry_id: Option<String>,
    pub modrinth_id: Option<String>,
    pub content_type: String,
    /// Human-readable release/version number. This is intentionally separate
    /// from `ResolvedDownload.version_id`, which may be a Modrinth UUID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Curated-resolver download strategy (`direct_hash`, `github_release`,
    /// `modrinth_id`, ...). Present only for curated items; tells the staging
    /// path how the artifact may be fetched (e.g. `direct_hash` artifacts are
    /// fetched via the signed-manifest host policy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_strategy: Option<String>,
    /// Host of the curator-pinned `source_identifier` for hand-pinned
    /// strategies. Sourced from the signed registry row, never re-derived from
    /// the URL being fetched, so it is a real constraint on the request rather
    /// than a tautology.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ResolvedArtifact {
    Download(ResolvedDownload),
    LocalFile(ResolvedLocal),
}

// ---------------------------------------------------------------------------
// 8. Dependency disposition
// ---------------------------------------------------------------------------

/// How a resolved dependency relates to the instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum DepDisposition {
    /// Already installed at a compatible version — no action needed.
    ReuseExisting {
        mod_jar_id: String,
        installed_filename: String,
    },
    /// Will be downloaded and installed.
    InstallCandidate { artifact: Box<ResolvedArtifact> },
    /// Another item in the same batch installs this dependency — no action
    /// needed here and no duplicate artifact is added.
    IncludedInBatch { target_filename: String },
    /// User chose to exclude this optional dependency.
    Excluded,
    /// Could not be resolved — kept for diagnostics.
    Unresolved { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedDep {
    pub mod_jar_id: String,
    pub requirement: Requirement,
    pub source: DepSource,
    pub disposition: DepDisposition,
    /// Human-readable project name when it could be resolved cheaply
    /// (registry lookup for curated deps, batched Modrinth fetch for raw
    /// deps). Falls back to `mod_jar_id` in the UI when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Canonical upstream page URL (registry `page_url` or Modrinth project
    /// URL). Absent when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
}

// ---------------------------------------------------------------------------
// 9. Conflict
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepConflict {
    /// Stable identifier for this conflict (used in PendingChoice responses).
    pub conflict_id: String,
    pub kind: ConflictKind,
    pub existing_mod_jar_id: String,
    pub incoming_mod_jar_id: String,
    pub message: String,
    pub blocking: bool,
    pub resolution_options: Vec<ConflictResolution>,
    /// Set by user override or by the resolver for non-blocking defaults.
    pub chosen: Option<ConflictResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictKind {
    VersionConflict,
    DuplicateMod,
    LoaderMismatch,
    GameVersionMismatch,
    IncompatibleMod,
    BrokenReverseDep,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictResolution {
    Replace,
    Skip,
    DisableExisting,
    Abort,
}

// ---------------------------------------------------------------------------
// 10. File actions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAdd {
    pub target_filename: String,
    pub staging_filename: String,
    pub artifact: ResolvedArtifact,
    pub hashes: HashSpec,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRemove {
    pub filename: String,
    /// When set, removal is scoped to this content type (mod, resourcepack,
    /// shader, datapack, world). When None, the legacy behaviour applies:
    /// all content collections are searched, and the first filename match
    /// wins.  This avoids collisions when e.g. a resource pack and a mod
    /// share the same filename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDisable {
    pub filename: String,
}

// ---------------------------------------------------------------------------
// 11. Warnings & errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanError {
    pub code: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// 12. Mandatory snapshot plan
// ---------------------------------------------------------------------------

/// Snapshot is always required for mutating operations.
/// This struct carries only the parameters, not an optional flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotPlan {
    pub label: String, // encodes plan fingerprint + timestamp
    pub estimated_bytes: u64,
}

// ---------------------------------------------------------------------------
// 13. Disk-space estimate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpaceEstimate {
    pub download_bytes: u64,
    pub snapshot_bytes: u64,
    pub apply_overhead_bytes: u64,
    /// Peak additional disk usage during the transaction.
    pub peak_additional_bytes: u64,
    /// Change in committed disk usage after the transaction.
    pub post_commit_delta_bytes: i64,
}

impl DiskSpaceEstimate {
    pub fn zero() -> Self {
        Self {
            download_bytes: 0,
            snapshot_bytes: 0,
            apply_overhead_bytes: 0,
            peak_additional_bytes: 0,
            post_commit_delta_bytes: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// 14. Pending choices — typed with stable identifiers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum PendingChoice {
    OptionalDependencies {
        choice_id: String,
        options: Vec<OptionalDepOption>,
    },
    Conflict {
        choice_id: String,
        conflict_id: String,
        options: Vec<ConflictResolutionOption>,
    },
    /// The plan would add files whose loader requirements the current loader
    /// does not satisfy. Structured evidence mirrors the signed-catalog
    /// report; approving the change is a backend override
    /// (`PlanOverrides::approve_loader_version`), which folds the version
    /// switch into the same install transaction.
    LoaderChange {
        choice_id: String,
        loader: String,
        current_version: String,
        recommended_version: String,
        compatible_versions: Vec<String>,
        requirements: Vec<crate::health::LoaderRequirementIssue>,
        conflicts: Vec<crate::loader_compatibility::LoaderConflict>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionalDepOption {
    pub mod_jar_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictResolutionOption {
    pub resolution: ConflictResolution,
    pub label: String,
    pub description: String,
}

/// An approved loader version switch that is committed atomically with the
/// rest of the install transaction (the manifest rename is the commit point,
/// and the recovery snapshot covers `instance_manifest.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderChangePlan {
    pub loader: String,
    pub from_version: String,
    pub to_version: String,
}

// ---------------------------------------------------------------------------
// 15. Plan overrides
// ---------------------------------------------------------------------------

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanOverrides {
    pub allow_replace: bool,
    pub skip_health_scan: bool,
    /// Permit the resolver to retry a failed exact/compatible version lookup
    /// using the nearest available candidate after explicit user approval.
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_closest_version: bool,
    /// Root batch items explicitly skipped after a version-resolution failure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip_items: Vec<String>,
    /// Override applied conflict resolutions by conflict_id.
    pub force_conflict_resolution: BTreeMap<String, ConflictResolution>,
    /// Explicit approval to switch the instance loader to a signed-catalog
    /// version that satisfies every hard requirement of the incoming mods, so
    /// the install can proceed. The change is applied inside the same install
    /// transaction. Any compatible version from the signed catalog is
    /// accepted (the recommended version is the common case); any other value
    /// leaves the `PendingChoice::LoaderChange` unresolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approve_loader_version: Option<String>,
}

// ---------------------------------------------------------------------------
// 16. Operation-specific resolved payload
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ResolvedOperation {
    Install {
        artifact: ResolvedArtifact,
    },
    Update {
        old_version_id: String,
        new_artifact: ResolvedArtifact,
    },
    Remove {
        target_filename: String,
        reverse_dependents: Vec<ReverseDepInfo>,
        /// Optional hint narrowing removal to one content type.  When set,
        /// the resolver produces a scoped `FileRemove` instead of searching
        /// every content collection for the filename.
        content_type: Option<String>,
    },
    BatchRemove {
        operations: Vec<ResolvedOperation>,
    },
    BatchUpdate {
        operations: Vec<ResolvedOperation>,
    },
    BatchInstall {
        operations: Vec<ResolvedOperation>,
    },
    Reconcile {
        operations: Vec<ResolvedOperation>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReverseDepInfo {
    pub mod_jar_id: String,
    pub filename: String,
    pub requirement: Requirement,
    /// How this reverse dep will be affected. None = unchanged.
    pub impact: Option<String>,
}

// ---------------------------------------------------------------------------
// 17. InstallIntent — action-tagged for operation safety
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum InstallAction {
    Install {
        source_type: SourceType,
        item_id: String,
        candidate_version: Option<String>,
    },
    Update {
        item_id: String,
        target_version: String,
    },
    Remove {
        filename: String,
    },
    BatchRemove {
        filenames: Vec<String>,
    },
    BatchUpdate {
        items: Vec<BatchUpdateItem>,
    },
    BatchInstall {
        items: Vec<BatchInstallItem>,
    },
    RepairLockfile {
        content_hash: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchUpdateItem {
    pub item_id: String,
    pub target_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchInstallItem {
    pub source_type: SourceType,
    pub item_id: String,
    pub candidate_version: Option<String>,
}

/// Pure input — what the user wants to do.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallIntent {
    pub action: InstallAction,
    pub target_instance: String,
    pub optional_deps: OptionalDepsPolicy,
    pub requested_by: RequestSource,
    pub overrides: PlanOverrides,
}

// ---------------------------------------------------------------------------
// 18. ResolvedInstallPlan
// ---------------------------------------------------------------------------

/// Full read-only plan. Making a plan commits to no instance changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedInstallPlan {
    pub fingerprint: String,
    pub intent: InstallIntent,
    pub operation: ResolvedOperation,
    pub dependencies: Vec<ResolvedDep>,
    pub conflicts: Vec<DepConflict>,
    pub files_to_add: Vec<FileAdd>,
    pub files_to_remove: Vec<FileRemove>,
    pub files_to_disable: Vec<FileDisable>,
    pub snapshot: SnapshotPlan,
    pub disk_estimate: DiskSpaceEstimate,
    pub warnings: Vec<PlanWarning>,
    pub blocking_errors: Vec<PlanError>,
    pub pending_choices: Vec<PendingChoice>,
    /// Approved loader version switch committed atomically with this plan's
    /// file changes. Absent unless the loader-change choice was approved via
    /// `PlanOverrides::approve_loader_version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_change: Option<LoaderChangePlan>,
    pub created_at: String,
    pub instance_state_hash: String,
    pub registry_revision: String,
}

/// Backend-prepared resolution facts. Network access and registry DB handles
/// remain outside `agora-core`; the core owns normalization, conflict policy,
/// file actions, fingerprinting, and mutation rules.
#[derive(Debug, Clone)]
pub struct PreparedPlan {
    pub operation: ResolvedOperation,
    pub dependencies: Vec<ResolvedDep>,
    pub conflicts: Vec<DepConflict>,
    pub registry_revision: String,
}

impl ResolvedInstallPlan {
    /// Whether the plan is fully resolved and can be submitted for execution.
    ///
    /// Does NOT check freshness — the executor must revalidate `instance_state_hash`
    /// and `registry_revision` against the current state before applying.
    pub fn is_fully_resolved(&self) -> bool {
        if !self.blocking_errors.is_empty() {
            return false;
        }
        if !self.pending_choices.is_empty() {
            return false;
        }
        // Every blocking conflict must have a chosen resolution.
        if self
            .conflicts
            .iter()
            .any(|c| c.blocking && c.chosen.is_none())
        {
            return false;
        }
        true
    }
}

// ---------------------------------------------------------------------------
// 19. Fingerprint input — deterministic ordering via BTreeMap
// ---------------------------------------------------------------------------

/// A dedicated input type for plan fingerprint computation.
/// Uses BTreeMap and sorted collections for deterministic serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFingerprintInput {
    pub schema_version: u32,
    pub action: InstallAction,
    pub resolved_artifacts: BTreeMap<String, ArtifactFingerprint>,
    pub dependency_dispositions: BTreeMap<String, String>,
    pub conflict_resolutions: BTreeMap<String, String>,
    pub instance_state_hash: String,
    pub registry_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactFingerprint {
    pub source_kind: String, // "download" | "local"
    pub version_id: String,
    pub filename: String,
    pub hashes: Vec<(String, String)>, // (algorithm, value) sorted pairs
    pub size: u64,
}

// ---------------------------------------------------------------------------
// 20. Health outcome
// ---------------------------------------------------------------------------

use crate::health::HealthReport;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum HealthOutcome {
    Completed { report: HealthReport },
    Skipped { reason: String },
}

// ---------------------------------------------------------------------------
// 21. InstallResult — typed outcome
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum InstallOutcome {
    Success {
        installed_items: Vec<String>,
        existing_items_reused: Vec<String>,
        warnings: Vec<PlanWarning>,
        health: HealthOutcome,
        snapshot_id: String,
    },
    /// Post-apply health found blockers. The install is **kept** so the
    /// user can inspect the report and repair (disable, loader switch,
    /// or manual snapshot rollback). The `snapshot_id` is the pre-install
    /// recovery snapshot that can be restored on demand.
    HealthRollback {
        health_report: HealthReport,
        snapshot_id: String,
        warnings: Vec<PlanWarning>,
    },
    Cancelled {
        phase: String,
        rollback_performed: bool,
    },
    Failed {
        error: String,
        rollback_performed: bool,
        snapshot_id: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// 22. Progress events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub plan_id: String,
    pub phase: ProgressPhase,
    pub step: u32,
    pub total_steps: u32,
    pub bytes_downloaded: u64,
    pub bytes_total: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProgressPhase {
    Resolving,
    Staging,
    Snapshotting,
    Applying,
    HealthScan,
    Done,
    Failed,
    Cancelled,
}

// ---------------------------------------------------------------------------
// 23. CancellationToken — re-exported from canonical event_sink module
// ---------------------------------------------------------------------------

/// Re-exported from `crate::event_sink` — the canonical cancellation type.
pub use crate::event_sink::CancellationToken;

// ---------------------------------------------------------------------------
// 24. ProgressReporter trait (Tauri-agnostic, uses legacy ProgressEvent)
// ---------------------------------------------------------------------------

pub trait ProgressReporter: Send + Sync {
    fn report(&self, event: ProgressEvent);
}

// ---------------------------------------------------------------------------
// 25. InstallPipeline
// ---------------------------------------------------------------------------

pub struct InstallPipeline;

impl InstallPipeline {
    /// Phase 1: normalize backend-resolved facts into a deterministic,
    /// read-only plan. Registry/network access happens in the facade; all
    /// business rules, file actions, conflicts, and fingerprinting live here.
    pub async fn resolve_plan(
        &self,
        intent: InstallIntent,
        instance_dir: &Path,
        prepared: PreparedPlan,
        reporter: &dyn ProgressReporter,
    ) -> Result<ResolvedInstallPlan, String> {
        reporter.report(ProgressEvent {
            plan_id: String::new(),
            phase: ProgressPhase::Resolving,
            step: 0,
            total_steps: 1,
            bytes_downloaded: 0,
            bytes_total: 0,
            message: "Resolving install plan…".into(),
        });

        let manifest_text = std::fs::read_to_string(instance_dir.join("instance_manifest.json"))
            .map_err(|e| format!("failed to read instance manifest: {e}"))?;
        let manifest: crate::models::InstanceManifest = serde_json::from_str(&manifest_text)
            .map_err(|e| format!("failed to parse instance manifest: {e}"))?;
        let live_index = crate::snapshot::live_file_index(instance_dir)?;
        let instance_state_hash = hash_serializable(&live_index)?;

        let PreparedPlan {
            operation,
            mut dependencies,
            mut conflicts,
            registry_revision,
        } = prepared;
        dependencies.sort_by(|a, b| a.mod_jar_id.cmp(&b.mod_jar_id));
        conflicts.sort_by(|a, b| a.conflict_id.cmp(&b.conflict_id));

        let mut warnings = Vec::new();
        let mut blocking_errors = Vec::new();
        let mut pending_choices = Vec::new();
        if intent.overrides.allow_closest_version {
            warnings.push(PlanWarning {
                code: "WARN_CLOSEST_VERSION".into(),
                message: "A closest available version was selected where the exact compatible version was unavailable. Verify the instance after installation.".into(),
            });
        }
        if manifest.is_locked {
            blocking_errors.push(PlanError {
                code: "ERR_INSTANCE_LOCKED".into(),
                message: "This instance is locked. Unlock it explicitly before changing files."
                    .into(),
            });
        }
        if registry_revision.is_empty() {
            blocking_errors.push(PlanError {
                code: "ERR_REGISTRY_REVISION".into(),
                message:
                    "The registry revision is unavailable, so plan freshness cannot be verified."
                        .into(),
            });
        }

        apply_optional_policy(
            &intent.optional_deps,
            &mut dependencies,
            &mut pending_choices,
        );
        for dependency in &dependencies {
            let dependency_name = dependency
                .display_name
                .as_deref()
                .unwrap_or(&dependency.mod_jar_id);
            if dependency.requirement == Requirement::Required {
                if let DepDisposition::Unresolved { reason } = &dependency.disposition {
                    blocking_errors.push(PlanError {
                        code: "ERR_REQUIRED_DEPENDENCY".into(),
                        message: format!(
                            "Required dependency {dependency_name} could not be resolved: {reason}"
                        ),
                    });
                }
            } else if matches!(&intent.optional_deps, OptionalDepsPolicy::Include { .. }) {
                if let DepDisposition::Unresolved { reason } = &dependency.disposition {
                    blocking_errors.push(PlanError {
                        code: "ERR_OPTIONAL_DEPENDENCY".into(),
                        message: format!(
                            "Selected optional dependency {dependency_name} could not be resolved: {reason}"
                        ),
                    });
                }
            }
        }

        apply_conflict_choices(
            &intent,
            &mut conflicts,
            &mut pending_choices,
            &mut blocking_errors,
        );

        let installed = all_installed(&manifest);
        let mut files_to_add = Vec::new();
        let mut files_to_remove = Vec::new();
        let mut files_to_disable = Vec::new();
        match &operation {
            ResolvedOperation::Install { artifact } => plan_artifact_change(
                artifact,
                false,
                &installed,
                &intent,
                &mut files_to_add,
                &mut files_to_remove,
                &mut conflicts,
                &mut pending_choices,
                &mut blocking_errors,
                &mut warnings,
            ),
            ResolvedOperation::Update { new_artifact, .. } => plan_artifact_change(
                new_artifact,
                true,
                &installed,
                &intent,
                &mut files_to_add,
                &mut files_to_remove,
                &mut conflicts,
                &mut pending_choices,
                &mut blocking_errors,
                &mut warnings,
            ),
            ResolvedOperation::Remove {
                target_filename,
                reverse_dependents,
                content_type: hint_content_type,
            } => {
                if validate_filename(target_filename).is_err() {
                    blocking_errors.push(PlanError {
                        code: "ERR_UNSAFE_FILENAME".into(),
                        message: format!("Unsafe removal filename: {target_filename}"),
                    });
                } else if installed
                    .iter()
                    .any(|item| item.filename == *target_filename)
                    || instance_dir.join("mods").join(target_filename).is_file()
                {
                    let ct = hint_content_type.clone().or_else(|| {
                        installed
                            .iter()
                            .find(|item| item.filename == *target_filename)
                            .map(|item| item.content_type.clone())
                    });
                    files_to_remove.push(FileRemove {
                        filename: target_filename.clone(),
                        content_type: ct,
                    });
                } else {
                    blocking_errors.push(PlanError {
                        code: "ERR_NOT_INSTALLED".into(),
                        message: format!("{target_filename} is not installed in this instance."),
                    });
                }
                for dependent in reverse_dependents {
                    if dependent.requirement == Requirement::Required {
                        blocking_errors.push(PlanError {
                            code: "ERR_BROKEN_REVERSE_DEP".into(),
                            message: format!(
                                "Removing {target_filename} would break required dependency for {}.",
                                dependent.mod_jar_id
                            ),
                        });
                    }
                }
            }
            ResolvedOperation::BatchUpdate { operations } => {
                if operations.is_empty() {
                    blocking_errors.push(PlanError {
                        code: "ERR_EMPTY_BATCH".into(),
                        message: "No updates were selected.".into(),
                    });
                }
                for operation in operations {
                    match operation {
                        ResolvedOperation::Update { new_artifact, .. } => plan_artifact_change(
                            new_artifact,
                            true,
                            &installed,
                            &intent,
                            &mut files_to_add,
                            &mut files_to_remove,
                            &mut conflicts,
                            &mut pending_choices,
                            &mut blocking_errors,
                            &mut warnings,
                        ),
                        _ => blocking_errors.push(PlanError {
                            code: "ERR_INVALID_BATCH_OPERATION".into(),
                            message: "A batch-update plan contained a non-update operation.".into(),
                        }),
                    }
                }
            }
            ResolvedOperation::BatchRemove { operations } => {
                if operations.is_empty() {
                    blocking_errors.push(PlanError {
                        code: "ERR_EMPTY_BATCH".into(),
                        message: "No content was selected for removal.".into(),
                    });
                }
                for operation in operations {
                    match operation {
                        ResolvedOperation::Remove {
                            target_filename,
                            reverse_dependents,
                            content_type: hint_content_type,
                        } => {
                            if validate_filename(target_filename).is_err() {
                                blocking_errors.push(PlanError {
                                    code: "ERR_UNSAFE_FILENAME".into(),
                                    message: format!("Unsafe removal filename: {target_filename}"),
                                });
                            } else if installed
                                .iter()
                                .any(|item| item.filename == *target_filename)
                                || instance_dir.join("mods").join(target_filename).is_file()
                            {
                                let ct = hint_content_type.clone().or_else(|| {
                                    installed
                                        .iter()
                                        .find(|item| item.filename == *target_filename)
                                        .map(|item| item.content_type.clone())
                                });
                                files_to_remove.push(FileRemove {
                                    filename: target_filename.clone(),
                                    content_type: ct,
                                });
                            } else {
                                blocking_errors.push(PlanError {
                                    code: "ERR_NOT_INSTALLED".into(),
                                    message: format!(
                                        "{target_filename} is not installed in this instance."
                                    ),
                                });
                            }
                            for dependent in reverse_dependents {
                                if dependent.requirement == Requirement::Required {
                                    blocking_errors.push(PlanError {
                                        code: "ERR_BROKEN_REVERSE_DEP".into(),
                                        message: format!(
                                            "Removing {target_filename} would break required dependency for {}.",
                                            dependent.mod_jar_id
                                        ),
                                    });
                                }
                            }
                        }
                        _ => blocking_errors.push(PlanError {
                            code: "ERR_INVALID_BATCH_OPERATION".into(),
                            message: "A batch-remove plan contained a non-remove operation.".into(),
                        }),
                    }
                }
            }
            ResolvedOperation::BatchInstall { operations } => {
                if operations.is_empty() {
                    blocking_errors.push(PlanError {
                        code: "ERR_EMPTY_BATCH".into(),
                        message: "No items were selected for installation.".into(),
                    });
                }
                for operation in operations {
                    match operation {
                        ResolvedOperation::Install { artifact } => plan_artifact_change(
                            artifact,
                            false,
                            &installed,
                            &intent,
                            &mut files_to_add,
                            &mut files_to_remove,
                            &mut conflicts,
                            &mut pending_choices,
                            &mut blocking_errors,
                            &mut warnings,
                        ),
                        _ => blocking_errors.push(PlanError {
                            code: "ERR_INVALID_BATCH_OPERATION".into(),
                            message: "A batch-install plan contained a non-install operation."
                                .into(),
                        }),
                    }
                }
            }
            ResolvedOperation::Reconcile { operations } => {
                if operations.is_empty() {
                    warnings.push(PlanWarning {
                        code: "LOCKFILE_ALREADY_IN_SYNC".into(),
                        message: "The instance already matches this lockfile.".into(),
                    });
                }
                for operation in operations {
                    match operation {
                        ResolvedOperation::Install { artifact } => plan_artifact_change(
                            artifact,
                            false,
                            &installed,
                            &intent,
                            &mut files_to_add,
                            &mut files_to_remove,
                            &mut conflicts,
                            &mut pending_choices,
                            &mut blocking_errors,
                            &mut warnings,
                        ),
                        ResolvedOperation::Update { new_artifact, .. } => plan_artifact_change(
                            new_artifact,
                            true,
                            &installed,
                            &intent,
                            &mut files_to_add,
                            &mut files_to_remove,
                            &mut conflicts,
                            &mut pending_choices,
                            &mut blocking_errors,
                            &mut warnings,
                        ),
                        ResolvedOperation::Remove {
                            target_filename,
                            reverse_dependents,
                            content_type: hint_content_type,
                        } => {
                            if validate_filename(target_filename).is_err() {
                                blocking_errors.push(PlanError {
                                    code: "ERR_UNSAFE_FILENAME".into(),
                                    message: format!("Unsafe removal filename: {target_filename}"),
                                });
                            } else {
                                let ct = hint_content_type.clone().or_else(|| {
                                    installed
                                        .iter()
                                        .find(|item| item.filename == *target_filename)
                                        .map(|item| item.content_type.clone())
                                });
                                files_to_remove.push(FileRemove {
                                    filename: target_filename.clone(),
                                    content_type: ct,
                                });
                            }
                            for dependent in reverse_dependents {
                                if dependent.requirement == Requirement::Required {
                                    blocking_errors.push(PlanError {
                                        code: "ERR_BROKEN_REVERSE_DEP".into(),
                                        message: format!(
                                            "Removing {target_filename} would break required dependency for {}.",
                                            dependent.mod_jar_id
                                        ),
                                    });
                                }
                            }
                        }
                        _ => blocking_errors.push(PlanError {
                            code: "ERR_INVALID_RECONCILE_OPERATION".into(),
                            message: "A lockfile repair contained an unsupported nested operation."
                                .into(),
                        }),
                    }
                }
            }
        }

        for dependency in &dependencies {
            if let DepDisposition::InstallCandidate { artifact } = &dependency.disposition {
                plan_artifact_change(
                    artifact,
                    false,
                    &installed,
                    &intent,
                    &mut files_to_add,
                    &mut files_to_remove,
                    &mut conflicts,
                    &mut pending_choices,
                    &mut blocking_errors,
                    &mut warnings,
                );
            }
        }
        for conflict in &conflicts {
            if conflict.chosen == Some(ConflictResolution::DisableExisting) {
                if let Some(existing) = installed.iter().find(|item| {
                    installed_identity(item).as_deref() == Some(&conflict.existing_mod_jar_id)
                }) {
                    files_to_disable.push(FileDisable {
                        filename: existing.filename.clone(),
                    });
                }
            }
        }

        files_to_add.sort_by(|a, b| a.target_filename.cmp(&b.target_filename));
        for pair in files_to_add.windows(2) {
            if pair[0].target_filename == pair[1].target_filename
                && hash_serializable(&pair[0].artifact)? != hash_serializable(&pair[1].artifact)?
            {
                blocking_errors.push(PlanError {
                    code: "ERR_DUPLICATE_TARGET".into(),
                    message: format!(
                        "Multiple different artifacts resolve to {}.",
                        pair[0].target_filename
                    ),
                });
            }
        }
        files_to_add.dedup_by(|a, b| a.target_filename == b.target_filename);

        let loader_change = apply_loader_change_policy(
            &manifest,
            &intent,
            instance_dir,
            &files_to_add,
            &files_to_remove,
            &files_to_disable,
            &mut pending_choices,
            &mut warnings,
        );
        files_to_remove.sort_by(|a, b| a.filename.cmp(&b.filename));
        files_to_remove
            .dedup_by(|a, b| a.filename == b.filename && a.content_type == b.content_type);
        files_to_disable.sort_by(|a, b| a.filename.cmp(&b.filename));
        files_to_disable.dedup_by(|a, b| a.filename == b.filename);
        conflicts.sort_by(|a, b| a.conflict_id.cmp(&b.conflict_id));
        conflicts.dedup_by(|a, b| a.conflict_id == b.conflict_id);
        pending_choices.sort_by(|a, b| pending_choice_id(a).cmp(pending_choice_id(b)));
        pending_choices.dedup_by(|a, b| pending_choice_id(a) == pending_choice_id(b));
        blocking_errors.sort_by(|a, b| a.code.cmp(&b.code).then(a.message.cmp(&b.message)));
        blocking_errors.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        warnings.sort_by(|a, b| a.code.cmp(&b.code).then(a.message.cmp(&b.message)));
        warnings.dedup_by(|a, b| a.code == b.code && a.message == b.message);

        let snapshot_bytes = live_index.iter().map(|entry| entry.size).sum::<u64>();
        let download_bytes = files_to_add.iter().map(|file| file.size).sum::<u64>();
        let removed_bytes = files_to_remove
            .iter()
            .filter_map(|file| {
                live_index
                    .iter()
                    .find(|entry| entry.path.ends_with(&file.filename))
                    .map(|entry| entry.size)
            })
            .sum::<u64>();
        let disk_estimate = DiskSpaceEstimate {
            download_bytes,
            snapshot_bytes,
            apply_overhead_bytes: download_bytes,
            peak_additional_bytes: snapshot_bytes.saturating_add(download_bytes.saturating_mul(2)),
            post_commit_delta_bytes: download_bytes as i64 - removed_bytes as i64,
        };

        let mut plan = ResolvedInstallPlan {
            fingerprint: String::new(),
            intent,
            operation,
            dependencies,
            conflicts,
            files_to_add,
            files_to_remove,
            files_to_disable,
            snapshot: SnapshotPlan {
                label: String::new(),
                estimated_bytes: snapshot_bytes,
            },
            disk_estimate,
            warnings,
            blocking_errors,
            pending_choices,
            loader_change,
            created_at: chrono::Utc::now().to_rfc3339(),
            instance_state_hash,
            registry_revision,
        };
        plan.fingerprint = compute_plan_fingerprint(&plan)?;
        plan.snapshot.label = format!("install-{}", &plan.fingerprint[..16]);
        Ok(plan)
    }

    /// Execute a backend-owned, fully resolved plan with freshness checks,
    /// verified staging, recovery snapshot, reversible application, and a
    /// post-commit health gate.
    pub async fn execute_plan(
        &self,
        plan: &ResolvedInstallPlan,
        instance_dir: &Path,
        current_registry_revision: &str,
        reporter: &dyn ProgressReporter,
        cancel: &CancellationToken,
    ) -> InstallOutcome {
        self.execute_plan_with_scheduler(
            plan,
            instance_dir,
            current_registry_revision,
            reporter,
            cancel,
            InstallExecutionResources::default(),
        )
        .await
    }

    /// Execute a plan while routing the large synchronous phases through the
    /// shared bounded worker scheduler.  The compatibility wrapper above is
    /// retained for core-only callers and tests that do not own a context.
    pub async fn execute_plan_with_scheduler(
        &self,
        plan: &ResolvedInstallPlan,
        instance_dir: &Path,
        current_registry_revision: &str,
        reporter: &dyn ProgressReporter,
        cancel: &CancellationToken,
        resources: InstallExecutionResources<'_>,
    ) -> InstallOutcome {
        let scheduler = resources.scheduler;
        let registry_db_path = resources.registry_db_path;
        let loader_service = resources.loader_service;
        let fail = |error: String, snapshot_id: Option<String>, rollback_performed: bool| {
            InstallOutcome::Failed {
                error,
                rollback_performed,
                snapshot_id,
            }
        };

        match crate::snapshot::snapshot_readiness(instance_dir) {
            crate::snapshot::SnapshotReadiness::Ready => {}
            crate::snapshot::SnapshotReadiness::Pending => {
                return fail(
                    "The instance is still finalizing its initial recovery snapshot. Try again shortly.".into(),
                    None,
                    false,
                )
            }
            crate::snapshot::SnapshotReadiness::Failed => {
                return fail(
                    crate::snapshot::snapshot_readiness_error(instance_dir)
                        .unwrap_or_else(|| "The initial recovery snapshot failed.".into()),
                    None,
                    false,
                )
            }
        }

        if !plan.is_fully_resolved() {
            return fail(
                "Plan still has blocking errors, unresolved choices, or conflicts.".into(),
                None,
                false,
            );
        }
        if plan.fingerprint.len() != 64
            || !plan
                .fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return fail("Plan fingerprint is malformed.".into(), None, false);
        }
        match compute_plan_fingerprint(plan) {
            Ok(computed) if computed == plan.fingerprint => {}
            Ok(_) => {
                return fail(
                    "Plan contents do not match its fingerprint.".into(),
                    None,
                    false,
                )
            }
            Err(error) => return fail(error, None, false),
        }
        if plan.registry_revision != current_registry_revision {
            return fail(
                "The catalog changed after this plan was reviewed. Resolve a fresh plan before installing."
                    .into(),
                None,
                false,
            );
        }
        let state_dir = instance_dir.to_path_buf();
        let current_state_hash =
            match run_blocking_phase(scheduler, "install state verification", move || {
                crate::snapshot::live_file_index(&state_dir)
                    .and_then(|index| hash_serializable(&index))
            })
            .await
            {
                Ok(hash) => hash,
                Err(error) => {
                    return fail(
                        format!("Could not verify current instance state: {error}"),
                        None,
                        false,
                    )
                }
            };
        if current_state_hash != plan.instance_state_hash {
            return fail(
                "The instance changed after this plan was reviewed. Resolve a fresh plan before installing."
                    .into(),
                None,
                false,
            );
        }

        let staging_dir = instance_dir
            .join(".agora")
            .join("staging")
            .join(&plan.fingerprint);
        reporter.report(ProgressEvent {
            plan_id: plan.fingerprint.clone(),
            phase: ProgressPhase::Staging,
            step: 0,
            total_steps: plan.files_to_add.len() as u32,
            bytes_downloaded: 0,
            bytes_total: plan.disk_estimate.download_bytes,
            message: "Downloading and verifying every artifact…".into(),
        });
        if let Err(error) = stage_plan_artifacts(plan, &staging_dir, reporter, cancel).await {
            cleanup_staging_dir(scheduler, &staging_dir).await;
            if cancel.is_cancelled() {
                return InstallOutcome::Cancelled {
                    phase: "staging".into(),
                    rollback_performed: false,
                };
            }
            return fail(error, None, false);
        }
        if cancel.is_cancelled() {
            cleanup_staging_dir(scheduler, &staging_dir).await;
            return InstallOutcome::Cancelled {
                phase: "staging".into(),
                rollback_performed: false,
            };
        }

        if let Err(error) = validate_staged_loader_compatibility(plan, instance_dir, &staging_dir) {
            cleanup_staging_dir(scheduler, &staging_dir).await;
            return fail(error, None, false);
        }

        // The loader profile is shared cache material, so install it before
        // committing mod files or the future instance manifest. A later
        // transaction failure leaves only harmless verified cache material.
        if let Some(change) = &plan.loader_change {
            let Some(loader_service) = loader_service else {
                cleanup_staging_dir(scheduler, &staging_dir).await;
                return fail(
                    "Approved loader changes require the core loader service.".into(),
                    None,
                    false,
                );
            };
            let minecraft_version = match std::fs::read_to_string(
                instance_dir.join("instance_manifest.json"),
            )
            .ok()
            .and_then(|text| serde_json::from_str::<crate::models::InstanceManifest>(&text).ok())
            .map(|manifest| manifest.minecraft_version)
            {
                Some(version) => version,
                None => {
                    cleanup_staging_dir(scheduler, &staging_dir).await;
                    return fail(
                        "Could not read the instance Minecraft version before installing the approved loader.".into(),
                        None,
                        false,
                    );
                }
            };
            if let Err(error) = loader_service
                .ensure_installed(
                    &change.loader,
                    &minecraft_version,
                    &change.to_version,
                    false,
                )
                .await
            {
                cleanup_staging_dir(scheduler, &staging_dir).await;
                return fail(error.to_string(), None, false);
            }
        }

        // Build and sync the future manifest before taking the snapshot or
        // touching live files. Serialization/disk failures are pre-mutation.
        let prepare_plan = plan.clone();
        let prepare_dir = instance_dir.to_path_buf();
        let prepare_staging = staging_dir.clone();
        if let Err(error) =
            run_blocking_phase(scheduler, "install manifest preparation", move || {
                prepare_manifest(&prepare_plan, &prepare_dir, &prepare_staging)
            })
            .await
        {
            cleanup_staging_dir(scheduler, &staging_dir).await;
            return fail(error, None, false);
        }

        reporter.report(ProgressEvent {
            plan_id: plan.fingerprint.clone(),
            phase: ProgressPhase::Snapshotting,
            step: 0,
            total_steps: 1,
            bytes_downloaded: plan.disk_estimate.download_bytes,
            bytes_total: plan.disk_estimate.download_bytes,
            message: "Creating the recovery snapshot…".into(),
        });
        let snapshot_dir = instance_dir.to_path_buf();
        let snapshot_label = plan.snapshot.label.clone();
        let snapshot = match run_blocking_phase(scheduler, "install recovery snapshot", move || {
            crate::snapshot::create_snapshot(&snapshot_dir, Some(&snapshot_label))
        })
        .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                cleanup_staging_dir(scheduler, &staging_dir).await;
                return fail(
                    format!("Snapshot failed before apply: {error}"),
                    None,
                    false,
                );
            }
        };
        if cancel.is_cancelled() {
            cleanup_staging_dir(scheduler, &staging_dir).await;
            return InstallOutcome::Cancelled {
                phase: "snapshotting".into(),
                rollback_performed: false,
            };
        }

        reporter.report(ProgressEvent {
            plan_id: plan.fingerprint.clone(),
            phase: ProgressPhase::Applying,
            step: 0,
            total_steps: 1,
            bytes_downloaded: plan.disk_estimate.download_bytes,
            bytes_total: plan.disk_estimate.download_bytes,
            message: "Atomically applying files and manifest…".into(),
        });
        let apply_plan = plan.clone();
        let apply_dir = instance_dir.to_path_buf();
        let apply_staging = staging_dir.clone();
        if let Err(apply_error) =
            run_blocking_phase(scheduler, "install transaction apply", move || {
                apply_transaction(&apply_plan, &apply_dir, &apply_staging)
            })
            .await
        {
            let fast_rollback_performed =
                apply_error.contains("pre-commit file moves were reversed");
            let restore_dir = instance_dir.to_path_buf();
            let restore_id = snapshot.id.clone();
            let restore = run_blocking_phase(scheduler, "install rollback", move || {
                crate::snapshot::restore_snapshot(&restore_dir, &restore_id)
            })
            .await;
            cleanup_staging_dir(scheduler, &staging_dir).await;
            return match restore {
                Ok(()) => fail(
                    format!("Apply failed; the recovery snapshot was restored: {apply_error}"),
                    Some(snapshot.id),
                    true,
                ),
                Err(restore_error) => fail(
                    format!(
                        "Apply failed and automatic restore could not complete. Original state remains protected in recovery storage. Apply error: {apply_error}; restore error: {restore_error}"
                    ),
                    Some(snapshot.id),
                    fast_rollback_performed,
                ),
            };
        }

        reporter.report(ProgressEvent {
            plan_id: plan.fingerprint.clone(),
            phase: ProgressPhase::HealthScan,
            step: 0,
            total_steps: 1,
            bytes_downloaded: plan.disk_estimate.download_bytes,
            bytes_total: plan.disk_estimate.download_bytes,
            message: "Checking the installed instance…".into(),
        });
        let health = if plan.intent.overrides.skip_health_scan {
            let cache_dir = instance_dir.to_path_buf();
            let cache_result =
                run_blocking_phase(scheduler, "install JAR metadata precompute", move || {
                    let manifest_path = cache_dir.join("instance_manifest.json");
                    let manifest_text = std::fs::read_to_string(&manifest_path)
                        .map_err(|error| format!("committed manifest is unreadable ({error})"))?;
                    let manifest: crate::models::InstanceManifest =
                        serde_json::from_str(&manifest_text)
                            .map_err(|error| format!("committed manifest is invalid ({error})"))?;
                    crate::health::precompute_jar_metadata_cache(&cache_dir, &manifest)
                })
                .await;
            HealthOutcome::Skipped {
                reason: match cache_result {
                    Ok(()) => "The caller explicitly skipped the post-install health scan; JAR metadata was precomputed for the next launch.".into(),
                    Err(error) => format!("The caller explicitly skipped the post-install health scan; cache precomputation was unavailable: {error}"),
                },
            }
        } else {
            let health_dir = instance_dir.to_path_buf();
            let health_registry = registry_db_path.map(Path::to_path_buf);
            let report =
                match run_blocking_phase(scheduler, "post-install health scan", move || {
                    let manifest_text =
                        std::fs::read_to_string(health_dir.join("instance_manifest.json"))
                            .map_err(|error| {
                                format!("Committed manifest is unreadable ({error})")
                            })?;
                    let manifest: crate::models::InstanceManifest =
                        serde_json::from_str(&manifest_text)
                            .map_err(|error| format!("Committed manifest is invalid ({error})"))?;
                    // The post-commit health pass is already part of the
                    // install's longer-running work.  Use the cache-aware
                    // path so this scan publishes both the durable report
                    // and per-JAR metadata for the next launch instead of
                    // making launch parse the same archives again.
                    Ok(crate::health::cached_health(
                        &health_dir,
                        &manifest,
                        health_registry.as_deref(),
                        None,
                    ))
                })
                .await
                {
                    Ok(report) => report,
                    Err(error) => {
                        let restore_dir = instance_dir.to_path_buf();
                        let restore_id = snapshot.id.clone();
                        let restore =
                            run_blocking_phase(scheduler, "install health rollback", move || {
                                crate::snapshot::restore_snapshot(&restore_dir, &restore_id)
                            })
                            .await;
                        cleanup_staging_dir(scheduler, &staging_dir).await;
                        return fail(
                            format!("{error}; restore result: {restore:?}"),
                            Some(snapshot.id),
                            restore.is_ok(),
                        );
                    }
                };
            if !report.blockers.is_empty() {
                // Keep the newly installed files so the user can see the exact
                // health failure, inspect blockers, and choose a repair (disable,
                // loader switch, or manual rollback) instead of silently
                // reverting to the pre-install snapshot.
                cleanup_staging_dir(scheduler, &staging_dir).await;
                reporter.report(ProgressEvent {
                    plan_id: plan.fingerprint.clone(),
                    phase: ProgressPhase::Done,
                    step: 1,
                    total_steps: 1,
                    bytes_downloaded: plan.disk_estimate.download_bytes,
                    bytes_total: plan.disk_estimate.download_bytes,
                    message: "Install complete — health issues need attention.".into(),
                });
                return InstallOutcome::HealthRollback {
                    health_report: report,
                    snapshot_id: snapshot.id,
                    warnings: plan.warnings.clone(),
                };
            }
            HealthOutcome::Completed { report }
        };

        cleanup_staging_dir(scheduler, &staging_dir).await;
        reporter.report(ProgressEvent {
            plan_id: plan.fingerprint.clone(),
            phase: ProgressPhase::Done,
            step: 1,
            total_steps: 1,
            bytes_downloaded: plan.disk_estimate.download_bytes,
            bytes_total: plan.disk_estimate.download_bytes,
            message: "Install complete.".into(),
        });
        InstallOutcome::Success {
            installed_items: plan
                .files_to_add
                .iter()
                .map(|file| artifact_item_id(&file.artifact).to_string())
                .collect(),
            existing_items_reused: plan
                .dependencies
                .iter()
                .filter_map(|dependency| match &dependency.disposition {
                    DepDisposition::ReuseExisting { mod_jar_id, .. } => Some(mod_jar_id.clone()),
                    _ => None,
                })
                .collect(),
            warnings: plan.warnings.clone(),
            health,
            snapshot_id: snapshot.id,
        }
    }
}

// -----------------------------------------------------------------------
// Internal helpers
// -----------------------------------------------------------------------

async fn run_blocking_phase<T, F>(
    scheduler: Option<&TaskScheduler>,
    label: &'static str,
    task: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    match scheduler {
        Some(scheduler) => scheduler
            .run_blocking(BlockingPriority::UserInitiated, task)
            .await
            .map_err(|error| format!("{label} worker failed: {error}"))?,
        None => task(),
    }
}

async fn cleanup_staging_dir(scheduler: Option<&TaskScheduler>, staging_dir: &Path) {
    let path = staging_dir.to_path_buf();
    let _ = run_blocking_phase(scheduler, "install staging cleanup", move || {
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .map_err(|error| format!("failed to remove staging directory: {error}"))?;
        }
        Ok(())
    })
    .await;
}

fn apply_optional_policy(
    policy: &OptionalDepsPolicy,
    dependencies: &mut [ResolvedDep],
    pending_choices: &mut Vec<PendingChoice>,
) {
    let mut prompt_options = Vec::new();
    for dependency in dependencies.iter_mut() {
        if dependency.requirement != Requirement::Optional {
            continue;
        }
        match policy {
            OptionalDepsPolicy::Include { deps } => {
                if !deps.iter().any(|id| id == &dependency.mod_jar_id)
                    && !matches!(
                        dependency.disposition,
                        DepDisposition::ReuseExisting { .. }
                            | DepDisposition::IncludedInBatch { .. }
                    )
                {
                    dependency.disposition = DepDisposition::Excluded;
                }
            }
            OptionalDepsPolicy::ExcludeAll => {
                if !matches!(
                    dependency.disposition,
                    DepDisposition::ReuseExisting { .. } | DepDisposition::IncludedInBatch { .. }
                ) {
                    dependency.disposition = DepDisposition::Excluded;
                }
            }
            OptionalDepsPolicy::Prompt => {
                if matches!(
                    dependency.disposition,
                    DepDisposition::InstallCandidate { .. } | DepDisposition::Unresolved { .. }
                ) {
                    prompt_options.push(OptionalDepOption {
                        mod_jar_id: dependency.mod_jar_id.clone(),
                        display_name: dependency
                            .display_name
                            .clone()
                            .unwrap_or_else(|| dependency.mod_jar_id.clone()),
                    });
                }
            }
        }
    }
    if !prompt_options.is_empty() {
        prompt_options.sort_by(|a, b| a.mod_jar_id.cmp(&b.mod_jar_id));
        pending_choices.push(PendingChoice::OptionalDependencies {
            choice_id: "optional-dependencies".into(),
            options: prompt_options,
        });
    }
}

#[cfg(test)]
thread_local! {
    static INSTALL_TEST_FAILPOINT: std::cell::RefCell<Option<&'static str>> = const { std::cell::RefCell::new(None) };
}

fn check_test_failpoint(name: &'static str) -> Result<(), String> {
    #[cfg(test)]
    {
        let should_fail = INSTALL_TEST_FAILPOINT.with(|slot| *slot.borrow() == Some(name));
        if should_fail {
            return Err(format!("injected install failpoint: {name}"));
        }
    }
    #[cfg(not(test))]
    let _ = name;
    Ok(())
}

fn apply_conflict_choices(
    intent: &InstallIntent,
    conflicts: &mut [DepConflict],
    pending_choices: &mut Vec<PendingChoice>,
    blocking_errors: &mut Vec<PlanError>,
) {
    for conflict in conflicts {
        if let Some(forced) = intent
            .overrides
            .force_conflict_resolution
            .get(&conflict.conflict_id)
        {
            if conflict.resolution_options.contains(forced) {
                conflict.chosen = Some(forced.clone());
            }
        } else if intent.overrides.allow_replace
            && conflict
                .resolution_options
                .contains(&ConflictResolution::Replace)
        {
            conflict.chosen = Some(ConflictResolution::Replace);
        }

        if conflict.chosen == Some(ConflictResolution::Abort) {
            blocking_errors.push(PlanError {
                code: "ERR_CONFLICT_ABORTED".into(),
                message: conflict.message.clone(),
            });
        } else if conflict.blocking && conflict.chosen.is_none() {
            pending_choices.push(PendingChoice::Conflict {
                choice_id: format!("conflict:{}", conflict.conflict_id),
                conflict_id: conflict.conflict_id.clone(),
                options: conflict
                    .resolution_options
                    .iter()
                    .map(conflict_resolution_option)
                    .collect(),
            });
        }
    }
}

fn conflict_resolution_option(resolution: &ConflictResolution) -> ConflictResolutionOption {
    let (label, description) = match resolution {
        ConflictResolution::Replace => ("Replace", "Replace the existing conflicting file."),
        ConflictResolution::Skip => ("Skip", "Keep the existing item and skip the incoming one."),
        ConflictResolution::DisableExisting => (
            "Disable existing",
            "Keep the existing item installed but disable it before adding the new one.",
        ),
        ConflictResolution::Abort => ("Abort", "Do not make any changes."),
    };
    ConflictResolutionOption {
        resolution: resolution.clone(),
        label: label.into(),
        description: description.into(),
    }
}

/// Work Package 8 — pre-commit loader-mismatch gate.
///
/// When a plan would add local-file artifacts whose loader requirements the
/// instance's current loader does not satisfy, surface a structured
/// `PendingChoice::LoaderChange` before any mod file is committed. Approval is
/// a backend override (`PlanOverrides::approve_loader_version` matching the
/// signed-catalog recommendation); the accepted switch is returned as a
/// `LoaderChangePlan` that `prepare_manifest` folds into the same atomic
/// manifest transaction as the file changes.
///
/// Only artifacts whose JARs exist at plan time are evaluated (manual /
/// local-file installs): their declarations are parsed exactly the way
/// `prepare_manifest` parses the staged JAR at apply time. Download-only
/// artifacts cannot contribute evidence before staging without downloading at
/// resolve time (a redesign), so they remain covered by the post-install
/// health scan.
#[allow(clippy::too_many_arguments)]
fn apply_loader_change_policy(
    manifest: &crate::models::InstanceManifest,
    intent: &InstallIntent,
    instance_dir: &Path,
    files_to_add: &[FileAdd],
    files_to_remove: &[FileRemove],
    files_to_disable: &[FileDisable],
    pending_choices: &mut Vec<PendingChoice>,
    warnings: &mut Vec<PlanWarning>,
) -> Option<LoaderChangePlan> {
    use crate::loader_compatibility::{
        evaluate_loader_compatibility, is_loader_requirement, CurrentLoaderStatus,
        LoaderCompatibilityRequest,
    };

    if matches!(manifest.loader.as_str(), "" | "vanilla") || manifest.loader_version.is_empty() {
        return None;
    }
    // Evaluate the prospective complete enabled set, not only the incoming
    // artifact. Existing requirements may conflict with the new mod, and
    // updates/removals must stop contributing after this plan commits.
    let removed: std::collections::BTreeSet<&str> = files_to_remove
        .iter()
        .map(|file| file.filename.as_str())
        .chain(files_to_disable.iter().map(|file| file.filename.as_str()))
        .collect();
    let mut decls = crate::health::inventory(instance_dir, manifest)
        .artifacts
        .into_iter()
        .filter(|artifact| !removed.contains(artifact.filename.as_str()))
        .flat_map(|artifact| artifact.metadata.dependency_decls)
        .collect::<Vec<_>>();
    for add in files_to_add {
        let source_path = match &add.artifact {
            ResolvedArtifact::LocalFile(local) => Some(local.source_path.as_str()),
            ResolvedArtifact::Download(download) => match &download.source {
                ArtifactSource::LocalFile { path } => Some(path.as_str()),
                ArtifactSource::Download { .. } => None,
            },
        };
        let Some(source_path) = source_path else {
            continue;
        };
        if !Path::new(source_path).is_file() {
            continue;
        }
        decls.extend(
            crate::jar_metadata::parse_jar_metadata_for_loader(
                Path::new(source_path),
                &manifest.loader,
            )
            .dependency_decls,
        );
    }
    if !decls
        .iter()
        .any(|decl| is_loader_requirement(decl).is_some())
    {
        return None;
    }
    let catalog = crate::loader_manifests::active_catalog();
    let report = evaluate_loader_compatibility(&LoaderCompatibilityRequest {
        loader: &manifest.loader,
        minecraft_version: &manifest.minecraft_version,
        current_loader_version: Some(&manifest.loader_version),
        requirements: &decls,
        catalog: &catalog,
    });
    let Some(recommended) = report.recommended_version.as_ref() else {
        if matches!(
            report.current_status,
            CurrentLoaderStatus::Incompatible | CurrentLoaderStatus::NoCompatibleCandidates
        ) {
            warnings.push(PlanWarning {
                code: "WARN_LOADER_INCOMPATIBLE".into(),
                message: format!(
                    "The installed {} loader {} does not satisfy the loader requirements of the added files, and no signed catalog version satisfies every requirement.",
                    manifest.loader, manifest.loader_version
                ),
            });
        }
        return None;
    };
    if report.current_status != CurrentLoaderStatus::Incompatible {
        return None;
    }
    if let Some(approved) = intent.overrides.approve_loader_version.as_deref() {
        // Accept any signed-catalog version that satisfies every hard
        // requirement — not just the recommendation — so the UI can offer the
        // full set of compatible versions and still commit the switch
        // atomically with the file changes.
        if report
            .compatible_versions
            .iter()
            .any(|candidate| candidate.loader_version == approved)
        {
            return Some(LoaderChangePlan {
                loader: manifest.loader.clone(),
                from_version: manifest.loader_version.clone(),
                to_version: approved.to_string(),
            });
        }
    }
    pending_choices.push(PendingChoice::LoaderChange {
        choice_id: "loader-change".into(),
        loader: manifest.loader.clone(),
        current_version: manifest.loader_version.clone(),
        recommended_version: recommended.loader_version.clone(),
        compatible_versions: report
            .compatible_versions
            .iter()
            .map(|candidate| candidate.loader_version.clone())
            .collect(),
        requirements: report
            .requirements
            .iter()
            .map(|result| crate::health::LoaderRequirementIssue {
                declaring_mod_id: result.declaration.declaring_mod_id.clone(),
                declaring_mod_ids: result.declaring_mod_ids.clone(),
                target_id: result.declaration.target_id.clone(),
                version_ranges: result.declaration.version_ranges.clone(),
                importance: result.declaration.importance,
                candidate_version: result.candidate_provided_version.clone(),
                verdict: result.verdict.clone(),
            })
            .collect(),
        conflicts: report.conflicts.clone(),
    });
    None
}

/// Recheck the complete prospective mod set after every artifact has been
/// downloaded and verified. Resolve-time planning cannot inspect a remote JAR
/// without downloading it, so this gate closes that gap before the first live
/// file or manifest mutation.
fn validate_staged_loader_compatibility(
    plan: &ResolvedInstallPlan,
    instance_dir: &Path,
    staging_dir: &Path,
) -> Result<(), String> {
    let manifest_path = instance_dir.join("instance_manifest.json");
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read manifest for loader validation: {error}"))?;
    let manifest: crate::models::InstanceManifest = serde_json::from_str(&manifest_text)
        .map_err(|error| format!("failed to parse manifest for loader validation: {error}"))?;
    if matches!(manifest.loader.as_str(), "" | "vanilla") || manifest.loader_version.is_empty() {
        return Ok(());
    }

    let removed: std::collections::BTreeSet<&str> = plan
        .files_to_remove
        .iter()
        .map(|file| file.filename.as_str())
        .chain(
            plan.files_to_disable
                .iter()
                .map(|file| file.filename.as_str()),
        )
        .collect();
    let mut decls = crate::health::inventory(instance_dir, &manifest)
        .artifacts
        .into_iter()
        .filter(|artifact| !removed.contains(artifact.filename.as_str()))
        .flat_map(|artifact| artifact.metadata.dependency_decls)
        .collect::<Vec<_>>();

    for add in &plan.files_to_add {
        if artifact_metadata(&add.artifact).content_type != "mod" {
            continue;
        }
        let staged = staging_dir.join("artifacts").join(&add.staging_filename);
        let metadata =
            crate::jar_metadata::parse_jar_metadata_for_loader(&staged, &manifest.loader);
        decls.extend(metadata.dependency_decls);
    }
    if !decls
        .iter()
        .any(|decl| crate::loader_compatibility::is_loader_requirement(decl).is_some())
    {
        return Ok(());
    }

    let catalog = crate::loader_manifests::active_catalog();
    let report = crate::loader_compatibility::evaluate_loader_compatibility(
        &crate::loader_compatibility::LoaderCompatibilityRequest {
            loader: &manifest.loader,
            minecraft_version: &manifest.minecraft_version,
            current_loader_version: Some(&manifest.loader_version),
            requirements: &decls,
            catalog: &catalog,
        },
    );
    match report.current_status {
        crate::loader_compatibility::CurrentLoaderStatus::Compatible => {
            if plan.loader_change.is_some() {
                return Err(
                    "The approved loader change is stale; the staged mod set already satisfies the current loader. Resolve a fresh plan.".into(),
                );
            }
            Ok(())
        }
        crate::loader_compatibility::CurrentLoaderStatus::Incompatible => {
            let Some(change) = &plan.loader_change else {
                return Err(
                    "The downloaded mod set introduces a loader mismatch. Resolve a fresh install plan to review the required loader change.".into(),
                );
            };
            let target_is_compatible = report
                .compatible_versions
                .iter()
                .any(|candidate| candidate.loader_version == change.to_version);
            if target_is_compatible {
                Ok(())
            } else {
                Err(
                    "The approved loader version does not satisfy the complete staged mod set. Resolve a fresh install plan.".into(),
                )
            }
        }
        crate::loader_compatibility::CurrentLoaderStatus::Indeterminate => Err(
            "The staged mod set contains loader requirements that cannot be verified safely. Review the loader manually before installing.".into(),
        ),
        crate::loader_compatibility::CurrentLoaderStatus::NoCompatibleCandidates => Err(
            "The staged mod set has no signed loader version satisfying all hard requirements.".into(),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn plan_artifact_change(
    artifact: &ResolvedArtifact,
    require_existing: bool,
    installed: &[&crate::models::InstalledMod],
    intent: &InstallIntent,
    files_to_add: &mut Vec<FileAdd>,
    files_to_remove: &mut Vec<FileRemove>,
    conflicts: &mut Vec<DepConflict>,
    pending_choices: &mut Vec<PendingChoice>,
    blocking_errors: &mut Vec<PlanError>,
    warnings: &mut Vec<PlanWarning>,
) {
    let filename = artifact_filename(artifact);
    if let Err(error) = validate_filename(filename) {
        blocking_errors.push(PlanError {
            code: "ERR_UNSAFE_FILENAME".into(),
            message: error,
        });
        return;
    }
    if let Err(error) = validate_artifact_hashes(artifact) {
        blocking_errors.push(PlanError {
            code: "ERR_HASH_UNAVAILABLE".into(),
            message: error,
        });
        return;
    }

    let same_identity = installed
        .iter()
        .find(|item| artifact_matches_installed(artifact, item));
    if require_existing && same_identity.is_none() {
        blocking_errors.push(PlanError {
            code: "ERR_UPDATE_TARGET_MISSING".into(),
            message: format!(
                "{} is not currently installed, so it cannot be updated.",
                artifact_item_id(artifact)
            ),
        });
        return;
    }

    if let Some(existing) = same_identity {
        let expected_sha256 = artifact_hash_value(artifact, HashAlgorithm::Sha256);
        let same_content = expected_sha256
            .as_deref()
            .map(|hash| hash.eq_ignore_ascii_case(&existing.sha256))
            .unwrap_or(false);
        let same_version = existing.version.as_deref() == Some(artifact_version_id(artifact));
        if same_content && same_version {
            warnings.push(PlanWarning {
                code: "ALREADY_INSTALLED".into(),
                message: format!(
                    "{} {} is already installed; no duplicate file will be added.",
                    artifact_item_id(artifact),
                    artifact_version_id(artifact)
                ),
            });
            return;
        }
        files_to_remove.push(FileRemove {
            filename: effective_installed_filename(existing),
            content_type: Some(existing.content_type.clone()),
        });
    }

    if let Some(collision) = installed.iter().find(|item| {
        effective_installed_filename(item) == filename
            && !artifact_matches_installed(artifact, item)
    }) {
        let conflict_id = format!("filename:{filename}");
        let chosen = intent
            .overrides
            .force_conflict_resolution
            .get(&conflict_id)
            .cloned()
            .or_else(|| {
                intent
                    .overrides
                    .allow_replace
                    .then_some(ConflictResolution::Replace)
            });
        let conflict = DepConflict {
            conflict_id: conflict_id.clone(),
            kind: ConflictKind::DuplicateMod,
            existing_mod_jar_id: installed_identity(collision)
                .unwrap_or_else(|| collision.filename.clone()),
            incoming_mod_jar_id: artifact_item_id(artifact).to_string(),
            message: format!(
                "{} is already used by a different installed item.",
                filename
            ),
            blocking: true,
            resolution_options: vec![ConflictResolution::Replace, ConflictResolution::Abort],
            chosen,
        };
        if conflict.chosen == Some(ConflictResolution::Replace) {
            files_to_remove.push(FileRemove {
                filename: effective_installed_filename(collision),
                content_type: Some(collision.content_type.clone()),
            });
        } else if conflict.chosen == Some(ConflictResolution::Abort) {
            blocking_errors.push(PlanError {
                code: "ERR_FILENAME_CONFLICT".into(),
                message: conflict.message.clone(),
            });
        } else {
            pending_choices.push(PendingChoice::Conflict {
                choice_id: format!("conflict:{conflict_id}"),
                conflict_id: conflict_id.clone(),
                options: conflict
                    .resolution_options
                    .iter()
                    .map(conflict_resolution_option)
                    .collect(),
            });
        }
        conflicts.push(conflict);
    }

    files_to_add.push(FileAdd {
        target_filename: filename.to_string(),
        staging_filename: filename.to_string(),
        artifact: artifact.clone(),
        hashes: artifact_hashes(artifact).clone(),
        size: artifact_size(artifact),
    });
}

fn all_installed(manifest: &crate::models::InstanceManifest) -> Vec<&crate::models::InstalledMod> {
    manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
        .chain(manifest.worlds.iter())
        .collect()
}

fn installed_identity(item: &crate::models::InstalledMod) -> Option<String> {
    item.registry_id
        .clone()
        .or_else(|| item.modrinth_id.clone())
        .or_else(|| item.mod_jar_id.clone())
}

fn artifact_matches_installed(
    artifact: &ResolvedArtifact,
    item: &crate::models::InstalledMod,
) -> bool {
    let metadata = artifact_metadata(artifact);
    metadata
        .registry_id
        .as_ref()
        .zip(item.registry_id.as_ref())
        .map(|(a, b)| a.eq_ignore_ascii_case(b))
        .unwrap_or(false)
        || metadata
            .modrinth_id
            .as_ref()
            .zip(item.modrinth_id.as_ref())
            .map(|(a, b)| a.eq_ignore_ascii_case(b))
            .unwrap_or(false)
        || item
            .mod_jar_id
            .as_deref()
            .map(|id| id.eq_ignore_ascii_case(artifact_item_id(artifact)))
            .unwrap_or(false)
}

fn effective_installed_filename(item: &crate::models::InstalledMod) -> String {
    if item.enabled || item.filename.ends_with(".disabled") {
        item.filename.clone()
    } else {
        format!("{}.disabled", item.filename)
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
        Err(format!("Artifact filename {filename:?} is unsafe."))
    } else {
        Ok(())
    }
}

fn validate_artifact_hashes(artifact: &ResolvedArtifact) -> Result<(), String> {
    let metadata = artifact_metadata(artifact);
    let hashes = &artifact_hashes(artifact).values;
    let valid = |algorithm: HashAlgorithm, length: usize| {
        hashes.iter().any(|value| {
            value.algorithm == algorithm
                && value.value.len() == length
                && value.value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    };
    let verified = match metadata.source_type {
        SourceType::Curated => {
            valid(HashAlgorithm::Sha512, 128) || valid(HashAlgorithm::Sha256, 64)
        }
        SourceType::Manual => valid(HashAlgorithm::Sha256, 64),
        SourceType::Modrinth => {
            valid(HashAlgorithm::Sha512, 128)
                || valid(HashAlgorithm::Sha256, 64)
                || valid(HashAlgorithm::Sha1, 40)
        }
    };
    if verified {
        Ok(())
    } else {
        Err(format!(
            "{} has no acceptable published hash for {:?} source.",
            artifact_item_id(artifact),
            metadata.source_type
        ))
    }
}

fn artifact_item_id(artifact: &ResolvedArtifact) -> &str {
    match artifact {
        ResolvedArtifact::Download(value) => &value.item_id,
        ResolvedArtifact::LocalFile(value) => &value.item_id,
    }
}

fn artifact_version_id(artifact: &ResolvedArtifact) -> &str {
    match artifact {
        ResolvedArtifact::Download(value) => &value.version_id,
        ResolvedArtifact::LocalFile(_) => "local",
    }
}

fn artifact_filename(artifact: &ResolvedArtifact) -> &str {
    match artifact {
        ResolvedArtifact::Download(value) => &value.filename,
        ResolvedArtifact::LocalFile(value) => &value.filename,
    }
}

fn artifact_hashes(artifact: &ResolvedArtifact) -> &HashSpec {
    match artifact {
        ResolvedArtifact::Download(value) => &value.hashes,
        ResolvedArtifact::LocalFile(value) => &value.hashes,
    }
}

fn artifact_size(artifact: &ResolvedArtifact) -> u64 {
    match artifact {
        ResolvedArtifact::Download(value) => value.size,
        ResolvedArtifact::LocalFile(value) => value.size,
    }
}

fn artifact_metadata(artifact: &ResolvedArtifact) -> &ArtifactMetadata {
    match artifact {
        ResolvedArtifact::Download(value) => &value.metadata,
        ResolvedArtifact::LocalFile(value) => &value.metadata,
    }
}

fn artifact_hash_value(artifact: &ResolvedArtifact, algorithm: HashAlgorithm) -> Option<String> {
    artifact_hashes(artifact)
        .values
        .iter()
        .find(|value| value.algorithm == algorithm)
        .map(|value| value.value.clone())
}

fn pending_choice_id(choice: &PendingChoice) -> &str {
    match choice {
        PendingChoice::OptionalDependencies { choice_id, .. }
        | PendingChoice::Conflict { choice_id, .. }
        | PendingChoice::LoaderChange { choice_id, .. } => choice_id,
    }
}

fn hash_serializable<T: Serialize>(value: &T) -> Result<String, String> {
    use sha2::Digest;
    let bytes =
        serde_json::to_vec(value).map_err(|e| format!("failed to serialize hash input: {e}"))?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanFingerprintMaterial<'a> {
    schema_version: u32,
    intent: &'a InstallIntent,
    operation: &'a ResolvedOperation,
    dependencies: &'a [ResolvedDep],
    conflicts: &'a [DepConflict],
    files_to_add: &'a [FileAdd],
    files_to_remove: &'a [FileRemove],
    files_to_disable: &'a [FileDisable],
    loader_change: &'a Option<LoaderChangePlan>,
    instance_state_hash: &'a str,
    registry_revision: &'a str,
}

fn compute_plan_fingerprint(plan: &ResolvedInstallPlan) -> Result<String, String> {
    hash_serializable(&PlanFingerprintMaterial {
        schema_version: 2,
        intent: &plan.intent,
        operation: &plan.operation,
        dependencies: &plan.dependencies,
        conflicts: &plan.conflicts,
        files_to_add: &plan.files_to_add,
        files_to_remove: &plan.files_to_remove,
        files_to_disable: &plan.files_to_disable,
        loader_change: &plan.loader_change,
        instance_state_hash: &plan.instance_state_hash,
        registry_revision: &plan.registry_revision,
    })
}

async fn stage_plan_artifacts(
    plan: &ResolvedInstallPlan,
    staging_dir: &Path,
    reporter: &dyn ProgressReporter,
    cancel: &CancellationToken,
) -> Result<(), String> {
    if staging_dir.exists() {
        std::fs::remove_dir_all(staging_dir)
            .map_err(|e| format!("failed to clear previous staging directory: {e}"))?;
    }
    let artifacts_dir = staging_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .map_err(|e| format!("failed to create staging directory: {e}"))?;

    let mut bytes_done = 0u64;
    for (index, file) in plan.files_to_add.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err("Install cancelled during staging.".into());
        }
        validate_filename(&file.staging_filename)?;
        let contents = match &file.artifact {
            ResolvedArtifact::Download(download) => match &download.source {
                ArtifactSource::Download { url } => {
                    let strategy = download
                        .metadata
                        .download_strategy
                        .as_deref()
                        .unwrap_or_default();
                    // Hand-pinned strategies are SHA-256-pinned in the signed
                    // registry, so the curator-reviewed host from that manifest
                    // row authorizes the request. `technic_pack` additionally
                    // permits plain HTTP: the hash is out-of-band, so transport
                    // is not the trust anchor.
                    if matches!(strategy, "direct_hash" | "technic_pack") {
                        let pinned_host =
                            download.metadata.pinned_host.as_deref().ok_or_else(|| {
                                format!(
                                    "{} uses {strategy} but carries no pinned host",
                                    download.item_id
                                )
                            })?;
                        crate::download::download_pinned_bytes_standalone(url, pinned_host)
                            .await
                            .map_err(|e| format!("failed to download {}: {e}", download.item_id))?
                    } else {
                        crate::download::download_mod_bytes_standalone(url)
                            .await
                            .map_err(|e| format!("failed to download {}: {e}", download.item_id))?
                    }
                }
                ArtifactSource::LocalFile { path } => std::fs::read(path).map_err(|e| {
                    format!("failed to read local artifact {}: {e}", download.item_id)
                })?,
            },
            ResolvedArtifact::LocalFile(local) => std::fs::read(&local.source_path)
                .map_err(|e| format!("failed to read local artifact {}: {e}", local.item_id))?,
        };
        if cancel.is_cancelled() {
            return Err("Install cancelled during staging.".into());
        }
        if file.size != 0 && contents.len() as u64 != file.size {
            return Err(format!(
                "artifact size mismatch for {}: expected {}, received {}",
                file.target_filename,
                file.size,
                contents.len()
            ));
        }
        verify_bytes(&contents, &file.hashes)
            .map_err(|e| format!("verification failed for {}: {e}", file.target_filename))?;

        let target = artifacts_dir.join(&file.staging_filename);
        let partial = artifacts_dir.join(format!("{}.part", file.staging_filename));
        check_test_failpoint("stage-write")?;
        let mut output = std::fs::File::create(&partial)
            .map_err(|e| format!("failed to create staged {}: {e}", file.target_filename))?;
        use std::io::Write;
        output
            .write_all(&contents)
            .map_err(|e| format!("failed to write staged {}: {e}", file.target_filename))?;
        output
            .sync_all()
            .map_err(|e| format!("failed to sync staged {}: {e}", file.target_filename))?;
        std::fs::rename(&partial, &target)
            .map_err(|e| format!("failed to finalize staged {}: {e}", file.target_filename))?;

        bytes_done = bytes_done.saturating_add(contents.len() as u64);
        reporter.report(ProgressEvent {
            plan_id: plan.fingerprint.clone(),
            phase: ProgressPhase::Staging,
            step: (index + 1) as u32,
            total_steps: plan.files_to_add.len() as u32,
            bytes_downloaded: bytes_done,
            bytes_total: plan.disk_estimate.download_bytes.max(bytes_done),
            message: format!("Verified {}", file.target_filename),
        });
    }
    Ok(())
}

fn verify_bytes(contents: &[u8], hashes: &HashSpec) -> Result<(), String> {
    use sha1::Digest as _;

    if hashes.values.is_empty() {
        return Err("no expected hashes were supplied".into());
    }
    for expected in &hashes.values {
        let actual = match expected.algorithm {
            HashAlgorithm::Sha256 => {
                let mut hasher = sha2::Sha256::new();
                hasher.update(contents);
                format!("{:x}", hasher.finalize())
            }
            HashAlgorithm::Sha512 => {
                let mut hasher = sha2::Sha512::new();
                hasher.update(contents);
                format!("{:x}", hasher.finalize())
            }
            HashAlgorithm::Sha1 => {
                let mut hasher = sha1::Sha1::new();
                hasher.update(contents);
                format!("{:x}", hasher.finalize())
            }
        };
        if !actual.eq_ignore_ascii_case(expected.value.trim()) {
            return Err(format!("{:?} hash mismatch", expected.algorithm));
        }
    }
    Ok(())
}

fn prepare_manifest(
    plan: &ResolvedInstallPlan,
    instance_dir: &Path,
    staging_dir: &Path,
) -> Result<(), String> {
    use std::io::Write;

    let manifest_path = instance_dir.join("instance_manifest.json");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("failed to read manifest before apply: {e}"))?;
    let mut manifest: crate::models::InstanceManifest = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse manifest before apply: {e}"))?;

    if let Some(change) = &plan.loader_change {
        if !manifest.loader.eq_ignore_ascii_case(&change.loader) {
            return Err(format!(
                "loader change targets {} but the instance uses {}",
                change.loader, manifest.loader
            ));
        }
        manifest.loader_version = change.to_version.clone();
    }

    for remove in &plan.files_to_remove {
        remove_manifest_entry(
            &mut manifest,
            &remove.filename,
            remove.content_type.as_deref(),
        );
    }
    for disable in &plan.files_to_disable {
        set_manifest_enabled(&mut manifest, &disable.filename, false);
    }
    for add in &plan.files_to_add {
        let staged = staging_dir.join("artifacts").join(&add.staging_filename);
        if !staged.is_file() {
            return Err(format!(
                "required staged artifact is missing: {}",
                add.staging_filename
            ));
        }
        let contents = std::fs::read(&staged)
            .map_err(|e| format!("failed to read staged {}: {e}", add.staging_filename))?;
        verify_bytes(&contents, &add.hashes)?;
        let metadata = artifact_metadata(&add.artifact);
        let jar = if metadata.content_type == "mod" {
            crate::jar_metadata::parse_jar_metadata_for_loader(&staged, &manifest.loader)
        } else {
            crate::dependency_ops::JarDeps::default()
        };
        let sha256 = crate::download::sha256_hex(&contents);
        let installed = crate::models::InstalledMod {
            // Individual install through the transaction pipeline. Pack-driven
            // installs stamp their own provenance; see PackOrigin.
            pack_managed: false,
            filename: add.target_filename.clone(),
            registry_id: metadata.registry_id.clone(),
            modrinth_id: metadata.modrinth_id.clone(),
            source: match metadata.source_type {
                SourceType::Curated => "registry",
                SourceType::Modrinth => "modrinth_raw",
                SourceType::Manual => "manual",
            }
            .into(),
            source_url: match &add.artifact {
                ResolvedArtifact::Download(download) => match &download.source {
                    ArtifactSource::Download { url } => Some(url.clone()),
                    ArtifactSource::LocalFile { .. } => None,
                },
                ResolvedArtifact::LocalFile(_) => None,
            },
            version: match &add.artifact {
                ResolvedArtifact::Download(download) => metadata
                    .version
                    .clone()
                    .or_else(|| Some(download.version_id.clone())),
                ResolvedArtifact::LocalFile(_) => None,
            },
            sha256,
            installed_at: chrono::Utc::now().to_rfc3339(),
            java_packages: jar.java_packages,
            mod_jar_id: jar.mod_jar_id.or_else(|| {
                metadata
                    .registry_id
                    .clone()
                    .or_else(|| metadata.modrinth_id.clone())
            }),
            enabled: true,
            content_type: normalized_content_type(&metadata.content_type).into(),
            depends_on: jar.depends_on,
            optional_deps: jar.optional_deps,
            incompatible_deps: jar.incompatible_deps,
            provided_mod_ids: jar
                .provided_mods
                .iter()
                .map(|pm| pm.mod_id.clone())
                .collect(),
        };
        remove_manifest_identity(&mut manifest, &installed);
        content_entries_mut(&mut manifest, &installed.content_type).push(installed);
    }

    check_test_failpoint("manifest-serialize")?;
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| format!("failed to serialize future manifest: {e}"))?;
    let path = staging_dir.join("instance_manifest.next.json");
    let mut file = std::fs::File::create(&path)
        .map_err(|e| format!("failed to create future manifest: {e}"))?;
    file.write_all(&bytes)
        .map_err(|e| format!("failed to write future manifest: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("failed to sync future manifest: {e}"))?;
    Ok(())
}

#[derive(Default)]
struct ApplyJournal {
    removed: Vec<(std::path::PathBuf, std::path::PathBuf)>,
    disabled: Vec<(std::path::PathBuf, std::path::PathBuf)>,
    added: Vec<(std::path::PathBuf, std::path::PathBuf)>,
}

fn apply_transaction(
    plan: &ResolvedInstallPlan,
    instance_dir: &Path,
    staging_dir: &Path,
) -> Result<(), String> {
    let manifest_path = instance_dir.join("instance_manifest.json");
    let original_text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("failed to read current manifest during apply: {e}"))?;
    let manifest: crate::models::InstanceManifest = serde_json::from_str(&original_text)
        .map_err(|e| format!("failed to parse current manifest during apply: {e}"))?;
    let manifest_backup = staging_dir.join("instance_manifest.original.json");
    std::fs::write(&manifest_backup, original_text.as_bytes())
        .map_err(|e| format!("failed to back up current manifest: {e}"))?;

    let mut journal = ApplyJournal::default();
    let result = (|| -> Result<(), String> {
        let trash_dir = staging_dir.join("trash");
        for remove in &plan.files_to_remove {
            let live = locate_live_file(
                instance_dir,
                &manifest,
                &remove.filename,
                remove.content_type.as_deref(),
            );
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
                .map_err(|e| format!("failed to stage removal of {}: {e}", remove.filename))?;
            journal.removed.push((backup, live));
        }

        for disable in &plan.files_to_disable {
            let live = locate_live_file(instance_dir, &manifest, &disable.filename, None);
            if !live.exists() {
                return Err(format!(
                    "file selected for disable is missing: {}",
                    disable.filename
                ));
            }
            let disabled = live.with_file_name(format!(
                "{}.disabled",
                live.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&disable.filename)
            ));
            if disabled.exists() {
                return Err(format!(
                    "disabled destination already exists: {}",
                    disabled.display()
                ));
            }
            std::fs::rename(&live, &disabled)
                .map_err(|e| format!("failed to disable {}: {e}", disable.filename))?;
            journal.disabled.push((disabled, live));
        }

        for add in &plan.files_to_add {
            let staged = staging_dir.join("artifacts").join(&add.staging_filename);
            if !staged.is_file() {
                return Err(format!(
                    "required staged artifact vanished: {}",
                    add.staging_filename
                ));
            }
            let subdir = content_subdir(&artifact_metadata(&add.artifact).content_type);
            let live = instance_dir.join(subdir).join(&add.target_filename);
            if live.exists() {
                return Err(format!("target file already exists: {}", live.display()));
            }
            if let Some(parent) = live.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create content directory: {e}"))?;
            }
            check_test_failpoint("artifact-rename")?;
            std::fs::rename(&staged, &live)
                .map_err(|e| format!("failed to promote {}: {e}", add.target_filename))?;
            journal.added.push((live, staged));
        }

        let prepared = staging_dir.join("instance_manifest.next.json");
        if !prepared.is_file() {
            return Err("prepared manifest vanished before commit".into());
        }
        let temporary = instance_dir.join(format!(
            "instance_manifest.json.tmp.{}",
            &plan.fingerprint[..16]
        ));
        check_test_failpoint("manifest-rename")?;
        std::fs::rename(&prepared, &temporary)
            .map_err(|e| format!("failed to move prepared manifest to commit location: {e}"))?;
        std::fs::rename(&temporary, &manifest_path)
            .map_err(|e| format!("failed to commit instance manifest: {e}"))?;
        Ok(())
    })();

    if let Err(error) = result {
        let rollback = rollback_apply(&journal, &manifest_path, &manifest_backup);
        return match rollback {
            Ok(()) => Err(format!("{error}; pre-commit file moves were reversed")),
            Err(rollback_error) => Err(format!("{error}; fast rollback failed: {rollback_error}")),
        };
    }
    // Tracked content changed; the next launch must not reuse a pre-install
    // recovery snapshot from before the apply.
    let _ = crate::snapshot::mark_instance_mutated(instance_dir);
    Ok(())
}

fn rollback_apply(
    journal: &ApplyJournal,
    manifest_path: &Path,
    manifest_backup: &Path,
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
    for (disabled, original) in journal.disabled.iter().rev() {
        if disabled.exists() && !original.exists() {
            if let Err(error) = std::fs::rename(disabled, original) {
                errors.push(format!("disabled {}: {error}", original.display()));
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
    if !manifest_path.is_file() && manifest_backup.is_file() {
        if let Err(error) = std::fs::copy(manifest_backup, manifest_path) {
            errors.push(format!("manifest: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn locate_live_file(
    instance_dir: &Path,
    manifest: &crate::models::InstanceManifest,
    filename: &str,
    content_type: Option<&str>,
) -> std::path::PathBuf {
    for item in all_installed(manifest) {
        if (item.filename == filename || effective_installed_filename(item) == filename)
            && content_type.is_none_or(|ct| {
                normalized_content_type(&item.content_type) == normalized_content_type(ct)
            })
        {
            return instance_dir
                .join(content_subdir(&item.content_type))
                .join(effective_installed_filename(item));
        }
    }
    instance_dir.join("mods").join(filename)
}

fn remove_manifest_entry(
    manifest: &mut crate::models::InstanceManifest,
    filename: &str,
    content_type: Option<&str>,
) {
    let ct_normalized = content_type.map(normalized_content_type);
    let remove = |entries: &mut Vec<crate::models::InstalledMod>| {
        entries.retain(|item| {
            item.filename != filename && effective_installed_filename(item) != filename
        });
    };
    match ct_normalized {
        Some("resourcepack") => remove(&mut manifest.resourcepacks),
        Some("shader") => remove(&mut manifest.shaders),
        Some("datapack") => remove(&mut manifest.datapacks),
        Some("world") => remove(&mut manifest.worlds),
        _ => {
            // None or "mod" — search all collections (legacy behaviour).
            remove(&mut manifest.mods);
            remove(&mut manifest.resourcepacks);
            remove(&mut manifest.shaders);
            remove(&mut manifest.datapacks);
            remove(&mut manifest.worlds);
        }
    }
}

fn set_manifest_enabled(
    manifest: &mut crate::models::InstanceManifest,
    filename: &str,
    enabled: bool,
) {
    for entries in [
        &mut manifest.mods,
        &mut manifest.resourcepacks,
        &mut manifest.shaders,
        &mut manifest.datapacks,
        &mut manifest.worlds,
    ] {
        if let Some(item) = entries.iter_mut().find(|item| {
            item.filename == filename || effective_installed_filename(item) == filename
        }) {
            item.enabled = enabled;
        }
    }
}

fn remove_manifest_identity(
    manifest: &mut crate::models::InstanceManifest,
    incoming: &crate::models::InstalledMod,
) {
    let identity = installed_identity(incoming);
    for entries in [
        &mut manifest.mods,
        &mut manifest.resourcepacks,
        &mut manifest.shaders,
        &mut manifest.datapacks,
        &mut manifest.worlds,
    ] {
        entries.retain(|item| {
            item.filename != incoming.filename
                && identity
                    .as_ref()
                    .zip(installed_identity(item).as_ref())
                    .map(|(a, b)| !a.eq_ignore_ascii_case(b))
                    .unwrap_or(true)
        });
    }
}

fn normalized_content_type(content_type: &str) -> &str {
    match content_type {
        "resourcepack" | "resourcepacks" => "resourcepack",
        "shader" | "shaderpack" | "shaderpacks" => "shader",
        "datapack" | "datapacks" => "datapack",
        "world" | "worlds" => "world",
        _ => "mod",
    }
}

fn content_subdir(content_type: &str) -> &str {
    match normalized_content_type(content_type) {
        "resourcepack" => "resourcepacks",
        "shader" => "shaderpacks",
        "datapack" => "datapacks",
        "world" => "saves",
        _ => "mods",
    }
}

fn content_entries_mut<'a>(
    manifest: &'a mut crate::models::InstanceManifest,
    content_type: &str,
) -> &'a mut Vec<crate::models::InstalledMod> {
    match normalized_content_type(content_type) {
        "resourcepack" => &mut manifest.resourcepacks,
        "shader" => &mut manifest.shaders,
        "datapack" => &mut manifest.datapacks,
        "world" => &mut manifest.worlds,
        _ => &mut manifest.mods,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Minecraft version of the instance built by `make_instance`.
    /// Shared so the catalog lookup and the instance manifest cannot drift.
    const TEST_INSTANCE_MC_VERSION: &str = "1.21.1";

    #[test]
    fn test_intent_install_round_trip() {
        let intent = InstallIntent {
            action: InstallAction::Install {
                source_type: SourceType::Curated,
                item_id: "test-mod".into(),
                candidate_version: Some("1.0.0".into()),
            },
            target_instance: "test-instance".into(),
            optional_deps: OptionalDepsPolicy::ExcludeAll,
            requested_by: RequestSource::Interactive,
            overrides: PlanOverrides::default(),
        };
        let json = serde_json::to_string(&intent).unwrap();
        let restored: InstallIntent = serde_json::from_str(&json).unwrap();
        match restored.action {
            InstallAction::Install {
                ref item_id,
                ref candidate_version,
                ..
            } => {
                assert_eq!(item_id, "test-mod");
                assert_eq!(candidate_version, &Some("1.0.0".into()));
            }
            _ => panic!("wrong action variant"),
        }
    }

    #[test]
    fn test_intent_remove_round_trip() {
        let intent = InstallIntent {
            action: InstallAction::Remove {
                filename: "old-mod.jar".into(),
            },
            target_instance: "test-instance".into(),
            optional_deps: OptionalDepsPolicy::ExcludeAll,
            requested_by: RequestSource::Interactive,
            overrides: PlanOverrides::default(),
        };
        let json = serde_json::to_string(&intent).unwrap();
        let restored: InstallIntent = serde_json::from_str(&json).unwrap();
        match restored.action {
            InstallAction::Remove { ref filename } => {
                assert_eq!(filename, "old-mod.jar");
            }
            _ => panic!("wrong action variant"),
        }
    }

    #[test]
    fn test_typescript_protocol_json_shapes() {
        let intent = InstallIntent {
            action: InstallAction::Install {
                source_type: SourceType::Curated,
                item_id: "fabric-api".into(),
                candidate_version: Some("0.100.0".into()),
            },
            target_instance: "survival".into(),
            optional_deps: OptionalDepsPolicy::Include {
                deps: vec!["mod-menu".into()],
            },
            requested_by: RequestSource::AutoUpdate,
            overrides: PlanOverrides::default(),
        };
        assert_eq!(
            serde_json::to_value(&intent).unwrap(),
            serde_json::json!({
                "action": {
                    "type": "install",
                    "sourceType": "curated",
                    "itemId": "fabric-api",
                    "candidateVersion": "0.100.0"
                },
                "targetInstance": "survival",
                "optionalDeps": { "type": "include", "deps": ["mod-menu"] },
                "requestedBy": "auto-update",
                "overrides": {
                    "allowReplace": false,
                    "skipHealthScan": false,
                    "forceConflictResolution": {}
                }
            })
        );

        let artifact = ResolvedArtifact::Download(ResolvedDownload {
            item_id: "fabric-api".into(),
            version_id: "0.100.0".into(),
            source: ArtifactSource::Download {
                url: "https://cdn.modrinth.com/file.jar".into(),
            },
            hashes: HashSpec {
                values: vec![HashedValue {
                    algorithm: HashAlgorithm::Sha256,
                    value: "abc".into(),
                }],
            },
            size: 42,
            filename: "fabric-api.jar".into(),
            metadata: ArtifactMetadata {
                source_type: SourceType::Curated,
                registry_id: Some("fabric-api".into()),
                modrinth_id: Some("P7dR8mSH".into()),
                content_type: "mod".into(),
                version: None,
                download_strategy: None,
                pinned_host: None,
            },
        });
        assert_eq!(
            serde_json::to_value(DepDisposition::InstallCandidate {
                artifact: Box::new(artifact.clone()),
            })
            .unwrap(),
            serde_json::json!({
                "type": "install-candidate",
                "artifact": {
                    "type": "download",
                    "itemId": "fabric-api",
                    "versionId": "0.100.0",
                    "source": {
                        "type": "download",
                        "url": "https://cdn.modrinth.com/file.jar"
                    },
                    "hashes": { "values": [{ "algorithm": "sha256", "value": "abc" }] },
                    "size": 42,
                    "filename": "fabric-api.jar",
                    "metadata": {
                        "sourceType": "curated",
                        "registryId": "fabric-api",
                        "modrinthId": "P7dR8mSH",
                        "contentType": "mod"
                    }
                }
            })
        );

        let health = HealthOutcome::Completed {
            report: crate::health::HealthReport {
                score: crate::health::HealthScore::Green,
                warnings: vec![],
                blockers: vec![],
                recommendations: vec![],
                scan_token: "test-scan".into(),
            },
        };
        let value = serde_json::to_value(health).unwrap();
        assert_eq!(value["type"], "completed");
        assert!(value.get("report").is_some());
    }

    #[test]
    fn test_is_fully_resolved_blocks_with_errors() {
        let plan = test_plan();
        // The fixture has a blocking error, so it is not executable.
        assert!(!plan.is_fully_resolved());
    }

    #[test]
    fn test_is_fully_resolved_ok() {
        let mut plan = test_plan();
        plan.blocking_errors.clear();
        assert!(plan.is_fully_resolved());

        // pending choices blocks
        plan.pending_choices.push(PendingChoice::Conflict {
            choice_id: "c1".into(),
            conflict_id: "conflict-1".into(),
            options: vec![ConflictResolutionOption {
                resolution: ConflictResolution::Skip,
                label: "Skip".into(),
                description: "Skip this mod".into(),
            }],
        });
        assert!(!plan.is_fully_resolved());
    }

    #[test]
    fn test_is_fully_resolved_blocks_with_unresolved_conflict() {
        let mut plan = test_plan();
        plan.blocking_errors.clear();
        plan.conflicts.push(DepConflict {
            conflict_id: "c1".into(),
            kind: ConflictKind::VersionConflict,
            existing_mod_jar_id: "a-1.0".into(),
            incoming_mod_jar_id: "a-2.0".into(),
            message: "Version conflict".into(),
            blocking: true,
            resolution_options: vec![ConflictResolution::Replace, ConflictResolution::Skip],
            chosen: None,
        });
        assert!(!plan.is_fully_resolved());

        // Set chosen → resolved
        plan.conflicts[0].chosen = Some(ConflictResolution::Replace);
        assert!(plan.is_fully_resolved());
    }

    #[test]
    fn test_cancellation_token() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_resolve_plan_builds_deterministic_read_only_plan() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pipeline = InstallPipeline;
        let intent = InstallIntent {
            action: InstallAction::Install {
                source_type: SourceType::Curated,
                item_id: "fabric-api".into(),
                candidate_version: Some("1.0".into()),
            },
            target_instance: "test".into(),
            optional_deps: OptionalDepsPolicy::ExcludeAll,
            requested_by: RequestSource::Interactive,
            overrides: PlanOverrides::default(),
        };
        let prepared = PreparedPlan {
            operation: ResolvedOperation::Install {
                artifact: test_artifact(
                    "fabric-api",
                    "fabric-api.jar",
                    "a".repeat(64),
                    ArtifactSource::Download {
                        url: "https://cdn.modrinth.com/data/file.jar".into(),
                    },
                    SourceType::Curated,
                ),
            },
            dependencies: vec![],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        let before = crate::snapshot::live_file_index(&instance_dir).unwrap();
        let first = rt.block_on(async {
            let reporter = NoopReporter;
            pipeline
                .resolve_plan(intent.clone(), &instance_dir, prepared.clone(), &reporter)
                .await
                .unwrap()
        });
        let second = rt.block_on(async {
            let reporter = NoopReporter;
            pipeline
                .resolve_plan(intent, &instance_dir, prepared, &reporter)
                .await
                .unwrap()
        });

        assert!(first.is_fully_resolved());
        assert_eq!(first.files_to_add.len(), 1);
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(
            crate::snapshot::live_file_index(&instance_dir).unwrap(),
            before
        );
        assert!(!instance_dir.join(".agora").exists());
    }

    #[test]
    fn test_resolve_plan_exposes_required_and_optional_dependencies() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let required_path = tmp.path().join("required.jar");
        let optional_path = tmp.path().join("optional.jar");
        std::fs::write(&required_path, b"required").unwrap();
        std::fs::write(&optional_path, b"optional").unwrap();
        let prepared = PreparedPlan {
            operation: ResolvedOperation::Install {
                artifact: test_artifact(
                    "root",
                    "root.jar",
                    "a".repeat(64),
                    ArtifactSource::Download {
                        url: "https://example.com/root.jar".into(),
                    },
                    SourceType::Curated,
                ),
            },
            dependencies: vec![
                ResolvedDep {
                    mod_jar_id: "required-dep".into(),
                    requirement: Requirement::Required,
                    source: DepSource::Manifest,
                    display_name: None,
                    page_url: None,
                    disposition: DepDisposition::InstallCandidate {
                        artifact: Box::new(test_artifact(
                            "required-dep",
                            "required.jar",
                            crate::download::sha256_hex(b"required"),
                            ArtifactSource::LocalFile {
                                path: required_path.to_string_lossy().into_owned(),
                            },
                            SourceType::Curated,
                        )),
                    },
                },
                ResolvedDep {
                    mod_jar_id: "optional-dep".into(),
                    requirement: Requirement::Optional,
                    source: DepSource::Manifest,
                    display_name: None,
                    page_url: None,
                    disposition: DepDisposition::InstallCandidate {
                        artifact: Box::new(test_artifact(
                            "optional-dep",
                            "optional.jar",
                            crate::download::sha256_hex(b"optional"),
                            ArtifactSource::LocalFile {
                                path: optional_path.to_string_lossy().into_owned(),
                            },
                            SourceType::Curated,
                        )),
                    },
                },
            ],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        let mut intent = local_intent("root");
        intent.optional_deps = OptionalDepsPolicy::Prompt;
        let plan = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(InstallPipeline.resolve_plan(intent, &instance_dir, prepared, &NoopReporter))
            .unwrap();

        assert_eq!(plan.dependencies.len(), 2);
        assert_eq!(plan.dependencies[0].mod_jar_id, "optional-dep");
        assert_eq!(plan.dependencies[0].requirement, Requirement::Optional);
        assert_eq!(plan.dependencies[1].mod_jar_id, "required-dep");
        assert_eq!(plan.dependencies[1].requirement, Requirement::Required);
        assert!(matches!(
            plan.pending_choices.as_slice(),
            [PendingChoice::OptionalDependencies { options, .. }] if options.len() == 1
        ));
    }

    #[test]
    fn test_resolve_plan_reuses_installed_dependency_without_duplicate_file_action() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let prepared = PreparedPlan {
            operation: ResolvedOperation::Install {
                artifact: test_artifact(
                    "root",
                    "root.jar",
                    "a".repeat(64),
                    ArtifactSource::Download {
                        url: "https://example.com/root.jar".into(),
                    },
                    SourceType::Curated,
                ),
            },
            dependencies: vec![ResolvedDep {
                mod_jar_id: "fabric-api".into(),
                requirement: Requirement::Required,
                source: DepSource::Manifest,
                display_name: None,
                page_url: None,
                disposition: DepDisposition::ReuseExisting {
                    mod_jar_id: "fabric-api".into(),
                    installed_filename: "fabric-api.jar".into(),
                },
            }],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        let plan = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(InstallPipeline.resolve_plan(
                local_intent("root"),
                &instance_dir,
                prepared,
                &NoopReporter,
            ))
            .unwrap();

        assert!(matches!(
            plan.dependencies[0].disposition,
            DepDisposition::ReuseExisting { .. }
        ));
        assert_eq!(
            plan.files_to_add.len(),
            1,
            "only the requested item is added"
        );
    }

    #[test]
    fn test_resolve_plan_surfaces_conflict_before_execution() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let conflict = DepConflict {
            conflict_id: "known:root:other".into(),
            kind: ConflictKind::IncompatibleMod,
            existing_mod_jar_id: "other".into(),
            incoming_mod_jar_id: "root".into(),
            message: "These mods are incompatible.".into(),
            blocking: true,
            resolution_options: vec![ConflictResolution::Abort, ConflictResolution::Skip],
            chosen: None,
        };
        let prepared = PreparedPlan {
            operation: ResolvedOperation::Install {
                artifact: test_artifact(
                    "root",
                    "root.jar",
                    "a".repeat(64),
                    ArtifactSource::Download {
                        url: "https://example.com/root.jar".into(),
                    },
                    SourceType::Curated,
                ),
            },
            dependencies: vec![],
            conflicts: vec![conflict],
            registry_revision: "registry-rev".into(),
        };
        let plan = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(InstallPipeline.resolve_plan(
                local_intent("root"),
                &instance_dir,
                prepared,
                &NoopReporter,
            ))
            .unwrap();

        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].message, "These mods are incompatible.");
        assert!(plan
            .pending_choices
            .iter()
            .any(|choice| matches!(choice, PendingChoice::Conflict { conflict_id, .. } if conflict_id == "known:root:other")));
        assert!(!plan.is_fully_resolved());
    }

    #[test]
    fn test_batch_dependency_included_in_batch_adds_no_duplicate_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let prepared = PreparedPlan {
            operation: ResolvedOperation::BatchInstall {
                operations: vec![ResolvedOperation::Install {
                    artifact: test_artifact(
                        "terrablender",
                        "TerraBlender-fabric-3.3.0.10.jar",
                        "a".repeat(64),
                        ArtifactSource::Download {
                            url: "https://example.com/terrablender.jar".into(),
                        },
                        SourceType::Modrinth,
                    ),
                }],
            },
            dependencies: vec![ResolvedDep {
                mod_jar_id: "terrablender".into(),
                requirement: Requirement::Required,
                source: DepSource::Manifest,
                display_name: Some("TerraBlender".into()),
                page_url: Some("https://modrinth.com/mod/terrablender".into()),
                disposition: DepDisposition::IncludedInBatch {
                    target_filename: "TerraBlender-fabric-3.3.0.10.jar".into(),
                },
            }],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        let plan = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(InstallPipeline.resolve_plan(
                local_intent("root"),
                &instance_dir,
                prepared,
                &NoopReporter,
            ))
            .unwrap();

        // The dependency is satisfied by the batch itself: no duplicate file,
        // no blocking error, and the display metadata survives into the plan.
        assert_eq!(
            plan.files_to_add.len(),
            1,
            "batch root artifact is the only file to add"
        );
        assert!(plan.blocking_errors.is_empty());
        assert!(plan.is_fully_resolved());
        assert_eq!(
            plan.dependencies[0].display_name.as_deref(),
            Some("TerraBlender")
        );
        assert!(matches!(
            plan.dependencies[0].disposition,
            DepDisposition::IncludedInBatch { .. }
        ));
    }

    #[test]
    fn test_resolve_plan_unresolved_required_dependency_is_blocking() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let prepared = PreparedPlan {
            operation: ResolvedOperation::Install {
                artifact: test_artifact(
                    "root",
                    "root.jar",
                    "a".repeat(64),
                    ArtifactSource::Download {
                        url: "https://example.com/root.jar".into(),
                    },
                    SourceType::Curated,
                ),
            },
            dependencies: vec![ResolvedDep {
                mod_jar_id: "missing-required".into(),
                requirement: Requirement::Required,
                source: DepSource::Manifest,
                display_name: Some("TerraBlender".into()),
                page_url: None,
                disposition: DepDisposition::Unresolved {
                    reason: "no compatible artifact".into(),
                },
            }],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        let plan = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(InstallPipeline.resolve_plan(
                local_intent("root"),
                &instance_dir,
                prepared,
                &NoopReporter,
            ))
            .unwrap();

        assert!(plan
            .blocking_errors
            .iter()
            .any(|error| error.code == "ERR_REQUIRED_DEPENDENCY"));
        assert!(plan.blocking_errors.iter().any(|error| {
            error
                .message
                .contains("Required dependency TerraBlender could not be resolved")
        }));
        assert!(!plan.is_fully_resolved());
    }

    #[test]
    fn test_curated_and_modrinth_artifacts_normalize_to_the_same_plan_shape() {
        fn object_keys(value: &serde_json::Value) -> Vec<String> {
            let mut keys = value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            keys.sort();
            keys
        }

        let tmp = tempfile::TempDir::new().unwrap();
        let curated_dir = make_instance(&tmp);
        let modrinth_dir = tmp.path().join("modrinth-instance");
        std::fs::create_dir_all(modrinth_dir.join("mods")).unwrap();
        std::fs::copy(
            curated_dir.join("instance_manifest.json"),
            modrinth_dir.join("instance_manifest.json"),
        )
        .unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let resolve = |instance_dir: &Path, source_type: SourceType| {
            let item_id = match &source_type {
                SourceType::Curated => "curated-item",
                SourceType::Modrinth => "modrinth-item",
                SourceType::Manual => unreachable!(),
            };
            let intent = InstallIntent {
                action: InstallAction::Install {
                    source_type: source_type.clone(),
                    item_id: item_id.into(),
                    candidate_version: Some("1.0".into()),
                },
                target_instance: "test".into(),
                optional_deps: OptionalDepsPolicy::ExcludeAll,
                requested_by: RequestSource::Interactive,
                overrides: PlanOverrides::default(),
            };
            let prepared = PreparedPlan {
                operation: ResolvedOperation::Install {
                    artifact: test_artifact(
                        item_id,
                        &format!("{item_id}.jar"),
                        "a".repeat(64),
                        ArtifactSource::Download {
                            url: format!("https://example.com/{item_id}.jar"),
                        },
                        source_type,
                    ),
                },
                dependencies: vec![],
                conflicts: vec![],
                registry_revision: "registry-rev".into(),
            };
            runtime
                .block_on(InstallPipeline.resolve_plan(
                    intent,
                    instance_dir,
                    prepared,
                    &NoopReporter,
                ))
                .unwrap()
        };
        let curated = resolve(&curated_dir, SourceType::Curated);
        let modrinth = resolve(&modrinth_dir, SourceType::Modrinth);
        let curated_json = serde_json::to_value(curated).unwrap();
        let modrinth_json = serde_json::to_value(modrinth).unwrap();

        assert_eq!(object_keys(&curated_json), object_keys(&modrinth_json));
        assert_eq!(
            object_keys(&curated_json["operation"]),
            object_keys(&modrinth_json["operation"])
        );
        assert_eq!(
            object_keys(&curated_json["filesToAdd"][0]),
            object_keys(&modrinth_json["filesToAdd"][0])
        );
    }

    #[test]
    fn test_execute_local_artifact_commits_manifest_and_snapshot() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("source.jar");
        std::fs::write(&source_path, b"verified local artifact").unwrap();
        let plan = resolve_local_plan(&instance_dir, &source_path, "manual.jar", None);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let outcome = rt.block_on(InstallPipeline.execute_plan(
            &plan,
            &instance_dir,
            "registry-rev",
            &NoopReporter,
            &CancellationToken::new(),
        ));
        assert!(matches!(outcome, InstallOutcome::Success { .. }));
        assert_eq!(
            std::fs::read(instance_dir.join("mods").join("manual.jar")).unwrap(),
            b"verified local artifact"
        );
        let manifest: crate::models::InstanceManifest = serde_json::from_slice(
            &std::fs::read(instance_dir.join("instance_manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest.mods.len(), 1);
        assert_eq!(manifest.mods[0].filename, "manual.jar");
        assert_eq!(
            crate::snapshot::list_snapshots(&instance_dir)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn test_execute_rejects_stale_instance_without_snapshot_or_mutation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("source.jar");
        std::fs::write(&source_path, b"verified local artifact").unwrap();
        let plan = resolve_local_plan(&instance_dir, &source_path, "manual.jar", None);
        std::fs::write(instance_dir.join("mods").join("player-change.jar"), b"mine").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let outcome = rt.block_on(InstallPipeline.execute_plan(
            &plan,
            &instance_dir,
            "registry-rev",
            &NoopReporter,
            &CancellationToken::new(),
        ));
        assert!(matches!(outcome, InstallOutcome::Failed { .. }));
        assert!(!instance_dir.join("mods").join("manual.jar").exists());
        assert!(crate::snapshot::list_snapshots(&instance_dir)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_execute_hash_failure_leaves_live_state_unchanged() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("source.jar");
        std::fs::write(&source_path, b"actual bytes").unwrap();
        let plan = resolve_local_plan(
            &instance_dir,
            &source_path,
            "manual.jar",
            Some("b".repeat(64)),
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let outcome = rt.block_on(InstallPipeline.execute_plan(
            &plan,
            &instance_dir,
            "registry-rev",
            &NoopReporter,
            &CancellationToken::new(),
        ));
        assert!(matches!(
            outcome,
            InstallOutcome::Failed {
                snapshot_id: None,
                ..
            }
        ));
        assert!(!instance_dir.join("mods").join("manual.jar").exists());
        assert!(crate::snapshot::list_snapshots(&instance_dir)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_execute_cancellation_during_staging_is_non_mutating() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("source.jar");
        std::fs::write(&source_path, b"verified local artifact").unwrap();
        let plan = resolve_local_plan(&instance_dir, &source_path, "manual.jar", None);
        let token = CancellationToken::new();
        token.cancel();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let outcome = rt.block_on(InstallPipeline.execute_plan(
            &plan,
            &instance_dir,
            "registry-rev",
            &NoopReporter,
            &token,
        ));
        assert!(matches!(outcome, InstallOutcome::Cancelled { .. }));
        assert!(!instance_dir.join("mods").join("manual.jar").exists());
    }

    #[test]
    fn test_execute_rejects_tampered_plan_body() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("source.jar");
        std::fs::write(&source_path, b"verified local artifact").unwrap();
        let mut plan = resolve_local_plan(&instance_dir, &source_path, "manual.jar", None);
        plan.files_to_add[0].target_filename = "tampered.jar".into();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let outcome = rt.block_on(InstallPipeline.execute_plan(
            &plan,
            &instance_dir,
            "registry-rev",
            &NoopReporter,
            &CancellationToken::new(),
        ));
        assert!(matches!(outcome, InstallOutcome::Failed { .. }));
        assert!(!instance_dir.join("mods").join("tampered.jar").exists());
    }

    #[test]
    fn test_second_dependency_stage_failure_cleans_first_and_keeps_live_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("primary.jar");
        std::fs::write(&source_path, b"primary bytes").unwrap();
        let primary_hash = crate::download::sha256_hex(b"primary bytes");
        let missing_hash = crate::download::sha256_hex(b"missing bytes");
        let intent = local_intent("primary");
        let prepared = PreparedPlan {
            operation: ResolvedOperation::Install {
                artifact: test_artifact(
                    "primary",
                    "a-primary.jar",
                    primary_hash,
                    ArtifactSource::LocalFile {
                        path: source_path.to_string_lossy().into_owned(),
                    },
                    SourceType::Manual,
                ),
            },
            dependencies: vec![ResolvedDep {
                mod_jar_id: "missing-dep".into(),
                requirement: Requirement::Required,
                source: DepSource::Manifest,
                display_name: None,
                page_url: None,
                disposition: DepDisposition::InstallCandidate {
                    artifact: Box::new(test_artifact(
                        "missing-dep",
                        "z-missing.jar",
                        missing_hash,
                        ArtifactSource::LocalFile {
                            path: tmp
                                .path()
                                .join("missing.jar")
                                .to_string_lossy()
                                .into_owned(),
                        },
                        SourceType::Manual,
                    )),
                },
            }],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let plan = rt
            .block_on(InstallPipeline.resolve_plan(intent, &instance_dir, prepared, &NoopReporter))
            .unwrap();
        let outcome = rt.block_on(InstallPipeline.execute_plan(
            &plan,
            &instance_dir,
            "registry-rev",
            &NoopReporter,
            &CancellationToken::new(),
        ));
        assert!(matches!(
            outcome,
            InstallOutcome::Failed {
                snapshot_id: None,
                ..
            }
        ));
        assert!(std::fs::read_dir(instance_dir.join("mods"))
            .unwrap()
            .next()
            .is_none());
        assert!(crate::snapshot::list_snapshots(&instance_dir)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_stage_write_failure_leaves_live_state_unchanged() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("source.jar");
        std::fs::write(&source_path, b"verified local artifact").unwrap();
        let plan = resolve_local_plan(&instance_dir, &source_path, "manual.jar", None);
        set_install_failpoint(Some("stage-write"));
        let outcome =
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(InstallPipeline.execute_plan(
                    &plan,
                    &instance_dir,
                    "registry-rev",
                    &NoopReporter,
                    &CancellationToken::new(),
                ));
        set_install_failpoint(None);

        assert!(matches!(
            outcome,
            InstallOutcome::Failed {
                snapshot_id: None,
                rollback_performed: false,
                ..
            }
        ));
        assert!(!instance_dir.join("mods/manual.jar").exists());
        assert!(crate::snapshot::list_snapshots(&instance_dir)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_manifest_serialization_failure_is_pre_mutation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("source.jar");
        std::fs::write(&source_path, b"verified local artifact").unwrap();
        let plan = resolve_local_plan(&instance_dir, &source_path, "manual.jar", None);
        let before = crate::snapshot::live_file_index(&instance_dir).unwrap();
        set_install_failpoint(Some("manifest-serialize"));
        let outcome =
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(InstallPipeline.execute_plan(
                    &plan,
                    &instance_dir,
                    "registry-rev",
                    &NoopReporter,
                    &CancellationToken::new(),
                ));
        set_install_failpoint(None);

        assert!(matches!(
            outcome,
            InstallOutcome::Failed {
                snapshot_id: None,
                rollback_performed: false,
                ..
            }
        ));
        assert_eq!(
            crate::snapshot::live_file_index(&instance_dir).unwrap(),
            before
        );
    }

    #[test]
    fn test_artifact_rename_failure_restores_original_state() {
        assert_apply_failpoint_restores_original_state("artifact-rename");
    }

    #[test]
    fn test_manifest_rename_failure_restores_original_state() {
        assert_apply_failpoint_restores_original_state("manifest-rename");
    }

    #[test]
    fn test_execute_health_recommendation_is_returned_without_rollback() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("optional-warning.jar");
        write_fabric_test_jar(
            &source_path,
            serde_json::json!({
                "schemaVersion": 1,
                "id": "optional_warning",
                "version": "1.0.0",
                "name": "Optional Warning",
                "recommends": { "missing_optional": "*" }
            }),
        );
        let mut plan =
            resolve_local_plan(&instance_dir, &source_path, "optional-warning.jar", None);
        plan.intent.overrides.skip_health_scan = false;
        plan.fingerprint = compute_plan_fingerprint(&plan).unwrap();
        let outcome =
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(InstallPipeline.execute_plan(
                    &plan,
                    &instance_dir,
                    "registry-rev",
                    &NoopReporter,
                    &CancellationToken::new(),
                ));

        match outcome {
            InstallOutcome::Success {
                health: HealthOutcome::Completed { report },
                ..
            } => {
                assert!(report.warnings.is_empty());
                assert!(!report.recommendations.is_empty());
            }
            other => panic!("expected successful health recommendation, got {other:?}"),
        }
        assert!(instance_dir.join("mods/optional-warning.jar").is_file());
    }

    #[test]
    fn test_execute_health_blocker_rolls_back_to_original_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("blocked.jar");
        write_fabric_test_jar(
            &source_path,
            serde_json::json!({
                "schemaVersion": 1,
                "id": "blocked_mod",
                "version": "1.0.0",
                "name": "Blocked Mod",
                "depends": { "missing_required": "*" }
            }),
        );
        let before = crate::snapshot::live_file_index(&instance_dir).unwrap();
        let mut plan = resolve_local_plan(&instance_dir, &source_path, "blocked.jar", None);
        plan.intent.overrides.skip_health_scan = false;
        plan.fingerprint = compute_plan_fingerprint(&plan).unwrap();
        let outcome =
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(InstallPipeline.execute_plan(
                    &plan,
                    &instance_dir,
                    "registry-rev",
                    &NoopReporter,
                    &CancellationToken::new(),
                ));

        // Health blockers no longer auto-restore: the install is kept so the
        // user can inspect and repair. The snapshot remains available for
        // manual rollback.
        assert!(matches!(outcome, InstallOutcome::HealthRollback { .. }));
        let after = crate::snapshot::live_file_index(&instance_dir).unwrap();
        assert_ne!(after, before);
        assert!(instance_dir.join("mods/blocked.jar").exists());
        if let InstallOutcome::HealthRollback {
            snapshot_id,
            health_report,
            ..
        } = outcome
        {
            assert!(!health_report.blockers.is_empty());
            crate::snapshot::restore_snapshot(&instance_dir, &snapshot_id).unwrap();
            assert_eq!(
                crate::snapshot::live_file_index(&instance_dir).unwrap(),
                before
            );
            assert!(!instance_dir.join("mods/blocked.jar").exists());
        } else {
            unreachable!();
        }
    }

    #[test]
    fn test_batch_update_applies_atomically_and_snapshot_restores_all_versions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        seed_installed_mod(&instance_dir, "alpha", "alpha-1.jar", "1.0", b"alpha old");
        seed_installed_mod(&instance_dir, "beta", "beta-1.jar", "1.0", b"beta old");
        let alpha_source = tmp.path().join("alpha-2.jar");
        let beta_source = tmp.path().join("beta-2.jar");
        std::fs::write(&alpha_source, b"alpha new").unwrap();
        std::fs::write(&beta_source, b"beta new").unwrap();
        let intent = batch_update_intent();
        let prepared = PreparedPlan {
            operation: ResolvedOperation::BatchUpdate {
                operations: vec![
                    ResolvedOperation::Update {
                        old_version_id: "1.0".into(),
                        new_artifact: test_artifact_with_version(
                            "alpha",
                            "alpha-2.jar",
                            "2.0",
                            crate::download::sha256_hex(b"alpha new"),
                            ArtifactSource::LocalFile {
                                path: alpha_source.to_string_lossy().into_owned(),
                            },
                            SourceType::Curated,
                        ),
                    },
                    ResolvedOperation::Update {
                        old_version_id: "1.0".into(),
                        new_artifact: test_artifact_with_version(
                            "beta",
                            "beta-2.jar",
                            "2.0",
                            crate::download::sha256_hex(b"beta new"),
                            ArtifactSource::LocalFile {
                                path: beta_source.to_string_lossy().into_owned(),
                            },
                            SourceType::Curated,
                        ),
                    },
                ],
            },
            dependencies: vec![],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let plan = runtime
            .block_on(InstallPipeline.resolve_plan(intent, &instance_dir, prepared, &NoopReporter))
            .unwrap();
        let outcome = runtime.block_on(InstallPipeline.execute_plan(
            &plan,
            &instance_dir,
            "registry-rev",
            &NoopReporter,
            &CancellationToken::new(),
        ));
        let snapshot_id = match outcome {
            InstallOutcome::Success { snapshot_id, .. } => snapshot_id,
            other => panic!("expected batch update success, got {other:?}"),
        };
        assert_eq!(
            std::fs::read(instance_dir.join("mods/alpha-2.jar")).unwrap(),
            b"alpha new"
        );
        assert_eq!(
            std::fs::read(instance_dir.join("mods/beta-2.jar")).unwrap(),
            b"beta new"
        );
        assert!(!instance_dir.join("mods/alpha-1.jar").exists());
        assert!(!instance_dir.join("mods/beta-1.jar").exists());

        crate::snapshot::restore_snapshot(&instance_dir, &snapshot_id).unwrap();
        assert_eq!(
            std::fs::read(instance_dir.join("mods/alpha-1.jar")).unwrap(),
            b"alpha old"
        );
        assert_eq!(
            std::fs::read(instance_dir.join("mods/beta-1.jar")).unwrap(),
            b"beta old"
        );
        assert!(!instance_dir.join("mods/alpha-2.jar").exists());
        assert!(!instance_dir.join("mods/beta-2.jar").exists());
    }

    #[test]
    fn test_batch_update_second_artifact_failure_keeps_every_original_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        seed_installed_mod(&instance_dir, "alpha", "alpha-1.jar", "1.0", b"alpha old");
        seed_installed_mod(&instance_dir, "beta", "beta-1.jar", "1.0", b"beta old");
        let alpha_source = tmp.path().join("alpha-2.jar");
        let beta_source = tmp.path().join("beta-2.jar");
        std::fs::write(&alpha_source, b"alpha new").unwrap();
        std::fs::write(&beta_source, b"beta wrong bytes").unwrap();
        let prepared = PreparedPlan {
            operation: ResolvedOperation::BatchUpdate {
                operations: vec![
                    ResolvedOperation::Update {
                        old_version_id: "1.0".into(),
                        new_artifact: test_artifact_with_version(
                            "alpha",
                            "alpha-2.jar",
                            "2.0",
                            crate::download::sha256_hex(b"alpha new"),
                            ArtifactSource::LocalFile {
                                path: alpha_source.to_string_lossy().into_owned(),
                            },
                            SourceType::Curated,
                        ),
                    },
                    ResolvedOperation::Update {
                        old_version_id: "1.0".into(),
                        new_artifact: test_artifact_with_version(
                            "beta",
                            "beta-2.jar",
                            "2.0",
                            crate::download::sha256_hex(b"expected beta bytes"),
                            ArtifactSource::LocalFile {
                                path: beta_source.to_string_lossy().into_owned(),
                            },
                            SourceType::Curated,
                        ),
                    },
                ],
            },
            dependencies: vec![],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let plan = runtime
            .block_on(InstallPipeline.resolve_plan(
                batch_update_intent(),
                &instance_dir,
                prepared,
                &NoopReporter,
            ))
            .unwrap();
        let outcome = runtime.block_on(InstallPipeline.execute_plan(
            &plan,
            &instance_dir,
            "registry-rev",
            &NoopReporter,
            &CancellationToken::new(),
        ));

        assert!(matches!(
            outcome,
            InstallOutcome::Failed {
                snapshot_id: None,
                ..
            }
        ));
        assert_eq!(
            std::fs::read(instance_dir.join("mods/alpha-1.jar")).unwrap(),
            b"alpha old"
        );
        assert_eq!(
            std::fs::read(instance_dir.join("mods/beta-1.jar")).unwrap(),
            b"beta old"
        );
        assert!(!instance_dir.join("mods/alpha-2.jar").exists());
        assert!(!instance_dir.join("mods/beta-2.jar").exists());
    }

    #[test]
    fn test_batch_update_conflict_blocks_before_artifact_staging() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        seed_installed_mod(&instance_dir, "alpha", "alpha-1.jar", "1.0", b"alpha old");
        let nonexistent = tmp.path().join("must-not-be-read.jar");
        let prepared = PreparedPlan {
            operation: ResolvedOperation::BatchUpdate {
                operations: vec![ResolvedOperation::Update {
                    old_version_id: "1.0".into(),
                    new_artifact: test_artifact_with_version(
                        "alpha",
                        "alpha-2.jar",
                        "2.0",
                        "a".repeat(64),
                        ArtifactSource::LocalFile {
                            path: nonexistent.to_string_lossy().into_owned(),
                        },
                        SourceType::Curated,
                    ),
                }],
            },
            dependencies: vec![],
            conflicts: vec![DepConflict {
                conflict_id: "batch-conflict".into(),
                kind: ConflictKind::IncompatibleMod,
                existing_mod_jar_id: "other".into(),
                incoming_mod_jar_id: "alpha".into(),
                message: "Selected updates conflict.".into(),
                blocking: true,
                resolution_options: vec![ConflictResolution::Abort],
                chosen: None,
            }],
            registry_revision: "registry-rev".into(),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let plan = runtime
            .block_on(InstallPipeline.resolve_plan(
                batch_update_intent(),
                &instance_dir,
                prepared,
                &NoopReporter,
            ))
            .unwrap();
        assert!(!plan.is_fully_resolved());
        let outcome = runtime.block_on(InstallPipeline.execute_plan(
            &plan,
            &instance_dir,
            "registry-rev",
            &NoopReporter,
            &CancellationToken::new(),
        ));
        assert!(matches!(
            outcome,
            InstallOutcome::Failed {
                snapshot_id: None,
                ..
            }
        ));
        assert!(crate::snapshot::list_snapshots(&instance_dir)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_locked_instance_batch_update_is_blocked_in_read_only_plan() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let manifest_path = instance_dir.join("instance_manifest.json");
        let mut manifest: crate::models::InstanceManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest.is_locked = true;
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let prepared = PreparedPlan {
            operation: ResolvedOperation::BatchUpdate { operations: vec![] },
            dependencies: vec![],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        let plan = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(InstallPipeline.resolve_plan(
                batch_update_intent(),
                &instance_dir,
                prepared,
                &NoopReporter,
            ))
            .unwrap();

        assert!(plan
            .blocking_errors
            .iter()
            .any(|error| error.code == "ERR_INSTANCE_LOCKED"));
        assert!(!plan.is_fully_resolved());
    }

    struct NoopReporter;

    impl ProgressReporter for NoopReporter {
        fn report(&self, _event: ProgressEvent) {}
    }

    fn set_install_failpoint(value: Option<&'static str>) {
        INSTALL_TEST_FAILPOINT.with(|slot| *slot.borrow_mut() = value);
    }

    fn assert_apply_failpoint_restores_original_state(failpoint: &'static str) {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("source.jar");
        std::fs::write(&source_path, b"verified local artifact").unwrap();
        let plan = resolve_local_plan(&instance_dir, &source_path, "manual.jar", None);
        let before = crate::snapshot::live_file_index(&instance_dir).unwrap();
        set_install_failpoint(Some(failpoint));
        let outcome =
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(InstallPipeline.execute_plan(
                    &plan,
                    &instance_dir,
                    "registry-rev",
                    &NoopReporter,
                    &CancellationToken::new(),
                ));
        set_install_failpoint(None);

        assert!(matches!(
            outcome,
            InstallOutcome::Failed {
                rollback_performed: true,
                ..
            }
        ));
        assert_eq!(
            crate::snapshot::live_file_index(&instance_dir).unwrap(),
            before
        );
        assert!(!instance_dir.join("mods/manual.jar").exists());
    }

    fn write_fabric_test_jar(path: &Path, metadata: serde_json::Value) {
        use std::io::Write;
        let file = std::fs::File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("fabric.mod.json", zip::write::FileOptions::default())
            .unwrap();
        archive
            .write_all(serde_json::to_string(&metadata).unwrap().as_bytes())
            .unwrap();
        archive.finish().unwrap();
    }

    fn make_instance(tmp: &tempfile::TempDir) -> std::path::PathBuf {
        let directory = tmp.path().join("instance");
        std::fs::create_dir_all(directory.join("mods")).unwrap();
        let manifest = crate::models::InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: "test".into(),
            name: "Test".into(),
            created_from_pack: None,
            minecraft_version: TEST_INSTANCE_MC_VERSION.into(),
            loader: "fabric".into(),
            loader_version: "0.16.0".into(),
            is_locked: false,
            mods: vec![],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        };
        std::fs::write(
            directory.join("instance_manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        directory
    }

    fn test_artifact(
        item_id: &str,
        filename: &str,
        sha256: String,
        source: ArtifactSource,
        source_type: SourceType,
    ) -> ResolvedArtifact {
        test_artifact_with_version(item_id, filename, "1.0", sha256, source, source_type)
    }

    fn test_artifact_with_version(
        item_id: &str,
        filename: &str,
        version: &str,
        sha256: String,
        source: ArtifactSource,
        source_type: SourceType,
    ) -> ResolvedArtifact {
        ResolvedArtifact::Download(ResolvedDownload {
            item_id: item_id.into(),
            version_id: version.into(),
            source,
            hashes: HashSpec {
                values: vec![HashedValue {
                    algorithm: HashAlgorithm::Sha256,
                    value: sha256,
                }],
            },
            size: 0,
            filename: filename.into(),
            metadata: ArtifactMetadata {
                source_type,
                registry_id: Some(item_id.into()),
                modrinth_id: None,
                content_type: "mod".into(),
                version: None,
                download_strategy: None,
                pinned_host: None,
            },
        })
    }

    #[test]
    fn curated_source_accepts_published_sha512_when_sha256_is_unavailable() {
        let artifact = ResolvedArtifact::Download(ResolvedDownload {
            item_id: "xaeros-minimap".into(),
            version_id: "fabric-26.2-26.4.2".into(),
            source: ArtifactSource::Download {
                url: "https://cdn.modrinth.com/xaerominimap.jar".into(),
            },
            hashes: HashSpec {
                values: vec![HashedValue {
                    algorithm: HashAlgorithm::Sha512,
                    value: "a".repeat(128),
                }],
            },
            size: 1,
            filename: "xaerominimap-fabric-26.2-26.4.2.jar".into(),
            metadata: ArtifactMetadata {
                source_type: SourceType::Curated,
                registry_id: Some("xaeros-minimap".into()),
                modrinth_id: Some("1bokaNcj".into()),
                content_type: "mod".into(),
                version: Some("fabric-26.2-26.4.2".into()),
                download_strategy: None,
                pinned_host: None,
            },
        });

        assert!(validate_artifact_hashes(&artifact).is_ok());
    }

    fn batch_update_intent() -> InstallIntent {
        InstallIntent {
            action: InstallAction::BatchUpdate {
                items: vec![
                    BatchUpdateItem {
                        item_id: "alpha".into(),
                        target_version: "2.0".into(),
                    },
                    BatchUpdateItem {
                        item_id: "beta".into(),
                        target_version: "2.0".into(),
                    },
                ],
            },
            target_instance: "test".into(),
            optional_deps: OptionalDepsPolicy::ExcludeAll,
            requested_by: RequestSource::AutoUpdate,
            overrides: PlanOverrides {
                skip_health_scan: true,
                ..PlanOverrides::default()
            },
        }
    }

    fn seed_installed_mod(
        instance_dir: &Path,
        item_id: &str,
        filename: &str,
        version: &str,
        contents: &[u8],
    ) {
        let file_path = instance_dir.join("mods").join(filename);
        std::fs::write(&file_path, contents).unwrap();
        let manifest_path = instance_dir.join("instance_manifest.json");
        let mut manifest: crate::models::InstanceManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest.mods.push(crate::models::InstalledMod {
            pack_managed: false,
            filename: filename.into(),
            registry_id: Some(item_id.into()),
            modrinth_id: None,
            source: "registry".into(),
            source_url: Some(format!("https://example.com/{filename}")),
            version: Some(version.into()),
            sha256: crate::download::sha256_hex(contents),
            installed_at: "2026-07-12T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: Some(item_id.into()),
            enabled: true,
            content_type: "mod".into(),
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
            provided_mod_ids: vec![],
        });
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn local_intent(item_id: &str) -> InstallIntent {
        InstallIntent {
            action: InstallAction::Install {
                source_type: SourceType::Manual,
                item_id: item_id.into(),
                candidate_version: Some("1.0".into()),
            },
            target_instance: "test".into(),
            optional_deps: OptionalDepsPolicy::ExcludeAll,
            requested_by: RequestSource::Interactive,
            overrides: PlanOverrides {
                skip_health_scan: true,
                ..PlanOverrides::default()
            },
        }
    }

    fn resolve_local_plan(
        instance_dir: &Path,
        source_path: &Path,
        target_filename: &str,
        expected_hash: Option<String>,
    ) -> ResolvedInstallPlan {
        let contents = std::fs::read(source_path).unwrap();
        let hash = expected_hash.unwrap_or_else(|| crate::download::sha256_hex(&contents));
        let prepared = PreparedPlan {
            operation: ResolvedOperation::Install {
                artifact: test_artifact(
                    "manual-item",
                    target_filename,
                    hash,
                    ArtifactSource::LocalFile {
                        path: source_path.to_string_lossy().into_owned(),
                    },
                    SourceType::Manual,
                ),
            },
            dependencies: vec![],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(InstallPipeline.resolve_plan(
                local_intent("manual-item"),
                instance_dir,
                prepared,
                &NoopReporter,
            ))
            .unwrap()
    }

    fn resolve_local_plan_with_overrides(
        instance_dir: &Path,
        source_path: &Path,
        target_filename: &str,
        overrides: PlanOverrides,
    ) -> ResolvedInstallPlan {
        let contents = std::fs::read(source_path).unwrap();
        let hash = crate::download::sha256_hex(&contents);
        let prepared = PreparedPlan {
            operation: ResolvedOperation::Install {
                artifact: test_artifact(
                    "manual-item",
                    target_filename,
                    hash,
                    ArtifactSource::LocalFile {
                        path: source_path.to_string_lossy().into_owned(),
                    },
                    SourceType::Manual,
                ),
            },
            dependencies: vec![],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        let intent = InstallIntent {
            action: InstallAction::Install {
                source_type: SourceType::Manual,
                item_id: "manual-item".into(),
                candidate_version: Some("1.0".into()),
            },
            target_instance: "test".into(),
            optional_deps: OptionalDepsPolicy::ExcludeAll,
            requested_by: RequestSource::Interactive,
            overrides,
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(InstallPipeline.resolve_plan(intent, instance_dir, prepared, &NoopReporter))
            .unwrap()
    }

    /// The newest stable fabric loader the embedded catalog carries for the
    /// test instance's Minecraft version.
    ///
    /// Derived rather than written as a literal. `loader-manifests/` is owned
    /// by the loader-refresh workflow and gains new versions on a schedule, so
    /// a pinned expectation here fails on a commit that changed no launcher
    /// code — which is exactly what a catalog refresh to 0.19.5 did to a test
    /// asserting 0.19.3. What these tests are actually about is that the
    /// pipeline recommends the newest usable loader and excludes the
    /// instance's current one; the specific number is incidental.
    fn newest_stable_fabric_for_test_instance() -> String {
        let catalog = crate::loader_manifests::LoaderCatalog::embedded();
        let mut versions: Vec<&str> = catalog
            .loaders
            .get("fabric")
            .expect("embedded catalog must carry fabric")
            .iter()
            .filter(|entry| {
                entry.mc_version == TEST_INSTANCE_MC_VERSION
                    && entry.release_channel
                        == crate::loader_manifests::LoaderReleaseChannel::Stable
            })
            .map(|entry| entry.loader_version.as_str())
            .collect();
        versions.sort_by(|a, b| crate::version_match::compare_versions(a, b));
        versions
            .last()
            .expect("embedded catalog must carry a stable fabric entry for the test MC version")
            .to_string()
    }

    /// A fabric JAR whose loader requirement is `fabricloader >= <range>`.
    /// The test instance is MC 1.21.1 / fabric 0.16.0 (both pinned tuples in
    /// the embedded signed catalog, which also carries later 1.21.1 entries),
    /// so a `>=0.17.0` requirement proves an Incompatible status with a
    /// deterministic recommendation of the newest stable catalog entry.
    fn write_loader_requiring_fabric_jar(path: &Path, loader_range: &str) {
        write_fabric_test_jar(
            path,
            serde_json::json!({
                "id": "loader-needy",
                "version": "1.0.0",
                "depends": {
                    "fabricloader": loader_range,
                    "minecraft": ">=1.21.1"
                }
            }),
        );
    }

    fn loader_change_choice(plan: &ResolvedInstallPlan) -> Option<&PendingChoice> {
        plan.pending_choices
            .iter()
            .find(|choice| matches!(choice, PendingChoice::LoaderChange { .. }))
    }

    #[test]
    fn loader_change_surfaces_pending_choice_before_commit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("needy.jar");
        write_loader_requiring_fabric_jar(&source_path, ">=0.17.0");

        let plan = resolve_local_plan(&instance_dir, &source_path, "needy.jar", None);

        let choice = loader_change_choice(&plan)
            .expect("a loader mismatch must surface a PendingChoice::LoaderChange");
        let PendingChoice::LoaderChange {
            choice_id,
            loader,
            current_version,
            recommended_version,
            compatible_versions,
            requirements,
            conflicts,
        } = choice
        else {
            unreachable!()
        };
        let newest = newest_stable_fabric_for_test_instance();
        assert_eq!(choice_id, "loader-change");
        assert_eq!(loader, "fabric");
        assert_eq!(current_version, "0.16.0");
        assert_eq!(recommended_version, &newest);
        assert!(compatible_versions.contains(&newest));
        // The instance's own version does not satisfy `>=0.17.0`, so it must
        // not be offered as a way out of the mismatch.
        assert!(!compatible_versions.contains(&"0.16.0".to_string()));
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].target_id, "fabricloader");
        assert_eq!(requirements[0].candidate_version.as_deref(), Some("0.16.0"));
        assert!(matches!(
            requirements[0].verdict,
            crate::loader_compatibility::RequirementVerdict::Unsatisfied
        ));
        assert!(conflicts.is_empty());
        assert!(!plan.is_fully_resolved(), "the choice must block execution");
    }

    #[test]
    fn loader_change_choice_carries_deterministic_json_shape() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("needy.jar");
        write_loader_requiring_fabric_jar(&source_path, ">=0.17.0");

        let plan = resolve_local_plan(&instance_dir, &source_path, "needy.jar", None);
        let value = serde_json::to_value(loader_change_choice(&plan).unwrap()).unwrap();
        let newest = newest_stable_fabric_for_test_instance();
        assert_eq!(value["type"], "loader-change");
        assert_eq!(value["currentVersion"], "0.16.0");
        assert_eq!(value["recommendedVersion"], newest.as_str());
        // Compatible versions are preference-sorted, so the recommendation is
        // also the head of the list.
        assert_eq!(value["compatibleVersions"][0], newest.as_str());
        assert_eq!(value["requirements"][0]["target_id"], "fabricloader");
        assert_eq!(value["requirements"][0]["candidate_version"], "0.16.0");
        assert_eq!(value["requirements"][0]["verdict"], "unsatisfied");
    }

    #[test]
    fn loader_change_approval_folds_into_plan_without_choice() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("needy.jar");
        write_loader_requiring_fabric_jar(&source_path, ">=0.17.0");

        let plan = resolve_local_plan_with_overrides(
            &instance_dir,
            &source_path,
            "needy.jar",
            PlanOverrides {
                skip_health_scan: true,
                approve_loader_version: Some("0.19.3".into()),
                ..PlanOverrides::default()
            },
        );

        assert!(plan.pending_choices.is_empty());
        assert_eq!(
            plan.loader_change,
            Some(LoaderChangePlan {
                loader: "fabric".into(),
                from_version: "0.16.0".into(),
                to_version: "0.19.3".into(),
            })
        );
        assert!(plan.is_fully_resolved());
        assert_eq!(plan.files_to_add.len(), 1);
    }

    #[test]
    fn loader_change_approval_with_incompatible_version_keeps_choice() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("needy.jar");
        write_loader_requiring_fabric_jar(&source_path, ">=0.17.0");

        let plan = resolve_local_plan_with_overrides(
            &instance_dir,
            &source_path,
            "needy.jar",
            PlanOverrides {
                skip_health_scan: true,
                approve_loader_version: Some("0.16.0".into()),
                ..PlanOverrides::default()
            },
        );

        assert!(plan.loader_change.is_none());
        assert!(loader_change_choice(&plan).is_some());
        assert!(!plan.is_fully_resolved());
    }

    #[test]
    fn loader_change_approval_accepts_any_compatible_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("needy.jar");
        write_loader_requiring_fabric_jar(&source_path, ">=0.17.0");

        // 0.17.0 satisfies the requirement and is a signed catalog candidate,
        // but is not the recommended 0.19.3 — it must still be accepted so the
        // UI can offer every compatible version, not just the recommendation.
        let plan = resolve_local_plan_with_overrides(
            &instance_dir,
            &source_path,
            "needy.jar",
            PlanOverrides {
                skip_health_scan: true,
                approve_loader_version: Some("0.17.0".into()),
                ..PlanOverrides::default()
            },
        );

        assert!(plan.pending_choices.is_empty());
        assert_eq!(
            plan.loader_change,
            Some(LoaderChangePlan {
                loader: "fabric".into(),
                from_version: "0.16.0".into(),
                to_version: "0.17.0".into(),
            })
        );
        assert!(plan.is_fully_resolved());
        assert_eq!(plan.files_to_add.len(), 1);
    }

    #[test]
    fn loader_change_not_surfaced_when_current_loader_satisfies_requirements() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("fine.jar");
        write_loader_requiring_fabric_jar(&source_path, ">=0.15.0");

        let plan = resolve_local_plan(&instance_dir, &source_path, "fine.jar", None);

        assert!(plan.pending_choices.is_empty());
        assert!(plan.loader_change.is_none());
        assert!(plan
            .warnings
            .iter()
            .all(|warning| warning.code != "WARN_LOADER_INCOMPATIBLE"));
        assert!(plan.is_fully_resolved());
    }

    #[test]
    fn loader_change_not_surfaced_when_no_candidate_satisfies_requirements() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("impossible.jar");
        write_loader_requiring_fabric_jar(&source_path, ">=99.0.0");

        let plan = resolve_local_plan(&instance_dir, &source_path, "impossible.jar", None);

        assert!(plan.pending_choices.is_empty());
        assert!(plan.loader_change.is_none());
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.code == "WARN_LOADER_INCOMPATIBLE"));
        assert!(plan.is_fully_resolved());
    }

    #[test]
    fn loader_change_considers_existing_enabled_mod_requirements() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        write_fabric_test_jar(
            &instance_dir.join("mods/existing.jar"),
            serde_json::json!({
                "id": "existing",
                "version": "1.0.0",
                "depends": {"fabricloader": "<0.17.0"}
            }),
        );
        let source_path = tmp.path().join("needy.jar");
        write_loader_requiring_fabric_jar(&source_path, ">=0.17.0");

        let plan = resolve_local_plan(&instance_dir, &source_path, "needy.jar", None);

        assert!(loader_change_choice(&plan).is_none());
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.code == "WARN_LOADER_INCOMPATIBLE"));
        assert!(plan.loader_change.is_none());
    }

    #[test]
    fn loader_change_not_surfaced_for_download_artifacts() {
        // Download artifacts have no JAR at plan time; the post-install
        // health scan remains the safety net (documented absence).
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("needy.jar");
        write_loader_requiring_fabric_jar(&source_path, ">=0.17.0");

        let contents = std::fs::read(&source_path).unwrap();
        let hash = crate::download::sha256_hex(&contents);
        let prepared = PreparedPlan {
            operation: ResolvedOperation::Install {
                artifact: test_artifact(
                    "manual-item",
                    "needy.jar",
                    hash,
                    ArtifactSource::Download {
                        url: "https://example.com/needy.jar".into(),
                    },
                    SourceType::Manual,
                ),
            },
            dependencies: vec![],
            conflicts: vec![],
            registry_revision: "registry-rev".into(),
        };
        let plan = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(InstallPipeline.resolve_plan(
                local_intent("manual-item"),
                &instance_dir,
                prepared,
                &NoopReporter,
            ))
            .unwrap();

        assert!(plan.pending_choices.is_empty());
        assert!(plan.loader_change.is_none());
    }

    #[test]
    fn loader_change_blocks_execution_until_approved() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("needy.jar");
        write_loader_requiring_fabric_jar(&source_path, ">=0.17.0");
        let manifest_path = instance_dir.join("instance_manifest.json");
        let manifest_before = std::fs::read_to_string(&manifest_path).unwrap();

        let plan = resolve_local_plan(&instance_dir, &source_path, "needy.jar", None);
        let outcome =
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(InstallPipeline.execute_plan(
                    &plan,
                    &instance_dir,
                    "registry-rev",
                    &NoopReporter,
                    &CancellationToken::new(),
                ));

        assert!(matches!(outcome, InstallOutcome::Failed { .. }));
        assert!(!instance_dir.join("mods/needy.jar").exists());
        assert_eq!(
            std::fs::read_to_string(&manifest_path).unwrap(),
            manifest_before,
            "the unapproved loader change must not modify the manifest"
        );
    }

    #[test]
    fn loader_change_without_core_loader_service_fails_before_mutation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let instance_dir = make_instance(&tmp);
        let source_path = tmp.path().join("needy.jar");
        write_loader_requiring_fabric_jar(&source_path, ">=0.17.0");
        let manifest_path = instance_dir.join("instance_manifest.json");

        let plan = resolve_local_plan_with_overrides(
            &instance_dir,
            &source_path,
            "needy.jar",
            PlanOverrides {
                skip_health_scan: true,
                approve_loader_version: Some("0.19.3".into()),
                ..PlanOverrides::default()
            },
        );
        let outcome =
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(InstallPipeline.execute_plan(
                    &plan,
                    &instance_dir,
                    "registry-rev",
                    &NoopReporter,
                    &CancellationToken::new(),
                ));

        assert!(matches!(outcome, InstallOutcome::Failed { .. }));
        assert!(!instance_dir.join("mods/needy.jar").exists());
        let manifest: crate::models::InstanceManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest.loader, "fabric");
        assert_eq!(manifest.loader_version, "0.16.0");
        assert!(manifest.mods.is_empty());
    }

    fn test_plan() -> ResolvedInstallPlan {
        ResolvedInstallPlan {
            fingerprint: "test-fp".into(),
            intent: InstallIntent {
                action: InstallAction::Install {
                    source_type: SourceType::Curated,
                    item_id: "test".into(),
                    candidate_version: None,
                },
                target_instance: "test".into(),
                optional_deps: OptionalDepsPolicy::ExcludeAll,
                requested_by: RequestSource::Interactive,
                overrides: PlanOverrides::default(),
            },
            operation: ResolvedOperation::Install {
                artifact: ResolvedArtifact::Download(ResolvedDownload {
                    item_id: "test".into(),
                    version_id: "1.0".into(),
                    source: ArtifactSource::Download {
                        url: "https://example.com/test.jar".into(),
                    },
                    hashes: HashSpec { values: vec![] },
                    size: 0,
                    filename: "test.jar".into(),
                    metadata: ArtifactMetadata {
                        source_type: SourceType::Curated,
                        registry_id: Some("test".into()),
                        modrinth_id: None,
                        content_type: "mod".into(),
                        version: None,
                        download_strategy: None,
                        pinned_host: None,
                    },
                }),
            },
            dependencies: vec![],
            conflicts: vec![],
            files_to_add: vec![],
            files_to_remove: vec![],
            files_to_disable: vec![],
            snapshot: SnapshotPlan {
                label: "test-snapshot".into(),
                estimated_bytes: 0,
            },
            disk_estimate: DiskSpaceEstimate::zero(),
            warnings: vec![],
            blocking_errors: vec![PlanError {
                code: "ERR_TEST_BLOCKER".into(),
                message: "Test blocker".into(),
            }],
            pending_choices: vec![],
            loader_change: None,
            created_at: String::new(),
            instance_state_hash: String::new(),
            registry_revision: String::new(),
        }
    }
}
