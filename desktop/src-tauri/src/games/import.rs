//! Import: bringing in a Mod Organizer 2 setup (MASTER_SPEC §26.10).
//! Owned by the `import` slice. A thin adapter over `agora_core::game_import`: the CLI's
//! `agora games import mo2` does the same work through the same functions.

use std::path::{Path, PathBuf};

use agora_core::content_store::Mo2Provenance;

use agora_core::game_import::{
    self, ImportProgress, Mo2Plan, Mo2Request, Mo2RunReport, RunOptions, ScanReport, SetupFound,
};
use agora_core::game_registry::installed_games;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

use super::error::{GamesError, CORE_FAILED, TASK_FAILED};

/// The event each progress report is sent on while a run stores mods.
pub const IMPORT_PROGRESS_EVENT: &str = "games-import-progress";

const INI_NAME: &str = "ModOrganizer.ini";
/// The request named a path that cannot be a setup (empty, missing, or not the setup's ini).
const BAD_PATH: &str = "bad_path";

// ---------------------------------------------------------------------------
// Finding setups
// ---------------------------------------------------------------------------

/// The setups on this machine, or the one folder the user chose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mo2ScanReply {
    pub setups: Vec<SetupDto>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupDto {
    pub ini_path: String,
    pub game_name: Option<String>,
    pub profiles: Vec<String>,
    /// `None` when the setup's `mods` folder cannot be read.
    pub mod_count: Option<usize>,
    pub found_by: String,
}

impl From<SetupFound> for SetupDto {
    fn from(found: SetupFound) -> Self {
        Self {
            ini_path: found.ini_path.to_string_lossy().into_owned(),
            game_name: found.game_name,
            profiles: found.profiles,
            mod_count: found.mod_count,
            found_by: found.found_by,
        }
    }
}

impl From<ScanReport> for Mo2ScanReply {
    fn from(report: ScanReport) -> Self {
        Self {
            setups: report.setups.into_iter().map(SetupDto::from).collect(),
            warnings: report.warnings,
        }
    }
}

/// Look for Mod Organizer 2 setups. With `chosen` (a folder the user picked, or the setup's
/// `ModOrganizer.ini`), summarise that one setup instead, and say why it is not one if it is not.
/// Scanning reads the drives, so the work runs on a blocking thread.
#[tauri::command]
pub async fn games_import_mo2_scan(chosen: Option<String>) -> Result<Mo2ScanReply, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<Mo2ScanReply, GamesError> {
        match chosen {
            None => Ok(game_import::scan().into()),
            Some(folder) => {
                let ini = resolve_setup_path(&folder)?;
                let found = game_import::inspect_setup(&ini).map_err(core_failed)?;
                Ok(Mo2ScanReply {
                    setups: vec![found.into()],
                    warnings: Vec::new(),
                })
            }
        }
    })
    .await
    .map_err(|_| {
        GamesError::new(
            TASK_FAILED,
            "Looking for Mod Organizer 2 setups did not finish.",
        )
    })?
}

/// The `ModOrganizer.ini` a user's choice points at: the setup's folder, or the file itself.
/// Anything else is refused with the reason, so a wrong pick never reads as "no setups".
pub fn resolve_setup_path(input: &str) -> Result<PathBuf, GamesError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(bad_path(
            "Pick the folder your Mod Organizer 2 setup is in (the one that holds ModOrganizer.ini).",
        ));
    }
    let path = Path::new(trimmed);
    if path.is_dir() {
        let ini = path.join(INI_NAME);
        if ini.is_file() {
            return Ok(ini);
        }
        return Err(bad_path(format!(
            "There is no {INI_NAME} in {}. Pick the folder Mod Organizer 2 keeps that file in.",
            path.display()
        )));
    }
    if path.is_file() {
        let is_ini = path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(INI_NAME));
        if is_ini {
            return Ok(path.to_path_buf());
        }
        return Err(bad_path(format!(
            "{} is not {INI_NAME}. Pick the folder the setup is in instead.",
            path.display()
        )));
    }
    Err(bad_path(format!("Nothing exists at {}.", path.display())))
}

fn bad_path(message: impl Into<String>) -> GamesError {
    GamesError::new(BAD_PATH, message)
}

// ---------------------------------------------------------------------------
// Plan and run
// ---------------------------------------------------------------------------

/// One import request. `dry_run` plans and reports; otherwise the profile is imported.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mo2ImportRequest {
    /// The folder or `ModOrganizer.ini` the user chose.
    pub setup_path: String,
    pub profile: String,
    /// The new instance's name. Needed only to import the same setup and profile again.
    pub name: Option<String>,
    pub copy_saves: bool,
    pub dry_run: bool,
}

/// What `games_import_mo2` answered: the plan for a dry run, or the report of an import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mo2ImportReply {
    Plan(Box<PlanDto>),
    Run(Box<RunDto>),
}

/// Progress of a long import, sent as the `games-import-progress` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgressDto {
    pub mods_done: usize,
    pub mods_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Plan or import a profile. Reads the setup and stores mods, so the work runs on a blocking
/// thread; progress goes out as events while mods are stored.
#[tauri::command]
pub async fn games_import_mo2(
    app: tauri::AppHandle,
    request: Mo2ImportRequest,
) -> Result<Mo2ImportReply, GamesError> {
    tokio::task::spawn_blocking(move || -> Result<Mo2ImportReply, GamesError> {
        let ctx = crate::core_context(&app)?;
        let ini_path = resolve_setup_path(&request.setup_path)?;
        let name = request
            .name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty());
        let core_request = Mo2Request {
            ini_path,
            profile: request.profile,
            name,
            copy_saves: request.copy_saves,
        };
        let inventory = installed_games(&ctx);
        if request.dry_run {
            let plan = game_import::plan(&ctx, &inventory, &core_request).map_err(core_failed)?;
            return Ok(Mo2ImportReply::Plan(Box::new(PlanDto::from(&plan))));
        }
        let emitter = app.clone();
        let report = game_import::run(
            &ctx,
            &inventory,
            &core_request,
            RunOptions::default(),
            &|progress: ImportProgress| {
                let payload = ImportProgressDto {
                    mods_done: progress.mods_done,
                    mods_total: progress.mods_total,
                    bytes_done: progress.bytes_done,
                    bytes_total: progress.bytes_total,
                };
                // A progress event that cannot be delivered does not change the import.
                let _ = emitter.emit(IMPORT_PROGRESS_EVENT, payload);
            },
        )
        .map_err(core_failed)?;
        Ok(Mo2ImportReply::Run(Box::new(RunDto::from(&report))))
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Importing the setup did not finish."))?
}

fn core_failed(error: impl std::fmt::Display) -> GamesError {
    GamesError::new(CORE_FAILED, error.to_string())
}

/// The plan, in words and numbers the import screen shows. Nothing here is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDto {
    pub setup: String,
    pub profile: String,
    pub mo2_game_name: String,
    pub game_id: String,
    pub install_location: String,
    pub store: String,
    pub instance_name: String,
    pub mods: Vec<PlannedModDto>,
    pub mods_enabled: usize,
    pub mods_disabled: usize,
    pub files: usize,
    pub bytes: u64,
    /// Top-level `meta.ini` files left out of the stored content, across all mods.
    pub meta_ini_excluded: usize,
    pub separators: usize,
    /// Entries MO2 lists that Agora does not import, by name.
    pub unmanaged: Vec<String>,
    /// Lines and folders that were skipped, each with the reason.
    pub problems: Vec<ProblemDto>,
    pub overwrite: OverwriteDto,
    pub plugins: PluginsDto,
    pub local_settings: bool,
    pub inis: Vec<String>,
    pub saves: SavesDto,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedModDto {
    pub line: usize,
    pub folder: String,
    pub enabled: bool,
    pub files: usize,
    pub bytes: u64,
    /// What the mod's `meta.ini` claims, in words. A claim is never checked against a download.
    pub provenance: String,
    pub plugin_files: Vec<String>,
    pub meta_ini_excluded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemDto {
    pub line: Option<usize>,
    pub entry: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverwriteDto {
    pub present: bool,
    pub files: usize,
    pub bytes: u64,
    /// Files a known tool produced, which become that tool's generated layer (inputs unknown).
    pub generated: Vec<GeneratedDto>,
    pub rest_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedDto {
    pub name: String,
    pub files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginsDto {
    /// Whether the game keeps a plugin list at all.
    pub kept: bool,
    pub listed: usize,
    pub active: usize,
    pub locked: Vec<String>,
    /// Plugins the game always loads, which MO2 lists but Agora does not write.
    pub implicit_skipped: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavesDto {
    /// The profile keeps its own saves.
    pub local: bool,
    pub files: usize,
    pub bytes: u64,
    pub note: String,
}

impl From<&Mo2Plan> for PlanDto {
    fn from(plan: &Mo2Plan) -> Self {
        Self {
            setup: plan.setup.to_string_lossy().into_owned(),
            profile: plan.profile.clone(),
            mo2_game_name: plan.mo2_game_name.clone(),
            game_id: plan.game.as_str().to_string(),
            install_location: plan.install_location.to_string_lossy().into_owned(),
            store: plan.store.clone(),
            instance_name: plan.instance_name.clone(),
            mods: plan
                .mods
                .iter()
                .map(|m| PlannedModDto {
                    line: m.line,
                    folder: m.folder.clone(),
                    enabled: m.enabled,
                    files: m.files,
                    bytes: m.bytes,
                    provenance: provenance_text(&m.provenance),
                    plugin_files: m.plugin_files.clone(),
                    meta_ini_excluded: m.meta_ini_excluded,
                })
                .collect(),
            mods_enabled: plan.mods_enabled,
            mods_disabled: plan.mods_disabled,
            files: plan.files,
            bytes: plan.bytes,
            meta_ini_excluded: plan.meta_ini_excluded,
            separators: plan.separators,
            unmanaged: plan.unmanaged.clone(),
            problems: plan
                .problems
                .iter()
                .map(|p| ProblemDto {
                    line: p.line,
                    entry: p.entry.clone(),
                    reason: p.reason.clone(),
                })
                .collect(),
            overwrite: OverwriteDto {
                present: plan.overwrite.present,
                files: plan.overwrite.files,
                bytes: plan.overwrite.bytes,
                generated: plan
                    .overwrite
                    .generated
                    .iter()
                    .map(|g| GeneratedDto {
                        name: g.name.clone(),
                        files: g.files,
                    })
                    .collect(),
                rest_files: plan.overwrite.rest_files,
            },
            plugins: PluginsDto {
                kept: plan.plugins.kept,
                listed: plan.plugins.lines.len(),
                active: plan.plugins.active,
                locked: plan.plugins.locked.clone(),
                implicit_skipped: plan.plugins.implicit_skipped.clone(),
                notes: plan.plugins.notes.clone(),
            },
            local_settings: plan.local_settings,
            inis: plan.inis.iter().map(|i| i.name.clone()).collect(),
            saves: SavesDto {
                local: plan.saves.local,
                files: plan.saves.files,
                bytes: plan.saves.bytes,
                note: plan.saves.note.clone(),
            },
            warnings: plan.warnings.clone(),
        }
    }
}

/// What an import did, after its plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDto {
    pub instance_id: Option<String>,
    pub instance_name: String,
    pub base_id: Option<String>,
    /// `true` when the base was built by this import, `false` when an existing one was reused.
    pub base_built: Option<bool>,
    pub stored_mods: usize,
    pub content_layers: usize,
    pub disabled_layers: usize,
    /// Mods whose bytes match a layer the instance already has.
    pub duplicates: Vec<String>,
    pub generated_layers: Vec<String>,
    pub plugins_written: Option<usize>,
    pub plugin_findings: Vec<String>,
    pub inis_copied: Vec<String>,
    pub saves_copied: usize,
    pub saves_skipped_existing: usize,
    pub saves_folder: Option<String>,
    /// What still has to be done by hand, including each skipped line and why.
    pub next_steps: Vec<String>,
}

impl From<&Mo2RunReport> for RunDto {
    fn from(report: &Mo2RunReport) -> Self {
        Self {
            instance_id: report.instance_id.clone(),
            instance_name: report.plan.instance_name.clone(),
            base_id: report.base_id.clone(),
            base_built: report.base_built,
            stored_mods: report.stored_mods,
            content_layers: report.content_layers,
            disabled_layers: report.disabled_layers,
            duplicates: report.duplicates.clone(),
            generated_layers: report.generated_layers.clone(),
            plugins_written: report.plugins_written,
            plugin_findings: report.plugin_findings.clone(),
            inis_copied: report.inis_copied.clone(),
            saves_copied: report.saves_copied,
            saves_skipped_existing: report.saves_skipped_existing,
            saves_folder: report
                .saves_folder
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            next_steps: report.next_steps.clone(),
        }
    }
}

/// A provenance claim as a person reads it. Claims are shown as claims, never as verified.
fn provenance_text(provenance: &Mo2Provenance) -> String {
    match provenance {
        Mo2Provenance::Claimed {
            modid: Some(id), ..
        } => format!("claims Nexus mod {id}, not verified"),
        Mo2Provenance::Claimed { modid: None, .. } => "meta.ini has no mod id".to_string(),
        Mo2Provenance::Absent => "no meta.ini".to_string(),
        Mo2Provenance::Unreadable { reason } => format!("meta.ini unreadable: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agora-import-test-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp folder can be made");
        dir
    }

    #[test]
    fn a_setup_folder_resolves_to_its_ini() {
        let dir = temp_dir("folder");
        std::fs::write(dir.join(INI_NAME), "[General]\n").unwrap();
        let resolved = resolve_setup_path(&dir.to_string_lossy()).unwrap();
        assert_eq!(resolved, dir.join(INI_NAME));
    }

    #[test]
    fn the_ini_file_itself_is_accepted_in_any_case() {
        let dir = temp_dir("ini-case");
        let ini = dir.join("modorganizer.INI");
        std::fs::write(&ini, "[General]\n").unwrap();
        assert_eq!(resolve_setup_path(&ini.to_string_lossy()).unwrap(), ini);
    }

    #[test]
    fn empty_and_whitespace_paths_are_refused_with_the_pick_prompt() {
        for input in ["", "   \t "] {
            let error = resolve_setup_path(input).expect_err(input);
            assert_eq!(error.code, BAD_PATH, "{input:?}");
            assert!(
                error.message.starts_with("Pick the folder"),
                "{input:?}: {}",
                error.message
            );
        }
    }

    #[test]
    fn a_folder_without_an_ini_says_so() {
        let dir = temp_dir("no-ini");
        let error = resolve_setup_path(&dir.to_string_lossy()).unwrap_err();
        assert_eq!(error.code, BAD_PATH);
        assert!(error.message.contains(INI_NAME), "{}", error.message);
    }

    #[test]
    fn a_file_that_is_not_the_setup_ini_is_refused() {
        let dir = temp_dir("other-file");
        let other = dir.join("plugins.txt");
        std::fs::write(&other, "").unwrap();
        let error = resolve_setup_path(&other.to_string_lossy()).unwrap_err();
        assert_eq!(error.code, BAD_PATH);
        assert!(
            error.message.contains("not ModOrganizer.ini"),
            "{}",
            error.message
        );
    }

    #[test]
    fn a_missing_path_is_refused_with_its_name() {
        let dir = temp_dir("missing");
        let missing = dir.join("nope");
        let error = resolve_setup_path(&missing.to_string_lossy()).unwrap_err();
        assert_eq!(error.code, BAD_PATH);
        assert!(
            error.message.contains("Nothing exists"),
            "{}",
            error.message
        );
    }

    #[test]
    fn a_claimed_mod_id_is_described_as_unverified() {
        let claimed = Mo2Provenance::Claimed {
            modid: Some(42),
            version: None,
            installation_file: None,
            repository: None,
            game_name: None,
            note: String::new(),
        };
        assert_eq!(
            provenance_text(&claimed),
            "claims Nexus mod 42, not verified"
        );
        assert_eq!(provenance_text(&Mo2Provenance::Absent), "no meta.ini");
    }
}
