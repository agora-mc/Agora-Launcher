//! FOMOD installers: the option pages a mod archive asks the user to answer.
//! Owned by the `fomod` slice.
//!
//! The wizard reads an installer (`games_fomod_show`), proposes choices, shows the files those
//! choices would install (`games_fomod_install` with `preview`), and installs them. Each command is
//! a thin adapter over `agora_core::content_fomod`, the same calls `agora games content fomod`
//! makes; the core refuses every choice it cannot check, and the desktop shows that refusal.

use agora_core::content_fomod::{
    self as fomod, Choice, Condition, FomodContext, FomodError, FomodInstaller, GroupType,
    PluginType, TypeDescriptor,
};
use agora_core::content_store;
use agora_core::ctx::Ctx;
use agora_core::game_deploy;
use agora_core::game_instance;
use serde::Serialize;

use super::content::{checked_id, content_error, deploy_error, instance_error};
use super::error::{GamesError, CORE_FAILED, TASK_FAILED};

// ---------------------------------------------------------------------------
// Replies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallerView {
    pub item_id: String,
    /// The installer's name: its module name, else its info name, else a plain placeholder.
    pub title: String,
    pub author: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    /// What the installer needs before it can run, in words (`None` when it needs nothing).
    pub requires: Option<String>,
    /// Files every install gets whatever the user picks.
    pub always_installed_count: usize,
    /// Extra files that depend on conditions, installed when those hold.
    pub conditional_install_count: usize,
    pub steps: Vec<StepView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepView {
    pub name: String,
    /// When the step is shown, in words. `None` means it is always shown.
    pub visible: Option<String>,
    pub groups: Vec<GroupView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub name: String,
    /// The rule in words, e.g. "select exactly one".
    pub rule: String,
    /// True when one option at most can be chosen, so the UI offers one choice (radio buttons).
    pub single_choice: bool,
    pub plugins: Vec<PluginView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginView {
    pub name: String,
    pub description: String,
    /// `required`, `optional`, `recommended`, `not_usable`, `could_be_usable`, or `dependent`
    /// (the type depends on the choices made, and is worked out when the choices are checked).
    pub plugin_type: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowView {
    pub installer: InstallerView,
    /// The choices Agora proposes, when the caller asked for them (`use_defaults`). Required and
    /// recommended options are in it. `None` when no proposal was asked for.
    pub suggested_choices: Option<Vec<Choice>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFileView {
    /// Relative to the folder the content goes to (the game's data folder, where the game has one).
    pub destination: String,
    /// Where the file comes from inside the archive.
    pub source: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    pub files: Vec<PlannedFileView>,
    pub total_size: u64,
    /// Things the user should know: skipped files, conflicts, conditions Agora assumed.
    pub notes: Vec<String>,
    /// The choices as they were applied, with required options filled in.
    pub choices: Vec<Choice>,
    /// The game's data folder the files go into, or `""` when the game has none or no instance
    /// was named.
    pub data_folder: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledView {
    /// The new store item the installer made.
    pub item_id: String,
    pub name: String,
    pub file_count: usize,
    pub total_size: u64,
    pub notes: Vec<String>,
    pub added_to_instance: bool,
    pub layer_id: Option<String>,
    pub mount_path: Option<String>,
}

/// A preview of what an install would do, or the install itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FomodInstallResult {
    Preview(PlanView),
    Installed(InstalledView),
}

// ---------------------------------------------------------------------------
// Pure mappings, tested below
// ---------------------------------------------------------------------------

fn plugin_type_name(descriptor: &TypeDescriptor) -> &'static str {
    match descriptor {
        TypeDescriptor::Simple { plugin_type } => simple_type_name(*plugin_type),
        TypeDescriptor::Dependent { .. } => "dependent",
    }
}

fn simple_type_name(plugin_type: PluginType) -> &'static str {
    match plugin_type {
        PluginType::Required => "required",
        PluginType::Optional => "optional",
        PluginType::Recommended => "recommended",
        PluginType::NotUsable => "not_usable",
        PluginType::CouldBeUsable => "could_be_usable",
    }
}

pub fn installer_view(installer: &FomodInstaller) -> InstallerView {
    let info = installer.info.as_ref();
    let title = if !installer.module_name.trim().is_empty() {
        installer.module_name.clone()
    } else if let Some(name) = info
        .and_then(|i| i.name.clone())
        .filter(|n| !n.trim().is_empty())
    {
        name
    } else {
        "(unnamed installer)".to_string()
    };
    InstallerView {
        item_id: installer.item_id.clone(),
        title,
        author: info.and_then(|i| i.author.clone()),
        version: info.and_then(|i| i.version.clone()),
        description: info.and_then(|i| i.description.clone()),
        requires: installer
            .module_dependencies
            .as_ref()
            .map(Condition::describe),
        always_installed_count: installer.required_files.len(),
        conditional_install_count: installer.conditional_installs.len(),
        steps: installer
            .steps
            .iter()
            .map(|step| StepView {
                name: step.name.clone(),
                visible: step.visible.as_ref().map(Condition::describe),
                groups: step
                    .groups
                    .iter()
                    .map(|group| GroupView {
                        name: group.name.clone(),
                        rule: group.group_type.rule().to_string(),
                        single_choice: matches!(
                            group.group_type,
                            GroupType::SelectExactlyOne | GroupType::SelectAtMostOne
                        ),
                        plugins: group
                            .plugins
                            .iter()
                            .map(|plugin| PluginView {
                                name: plugin.name.clone(),
                                description: plugin.description.clone(),
                                plugin_type: plugin_type_name(&plugin.type_descriptor),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn plan_view(plan: &fomod::InstallPlan, data_folder: String) -> PlanView {
    PlanView {
        files: plan
            .files
            .iter()
            .map(|file| PlannedFileView {
                destination: file.destination.as_str().to_string(),
                source: file.source.as_str().to_string(),
                size: file.size,
            })
            .collect(),
        total_size: plan.files.iter().map(|file| file.size).sum(),
        notes: plan.notes.clone(),
        choices: plan.choices.clone(),
        data_folder,
    }
}

/// A FOMOD refusal, with the core's own words. The core says what it could not check and why.
pub(super) fn fomod_error(error: FomodError) -> GamesError {
    GamesError::new(CORE_FAILED, error.to_string())
}

// ---------------------------------------------------------------------------
// Instance helpers
// ---------------------------------------------------------------------------

/// What the named instance has, for the installer's file and version checks. No instance means
/// the checks have nothing to answer, so they are unknown (and the core says so when it matters).
fn context_for(ctx: &Ctx, instance_id: Option<&str>) -> Result<Option<FomodContext>, GamesError> {
    match instance_id {
        None => Ok(None),
        Some(id) => fomod::instance_context(ctx, id)
            .map(Some)
            .map_err(fomod_error),
    }
}

/// The game's data folder for an instance (`""` when the game installs at its root). Refuses a
/// game with no content rules, since Agora cannot say where an installer's files go then.
fn data_folder(ctx: &Ctx, instance_id: &str) -> Result<String, GamesError> {
    let manifest = game_instance::get_manifest(ctx, instance_id).map_err(instance_error)?;
    let layout = ctx
        .games
        .game(&manifest.game)
        .and_then(|game| game.content_layout.as_ref())
        .ok_or_else(|| {
            GamesError::new(
                CORE_FAILED,
                format!(
                    "The game '{}' has no content rules, so Agora does not know where an installer's files go.",
                    manifest.game
                ),
            )
        })?;
    Ok(layout.data_path.as_str().to_string())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// An archive's installer: its steps and options. With `use_defaults`, also the choices Agora
/// proposes. Refuses (with the core's reason) when a choice depends on something Agora cannot
/// check, so the user picks that option by hand.
#[tauri::command]
pub async fn games_fomod_show(
    app: tauri::AppHandle,
    content_id: String,
    instance_id: Option<String>,
    use_defaults: bool,
) -> Result<ShowView, GamesError> {
    let content_id = checked_id(&content_id)?;
    let instance_id = instance_id.as_deref().map(checked_id).transpose()?;
    tokio::task::spawn_blocking(move || -> Result<ShowView, GamesError> {
        let ctx = crate::core_context(&app)?;
        let item_id = content_store::resolve_item_id(&ctx, &content_id).map_err(content_error)?;
        let installer = fomod::parse(&ctx, &item_id).map_err(fomod_error)?;
        let suggested_choices = if use_defaults {
            let context = context_for(&ctx, instance_id.as_deref())?;
            Some(fomod::defaults(&installer, context.as_ref()).map_err(fomod_error)?)
        } else {
            None
        };
        Ok(ShowView {
            installer: installer_view(&installer),
            suggested_choices,
        })
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Reading the installer did not finish."))?
}

/// Check the user's choices. With `preview`, return the files they would install and change
/// nothing. Without it, install them as a new store item, and add that item to the instance if one
/// is named (in the game's data folder, as an ordinary content item).
#[tauri::command]
pub async fn games_fomod_install(
    app: tauri::AppHandle,
    content_id: String,
    instance_id: Option<String>,
    choices: Vec<Choice>,
    preview: bool,
) -> Result<FomodInstallResult, GamesError> {
    let content_id = checked_id(&content_id)?;
    let instance_id = instance_id.as_deref().map(checked_id).transpose()?;
    tokio::task::spawn_blocking(move || -> Result<FomodInstallResult, GamesError> {
        let ctx = crate::core_context(&app)?;
        let item_id = content_store::resolve_item_id(&ctx, &content_id).map_err(content_error)?;
        let context = context_for(&ctx, instance_id.as_deref())?;

        if preview {
            let installer = fomod::parse(&ctx, &item_id).map_err(fomod_error)?;
            let plan =
                fomod::evaluate(&installer, &choices, context.as_ref()).map_err(fomod_error)?;
            let folder = match instance_id.as_deref() {
                Some(id) => data_folder(&ctx, id)?,
                None => String::new(),
            };
            return Ok(FomodInstallResult::Preview(plan_view(&plan, folder)));
        }

        // The mount is the game's data folder; a game installing at its root mounts at the root.
        let mount = match instance_id.as_deref() {
            Some(id) => Some(data_folder(&ctx, id)?).filter(|folder| !folder.is_empty()),
            None => None,
        };
        let (_installer, plan, outcome) =
            fomod::install(&ctx, &item_id, &choices, context.as_ref()).map_err(fomod_error)?;
        let derived = outcome.item();
        let notes = plan.notes.clone();

        let (added_to_instance, layer_id, mount_path) = match instance_id.as_deref() {
            None => (false, None, None),
            Some(id) => {
                let layer = game_deploy::add_content(&ctx, id, &derived.item_id, mount.as_deref(), None)
                    .map_err(|error| {
                        GamesError::new(
                            CORE_FAILED,
                            format!(
                                "The installer made item {} but it could not be added to the instance: {}",
                                derived.item_id,
                                deploy_error(error).message
                            ),
                        )
                    })?;
                (
                    true,
                    Some(layer.id.as_str().to_string()),
                    Some(layer.mount_path.as_str().to_string()),
                )
            }
        };

        Ok(FomodInstallResult::Installed(InstalledView {
            item_id: derived.item_id.clone(),
            name: derived.name.clone(),
            file_count: derived.files.len(),
            total_size: derived.total_size,
            notes,
            added_to_instance,
            layer_id,
            mount_path,
        }))
    })
    .await
    .map_err(|_| GamesError::new(TASK_FAILED, "Installing the options did not finish."))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// An installer as the core parses it: snake_case keys, as `FomodInstaller` declares them.
    fn installer(
        module_name: &str,
        info: serde_json::Value,
        steps: serde_json::Value,
    ) -> FomodInstaller {
        serde_json::from_value(json!({
            "item_id": "aaaa1111",
            "root": "",
            "module_name": module_name,
            "info": info,
            "module_dependencies": null,
            "required_files": [],
            "steps": steps,
            "conditional_installs": []
        }))
        .expect("an installer deserializes")
    }

    fn plugin(name: &str, plugin_type: &str) -> serde_json::Value {
        json!({
            "name": name,
            "description": format!("About {name}"),
            "image": null,
            "files": [],
            "flags": [],
            "type_descriptor": { "kind": "simple", "plugin_type": plugin_type }
        })
    }

    #[test]
    fn every_simple_plugin_type_has_a_name_the_ui_can_show() {
        let names = [
            (PluginType::Required, "required"),
            (PluginType::Optional, "optional"),
            (PluginType::Recommended, "recommended"),
            (PluginType::NotUsable, "not_usable"),
            (PluginType::CouldBeUsable, "could_be_usable"),
        ];
        for (plugin_type, expected) in names {
            assert_eq!(
                plugin_type_name(&TypeDescriptor::Simple { plugin_type }),
                expected
            );
        }
    }

    #[test]
    fn a_type_that_depends_on_choices_is_named_as_such_not_guessed() {
        let descriptor = TypeDescriptor::Dependent {
            default: PluginType::Optional,
            patterns: Vec::new(),
        };
        assert_eq!(plugin_type_name(&descriptor), "dependent");
    }

    #[test]
    fn a_single_choice_group_is_radio_and_a_multi_choice_group_is_not() {
        let view = installer_view(&installer(
            "Pack",
            json!(null),
            json!([{
                "name": "Look",
                "visible": null,
                "groups": [
                    { "name": "Body", "group_type": "select_exactly_one", "plugins": [plugin("Slim", "recommended")] },
                    { "name": "Extras", "group_type": "select_any", "plugins": [plugin("Glow", "optional")] }
                ]
            }]),
        ));
        let groups = &view.steps[0].groups;
        assert!(groups[0].single_choice);
        assert_eq!(groups[0].rule, "select exactly one");
        assert!(!groups[1].single_choice);
        assert_eq!(groups[1].plugins[0].plugin_type, "optional");
    }

    #[test]
    fn a_step_shows_when_it_is_visible_in_words() {
        let view = installer(
            "Pack",
            json!(null),
            json!([{
                "name": "Patch",
                "visible": { "type": "flag", "flag": "patched", "value": "yes" },
                "groups": []
            }]),
        );
        let shown = installer_view(&view).steps[0].visible.clone();
        assert_eq!(shown.as_deref(), Some("option flag 'patched' is 'yes'"));
    }

    #[test]
    fn an_installer_with_no_name_is_titled_plainly_not_blank() {
        assert_eq!(
            installer_view(&installer("", json!(null), json!([]))).title,
            "(unnamed installer)"
        );
        assert_eq!(
            installer_view(&installer("  ", json!(null), json!([]))).title,
            "(unnamed installer)"
        );
        let from_info = installer(
            "",
            json!({ "name": "From Info", "author": null, "version": "2.0", "website": null, "description": null }),
            json!([]),
        );
        let view = installer_view(&from_info);
        assert_eq!(view.title, "From Info");
        assert_eq!(view.version.as_deref(), Some("2.0"));
    }

    #[test]
    fn a_refusal_from_the_core_reaches_the_user_with_its_own_words() {
        let error = fomod_error(FomodError::NothingToInstall);
        assert_eq!(error.code, CORE_FAILED);
        assert_eq!(error.message, "the chosen options install no files");
    }

    #[test]
    fn an_unknown_condition_refusal_is_kept_not_turned_into_a_default() {
        let error = fomod_error(FomodError::UnknownCondition {
            what: "a step".into(),
            condition: "Skyrim version 1.6".into(),
            reason: "the game version is unknown".into(),
        });
        assert_eq!(error.code, CORE_FAILED);
        assert!(error.message.contains("Skyrim version 1.6"));
        assert!(error.message.contains("the game version is unknown"));
    }
}
