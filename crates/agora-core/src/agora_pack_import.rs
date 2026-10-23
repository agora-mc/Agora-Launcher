//! Installing the mods of an Agora pack file (`.agora-pack.json`), exactly and
//! without silent drops.
//!
//! A pack file lists mods the sender had installed, each with the exact file
//! (`filename`, `version`) and the SHA-256 of its bytes. Importing it must
//! reproduce that file or say why it could not:
//!
//! - **Exact.** The entry's pinned file is installed, never "the latest". When
//!   the pack pins a SHA-256 the downloaded bytes must match it, or the mod is
//!   not installed at all.
//! - **Honest.** Every entry ends as installed, failed (with a reason) or
//!   needing an action from the user (e.g. Modrinth is off). Nothing is skipped
//!   quietly and no error is swallowed.
//! - **Retryable.** A needs-action entry carries what is needed to try it again
//!   after the user acts, without re-importing the pack.
//!
//! The network and install steps sit behind [`PackBackend`] so the decision
//! logic can be tested without a network; [`CoreBackend`] is the real one.

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::models::{InstanceManifest, ModVersionCandidate, PackOrigin, PackPlatform};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The note shown with a Modrinth-dependent entry while Modrinth is off. Same
/// privacy wording as the Browse offer to search Modrinth.
pub const MODRINTH_OFF_NOTE: &str =
    "Needs Modrinth, which is off. Turning it on sends requests for this mod to Modrinth.";

/// One entry of the pack's `mods` array.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgoraPackEntry {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub registry_id: Option<String>,
    #[serde(default)]
    pub modrinth_id: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

impl AgoraPackEntry {
    fn display_name(&self) -> String {
        let name = self.filename.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        non_empty(&self.registry_id)
            .or_else(|| non_empty(&self.modrinth_id))
            .unwrap_or("(unnamed entry)")
            .to_string()
    }
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// The `instance` object of a pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgoraPackInstance {
    pub id: Option<String>,
    pub name: Option<String>,
    pub minecraft_version: String,
    pub loader: String,
    pub loader_version: String,
}

/// A `mods` element: a usable entry, or one that could not even be read (which
/// is reported, not dropped).
#[derive(Debug, Clone)]
pub enum PackEntryParse {
    Entry(AgoraPackEntry),
    Invalid { name: String, reason: String },
}

#[derive(Debug, Clone)]
pub struct ParsedAgoraPack {
    pub instance: AgoraPackInstance,
    pub mods: Vec<PackEntryParse>,
}

/// Largest pack file Agora will read, and most mod entries it will import from
/// one. Real packs are far below both; the cap keeps a hostile or broken file
/// from making the importer read or loop without end.
pub const MAX_PACK_FILE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_PACK_ENTRIES: usize = 5_000;

/// The refusal for a pack file over [`MAX_PACK_FILE_BYTES`].
pub fn pack_too_big_error() -> LauncherError {
    parse_error("This pack file is larger than 5 MB, which is more than Agora will import.")
}

fn parse_error(message: &str) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_PACK_PARSE".into(),
        message: message.into(),
    }
}

/// Parse a `.agora-pack.json` document. The instance block must be complete;
/// each mod entry is parsed on its own so one bad entry cannot hide the rest.
pub fn parse_agora_pack(text: &str) -> LauncherResult<ParsedAgoraPack> {
    if text.len() as u64 > MAX_PACK_FILE_BYTES {
        return Err(pack_too_big_error());
    }
    let pack: serde_json::Value =
        serde_json::from_str(text).map_err(|_| parse_error("Failed to parse agora-pack JSON."))?;
    let inst = pack
        .get("instance")
        .ok_or_else(|| parse_error("agora-pack missing 'instance' object."))?;
    let field = |key: &str| {
        inst.get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let required =
        |key: &str| field(key).ok_or_else(|| parse_error(&format!("agora-pack missing {key}.")));
    let instance = AgoraPackInstance {
        id: field("id"),
        name: field("name"),
        minecraft_version: required("minecraft_version")?,
        loader: required("loader")?,
        loader_version: required("loader_version")?,
    };

    let mods = match pack.get("mods") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(entries)) if entries.len() > MAX_PACK_ENTRIES => {
            return Err(parse_error(&format!(
                "This pack lists {} mods; Agora imports at most {MAX_PACK_ENTRIES} from one \
                 pack file.",
                entries.len()
            )))
        }
        Some(serde_json::Value::Array(entries)) => entries
            .iter()
            .enumerate()
            .map(
                |(index, raw)| match serde_json::from_value::<AgoraPackEntry>(raw.clone()) {
                    Ok(entry) => PackEntryParse::Entry(entry),
                    Err(error) => PackEntryParse::Invalid {
                        name: format!("mods[{index}]"),
                        reason: format!("This entry is not a readable mod record ({error})."),
                    },
                },
            )
            .collect(),
        Some(_) => return Err(parse_error("agora-pack 'mods' must be a list.")),
    };
    Ok(ParsedAgoraPack { instance, mods })
}

/// The name a pack's instance gets: its name, else its id, else a fallback.
pub fn pack_display_name(instance: &AgoraPackInstance) -> String {
    instance
        .name
        .clone()
        .or_else(|| instance.id.clone())
        .unwrap_or_else(|| "imported-pack".to_string())
}

/// Where a pack entry would be fetched from, judged from the pack and this
/// install's settings alone. Nothing is downloaded to decide it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannedSource {
    /// The curated catalog (the pack gave a registry id this install knows).
    Curated,
    /// Modrinth, by project id, as an uncurated file.
    Modrinth,
    /// No source this install can use. The entry's `reason` says why.
    CannotGet,
}

/// One entry of an import plan. Says what the import will try, and what the
/// user must do first; it is not a result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedEntry {
    /// The filename, or `mods[n]` for an entry that could not be read.
    pub name: String,
    /// The catalog's name for the mod when known.
    pub display_name: Option<String>,
    pub source: PlannedSource,
    /// The pack pinned a valid SHA-256, so the download can be checked against
    /// the pack. `false` means it is checked only against its source's hash.
    pub checkable: bool,
    /// What needs action, or why the entry will not install. `None` when the
    /// entry is ready to be fetched.
    pub reason: Option<String>,
    /// Set when a user action (turning Modrinth on, Settings) would unblock it.
    pub action: Option<PackNeedsAction>,
}

/// What importing a pack would do, shown to the user before anything is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackImportPlan {
    /// What the new instance will be called ([`pack_display_name`]).
    pub pack_name: String,
    pub instance: AgoraPackInstance,
    pub entries: Vec<PlannedEntry>,
    /// SHA-256 (lowercase hex) of the exact file bytes that were reviewed. The
    /// import refuses any file whose bytes no longer hash to this.
    pub file_sha256: String,
    /// Size in bytes of the reviewed file.
    pub file_size: u64,
}

/// SHA-256 (lowercase hex) of a pack file's bytes.
pub fn pack_fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// The bytes of a pack file, parsed. Refuses an oversized file.
fn parse_pack_bytes(bytes: &[u8]) -> LauncherResult<ParsedAgoraPack> {
    if bytes.len() as u64 > MAX_PACK_FILE_BYTES {
        return Err(pack_too_big_error());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| parse_error("Failed to parse agora-pack JSON: the file is not UTF-8."))?;
    parse_agora_pack(text)
}

/// Parse the exact bytes the user reviewed, and only those. The import is
/// refused unless `reviewed_sha256` is present and the bytes hash to it, so a
/// file changed after review is never installed unreviewed.
pub fn parse_reviewed_agora_pack(
    bytes: &[u8],
    reviewed_sha256: Option<&str>,
) -> LauncherResult<ParsedAgoraPack> {
    if bytes.len() as u64 > MAX_PACK_FILE_BYTES {
        return Err(pack_too_big_error());
    }
    let Some(reviewed) = reviewed_sha256 else {
        return Err(LauncherError::Generic {
            code: "ERR_PACK_NOT_REVIEWED".into(),
            message: "Review the pack before importing it.".into(),
        });
    };
    if !pack_fingerprint(bytes).eq_ignore_ascii_case(reviewed.trim()) {
        return Err(LauncherError::Generic {
            code: "ERR_PACK_CHANGED".into(),
            message: "The pack file changed since you reviewed it. Review it again.".into(),
        });
    }
    parse_pack_bytes(bytes)
}

/// [`plan_agora_pack_import`] for the exact bytes of a pack file: also records
/// the fingerprint the import will be held to.
pub fn plan_agora_pack_bytes(ctx: &Ctx, bytes: &[u8]) -> LauncherResult<PackImportPlan> {
    let pack = parse_pack_bytes(bytes)?;
    let mut plan = plan_agora_pack_import(ctx, &pack);
    plan.file_sha256 = pack_fingerprint(bytes);
    plan.file_size = bytes.len() as u64;
    Ok(plan)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackEntryStatus {
    Installed,
    Failed,
    NeedsAction,
}

/// What the user can do about a needs-action entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackNeedsAction {
    /// Modrinth is off; turning it on lets the entry install.
    EnableModrinth,
    /// A source or network switch in Settings is off.
    OpenSettings,
}

/// How one pack entry ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackEntryOutcome {
    /// What to call the entry in messages: its filename.
    pub name: String,
    pub status: PackEntryStatus,
    /// Installed only: the bytes matched the SHA-256 the pack pinned. `false`
    /// means the pack pinned none, so the file could not be checked against it.
    pub verified_against_pack: bool,
    /// Why it failed or what it needs; for an unverified install, the caveat.
    pub reason: Option<String>,
    pub action: Option<PackNeedsAction>,
    /// The catalog's name for the mod when known; `name` (the filename) otherwise.
    #[serde(default)]
    pub display_name: Option<String>,
    /// The original entry, handed back to retry a needs-action one.
    pub entry: AgoraPackEntry,
}

/// The result of installing a pack's mods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackImportReport {
    pub instance_id: String,
    pub instance_name: String,
    pub total: usize,
    pub installed: usize,
    pub outcomes: Vec<PackEntryOutcome>,
    /// Problems with the bookkeeping after the installs (e.g. the pack
    /// inventory could not be written). Installs are unaffected.
    pub warnings: Vec<String>,
}

/// True when the import installed nothing and every entry failed only for lack
/// of a connection. Such an import left nothing usable behind, so the caller
/// may remove the instance it made and let the user start again later.
pub fn nothing_installed_for_lack_of_connection(report: &PackImportReport) -> bool {
    report.installed == 0
        && !report.outcomes.is_empty()
        && report.outcomes.iter().all(|o| {
            o.status == PackEntryStatus::NeedsAction
                && o.action.is_none()
                && o.reason.as_deref() == Some(OFFLINE_NOTE)
        })
}

impl PackImportReport {
    fn from_outcomes(
        instance_id: &str,
        instance_name: &str,
        outcomes: Vec<PackEntryOutcome>,
        warnings: Vec<String>,
    ) -> Self {
        let installed = outcomes
            .iter()
            .filter(|o| o.status == PackEntryStatus::Installed)
            .count();
        Self {
            instance_id: instance_id.to_string(),
            instance_name: instance_name.to_string(),
            total: outcomes.len(),
            installed,
            outcomes,
            warnings,
        }
    }
}

// ---------------------------------------------------------------------------
// Backend
// ---------------------------------------------------------------------------

/// One installable file offered by a source, with the hashes to hold it to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackCandidate {
    pub version: String,
    pub filename: String,
    pub download_url: String,
    pub sha1: Option<String>,
    /// SHA-512 the source published (Modrinth does); checked as well as SHA-1.
    pub sha512: Option<String>,
    /// The SHA-256 the *source's own rules* expect (see
    /// [`expected_sha256_for_candidate`]); `None` when it published none.
    pub expected_sha256: Option<String>,
    pub content_type: String,
    pub registry_id: Option<String>,
    pub modrinth_id: Option<String>,
    /// Manifest `source` value: `registry` or `modrinth_raw`.
    pub source: String,
}

/// Why a lookup produced no candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    NeedsAction(PackNeedsAction, String),
    /// The source could not be reached (no connection, DNS, TLS, timeout).
    /// Nothing was written; the entry waits until the user can get online.
    Offline,
    Failed(String),
}

/// What an entry says when it could not be downloaded for lack of a connection.
pub const OFFLINE_NOTE: &str =
    "This mod needs to be downloaded. Couldn't connect: no internet, or the server can't be reached right now. Try again when you're online.";

/// Whether an error is a failure to reach the source at all. The shared HTTP
/// layer reports every send failure as `ERR_NETWORK`; that code does not by
/// itself prove the machine is offline (a timeout or TLS error has it too).
fn is_unreachable(error: &LauncherError) -> bool {
    error.code() == "ERR_NETWORK"
}

#[async_trait]
pub trait PackBackend: Send + Sync {
    /// Filenames already in the instance, so a retry installs only what is missing.
    fn installed_filenames(&self, instance_id: &str) -> Vec<String>;
    /// SHA-256 of a file already in the instance, `None` if it cannot be read.
    fn installed_sha256(&self, instance_id: &str, filename: &str) -> Option<String>;
    /// The catalog's name for a registry id, when it is known.
    fn display_name(&self, registry_id: &str) -> Option<String>;
    /// Whether this install's catalog has the registry id at all. Local only.
    /// An unreadable catalog is an error, not "not there".
    fn knows_registry_item(&self, registry_id: &str) -> Result<bool, String>;
    /// Why Modrinth may not be used for this import, `None` when it may. Reads
    /// local settings only; no request is made.
    fn modrinth_refusal(&self) -> Option<BackendError>;
    async fn registry_versions(
        &self,
        instance_id: &str,
        registry_id: &str,
    ) -> Result<Vec<PackCandidate>, BackendError>;
    async fn modrinth_versions(
        &self,
        instance_id: &str,
        project_id: &str,
    ) -> Result<Vec<PackCandidate>, BackendError>;
    /// Download, verify against the source's hash and `required_sha256`, and
    /// install. Nothing may be written unless every check passes.
    async fn install(
        &self,
        instance_id: &str,
        candidate: &PackCandidate,
        required_sha256: Option<&str>,
    ) -> Result<(), BackendError>;
}

/// A pack-pinned SHA-256: absent, or exactly 64 hex digits. Anything else is a
/// corrupt pin, and a corrupt pin must not be read as "no pin".
fn normalize_pin(raw: &Option<String>) -> Result<Option<String>, String> {
    // Absent means no pin. A blank pin is a damaged one and fails closed.
    let Some(value) = raw.as_deref().map(str::trim) else {
        return Ok(None);
    };
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(Some(value.to_ascii_lowercase()))
    } else {
        Err(
            "The pack's pinned SHA-256 for this file is malformed, so the file cannot be \
             checked and was not installed."
                .to_string(),
        )
    }
}

/// The candidate the pack pinned: the one with its filename, else its version.
/// Never "the latest": a missing pinned file is a failure, not a substitution.
fn select_pinned<'a>(
    candidates: &'a [PackCandidate],
    entry: &AgoraPackEntry,
) -> Option<&'a PackCandidate> {
    let filename = entry.filename.trim();
    if !filename.is_empty() {
        if let Some(found) = candidates.iter().find(|c| c.filename == filename) {
            return Some(found);
        }
    }
    let version = non_empty(&entry.version)?;
    candidates.iter().find(|c| c.version == version)
}

fn outcome(
    entry: &AgoraPackEntry,
    status: PackEntryStatus,
    reason: Option<String>,
    action: Option<PackNeedsAction>,
) -> PackEntryOutcome {
    PackEntryOutcome {
        name: entry.display_name(),
        status,
        verified_against_pack: false,
        reason,
        action,
        display_name: None,
        entry: entry.clone(),
    }
}

fn failed(entry: &AgoraPackEntry, reason: impl Into<String>) -> PackEntryOutcome {
    outcome(entry, PackEntryStatus::Failed, Some(reason.into()), None)
}

fn from_backend_error(entry: &AgoraPackEntry, error: BackendError) -> PackEntryOutcome {
    match error {
        BackendError::NeedsAction(action, note) => outcome(
            entry,
            PackEntryStatus::NeedsAction,
            Some(note),
            Some(action),
        ),
        // Waits for a connection, not for a setting: no action to take in Settings.
        BackendError::Offline => outcome(
            entry,
            PackEntryStatus::NeedsAction,
            Some(OFFLINE_NOTE.to_string()),
            None,
        ),
        BackendError::Failed(reason) => failed(entry, reason),
    }
}

async fn install_one(
    backend: &dyn PackBackend,
    instance_id: &str,
    entry: &AgoraPackEntry,
) -> PackEntryOutcome {
    let pin = match normalize_pin(&entry.sha256) {
        Ok(pin) => pin,
        Err(reason) => return failed(entry, reason),
    };

    // Registry id wins: the curated catalog is the trusted route. An entry with
    // only a Modrinth id is an uncurated install and goes through the raw path.
    let looked_up = if let Some(registry_id) = non_empty(&entry.registry_id) {
        backend.registry_versions(instance_id, registry_id).await
    } else if let Some(project_id) = non_empty(&entry.modrinth_id) {
        backend.modrinth_versions(instance_id, project_id).await
    } else {
        return failed(
            entry,
            "The pack records no catalog or Modrinth id for this file, so there is nowhere \
             to download it from.",
        );
    };
    let candidates = match looked_up {
        Ok(candidates) => candidates,
        Err(error) => return from_backend_error(entry, error),
    };

    let Some(candidate) = select_pinned(&candidates, entry) else {
        let pinned = non_empty(&entry.version)
            .map(|v| format!("version {v}"))
            .unwrap_or_else(|| entry.display_name());
        return failed(
            entry,
            format!(
                "The pinned file ({pinned}) is not offered by its source for this Minecraft \
                 version and loader ({} versions listed). Agora will not substitute another version.",
                candidates.len()
            ),
        );
    };
    if !crate::app_paths::is_safe_file_name(&candidate.filename) {
        return failed(
            entry,
            "The source offers a file with an unsafe name; it was not installed.",
        );
    }

    match backend
        .install(instance_id, candidate, pin.as_deref())
        .await
    {
        Ok(()) => {
            let mut done = outcome(entry, PackEntryStatus::Installed, None, None);
            done.name = candidate.filename.clone();
            if pin.is_some() {
                done.verified_against_pack = true;
            } else {
                done.reason = Some(
                    "The pack pinned no SHA-256, so this file could not be checked against the \
                     pack. It was checked against the hash its source published."
                        .to_string(),
                );
            }
            done
        }
        Err(error) => from_backend_error(entry, error),
    }
}

/// Run every entry to an outcome. Entries already present in the instance (a
/// retry) are reported installed without being downloaded again.
pub async fn install_pack_entries(
    backend: &dyn PackBackend,
    instance_id: &str,
    entries: Vec<PackEntryParse>,
) -> Vec<PackEntryOutcome> {
    let mut outcomes: Vec<PackEntryOutcome> = Vec::with_capacity(entries.len());
    // The same entry listed twice is fetched and installed once; every listing
    // still gets its outcome.
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for parsed in entries {
        match parsed {
            PackEntryParse::Invalid { name, reason } => outcomes.push(PackEntryOutcome {
                name,
                status: PackEntryStatus::Failed,
                verified_against_pack: false,
                reason: Some(reason),
                action: None,
                display_name: None,
                entry: AgoraPackEntry::default(),
            }),
            PackEntryParse::Entry(entry) => {
                let key = serde_json::to_string(&entry).unwrap_or_default();
                if let Some(&first) = seen.get(&key) {
                    let copy = outcomes[first].clone();
                    outcomes.push(copy);
                    continue;
                }
                let mut done = settle_entry(backend, instance_id, &entry).await;
                done.display_name =
                    non_empty(&entry.registry_id).and_then(|id| backend.display_name(id));
                seen.insert(key, outcomes.len());
                outcomes.push(done);
            }
        }
    }
    outcomes
}

/// Reason on an entry that was already in the instance with nothing to check
/// it against. Such a file is the user's, not the pack's.
const PRESENT_UNPINNED: &str = "Already installed.";

async fn settle_entry(
    backend: &dyn PackBackend,
    instance_id: &str,
    entry: &AgoraPackEntry,
) -> PackEntryOutcome {
    let filename = entry.filename.trim();
    let present = !filename.is_empty()
        && backend
            .installed_filenames(instance_id)
            .iter()
            .any(|f| f == filename);
    if !present {
        return install_one(backend, instance_id, entry).await;
    }
    let pin = match normalize_pin(&entry.sha256) {
        Ok(pin) => pin,
        Err(reason) => return failed(entry, reason),
    };
    let mut done = outcome(entry, PackEntryStatus::Installed, None, None);
    match pin {
        None => done.reason = Some(PRESENT_UNPINNED.into()),
        Some(pin) => match backend.installed_sha256(instance_id, filename) {
            Some(actual) if actual == pin => {
                done.verified_against_pack = true;
                done.reason = Some("Already installed (verified against the pack).".into());
            }
            Some(_) => {
                return failed(
                    entry,
                    "A file with this name is already in the instance but is not the file the \
                     pack pinned (its SHA-256 differs). It was left as it is.",
                )
            }
            None => {
                done.reason = Some(
                    "Already installed, but it could not be read to be checked against the \
                     pack SHA-256."
                        .into(),
                )
            }
        },
    }
    done
}

// ---------------------------------------------------------------------------
// Real backend
// ---------------------------------------------------------------------------

/// Which SHA-256 a curated candidate must match, by where it came from.
///
/// Depends on the candidate's own source, not the item's preferred strategy: an
/// item can resolve from a fallback, and a Modrinth-served file must not be
/// judged against a hash pinned for the GitHub release.
pub fn expected_sha256_for_candidate(
    item: &crate::registry::RegistryItem,
    candidate: &ModVersionCandidate,
) -> Option<String> {
    let strategy = candidate
        .source_strategy
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(item.download_strategy.as_str());
    let pinned = Some(item.sha256.trim()).filter(|h| !h.is_empty());
    match strategy {
        // Hand-pinned: the registry hash is the whole integrity story.
        "direct_hash" | "technic_pack" => pinned.map(str::to_string),
        // GitHub publishes a per-asset digest on newer releases and nothing on
        // older ones; the pinned hash describes one historical file only.
        "github_release" => candidate.sha256.clone(),
        // Otherwise prefer the hash the version published, then the pinned one.
        _ => candidate
            .sha256
            .clone()
            .or_else(|| pinned.map(str::to_string)),
    }
}

pub struct CoreBackend {
    ctx: Ctx,
    /// The user allowed Modrinth for this import only (see
    /// [`CoreBackend::with_modrinth_allowed_once`]).
    allow_modrinth_once: bool,
}

impl CoreBackend {
    pub fn new(ctx: Ctx) -> Self {
        Self {
            ctx,
            allow_modrinth_once: false,
        }
    }

    /// A backend for an import the user allowed Modrinth for, once. The
    /// `modrinth_enabled` toggle stays as it is; only these entries' Modrinth
    /// requests skip it. The Privacy switch still applies.
    pub fn with_modrinth_allowed_once(ctx: Ctx) -> Self {
        Self {
            ctx,
            allow_modrinth_once: true,
        }
    }

    fn instance(&self, instance_id: &str) -> Result<crate::models::InstanceRow, BackendError> {
        crate::instance_service::InstanceService::new(self.ctx.clone())
            .get(instance_id)
            .map_err(|e| BackendError::Failed(e.to_string()))?
            .map(|detail| detail.row)
            .ok_or_else(|| BackendError::Failed(format!("Instance '{instance_id}' not found.")))
    }
}

#[async_trait]
impl PackBackend for CoreBackend {
    fn installed_filenames(&self, instance_id: &str) -> Vec<String> {
        let Ok(path) = self.ctx.paths.instance_manifest(instance_id) else {
            return Vec::new();
        };
        let Ok(manifest) = crate::helpers::read_manifest(&path) else {
            return Vec::new();
        };
        all_entries(&manifest).map(|e| e.filename.clone()).collect()
    }

    fn installed_sha256(&self, instance_id: &str, filename: &str) -> Option<String> {
        let manifest_path = self.ctx.paths.instance_manifest(instance_id).ok()?;
        let manifest = crate::helpers::read_manifest(&manifest_path).ok()?;
        let entry = all_entries(&manifest).find(|e| e.filename == filename)?;
        let dir = self.ctx.paths.instance_dir(instance_id).ok()?;
        let path = dir
            .join(crate::helpers::content_subdir(&entry.content_type))
            .join(filename);
        crate::helpers::hash_file_sha256(&path).ok()
    }

    fn display_name(&self, registry_id: &str) -> Option<String> {
        crate::registry::RegistryService::new(self.ctx.clone())
            .get_item_by_id(registry_id)
            .ok()
            .flatten()
            .map(|item| item.name)
            .filter(|name| !name.trim().is_empty())
    }

    fn knows_registry_item(&self, registry_id: &str) -> Result<bool, String> {
        crate::registry::RegistryService::new(self.ctx.clone())
            .get_item_by_id(registry_id)
            .map(|item| item.is_some())
            .map_err(|e| e.to_string())
    }

    fn modrinth_refusal(&self) -> Option<BackendError> {
        let svc = crate::modrinth::ModrinthService::new(self.ctx.clone());
        let checked = if self.allow_modrinth_once {
            svc.check_network_enabled()
        } else {
            svc.check_enabled()
        };
        checked.err().map(|error| match error {
            LauncherError::ModrinthDisabled => BackendError::NeedsAction(
                PackNeedsAction::EnableModrinth,
                MODRINTH_OFF_NOTE.to_string(),
            ),
            other if other.code() == "ERR_NETWORK_DISABLED" => {
                BackendError::NeedsAction(PackNeedsAction::OpenSettings, other.to_string())
            }
            other => BackendError::Failed(other.to_string()),
        })
    }

    async fn registry_versions(
        &self,
        instance_id: &str,
        registry_id: &str,
    ) -> Result<Vec<PackCandidate>, BackendError> {
        let instance = self.instance(instance_id)?;
        let item = crate::registry::RegistryService::new(self.ctx.clone())
            .get_item_by_id(registry_id)
            .map_err(|e| BackendError::Failed(e.to_string()))?
            .ok_or_else(|| {
                BackendError::Failed(format!(
                    "'{registry_id}' is not in the catalog this install has."
                ))
            })?;
        let candidates = crate::resolver::Resolver::new(self.ctx.clone())
            .list_curated_versions(&item, &instance.minecraft_version, &instance.loader)
            .await
            .map_err(|error| {
                if error.code() == "ERR_NO_ENABLED_SOURCE" {
                    BackendError::NeedsAction(PackNeedsAction::OpenSettings, error.to_string())
                } else if is_unreachable(&error) {
                    BackendError::Offline
                } else {
                    BackendError::Failed(format!("Could not list versions: {error}"))
                }
            })?;
        let content_type = if item.content_type.is_empty() {
            "mod".to_string()
        } else {
            item.content_type.clone()
        };
        Ok(candidates
            .iter()
            .map(|c| PackCandidate {
                version: c.version.clone(),
                filename: c.filename.clone(),
                download_url: c.download_url.clone(),
                sha1: c.sha1.clone(),
                sha512: c.sha512.clone(),
                expected_sha256: expected_sha256_for_candidate(&item, c),
                content_type: content_type.clone(),
                registry_id: Some(item.id.clone()),
                modrinth_id: item.modrinth_id.clone(),
                source: "registry".into(),
            })
            .collect())
    }

    async fn modrinth_versions(
        &self,
        instance_id: &str,
        project_id: &str,
    ) -> Result<Vec<PackCandidate>, BackendError> {
        if let Some(refusal) = self.modrinth_refusal() {
            return Err(refusal);
        }
        self.instance(instance_id)?;
        let svc = crate::modrinth::ModrinthService::new(self.ctx.clone());
        let listed = if self.allow_modrinth_once {
            svc.list_raw_modrinth_versions_allowed_once(Some(instance_id), project_id, Some("mod"))
                .await
        } else {
            svc.list_raw_modrinth_versions(Some(instance_id), project_id, Some("mod"))
                .await
        };
        let versions = listed.map_err(|e| {
            if is_unreachable(&e) {
                BackendError::Offline
            } else {
                BackendError::Failed(format!("Could not list Modrinth versions: {e}"))
            }
        })?;
        Ok(versions
            .into_iter()
            .map(|v| PackCandidate {
                version: v.version,
                filename: v.filename,
                download_url: v.download_url,
                sha1: v.sha1,
                sha512: v.sha512,
                expected_sha256: None,
                content_type: "mod".into(),
                registry_id: None,
                modrinth_id: Some(project_id.to_string()),
                source: "modrinth_raw".into(),
            })
            .collect())
    }

    async fn install(
        &self,
        instance_id: &str,
        candidate: &PackCandidate,
        required_sha256: Option<&str>,
    ) -> Result<(), BackendError> {
        let installer = crate::install_service::InstallService::new(self.ctx.clone());
        // A pinned hash also stands in when the source published none: the
        // bytes are then held to the one hash anybody vouched for.
        let expected_sha256 = candidate.expected_sha256.as_deref().or(required_sha256);
        if candidate.source == "modrinth_raw" {
            // Raw Modrinth keeps its own rules (gate re-check, SHA-1 required).
            let raw = crate::modrinth::RawModrinthVersionCandidate {
                version: candidate.version.clone(),
                version_id: String::new(),
                name: candidate.version.clone(),
                filename: candidate.filename.clone(),
                download_url: candidate.download_url.clone(),
                sha1: candidate.sha1.clone(),
                sha512: candidate.sha512.clone(),
                size: None,
                dependencies: Vec::new(),
                mc_versions: Vec::new(),
                loaders: Vec::new(),
                release_date: None,
                primary: true,
                changelog: None,
                is_prerelease: false,
                version_type: String::new(),
            };
            let svc = crate::modrinth::ModrinthService::new(self.ctx.clone());
            let project_id = candidate.modrinth_id.as_deref().unwrap_or_default();
            let installed = if self.allow_modrinth_once {
                svc.install_raw_modrinth_pinned_allowed_once(
                    instance_id,
                    project_id,
                    &raw,
                    "mod",
                    required_sha256,
                )
                .await
            } else {
                svc.install_raw_modrinth_pinned(
                    instance_id,
                    project_id,
                    &raw,
                    "mod",
                    required_sha256,
                )
                .await
            };
            return installed.map(|_| ()).map_err(install_error);
        }
        installer
            .install_artifact_pinned(
                instance_id,
                &candidate.filename,
                &candidate.content_type,
                &candidate.download_url,
                candidate.registry_id.as_deref(),
                candidate.modrinth_id.as_deref(),
                &candidate.source,
                Some(&candidate.version),
                candidate.sha1.as_deref(),
                expected_sha256,
                required_sha256,
                candidate.sha512.as_deref(),
            )
            .await
            .map(|_| ())
            .map_err(install_error)
    }
}

fn describe_install_error(error: LauncherError) -> String {
    match error {
        LauncherError::HashMismatch => "The downloaded file does not match the SHA-256 \
            hash the pack pinned (or the hash its source published). It was not installed."
            .to_string(),
        other => other.to_string(),
    }
}

fn install_error(error: LauncherError) -> BackendError {
    if is_unreachable(&error) {
        BackendError::Offline
    } else {
        BackendError::Failed(describe_install_error(error))
    }
}

fn all_entries(manifest: &InstanceManifest) -> impl Iterator<Item = &crate::models::InstalledMod> {
    manifest
        .mods
        .iter()
        .chain(manifest.resourcepacks.iter())
        .chain(manifest.shaders.iter())
        .chain(manifest.datapacks.iter())
        .chain(manifest.worlds.iter())
}

fn all_entries_mut(
    manifest: &mut InstanceManifest,
) -> impl Iterator<Item = &mut crate::models::InstalledMod> {
    manifest
        .mods
        .iter_mut()
        .chain(manifest.resourcepacks.iter_mut())
        .chain(manifest.shaders.iter_mut())
        .chain(manifest.datapacks.iter_mut())
        .chain(manifest.worlds.iter_mut())
}

// ---------------------------------------------------------------------------
// Bookkeeping
// ---------------------------------------------------------------------------

/// Mark the pack's files as pack-owned and refresh the pack inventory.
///
/// `pack_managed` is what the Mods tab groups on ("From the modpack" vs "Added
/// by you"); installing a file does not set it, so without this every imported
/// mod reads as added by the user. `origin` creates the pack origin when the
/// instance has none yet (first import); a retry passes `None` and only
/// refreshes the content hash of the existing one.
pub fn finalize_pack_files(
    ctx: &Ctx,
    instance_id: &str,
    pack_filenames: &[String],
    origin: Option<(&str, &AgoraPackInstance)>,
) -> Result<(), String> {
    let manifest_path = ctx
        .paths
        .instance_manifest(instance_id)
        .map_err(|e| e.to_string())?;
    let mut manifest = crate::helpers::read_manifest(&manifest_path).map_err(|e| e.to_string())?;
    for entry in all_entries_mut(&mut manifest) {
        if pack_filenames.contains(&entry.filename) {
            entry.pack_managed = true;
        }
    }

    let instance_dir = ctx
        .paths
        .instance_dir(instance_id)
        .map_err(|e| e.to_string())?;
    let files = crate::pack_inventory::collect_pack_inventory(&instance_dir)?;
    let hash = if files.is_empty() {
        None
    } else {
        Some(crate::pack_inventory::pack_content_hash(&files))
    };
    let conn = crate::db::local_state_connection(&ctx.paths.local_state_db())
        .map_err(|e| e.to_string())?;
    crate::db::replace_instance_pack_files(&conn, instance_id, &files)
        .map_err(|e| e.to_string())?;

    match (&mut manifest.pack_origin, origin) {
        (Some(existing), _) => existing.pack_content_hash = hash,
        (slot @ None, Some((pack_name, instance))) => {
            // Honest identity is the display name only; every id stays None so
            // pack-update can tell "unknown" from "known".
            *slot = Some(PackOrigin {
                platform: PackPlatform::LocalFile,
                pack_name: pack_name.to_string(),
                project_id: None,
                version_id: None,
                version_number: None,
                origin_url: None,
                pack_content_hash: hash,
                pack_minecraft_version: Some(instance.minecraft_version.clone()),
                pack_loader: Some(instance.loader.clone()),
                pack_loader_version: Some(instance.loader_version.clone()),
                launcher_kind: None,
                installation_key: None,
                source_key: None,
                cloned_from: None,
                installed_at: chrono::Utc::now().to_rfc3339(),
            });
        }
        (None, None) => {}
    }
    manifest.manifest_version = crate::models::CURRENT_MANIFEST_VERSION;
    crate::helpers::atomic_write_manifest(&manifest_path, &manifest).map_err(|e| e.to_string())
}

fn pack_filenames(outcomes: &[PackEntryOutcome]) -> Vec<String> {
    outcomes
        .iter()
        .filter(|o| o.status == PackEntryStatus::Installed)
        // A same-name file that was already there and not proven to be the
        // pack's is the user's; only fresh installs and hash-verified files
        // count as the pack's.
        .filter(|o| {
            o.verified_against_pack
                || !o
                    .reason
                    .as_deref()
                    .is_some_and(|r| r.starts_with("Already installed"))
        })
        // `name` is the installed filename for an installed entry.
        .map(|o| o.name.clone())
        .collect()
}

/// Install a freshly created instance's mods from a parsed pack.
pub async fn install_agora_pack_mods(
    ctx: &Ctx,
    instance_id: &str,
    pack_name: &str,
    pack: ParsedAgoraPack,
) -> PackImportReport {
    install_agora_pack_mods_with(
        &CoreBackend::new(ctx.clone()),
        ctx,
        instance_id,
        pack_name,
        pack,
    )
    .await
}

pub async fn install_agora_pack_mods_with(
    backend: &dyn PackBackend,
    ctx: &Ctx,
    instance_id: &str,
    pack_name: &str,
    pack: ParsedAgoraPack,
) -> PackImportReport {
    let outcomes = install_pack_entries(backend, instance_id, pack.mods).await;
    let mut warnings = Vec::new();
    if let Err(error) = finalize_pack_files(
        ctx,
        instance_id,
        &pack_filenames(&outcomes),
        Some((pack_name, &pack.instance)),
    ) {
        warnings.push(format!(
            "The mods were installed but the instance could not be marked as coming from this \
             pack: {error}"
        ));
    }
    PackImportReport::from_outcomes(instance_id, pack_name, outcomes, warnings)
}

/// Try the given entries again on an existing instance (after the user turned
/// Modrinth on, or allowed it for this import). Only entries not already
/// installed are fetched. `allow_modrinth_once` leaves the `modrinth_enabled`
/// toggle alone and lets just these entries use Modrinth.
pub async fn retry_agora_pack_entries(
    ctx: &Ctx,
    instance_id: &str,
    entries: Vec<AgoraPackEntry>,
    allow_modrinth_once: bool,
) -> LauncherResult<PackImportReport> {
    let backend = if allow_modrinth_once {
        CoreBackend::with_modrinth_allowed_once(ctx.clone())
    } else {
        CoreBackend::new(ctx.clone())
    };
    retry_agora_pack_entries_with(&backend, ctx, instance_id, entries).await
}

/// What importing `pack` would do, judged without writing or downloading
/// anything. Every entry gets a planned line, including ones that cannot be
/// read, so the review shows the whole pack.
pub fn plan_agora_pack(backend: &dyn PackBackend, pack: &ParsedAgoraPack) -> PackImportPlan {
    let refusal = backend.modrinth_refusal();
    let entries = pack
        .mods
        .iter()
        .map(|parsed| match parsed {
            PackEntryParse::Invalid { name, reason } => PlannedEntry {
                name: name.clone(),
                display_name: None,
                source: PlannedSource::CannotGet,
                checkable: false,
                reason: Some(reason.clone()),
                action: None,
            },
            PackEntryParse::Entry(entry) => plan_entry(backend, entry, &refusal),
        })
        .collect();
    PackImportPlan {
        pack_name: pack_display_name(&pack.instance),
        instance: pack.instance.clone(),
        entries,
        // Filled in by `plan_agora_pack_bytes`, which has the file's bytes.
        file_sha256: String::new(),
        file_size: 0,
    }
}

fn plan_entry(
    backend: &dyn PackBackend,
    entry: &AgoraPackEntry,
    refusal: &Option<BackendError>,
) -> PlannedEntry {
    let (source, mut reason, mut action) = if let Some(registry_id) = non_empty(&entry.registry_id)
    {
        match backend.knows_registry_item(registry_id) {
            Ok(true) => (PlannedSource::Curated, None, None),
            Ok(false) => (
                PlannedSource::CannotGet,
                Some(format!(
                    "'{registry_id}' is not in the catalog this install has."
                )),
                None,
            ),
            Err(error) => (
                PlannedSource::CannotGet,
                Some(format!("The catalog could not be read: {error}")),
                None,
            ),
        }
    } else if non_empty(&entry.modrinth_id).is_some() {
        match refusal {
            None => (PlannedSource::Modrinth, None, None),
            Some(BackendError::NeedsAction(needed, note)) => {
                (PlannedSource::Modrinth, Some(note.clone()), Some(*needed))
            }
            Some(BackendError::Failed(why)) => (PlannedSource::Modrinth, Some(why.clone()), None),
            // The settings check makes no request, so this is not expected here.
            Some(BackendError::Offline) => (
                PlannedSource::Modrinth,
                Some(OFFLINE_NOTE.to_string()),
                None,
            ),
        }
    } else {
        (
            PlannedSource::CannotGet,
            Some(
                "The pack records no catalog or Modrinth id for this file, so there is nowhere \
                 to download it from."
                    .to_string(),
            ),
            None,
        )
    };

    let checkable = match normalize_pin(&entry.sha256) {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(pin_problem) => {
            // A damaged pin is a certain failure; say so before the import does.
            reason = Some(pin_problem);
            action = None;
            false
        }
    };

    PlannedEntry {
        name: entry.display_name(),
        display_name: non_empty(&entry.registry_id).and_then(|id| backend.display_name(id)),
        source,
        checkable,
        reason,
        action,
    }
}

/// [`plan_agora_pack`] against this install's real settings and catalog.
pub fn plan_agora_pack_import(ctx: &Ctx, pack: &ParsedAgoraPack) -> PackImportPlan {
    plan_agora_pack(&CoreBackend::new(ctx.clone()), pack)
}

pub async fn retry_agora_pack_entries_with(
    backend: &dyn PackBackend,
    ctx: &Ctx,
    instance_id: &str,
    entries: Vec<AgoraPackEntry>,
) -> LauncherResult<PackImportReport> {
    let manifest_path = ctx.paths.instance_manifest(instance_id)?;
    let manifest = crate::helpers::read_manifest(&manifest_path)?;
    let instance_name = manifest
        .pack_origin
        .as_ref()
        .map(|o| o.pack_name.clone())
        .unwrap_or_else(|| instance_id.to_string());
    let outcomes = install_pack_entries(
        backend,
        instance_id,
        entries.into_iter().map(PackEntryParse::Entry).collect(),
    )
    .await;
    let mut warnings = Vec::new();
    if let Err(error) = finalize_pack_files(ctx, instance_id, &pack_filenames(&outcomes), None) {
        warnings.push(format!(
            "The mods were installed but could not be marked as part of the pack: {error}"
        ));
    }
    Ok(PackImportReport::from_outcomes(
        instance_id,
        &instance_name,
        outcomes,
        warnings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const GOOD_PIN: &str = "4277bee0ef05d1e9dc1e03b51d52c67bd6457bb57402407e59b16ad7b08e8379";

    fn candidate(version: &str, filename: &str) -> PackCandidate {
        PackCandidate {
            version: version.into(),
            filename: filename.into(),
            download_url: format!("https://cdn.modrinth.com/{filename}"),
            sha1: Some("abc".into()),
            sha512: None,
            expected_sha256: None,
            content_type: "mod".into(),
            registry_id: None,
            modrinth_id: Some("proj".into()),
            source: "modrinth_raw".into(),
        }
    }

    struct Fake {
        modrinth_on: bool,
        registry: Result<Vec<PackCandidate>, BackendError>,
        modrinth: Vec<PackCandidate>,
        /// Filenames whose install fails with this message.
        install_error: Option<(String, String)>,
        /// Every download fails for lack of a connection.
        install_offline: bool,
        present: Vec<String>,
        present_sha256: Option<String>,
        names: Vec<(String, String)>,
        /// Registry ids this install's catalog has.
        known: Vec<String>,
        /// When set, the catalog cannot be read.
        catalog_error: Option<String>,
        installs: Mutex<Vec<(String, Option<String>)>>,
        modrinth_lookups: Mutex<u32>,
    }

    fn fake() -> Fake {
        Fake {
            modrinth_on: true,
            ..Fake::blank()
        }
    }

    impl Fake {
        fn blank() -> Self {
            Fake {
                modrinth_on: false,
                registry: Ok(Vec::new()),
                modrinth: Vec::new(),
                install_error: None,
                install_offline: false,
                present: Vec::new(),
                present_sha256: None,
                names: Vec::new(),
                known: Vec::new(),
                catalog_error: None,
                installs: Mutex::new(Vec::new()),
                modrinth_lookups: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl PackBackend for Fake {
        fn installed_filenames(&self, _: &str) -> Vec<String> {
            let mut all = self.present.clone();
            all.extend(self.installs.lock().unwrap().iter().map(|i| i.0.clone()));
            all
        }
        fn installed_sha256(&self, _: &str, _: &str) -> Option<String> {
            self.present_sha256.clone()
        }
        fn display_name(&self, registry_id: &str) -> Option<String> {
            self.names
                .iter()
                .find(|(id, _)| id == registry_id)
                .map(|(_, name)| name.clone())
        }
        fn knows_registry_item(&self, registry_id: &str) -> Result<bool, String> {
            if let Some(message) = &self.catalog_error {
                return Err(message.clone());
            }
            Ok(self.known.iter().any(|id| id == registry_id))
        }
        fn modrinth_refusal(&self) -> Option<BackendError> {
            if self.modrinth_on {
                None
            } else {
                Some(BackendError::NeedsAction(
                    PackNeedsAction::EnableModrinth,
                    MODRINTH_OFF_NOTE.into(),
                ))
            }
        }
        async fn registry_versions(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<PackCandidate>, BackendError> {
            self.registry.clone()
        }
        async fn modrinth_versions(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<PackCandidate>, BackendError> {
            *self.modrinth_lookups.lock().unwrap() += 1;
            if !self.modrinth_on {
                return Err(BackendError::NeedsAction(
                    PackNeedsAction::EnableModrinth,
                    MODRINTH_OFF_NOTE.into(),
                ));
            }
            Ok(self.modrinth.clone())
        }
        async fn install(
            &self,
            _: &str,
            candidate: &PackCandidate,
            required: Option<&str>,
        ) -> Result<(), BackendError> {
            if self.install_offline {
                return Err(BackendError::Offline);
            }
            if let Some((file, message)) = &self.install_error {
                if *file == candidate.filename {
                    return Err(BackendError::Failed(message.clone()));
                }
            }
            self.installs
                .lock()
                .unwrap()
                .push((candidate.filename.clone(), required.map(str::to_string)));
            Ok(())
        }
    }

    fn entry(filename: &str, version: &str, sha: Option<&str>) -> PackEntryParse {
        PackEntryParse::Entry(AgoraPackEntry {
            filename: filename.into(),
            version: Some(version.into()),
            registry_id: None,
            modrinth_id: Some("proj".into()),
            sha256: sha.map(str::to_string),
            source: Some("modrinth_raw".into()),
        })
    }

    fn run(backend: &Fake, entries: Vec<PackEntryParse>) -> Vec<PackEntryOutcome> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(install_pack_entries(backend, "inst", entries))
    }

    #[test]
    fn modrinth_entry_with_modrinth_off_is_needs_action_not_silent() {
        let mut backend = fake();
        backend.modrinth_on = false;
        let out = run(&backend, vec![entry("xaero.jar", "1", Some(GOOD_PIN))]);
        assert_eq!(out.len(), 1, "every entry gets an outcome");
        assert_eq!(out[0].status, PackEntryStatus::NeedsAction);
        assert_eq!(out[0].action, Some(PackNeedsAction::EnableModrinth));
        assert!(out[0].reason.as_deref().unwrap().contains("Modrinth"));
        assert!(backend.installs.lock().unwrap().is_empty());
    }

    #[test]
    fn retry_after_enabling_installs_only_the_missing_entries() {
        let mut backend = fake();
        backend.modrinth_on = false;
        backend.registry = Ok(vec![PackCandidate {
            registry_id: Some("sodium".into()),
            source: "registry".into(),
            ..candidate("1", "sodium.jar")
        }]);
        let sodium = PackEntryParse::Entry(AgoraPackEntry {
            filename: "sodium.jar".into(),
            version: Some("1".into()),
            registry_id: Some("sodium".into()),
            sha256: Some(GOOD_PIN.into()),
            ..Default::default()
        });
        let first = run(
            &backend,
            vec![sodium, entry("xaero.jar", "1", Some(GOOD_PIN))],
        );
        assert_eq!(first[0].status, PackEntryStatus::Installed);
        assert_eq!(first[1].status, PackEntryStatus::NeedsAction);

        // The user turns Modrinth on; sodium is now in the instance.
        backend.modrinth_on = true;
        backend.modrinth = vec![candidate("1", "xaero.jar")];
        backend.present = vec!["sodium.jar".into()];
        backend.installs.lock().unwrap().clear();
        let retry: Vec<PackEntryParse> = first
            .iter()
            .map(|o| PackEntryParse::Entry(o.entry.clone()))
            .collect();
        let second = run(&backend, retry);
        assert!(second
            .iter()
            .all(|o| o.status == PackEntryStatus::Installed));
        let installs = backend.installs.lock().unwrap();
        assert_eq!(installs.len(), 1);
        assert_eq!(installs[0].0, "xaero.jar");
    }

    #[test]
    fn installs_the_pinned_version_not_the_latest() {
        let mut backend = fake();
        backend.modrinth = vec![
            candidate("3.0", "mod-3.0.jar"),
            candidate("2.0", "mod-2.0.jar"),
        ];
        let out = run(&backend, vec![entry("mod-2.0.jar", "2.0", Some(GOOD_PIN))]);
        assert_eq!(out[0].status, PackEntryStatus::Installed);
        assert_eq!(backend.installs.lock().unwrap()[0].0, "mod-2.0.jar");
    }

    #[test]
    fn pinned_version_is_matched_by_version_when_filename_differs() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("2.0", "renamed.jar"), candidate("3.0", "new.jar")];
        let out = run(&backend, vec![entry("old-name.jar", "2.0", Some(GOOD_PIN))]);
        assert_eq!(out[0].status, PackEntryStatus::Installed);
        assert_eq!(backend.installs.lock().unwrap()[0].0, "renamed.jar");
    }

    #[test]
    fn missing_pinned_version_fails_instead_of_installing_another() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("3.0", "mod-3.0.jar")];
        let out = run(&backend, vec![entry("mod-2.0.jar", "2.0", Some(GOOD_PIN))]);
        assert_eq!(out[0].status, PackEntryStatus::Failed);
        assert!(backend.installs.lock().unwrap().is_empty());
    }

    #[test]
    fn pinned_hash_is_required_of_the_install() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("2.0", "mod-2.0.jar")];
        let out = run(&backend, vec![entry("mod-2.0.jar", "2.0", Some(GOOD_PIN))]);
        assert!(out[0].verified_against_pack);
        assert_eq!(
            backend.installs.lock().unwrap()[0].1.as_deref(),
            Some(GOOD_PIN)
        );
    }

    #[test]
    fn pin_is_normalized_to_lowercase() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("2.0", "mod-2.0.jar")];
        run(
            &backend,
            vec![entry("mod-2.0.jar", "2.0", Some(&GOOD_PIN.to_uppercase()))],
        );
        assert_eq!(
            backend.installs.lock().unwrap()[0].1.as_deref(),
            Some(GOOD_PIN)
        );
    }

    #[test]
    fn hash_mismatch_is_a_named_failure() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("2.0", "mod-2.0.jar")];
        backend.install_error = Some((
            "mod-2.0.jar".into(),
            describe_install_error(LauncherError::HashMismatch),
        ));
        let out = run(&backend, vec![entry("mod-2.0.jar", "2.0", Some(GOOD_PIN))]);
        assert_eq!(out[0].status, PackEntryStatus::Failed);
        assert_eq!(out[0].name, "mod-2.0.jar");
        assert!(out[0].reason.as_deref().unwrap().contains("does not match"));
        assert!(!out[0].verified_against_pack);
    }

    #[test]
    fn malformed_pin_fails_closed_rather_than_meaning_no_pin() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("2.0", "mod-2.0.jar")];
        let out = run(
            &backend,
            vec![entry("mod-2.0.jar", "2.0", Some("deadbeef"))],
        );
        assert_eq!(out[0].status, PackEntryStatus::Failed);
        assert!(backend.installs.lock().unwrap().is_empty());
    }

    #[test]
    fn no_pin_installs_but_says_it_could_not_be_checked_against_the_pack() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("2.0", "mod-2.0.jar")];
        let out = run(&backend, vec![entry("mod-2.0.jar", "2.0", None)]);
        assert_eq!(out[0].status, PackEntryStatus::Installed);
        assert!(!out[0].verified_against_pack);
        assert!(out[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("could not be checked against the pack"));
    }

    #[test]
    fn version_lookup_error_is_a_reported_failure() {
        let mut backend = fake();
        backend.registry = Err(BackendError::Failed(
            "Could not list versions: offline".into(),
        ));
        let registry_entry = PackEntryParse::Entry(AgoraPackEntry {
            filename: "sodium.jar".into(),
            version: Some("1".into()),
            registry_id: Some("sodium".into()),
            ..Default::default()
        });
        let out = run(&backend, vec![registry_entry]);
        assert_eq!(out[0].status, PackEntryStatus::Failed);
        assert!(out[0].reason.as_deref().unwrap().contains("offline"));
    }

    #[test]
    fn entry_with_no_source_and_unreadable_entry_are_reported() {
        let backend = fake();
        let bare = PackEntryParse::Entry(AgoraPackEntry {
            filename: "hand.jar".into(),
            ..Default::default()
        });
        let invalid = PackEntryParse::Invalid {
            name: "mods[1]".into(),
            reason: "bad".into(),
        };
        let out = run(&backend, vec![bare, invalid]);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|o| o.status == PackEntryStatus::Failed));
    }

    #[test]
    fn unsafe_candidate_filename_is_refused() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("2.0", "..\\..\\evil.jar")];
        let out = run(&backend, vec![entry("x.jar", "2.0", Some(GOOD_PIN))]);
        assert_eq!(out[0].status, PackEntryStatus::Failed);
        assert!(backend.installs.lock().unwrap().is_empty());
    }

    #[test]
    fn retry_skips_what_is_already_installed() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("2.0", "mod-2.0.jar")];
        backend.present = vec!["have.jar".into()];
        let out = run(
            &backend,
            vec![
                entry("have.jar", "1.0", Some(GOOD_PIN)),
                entry("mod-2.0.jar", "2.0", Some(GOOD_PIN)),
            ],
        );
        assert!(out.iter().all(|o| o.status == PackEntryStatus::Installed));
        let installs = backend.installs.lock().unwrap();
        assert_eq!(installs.len(), 1, "only the missing one is fetched");
        assert_eq!(installs[0].0, "mod-2.0.jar");
        assert_eq!(*backend.modrinth_lookups.lock().unwrap(), 1);
    }

    #[test]
    fn parse_keeps_unreadable_entries_and_requires_a_complete_instance() {
        let text = r#"{"instance":{"id":"a","loader":"fabric","loader_version":"1","minecraft_version":"1.21"},
            "mods":[{"filename":"a.jar","registry_id":null,"modrinth_id":"x"}, 7]}"#;
        let parsed = parse_agora_pack(text).unwrap();
        assert_eq!(parsed.mods.len(), 2);
        assert!(matches!(parsed.mods[0], PackEntryParse::Entry(_)));
        assert!(matches!(parsed.mods[1], PackEntryParse::Invalid { .. }));
        assert!(parse_agora_pack(r#"{"instance":{"loader":"fabric"}}"#).is_err());
        assert!(parse_agora_pack(
            r#"{"instance":{"loader":"f","loader_version":"1","minecraft_version":"1"},"mods":5}"#
        )
        .is_err());
    }

    // ---- commit path: the hash gate on real bytes, nothing written on failure

    fn temp_ctx_with_instance() -> (Ctx, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "agora-pack-import-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let ctx = Ctx::for_testing(root.clone());
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        let dir = ctx.paths.instance_dir("inst").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let request = crate::instance_service::CreateInstanceRequest {
            name: "Inst".into(),
            instance_id: "inst".into(),
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16.0".into(),
            jvm_memory_mb: None,
            jvm_memory_mode: None,
            jvm_gc: None,
            jvm_custom_args: None,
            jvm_always_pre_touch: None,
            is_modpack: None,
            pack_icon_url: None,
            template_id: None,
        };
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::upsert_instance(
            &conn,
            &crate::instance_service::prepare_row("inst", &request),
        )
        .unwrap();
        drop(conn);
        let manifest: InstanceManifest = serde_json::from_value(serde_json::json!({
            "instance_id": "inst", "name": "Inst", "minecraft_version": "1.21",
            "loader": "fabric", "loader_version": "0.16.0", "is_locked": false,
            "mods": [], "resourcepacks": [], "shaders": [], "datapacks": [], "worlds": [],
            "user_preferences": {}, "manifest_version": crate::models::CURRENT_MANIFEST_VERSION
        }))
        .unwrap();
        crate::helpers::atomic_write_manifest(
            &ctx.paths.instance_manifest("inst").unwrap(),
            &manifest,
        )
        .unwrap();
        (ctx, root)
    }

    fn commit(
        ctx: &Ctx,
        bytes: &[u8],
        sha1: &str,
        required: Option<&str>,
    ) -> LauncherResult<crate::models::InstalledMod> {
        commit_with_sha512(ctx, bytes, sha1, required, None)
    }

    fn commit_with_sha512(
        ctx: &Ctx,
        bytes: &[u8],
        sha1: &str,
        required: Option<&str>,
        sha512: Option<&str>,
    ) -> LauncherResult<crate::models::InstalledMod> {
        crate::install_service::InstallService::new(ctx.clone()).commit_verified_artifact(
            "inst",
            "m.jar",
            "mod",
            "https://cdn.modrinth.com/m.jar",
            None,
            Some("proj"),
            "modrinth_raw",
            Some("1"),
            Some(sha1),
            None,
            required,
            sha512,
            bytes.to_vec(),
        )
    }

    #[test]
    fn required_sha256_mismatch_installs_nothing_even_when_sha1_matches() {
        let (ctx, root) = temp_ctx_with_instance();
        let bytes = b"jar bytes";
        let sha1 = crate::download::sha1_hex(bytes);
        let err = commit(&ctx, bytes, &sha1, Some(GOOD_PIN)).unwrap_err();
        assert_eq!(err.code(), "ERR_HASH_MISMATCH");
        let dir = ctx.paths.instance_dir("inst").unwrap();
        assert!(!dir.join("mods").join("m.jar").exists(), "no file written");
        let manifest =
            crate::helpers::read_manifest(&ctx.paths.instance_manifest("inst").unwrap()).unwrap();
        assert!(manifest.mods.is_empty(), "no manifest entry");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn required_sha256_match_installs() {
        let (ctx, root) = temp_ctx_with_instance();
        let bytes = b"jar bytes";
        let sha1 = crate::download::sha1_hex(bytes);
        let sha256 = crate::download::sha256_hex(bytes);
        commit(&ctx, bytes, &sha1, Some(&sha256)).unwrap();
        assert!(ctx
            .paths
            .instance_dir("inst")
            .unwrap()
            .join("mods")
            .join("m.jar")
            .exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn imported_files_are_marked_as_from_the_pack() {
        let (ctx, root) = temp_ctx_with_instance();
        let bytes = b"jar bytes";
        let sha1 = crate::download::sha1_hex(bytes);
        let installed = commit(&ctx, bytes, &sha1, None).unwrap();
        assert!(
            !installed.pack_managed,
            "installing alone does not stamp it"
        );
        let instance = AgoraPackInstance {
            id: None,
            name: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16.0".into(),
        };
        finalize_pack_files(
            &ctx,
            "inst",
            &["m.jar".to_string()],
            Some(("Pack", &instance)),
        )
        .unwrap();
        let manifest =
            crate::helpers::read_manifest(&ctx.paths.instance_manifest("inst").unwrap()).unwrap();
        assert!(manifest.mods[0].pack_managed);
        assert_eq!(manifest.pack_origin.unwrap().pack_name, "Pack");
        let _ = std::fs::remove_dir_all(root);
    }

    // ---- review: adversarial probes (each expected to FAIL on df7b81b)

    #[test]
    fn review_duplicate_entries_in_one_pack_are_fetched_once() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("2.0", "mod-2.0.jar")];
        let dup = || entry("mod-2.0.jar", "2.0", Some(GOOD_PIN));
        let out = run(&backend, vec![dup(), dup(), dup()]);
        assert_eq!(out.len(), 3, "every entry gets an outcome");
        assert_eq!(
            *backend.modrinth_lookups.lock().unwrap(),
            1,
            "one lookup per distinct file"
        );
        assert_eq!(
            backend.installs.lock().unwrap().len(),
            1,
            "one install per distinct file"
        );
    }

    #[test]
    fn review_whitespace_only_pin_fails_closed_instead_of_meaning_no_pin() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("2.0", "mod-2.0.jar")];
        let out = run(&backend, vec![entry("mod-2.0.jar", "2.0", Some("   "))]);
        assert_eq!(out[0].status, PackEntryStatus::Failed);
        assert!(backend.installs.lock().unwrap().is_empty());
    }

    #[test]
    fn review_windows_device_names_are_refused_as_filenames() {
        for name in ["NUL.jar", "con.jar", "COM1.txt", "lpt9.jar", "aux", "nul"] {
            let mut backend = fake();
            backend.modrinth = vec![candidate("2.0", name)];
            let out = run(&backend, vec![entry("x.jar", "2.0", Some(GOOD_PIN))]);
            assert_eq!(
                out[0].status,
                PackEntryStatus::Failed,
                "{name} must not install"
            );
            assert!(
                backend.installs.lock().unwrap().is_empty(),
                "{name} written"
            );
        }
    }

    #[test]
    fn review_trailing_dot_or_space_filenames_are_refused() {
        // Windows strips trailing dots and spaces, so "sodium.jar." lands on
        // "sodium.jar" and overwrites it.
        for name in ["sodium.jar.", "sodium.jar ", "sodium.jar. "] {
            let mut backend = fake();
            backend.modrinth = vec![candidate("2.0", name)];
            let out = run(&backend, vec![entry("x.jar", "2.0", Some(GOOD_PIN))]);
            assert_eq!(
                out[0].status,
                PackEntryStatus::Failed,
                "{name:?} must not install"
            );
            assert!(
                backend.installs.lock().unwrap().is_empty(),
                "{name:?} written"
            );
        }
    }

    #[test]
    fn review_present_file_with_a_pin_is_not_reported_as_a_plain_install() {
        // The bytes already on disk are never hashed against the pack's pin, and
        // the outcome says only "Already installed." with verified_against_pack
        // false, which the struct documents as "the pack pinned no SHA-256".
        let mut backend = fake();
        backend.present = vec!["have.jar".into()];
        let out = run(&backend, vec![entry("have.jar", "1.0", Some(GOOD_PIN))]);
        assert_eq!(out[0].status, PackEntryStatus::Installed);
        assert!(
            out[0].verified_against_pack
                || out[0]
                    .reason
                    .as_deref()
                    .is_some_and(|r| r.contains("checked")),
            "a pinned file never hashed against the pin must say so, got {:?}",
            out[0]
        );
    }

    #[test]
    fn matching_sha1_with_wrong_sha512_installs_nothing() {
        let (ctx, root) = temp_ctx_with_instance();
        let bytes = b"jar bytes";
        let sha1 = crate::download::sha1_hex(bytes);
        let wrong = crate::download::sha512_hex(b"other bytes");
        let err = commit_with_sha512(&ctx, bytes, &sha1, None, Some(&wrong)).unwrap_err();
        assert_eq!(err.code(), "ERR_HASH_MISMATCH");
        assert!(!ctx
            .paths
            .instance_dir("inst")
            .unwrap()
            .join("mods")
            .join("m.jar")
            .exists());
        let right = crate::download::sha512_hex(bytes);
        commit_with_sha512(&ctx, bytes, &sha1, None, Some(&right)).unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn present_pinned_file_is_hashed_and_a_mismatch_is_reported() {
        let mut backend = fake();
        backend.present = vec!["have.jar".into()];
        backend.present_sha256 = Some(GOOD_PIN.into());
        let ok = run(&backend, vec![entry("have.jar", "1", Some(GOOD_PIN))]);
        assert!(ok[0].verified_against_pack);
        backend.present_sha256 = Some("b".repeat(64));
        let bad = run(&backend, vec![entry("have.jar", "1", Some(GOOD_PIN))]);
        assert_eq!(bad[0].status, PackEntryStatus::Failed);
    }

    #[test]
    fn retry_does_not_stamp_a_users_same_name_file_as_the_packs() {
        let mut backend = fake();
        backend.present = vec!["mine.jar".into()];
        backend.present_sha256 = None;
        let out = run(&backend, vec![entry("mine.jar", "1", Some(GOOD_PIN))]);
        assert!(
            pack_filenames(&out).is_empty(),
            "unproven file must not be stamped"
        );
        backend.present_sha256 = Some(GOOD_PIN.into());
        let out = run(&backend, vec![entry("mine.jar", "1", Some(GOOD_PIN))]);
        assert_eq!(pack_filenames(&out), vec!["mine.jar".to_string()]);
    }

    #[test]
    fn oversized_packs_are_refused() {
        let big = " ".repeat(MAX_PACK_FILE_BYTES as usize + 1);
        assert!(parse_agora_pack(&big).is_err());
        let mods = vec!["{\"filename\":\"a.jar\"}"; MAX_PACK_ENTRIES + 1].join(",");
        let text = format!(
            "{{\"instance\":{{\"loader\":\"f\",\"loader_version\":\"1\",\"minecraft_version\":\"1\"}},\"mods\":[{mods}]}}"
        );
        assert!(parse_agora_pack(&text).is_err());
    }

    #[test]
    fn outcomes_carry_the_catalog_name_when_known() {
        let mut backend = fake();
        backend.names = vec![("sodium".into(), "Sodium".into())];
        backend.registry = Err(BackendError::Failed("x".into()));
        let e = PackEntryParse::Entry(AgoraPackEntry {
            filename: "sodium.jar".into(),
            registry_id: Some("sodium".into()),
            ..Default::default()
        });
        let out = run(&backend, vec![e]);
        assert_eq!(out[0].display_name.as_deref(), Some("Sodium"));
    }

    // -- Review (plan) step --------------------------------------------------

    fn pack_of(mods: Vec<PackEntryParse>) -> ParsedAgoraPack {
        ParsedAgoraPack {
            instance: AgoraPackInstance {
                id: Some("friend-pack".into()),
                name: Some("Friend Pack".into()),
                minecraft_version: "1.21".into(),
                loader: "fabric".into(),
                loader_version: "0.16.0".into(),
            },
            mods,
        }
    }

    fn registry_entry(filename: &str, registry_id: &str, sha: Option<&str>) -> PackEntryParse {
        PackEntryParse::Entry(AgoraPackEntry {
            filename: filename.into(),
            version: Some("1".into()),
            registry_id: Some(registry_id.into()),
            modrinth_id: None,
            sha256: sha.map(str::to_string),
            source: Some("registry".into()),
        })
    }

    fn modrinth_entry(filename: &str, project: &str, sha: Option<&str>) -> PackEntryParse {
        PackEntryParse::Entry(AgoraPackEntry {
            filename: filename.into(),
            version: Some("1".into()),
            registry_id: None,
            modrinth_id: Some(project.into()),
            sha256: sha.map(str::to_string),
            source: Some("modrinth_raw".into()),
        })
    }

    fn plan_of(backend: &Fake, mods: Vec<PackEntryParse>) -> PackImportPlan {
        plan_agora_pack(backend, &pack_of(mods))
    }

    #[test]
    fn plan_lists_every_entry_and_writes_or_downloads_nothing() {
        let mut backend = fake();
        backend.modrinth_on = false;
        backend.known = vec!["sodium".into()];
        let plan = plan_of(
            &backend,
            vec![
                registry_entry("sodium.jar", "sodium", Some(GOOD_PIN)),
                modrinth_entry("xaero.jar", "xaero", Some(GOOD_PIN)),
                PackEntryParse::Invalid {
                    name: "mods[2]".into(),
                    reason: "unreadable".into(),
                },
            ],
        );
        assert_eq!(plan.entries.len(), 3, "every line of the pack is planned");
        assert_eq!(plan.pack_name, "Friend Pack");
        assert_eq!(plan.instance.loader, "fabric");
        assert!(
            backend.installs.lock().unwrap().is_empty(),
            "plan installs nothing"
        );
        assert_eq!(
            *backend.modrinth_lookups.lock().unwrap(),
            0,
            "plan contacts no source"
        );
    }

    #[test]
    fn plan_marks_modrinth_off_as_an_action_with_the_note() {
        let mut backend = fake();
        backend.modrinth_on = false;
        let plan = plan_of(&backend, vec![modrinth_entry("xaero.jar", "xaero", None)]);
        let line = &plan.entries[0];
        assert_eq!(line.source, PlannedSource::Modrinth);
        assert_eq!(line.action, Some(PackNeedsAction::EnableModrinth));
        assert_eq!(line.reason.as_deref(), Some(MODRINTH_OFF_NOTE));
    }

    #[test]
    fn plan_with_modrinth_on_leaves_a_modrinth_entry_ready() {
        let plan = plan_of(
            &fake(),
            vec![modrinth_entry("xaero.jar", "xaero", Some(GOOD_PIN))],
        );
        let line = &plan.entries[0];
        assert_eq!(line.source, PlannedSource::Modrinth);
        assert_eq!(line.action, None);
        assert_eq!(line.reason, None);
        assert!(line.checkable);
    }

    #[test]
    fn plan_names_a_registry_id_the_catalog_does_not_have() {
        let mut backend = fake();
        backend.known = vec!["sodium".into()];
        let plan = plan_of(&backend, vec![registry_entry("ghost.jar", "ghost", None)]);
        let line = &plan.entries[0];
        assert_eq!(line.source, PlannedSource::CannotGet);
        assert!(line.reason.as_deref().unwrap().contains("'ghost'"));
        assert_eq!(line.action, None);
    }

    #[test]
    fn plan_says_so_when_the_catalog_cannot_be_read() {
        let mut backend = fake();
        backend.catalog_error = Some("disk gone".into());
        let plan = plan_of(&backend, vec![registry_entry("sodium.jar", "sodium", None)]);
        let line = &plan.entries[0];
        assert_eq!(line.source, PlannedSource::CannotGet);
        let reason = line.reason.as_deref().unwrap();
        assert!(reason.contains("could not be read"), "{reason}");
        assert!(!reason.contains("not in the catalog"), "{reason}");
    }

    #[test]
    fn plan_treats_a_blank_registry_id_as_absent_and_uses_modrinth() {
        let plan = plan_of(
            &fake(),
            vec![PackEntryParse::Entry(AgoraPackEntry {
                filename: "xaero.jar".into(),
                registry_id: Some("   ".into()),
                modrinth_id: Some("xaero".into()),
                ..Default::default()
            })],
        );
        assert_eq!(plan.entries[0].source, PlannedSource::Modrinth);
    }

    #[test]
    fn plan_entry_with_no_source_cannot_be_got() {
        let plan = plan_of(
            &fake(),
            vec![PackEntryParse::Entry(AgoraPackEntry {
                filename: "lost.jar".into(),
                ..Default::default()
            })],
        );
        let line = &plan.entries[0];
        assert_eq!(line.source, PlannedSource::CannotGet);
        assert!(line.reason.is_some());
    }

    #[test]
    fn plan_checkable_only_for_a_valid_pin_and_flags_a_damaged_one() {
        let mut backend = fake();
        backend.known = vec!["sodium".into()];
        let plan = plan_of(
            &backend,
            vec![
                registry_entry("a.jar", "sodium", Some(GOOD_PIN)),
                registry_entry("b.jar", "sodium", None),
                registry_entry("c.jar", "sodium", Some("zz")),
                registry_entry("d.jar", "sodium", Some("   ")),
            ],
        );
        assert!(plan.entries[0].checkable);
        assert!(!plan.entries[1].checkable);
        assert_eq!(
            plan.entries[1].reason, None,
            "no pin is not a problem by itself"
        );
        assert!(!plan.entries[2].checkable);
        assert!(plan.entries[2].reason.is_some(), "a damaged pin is named");
        assert!(!plan.entries[3].checkable, "a blank pin fails closed");
        assert!(plan.entries[3].reason.is_some());
    }

    #[test]
    fn plan_carries_the_catalog_name_and_reports_unreadable_lines() {
        let mut backend = fake();
        backend.known = vec!["sodium".into()];
        backend.names = vec![("sodium".into(), "Sodium".into())];
        let plan = plan_of(
            &backend,
            vec![
                registry_entry("sodium.jar", "sodium", None),
                PackEntryParse::Invalid {
                    name: "mods[1]".into(),
                    reason: "not a readable mod record".into(),
                },
            ],
        );
        assert_eq!(plan.entries[0].display_name.as_deref(), Some("Sodium"));
        assert_eq!(plan.entries[1].name, "mods[1]");
        assert_eq!(plan.entries[1].source, PlannedSource::CannotGet);
        assert_eq!(
            plan.entries[1].reason.as_deref(),
            Some("not a readable mod record")
        );
    }

    /// A temp instance whose Modrinth toggle is off and whose Privacy switch
    /// is on, so only the toggle stands in the way.
    fn modrinth_off_ctx() -> (Ctx, std::path::PathBuf) {
        let (ctx, root) = temp_ctx_with_instance();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::set_setting(&conn, "modrinth_enabled", &serde_json::json!(false)).unwrap();
        crate::db::set_setting(&conn, "network_modrinth_enabled", &serde_json::json!(true))
            .unwrap();
        (ctx, root)
    }

    #[test]
    fn allowing_modrinth_once_skips_the_toggle_and_leaves_it_off() {
        let (ctx, root) = modrinth_off_ctx();

        assert_eq!(
            CoreBackend::new(ctx.clone()).modrinth_refusal(),
            Some(BackendError::NeedsAction(
                PackNeedsAction::EnableModrinth,
                MODRINTH_OFF_NOTE.into()
            )),
            "a normal import is still refused while the toggle is off"
        );
        assert_eq!(
            CoreBackend::with_modrinth_allowed_once(ctx.clone()).modrinth_refusal(),
            None,
            "the one-time allow lets Modrinth through for this import"
        );

        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        assert_eq!(
            crate::db::get_setting(&conn, "modrinth_enabled").unwrap(),
            Some(serde_json::json!(false)),
            "the global Modrinth setting is not changed by the one-time allow"
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn allowing_modrinth_once_still_respects_the_privacy_switch() {
        let (ctx, root) = modrinth_off_ctx();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        crate::db::set_setting(&conn, "network_modrinth_enabled", &serde_json::json!(false))
            .unwrap();
        drop(conn);

        assert_eq!(
            CoreBackend::with_modrinth_allowed_once(ctx.clone()).modrinth_refusal(),
            Some(BackendError::NeedsAction(
                PackNeedsAction::OpenSettings,
                "Modrinth catalog API is disabled in Privacy settings.".into()
            ))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    // -- Offline -------------------------------------------------------------

    fn network_error(code: &str) -> LauncherError {
        LauncherError::Generic {
            code: code.into(),
            message: "HTTP request failed for https://cdn.modrinth.com/x.jar: dns error".into(),
        }
    }

    #[test]
    fn offline_download_waits_with_the_offline_note_and_installs_nothing() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("1", "mod.jar")];
        backend.install_offline = true;
        let out = run(&backend, vec![entry("mod.jar", "1", Some(GOOD_PIN))]);
        assert_eq!(out[0].status, PackEntryStatus::NeedsAction);
        assert_eq!(
            out[0].action, None,
            "no Settings switch would fix a dropped connection"
        );
        assert_eq!(out[0].reason.as_deref(), Some(OFFLINE_NOTE));
        assert!(backend.installs.lock().unwrap().is_empty());
    }

    #[test]
    fn offline_import_with_nothing_installed_is_recognised_for_cleanup() {
        let mut backend = fake();
        backend.modrinth = vec![candidate("1", "a.jar"), candidate("1", "b.jar")];
        backend.install_offline = true;
        let out = run(
            &backend,
            vec![entry("a.jar", "1", None), entry("b.jar", "1", None)],
        );
        let report = PackImportReport::from_outcomes("inst", "Pack", out, Vec::new());
        assert_eq!(report.installed, 0);
        assert!(nothing_installed_for_lack_of_connection(&report));
    }

    #[test]
    fn a_partial_or_non_connection_failure_is_not_treated_as_offline_only() {
        // One installed: the instance is kept.
        let mut partly = fake();
        partly.modrinth = vec![candidate("1", "a.jar"), candidate("1", "b.jar")];
        partly.install_error = Some(("b.jar".into(), "disk full".into()));
        let out = run(
            &partly,
            vec![entry("a.jar", "1", None), entry("b.jar", "1", None)],
        );
        let report = PackImportReport::from_outcomes("inst", "Pack", out, Vec::new());
        assert!(!nothing_installed_for_lack_of_connection(&report));

        // Modrinth off is an action, not a connection problem.
        let mut off = fake();
        off.modrinth_on = false;
        let out = run(&off, vec![entry("xaero.jar", "1", None)]);
        let report = PackImportReport::from_outcomes("inst", "Pack", out, Vec::new());
        assert!(!nothing_installed_for_lack_of_connection(&report));

        // An empty pack is not "lost to the connection".
        let report = PackImportReport::from_outcomes("inst", "Pack", Vec::new(), Vec::new());
        assert!(!nothing_installed_for_lack_of_connection(&report));
    }

    #[test]
    fn only_a_send_failure_counts_as_unreachable() {
        assert_eq!(
            install_error(network_error("ERR_NETWORK")),
            BackendError::Offline
        );
        // The privacy switch and a hash mismatch are not connection failures.
        assert!(matches!(
            install_error(network_error("ERR_NETWORK_DISABLED")),
            BackendError::Failed(_)
        ));
        assert!(matches!(
            install_error(LauncherError::HashMismatch),
            BackendError::Failed(_)
        ));
    }

    // -- Review fingerprint --------------------------------------------------

    fn pack_bytes(mod_file: &str) -> Vec<u8> {
        format!(
            "{{\"instance\":{{\"name\":\"Friend\",\"minecraft_version\":\"1.21\",\"loader\":\"fabric\",\"loader_version\":\"0.16.0\"}},\"mods\":[{{\"filename\":\"{mod_file}\",\"version\":\"1\",\"modrinth_id\":\"proj\",\"sha256\":null}}]}}"
        )
        .into_bytes()
    }

    #[test]
    fn file_changed_between_review_and_import_is_refused() {
        let reviewed = pack_bytes("sodium.jar");
        let fingerprint = pack_fingerprint(&reviewed);
        // Same name, different content on disk now: the review is out of date.
        let changed = pack_bytes("evil.jar");

        let err = parse_reviewed_agora_pack(&changed, Some(&fingerprint)).unwrap_err();
        assert_eq!(err.code(), "ERR_PACK_CHANGED");
        assert!(err.to_string().contains("Review it again"), "{err}");

        // The bytes that were reviewed still import.
        assert!(parse_reviewed_agora_pack(&reviewed, Some(&fingerprint)).is_ok());
    }

    #[test]
    fn import_without_a_review_is_refused() {
        let bytes = pack_bytes("sodium.jar");
        let err = parse_reviewed_agora_pack(&bytes, None).unwrap_err();
        assert_eq!(err.code(), "ERR_PACK_NOT_REVIEWED");
    }

    #[test]
    fn the_reviewed_hash_is_matched_exactly_but_not_by_case_or_padding() {
        let bytes = pack_bytes("sodium.jar");
        let fingerprint = pack_fingerprint(&bytes);
        assert!(parse_reviewed_agora_pack(&bytes, Some(&fingerprint.to_uppercase())).is_ok());
        assert!(parse_reviewed_agora_pack(&bytes, Some(&format!("  {fingerprint}\n"))).is_ok());
        // Empty, blank or truncated hashes fail closed.
        for bad in ["", "   ", &fingerprint[..10]] {
            let err = parse_reviewed_agora_pack(&bytes, Some(bad)).unwrap_err();
            assert_eq!(err.code(), "ERR_PACK_CHANGED", "hash {bad:?}");
        }
    }

    #[test]
    fn reviewed_import_refuses_an_oversized_file_before_hashing_it() {
        let bytes = vec![b' '; (MAX_PACK_FILE_BYTES + 1) as usize];
        let err = parse_reviewed_agora_pack(&bytes, Some(&pack_fingerprint(&bytes))).unwrap_err();
        assert!(err.to_string().contains("larger than 5 MB"), "{err}");
    }

    #[test]
    fn the_plan_records_the_exact_file_it_reviewed() {
        let (ctx, root) = temp_ctx_with_instance();
        let bytes = pack_bytes("sodium.jar");
        let plan = plan_agora_pack_bytes(&ctx, &bytes).unwrap();
        assert_eq!(plan.file_sha256, pack_fingerprint(&bytes));
        assert_eq!(plan.file_size, bytes.len() as u64);
        let _ = std::fs::remove_dir_all(root);
    }
}
