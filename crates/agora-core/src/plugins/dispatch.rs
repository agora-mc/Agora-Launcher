//! The method table: every operation a plugin can ask the launcher to perform.
//!
//! This is the entire attack surface, in one file, on purpose. A method that
//! is not here does not exist, and every method that is here names the
//! capability it requires in the same place it names its implementation —
//! so "which permission does this need?" is answered by reading one match arm
//! rather than by auditing a service.
//!
//! Three rules hold for every arm:
//!
//! 1. **Capability first.** Checked before arguments are even parsed, so a
//!    plugin without `content:write` learns it cannot write rather than
//!    learning that its argument was malformed.
//! 2. **Reuse, never reimplement.** Each arm calls the same `agora-core`
//!    service the GUI and the CLI call, so plugins inherit the locks, the
//!    validation, the operation state and the recovery behaviour for free. A
//!    plugin cannot reach a code path the user could not reach themselves.
//! 3. **No handles out.** Arguments and results are JSON. No `Ctx`, no
//!    `Connection`, no filesystem path a plugin could use to go around this.

use super::events::EventSubscriptions;
use super::store::{self, StorageKind};
use crate::ctx::Ctx;
use crate::error::LauncherError;
use agora_plugin_api::capability::{Capability, CapabilitySet};
use agora_plugin_api::contributions::SettingDefinition;
use agora_plugin_api::dto;
use agora_plugin_api::error::{PluginError, PluginErrorCode, PluginResult};
use agora_plugin_api::manifest::PluginId;
use agora_plugin_api::protocol::PluginEvent;

/// Where a plugin's UI intents go.
///
/// A plugin cannot draw on the launcher directly; it can ask for its own view
/// to re-render, or raise a toast attributed to it. The adapter decides how,
/// and is free to ignore a plugin that asks too often.
pub trait PluginUiSink: Send + Sync {
    fn refresh(&self, plugin_id: &PluginId, view_id: Option<&str>);
    fn notify(&self, plugin_id: &PluginId, tone: dto::Tone, message: &str);
}

/// A sink that drops everything. Used by the CLI and by tests.
pub struct NoopUiSink;

impl PluginUiSink for NoopUiSink {
    fn refresh(&self, _plugin_id: &PluginId, _view_id: Option<&str>) {}
    fn notify(&self, _plugin_id: &PluginId, _tone: dto::Tone, _message: &str) {}
}

/// Everything one dispatch needs to know about the calling plugin.
pub struct DispatchContext<'a> {
    pub ctx: &'a Ctx,
    pub plugin_id: &'a PluginId,
    pub granted: &'a CapabilitySet,
    /// Hosts from the plugin's manifest, for `net.*`.
    pub declared_hosts: &'a [String],
    /// Declared settings, so a write can be validated against its schema.
    pub settings_schema: &'a [SettingDefinition],
    pub subscriptions: &'a EventSubscriptions,
    pub ui: &'a dyn PluginUiSink,
}

/// Every method this host API version serves.
///
/// Kept as data so the documentation, the CLI's `agora plugin methods`, and
/// the capability check all read from one list that cannot drift apart.
pub const METHODS: &[(&str, Option<Capability>, &str)] = &[
    (
        "instance.list",
        Some(Capability::InstanceRead),
        "List every instance.",
    ),
    (
        "instance.get",
        Some(Capability::InstanceRead),
        "Inspect one instance.",
    ),
    (
        "instance.rename",
        Some(Capability::InstanceWrite),
        "Rename an instance.",
    ),
    (
        "instance.setMemory",
        Some(Capability::InstanceWrite),
        "Set an instance's heap ceiling.",
    ),
    (
        "content.list",
        Some(Capability::ContentRead),
        "List installed content.",
    ),
    (
        "content.enable",
        Some(Capability::ContentWrite),
        "Enable an installed item.",
    ),
    (
        "content.disable",
        Some(Capability::ContentWrite),
        "Disable an installed item.",
    ),
    (
        "content.setUpdatePinned",
        Some(Capability::ContentWrite),
        "Pin or unpin an item against updates.",
    ),
    (
        "launch.state",
        Some(Capability::LaunchRead),
        "What an instance is doing now.",
    ),
    (
        "launch.history",
        Some(Capability::LaunchRead),
        "Past launches for an instance.",
    ),
    (
        "storage.get",
        None,
        "Read one of this plugin's stored values.",
    ),
    (
        "storage.set",
        None,
        "Write one of this plugin's stored values.",
    ),
    (
        "storage.all",
        None,
        "Read all of this plugin's stored values.",
    ),
    (
        "storage.remove",
        None,
        "Delete one of this plugin's stored values.",
    ),
    (
        "settings.all",
        None,
        "Read this plugin's declared settings.",
    ),
    ("events.subscribe", None, "Start receiving an event."),
    ("events.unsubscribe", None, "Stop receiving an event."),
    (
        "net.fetchJson",
        Some(Capability::Network),
        "GET JSON from a declared host.",
    ),
    ("ui.refresh", None, "Ask this plugin's view to re-render."),
    (
        "ui.notify",
        None,
        "Raise a toast attributed to this plugin.",
    ),
];

/// What calling a method requires.
///
/// Three outcomes rather than two, because "there is no such method" and "you
/// may not call that method" are different answers and a plugin author
/// debugging a typo should not be told they lack a permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodRequirement {
    /// Not part of this host API version at all.
    Unknown,
    /// Exists and needs no capability — a plugin's own storage, its settings,
    /// its own UI.
    Open,
    /// Exists and needs this capability.
    Requires(Capability),
}

/// What `method` requires of its caller.
pub fn required_capability(method: &str) -> MethodRequirement {
    match METHODS.iter().find(|(name, _, _)| *name == method) {
        None => MethodRequirement::Unknown,
        Some((_, None, _)) => MethodRequirement::Open,
        Some((_, Some(capability), _)) => MethodRequirement::Requires(*capability),
    }
}

/// Serve one plugin request.
pub fn dispatch(
    cx: &DispatchContext<'_>,
    method: &str,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    // Rule 1: the capability gate runs before anything looks at `args`.
    match required_capability(method) {
        MethodRequirement::Unknown => return Err(PluginError::unknown_method(method)),
        MethodRequirement::Requires(capability) if !cx.granted.contains(capability) => {
            return Err(PluginError::capability_denied(capability, method))
        }
        MethodRequirement::Requires(_) | MethodRequirement::Open => {}
    }

    match method {
        "instance.list" => instance_list(cx),
        "instance.get" => instance_get(cx, args),
        "instance.rename" => instance_rename(cx, args),
        "instance.setMemory" => instance_set_memory(cx, args),
        "content.list" => content_list(cx, args),
        "content.enable" => content_set_enabled(cx, args, true),
        "content.disable" => content_set_enabled(cx, args, false),
        "content.setUpdatePinned" => content_set_pinned(cx, args),
        "launch.state" => launch_state(cx, args),
        "launch.history" => launch_history(cx, args),
        "storage.get" => storage_get(cx, args, StorageKind::Data),
        "storage.set" => storage_set(cx, args),
        "storage.all" => storage_all(cx, args, StorageKind::Data),
        "storage.remove" => storage_remove(cx, args),
        "settings.all" => storage_all(cx, args, StorageKind::Setting),
        "events.subscribe" => events_subscribe(cx, args),
        "events.unsubscribe" => events_unsubscribe(cx, args),
        "net.fetchJson" => net_fetch_json(cx, args),
        "ui.refresh" => ui_refresh(cx, args),
        "ui.notify" => ui_notify(cx, args),
        // Unreachable: `required_capability` already rejected unknown names.
        other => Err(PluginError::unknown_method(other)),
    }
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

fn arg_str(args: &serde_json::Value, key: &str) -> PluginResult<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .ok_or_else(|| PluginError::invalid_arguments(format!("`{key}` must be a string")))
}

fn arg_opt_str(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn arg_i64(args: &serde_json::Value, key: &str) -> PluginResult<i64> {
    args.get(key)
        .and_then(|value| value.as_i64())
        .ok_or_else(|| PluginError::invalid_arguments(format!("`{key}` must be a whole number")))
}

fn arg_bool(args: &serde_json::Value, key: &str) -> PluginResult<bool> {
    args.get(key)
        .and_then(|value| value.as_bool())
        .ok_or_else(|| PluginError::invalid_arguments(format!("`{key}` must be true or false")))
}

/// Turn a core error into a plugin error without leaking internals.
///
/// The code is preserved because it is already a stable, user-facing string in
/// this codebase, and a plugin author debugging `ERR_INSTANCE_NOT_FOUND` is
/// better served than one debugging "operation failed".
fn operation_failed(error: LauncherError) -> PluginError {
    let message = error.to_string();
    PluginError::new(PluginErrorCode::OperationFailed, message)
}

fn json<T: serde::Serialize>(value: T) -> PluginResult<serde_json::Value> {
    serde_json::to_value(value)
        .map_err(|e| PluginError::internal(format!("could not serialise the result: {e}")))
}

// ---------------------------------------------------------------------------
// Instances
// ---------------------------------------------------------------------------

fn to_summary(row: &crate::models::InstanceRow) -> dto::InstanceSummary {
    dto::InstanceSummary {
        id: row.instance_id.clone(),
        name: row.name.clone(),
        minecraft_version: row.minecraft_version.clone(),
        loader: row.loader.clone(),
        loader_version: row.loader_version.clone(),
        is_modpack: row.is_modpack,
        is_locked: row.is_locked,
        last_launched_at: row.last_launched_at.clone(),
        created_at: row.created_at.clone(),
    }
}

fn instance_list(cx: &DispatchContext<'_>) -> PluginResult<serde_json::Value> {
    let service = crate::instance_service::InstanceService::new(cx.ctx.clone());
    let rows = service.list().map_err(operation_failed)?;
    json(rows.iter().map(to_summary).collect::<Vec<_>>())
}

fn instance_get(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let instance_id = arg_str(args, "instanceId")?;
    let service = crate::instance_service::InstanceService::new(cx.ctx.clone());
    let Some(detail) = service.get(&instance_id).map_err(operation_failed)? else {
        return Ok(serde_json::Value::Null);
    };

    let mut counts = dto::ContentCounts::default();
    if let Some(manifest) = &detail.manifest {
        counts.mods = manifest.mods.len() as u32;
        counts.resourcepacks = manifest.resourcepacks.len() as u32;
        counts.shaders = manifest.shaders.len() as u32;
        counts.datapacks = manifest.datapacks.len() as u32;
        counts.worlds = manifest.worlds.len() as u32;
        counts.disabled = manifest
            .mods
            .iter()
            .chain(manifest.resourcepacks.iter())
            .chain(manifest.shaders.iter())
            .chain(manifest.datapacks.iter())
            .chain(manifest.worlds.iter())
            .filter(|entry| !entry.enabled)
            .count() as u32;
    }

    json(dto::InstanceDetail {
        summary: to_summary(&detail.row),
        jvm: dto::JvmSettings {
            memory_mb: detail.row.jvm_memory_mb,
            memory_mode: detail.row.jvm_memory_mode.clone(),
            gc: detail.row.jvm_gc.clone(),
            custom_args: detail.row.jvm_custom_args.clone(),
            always_pre_touch: detail.row.jvm_always_pre_touch,
            // The path itself stays behind the boundary.
            has_java_override: detail.row.java_path.is_some(),
        },
        launch_mode: detail.row.launch_mode_override.clone(),
        pack_origin: detail
            .manifest
            .as_ref()
            .and_then(|manifest| manifest.created_from_pack.clone()),
        content_counts: counts,
    })
}

fn instance_rename(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let instance_id = arg_str(args, "instanceId")?;
    let name = arg_str(args, "name")?;
    let service = crate::instance_service::InstanceService::new(cx.ctx.clone());
    service
        .rename(&instance_id, &name)
        .map_err(operation_failed)?;
    Ok(serde_json::json!({ "renamed": true }))
}

fn instance_set_memory(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let instance_id = arg_str(args, "instanceId")?;
    let memory_mb = arg_i64(args, "memoryMb")?;
    let service = crate::instance_service::InstanceService::new(cx.ctx.clone());
    // Read-modify-write through the same service the settings page uses, so
    // the other JVM fields keep their values and core's own clamping applies.
    let Some(detail) = service.get(&instance_id).map_err(operation_failed)? else {
        return Err(PluginError::new(
            PluginErrorCode::OperationFailed,
            format!("no instance `{instance_id}`"),
        ));
    };
    service
        .update_jvm(
            &instance_id,
            memory_mb,
            &detail.row.jvm_gc,
            detail.row.jvm_always_pre_touch,
            &detail.row.jvm_custom_args,
            // A plugin setting an explicit ceiling means the user is no longer
            // on automatic sizing; saying so is more honest than leaving the
            // settings page claiming "auto" while showing a fixed number.
            "manual",
        )
        .map_err(operation_failed)?;
    Ok(serde_json::json!({ "memoryMb": memory_mb }))
}

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

fn read_content(
    cx: &DispatchContext<'_>,
    instance_id: &str,
    content_type: Option<&str>,
) -> PluginResult<Vec<crate::installed_content::InstalledContentRow>> {
    let service = crate::instance_service::InstanceService::new(cx.ctx.clone());
    let Some(detail) = service.get(instance_id).map_err(operation_failed)? else {
        return Err(PluginError::new(
            PluginErrorCode::OperationFailed,
            format!("no instance `{instance_id}`"),
        ));
    };
    let Some(manifest) = detail.manifest else {
        return Ok(Vec::new());
    };
    let instance_dir = cx
        .ctx
        .paths
        .instance_dir(instance_id)
        .map_err(operation_failed)?;
    Ok(crate::installed_content::list_installed_content(
        &instance_dir,
        &manifest,
        content_type,
        None,
    ))
}

fn content_list(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let instance_id = arg_str(args, "instanceId")?;
    let content_type = arg_opt_str(args, "contentType");
    let rows = read_content(cx, &instance_id, content_type.as_deref())?;
    let entries: Vec<dto::ContentEntry> = rows
        .into_iter()
        .map(|row| dto::ContentEntry {
            key: row.key,
            filename: row.filename,
            display_name: row.display_name,
            version: row.version,
            content_type: row.content_type,
            enabled: row.enabled,
            installed_at: row.installed_at,
            source_label: row.source_label,
            pack_managed: row.pack_managed,
            installed_as_dependency: row.installed_as_dependency,
            update_pinned: row.update_pinned,
            file_present: row.file_present,
            size_bytes: row.size_bytes,
            author: row.author,
            categories: row.categories,
            source_url: row.source_url,
            registry_id: row.registry_id,
            modrinth_id: row.modrinth_id,
            // `resolved_path` is deliberately dropped here.
        })
        .collect();
    json(entries)
}

/// Turn the key a plugin holds into the filename core's services take.
///
/// Done by looking the key up in the instance's real content list rather than
/// by splitting the string, which means a plugin cannot name a file that is
/// not actually installed — the mapping is a lookup, not a parse.
fn filename_for_key(
    cx: &DispatchContext<'_>,
    instance_id: &str,
    key: &str,
) -> PluginResult<String> {
    read_content(cx, instance_id, None)?
        .into_iter()
        .find(|row| row.key == key)
        .map(|row| row.filename)
        .ok_or_else(|| {
            PluginError::invalid_arguments(format!("`{key}` is not installed in `{instance_id}`"))
        })
}

fn content_set_enabled(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
    enabled: bool,
) -> PluginResult<serde_json::Value> {
    let instance_id = arg_str(args, "instanceId")?;
    let key = arg_str(args, "key")?;
    let filename = filename_for_key(cx, &instance_id, &key)?;
    let service = crate::crash_service::CrashService::new(cx.ctx.clone());
    let result = if enabled {
        service.enable_mod(&instance_id, &filename)
    } else {
        service.disable_mod(&instance_id, &filename)
    };
    result.map_err(operation_failed)?;
    Ok(serde_json::json!({ "key": key, "enabled": enabled }))
}

fn content_set_pinned(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let instance_id = arg_str(args, "instanceId")?;
    let key = arg_str(args, "key")?;
    let pinned = arg_bool(args, "pinned")?;
    let filename = filename_for_key(cx, &instance_id, &key)?;
    let service = crate::install_service::InstallService::new(cx.ctx.clone());
    let changed = service
        .set_update_pinned(&instance_id, &filename, pinned)
        .map_err(operation_failed)?;
    Ok(serde_json::json!({ "key": key, "pinned": pinned, "changed": changed }))
}

// ---------------------------------------------------------------------------
// Launch
// ---------------------------------------------------------------------------

fn launch_state(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let instance_id = arg_str(args, "instanceId")?;
    let session = cx
        .ctx
        .process_session_manager
        .list()
        .into_iter()
        .find(|session| session.instance_id == instance_id);
    let state = match session {
        Some(session) => dto::LaunchState {
            instance_id,
            status: if session.attached {
                "running"
            } else {
                "exited"
            }
            .to_string(),
            started_at: chrono::DateTime::<chrono::Utc>::from(session.start_time)
                .to_rfc3339()
                .into(),
            pid: Some(session.pid),
        },
        None => dto::LaunchState {
            instance_id,
            status: "idle".to_string(),
            started_at: None,
            pid: None,
        },
    };
    json(state)
}

fn launch_history(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let instance_id = arg_str(args, "instanceId")?;
    // Bounded regardless of what the plugin asks for: a view that renders
    // every launch ever is a view nobody reads and a payload nobody wants.
    let limit = args
        .get("limit")
        .and_then(|value| value.as_u64())
        .unwrap_or(25)
        .clamp(1, 200) as usize;
    let conn = crate::db::local_state_connection(&cx.ctx.paths.local_state_db())
        .map_err(|e| PluginError::new(PluginErrorCode::OperationFailed, e.to_string()))?;
    let records = crate::launch_history::list_history(&conn, &instance_id, limit)
        .map_err(|e| PluginError::new(PluginErrorCode::OperationFailed, e.to_string()))?;
    let entries: Vec<dto::LaunchHistoryEntry> = records
        .into_iter()
        .map(|record| dto::LaunchHistoryEntry {
            instance_id: record.instance_id,
            started_at: record.started_at,
            prep_ms: record.prep_ms,
            duration_ms: record.duration_ms,
            outcome: record.outcome.map(|outcome| outcome.as_str().to_string()),
            enabled_mod_count: record.enabled_mod_count,
            minecraft_version: record.minecraft_version,
            loader: record.loader,
            peak_memory_mb: record.peak_memory_mb,
        })
        .collect();
    json(entries)
}

// ---------------------------------------------------------------------------
// Storage and settings
// ---------------------------------------------------------------------------

fn storage_conn(cx: &DispatchContext<'_>) -> PluginResult<rusqlite::Connection> {
    crate::db::local_state_connection(&cx.ctx.paths.local_state_db())
        .map_err(|e| PluginError::new(PluginErrorCode::OperationFailed, e.to_string()))
}

fn storage_get(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
    kind: StorageKind,
) -> PluginResult<serde_json::Value> {
    let key = arg_str(args, "key")?;
    let instance_id = arg_opt_str(args, "instanceId");
    let conn = storage_conn(cx)?;
    let value = store::storage_get(&conn, cx.plugin_id, kind, instance_id.as_deref(), &key)
        .map_err(operation_failed)?;
    Ok(value.unwrap_or(serde_json::Value::Null))
}

fn storage_all(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
    kind: StorageKind,
) -> PluginResult<serde_json::Value> {
    let instance_id = arg_opt_str(args, "instanceId");
    let conn = storage_conn(cx)?;
    let mut values = store::storage_all(&conn, cx.plugin_id, kind, instance_id.as_deref())
        .map_err(operation_failed)?;

    // A declared setting the user never touched still has a value: its
    // default. Filling it in here means a plugin never has to carry its own
    // copy of the defaults and never sees `undefined` for a setting it
    // declared.
    if kind == StorageKind::Setting {
        for definition in cx.settings_schema {
            values
                .entry(definition.key.clone())
                .or_insert_with(|| definition.schema.default_value());
        }
    }
    Ok(serde_json::Value::Object(values))
}

fn storage_set(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let key = arg_str(args, "key")?;
    let instance_id = arg_opt_str(args, "instanceId");
    let value = args
        .get("value")
        .cloned()
        .ok_or_else(|| PluginError::invalid_arguments("`value` is required"))?;
    let conn = storage_conn(cx)?;
    store::storage_set(
        &conn,
        cx.plugin_id,
        StorageKind::Data,
        instance_id.as_deref(),
        &key,
        &value,
    )
    .map_err(|e| match &e {
        LauncherError::Generic { code, message } if code == "ERR_PLUGIN_STORAGE_QUOTA" => {
            PluginError::new(PluginErrorCode::ResourceExhausted, message.clone())
        }
        LauncherError::Generic { code, message } if code == "ERR_PLUGIN_STORAGE_KEY" => {
            PluginError::invalid_arguments(message.clone())
        }
        _ => operation_failed(e),
    })?;
    Ok(serde_json::json!({ "stored": true }))
}

fn storage_remove(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let key = arg_str(args, "key")?;
    let instance_id = arg_opt_str(args, "instanceId");
    let conn = storage_conn(cx)?;
    let removed = store::storage_remove(
        &conn,
        cx.plugin_id,
        StorageKind::Data,
        instance_id.as_deref(),
        &key,
    )
    .map_err(operation_failed)?;
    Ok(serde_json::json!({ "removed": removed }))
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

fn events_subscribe(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let event = arg_str(args, "event")?;
    if !PluginEvent::NAMES.contains(&event.as_str()) {
        return Err(PluginError::invalid_arguments(format!(
            "`{event}` is not an event Agora emits; known events are {}",
            PluginEvent::NAMES.join(", ")
        )));
    }
    // Subscribing is reading. The capability that guards the *event* is what
    // decides, not the fact that `events.subscribe` itself needs none.
    let capability = capability_for_event(&event);
    if let Some(capability) = capability {
        if !cx.granted.contains(capability) {
            return Err(PluginError::capability_denied(
                capability,
                &format!("events.subscribe({event})"),
            ));
        }
    }
    cx.subscriptions.subscribe(cx.plugin_id, &event);
    Ok(serde_json::json!({ "subscribed": event }))
}

fn events_unsubscribe(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let event = arg_str(args, "event")?;
    cx.subscriptions.unsubscribe(cx.plugin_id, &event);
    Ok(serde_json::json!({ "unsubscribed": event }))
}

/// The capability needed to hear about an event, by name.
pub fn capability_for_event(event: &str) -> Option<Capability> {
    match event {
        "instance.created" | "instance.deleted" | "instance.renamed" | "registry.synced" => {
            Some(Capability::InstanceRead)
        }
        "content.installed" | "content.removed" | "content.enabled" | "content.disabled" => {
            Some(Capability::ContentRead)
        }
        "launch.started" | "launch.exited" => Some(Capability::LaunchRead),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

/// How much of a plugin's HTTP response is read before giving up.
const MAX_FETCH_BYTES: usize = 2 * 1024 * 1024;

fn net_fetch_json(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let url = arg_str(args, "url")?;
    if cx.declared_hosts.is_empty() {
        return Err(PluginError::new(
            PluginErrorCode::NetworkDenied,
            "this plugin declared no hosts, so it can reach none",
        ));
    }
    // Goes through the same broker as every other outbound request: the
    // network gate (including Lockdown) runs first, then the URL is checked
    // against the plugin's declared hosts, then each redirect hop is
    // re-checked against the same list.
    let response = crate::http_client::blocking_checked_request_with_policy(
        &cx.ctx.http_clients,
        crate::http_client::ClientCategory::Plugin,
        &url,
        crate::http_client::HostPolicy::PluginDeclared(cx.declared_hosts),
    )
    .map_err(|e| {
        let message = e.to_string();
        let code = match &e {
            LauncherError::Generic { code, .. }
                if code == "ERR_NETWORK_LOCKDOWN"
                    || code == "ERR_NETWORK_ENDPOINT_DISABLED"
                    || code.starts_with("ERR_NETWORK_HOST") =>
            {
                PluginErrorCode::NetworkDenied
            }
            _ => PluginErrorCode::NetworkDenied,
        };
        PluginError::new(code, message)
    })?;

    let status = response.status().as_u16();
    let body = response.text().map_err(|e| {
        PluginError::new(
            PluginErrorCode::OperationFailed,
            format!("could not read the response body: {e}"),
        )
    })?;
    if body.len() > MAX_FETCH_BYTES {
        return Err(PluginError::new(
            PluginErrorCode::ResourceExhausted,
            format!("the response is larger than the {MAX_FETCH_BYTES} byte limit"),
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        PluginError::new(
            PluginErrorCode::OperationFailed,
            format!("the response was not JSON: {e}"),
        )
    })?;
    Ok(serde_json::json!({ "status": status, "body": parsed }))
}

// ---------------------------------------------------------------------------
// UI intents
// ---------------------------------------------------------------------------

fn ui_refresh(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    cx.ui
        .refresh(cx.plugin_id, arg_opt_str(args, "viewId").as_deref());
    Ok(serde_json::json!({ "requested": true }))
}

/// Longest toast a plugin may raise.
const MAX_NOTIFY_CHARS: usize = 200;

fn ui_notify(
    cx: &DispatchContext<'_>,
    args: &serde_json::Value,
) -> PluginResult<serde_json::Value> {
    let message = arg_str(args, "message")?;
    if message.chars().count() > MAX_NOTIFY_CHARS {
        return Err(PluginError::invalid_arguments(format!(
            "a notification may be {MAX_NOTIFY_CHARS} characters; that one is {}",
            message.chars().count()
        )));
    }
    let tone = arg_opt_str(args, "tone")
        .and_then(|tone| serde_json::from_value(serde_json::Value::String(tone)).ok())
        .unwrap_or(dto::Tone::Neutral);
    cx.ui.notify(cx.plugin_id, tone, &message);
    Ok(serde_json::json!({ "shown": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_method_name_is_unique() {
        let mut names: Vec<&str> = METHODS.iter().map(|(name, _, _)| *name).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), before, "a method name is listed twice");
    }

    #[test]
    fn an_unknown_method_is_not_a_permission_problem() {
        assert_eq!(
            required_capability("instance.destroyEverything"),
            MethodRequirement::Unknown
        );
    }

    #[test]
    fn reads_and_writes_require_different_capabilities() {
        assert_eq!(
            required_capability("content.list"),
            MethodRequirement::Requires(Capability::ContentRead)
        );
        assert_eq!(
            required_capability("content.disable"),
            MethodRequirement::Requires(Capability::ContentWrite)
        );
    }

    #[test]
    fn a_plugins_own_storage_needs_no_capability() {
        for method in [
            "storage.get",
            "storage.set",
            "storage.all",
            "storage.remove",
        ] {
            assert_eq!(
                required_capability(method),
                MethodRequirement::Open,
                "{method}"
            );
        }
    }

    #[test]
    fn every_mutating_method_requires_a_mutating_capability() {
        for (name, capability, _) in METHODS {
            let mutates = name.contains(".set")
                || name.ends_with(".enable")
                || name.ends_with(".disable")
                || name.ends_with(".rename");
            // Storage is the plugin's own, so it is excluded by design.
            if mutates && !name.starts_with("storage.") {
                let capability = capability
                    .unwrap_or_else(|| panic!("`{name}` mutates but needs no capability"));
                assert!(
                    capability.is_mutating(),
                    "`{name}` mutates but requires the non-mutating `{capability}`"
                );
            }
        }
    }

    #[test]
    fn subscribing_to_an_event_needs_the_capability_that_guards_its_data() {
        assert_eq!(
            capability_for_event("content.installed"),
            Some(Capability::ContentRead)
        );
        assert_eq!(
            capability_for_event("launch.started"),
            Some(Capability::LaunchRead)
        );
    }

    #[test]
    fn every_documented_event_has_a_guarding_capability() {
        for name in PluginEvent::NAMES {
            assert!(
                capability_for_event(name).is_some(),
                "`{name}` can be subscribed to with no capability at all"
            );
        }
    }
}
