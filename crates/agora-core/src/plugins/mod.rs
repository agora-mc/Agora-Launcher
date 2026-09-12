//! Community plugins: policy, storage and dispatch.
//!
//! Agora's extension system lives here, in core, for the same reason
//! everything else does — all three frontends must get identical behaviour.
//! The GUI, the CLI and (later) the MCP dispatcher all drive
//! [`service::PluginService`]; none of them re-implements a single decision.
//!
//! # What lives where
//!
//! | Concern | Module | Note |
//! |---|---|---|
//! | What is installed, enabled, granted | [`store`] | `local_state.db`, schema v14 |
//! | What may run, and in what order | [`registry`] | Pure function of the records |
//! | Getting files on and off disk | [`install`] | Archive validation, rollback |
//! | What a plugin may ask for | [`dispatch`] | The whole method table, in one place |
//! | Telling plugins what happened | [`events`] | Subscriptions, origin, depth |
//! | Per-plugin logs | [`logs`] | One file each, rotated once |
//! | Everything above, composed | [`service`] | What adapters call |
//!
//! # What is deliberately *not* here
//!
//! The JavaScript engine. Core holds an `Arc<dyn ScriptHost>` supplied by an
//! adapter, exactly as it holds a `dyn Clock`. That is what keeps a second
//! runtime — another language, an out-of-process companion — a matter of
//! writing a new implementation rather than reworking plugin policy.
//!
//! # The three properties everything else rests on
//!
//! 1. **Nothing runs unless the user opted in.** `plugins_enabled` defaults to
//!    off, and a user who never turns it on never has a plugin runtime in
//!    their process.
//! 2. **A plugin proposes; core disposes.** Plugins return descriptions of
//!    operations. Core re-validates and executes them through the same
//!    services the GUI uses, with the same locks and the same user decisions.
//! 3. **Failure is contained and visible.** A plugin that throws, hangs, or
//!    exhausts its memory affects only itself, and the reason ends up
//!    somewhere the user can read it.

pub mod dispatch;
pub mod events;
pub mod install;
pub mod logs;
pub mod registry;
pub mod service;
pub mod store;
pub mod updates;

pub use dispatch::{MethodRequirement, NoopUiSink, PluginUiSink};
pub use events::{EventSubscriptions, PluginEventBus};
pub use install::{
    added_capabilities, CapabilityDescription, ExistingInstall, InstallPreview, KeyFingerprint,
    UpdateSourceSummary,
};
pub use registry::{NamespacedContribution, PluginStatus, Resolution};
pub use service::{
    LaunchCheckOutcome, LaunchCheckResult, PluginService, PluginSettingsView, PluginSummary,
    RepairConflict, RepairOutcome, ReplacementOffer, SurfaceChoice, UpdateCheckRecord,
    UpdateOutcome, PLUGINS_ENABLED_SETTING, PLUGIN_UPDATES_ENABLED_SETTING,
    SURFACE_SELECTIONS_SETTING,
};
pub use store::{CheckpointSummary, PluginRecord, PluginSource, StorageKind};
pub use updates::{PinnedTrust, UpdateVerdict};
