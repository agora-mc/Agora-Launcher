//! The `agora` module, as a plugin sees it.
//!
//! This is the *implementation* of the SDK, built into the host. The npm
//! package under `sdk/` ships the matching TypeScript declarations and a dev
//! stub so authors get types and autocomplete, but at runtime the real module
//! is this one — a plugin cannot substitute its own, and cannot reach the two
//! host primitives except through it.
//!
//! Everything here is ordinary JavaScript with no platform assumptions. There
//! is no `require`, no `process`, no `fs`, no `fetch`; QuickJS provides none
//! of them and the host adds none. `agora.net.fetch` exists only when the
//! plugin holds the `network` capability, and it is brokered by the launcher.

/// Source of the builtin `agora` module.
pub const AGORA_MODULE: &str = r#"
let nextRequestId = 1;

function call(method, args, options) {
  const requestId = nextRequestId++;
  const deadlineMs = (options && options.deadlineMs) || 0;
  return __agora_call(requestId, method, JSON.stringify(args === undefined ? {} : args), deadlineMs)
    .then((raw) => {
      const response = JSON.parse(raw);
      if (response.status === "error") {
        const error = new Error(response.error.message);
        error.code = response.error.code;
        error.detail = response.error.detail;
        throw error;
      }
      return response.value;
    });
}

const log = {
  debug: (...parts) => __agora_log("debug", parts.map(stringify).join(" ")),
  info: (...parts) => __agora_log("info", parts.map(stringify).join(" ")),
  warn: (...parts) => __agora_log("warn", parts.map(stringify).join(" ")),
  error: (...parts) => __agora_log("error", parts.map(stringify).join(" ")),
};

function stringify(value) {
  if (typeof value === "string") return value;
  if (value instanceof Error) return value.stack || value.message;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

const instances = {
  list: () => call("instance.list"),
  get: (instanceId) => call("instance.get", { instanceId }),
  rename: (instanceId, name) => call("instance.rename", { instanceId, name }),
  setMemory: (instanceId, memoryMb) => call("instance.setMemory", { instanceId, memoryMb }),
};

const content = {
  list: (instanceId, contentType) => call("content.list", { instanceId, contentType }),
  enable: (instanceId, key) => call("content.enable", { instanceId, key }),
  disable: (instanceId, key) => call("content.disable", { instanceId, key }),
  setUpdatePinned: (instanceId, key, pinned) =>
    call("content.setUpdatePinned", { instanceId, key, pinned }),
};

const launch = {
  state: (instanceId) => call("launch.state", { instanceId }),
  history: (instanceId, limit) => call("launch.history", { instanceId, limit }),
};

const storage = {
  get: (key) => call("storage.get", { key }),
  set: (key, value) => call("storage.set", { key, value }),
  all: () => call("storage.all"),
  remove: (key) => call("storage.remove", { key }),
  forInstance: (instanceId) => ({
    get: (key) => call("storage.get", { key, instanceId }),
    set: (key, value) => call("storage.set", { key, value, instanceId }),
    all: () => call("storage.all", { instanceId }),
    remove: (key) => call("storage.remove", { key, instanceId }),
  }),
};

const net = {
  fetchJson: (url, options) => call("net.fetchJson", { url, ...(options || {}) }, { deadlineMs: 30000 }),
};

const ui = {
  refresh: (viewId) => call("ui.refresh", { viewId }),
  notify: (tone, message) => call("ui.notify", { tone, message }),
};

// --- events ---------------------------------------------------------------

const handlers = new Map();

function on(eventName, handler) {
  if (typeof handler !== "function") throw new TypeError("handler must be a function");
  let list = handlers.get(eventName);
  if (!list) {
    list = [];
    handlers.set(eventName, list);
    call("events.subscribe", { event: eventName }).catch((e) =>
      log.error(`could not subscribe to ${eventName}:`, e),
    );
  }
  list.push(handler);
  return () => off(eventName, handler);
}

function off(eventName, handler) {
  const list = handlers.get(eventName);
  if (!list) return;
  const index = list.indexOf(handler);
  if (index >= 0) list.splice(index, 1);
  if (list.length === 0) {
    handlers.delete(eventName);
    call("events.unsubscribe", { event: eventName }).catch(() => {});
  }
}

// Called by the host. A throwing handler is logged and the remaining handlers
// still run: one bad subscriber must not silence the others.
globalThis.__agora_dispatch_event = (json) => {
  const envelope = JSON.parse(json);
  const list = handlers.get(envelope.event);
  if (!list) return;
  for (const handler of list.slice()) {
    try {
      const result = handler(envelope);
      if (result && typeof result.catch === "function") {
        result.catch((e) => log.error(`handler for ${envelope.event} failed:`, e));
      }
    } catch (e) {
      log.error(`handler for ${envelope.event} failed:`, e);
    }
  }
};

const apiVersion = __agora_api_version;
const pluginId = __agora_plugin_id;
const settings = JSON.parse(__agora_settings);

export { call, log, instances, content, launch, storage, net, ui, on, off, apiVersion, pluginId, settings };
export default { call, log, instances, content, launch, storage, net, ui, on, off, apiVersion, pluginId, settings };
"#;

/// Name plugins import: `import { instances } from "agora"`.
pub const AGORA_MODULE_NAME: &str = "agora";
