//! The Agora plugin contract.
//!
//! This crate is the *only* thing a plugin author's tooling and the launcher
//! agree on. It deliberately depends on nothing but `serde`, `semver` and
//! `thiserror`: no `tauri`, no `clap`, no MCP protocol types, no `rusqlite`,
//! and no JavaScript engine. That keeps it usable from
//!
//! - `agora-core`, which owns plugin policy, storage and service dispatch,
//! - `agora-plugin-host`, which runs plugin script in QuickJS,
//! - the adapters, which only move these types across a transport,
//! - and, later, an out-of-process companion runtime for another language.
//!
//! Everything here is serialisable. Nothing here holds a handle to a database,
//! a filesystem path the plugin chose, or a live core service. A plugin asks
//! for work by name and arguments; the host decides whether that is allowed.
//!
//! # Versioning
//!
//! [`HOST_API_VERSION`] is the semantic version of this contract. A plugin
//! declares the range it supports in its manifest (`apiRange`) and the host
//! refuses to activate a plugin whose range excludes the running host.
//!
//! `0.1` is experimental and has **no** support window yet; what that means in
//! practice is written down in `docs/plugins/implementation-status.md`, and
//! `docs/plugins/fixtures/` pins the surface that has actually shipped.

pub mod capability;
pub mod contributions;
pub mod diagnostics;
pub mod distribution;
pub mod dto;
pub mod error;
pub mod host;
pub mod manifest;
pub mod protocol;

pub use capability::{Capability, CapabilitySet};
pub use error::{PluginError, PluginErrorCode};
pub use manifest::{PluginId, PluginManifest, MANIFEST_SCHEMA_VERSION};

/// Semantic version of the plugin contract implemented by this build.
///
/// Bump the minor while the contract is additive; bump the major only with a
/// documented deprecation window. `0.x` means the surface is still allowed to
/// change, and the compatibility fixtures under `docs/plugins/fixtures/` are
/// the record of what has actually shipped.
pub const HOST_API_VERSION: semver::Version = semver::Version::new(0, 1, 0);

/// The `api` the host reports to plugins at runtime, as a string.
pub fn host_api_version_string() -> String {
    HOST_API_VERSION.to_string()
}
