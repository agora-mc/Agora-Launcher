//! Types shared by more than one `games` slice. Slice-only shapes live in the slice's file.

use std::path::Path;

use agora_core::game_registry::{IdentifiedInstall, RuntimeResolution};
use serde::Serialize;

/// Every game with at least one install found, in the order discovery met them.
/// A game with no installs is not listed: the switcher treats "listed" as "found".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamesList {
    pub games: Vec<GameListing>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameListing {
    pub game_id: String,
    pub name: String,
    pub installs: Vec<InstallSummary>,
}

/// One store install of a game, as discovery sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallSummary {
    pub install_id: String,
    pub store: String,
    pub name: String,
    pub location: String,
    pub store_version: Option<String>,
    pub store_build: Option<String>,
    pub add_on_count: usize,
    pub runtime: RuntimeResolution,
}

impl From<&IdentifiedInstall> for InstallSummary {
    fn from(install: &IdentifiedInstall) -> Self {
        let discovered = &install.discovered;
        Self {
            install_id: install.install_id.to_string(),
            store: discovered.store.to_string(),
            name: discovered.name.clone(),
            location: display_path(&discovered.location),
            store_version: discovered.store_version.clone(),
            store_build: discovered.store_build.clone(),
            add_on_count: install.add_ons.len(),
            runtime: install.runtime.clone(),
        }
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
