//! Declarative contributions: what a plugin adds to the launcher's surfaces.
//!
//! Contributions are data, not code. They are read straight out of the
//! manifest, before any script runs, so the host can show a plugin's pages and
//! commands in the manager, detect a duplicate identifier, and render a
//! sensible fallback for a plugin that is installed but disabled or broken.
//!
//! Every identifier here is *local* to the plugin. The host namespaces it on
//! load — `acme.dashboard` contributing `overview` becomes
//! `acme.dashboard/overview` — so two plugins can both have an `overview`
//! without colliding, and a stored navigation destination always says which
//! plugin owned it.

use serde::{Deserialize, Serialize};

/// Everything a manifest may contribute to the launcher.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contributions {
    /// Full-width destinations reachable from the sidebar.
    #[serde(default)]
    pub pages: Vec<PageContribution>,
    /// Panels shown inside the instance editor for a selected instance.
    #[serde(default)]
    pub instance_panels: Vec<InstancePanelContribution>,
    /// Actions in the command palette and, optionally, context menus.
    #[serde(default)]
    pub commands: Vec<CommandContribution>,
    /// Typed settings rendered by the host in the plugin's settings section.
    #[serde(default)]
    pub settings: Vec<SettingDefinition>,
    /// Semantic theme tokens. Declarative only — a theme needs no script.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeContribution>,
    /// Diagnostic checks the plugin can run against an instance.
    #[serde(default)]
    pub diagnostics: Vec<DiagnosticContribution>,
    /// Bounded checks that run before a launch, when the user opts in.
    #[serde(default)]
    pub launch_checks: Vec<LaunchCheckContribution>,
    /// Built-in surfaces this plugin offers to render instead of Agora.
    ///
    /// An offer, never a takeover — see [`ReplacementContribution`].
    #[serde(default)]
    pub replacements: Vec<ReplacementContribution>,
}

impl Contributions {
    /// Every local identifier this manifest claims, with the surface it is on.
    ///
    /// The host uses this to detect a plugin colliding with itself, and to
    /// detect two enabled plugins colliding with each other after namespacing.
    pub fn local_ids(&self) -> Vec<(ContributionKind, &str)> {
        let mut ids = Vec::new();
        if let Some(theme) = &self.theme {
            ids.push((ContributionKind::Theme, theme.id.as_str()));
        }
        ids.extend(
            self.pages
                .iter()
                .map(|c| (ContributionKind::Page, c.id.as_str())),
        );
        ids.extend(
            self.instance_panels
                .iter()
                .map(|c| (ContributionKind::InstancePanel, c.id.as_str())),
        );
        ids.extend(
            self.commands
                .iter()
                .map(|c| (ContributionKind::Command, c.id.as_str())),
        );
        ids.extend(
            self.settings
                .iter()
                .map(|c| (ContributionKind::Setting, c.key.as_str())),
        );
        ids.extend(
            self.diagnostics
                .iter()
                .map(|c| (ContributionKind::Diagnostic, c.id.as_str())),
        );
        ids.extend(
            self.launch_checks
                .iter()
                .map(|c| (ContributionKind::LaunchCheck, c.id.as_str())),
        );
        ids.extend(
            self.replacements
                .iter()
                .map(|c| (ContributionKind::Replacement, c.id.as_str())),
        );
        ids
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
            && self.instance_panels.is_empty()
            && self.commands.is_empty()
            && self.settings.is_empty()
            && self.theme.is_none()
            && self.diagnostics.is_empty()
            && self.launch_checks.is_empty()
            && self.replacements.is_empty()
    }
}

/// Which surface a contribution belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContributionKind {
    Page,
    InstancePanel,
    Command,
    Setting,
    Diagnostic,
    LaunchCheck,
    Theme,
    Replacement,
}

impl ContributionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ContributionKind::Page => "page",
            ContributionKind::InstancePanel => "instance-panel",
            ContributionKind::Command => "command",
            ContributionKind::Setting => "setting",
            ContributionKind::Diagnostic => "diagnostic",
            ContributionKind::LaunchCheck => "launch-check",
            ContributionKind::Theme => "theme",
            ContributionKind::Replacement => "replacement",
        }
    }
}

impl std::fmt::Display for ContributionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a contributed surface gets its content on screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum ViewSource {
    /// The plugin exports a function returning a [`crate::dto::ViewModel`] and
    /// the host renders it with its own components. Themed, accessible,
    /// keyboard- and controller-navigable for free.
    Host {
        /// Name of the exported function on the plugin's entrypoint module.
        export: String,
    },
    /// **Withdrawn in API 0.1.** Kept only so that a manifest written against
    /// the prototype is refused with an explanation rather than an
    /// "unknown variant" parse error.
    ///
    /// This shipped a plugin's own HTML in a `data:` iframe. It was removed
    /// rather than finished, for a reason worth recording: the script inside
    /// that frame ran in the WebView, outside every bound the plugin runtime
    /// exists to impose. QuickJS plugins get a memory ceiling, an interrupt
    /// handler and a deadline; a 512 KiB HTML document got none of them and
    /// could hang the launcher with `while (true) {}`. A bounded file and a
    /// throttled command bridge do not make a bounded view.
    ///
    /// Accessibility and controller navigation were the author's problem
    /// inside the frame, in an application where controller support is a
    /// first-class feature. And the isolation it did provide could not be
    /// proven for the packaged app: Tauri documents that on some platforms it
    /// cannot distinguish IPC from an embedded frame from IPC from the window
    /// containing it.
    ///
    /// The host-rendered path exists precisely so none of that is an author's
    /// problem. If a real plugin needs an interaction `ViewModel` cannot
    /// express, the answer is a new block type, not a second renderer.
    Custom {
        /// Package-relative path to the entry HTML file.
        html: String,
    },
}

/// A full-width page reachable from the sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageContribution {
    pub id: String,
    pub title: String,
    /// A `lucide-react` icon name. Unknown names fall back to a generic icon
    /// rather than failing the plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub view: ViewSource,
}

/// A panel inside the instance editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstancePanelContribution {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub view: ViewSource,
}

/// A built-in surface a plugin may offer to render instead of Agora.
///
/// A closed set, deliberately. "Any component, addressed by name" would make
/// every internal rename a breaking change for plugins and would stop the
/// launcher from being refactored — which is exactly the trap the plan warns
/// about when it says not to depend on private selectors or internal
/// component injection. Adding a surface here is a considered decision with a
/// compatibility cost attached; there is no escape hatch that avoids paying it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReplaceableSurface {
    /// The first screen: what the launcher shows with nothing else selected.
    Home,
    /// The summary at the top of an opened instance, above its editor tabs.
    ///
    /// The replacement receives `{ instanceId }`, so one view serves every
    /// instance rather than the plugin having to guess which is open.
    InstanceOverview,
}

impl ReplaceableSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            ReplaceableSurface::Home => "home",
            ReplaceableSurface::InstanceOverview => "instance-overview",
        }
    }

    /// What the user is choosing between, in their words.
    pub fn title(self) -> &'static str {
        match self {
            ReplaceableSurface::Home => "Home",
            ReplaceableSurface::InstanceOverview => "Instance overview",
        }
    }

    /// Every surface, for a settings page that lists them.
    ///
    /// A surface belongs here only when the launcher can actually hand it
    /// over. Naming one a plugin could declare but never render would be
    /// worse than not offering it. Adding one is additive: an author gains an
    /// option and nothing already written stops working.
    pub const ALL: [ReplaceableSurface; 2] = [
        ReplaceableSurface::Home,
        ReplaceableSurface::InstanceOverview,
    ];
}

impl std::fmt::Display for ReplaceableSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReplaceableSurface {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "home" => Ok(ReplaceableSurface::Home),
            "instance-overview" => Ok(ReplaceableSurface::InstanceOverview),
            _ => Err(()),
        }
    }
}

/// An offer to render a built-in surface.
///
/// Declaring one does **not** take the surface over. It puts the plugin on a
/// list the user chooses from, and the built-in is what is chosen until they
/// say otherwise. That is the whole design:
///
/// - two plugins offering the same surface is a list of two, not a conflict
///   resolved by install order;
/// - a plugin that is disabled, removed, broken, or simply slow falls back to
///   the built-in rather than leaving a blank screen;
/// - the built-in is always reachable, so a replacement can never be a trap.
///
/// Only [`ViewSource::Host`] is permitted here. A replacement is the whole
/// screen, and the host-rendered path is the one that is themed, accessible
/// and controller-navigable by construction. Custom frames remain an additive
/// prototype; putting an unproven one where the home page used to be is the
/// definition of building a product on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplacementContribution {
    pub id: String,
    /// Shown in the picker, so it should say what this version *is* rather
    /// than repeating the surface name.
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub surface: ReplaceableSurface,
    pub view: ViewSource,
}

/// Where a command may be invoked from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandSurface {
    /// The command palette. Always available.
    Palette,
    /// The context menu of an instance. The command receives the instance id.
    InstanceContext,
}

/// An action the user can invoke.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandContribution {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Name of the exported function invoked when the user picks the command.
    pub export: String,
    #[serde(default = "default_command_surfaces")]
    pub surfaces: Vec<CommandSurface>,
}

fn default_command_surfaces() -> Vec<CommandSurface> {
    vec![CommandSurface::Palette]
}

/// The type and default of one plugin setting.
///
/// The host renders and persists these; the plugin reads them back through
/// `storage`. Keeping the schema declarative means the settings page works
/// with the plugin's script unloaded.
// No `deny_unknown_fields` here: serde cannot combine it with `flatten`, and
// the flattened `SettingSchema` is what carries the `type` discriminator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingDefinition {
    pub key: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub schema: SettingSchema,
}

/// The typed shape of a single setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum SettingSchema {
    Boolean {
        #[serde(default)]
        default: bool,
    },
    String {
        #[serde(default)]
        default: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<usize>,
    },
    Number {
        #[serde(default)]
        default: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    Enum {
        default: String,
        options: Vec<EnumOption>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnumOption {
    pub value: String,
    pub label: String,
}

impl SettingSchema {
    /// The value the host stores when the user has never touched the setting.
    pub fn default_value(&self) -> serde_json::Value {
        match self {
            SettingSchema::Boolean { default } => serde_json::Value::Bool(*default),
            SettingSchema::String { default, .. } => serde_json::Value::String(default.clone()),
            SettingSchema::Number { default, .. } => serde_json::Number::from_f64(*default)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            SettingSchema::Enum { default, .. } => serde_json::Value::String(default.clone()),
        }
    }

    /// Whether a candidate value is acceptable for this setting.
    ///
    /// Applied on every write, including writes from the plugin's own script —
    /// a plugin must not be able to store a value its own settings page could
    /// then fail to render.
    pub fn accepts(&self, value: &serde_json::Value) -> bool {
        match (self, value) {
            (SettingSchema::Boolean { .. }, serde_json::Value::Bool(_)) => true,
            (SettingSchema::String { max_length, .. }, serde_json::Value::String(s)) => {
                max_length.is_none_or(|max| s.chars().count() <= max)
            }
            (SettingSchema::Number { min, max, .. }, serde_json::Value::Number(n)) => {
                let Some(v) = n.as_f64() else { return false };
                min.is_none_or(|m| v >= m) && max.is_none_or(|m| v <= m)
            }
            (SettingSchema::Enum { options, .. }, serde_json::Value::String(s)) => {
                options.iter().any(|o| &o.value == s)
            }
            _ => false,
        }
    }
}

/// Semantic theme tokens.
///
/// Values are CSS colors applied to Agora's own custom properties. A theme
/// cannot inject arbitrary CSS: the host validates each token name against the
/// documented set and each value against a strict color grammar, so a theme
/// cannot smuggle in a `url()` or an `expression()`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeContribution {
    pub id: String,
    pub title: String,
    /// `light`, `dark`, or both. A theme that only defines one is offered only
    /// in that mode rather than breaking the other.
    #[serde(default)]
    pub light: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub dark: std::collections::BTreeMap<String, String>,
}

/// A check the plugin can run against an instance on demand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiagnosticContribution {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Exported function returning [`crate::diagnostics::DiagnosticReport`].
    pub export: String,
}

/// A bounded check run during launch preparation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchCheckContribution {
    pub id: String,
    pub title: String,
    /// Exported function returning [`crate::diagnostics::DiagnosticReport`].
    pub export: String,
    /// Hard ceiling in milliseconds. The host clamps this to
    /// [`MAX_LAUNCH_CHECK_MS`] so no plugin can hold a launch open.
    #[serde(default = "default_launch_check_timeout_ms")]
    pub timeout_ms: u64,
    /// Whether a failing check blocks the launch, or is reported as a warning
    /// and the user decides. Warn is the default: Agora warns, it does not
    /// veto, and a plugin does not get to be stricter than the launcher.
    #[serde(default)]
    pub on_failure: LaunchCheckFailure,
}

/// Longest a launch check may run before the host interrupts it.
pub const MAX_LAUNCH_CHECK_MS: u64 = 5_000;

fn default_launch_check_timeout_ms() -> u64 {
    2_000
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LaunchCheckFailure {
    /// Surface the finding and let the user launch anyway.
    #[default]
    Warn,
    /// Offer the user a blocking prompt. Still user-dismissable.
    Prompt,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_schema_rejects_a_value_of_the_wrong_type() {
        let schema = SettingSchema::Boolean { default: false };
        assert!(schema.accepts(&serde_json::json!(true)));
        assert!(!schema.accepts(&serde_json::json!("true")));
    }

    #[test]
    fn number_setting_enforces_its_declared_bounds() {
        let schema = SettingSchema::Number {
            default: 5.0,
            min: Some(1.0),
            max: Some(10.0),
        };
        assert!(schema.accepts(&serde_json::json!(5)));
        assert!(!schema.accepts(&serde_json::json!(0)));
        assert!(!schema.accepts(&serde_json::json!(11)));
    }

    #[test]
    fn enum_setting_only_accepts_a_declared_option() {
        let schema = SettingSchema::Enum {
            default: "compact".into(),
            options: vec![
                EnumOption {
                    value: "compact".into(),
                    label: "Compact".into(),
                },
                EnumOption {
                    value: "roomy".into(),
                    label: "Roomy".into(),
                },
            ],
        };
        assert!(schema.accepts(&serde_json::json!("roomy")));
        assert!(!schema.accepts(&serde_json::json!("enormous")));
    }

    #[test]
    fn local_ids_covers_every_surface_so_collision_checks_cannot_miss_one() {
        let json = serde_json::json!({
            "pages": [{ "id": "p", "title": "P", "view": { "kind": "host", "export": "page" } }],
            "instancePanels": [{ "id": "ip", "title": "IP", "view": { "kind": "host", "export": "panel" } }],
            "commands": [{ "id": "c", "title": "C", "export": "run" }],
            "settings": [{ "key": "s", "title": "S", "type": "boolean" }],
            "diagnostics": [{ "id": "d", "title": "D", "export": "check" }],
            "launchChecks": [{ "id": "lc", "title": "LC", "export": "preflight" }],
        });
        let contributions: Contributions = serde_json::from_value(json).unwrap();
        let kinds: Vec<_> = contributions
            .local_ids()
            .into_iter()
            .map(|(kind, _)| kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                ContributionKind::Page,
                ContributionKind::InstancePanel,
                ContributionKind::Command,
                ContributionKind::Setting,
                ContributionKind::Diagnostic,
                ContributionKind::LaunchCheck,
            ]
        );
    }

    #[test]
    fn a_command_defaults_to_the_palette_only() {
        let command: CommandContribution =
            serde_json::from_value(serde_json::json!({ "id": "c", "title": "C", "export": "run" }))
                .unwrap();
        assert_eq!(command.surfaces, vec![CommandSurface::Palette]);
    }
}
