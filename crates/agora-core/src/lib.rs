//! Agora launcher core — shared business logic consumed by the Tauri GUI,
//! the standalone `agora` CLI, and the in-process MCP listener.
//!
//! Constraint (plan C2/C3): this crate MUST NOT depend on `tauri`, `clap`,
//! or any MCP-protocol crate. Every operation takes a `&Ctx` (introduced
//! later). For now this crate only hosts the pure data/error modules moved
//! out of the desktop crate in Phase 1A.

pub mod app_paths;
pub mod artifact_hash;
pub mod artifact_receipt;
pub mod auth;
pub mod backup;
pub mod bisect;
pub mod catalog_install;
pub mod community_image;
pub mod content_fomod;
pub mod content_store;
pub mod content_thunderstore;
pub mod crash_diagnostics;
pub mod crash_evidence;
pub mod crash_export;
pub mod crash_service;
pub mod ctx;
pub mod data_migration;
pub mod db;
pub mod dependency_ops;
pub mod download;
pub mod error;
pub mod event_sink;
pub mod game_base;
pub mod game_deploy;
pub mod game_discovery;
pub mod game_frameworks;
pub mod game_hooks;
pub mod game_import;
pub mod game_ini;
pub mod game_instance;
pub mod game_launch;
pub mod game_load_order;
pub mod game_plugins;
pub mod game_registry;
pub mod game_saves;
pub mod game_tool_swap;
pub mod game_tools;
pub mod game_user_files;
pub mod github_ratelimit;
pub mod github_release;
pub mod governance;
pub mod helpers;
pub mod http_client;
pub mod icon;
pub mod install_watch;
pub mod instance_runtime;
/// Metadata types and Forge/NeoForge install-profile helpers.
///
/// # Deprecation
/// This module retains **only** reusable type definitions and Forge helpers.
/// The legacy direct-launch orchestration (`fetch_version_manifest`,
/// `build_launch_command`, `spawn_java`, `prepare_loader`, etc.) has been
/// removed. Use [`crate::launch_planner`] for all production Java launches.
pub mod launch_history;
pub mod lkg;
pub mod loadout;
pub mod lock_manager;
pub mod log_sanitizer;
pub mod mod_groups;
pub mod models;
pub mod network;
pub mod network_gate;
pub mod operation_manager;
pub mod paths;
pub mod pe_version;
pub mod plugins;
pub mod process_identity;
pub mod process_session_manager;
pub mod providers;
pub mod ranking;
pub mod registry;
pub mod registry_sync;
pub mod settings;
pub mod shared_folder;
pub mod snapshot;
pub mod snapshot_service;
pub mod task_scheduler;
pub mod version_changelogs;
pub mod version_match;
