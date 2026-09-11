//! One plugin, one thread, one QuickJS runtime.
//!
//! Isolation here is structural rather than conventional. Each plugin gets its
//! own OS thread, its own `AsyncRuntime`, its own heap ceiling and its own
//! interrupt flag. Nothing is shared between two plugins except the host
//! bridge, which is stateless and re-checks capabilities on every call. A
//! plugin that throws, loops forever, or exhausts its heap takes down exactly
//! itself.
//!
//! The two ways a plugin gets stopped are complementary and both are needed:
//!
//! - a **QuickJS interrupt handler** stops JavaScript that is *running*, which
//!   is the `while (true) {}` case, and
//! - a **Tokio timeout** stops a task that is *awaiting*, which is the case
//!   where a host call is slow.
//!
//! Neither alone is sufficient: the interrupt handler never fires while the
//! engine is parked on a future, and the timeout never fires while the engine
//! is busy in a tight loop that yields to nothing.

use agora_plugin_api::error::{PluginError, PluginErrorCode, PluginResult};
use agora_plugin_api::host::{ActivationRequest, HostBridge};
use agora_plugin_api::protocol::{HostRequest, HostResponse, LogLevel, PluginEventEnvelope};
use rquickjs::function::Async;
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Function, Module, Object, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::modules::{PackageLoader, PackageResolver};
use crate::sdk::AGORA_MODULE;

/// How many events may queue for one plugin before the oldest are dropped.
///
/// A bound rather than an unbounded queue because the alternative to dropping
/// events is either unbounded memory in the host or back-pressure onto the
/// core operation that emitted them — and a slow plugin must never be able to
/// slow down an install.
pub const EVENT_QUEUE_CAPACITY: usize = 64;

/// How long a single event handler may run before it is interrupted.
const EVENT_DISPATCH_TIMEOUT: Duration = Duration::from_millis(2_000);

/// How long `activate` may take before the host gives up on the plugin.
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(10);

/// How long `deactivate` may take before the runtime is dropped anyway.
const DEACTIVATION_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) enum Control {
    Invoke {
        export: String,
        args: serde_json::Value,
        timeout: Duration,
        reply: SyncSender<PluginResult<serde_json::Value>>,
    },
    Deactivate {
        reply: SyncSender<PluginResult<()>>,
    },
}

/// The host's handle on a running plugin.
pub(crate) struct PluginHandle {
    pub(crate) control: tokio::sync::mpsc::UnboundedSender<Control>,
    pub(crate) events: tokio::sync::mpsc::Sender<PluginEventEnvelope>,
    /// Set from any thread; read by the plugin's QuickJS interrupt handler.
    /// The deadline half of that handler lives on the plugin thread, which is
    /// the only place that knows what call is in flight.
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) thread: Option<std::thread::JoinHandle<()>>,
}

impl PluginHandle {
    /// Ask the plugin to stop whatever it is doing, right now.
    pub(crate) fn interrupt(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub(crate) fn clear_interrupt(&self) {
        self.cancel.store(false, Ordering::Relaxed);
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Start a plugin on its own thread and wait for activation to finish.
pub(crate) fn spawn(
    request: ActivationRequest,
    bridge: Arc<dyn HostBridge>,
) -> PluginResult<PluginHandle> {
    let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
    let (events_tx, events_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<PluginResult<()>>(1);

    let cancel = Arc::new(AtomicBool::new(false));
    let deadline = Arc::new(AtomicU64::new(0));

    let thread_cancel = cancel.clone();
    let thread_deadline = deadline.clone();
    let plugin_id = request.plugin_id.clone();
    let thread_name = format!("agora-plugin-{}", plugin_id.as_str());

    let thread = std::thread::Builder::new()
        .name(thread_name)
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .max_blocking_threads(2)
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(PluginError::internal(format!(
                        "could not start a runtime for the plugin: {e}"
                    ))));
                    return;
                }
            };
            rt.block_on(plugin_main(
                request,
                bridge,
                control_rx,
                events_rx,
                thread_cancel,
                thread_deadline,
                ready_tx,
            ));
        })
        .map_err(|e| PluginError::internal(format!("could not start a plugin thread: {e}")))?;

    // `ACTIVATION_TIMEOUT + 1s` so the inner timeout is the one that reports,
    // and this only catches a thread that died without answering.
    match ready_rx.recv_timeout(ACTIVATION_TIMEOUT + Duration::from_secs(1)) {
        Ok(Ok(())) => Ok(PluginHandle {
            control: control_tx,
            events: events_tx,
            cancel,
            thread: Some(thread),
        }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            cancel.store(true, Ordering::Relaxed);
            Err(PluginError::new(
                PluginErrorCode::Timeout,
                "the plugin did not finish activating",
            ))
        }
    }
}

async fn plugin_main(
    request: ActivationRequest,
    bridge: Arc<dyn HostBridge>,
    mut control_rx: tokio::sync::mpsc::UnboundedReceiver<Control>,
    mut events_rx: tokio::sync::mpsc::Receiver<PluginEventEnvelope>,
    cancel: Arc<AtomicBool>,
    deadline: Arc<AtomicU64>,
    ready_tx: SyncSender<PluginResult<()>>,
) {
    let plugin_id = request.plugin_id.clone();

    let context = match build_context(&request, &bridge, &cancel, &deadline).await {
        Ok(context) => context,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };

    let activation = load_and_activate(&context, &request, &deadline).await;
    let activated = activation.is_ok();
    let _ = ready_tx.send(activation);
    if !activated {
        return;
    }

    loop {
        tokio::select! {
            // Control before events: a deactivate should not wait behind a
            // backlog of notifications the plugin is about to stop caring about.
            biased;

            command = control_rx.recv() => {
                match command {
                    None => break,
                    Some(Control::Deactivate { reply }) => {
                        let result = run_optional_export(
                            &context,
                            &deadline,
                            "deactivate",
                            serde_json::Value::Null,
                            DEACTIVATION_TIMEOUT,
                        )
                        .await
                        .map(|_| ());
                        let _ = reply.send(result);
                        break;
                    }
                    Some(Control::Invoke { export, args, timeout, reply }) => {
                        let result = invoke_export(&context, &deadline, &export, args, timeout).await;
                        if let Err(error) = &result {
                            if error.code == PluginErrorCode::ScriptError {
                                bridge.log(&plugin_id, LogLevel::Error, &format!("`{export}` failed: {}", error.message));
                            }
                        }
                        let _ = reply.send(result);
                    }
                }
            }

            envelope = events_rx.recv() => {
                let Some(envelope) = envelope else { continue };
                let json = match serde_json::to_string(&envelope) {
                    Ok(json) => json,
                    Err(_) => continue,
                };
                // A failing handler is the plugin's problem, not the
                // launcher's: log it and keep the subscription alive.
                if let Err(error) = dispatch_event(&context, &deadline, &json).await {
                    bridge.log(
                        &plugin_id,
                        LogLevel::Error,
                        &format!("event handler failed: {}", error.message),
                    );
                }
            }
        }
    }
}

async fn build_context(
    request: &ActivationRequest,
    bridge: &Arc<dyn HostBridge>,
    cancel: &Arc<AtomicBool>,
    deadline: &Arc<AtomicU64>,
) -> PluginResult<AsyncContext> {
    let rt = AsyncRuntime::new()
        .map_err(|e| PluginError::internal(format!("could not create a JS runtime: {e}")))?;
    rt.set_memory_limit(request.memory_limit_bytes).await;
    rt.set_max_stack_size(request.stack_limit_bytes).await;
    rt.set_loader(
        PackageResolver::new(request.package_root.clone()),
        PackageLoader::new(request.package_root.clone(), AGORA_MODULE),
    )
    .await;

    let interrupt_cancel = cancel.clone();
    let interrupt_deadline = deadline.clone();
    rt.set_interrupt_handler(Some(Box::new(move || {
        if interrupt_cancel.load(Ordering::Relaxed) {
            return true;
        }
        let at = interrupt_deadline.load(Ordering::Relaxed);
        at != 0 && now_millis() > at
    })))
    .await;

    // `AsyncContext` holds its runtime by value, so `rt` may go out of scope
    // here: the last context to drop is what actually reclaims the engine.
    let context = AsyncContext::full(&rt)
        .await
        .map_err(|e| PluginError::internal(format!("could not create a JS context: {e}")))?;

    install_host_api(&context, request, bridge.clone()).await?;
    Ok(context)
}

async fn install_host_api(
    context: &AsyncContext,
    request: &ActivationRequest,
    bridge: Arc<dyn HostBridge>,
) -> PluginResult<()> {
    let plugin_id = request.plugin_id.clone();
    let api_version = request.api_version.clone();
    let settings = serde_json::to_string(&request.settings).unwrap_or_else(|_| "{}".into());
    let plugin_id_string = plugin_id.to_string();

    let log_bridge = bridge.clone();
    let log_plugin = plugin_id.clone();

    context
        .async_with(async move |ctx: Ctx<'_>| -> PluginResult<()> {
            let globals = ctx.globals();

            let call_fn = Function::new(
                ctx.clone(),
                Async(
                    move |request_id: u64, method: String, args_json: String, deadline_ms: u64| {
                        let bridge = bridge.clone();
                        let plugin_id = plugin_id.clone();
                        async move {
                            let args: serde_json::Value =
                                serde_json::from_str(&args_json).unwrap_or(serde_json::Value::Null);
                            let mut host_request = HostRequest::new(request_id, method, args);
                            if deadline_ms > 0 {
                                host_request.deadline_ms = deadline_ms;
                            }
                            // The bridge is synchronous core code. Running it
                            // on the blocking pool keeps this thread's event
                            // loop free, which is what lets the timeout below
                            // still fire while a host call is in flight.
                            let response = tokio::task::spawn_blocking(move || {
                                bridge.call(&plugin_id, host_request)
                            })
                            .await
                            .unwrap_or_else(|e| {
                                HostResponse::Error {
                                    request_id,
                                    error: PluginError::internal(format!(
                                        "host call panicked: {e}"
                                    )),
                                }
                            });
                            Ok::<String, rquickjs::Error>(
                                serde_json::to_string(&response).unwrap_or_else(|_| {
                                    r#"{"status":"error","requestId":0,"error":{"code":"INTERNAL","message":"unserialisable response"}}"#.into()
                                }),
                            )
                        }
                    },
                ),
            )
            .map_err(js_internal)?;
            globals.set("__agora_call", call_fn).map_err(js_internal)?;

            let log_fn = Function::new(ctx.clone(), move |level: String, message: String| {
                let level = match level.as_str() {
                    "debug" => LogLevel::Debug,
                    "warn" => LogLevel::Warn,
                    "error" => LogLevel::Error,
                    _ => LogLevel::Info,
                };
                log_bridge.log(&log_plugin, level, &message);
            })
            .map_err(js_internal)?;
            globals.set("__agora_log", log_fn).map_err(js_internal)?;

            globals
                .set("__agora_api_version", api_version)
                .map_err(js_internal)?;
            globals
                .set("__agora_plugin_id", plugin_id_string)
                .map_err(js_internal)?;
            globals
                .set("__agora_settings", settings)
                .map_err(js_internal)?;
            Ok(())
        })
        .await
}

async fn load_and_activate(
    context: &AsyncContext,
    request: &ActivationRequest,
    deadline: &Arc<AtomicU64>,
) -> PluginResult<()> {
    // The entrypoint is read here rather than through the loader so that a
    // missing or unreadable bundle is reported as a package problem, not as a
    // JavaScript resolution failure the author has to decode. Its *name* is
    // still the package-relative path, so `./util.js` beside it resolves.
    let entrypoint = request.entrypoint.replace('\\', "/");
    let entry_path = request.package_root.join(&entrypoint);
    let source = std::fs::read_to_string(&entry_path).map_err(|e| {
        PluginError::new(
            PluginErrorCode::InvalidPackage,
            format!("could not read the entrypoint `{entrypoint}`: {e}"),
        )
    })?;

    set_deadline(deadline, ACTIVATION_TIMEOUT);
    let load = context
        .async_with(async move |ctx: Ctx<'_>| -> PluginResult<()> {
            let (module, promise) = Module::declare(ctx.clone(), entrypoint.as_str(), source)
                .catch(&ctx)
                .map_err(|e| script_error(&e.to_string()))?
                .eval()
                .catch(&ctx)
                .map_err(|e| script_error(&e.to_string()))?;
            // Top-level await and any import side effects settle here.
            promise
                .into_future::<()>()
                .await
                .catch(&ctx)
                .map_err(|e| script_error(&e.to_string()))?;
            let namespace: Object = module.namespace().map_err(js_internal)?;
            ctx.globals()
                .set("__agora_exports", namespace)
                .map_err(js_internal)?;
            Ok(())
        })
        .await;
    clear_deadline(deadline);
    load?;

    run_optional_export(
        context,
        deadline,
        "activate",
        serde_json::Value::Null,
        ACTIVATION_TIMEOUT,
    )
    .await
    .map(|_| ())
}

fn set_deadline(deadline: &Arc<AtomicU64>, timeout: Duration) {
    deadline.store(now_millis() + timeout.as_millis() as u64, Ordering::Relaxed);
}

fn clear_deadline(deadline: &Arc<AtomicU64>) {
    deadline.store(0, Ordering::Relaxed);
}

fn js_internal(e: rquickjs::Error) -> PluginError {
    PluginError::internal(e.to_string())
}

fn script_error(message: &str) -> PluginError {
    if message.contains("interrupted") {
        PluginError::new(
            PluginErrorCode::Timeout,
            "the plugin was interrupted after exceeding its time limit",
        )
    } else if message.contains("out of memory") {
        PluginError::new(
            PluginErrorCode::ResourceExhausted,
            "the plugin exceeded its memory limit",
        )
    } else {
        PluginError::new(PluginErrorCode::ScriptError, message.to_string())
    }
}

/// Call an export that may or may not exist. A missing export is `Ok(Null)`.
async fn run_optional_export(
    context: &AsyncContext,
    deadline: &Arc<AtomicU64>,
    export: &str,
    args: serde_json::Value,
    timeout: Duration,
) -> PluginResult<serde_json::Value> {
    match call_export(context, deadline, export, args, timeout, true).await {
        Ok(value) => Ok(value),
        Err(e) if e.code == PluginErrorCode::NotActivated => Ok(serde_json::Value::Null),
        Err(e) => Err(e),
    }
}

async fn invoke_export(
    context: &AsyncContext,
    deadline: &Arc<AtomicU64>,
    export: &str,
    args: serde_json::Value,
    timeout: Duration,
) -> PluginResult<serde_json::Value> {
    call_export(context, deadline, export, args, timeout, false).await
}

async fn call_export(
    context: &AsyncContext,
    deadline: &Arc<AtomicU64>,
    export: &str,
    args: serde_json::Value,
    timeout: Duration,
    optional: bool,
) -> PluginResult<serde_json::Value> {
    let export_name = export.to_string();
    let args_json = serde_json::to_string(&args).unwrap_or_else(|_| "null".into());
    set_deadline(deadline, timeout);
    let started = Instant::now();

    let work = context.async_with(
        async move |ctx: Ctx<'_>| -> PluginResult<serde_json::Value> {
            let exports: Object = ctx.globals().get("__agora_exports").map_err(|_| {
                PluginError::new(
                    PluginErrorCode::NotActivated,
                    "plugin exports are unavailable",
                )
            })?;
            let function: Value = exports
                .get(export_name.as_str())
                .map_err(|_| missing_export(&export_name, optional))?;
            let Some(function) = function.as_function().cloned() else {
                return Err(missing_export(&export_name, optional));
            };

            // Arguments cross as JSON text so the plugin can never be handed a
            // live host object by accident.
            let parse: Function = ctx.eval::<Function, _>("JSON.parse").map_err(js_internal)?;
            let parsed: Value = parse.call((args_json,)).map_err(js_internal)?;

            let returned: Value = function
                .call((parsed,))
                .catch(&ctx)
                .map_err(|e| script_error(&e.to_string()))?;

            let resolved: Value = if let Some(promise) = returned.as_promise() {
                promise
                    .clone()
                    .into_future::<Value>()
                    .await
                    .catch(&ctx)
                    .map_err(|e| script_error(&e.to_string()))?
            } else {
                returned
            };

            if resolved.is_undefined() || resolved.is_null() {
                return Ok(serde_json::Value::Null);
            }
            let stringify: Function = ctx
                .eval::<Function, _>("JSON.stringify")
                .map_err(js_internal)?;
            let text: Option<String> = stringify
                .call((resolved,))
                .catch(&ctx)
                .map_err(|e| script_error(&e.to_string()))?;
            let Some(text) = text else {
                return Ok(serde_json::Value::Null);
            };
            serde_json::from_str(&text).map_err(|e| {
                PluginError::new(
                    PluginErrorCode::ScriptError,
                    format!("`{export_name}` returned a value Agora could not read: {e}"),
                )
            })
        },
    );

    // The timeout covers awaits; the interrupt handler covers running code.
    // Give the timeout a little slack past the deadline so that, when the
    // plugin is spinning, the interrupt wins and the error says so.
    let result = tokio::time::timeout(timeout + Duration::from_millis(250), work).await;
    clear_deadline(deadline);

    match result {
        Ok(inner) => inner,
        Err(_) => Err(PluginError::new(
            PluginErrorCode::Timeout,
            format!(
                "`{export}` did not finish within {}ms",
                started.elapsed().as_millis()
            ),
        )),
    }
}

fn missing_export(export: &str, optional: bool) -> PluginError {
    if optional {
        PluginError::new(PluginErrorCode::NotActivated, "no such export")
    } else {
        PluginError::new(
            PluginErrorCode::ScriptError,
            format!("the plugin exports no function named `{export}`"),
        )
    }
}

async fn dispatch_event(
    context: &AsyncContext,
    deadline: &Arc<AtomicU64>,
    envelope_json: &str,
) -> PluginResult<()> {
    let json = envelope_json.to_string();
    set_deadline(deadline, EVENT_DISPATCH_TIMEOUT);
    let work = context.async_with(async move |ctx: Ctx<'_>| -> PluginResult<()> {
        let dispatch: Option<Function> = ctx.globals().get("__agora_dispatch_event").ok();
        // A plugin that never imported the SDK has no dispatcher, which simply
        // means it subscribed to nothing.
        let Some(dispatch) = dispatch else {
            return Ok(());
        };
        dispatch
            .call::<_, ()>((json,))
            .catch(&ctx)
            .map_err(|e| script_error(&e.to_string()))
    });
    let result = tokio::time::timeout(EVENT_DISPATCH_TIMEOUT + Duration::from_millis(250), work)
        .await
        .unwrap_or_else(|_| {
            Err(PluginError::new(
                PluginErrorCode::Timeout,
                "an event handler exceeded its time limit",
            ))
        });
    clear_deadline(deadline);
    result
}
