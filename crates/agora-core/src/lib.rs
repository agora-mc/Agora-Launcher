//! Agora launcher core — shared business logic consumed by the Tauri GUI,
//! the standalone `agora` CLI, and the in-process MCP listener.
//!
//! Constraint (plan C2/C3): this crate MUST NOT depend on `tauri`, `clap`,
//! or any MCP-protocol crate. Every operation takes a `&Ctx` (introduced
//! later). For now this crate only hosts the pure data/error modules moved
//! out of the desktop crate in Phase 1A.

pub mod ai_assistant;
pub mod app_paths;
pub mod artifact_receipt;
pub mod auth;
pub mod backup;
pub mod bisect;
pub mod browse_cache;
pub mod clone;
pub mod controller_service;
pub mod crash_diagnostics;
pub mod crash_evidence;
pub mod crash_service;
pub mod ctx;
pub mod data_migration;
pub mod db;
pub mod dependency_ops;
pub mod download;
pub mod error;
pub mod event_sink;
pub mod export_service;
pub mod gc;
pub mod github_ratelimit;
pub mod governance;
pub mod health;
pub mod helpers;
pub mod http_client;
pub mod icon;
pub mod import;
pub mod import_service;
pub mod install_pipeline;
pub mod install_service;
pub mod installed_artifact;
pub mod installed_content;
pub mod installed_profile;
pub mod instance_service;
pub mod jar_metadata;
pub mod java;
/// Metadata types and Forge/NeoForge install-profile helpers.
///
/// # Deprecation
/// This module retains **only** reusable type definitions and Forge helpers.
/// The legacy direct-launch orchestration (`fetch_version_manifest`,
/// `build_launch_command`, `spawn_java`, `prepare_loader`, etc.) has been
/// removed. Use [`crate::launch_planner`] for all production Java launches.
pub mod launch;
pub mod launch_history;
pub mod launch_planner;
pub mod launch_service;
pub mod launcher_import;
pub mod launcher_import_service;
pub mod launcher_profiles;
pub mod launcher_ui_state;
pub mod lkg;
pub mod loader_compatibility;
pub mod loader_manifests;
pub mod loader_service;
pub mod loadout;
pub mod lock_manager;
pub mod lockfile;
pub mod log_sanitizer;
pub mod maintenance;
pub mod mcp_dispatcher;
pub mod memory_recommendation;
pub mod migration_report;
pub mod minecraft_metadata;
pub mod minecraft_runtime;
pub mod mod_cache;
pub mod mod_groups;
pub mod models;
pub mod modrinth;
pub mod msa;
pub mod network;
pub mod official_launcher;
pub mod operation_manager;
pub mod override_sanitizer;
pub mod pack_install;
pub mod pack_inventory;
pub mod pack_merge;
pub mod pack_update;
pub mod paths;
pub mod process_identity;
pub mod process_session_manager;
pub mod prune_service;
pub mod ranking;
pub mod registry;
pub mod registry_sync;
pub mod resolver;
pub mod runtime_catalog;
pub mod runtime_manager;
pub mod runtime_service;
pub mod server_export;
pub mod settings;
pub mod shared_folder;
pub mod snapshot;
pub mod state;
pub mod task_scheduler;
pub mod technic;
pub mod template_service;
pub mod update_cache;
pub mod version_changelogs;
pub mod version_match;
pub mod version_migration;
