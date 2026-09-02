//! Minecraft-version migration readiness report.
//!
//! Answers: "can this instance move to `target_version` yet, and what breaks?"
//! without touching the instance. Every installed content entry is classified and
//! counted so the UI can render `17 ready, 2 not yet, 1 abandoned` and let the
//! user drill into the per-mod detail.
//!
//! # Classification
//!
//! * `ready` — a build for the target exists (Modrinth reports a version whose
//!   `game_versions` contains the target *and* whose `loaders` matches the
//!   instance loader when the project is a mod).
//! * `not yet` — the project is alive but has no target build.
//! * `abandoned` — no target build and the project has not been updated for a
//!   long time (`ABANDONED_THRESHOLD_DAYS`; see below).
//! * `superseded` — curator data names a successor. This is checked first and
//!   outranks every other signal: a dead project with a known replacement is
//!   `superseded`, not `abandoned`.
//! * `unknown` — the target-support lookup failed (network, rate-limit,
//!   `modrinth_enabled` off, …). This is *never* collapsed into `abandoned` —
//!   telling a user their favourite mod is dead because Modrinth rate-limited us
//!   is worse than saying nothing.
//! * `unclassifiable` — the entry has no Modrinth identity at all (manual jars,
//!   curated-registry-only entries with no `modrinth_id`). These cannot be
//!   classified by API lookup and are reported explicitly rather than silently
//!   dropped.
//!
//! # Usage
//!
//! The pure core is [`generate_migration_report`]: it takes a slice of
//! [`crate::models::InstalledMod`], a target version, the instance loader, and
//! two injected seams:
//!
//! * a [`ModrinthChecker`] that answers "does this Modrinth project have a
//!   target build?" plus its last-updated timestamp for staleness;
//! * a [`SuccessorLookup`] that answers "does curator data name a replacement?"
//!
//! Both seams are trait objects so tests can inject a fake lookup without
//! touching the network and so production can wire curator data later without a
//! schema change. Today the successor map can be empty — the default
//! [`NoopSuccessorLookup`] and [`EmptySuccessorLookup`] do exactly that.
//!
//! Production callers that already have a [`crate::ctx::Ctx`] can use
//! [`MigrationService`], which builds a live checker from the existing
//! helpers in [`crate::modrinth`] (`fetch_project_full`,
//! `list_raw_modrinth_versions_http` via a dummy [`crate::models::InstanceRow`])
//! rather than a parallel HTTP path. The live checker respects the
//! `modrinth_enabled` / `network_modrinth_enabled` gates the same way
//! [`crate::modrinth::ModrinthService`] does; a disabled gate surfaces as
//! `unknown`, not `abandoned`.
//!
//! # Abandoned threshold
//!
//! `abandoned` is staleness + absence. The current threshold is
//! [`ABANDONED_THRESHOLD_DAYS`] (180 days ≈ six months) after the project's
//! `updated` timestamp (`source_updated_at` from `GET /v2/project/{id}`).
//! The value is a judgement call; six months is a conventional "no recent
//! activity" horizon for Modrinth mods and fits the "not yet vs. probably never"
//! intuition better than a shorter window (too many false abandonments) or a
//! longer one (hides real deaths). It is a `const` so a future curator policy
//! can tighten or loosen it without touching the classification logic. When the
//! timestamp is missing or unparseable the entry is conservatively `not yet`.
//!
//! # Unclassifiable handling
//!
//! Entries with no `modrinth_id` are `unclassifiable`, never silently omitted.
//! The report distinguishes `manual` (no registry id either, or a manual source)
//! from `curated_only` (has a `registry_id` but the curated row carries no
//! external Modrinth mapping). A live [`MigrationService`] optionally enriches a
//! `registry_id`-only entry via `registry.db` before classifying, so a curated
//! entry whose row *does* carry a `modrinth_id` is still checked.

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::models::InstalledMod;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// How long without an upstream update before "no target build" is treated as
/// abandoned rather than merely not yet.
pub const ABANDONED_THRESHOLD_DAYS: i64 = 180;

/// Per-mod migration status — the inner classification that powers the summary
/// counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStatus {
    Ready,
    NotYet,
    Abandoned,
    Superseded,
    Unknown,
    Unclassifiable,
}

impl MigrationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotYet => "not_yet",
            Self::Abandoned => "abandoned",
            Self::Superseded => "superseded",
            Self::Unknown => "unknown",
            Self::Unclassifiable => "unclassifiable",
        }
    }
}

/// Why an entry could not be classified by Modrinth lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnclassifiableReason {
    /// No Modrinth or registry identity — a manual jar, drag-dropped file, etc.
    Manual,
    /// Curated registry entry that carries no external Modrinth mapping.
    CuratedOnly,
    /// Other (should not happen; kept so a future source is not forced into the
    /// wrong bucket).
    Other,
}

/// Overall verdict for the instance. The counts are authoritative; the verdict
/// is the honest one-liner the UI can headline with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationVerdict {
    /// Every classifiable entry is ready and there are no unknowns or
    /// unclassifiables. Safe to migrate.
    Ready,
    /// At least one entry is alive but has no target build yet. Waiting may
    /// resolve it; nothing is known-dead.
    NotYet,
    /// At least one entry is abandoned or superseded. Migration would drop
    /// content without a replacement.
    Blocked,
    /// At least one entry could not be checked (network, rate-limit, disabled).
    /// The report is incomplete and must not be presented as definitive.
    Unknown,
    /// At least one entry has no Modrinth identity and needs manual review.
    NeedsReview,
}

impl MigrationVerdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotYet => "not_yet",
            Self::Blocked => "blocked",
            Self::Unknown => "unknown",
            Self::NeedsReview => "needs_review",
        }
    }
}

/// Counts for the header line: "17 ready, 2 not yet, 1 abandoned".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MigrationSummary {
    pub total: usize,
    pub ready: usize,
    pub not_yet: usize,
    pub abandoned: usize,
    pub superseded: usize,
    pub unknown: usize,
    pub unclassifiable: usize,
}

/// Curator-provided successor for a dead project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessorInfo {
    /// Replacement identifier (registry id, Modrinth id, or display name).
    pub replacement_id: String,
    /// Human-readable replacement name, if known.
    pub replacement_name: Option<String>,
    /// Optional curator note / reason.
    pub reason: Option<String>,
}

/// The per-mod detail behind the summary counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModMigrationEntry {
    /// The physical filename on disk (`sodium-0.5.0.jar`).
    pub filename: String,
    /// Display name for the UI — `registry_id` / `modrinth_id` fallback to
    /// filename when no registry metadata is available.
    pub display_name: String,
    /// Raw Modrinth project id, if any.
    pub modrinth_id: Option<String>,
    /// Curated registry id, if any.
    pub registry_id: Option<String>,
    /// Content type (`mod`, `resourcepack`, …) — preserved so the UI can group.
    pub content_type: String,
    /// Installed version string, if recorded.
    pub installed_version: Option<String>,
    /// Classification for this entry.
    pub status: MigrationStatus,
    /// Why an `Unclassifiable` entry could not be checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unclassifiable_reason: Option<UnclassifiableReason>,
    /// When the upstream project was last updated (RFC3339), if known. Present
    /// for `not_yet` / `abandoned` to explain the distinction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    /// Whether a target build was found (only meaningful for `ready` / `not_yet`
    /// / `abandoned`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_target_build: Option<bool>,
    /// Successor for `superseded` entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub successor: Option<SuccessorInfo>,
    /// The concrete build a migration would install for `ready` entries, when
    /// the checker could name one. `None` on every non-`ready` entry, and on
    /// `ready` entries whose checker only answered the boolean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_build: Option<TargetBuildInfo>,
    /// Launcher error code for `unknown` entries (e.g. `ERR_NETWORK`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Human-readable error for `unknown` entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Full migration readiness report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    /// Instance id the report was generated for.
    pub instance_id: String,
    pub source_version: String,
    pub target_version: String,
    /// Loader of the source instance (used for Modrinth loader filtering).
    pub loader: String,
    pub summary: MigrationSummary,
    pub verdict: MigrationVerdict,
    pub mods: Vec<ModMigrationEntry>,
    /// Non-fatal warnings (e.g. empty instance, target == source).
    #[serde(default)]
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Seams
// ---------------------------------------------------------------------------

/// The concrete build selected for a target version — the answer to "which
/// one", not just "is there one".
///
/// The [`ModrinthChecker`] already fetches the candidate list to decide
/// `has_target_build`; choosing the best candidate from that same list costs
/// zero additional API calls and guarantees the executor installs exactly the
/// build the report judged ready. The readiness report itself ignores this
/// field; it exists so [`crate::version_migration`] can execute without a
/// second resolution pass whose filter policy could drift from the checker's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetBuildInfo {
    /// Modrinth version id (UUID).
    pub version_id: String,
    /// Human-readable version number for the manifest entry.
    pub version_number: String,
    /// Published filename of the primary file.
    pub filename: String,
    /// Download URL of the primary file.
    pub download_url: String,
    /// Modrinth-published SHA-1, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    /// Modrinth-published SHA-512, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha512: Option<String>,
    /// Published file size in bytes, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Whether the selected version is an alpha/beta build. Migration warns
    /// rather than silently swapping a release mod for a prerelease.
    #[serde(default)]
    pub is_prerelease: bool,
}

/// Outcome of checking whether a Modrinth project has a target build.
#[derive(Debug, Clone)]
pub struct SupportInfo {
    /// Whether any version supports the target `game_version` (and loader when
    /// the project is a `mod`).
    pub has_target_build: bool,
    /// Upstream `updated` timestamp (RFC3339) for abandonment detection. `None`
    /// when the project has a target build (not needed) or when the timestamp
    /// could not be fetched.
    pub last_updated: Option<String>,
    /// The specific build chosen for the target version. `Some` exactly when a
    /// target build was found and the checker could name one; a checker that
    /// only answers the boolean is still valid (the report ignores this field),
    /// but such an entry cannot be *executed* by [`crate::version_migration`].
    pub target_build: Option<TargetBuildInfo>,
}

/// Trait that answers "does this Modrinth project have a target build?"
///
/// Production uses [`LiveModrinthChecker`] (which delegates to the existing
/// helpers in [`crate::modrinth`]); tests inject a fake that never touches the
/// network.
#[async_trait]
pub trait ModrinthChecker: Send + Sync {
    async fn check_target_support(
        &self,
        project_id: &str,
        target_version: &str,
        loader: &str,
        content_type: &str,
    ) -> Result<SupportInfo, LauncherError>;
}

/// Trait that answers "does curator data name a replacement for this mod?"
///
/// The key is the mod's stable identity — typically `modrinth_id` or
/// `registry_id`. The trait is deliberately small so a `HashMap` can back it
/// today and a `registry.db` query can back it tomorrow without changing the
/// report logic.
pub trait SuccessorLookup: Send + Sync {
    fn successor_for(&self, id: &str) -> Option<SuccessorInfo>;
}

/// Always-empty successor lookup. Suitable as the default until curator data
/// exists.
#[derive(Debug, Clone, Default)]
pub struct NoopSuccessorLookup;

impl SuccessorLookup for NoopSuccessorLookup {
    fn successor_for(&self, _id: &str) -> Option<SuccessorInfo> {
        None
    }
}

/// Map-backed successor lookup.
#[derive(Debug, Clone, Default)]
pub struct MapSuccessorLookup {
    map: HashMap<String, SuccessorInfo>,
}

impl MapSuccessorLookup {
    pub fn new(map: HashMap<String, SuccessorInfo>) -> Self {
        // Normalize keys to lowercase so lookups are case-insensitive.
        let map = map
            .into_iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v))
            .collect();
        Self { map }
    }

    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn insert(&mut self, id: impl Into<String>, info: SuccessorInfo) {
        self.map.insert(id.into().to_ascii_lowercase(), info);
    }
}

impl SuccessorLookup for MapSuccessorLookup {
    fn successor_for(&self, id: &str) -> Option<SuccessorInfo> {
        self.map.get(&id.to_ascii_lowercase()).cloned()
    }
}

/// Convenience alias so `HashMap<String, SuccessorInfo>` can be passed directly
/// where a lookup is expected without wrapping.
impl SuccessorLookup for HashMap<String, SuccessorInfo> {
    fn successor_for(&self, id: &str) -> Option<SuccessorInfo> {
        self.get(&id.to_ascii_lowercase()).cloned().or_else(|| {
            // Also try exact key for callers that already normalized.
            self.get(id).cloned()
        })
    }
}

// ---------------------------------------------------------------------------
// Pure report generation (offline-testable)
// ---------------------------------------------------------------------------

/// Generate a migration report for a set of installed mods.
///
/// `loader` is the instance loader (`fabric`, `neoforge`, `forge`, `quilt`,
/// `vanilla`). It is forwarded to the [`ModrinthChecker`] so Modrinth mods are
/// filtered by loader; other content types ignore it.
///
/// The `successor_lookup` seam is checked first, so a known replacement
/// outranks the Modrinth liveness signal.
pub async fn generate_migration_report(
    instance_id: &str,
    source_version: &str,
    target_version: &str,
    loader: &str,
    mods: &[InstalledMod],
    checker: &dyn ModrinthChecker,
    successor_lookup: &dyn SuccessorLookup,
) -> MigrationReport {
    let mut warnings = Vec::new();
    if mods.is_empty() {
        warnings.push("Instance has no installed content to migrate.".to_string());
    }
    if source_version == target_version {
        warnings.push(format!(
            "Source and target are both {source_version}; report is a no-op check."
        ));
    }

    let mut entries = Vec::with_capacity(mods.len());

    for m in mods {
        let entry = classify_one(m, target_version, loader, checker, successor_lookup).await;
        entries.push(entry);
    }

    // Deterministic order for the UI.
    entries.sort_by(|a, b| a.filename.cmp(&b.filename));

    let summary = summarize(&entries);
    let verdict = derive_verdict(&summary);

    MigrationReport {
        instance_id: instance_id.to_string(),
        source_version: source_version.to_string(),
        target_version: target_version.to_string(),
        loader: loader.to_string(),
        summary,
        verdict,
        mods: entries,
        warnings,
    }
}

async fn classify_one(
    m: &InstalledMod,
    target_version: &str,
    loader: &str,
    checker: &dyn ModrinthChecker,
    successor_lookup: &dyn SuccessorLookup,
) -> ModMigrationEntry {
    let display_name = m
        .registry_id
        .clone()
        .or_else(|| m.modrinth_id.clone())
        .unwrap_or_else(|| m.filename.clone());

    // 1) Superseded: curator seam, checked before any network.
    if let Some(successor) = lookup_successor(m, successor_lookup) {
        return ModMigrationEntry {
            filename: m.filename.clone(),
            display_name,
            modrinth_id: m.modrinth_id.clone(),
            registry_id: m.registry_id.clone(),
            content_type: m.content_type.clone(),
            installed_version: m.version.clone(),
            status: MigrationStatus::Superseded,
            unclassifiable_reason: None,
            last_updated: None,
            has_target_build: Some(false),
            successor: Some(successor),
            target_build: None,
            error_code: None,
            error_message: None,
        };
    }

    // 2) Unclassifiable: no Modrinth identity at all.
    let Some(project_id) = m
        .modrinth_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        let reason = classify_unclassifiable(m);
        return ModMigrationEntry {
            filename: m.filename.clone(),
            display_name,
            modrinth_id: m.modrinth_id.clone(),
            registry_id: m.registry_id.clone(),
            content_type: m.content_type.clone(),
            installed_version: m.version.clone(),
            status: MigrationStatus::Unclassifiable,
            unclassifiable_reason: Some(reason),
            last_updated: None,
            has_target_build: None,
            successor: None,
            target_build: None,
            error_code: None,
            error_message: None,
        };
    };

    // 3) Check Modrinth for target support. Any LauncherError becomes Unknown.
    match checker
        .check_target_support(project_id, target_version, loader, &m.content_type)
        .await
    {
        Err(err) => ModMigrationEntry {
            filename: m.filename.clone(),
            display_name,
            modrinth_id: m.modrinth_id.clone(),
            registry_id: m.registry_id.clone(),
            content_type: m.content_type.clone(),
            installed_version: m.version.clone(),
            status: MigrationStatus::Unknown,
            unclassifiable_reason: None,
            last_updated: None,
            has_target_build: None,
            successor: None,
            target_build: None,
            error_code: Some(err.code()),
            error_message: Some(err.to_string()),
        },
        Ok(info) => {
            if info.has_target_build {
                ModMigrationEntry {
                    filename: m.filename.clone(),
                    display_name,
                    modrinth_id: m.modrinth_id.clone(),
                    registry_id: m.registry_id.clone(),
                    content_type: m.content_type.clone(),
                    installed_version: m.version.clone(),
                    status: MigrationStatus::Ready,
                    unclassifiable_reason: None,
                    last_updated: info.last_updated,
                    has_target_build: Some(true),
                    successor: None,
                    target_build: info.target_build,
                    error_code: None,
                    error_message: None,
                }
            } else if is_abandoned(info.last_updated.as_deref()) {
                ModMigrationEntry {
                    filename: m.filename.clone(),
                    display_name,
                    modrinth_id: m.modrinth_id.clone(),
                    registry_id: m.registry_id.clone(),
                    content_type: m.content_type.clone(),
                    installed_version: m.version.clone(),
                    status: MigrationStatus::Abandoned,
                    unclassifiable_reason: None,
                    last_updated: info.last_updated,
                    has_target_build: Some(false),
                    successor: None,
                    target_build: None,
                    error_code: None,
                    error_message: None,
                }
            } else {
                ModMigrationEntry {
                    filename: m.filename.clone(),
                    display_name,
                    modrinth_id: m.modrinth_id.clone(),
                    registry_id: m.registry_id.clone(),
                    content_type: m.content_type.clone(),
                    installed_version: m.version.clone(),
                    status: MigrationStatus::NotYet,
                    unclassifiable_reason: None,
                    last_updated: info.last_updated,
                    has_target_build: Some(false),
                    successor: None,
                    target_build: None,
                    error_code: None,
                    error_message: None,
                }
            }
        }
    }
}

fn lookup_successor(m: &InstalledMod, lookup: &dyn SuccessorLookup) -> Option<SuccessorInfo> {
    // Try Modrinth id first, then registry id, then normalized filename.
    // The curator map can choose whatever key it curates; trying a few
    // maximizes the chance a manually-maintained entry matches without
    // requiring the report caller to know which identifier the curator used.
    for key in [
        m.modrinth_id.as_deref(),
        m.registry_id.as_deref(),
        Some(m.filename.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(info) = lookup.successor_for(trimmed) {
            return Some(info);
        }
        // Also try lowercased filename without extension as a convenience.
        if key == m.filename {
            if let Some(stem) = std::path::Path::new(&m.filename)
                .file_stem()
                .and_then(|s| s.to_str())
            {
                if let Some(info) = lookup.successor_for(stem) {
                    return Some(info);
                }
            }
        }
    }
    None
}

fn classify_unclassifiable(m: &InstalledMod) -> UnclassifiableReason {
    if m.registry_id.is_some() {
        return UnclassifiableReason::CuratedOnly;
    }
    let src = m.source.trim().to_ascii_lowercase();
    if src.contains("manual")
        || src.contains("local")
        || src == "modrinth_raw" && m.modrinth_id.is_none()
        || src.is_empty()
    {
        // `modrinth_raw` without an id is effectively manual.
        return UnclassifiableReason::Manual;
    }
    if m.registry_id.is_none() && m.modrinth_id.is_none() {
        return UnclassifiableReason::Manual;
    }
    UnclassifiableReason::Other
}

fn is_abandoned(last_updated: Option<&str>) -> bool {
    let Some(s) = last_updated else {
        return false;
    };
    let Ok(updated) = chrono::DateTime::parse_from_rfc3339(s) else {
        return false;
    };
    let updated_utc = updated.with_timezone::<chrono::Utc>(&chrono::Utc);
    let age = chrono::Utc::now().signed_duration_since(updated_utc);
    age.num_days() >= ABANDONED_THRESHOLD_DAYS
}

fn summarize(entries: &[ModMigrationEntry]) -> MigrationSummary {
    let mut s = MigrationSummary {
        total: entries.len(),
        ..Default::default()
    };
    for e in entries {
        match e.status {
            MigrationStatus::Ready => s.ready += 1,
            MigrationStatus::NotYet => s.not_yet += 1,
            MigrationStatus::Abandoned => s.abandoned += 1,
            MigrationStatus::Superseded => s.superseded += 1,
            MigrationStatus::Unknown => s.unknown += 1,
            MigrationStatus::Unclassifiable => s.unclassifiable += 1,
        }
    }
    s
}

fn derive_verdict(summary: &MigrationSummary) -> MigrationVerdict {
    // Honesty first: unknowns mean the report is incomplete. Even if other
    // entries are blocked, we must not present a definitive "blocked" headline
    // when we could not even check some mods.
    if summary.unknown > 0 {
        return MigrationVerdict::Unknown;
    }
    if summary.abandoned > 0 || summary.superseded > 0 {
        return MigrationVerdict::Blocked;
    }
    if summary.unclassifiable > 0 {
        return MigrationVerdict::NeedsReview;
    }
    if summary.not_yet > 0 {
        return MigrationVerdict::NotYet;
    }
    MigrationVerdict::Ready
}

// ---------------------------------------------------------------------------
// Live checker — uses the existing helpers in crate::modrinth
// ---------------------------------------------------------------------------

fn modrinth_project_type(content_type: &str) -> &'static str {
    match content_type {
        "resourcepack" | "resourcepacks" => "resourcepack",
        "shader" | "shaders" | "shaderpack" | "shaderpacks" => "shader",
        "datapack" | "datapacks" => "datapack",
        "world" | "worlds" => "modpack",
        _ => "mod",
    }
}

/// Live Modrinth checker that delegates to the existing helpers in
/// `crate::modrinth` rather than building a parallel HTTP path.
///
/// It respects the `modrinth_enabled` / `network_modrinth_enabled` gates via
/// [`crate::modrinth::ModrinthService::check_enabled`]; a disabled gate maps
/// to `Err` and thus to `Unknown`, never to `Abandoned`.
pub struct LiveModrinthChecker {
    ctx: Ctx,
}

impl LiveModrinthChecker {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl ModrinthChecker for LiveModrinthChecker {
    async fn check_target_support(
        &self,
        project_id: &str,
        target_version: &str,
        loader: &str,
        content_type: &str,
    ) -> Result<SupportInfo, LauncherError> {
        // Gate — mirrors ModrinthService::check_enabled via crate::db directly
        // to avoid holding a Connection across an await point longer than needed.
        {
            let conn = crate::db::local_state_connection(&self.ctx.paths.local_state_db())
                .map_err(|e| LauncherError::Generic {
                    code: "ERR_LOCAL_STATE_FAILED".into(),
                    message: e.to_string(),
                })?;
            if !crate::db::is_network_enabled(&conn, "network_modrinth_enabled") {
                return Err(LauncherError::Generic {
                    code: "ERR_NETWORK_DISABLED".into(),
                    message: "Modrinth catalog API is disabled in Privacy settings.".into(),
                });
            }
            // Call the real gate rather than a copy of it: a replicated
            // opt-in check is one that silently diverges the day the original
            // changes, and this one decides whether we talk to the network.
            crate::modrinth::require_modrinth_enabled(&conn)?;
        }

        let project_type = modrinth_project_type(content_type);
        // Build a dummy instance row so the existing helper
        // `list_raw_modrinth_versions_http` can filter by game version + loader
        // without us rebuilding the URL ourselves.
        let dummy = crate::models::InstanceRow {
            instance_id: "migration-check".to_string(),
            name: "migration-check".to_string(),
            minecraft_version: target_version.to_string(),
            loader: loader.to_string(),
            loader_version: String::new(),
            is_modpack: false,
            is_locked: false,
            last_launched_at: None,
            jvm_memory_mb: 4096,
            jvm_memory_mode: "auto".to_string(),
            jvm_gc: "auto".to_string(),
            jvm_custom_args: String::new(),
            jvm_always_pre_touch: false,
            created_at: chrono::Utc::now().to_rfc3339(),
            java_path: None,
            java_incompatible_override: false,
            icon_path: None,
            launch_mode_override: "auto".to_string(),
            import_source: None,
        };

        let candidates = crate::modrinth::list_raw_modrinth_versions_http(
            Some(&dummy),
            project_id,
            Some(project_type),
        )
        .await?;

        let has_target_build = !candidates.is_empty();

        let target_build = if has_target_build {
            // Same selection the raw-install update path effectively makes:
            // `list_raw_modrinth_versions_http` pre-sorts stable-first,
            // newest-first, and `select_raw_modrinth_candidate` with no
            // requested version takes the head. Naming the build here (rather
            // than in a second lookup pass) guarantees "ready" and "which
            // build" can never disagree.
            candidates.first().map(|c| TargetBuildInfo {
                version_id: c.version_id.clone(),
                version_number: c.version.clone(),
                filename: c.filename.clone(),
                download_url: c.download_url.clone(),
                sha1: c.sha1.clone(),
                sha512: c.sha512.clone(),
                size: c.size,
                is_prerelease: c.is_prerelease,
            })
        } else {
            None
        };

        let last_updated = if has_target_build {
            None
        } else {
            // Fetch project metadata for staleness. A failure here does not
            // hide the "no target build" fact — we degrade to `not yet`.
            // Use the service facade so the connection is not held across the
            // await (the service opens and drops the connection internally).
            let svc = crate::modrinth::ModrinthService::new(self.ctx.clone());
            match svc.fetch_project_full(project_id).await {
                Ok(proj) => proj.source_updated_at,
                Err(_) => None,
            }
        };

        Ok(SupportInfo {
            has_target_build,
            last_updated,
            target_build,
        })
    }
}

// ---------------------------------------------------------------------------
// MigrationService — Ctx-owned facade for the desktop/CLI adapters
// ---------------------------------------------------------------------------

/// Core-owned service that produces migration reports for real instances.
///
/// The service handles manifest I/O and Modrinth identity enrichment; the pure
/// classification logic remains in [`generate_migration_report`] so it stays
/// testable without a filesystem or network.
#[derive(Clone)]
pub struct MigrationService {
    ctx: Ctx,
    successor_lookup: std::sync::Arc<dyn SuccessorLookup>,
}

impl MigrationService {
    pub fn new(ctx: Ctx) -> Self {
        Self {
            ctx,
            successor_lookup: std::sync::Arc::new(NoopSuccessorLookup),
        }
    }

    pub fn with_successor_lookup(mut self, lookup: std::sync::Arc<dyn SuccessorLookup>) -> Self {
        self.successor_lookup = lookup;
        self
    }

    /// Generate a report for an existing instance on disk.
    ///
    /// Reads the instance manifest via the canonical
    /// [`crate::helpers::read_manifest`] (which heals legacy fields), collects
    /// every content array, optionally enriches `registry_id`-only entries via
    /// `registry.db`, and classifies each entry against `target_version`.
    pub async fn report_for_instance(
        &self,
        instance_id: &str,
        target_version: &str,
    ) -> LauncherResult<MigrationReport> {
        let manifest_path = self.ctx.paths.instance_manifest(instance_id)?;
        let manifest = crate::helpers::read_manifest(&manifest_path)?;

        let source_version = manifest.minecraft_version.clone();
        let loader = manifest.loader.clone();

        // Collect every content array — an instance is more than just `mods`.
        let mut mods: Vec<InstalledMod> = Vec::new();
        mods.extend(manifest.mods.clone());
        mods.extend(manifest.resourcepacks.clone());
        mods.extend(manifest.shaders.clone());
        mods.extend(manifest.datapacks.clone());
        mods.extend(manifest.worlds.clone());

        // Optional enrichment: if an entry has a registry_id but no modrinth_id,
        // look up the registry row's `modrinth_id`. This is best-effort — a
        // missing or unreadable registry degrades to no enrichment.
        enrich_with_registry(&self.ctx, &mut mods);

        let checker = LiveModrinthChecker::new(self.ctx.clone());
        let report = generate_migration_report(
            instance_id,
            &source_version,
            target_version,
            &loader,
            &mods,
            &checker,
            self.successor_lookup.as_ref(),
        )
        .await;

        Ok(report)
    }

    /// Generate a report from an already-loaded manifest (no filesystem I/O
    /// beyond the checker).
    pub async fn report_for_manifest(
        &self,
        instance_id: &str,
        manifest: &crate::models::InstanceManifest,
        target_version: &str,
    ) -> MigrationReport {
        let mut mods: Vec<InstalledMod> = Vec::new();
        mods.extend(manifest.mods.clone());
        mods.extend(manifest.resourcepacks.clone());
        mods.extend(manifest.shaders.clone());
        mods.extend(manifest.datapacks.clone());
        mods.extend(manifest.worlds.clone());

        enrich_with_registry(&self.ctx, &mut mods);

        let checker = LiveModrinthChecker::new(self.ctx.clone());
        generate_migration_report(
            instance_id,
            &manifest.minecraft_version,
            target_version,
            &manifest.loader,
            &mods,
            &checker,
            self.successor_lookup.as_ref(),
        )
        .await
    }
}

pub(crate) fn enrich_with_registry(ctx: &Ctx, mods: &mut [InstalledMod]) {
    let registry_ids: Vec<String> = mods
        .iter()
        .filter(|m| m.modrinth_id.is_none())
        .filter_map(|m| m.registry_id.clone())
        .collect();
    if registry_ids.is_empty() {
        return;
    }
    let Ok(reg) =
        crate::registry::RegistryService::new(ctx.clone()).get_items_by_ids(&registry_ids)
    else {
        return;
    };
    for m in mods.iter_mut() {
        if m.modrinth_id.is_some() {
            continue;
        }
        let Some(rid) = m.registry_id.as_deref() else {
            continue;
        };
        if let Some(item) = reg.get(rid) {
            if let Some(mid) = item
                .modrinth_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                m.modrinth_id = Some(mid.to_string());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers for tests and adapters
// ---------------------------------------------------------------------------

/// Build a minimal `InstalledMod` for tests / examples.
#[cfg(test)]
pub(crate) fn test_mod(
    filename: &str,
    modrinth_id: Option<&str>,
    registry_id: Option<&str>,
    source: &str,
) -> InstalledMod {
    InstalledMod {
        update_pinned: false,
        pack_managed: false,
        installed_as_dependency: false,
        filename: filename.to_string(),
        registry_id: registry_id.map(str::to_string),
        modrinth_id: modrinth_id.map(str::to_string),
        source: source.to_string(),
        source_url: None,
        version: Some("1.0.0".to_string()),
        sha256: "aa".repeat(32),
        installed_at: chrono::Utc::now().to_rfc3339(),
        java_packages: Vec::new(),
        mod_jar_id: None,
        depends_on: Vec::new(),
        optional_deps: Vec::new(),
        incompatible_deps: Vec::new(),
        provided_mod_ids: Vec::new(),
        enabled: true,
        content_type: "mod".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests (offline — injected fake checker)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct MockChecker {
        /// project_id -> Result<SupportInfo>
        map: HashMap<String, Result<SupportInfo, LauncherError>>,
    }

    impl MockChecker {
        fn new() -> Self {
            Self {
                map: HashMap::new(),
            }
        }

        fn with_ready(mut self, project_id: &str) -> Self {
            self.map.insert(
                project_id.to_ascii_lowercase(),
                Ok(SupportInfo {
                    has_target_build: true,
                    last_updated: None,
                    target_build: None,
                }),
            );
            self
        }

        fn with_not_yet(mut self, project_id: &str, last_updated: Option<&str>) -> Self {
            self.map.insert(
                project_id.to_ascii_lowercase(),
                Ok(SupportInfo {
                    has_target_build: false,
                    last_updated: last_updated.map(str::to_string),
                    target_build: None,
                }),
            );
            self
        }

        fn with_abandoned(mut self, project_id: &str) -> Self {
            // Updated long ago.
            let old = (chrono::Utc::now() - chrono::Duration::days(400)).to_rfc3339();
            self.map.insert(
                project_id.to_ascii_lowercase(),
                Ok(SupportInfo {
                    has_target_build: false,
                    last_updated: Some(old),
                    target_build: None,
                }),
            );
            self
        }

        fn with_failure(mut self, project_id: &str) -> Self {
            self.map.insert(
                project_id.to_ascii_lowercase(),
                Err(LauncherError::Generic {
                    code: "ERR_NETWORK".into(),
                    message: "simulated network failure".into(),
                }),
            );
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
            match self.map.get(&project_id.to_ascii_lowercase()) {
                Some(Ok(info)) => Ok(info.clone()),
                Some(Err(e)) => Err(match e {
                    LauncherError::Generic { code, message } => LauncherError::Generic {
                        code: code.clone(),
                        message: message.clone(),
                    },
                    other => LauncherError::Generic {
                        code: other.code(),
                        message: other.to_string(),
                    },
                }),
                None => Err(LauncherError::Generic {
                    code: "ERR_NOT_MOCKED".into(),
                    message: format!("no mock for {project_id}"),
                }),
            }
        }
    }

    fn noop() -> NoopSuccessorLookup {
        NoopSuccessorLookup
    }

    fn target() -> &'static str {
        "1.21.4"
    }

    fn source() -> &'static str {
        "1.21.1"
    }

    #[tokio::test]
    async fn lookup_failure_is_unknown_not_abandoned() {
        // A Modrinth rate-limit must not make the report claim the mod is dead.
        let mods = vec![test_mod(
            "fancy.jar",
            Some("fancy-id"),
            None,
            "modrinth_raw",
        )];
        let checker = MockChecker::new().with_failure("fancy-id");

        let report = generate_migration_report(
            "inst",
            source(),
            target(),
            "fabric",
            &mods,
            &checker,
            &noop(),
        )
        .await;

        assert_eq!(report.mods.len(), 1);
        assert_eq!(report.mods[0].status, MigrationStatus::Unknown);
        assert_eq!(report.mods[0].error_code.as_deref(), Some("ERR_NETWORK"));
        // Unknown outranks everything — verdict must be Unknown even though the
        // only other signal would have been "blocked" if we had misclassified.
        assert_eq!(report.verdict, MigrationVerdict::Unknown);
        assert_eq!(report.summary.unknown, 1);
        assert_eq!(report.summary.abandoned, 0);
    }

    #[tokio::test]
    async fn all_ready_reports_ready() {
        let mods = vec![
            test_mod("a.jar", Some("a"), None, "modrinth_raw"),
            test_mod("b.jar", Some("b"), None, "modrinth_raw"),
            test_mod("c.jar", Some("c"), None, "modrinth_raw"),
        ];
        let checker = MockChecker::new()
            .with_ready("a")
            .with_ready("b")
            .with_ready("c");

        let report = generate_migration_report(
            "inst",
            source(),
            target(),
            "fabric",
            &mods,
            &checker,
            &noop(),
        )
        .await;

        assert_eq!(report.summary.total, 3);
        assert_eq!(report.summary.ready, 3);
        assert_eq!(report.summary.not_yet, 0);
        assert_eq!(report.summary.unknown, 0);
        assert_eq!(report.summary.unclassifiable, 0);
        assert_eq!(report.verdict, MigrationVerdict::Ready);
        for m in &report.mods {
            assert_eq!(m.status, MigrationStatus::Ready);
        }
    }

    #[tokio::test]
    async fn unclassifiable_mods_are_visible_not_dropped() {
        let mods = vec![
            test_mod("manual.jar", None, None, "manual_drag_drop"),
            test_mod("curated.jar", None, Some("sodium"), "registry"),
            test_mod("ready.jar", Some("ready-id"), None, "modrinth_raw"),
        ];
        let checker = MockChecker::new().with_ready("ready-id");

        let report = generate_migration_report(
            "inst",
            source(),
            target(),
            "fabric",
            &mods,
            &checker,
            &noop(),
        )
        .await;

        // All three must appear.
        assert_eq!(report.mods.len(), 3);
        assert_eq!(report.summary.total, 3);

        let manual = report
            .mods
            .iter()
            .find(|m| m.filename == "manual.jar")
            .unwrap();
        assert_eq!(manual.status, MigrationStatus::Unclassifiable);
        assert_eq!(
            manual.unclassifiable_reason,
            Some(UnclassifiableReason::Manual)
        );

        let curated = report
            .mods
            .iter()
            .find(|m| m.filename == "curated.jar")
            .unwrap();
        assert_eq!(curated.status, MigrationStatus::Unclassifiable);
        assert_eq!(
            curated.unclassifiable_reason,
            Some(UnclassifiableReason::CuratedOnly)
        );

        let ready = report
            .mods
            .iter()
            .find(|m| m.filename == "ready.jar")
            .unwrap();
        assert_eq!(ready.status, MigrationStatus::Ready);

        assert_eq!(report.summary.unclassifiable, 2);
        assert_eq!(report.summary.ready, 1);
        // Presence of unclassifiables needs review — not Ready even though the
        // only classifiable entry is ready.
        assert_eq!(report.verdict, MigrationVerdict::NeedsReview);
    }

    #[tokio::test]
    async fn superseded_outranks_modrinth_signal() {
        let mods = vec![test_mod("old.jar", Some("old-id"), None, "modrinth_raw")];
        let checker = MockChecker::new().with_ready("old-id");
        let mut succ_map = HashMap::new();
        succ_map.insert(
            "old-id".to_string(),
            SuccessorInfo {
                replacement_id: "new-id".to_string(),
                replacement_name: Some("New Mod".to_string()),
                reason: Some("Rewritten for 1.21.4".to_string()),
            },
        );
        let lookup = MapSuccessorLookup::new(succ_map);

        let report = generate_migration_report(
            "inst",
            source(),
            target(),
            "fabric",
            &mods,
            &checker,
            &lookup,
        )
        .await;

        assert_eq!(report.mods[0].status, MigrationStatus::Superseded);
        assert!(report.mods[0].successor.is_some());
        assert_eq!(report.summary.superseded, 1);
        assert_eq!(report.verdict, MigrationVerdict::Blocked);
    }

    #[tokio::test]
    async fn abandoned_vs_not_yet_threshold() {
        let recent = (chrono::Utc::now() - chrono::Duration::days(10)).to_rfc3339();
        let old = (chrono::Utc::now() - chrono::Duration::days(400)).to_rfc3339();

        let mods = vec![
            test_mod("recent.jar", Some("recent-id"), None, "modrinth_raw"),
            test_mod("old.jar", Some("old-id"), None, "modrinth_raw"),
            test_mod("no-date.jar", Some("no-date-id"), None, "modrinth_raw"),
        ];
        let mut checker = MockChecker::new();
        checker.map.insert(
            "recent-id".to_string(),
            Ok(SupportInfo {
                has_target_build: false,
                last_updated: Some(recent),
                target_build: None,
            }),
        );
        checker.map.insert(
            "old-id".to_string(),
            Ok(SupportInfo {
                has_target_build: false,
                last_updated: Some(old),
                target_build: None,
            }),
        );
        checker.map.insert(
            "no-date-id".to_string(),
            Ok(SupportInfo {
                has_target_build: false,
                last_updated: None,
                target_build: None,
            }),
        );

        let report = generate_migration_report(
            "inst",
            source(),
            target(),
            "fabric",
            &mods,
            &checker,
            &noop(),
        )
        .await;

        let recent_entry = report
            .mods
            .iter()
            .find(|m| m.filename == "recent.jar")
            .unwrap();
        assert_eq!(recent_entry.status, MigrationStatus::NotYet);

        let old_entry = report
            .mods
            .iter()
            .find(|m| m.filename == "old.jar")
            .unwrap();
        assert_eq!(old_entry.status, MigrationStatus::Abandoned);

        // Missing timestamp is conservatively not yet, not abandoned.
        let nodate = report
            .mods
            .iter()
            .find(|m| m.filename == "no-date.jar")
            .unwrap();
        assert_eq!(nodate.status, MigrationStatus::NotYet);

        assert_eq!(report.summary.not_yet, 2);
        assert_eq!(report.summary.abandoned, 1);
        assert_eq!(report.verdict, MigrationVerdict::Blocked);
    }

    #[tokio::test]
    async fn unknown_outranks_blocked_for_honesty() {
        let mods = vec![
            test_mod("abandoned.jar", Some("ab-id"), None, "modrinth_raw"),
            test_mod("unknown.jar", Some("unk-id"), None, "modrinth_raw"),
        ];
        let checker = MockChecker::new()
            .with_abandoned("ab-id")
            .with_failure("unk-id");

        let report = generate_migration_report(
            "inst",
            source(),
            target(),
            "fabric",
            &mods,
            &checker,
            &noop(),
        )
        .await;

        assert_eq!(report.summary.abandoned, 1);
        assert_eq!(report.summary.unknown, 1);
        // Unknown is the honest verdict even though something is also blocked.
        assert_eq!(report.verdict, MigrationVerdict::Unknown);
    }

    #[tokio::test]
    async fn summary_counts_match_entries() {
        let mods = vec![
            test_mod("a.jar", Some("a"), None, "modrinth_raw"),
            test_mod("b.jar", Some("b"), None, "modrinth_raw"),
            test_mod("c.jar", None, None, "manual"),
        ];
        let checker = MockChecker::new().with_ready("a").with_not_yet("b", None);

        let report = generate_migration_report(
            "inst",
            source(),
            target(),
            "fabric",
            &mods,
            &checker,
            &noop(),
        )
        .await;

        assert_eq!(report.summary.total, report.mods.len());
        assert_eq!(
            report.summary.ready
                + report.summary.not_yet
                + report.summary.abandoned
                + report.summary.superseded
                + report.summary.unknown
                + report.summary.unclassifiable,
            report.summary.total
        );
    }
}
