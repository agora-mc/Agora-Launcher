//! What the script host actually guarantees, exercised against real plugins.
//!
//! These are the P0 acceptance gates in executable form. Each one is a claim
//! the rest of the plugin system leans on — "a broken plugin cannot break the
//! launcher" is only worth saying if something checks it.

use agora_plugin_api::error::PluginErrorCode;
use agora_plugin_api::host::{ActivationRequest, HostBridge, ScriptHost};
use agora_plugin_api::manifest::PluginId;
use agora_plugin_api::protocol::{
    EventOrigin, HostRequest, HostResponse, LogLevel, PluginEvent, PluginEventEnvelope,
};
use agora_plugin_host::QuickJsHost;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------------------------------------------------------------------------
// A bridge that stands in for agora-core
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestBridge {
    calls: AtomicU64,
    logs: Mutex<Vec<(LogLevel, String)>>,
    /// Methods that should be refused, to exercise the error path.
    denied: Mutex<Vec<String>>,
}

impl TestBridge {
    fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn logs(&self) -> Vec<(LogLevel, String)> {
        self.logs.lock().unwrap().clone()
    }
}

impl HostBridge for TestBridge {
    fn call(&self, _plugin_id: &PluginId, request: HostRequest) -> HostResponse {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.denied.lock().unwrap().contains(&request.method) {
            return HostResponse::Error {
                request_id: request.request_id,
                error: agora_plugin_api::PluginError::capability_denied(
                    "instance:write",
                    &request.method,
                ),
            };
        }
        let value = match request.method.as_str() {
            "instance.list" => serde_json::json!([
                { "id": "inst-a", "name": "Skyblock" },
                { "id": "inst-b", "name": "Vanilla+" },
            ]),
            "instance.get" => {
                serde_json::json!({ "id": request.args["instanceId"], "modCount": 7 })
            }
            // Far slower than any deadline a test sets, so "the timeout fired"
            // and "the call happened to finish" can never be confused.
            "slow" => {
                std::thread::sleep(Duration::from_millis(1_500));
                serde_json::json!("eventually")
            }
            "events.subscribe" | "events.unsubscribe" => serde_json::json!({ "ok": true }),
            _ => serde_json::json!(null),
        };
        HostResponse::Ok {
            request_id: request.request_id,
            value,
        }
    }

    fn log(&self, _plugin_id: &PluginId, level: LogLevel, message: &str) {
        self.logs.lock().unwrap().push((level, message.to_string()));
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct Fixture {
    _dir: tempfile::TempDir,
    request: ActivationRequest,
}

fn plugin(id: &str, files: &[(&str, &str)]) -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, source) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, source).unwrap();
    }
    Fixture {
        request: ActivationRequest {
            plugin_id: PluginId::parse(id).unwrap(),
            package_root: dir.path().to_path_buf(),
            entrypoint: "main.js".into(),
            memory_limit_bytes: ActivationRequest::DEFAULT_MEMORY_LIMIT,
            stack_limit_bytes: ActivationRequest::DEFAULT_STACK_LIMIT,
            api_version: agora_plugin_api::host_api_version_string(),
            settings: serde_json::json!({ "density": "compact" }),
        },
        _dir: dir,
    }
}

fn id(raw: &str) -> PluginId {
    PluginId::parse(raw).unwrap()
}

// ---------------------------------------------------------------------------
// Gate: a plugin can read real data and act on it
// ---------------------------------------------------------------------------

#[test]
fn a_plugin_reads_through_the_sdk_and_returns_a_value() {
    let host = QuickJsHost::new();
    let bridge = TestBridge::shared();
    let fixture = plugin(
        "acme.dashboard",
        &[(
            "main.js",
            r#"
            import { instances, settings, pluginId, apiVersion } from "agora";
            export async function overview() {
                const all = await instances.list();
                const first = await instances.get(all[0].id);
                return {
                    count: all.length,
                    mods: first.modCount,
                    density: settings.density,
                    pluginId,
                    apiVersion,
                };
            }
            "#,
        )],
    );

    host.activate(fixture.request.clone(), bridge.clone())
        .expect("activation");
    let value = host
        .invoke(
            &id("acme.dashboard"),
            "overview",
            serde_json::json!(null),
            Duration::from_secs(5),
        )
        .expect("invoke");

    assert_eq!(value["count"], 2);
    assert_eq!(value["mods"], 7);
    assert_eq!(value["density"], "compact");
    assert_eq!(value["pluginId"], "acme.dashboard");
    assert_eq!(
        value["apiVersion"],
        agora_plugin_api::host_api_version_string()
    );
    host.deactivate(&id("acme.dashboard")).unwrap();
}

#[test]
fn arguments_reach_the_export_and_activate_runs_first() {
    let host = QuickJsHost::new();
    let bridge = TestBridge::shared();
    let fixture = plugin(
        "acme.args",
        &[(
            "main.js",
            r#"
            let ready = false;
            export function activate() { ready = true; }
            export function echo(args) { return { ready, got: args.value }; }
            "#,
        )],
    );
    host.activate(fixture.request.clone(), bridge).unwrap();
    let value = host
        .invoke(
            &id("acme.args"),
            "echo",
            serde_json::json!({ "value": 42 }),
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(value["ready"], true);
    assert_eq!(value["got"], 42);
}

#[test]
fn a_host_error_surfaces_in_javascript_as_a_catchable_error_with_its_code() {
    let host = QuickJsHost::new();
    let bridge = TestBridge::shared();
    bridge
        .denied
        .lock()
        .unwrap()
        .push("instance.rename".to_string());
    let fixture = plugin(
        "acme.denied",
        &[(
            "main.js",
            r#"
            import { instances } from "agora";
            export async function attempt() {
                try {
                    await instances.rename("inst-a", "nope");
                    return { threw: false };
                } catch (e) {
                    return { threw: true, code: e.code, message: e.message };
                }
            }
            "#,
        )],
    );
    host.activate(fixture.request.clone(), bridge).unwrap();
    let value = host
        .invoke(
            &id("acme.denied"),
            "attempt",
            serde_json::json!(null),
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(value["threw"], true);
    assert_eq!(value["code"], "CAPABILITY_DENIED");
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("instance:write"),
        "{value}"
    );
}

// ---------------------------------------------------------------------------
// Gate: module resolution stays inside the package
// ---------------------------------------------------------------------------

#[test]
fn a_relative_import_inside_the_package_resolves() {
    let host = QuickJsHost::new();
    let fixture = plugin(
        "acme.multi",
        &[
            (
                "main.js",
                r#"
                import { double } from "./lib/math.js";
                export function run() { return double(21); }
                "#,
            ),
            ("lib/math.js", "export const double = (n) => n * 2;"),
        ],
    );
    host.activate(fixture.request.clone(), TestBridge::shared())
        .unwrap();
    let value = host
        .invoke(
            &id("acme.multi"),
            "run",
            serde_json::json!(null),
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(value, 42);
}

#[test]
fn an_import_that_climbs_out_of_the_package_is_refused_at_activation() {
    let host = QuickJsHost::new();
    let fixture = plugin(
        "acme.escape",
        &[(
            "main.js",
            r#"import "../../../secrets.js"; export function run() { return 1; }"#,
        )],
    );
    let err = host
        .activate(fixture.request.clone(), TestBridge::shared())
        .unwrap_err();
    assert_eq!(err.code, PluginErrorCode::ScriptError);
    assert!(!host.is_active(&id("acme.escape")));
}

#[test]
fn a_bare_specifier_is_refused_with_advice_rather_than_a_node_lookup() {
    let host = QuickJsHost::new();
    let fixture = plugin(
        "acme.bare",
        &[(
            "main.js",
            r#"import lodash from "lodash"; export function run() { return 1; }"#,
        )],
    );
    let err = host
        .activate(fixture.request.clone(), TestBridge::shared())
        .unwrap_err();
    assert!(
        err.message.contains("Bundle your dependencies"),
        "{}",
        err.message
    );
}

// ---------------------------------------------------------------------------
// Gate: a runaway plugin is stopped
// ---------------------------------------------------------------------------

#[test]
fn a_plugin_spinning_forever_is_interrupted_and_the_host_survives() {
    let host = QuickJsHost::new();
    let fixture = plugin(
        "acme.spin",
        &[(
            "main.js",
            "export function run() { let n = 0; for (;;) { n++; } }",
        )],
    );
    host.activate(fixture.request.clone(), TestBridge::shared())
        .unwrap();

    let err = host
        .invoke(
            &id("acme.spin"),
            "run",
            serde_json::json!(null),
            Duration::from_millis(300),
        )
        .unwrap_err();
    assert_eq!(err.code, PluginErrorCode::Timeout);

    // The point of the gate: the plugin is still usable afterwards, which
    // means the interrupt unwound cleanly rather than wedging the runtime.
    let value = host
        .invoke(
            &id("acme.spin"),
            "run",
            serde_json::json!(null),
            Duration::from_millis(200),
        )
        .unwrap_err();
    assert_eq!(value.code, PluginErrorCode::Timeout);
    host.deactivate(&id("acme.spin")).ok();
}

#[test]
fn a_plugin_awaiting_a_slow_host_call_still_hits_its_deadline() {
    let host = QuickJsHost::new();
    let fixture = plugin(
        "acme.waiter",
        &[(
            "main.js",
            r#"
            import { call } from "agora";
            export async function run() { return await call("slow"); }
            "#,
        )],
    );
    host.activate(fixture.request.clone(), TestBridge::shared())
        .unwrap();
    // The bridge sleeps 1500ms; the deadline is 200ms. Nothing is *running* in
    // JS here — the engine is parked on a future — so the interrupt handler
    // can never fire and only the Tokio timeout can catch this.
    let started = std::time::Instant::now();
    let err = host
        .invoke(
            &id("acme.waiter"),
            "run",
            serde_json::json!(null),
            Duration::from_millis(200),
        )
        .unwrap_err();
    assert_eq!(err.code, PluginErrorCode::Timeout);
    assert!(
        started.elapsed() < Duration::from_millis(1_200),
        "waited {:?}, so the call completed rather than timing out",
        started.elapsed()
    );
}

#[test]
fn cancel_stops_a_plugin_that_is_mid_loop() {
    let host = Arc::new(QuickJsHost::new());
    let fixture = plugin(
        "acme.cancelme",
        &[(
            "main.js",
            "export function run() { let n = 0; for (;;) { n++; } }",
        )],
    );
    host.activate(fixture.request.clone(), TestBridge::shared())
        .unwrap();

    let canceller = host.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        canceller.cancel(&id("acme.cancelme"));
    });

    let started = std::time::Instant::now();
    let err = host
        .invoke(
            &id("acme.cancelme"),
            "run",
            serde_json::json!(null),
            Duration::from_secs(30),
        )
        .unwrap_err();
    // Returned because it was cancelled, not because 30s elapsed.
    assert!(started.elapsed() < Duration::from_secs(5), "{err:?}");
}

// ---------------------------------------------------------------------------
// Gate: failure is isolated
// ---------------------------------------------------------------------------

#[test]
fn a_plugin_that_throws_on_activation_does_not_stop_another_from_running() {
    let host = QuickJsHost::new();
    let bridge = TestBridge::shared();

    let broken = plugin(
        "broken.one",
        &[(
            "main.js",
            "export function activate() { throw new TypeError('I am broken'); }",
        )],
    );
    let err = host
        .activate(broken.request.clone(), bridge.clone())
        .unwrap_err();
    assert_eq!(err.code, PluginErrorCode::ScriptError);
    assert!(err.message.contains("I am broken"), "{}", err.message);
    assert!(!host.is_active(&id("broken.one")));

    let healthy = plugin(
        "healthy.two",
        &[(
            "main.js",
            r#"
            import { instances } from "agora";
            export async function run() { return (await instances.list()).length; }
            "#,
        )],
    );
    host.activate(healthy.request.clone(), bridge).unwrap();
    let value = host
        .invoke(
            &id("healthy.two"),
            "run",
            serde_json::json!(null),
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(value, 2);
}

#[test]
fn a_plugin_exceeding_its_memory_ceiling_fails_without_taking_the_process() {
    let host = QuickJsHost::new();
    let mut fixture = plugin(
        "acme.greedy",
        &[(
            "main.js",
            "export function run() { const a = []; for (;;) { a.push(new Array(10000).fill('x')); } }",
        )],
    );
    fixture.request.memory_limit_bytes = 2 * 1024 * 1024;
    host.activate(fixture.request.clone(), TestBridge::shared())
        .unwrap();
    let err = host
        .invoke(
            &id("acme.greedy"),
            "run",
            serde_json::json!(null),
            Duration::from_secs(10),
        )
        .unwrap_err();
    assert_eq!(err.code, PluginErrorCode::ResourceExhausted);
}

#[test]
fn calling_an_export_a_plugin_does_not_have_names_the_missing_export() {
    let host = QuickJsHost::new();
    let fixture = plugin(
        "acme.thin",
        &[("main.js", "export function run() { return 1; }")],
    );
    host.activate(fixture.request.clone(), TestBridge::shared())
        .unwrap();
    let err = host
        .invoke(
            &id("acme.thin"),
            "nope",
            serde_json::json!(null),
            Duration::from_secs(2),
        )
        .unwrap_err();
    assert_eq!(err.code, PluginErrorCode::ScriptError);
    assert!(err.message.contains("`nope`"), "{}", err.message);
}

#[test]
fn invoking_a_plugin_that_is_not_running_says_so() {
    let host = QuickJsHost::new();
    let err = host
        .invoke(
            &id("never.installed"),
            "run",
            serde_json::json!(null),
            Duration::from_secs(1),
        )
        .unwrap_err();
    assert_eq!(err.code, PluginErrorCode::NotActivated);
}

#[test]
fn activating_the_same_plugin_twice_is_refused() {
    let host = QuickJsHost::new();
    let fixture = plugin(
        "acme.once",
        &[("main.js", "export function run() { return 1; }")],
    );
    host.activate(fixture.request.clone(), TestBridge::shared())
        .unwrap();
    assert!(host
        .activate(fixture.request.clone(), TestBridge::shared())
        .is_err());
}

// ---------------------------------------------------------------------------
// Gate: events
// ---------------------------------------------------------------------------

fn envelope(sequence: u64, event: PluginEvent) -> PluginEventEnvelope {
    PluginEventEnvelope {
        sequence,
        event,
        origin: EventOrigin::User,
        depth: 0,
        operation_id: None,
    }
}

#[test]
fn a_subscriber_receives_events_and_can_act_on_them() {
    let host = QuickJsHost::new();
    let bridge = TestBridge::shared();
    let fixture = plugin(
        "acme.watcher",
        &[(
            "main.js",
            r#"
            import { on, log } from "agora";
            let seen = [];
            export function activate() {
                on("content.installed", (e) => { seen.push(e.key); log.info("saw " + e.key); });
            }
            export function report() { return seen; }
            "#,
        )],
    );
    host.activate(fixture.request.clone(), bridge.clone())
        .unwrap();

    host.deliver_event(
        &id("acme.watcher"),
        &envelope(
            1,
            PluginEvent::ContentInstalled {
                instance_id: "inst-a".into(),
                key: "sodium".into(),
            },
        ),
    )
    .unwrap();

    // Give the plugin's loop a moment to drain its queue.
    let mut seen = serde_json::Value::Null;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(20));
        seen = host
            .invoke(
                &id("acme.watcher"),
                "report",
                serde_json::json!(null),
                Duration::from_secs(2),
            )
            .unwrap();
        if seen.as_array().is_some_and(|a| !a.is_empty()) {
            break;
        }
    }
    assert_eq!(seen, serde_json::json!(["sodium"]));
    assert!(bridge
        .logs()
        .iter()
        .any(|(level, msg)| *level == LogLevel::Info && msg.contains("sodium")));
}

#[test]
fn a_flood_of_events_is_dropped_rather_than_blocking_the_caller() {
    let host = QuickJsHost::new();
    let fixture = plugin(
        "acme.slowsub",
        &[(
            "main.js",
            r#"
            import { on } from "agora";
            export function activate() {
                // A handler that burns time, so the queue cannot drain.
                on("launch.started", () => { const end = Date.now() + 60; while (Date.now() < end) {} });
            }
            "#,
        )],
    );
    host.activate(fixture.request.clone(), TestBridge::shared())
        .unwrap();

    let started = std::time::Instant::now();
    let mut dropped = 0;
    for sequence in 0..(agora_plugin_host::EVENT_QUEUE_CAPACITY as u64 * 4) {
        let result = host.deliver_event(
            &id("acme.slowsub"),
            &envelope(
                sequence,
                PluginEvent::LaunchStarted {
                    instance_id: "inst-a".into(),
                },
            ),
        );
        if let Err(e) = result {
            assert_eq!(e.code, PluginErrorCode::ResourceExhausted);
            dropped += 1;
        }
    }
    assert!(dropped > 0, "a full queue should have dropped something");
    // The whole point: emitting events never waits on the plugin.
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "delivering events blocked for {:?}",
        started.elapsed()
    );
}

// ---------------------------------------------------------------------------
// Gate: lifecycle
// ---------------------------------------------------------------------------

#[test]
fn deactivate_runs_the_plugin_cleanup_export_and_stops_the_runtime() {
    let host = QuickJsHost::new();
    let bridge = TestBridge::shared();
    let fixture = plugin(
        "acme.tidy",
        &[(
            "main.js",
            r#"
            import { log } from "agora";
            export function deactivate() { log.info("cleaned up"); }
            "#,
        )],
    );
    host.activate(fixture.request.clone(), bridge.clone())
        .unwrap();
    assert!(host.is_active(&id("acme.tidy")));

    host.deactivate(&id("acme.tidy")).unwrap();
    assert!(!host.is_active(&id("acme.tidy")));
    assert!(bridge.logs().iter().any(|(_, msg)| msg == "cleaned up"));
}

#[test]
fn deactivating_something_that_never_ran_is_not_an_error() {
    let host = QuickJsHost::new();
    assert!(host.deactivate(&id("never.installed")).is_ok());
}

#[test]
fn a_plugin_without_an_activate_export_still_activates() {
    let host = QuickJsHost::new();
    let fixture = plugin(
        "acme.lazy",
        &[("main.js", "export function run() { return 'fine'; }")],
    );
    host.activate(fixture.request.clone(), TestBridge::shared())
        .unwrap();
    assert!(host.is_active(&id("acme.lazy")));
    assert_eq!(host.active_plugins(), vec![id("acme.lazy")]);
}

#[test]
fn a_plugin_whose_entrypoint_is_missing_reports_a_package_problem() {
    let dir = tempfile::tempdir().unwrap();
    let host = QuickJsHost::new();
    let request = ActivationRequest {
        plugin_id: id("acme.absent"),
        package_root: dir.path().to_path_buf(),
        entrypoint: "main.js".into(),
        memory_limit_bytes: ActivationRequest::DEFAULT_MEMORY_LIMIT,
        stack_limit_bytes: ActivationRequest::DEFAULT_STACK_LIMIT,
        api_version: agora_plugin_api::host_api_version_string(),
        settings: serde_json::Value::Null,
    };
    let err = host.activate(request, TestBridge::shared()).unwrap_err();
    assert_eq!(err.code, PluginErrorCode::InvalidPackage);
}

#[test]
fn the_host_names_the_engine_it_runs() {
    assert!(QuickJsHost::new().describe().contains("quickjs"));
}
