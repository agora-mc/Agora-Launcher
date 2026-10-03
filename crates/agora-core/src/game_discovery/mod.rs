pub mod epic;
pub mod gog;
pub mod microsoft_store;
pub mod platform;
pub mod steam;
pub mod volume;

use std::path::PathBuf;

pub use agora_game_api::{InstallCapabilities, InstallKind, StoreId, VolumeInfo};
use serde::{Deserialize, Serialize};

use self::epic::EpicAdapter;
use self::gog::GogAdapter;
use self::microsoft_store::MicrosoftStoreAdapter;
use self::steam::SteamAdapter;
use self::volume::VolumeDetector;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredInstall {
    pub store: StoreId,
    pub product: String, // Steam app id, GOG game id, Epic AppName, MS Store Identity Name
    pub name: String,
    pub kind: InstallKind,
    pub parent_product: Option<String>, // for add-ons: the base game's `product` in the same store
    pub location: PathBuf,
    pub store_version: Option<String>,
    pub store_build: Option<String>,
    pub executables: Vec<String>, // launch executables the store declares, relative to location, game before launcher
    pub capabilities: InstallCapabilities,
    pub volume: Option<VolumeInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryWarning {
    pub store: StoreId,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryReport {
    pub installs: Vec<DiscoveredInstall>,
    pub warnings: Vec<DiscoveryWarning>,
}

impl DiscoveryReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn combine(&mut self, mut other: DiscoveryReport) {
        self.installs.append(&mut other.installs);
        self.warnings.append(&mut other.warnings);
    }
}

pub trait StoreAdapter: Send + Sync {
    fn store_id(&self) -> StoreId;
    fn discover(&self, volume_detector: &VolumeDetector) -> DiscoveryReport;
}

/// Discover all game installs and add-ons across all supported stores.
pub fn discover_all() -> DiscoveryReport {
    let detector = VolumeDetector::new();
    let mut report = DiscoveryReport::new();

    report.combine(SteamAdapter::discover_system(&detector));
    report.combine(GogAdapter::discover_system(&detector));
    report.combine(EpicAdapter::discover_system(&detector));
    report.combine(MicrosoftStoreAdapter::discover_system(&detector));

    report
}
