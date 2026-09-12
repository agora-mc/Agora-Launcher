//! The data a plugin sees, and the UI it can describe.
//!
//! These are a deliberately narrow *projection* of the launcher's own models,
//! not a re-export of them. `agora-core` is free to reshape `InstanceRow` or
//! `InstalledContentRow` tomorrow; a plugin written today keeps working
//! because it never saw those types. Two things are left out on purpose:
//!
//! - absolute filesystem paths, which are the launcher's business, and
//! - anything credential-shaped, which never crosses this boundary at all.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Instances and content
// ---------------------------------------------------------------------------

/// An instance as it appears in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSummary {
    pub id: String,
    pub name: String,
    pub minecraft_version: String,
    /// `vanilla`, `fabric`, `forge`, `neoforge`, `quilt`.
    pub loader: String,
    pub loader_version: String,
    pub is_modpack: bool,
    /// A locked instance refuses content changes. A plugin should check this
    /// before offering an action rather than discovering it on failure.
    pub is_locked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_launched_at: Option<String>,
    pub created_at: String,
}

/// JVM configuration, as the user set it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JvmSettings {
    pub memory_mb: i64,
    /// `auto` or `manual`.
    pub memory_mode: String,
    pub gc: String,
    pub custom_args: String,
    pub always_pre_touch: bool,
    /// Whether this instance overrides the global Java selection. The path
    /// itself is not exposed — a plugin has no business reading it.
    pub has_java_override: bool,
}

/// Everything a plugin can learn about one instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceDetail {
    #[serde(flatten)]
    pub summary: InstanceSummary,
    pub jvm: JvmSettings,
    /// `auto`, `direct` or `delegated`.
    pub launch_mode: String,
    /// Where the instance came from, when it came from a pack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pack_origin: Option<String>,
    pub content_counts: ContentCounts,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentCounts {
    pub mods: u32,
    pub resourcepacks: u32,
    pub shaders: u32,
    pub datapacks: u32,
    pub worlds: u32,
    /// How many of the above are currently disabled.
    pub disabled: u32,
}

/// One installed item inside an instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentEntry {
    /// Stable key within the instance. This is what mutating methods take.
    pub key: String,
    pub filename: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `mod`, `resourcepack`, `shader`, `datapack`, `world`.
    pub content_type: String,
    pub enabled: bool,
    pub installed_at: String,
    /// Human-readable origin, e.g. `Curated`, `Modrinth`, `Manual`.
    pub source_label: String,
    pub pack_managed: bool,
    pub installed_as_dependency: bool,
    pub update_pinned: bool,
    /// Whether the file is actually on disk. A `false` here is the usual
    /// cause of "my mod vanished" and a good thing for a plugin to check.
    pub file_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modrinth_id: Option<String>,
}

/// What an instance is doing right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchState {
    pub instance_id: String,
    /// `idle`, `preparing`, `running`, `exited`, `crashed`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Present for a direct launch that is actually running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

/// One past launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchHistoryEntry {
    pub instance_id: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prep_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    pub enabled_mod_count: i64,
    pub minecraft_version: String,
    pub loader: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_memory_mb: Option<i64>,
}

// ---------------------------------------------------------------------------
// Host-rendered views
// ---------------------------------------------------------------------------

/// The emphasis a piece of UI carries.
///
/// A plugin picks a meaning, not a colour. The active theme decides what
/// `Warning` looks like, so a plugin cannot produce unreadable contrast or
/// ignore the user's theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tone {
    #[default]
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

/// A complete screen described as data.
///
/// The host renders this with its own components. That is the whole reason it
/// exists: a plugin gets a themed, accessible, keyboard- and controller-
/// navigable panel without importing React, and Agora never hands community
/// content to `dangerouslySetInnerHTML`, because there is no HTML to hand it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewModel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub blocks: Vec<ViewBlock>,
}

impl ViewModel {
    /// How many blocks a single view may contain.
    ///
    /// A bound exists so a plugin that loops while building a view produces a
    /// clear error instead of a frozen window.
    pub const MAX_BLOCKS: usize = 200;

    /// How many rows one table may carry across the boundary.
    pub const MAX_TABLE_ROWS: usize = 500;

    pub fn validate(&self) -> Result<(), crate::PluginError> {
        if self.blocks.len() > Self::MAX_BLOCKS {
            return Err(crate::PluginError::new(
                crate::PluginErrorCode::ResourceExhausted,
                format!(
                    "a view may contain {} blocks; this one has {}",
                    Self::MAX_BLOCKS,
                    self.blocks.len()
                ),
            ));
        }
        for block in &self.blocks {
            if let ViewBlock::Table { rows, .. } = block {
                if rows.len() > Self::MAX_TABLE_ROWS {
                    return Err(crate::PluginError::new(
                        crate::PluginErrorCode::ResourceExhausted,
                        format!(
                            "a table may contain {} rows; this one has {}",
                            Self::MAX_TABLE_ROWS,
                            rows.len()
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// One renderable unit of a [`ViewModel`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum ViewBlock {
    Heading {
        text: String,
    },
    Text {
        text: String,
        #[serde(default)]
        tone: Tone,
    },
    /// A row of headline numbers.
    Stats {
        items: Vec<Stat>,
    },
    Table {
        columns: Vec<Column>,
        rows: Vec<Vec<Cell>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty_message: Option<String>,
    },
    List {
        items: Vec<ListItem>,
    },
    /// A callout: something the user should notice.
    Status {
        tone: Tone,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Actions {
        items: Vec<ActionButton>,
    },
    /// A visual break between sections.
    Divider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stat {
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default)]
    pub tone: Tone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub label: String,
    #[serde(default)]
    pub align: ColumnAlign,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColumnAlign {
    #[default]
    Start,
    End,
}

/// One table cell. Text or a badge — never markup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum Cell {
    Text {
        text: String,
    },
    Badge {
        text: String,
        #[serde(default)]
        tone: Tone,
    },
    /// A yes/no, rendered as a check or a dash rather than the word.
    Flag {
        value: bool,
    },
}

impl Cell {
    pub fn text(value: impl Into<String>) -> Self {
        Cell::Text { text: value.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListItem {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default)]
    pub tone: Tone,
}

/// A button the user can press inside a plugin view.
///
/// Pressing it calls back into the plugin's own script. It does not name a
/// host method: a plugin cannot draw a button that performs a privileged
/// operation the plugin itself is not allowed to request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionButton {
    pub id: String,
    pub label: String,
    /// Exported function invoked on press.
    pub export: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
    #[serde(default)]
    pub tone: Tone,
    /// When set, the host asks the user this question first. Required by the
    /// host for anything it knows to be destructive, regardless of what the
    /// plugin asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_block_names_its_type_on_the_wire() {
        let block = ViewBlock::Status {
            tone: Tone::Warning,
            title: "Two mods want the same library".into(),
            message: None,
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "status");
        assert_eq!(json["tone"], "warning");
    }

    #[test]
    fn tone_defaults_to_neutral_when_a_plugin_omits_it() {
        let block: ViewBlock =
            serde_json::from_value(serde_json::json!({ "type": "text", "text": "hi" })).unwrap();
        assert_eq!(
            block,
            ViewBlock::Text {
                text: "hi".into(),
                tone: Tone::Neutral
            }
        );
    }

    #[test]
    fn an_oversized_view_is_refused_rather_than_rendered() {
        let model = ViewModel {
            title: None,
            subtitle: None,
            blocks: vec![ViewBlock::Divider; ViewModel::MAX_BLOCKS + 1],
        };
        let err = model.validate().unwrap_err();
        assert_eq!(err.code, crate::PluginErrorCode::ResourceExhausted);
    }

    #[test]
    fn an_oversized_table_is_refused_rather_than_rendered() {
        let model = ViewModel {
            title: None,
            subtitle: None,
            blocks: vec![ViewBlock::Table {
                columns: vec![Column {
                    label: "Name".into(),
                    align: ColumnAlign::Start,
                }],
                rows: vec![vec![Cell::text("x")]; ViewModel::MAX_TABLE_ROWS + 1],
                empty_message: None,
            }],
        };
        assert!(model.validate().is_err());
    }

    #[test]
    fn instance_detail_flattens_its_summary_so_plugins_see_one_object() {
        let detail = InstanceDetail {
            summary: InstanceSummary {
                id: "i1".into(),
                name: "Skyblock".into(),
                minecraft_version: "1.21.1".into(),
                loader: "fabric".into(),
                loader_version: "0.16.5".into(),
                is_modpack: false,
                is_locked: false,
                last_launched_at: None,
                created_at: "2026-01-01T00:00:00Z".into(),
            },
            jvm: JvmSettings {
                memory_mb: 4096,
                memory_mode: "auto".into(),
                gc: "g1".into(),
                custom_args: String::new(),
                always_pre_touch: false,
                has_java_override: false,
            },
            launch_mode: "auto".into(),
            pack_origin: None,
            content_counts: ContentCounts::default(),
        };
        let json = serde_json::to_value(&detail).unwrap();
        assert_eq!(json["id"], "i1");
        assert_eq!(json["jvm"]["memoryMb"], 4096);
        assert!(json.get("summary").is_none());
    }
}
