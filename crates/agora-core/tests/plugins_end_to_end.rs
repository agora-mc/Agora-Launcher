//! The plugin system, end to end, against the runtime that actually ships.
//!
//! Everything here goes through `PluginService` and the real `QuickJsHost`:
//! a real manifest is validated, a real plugin is loaded from disk, real
//! JavaScript calls back into real `agora-core` services, and the assertions
//! are about what happened to the instance on disk — not about a mock.
//!
//! This is the P1–P2 acceptance gate in executable form. If these pass, an
//! author outside the project can install a plugin that reads their instances
//! and does something useful with them, and a plugin that misbehaves does not
//! take the launcher with it.

use agora_core::ctx::Ctx;
use agora_core::plugins::{PluginService, PLUGINS_ENABLED_SETTING};
use agora_plugin_api::error::PluginErrorCode;
use agora_plugin_api::host::ScriptHost;
use agora_plugin_api::manifest::PluginId;
use agora_plugin_host::QuickJsHost;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// A launcher with one instance in it
// ---------------------------------------------------------------------------

struct World {
    _dir: tempfile::TempDir,
    ctx: Ctx,
    service: PluginService,
}

const INSTANCE: &str = "skyblock";

fn world() -> World {
    let dir = tempfile::tempdir().expect("temp dir");
    let ctx = Ctx::for_testing(dir.path().to_path_buf());
    ctx.paths.create_required_dirs().expect("data dirs");
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).expect("db");

    let conn = agora_core::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
    agora_core::db::set_setting(&conn, PLUGINS_ENABLED_SETTING, &serde_json::json!(true)).unwrap();
    conn.execute(
        "INSERT INTO user_instances (
             instance_id, name, minecraft_version, loader, loader_version,
             is_modpack, is_locked, last_launched_at,
             jvm_memory_mb, jvm_gc, jvm_custom_args, jvm_always_pre_touch, created_at,
             java_path, java_incompatible_override, icon_path, launch_mode_override,
             jvm_memory_mode
         ) VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, NULL, 4096, 'auto', '', 0, ?6, NULL, 0, NULL, 'auto', 'auto')",
        rusqlite::params![
            INSTANCE,
            "Skyblock",
            "1.21.1",
            "fabric",
            "0.16.5",
            "2026-01-01T00:00:00Z",
        ],
    )
    .unwrap();

    // Two mods on disk and in the manifest, so content reads and writes have
    // something real to act on.
    let instance_dir = ctx.paths.instance_dir(INSTANCE).unwrap();
    let mods_dir = instance_dir.join("mods");
    std::fs::create_dir_all(&mods_dir).unwrap();
    std::fs::write(mods_dir.join("sodium.jar"), b"not really a jar").unwrap();
    std::fs::write(mods_dir.join("iris.jar"), b"not really a jar either").unwrap();

    let manifest = serde_json::json!({
        "manifest_version": 2,
        "instance_id": INSTANCE,
        "name": "Skyblock",
        "minecraft_version": "1.21.1",
        "loader": "fabric",
        "loader_version": "0.16.5",
        "mods": [
            {
                "filename": "sodium.jar",
                "registry_id": null,
                "modrinth_id": null,
                "source": "manual",
                "version": "0.6.0",
                "sha256": "a".repeat(64),
                "installed_at": "2026-01-01T00:00:00Z",
                "enabled": true,
                "content_type": "mod",
            },
            {
                "filename": "iris.jar",
                "registry_id": null,
                "modrinth_id": null,
                "source": "manual",
                "version": "1.8.0",
                "sha256": "b".repeat(64),
                "installed_at": "2026-01-01T00:00:00Z",
                "enabled": true,
                "content_type": "mod",
            },
        ],
    });
    std::fs::write(
        ctx.paths.instance_manifest(INSTANCE).unwrap(),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let host: Arc<dyn ScriptHost> = Arc::new(QuickJsHost::new());
    let service = PluginService::headless(ctx.clone(), host);
    World {
        _dir: dir,
        ctx,
        service,
    }
}

impl World {
    fn instance_manifest(&self) -> serde_json::Value {
        let text =
            std::fs::read_to_string(self.ctx.paths.instance_manifest(INSTANCE).unwrap()).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn mod_enabled(&self, filename: &str) -> bool {
        self.instance_manifest()["mods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["filename"] == filename)
            .map(|entry| entry["enabled"].as_bool().unwrap_or(true))
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// Plugin fixtures
// ---------------------------------------------------------------------------

fn plugin_folder(root: &Path, id: &str, manifest: serde_json::Value, main: &str) -> PathBuf {
    let folder = root.join(id);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("agora-plugin.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), main).unwrap();
    folder
}

fn dashboard_manifest(capabilities: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "manifest": 1,
        "id": "acme.dashboard",
        "name": "Instance Dashboard",
        "version": "1.0.0",
        "license": "MIT",
        "apiRange": ">=0.1, <0.2",
        "entrypoint": "main.js",
        "activation": ["onView:overview"],
        "capabilities": capabilities,
        "contributions": {
            "pages": [{
                "id": "overview",
                "title": "Dashboard",
                "view": { "kind": "host", "export": "overview" }
            }],
            "commands": [{ "id": "tidy", "title": "Disable Iris", "export": "tidy" }],
            "diagnostics": [{ "id": "check", "title": "Check mods", "export": "check" }],
            "settings": [{
                "key": "density",
                "title": "Density",
                "type": "enum",
                "default": "compact",
                "options": [
                    { "value": "compact", "label": "Compact" },
                    { "value": "roomy", "label": "Roomy" }
                ]
            }]
        }
    })
}

const DASHBOARD_MAIN: &str = r#"
import { instances, content, storage, settings, log } from "agora";

export async function overview() {
    const all = await instances.list();
    const detail = await instances.get(all[0].id);
    const mods = await content.list(all[0].id, "mod");
    await storage.set("lastViewed", detail.id);
    return {
        title: detail.name,
        subtitle: `${detail.minecraftVersion} / ${detail.loader}`,
        blocks: [
            {
                type: "stats",
                items: [
                    { label: "Instances", value: String(all.length) },
                    { label: "Mods", value: String(mods.length) },
                    { label: "Memory", value: `${detail.jvm.memoryMb} MB` },
                    { label: "Density", value: settings.density },
                ],
            },
            {
                type: "table",
                columns: [{ label: "Mod" }, { label: "Enabled" }],
                rows: mods.map((m) => [
                    { type: "text", text: m.displayName },
                    { type: "flag", value: m.enabled },
                ]),
            },
        ],
    };
}

export async function tidy(args) {
    const mods = await content.list(args.instanceId, "mod");
    const iris = mods.find((m) => m.filename === "iris.jar");
    await content.disable(args.instanceId, iris.key);
    log.info("disabled " + iris.filename);
    return { disabled: iris.filename };
}

export async function check(args) {
    const mods = await content.list(args.instanceId, "mod");
    const disabled = mods.filter((m) => !m.enabled);
    return {
        findings: disabled.map((m) => ({
            id: `disabled:${m.key}`,
            title: `${m.displayName} is turned off`,
            severity: "warning",
            evidence: [{ label: "File", value: m.filename }],
            repairs: [{
                id: `enable:${m.key}`,
                title: `Turn ${m.displayName} back on`,
                actions: [{ action: "enableContent", instanceId: args.instanceId, key: m.key }],
            }],
        })),
    };
}

export async function lastViewed() {
    return await storage.get("lastViewed");
}
"#;

fn id(raw: &str) -> PluginId {
    PluginId::parse(raw).unwrap()
}

// ---------------------------------------------------------------------------
// The acceptance gate
// ---------------------------------------------------------------------------

#[test]
fn a_plugin_reads_a_real_instance_and_renders_a_real_view() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({
            "required": ["instance:read", "content:read"]
        })),
        DASHBOARD_MAIN,
    );

    let summary = world
        .service
        .add_development_folder(&folder, true)
        .expect("install");
    assert_eq!(summary.id, "acme.dashboard");
    assert!(summary.enabled);
    assert!(summary.development);

    let failures = world.service.activate_all().expect("activate");
    assert!(failures.is_empty(), "{failures:?}");

    let view = world
        .service
        .render_view(&id("acme.dashboard"), "overview", serde_json::json!(null))
        .expect("render");

    assert_eq!(view.title.as_deref(), Some("Skyblock"));
    assert_eq!(view.subtitle.as_deref(), Some("1.21.1 / fabric"));

    let json = serde_json::to_value(&view).unwrap();
    let stats = &json["blocks"][0]["items"];
    assert_eq!(stats[0]["value"], "1", "instance count");
    assert_eq!(stats[1]["value"], "2", "mod count");
    assert_eq!(
        stats[2]["value"], "4096 MB",
        "memory read from the real row"
    );
    assert_eq!(stats[3]["value"], "compact", "declared setting default");

    // The table is built from the launcher's own content listing.
    let rows = json["blocks"][1]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn a_plugin_action_changes_the_instance_on_disk() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({
            "required": ["instance:read", "content:read", "content:write"]
        })),
        DASHBOARD_MAIN,
    );
    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();

    assert!(world.mod_enabled("iris.jar"));

    let result = world
        .service
        .run_command(
            &id("acme.dashboard"),
            "tidy",
            serde_json::json!({ "instanceId": INSTANCE }),
        )
        .expect("command");
    assert_eq!(result["disabled"], "iris.jar");

    // The assertion that matters: the launcher's own state changed, through
    // the launcher's own service, not through anything the plugin reached
    // around the side.
    assert!(!world.mod_enabled("iris.jar"));
    assert!(world.mod_enabled("sodium.jar"));
    let mods_dir = world.ctx.paths.instance_dir(INSTANCE).unwrap().join("mods");
    assert!(mods_dir.join("iris.jar.disabled").is_file());
    assert!(!mods_dir.join("iris.jar").exists());

    let logs = world.service.logs(&id("acme.dashboard"), 10);
    assert!(
        logs.iter().any(|line| line.contains("disabled iris.jar")),
        "{logs:?}"
    );
}

#[test]
fn a_plugin_without_the_write_capability_is_refused_at_the_boundary() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        // Reads only. The plugin's code is identical; only the grant differs.
        dashboard_manifest(serde_json::json!({
            "required": ["instance:read", "content:read"]
        })),
        DASHBOARD_MAIN,
    );
    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();

    let error = world
        .service
        .run_command(
            &id("acme.dashboard"),
            "tidy",
            serde_json::json!({ "instanceId": INSTANCE }),
        )
        .unwrap_err();
    assert_eq!(error.code, PluginErrorCode::ScriptError);
    assert!(
        error.message.contains("content:write"),
        "the plugin should have been told which capability it lacks: {}",
        error.message
    );
    // And nothing happened.
    assert!(world.mod_enabled("iris.jar"));
}

#[test]
fn a_diagnostic_produces_findings_and_a_repair_that_core_then_applies() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({
            "required": ["instance:read", "content:read", "content:write"]
        })),
        DASHBOARD_MAIN,
    );
    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();

    // Turn something off so the diagnostic has something to find.
    world
        .service
        .run_command(
            &id("acme.dashboard"),
            "tidy",
            serde_json::json!({ "instanceId": INSTANCE }),
        )
        .unwrap();
    assert!(!world.mod_enabled("iris.jar"));

    let report = world
        .service
        .run_diagnostic(
            &id("acme.dashboard"),
            "check",
            serde_json::json!({ "instanceId": INSTANCE }),
        )
        .expect("diagnostic");
    assert_eq!(report.findings.len(), 1);
    assert_eq!(
        report.highest_severity(),
        Some(agora_plugin_api::diagnostics::Severity::Warning)
    );
    assert_eq!(report.findings[0].evidence[0].value, "iris.jar");

    // The plugin proposed; core disposes.
    let proposal = &report.findings[0].repairs[0];
    let outcome = world
        .service
        .apply_repair(&id("acme.dashboard"), proposal)
        .expect("repair");
    assert!(outcome.is_complete(), "{outcome:?}");
    assert_eq!(outcome.applied.len(), 1);
    assert!(world.mod_enabled("iris.jar"));
}

#[test]
fn a_repair_for_something_that_has_since_been_removed_is_reported_as_stale() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({
            "required": ["instance:read", "content:read", "content:write"]
        })),
        DASHBOARD_MAIN,
    );
    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();

    let proposal = agora_plugin_api::diagnostics::RepairProposal {
        id: "enable-ghost".into(),
        title: "Enable something that is not there".into(),
        description: None,
        actions: vec![agora_plugin_api::diagnostics::RepairAction::EnableContent {
            instance_id: INSTANCE.into(),
            key: "mod:ghost.jar:0000".into(),
        }],
    };
    let outcome = world
        .service
        .apply_repair(&id("acme.dashboard"), &proposal)
        .expect("repair call itself succeeds");
    assert!(!outcome.is_complete());
    assert_eq!(outcome.applied.len(), 0);
    assert_eq!(outcome.stale.len(), 1);
    assert!(
        outcome.stale[0].contains("no longer installed"),
        "{:?}",
        outcome.stale
    );
}

#[test]
fn a_repair_that_contradicts_itself_is_refused_before_anything_is_applied() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({ "required": ["instance:read"] })),
        DASHBOARD_MAIN,
    );
    world.service.add_development_folder(&folder, true).unwrap();

    use agora_plugin_api::diagnostics::{RepairAction, RepairProposal};
    let proposal = RepairProposal {
        id: "confused".into(),
        title: "Both at once".into(),
        description: None,
        actions: vec![
            RepairAction::DisableContent {
                instance_id: INSTANCE.into(),
                key: "mod:sodium.jar:aaa".into(),
            },
            RepairAction::EnableContent {
                instance_id: INSTANCE.into(),
                key: "mod:sodium.jar:aaa".into(),
            },
        ],
    };
    let error = world
        .service
        .apply_repair(&id("acme.dashboard"), &proposal)
        .unwrap_err();
    assert!(error.to_string().contains("incompatible"), "{error}");
    assert!(world.mod_enabled("sodium.jar"));
}

#[test]
fn plugin_storage_survives_being_disabled_and_re_enabled() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({
            "required": ["instance:read", "content:read"]
        })),
        DASHBOARD_MAIN,
    );
    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();
    world
        .service
        .render_view(&id("acme.dashboard"), "overview", serde_json::json!(null))
        .unwrap();

    world
        .service
        .set_enabled(&id("acme.dashboard"), false)
        .unwrap();
    world
        .service
        .set_enabled(&id("acme.dashboard"), true)
        .unwrap();
    world.service.activate_all().unwrap();

    let stored = world
        .service
        .run_command(&id("acme.dashboard"), "lastViewed", serde_json::json!(null))
        .unwrap();
    assert_eq!(stored, serde_json::json!(INSTANCE));
}

#[test]
fn uninstalling_without_purging_keeps_the_users_settings_for_a_reinstall() {
    let world = world();
    let manifest = dashboard_manifest(serde_json::json!({
        "required": ["instance:read", "content:read"]
    }));
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        manifest.clone(),
        DASHBOARD_MAIN,
    );
    world.service.add_development_folder(&folder, true).unwrap();

    world
        .service
        .set_setting(
            &id("acme.dashboard"),
            "density",
            &serde_json::json!("roomy"),
        )
        .unwrap();

    world
        .service
        .uninstall(&id("acme.dashboard"), false)
        .unwrap();
    assert!(world.service.list().is_empty(), "the plugin should be gone");

    // The author's folder is untouched, because it is theirs.
    assert!(folder.join("main.js").is_file());

    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();
    let view = world
        .service
        .render_view(&id("acme.dashboard"), "overview", serde_json::json!(null))
        .unwrap();
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(
        json["blocks"][0]["items"][3]["value"], "roomy",
        "the setting the user chose should have come back"
    );
}

#[test]
fn uninstalling_with_purge_really_does_discard_the_data() {
    let world = world();
    let manifest = dashboard_manifest(serde_json::json!({
        "required": ["instance:read", "content:read"]
    }));
    let folder = plugin_folder(world._dir.path(), "dashboard", manifest, DASHBOARD_MAIN);
    world.service.add_development_folder(&folder, true).unwrap();
    world
        .service
        .set_setting(
            &id("acme.dashboard"),
            "density",
            &serde_json::json!("roomy"),
        )
        .unwrap();

    world
        .service
        .uninstall(&id("acme.dashboard"), true)
        .unwrap();
    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();

    let view = world
        .service
        .render_view(&id("acme.dashboard"), "overview", serde_json::json!(null))
        .unwrap();
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["blocks"][0]["items"][3]["value"], "compact");
}

#[test]
fn a_setting_the_plugin_never_declared_cannot_be_written() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({ "required": ["instance:read"] })),
        DASHBOARD_MAIN,
    );
    world.service.add_development_folder(&folder, true).unwrap();

    assert!(world
        .service
        .set_setting(&id("acme.dashboard"), "sneaky", &serde_json::json!("x"))
        .is_err());
    // And a declared setting still rejects a value outside its schema.
    assert!(world
        .service
        .set_setting(
            &id("acme.dashboard"),
            "density",
            &serde_json::json!("enormous")
        )
        .is_err());
}

#[test]
fn installing_a_plugin_that_wants_capabilities_requires_consent() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({ "required": ["instance:read"] })),
        DASHBOARD_MAIN,
    );
    let error = world
        .service
        .add_development_folder(&folder, false)
        .unwrap_err();
    assert!(error.to_string().contains("not been accepted"), "{error}");
    assert!(world.service.list().is_empty());
}

#[test]
fn a_plugin_that_throws_on_activation_is_recorded_and_the_rest_still_run() {
    let world = world();
    let broken = plugin_folder(
        world._dir.path(),
        "broken",
        serde_json::json!({
            "manifest": 1,
            "id": "broken.one",
            "name": "Broken",
            "version": "1.0.0",
            "license": "MIT",
            "apiRange": ">=0.1, <0.2",
            "entrypoint": "main.js",
        }),
        "export function activate() { throw new Error('nope'); }",
    );
    let healthy = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({
            "required": ["instance:read", "content:read"]
        })),
        DASHBOARD_MAIN,
    );

    world.service.add_development_folder(&broken, true).unwrap();
    world
        .service
        .add_development_folder(&healthy, true)
        .unwrap();

    let failures = world.service.activate_all().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0.as_str(), "broken.one");

    // The healthy plugin still works, which is the whole point of isolation.
    assert!(world
        .service
        .render_view(&id("acme.dashboard"), "overview", serde_json::json!(null))
        .is_ok());

    // And the failure is visible rather than buried in a log.
    let broken_summary = world
        .service
        .list()
        .into_iter()
        .find(|summary| summary.id == "broken.one")
        .unwrap();
    assert!(
        broken_summary.status_text.contains("nope"),
        "{}",
        broken_summary.status_text
    );
    assert!(!broken_summary.running);
}

#[test]
fn disable_all_is_a_recovery_path_that_does_not_need_plugins_to_cooperate() {
    let world = world();
    let hostile = plugin_folder(
        world._dir.path(),
        "hostile",
        serde_json::json!({
            "manifest": 1,
            "id": "hostile.one",
            "name": "Hostile",
            "version": "1.0.0",
            "license": "MIT",
            "apiRange": ">=0.1, <0.2",
            "entrypoint": "main.js",
        }),
        // Refuses to shut down cleanly.
        "export function deactivate() { for (;;) {} }",
    );
    world
        .service
        .add_development_folder(&hostile, true)
        .unwrap();
    world.service.activate_all().unwrap();

    let started = std::time::Instant::now();
    let disabled = world.service.disable_all().expect("disable all");
    assert_eq!(disabled, 1);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "recovery took {:?}",
        started.elapsed()
    );

    let summary = &world.service.list()[0];
    assert!(!summary.enabled);
    assert!(!summary.running);
}

#[test]
fn a_plugin_that_is_turned_off_contributes_nothing_to_the_ui() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({ "required": ["instance:read"] })),
        DASHBOARD_MAIN,
    );
    world.service.add_development_folder(&folder, true).unwrap();
    assert!(!world.service.contributions().is_empty());

    world
        .service
        .set_enabled(&id("acme.dashboard"), false)
        .unwrap();
    assert!(
        world.service.contributions().is_empty(),
        "a disabled plugin must leave no pages, commands or settings behind"
    );
}

#[test]
fn contributions_carry_the_plugin_namespace_so_the_ui_can_route_them_back() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({ "required": ["instance:read"] })),
        DASHBOARD_MAIN,
    );
    world.service.add_development_folder(&folder, true).unwrap();

    let ids: Vec<String> = world
        .service
        .contributions()
        .into_iter()
        .map(|contribution| contribution.id)
        .collect();
    assert!(
        ids.contains(&"acme.dashboard/overview".to_string()),
        "{ids:?}"
    );
    assert!(ids.contains(&"acme.dashboard/tidy".to_string()), "{ids:?}");
    assert!(
        ids.contains(&"acme.dashboard/density".to_string()),
        "{ids:?}"
    );
}

#[test]
fn nothing_activates_while_the_plugin_system_is_switched_off() {
    let world = world();
    let conn = agora_core::db::local_state_connection(&world.ctx.paths.local_state_db()).unwrap();
    agora_core::db::set_setting(&conn, PLUGINS_ENABLED_SETTING, &serde_json::json!(false)).unwrap();

    let folder = plugin_folder(
        world._dir.path(),
        "dashboard",
        dashboard_manifest(serde_json::json!({ "required": ["instance:read"] })),
        DASHBOARD_MAIN,
    );
    world.service.add_development_folder(&folder, true).unwrap();

    assert!(!world.service.is_enabled());
    let failures = world.service.activate_all().unwrap();
    assert!(failures.is_empty());
    assert!(!world.service.list()[0].running);
    assert!(world.service.contributions().is_empty());
    assert!(world
        .service
        .run_command(&id("acme.dashboard"), "lastViewed", serde_json::Value::Null)
        .is_err());
}

#[test]
fn shipped_examples_install_and_run_using_the_public_contract() {
    let world = world();
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");
    for name in ["dashboard", "diagnostics", "theme", "custom-dashboard"] {
        world
            .service
            .add_development_folder(&examples.join(name), true)
            .unwrap();
    }
    assert!(world.service.activate_all().unwrap().is_empty());
    let dashboard = id("agora.dashboard");
    world
        .service
        .render_view(&dashboard, "dashboard", serde_json::Value::Null)
        .unwrap();
    let result = world
        .service
        .run_command(&dashboard, "remember", serde_json::Value::Null)
        .unwrap();
    assert_eq!(result["count"], 1);
    assert!(world
        .service
        .contributions()
        .iter()
        .any(|entry| entry.id == "agora.forest/forest"));
    let html = world
        .service
        .custom_view_html(&id("agora.custom-dashboard"), "dashboard")
        .unwrap();
    assert!(html.contains("agora:command"));
    assert!(world
        .service
        .custom_view_html(&id("agora.custom-dashboard"), "../main.js")
        .is_err());
    world
        .service
        .set_enabled(&id("agora.custom-dashboard"), false)
        .unwrap();
    assert!(world
        .service
        .custom_view_html(&id("agora.custom-dashboard"), "dashboard")
        .is_err());
}

#[test]
fn a_development_folder_cannot_shadow_an_installed_package_of_the_same_id() {
    let world = world();
    let manifest = dashboard_manifest(serde_json::json!({ "required": ["instance:read"] }));

    // Build a package with the same id and install it.
    let archive = world._dir.path().join("acme.dashboard.zip");
    {
        use std::io::Write;
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("agora-plugin.json", options).unwrap();
        zip.write_all(serde_json::to_string(&manifest).unwrap().as_bytes())
            .unwrap();
        zip.start_file("main.js", options).unwrap();
        zip.write_all(DASHBOARD_MAIN.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    world.service.install_package(&archive, true).unwrap();

    let folder = plugin_folder(world._dir.path(), "dashboard", manifest, DASHBOARD_MAIN);
    let error = world
        .service
        .add_development_folder(&folder, true)
        .unwrap_err();
    assert!(
        error.to_string().contains("already installed as a package"),
        "{error}"
    );
}

#[test]
fn a_package_install_lands_files_in_agoras_own_directory() {
    let world = world();
    // The view reads content as well as instances, so the grant has to cover
    // both; granting less is a different test.
    let manifest = dashboard_manifest(serde_json::json!({
        "required": ["instance:read", "content:read"]
    }));
    let archive = world._dir.path().join("acme.dashboard.zip");
    {
        use std::io::Write;
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("agora-plugin.json", options).unwrap();
        zip.write_all(serde_json::to_string(&manifest).unwrap().as_bytes())
            .unwrap();
        zip.start_file("main.js", options).unwrap();
        zip.write_all(DASHBOARD_MAIN.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    let preview = world.service.preview_package(&archive).unwrap();
    assert_eq!(preview.manifest.id.as_str(), "acme.dashboard");
    assert_eq!(preview.required_capabilities.len(), 2);
    assert!(preview.replaces_version.is_none());
    assert!(!preview.migrates_data);

    let summary = world.service.install_package(&archive, true).unwrap();
    assert!(!summary.development);

    let installed = world
        .ctx
        .paths
        .plugin_packages_root()
        .join("acme.dashboard");
    assert!(installed.join("main.js").is_file());
    assert!(installed.join("agora-plugin.json").is_file());

    world.service.activate_all().unwrap();
    assert!(world
        .service
        .render_view(&id("acme.dashboard"), "overview", serde_json::json!(null))
        .is_ok());
}

// ---------------------------------------------------------------------------
// Activation events actually gate activation
// ---------------------------------------------------------------------------

/// A plugin whose manifest declares exactly `activation`, and which records
/// whether it ever ran.
fn lazily_activated(world: &World, id: &str, activation: serde_json::Value) -> PathBuf {
    plugin_folder(
        world._dir.path(),
        id,
        serde_json::json!({
            "manifest": 1,
            "id": id,
            "name": id,
            "version": "1.0.0",
            "license": "MIT",
            "apiRange": ">=0.1, <0.2",
            "entrypoint": "main.js",
            "activation": activation,
            "capabilities": { "required": ["instance:read", "content:read"] },
            "contributions": {
                "pages": [{
                    "id": "overview",
                    "title": "Overview",
                    "view": { "kind": "host", "export": "overview" }
                }]
            }
        }),
        r#"
        export function overview() {
            return { blocks: [{ type: "text", text: "hello" }] };
        }
        "#,
    )
}

#[test]
fn a_plugin_that_only_declares_a_view_is_not_started_at_launch() {
    let world = world();
    let folder = lazily_activated(&world, "lazy.page", serde_json::json!(["onView:overview"]));
    world.service.add_development_folder(&folder, true).unwrap();

    assert!(world.service.activate_all().unwrap().is_empty());
    // The whole promise of activation events: an installed plugin that nothing
    // has asked for yet costs no runtime.
    assert!(
        !world.service.list()[0].running,
        "a plugin declaring only `onView:` must not be started eagerly"
    );
}

#[test]
fn opening_that_plugins_view_is_what_starts_it() {
    let world = world();
    let folder = lazily_activated(&world, "lazy.page", serde_json::json!(["onView:overview"]));
    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();
    assert!(!world.service.list()[0].running);

    world
        .service
        .render_view(&id("lazy.page"), "overview", serde_json::Value::Null)
        .expect("the view should start its own plugin on demand");
    assert!(world.service.list()[0].running);
}

#[test]
fn a_plugin_that_asks_for_startup_is_started_at_launch() {
    let world = world();
    let folder = lazily_activated(
        &world,
        "eager.page",
        serde_json::json!(["onStartup", "onView:overview"]),
    );
    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();
    assert!(world.service.list()[0].running);
}

#[test]
fn a_plugin_declaring_no_activation_at_all_still_starts() {
    // Otherwise the simplest possible plugin would never run, which is a worse
    // failure than starting one runtime that was not strictly needed.
    let world = world();
    let folder = lazily_activated(&world, "plain.page", serde_json::json!([]));
    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();
    assert!(world.service.list()[0].running);
}

#[test]
fn an_event_starts_the_plugin_that_was_waiting_for_it() {
    use agora_plugin_api::protocol::PluginEvent;

    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "waiter",
        serde_json::json!({
            "manifest": 1,
            "id": "waiter.one",
            "name": "Waits for an event",
            "version": "1.0.0",
            "license": "MIT",
            "apiRange": ">=0.1, <0.2",
            "entrypoint": "main.js",
            "activation": ["onEvent:content.installed"],
            "capabilities": { "required": ["content:read"] }
        }),
        r#"
        import { on } from "agora";
        let seen = 0;
        export function activate() { on("content.installed", () => { seen += 1; }); }
        export function count() { return seen; }
        "#,
    );
    world.service.add_development_folder(&folder, true).unwrap();

    // Nothing has happened yet, so nothing is running.
    world.service.activate_all().unwrap();
    assert!(!world.service.list()[0].running);

    // Publishing through the service — not straight to the bus — is what makes
    // `onEvent:` mean anything: the plugin has to be started before it can
    // possibly have subscribed.
    world.service.publish_event(
        PluginEvent::ContentInstalled {
            instance_id: INSTANCE.into(),
            key: "mod:sodium.jar:aaa".into(),
        },
        None,
    );
    assert!(
        world.service.list()[0].running,
        "an `onEvent:` declaration must start the plugin when that event fires"
    );
}

#[test]
fn an_unrelated_event_does_not_start_a_plugin_waiting_for_a_different_one() {
    use agora_plugin_api::protocol::PluginEvent;

    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "waiter",
        serde_json::json!({
            "manifest": 1,
            "id": "waiter.one",
            "name": "Waits for an event",
            "version": "1.0.0",
            "license": "MIT",
            "apiRange": ">=0.1, <0.2",
            "entrypoint": "main.js",
            "activation": ["onEvent:content.installed"],
            "capabilities": { "required": ["launch:read"] }
        }),
        "export function activate() {}",
    );
    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();

    world.service.publish_event(
        PluginEvent::LaunchStarted {
            instance_id: INSTANCE.into(),
        },
        None,
    );
    assert!(!world.service.list()[0].running);
}

#[test]
fn opening_an_instance_starts_a_plugin_that_asked_for_it() {
    let world = world();
    let folder = plugin_folder(
        world._dir.path(),
        "onopen",
        serde_json::json!({
            "manifest": 1,
            "id": "onopen.one",
            "name": "Reacts to an instance opening",
            "version": "1.0.0",
            "license": "MIT",
            "apiRange": ">=0.1, <0.2",
            "entrypoint": "main.js",
            "activation": ["onInstanceOpened"],
            "capabilities": { "required": ["instance:read"] }
        }),
        "export function activate() {}",
    );
    world.service.add_development_folder(&folder, true).unwrap();
    world.service.activate_all().unwrap();
    assert!(!world.service.list()[0].running);

    assert_eq!(world.service.notify_instance_opened(), 1);
    assert!(world.service.list()[0].running);
}

#[test]
fn nothing_is_woken_by_an_event_while_plugins_are_switched_off() {
    use agora_plugin_api::protocol::PluginEvent;

    let world = world();
    let folder = lazily_activated(
        &world,
        "lazy.page",
        serde_json::json!(["onEvent:launch.started"]),
    );
    world.service.add_development_folder(&folder, true).unwrap();

    let conn = agora_core::db::local_state_connection(&world.ctx.paths.local_state_db()).unwrap();
    agora_core::db::set_setting(&conn, PLUGINS_ENABLED_SETTING, &serde_json::json!(false)).unwrap();

    assert_eq!(
        world.service.publish_event(
            PluginEvent::LaunchStarted {
                instance_id: INSTANCE.into()
            },
            None
        ),
        0
    );
    assert!(!world.service.list()[0].running);
    assert_eq!(world.service.notify_instance_opened(), 0);
}
