# Community plugins: experimental author guide

The plugin host is opt-in and uses API **0.1.0**. It adds behavior to the existing Minecraft launcher without rebuilding Agora. This is a development feature; the public release acceptance gates are recorded in `implementation-status.md`.

## Try the examples

Build the desktop application, open **Settings → Services → Plugins**, and enable community plugins. Choose **Load a folder** and select one of the directories in `examples/plugins`. Review the requested permissions. Restart Agora to run startup activation. Pages and commands also activate their scripts when first invoked.

- `dashboard`: reads real instances, renders a host-owned table, and saves a count to plugin storage from a button.
- `diagnostics`: lists disabled mods in an instance and proposes enabling one. The instance's integration area shows findings and a repair review; no repair runs until the user approves it. Its launch reminder feeds into the normal launch flow.
- `theme`: adds selectable Forest accents. Choose it in the Plugins page. Built-in appearance preferences remain stored and resume when the plugin is disabled.
- `home-replacement`: offers to render Agora's home screen instead of adding a page. Installing it changes nothing until you choose it under **Who draws each screen**, and Agora's own screen comes back the moment you disable it.

The same operations exist in the standalone CLI, which is usually faster while iterating:

```bash
agora plugin install --yes development ./my-plugin
agora plugin list
agora plugin log author.my-plugin
agora plugin disable-all
```

`agora plugin --help` lists the rest, and `docs/CLI.md` documents consent, JSON output, and the
data-preserving removal default. The CLI and the desktop app drive the same core service, so
they always agree about what is installed.

Every folder contains `agora-plugin.json` at its root. A local ZIP must have that file at the archive root, not inside an extra enclosing folder. Load folders during development; ZIP installation copies files into Agora's package directory. Never distribute secrets in either format. For publishing updates from your own host, and what the signature on them does and does not prove, see [publishing.md](publishing.md).

## Manifest and runtime

Use the examples as complete manifests. Required fields include manifest schema `1`, a publisher/plugin ID such as `author.dashboard`, display name, semver package version, license, and an API range such as `>=0.1, <0.2`. Script plugins name a relative entrypoint. Optional fields include description, source URL, dependencies, data version, activation events, contributions, and network hosts. The authoritative validation is `agora-plugin-api/src/manifest.rs`.

Scripts run in QuickJS through `rquickjs 0.13`, with separate bounded runtimes on host-owned worker threads. This is **not an OS process sandbox**. QuickJS supports ES modules, promises, and async functions; the host adds the `agora` module. It does not add `require`, `process`, filesystem access, DOM APIs, native modules, or global `fetch`. Bundle third-party JavaScript and mark `agora` external. Use the standalone declarations in `sdk/` for TypeScript.

View and command calls have a ten-second host deadline. Diagnostic calls use their host deadline, and launch checks clamp the manifest timeout. Exceptions, timeouts, and allocation limits produce errors. These controls do not establish protection against native engine defects. Windows/MSVC was tested locally; other platform packaging remains an acceptance gate.

## Service API

| SDK | Capability | Behavior |
|---|---|---|
| `instances.list/get` | `instance:read` | Stable instance DTOs |
| `instances.rename/setMemory` | `instance:write` | Existing instance services |
| `content.list` | `content:read` | Installed content DTOs |
| `content.enable/disable/setUpdatePinned` | `content:write` | Existing content services |
| `launch.state/history` | `launch:read` | Read launch information |
| Launch-check contributions | `launch:prepare` | Bounded checks before normal GUI launch preflight |
| `storage.get/set/all/remove/forInstance` | No extra capability | Namespaced plugin data |
| `settings` | No extra capability | Activation-time snapshot of declared settings; restart to refresh |
| `ui.refresh/notify`, `log` | No extra capability | Attributed UI messages and plugin logs |
| `on/off` | Event's read capability | Subscribe/unsubscribe to documented events |
| `net.fetchJson` | `network` | Declared host allowlist, network opt-in, and live lockdown policy |

All service failures reject the promise with a structured error code and message. Treat errors as failures; never display a successful mutation after a rejection. The low-level `call` function exposes the same capability checks, not an escape hatch. There is no shell or arbitrary executable API. Current method definitions are in `agora-core/src/plugins/dispatch.rs`.

## Contributions

Pages and instance panels declare `view: { kind: "host", export: "functionName" }`. The ID and export name need not match. The function returns a serializable view model; the dashboard example shows stats, tables and actions. Action buttons need a stable `id`, label, and export. Commands declare their export and surfaces (`palette`, `instance-context`). Instance actions and panels receive `{ instanceId }`.

Diagnostics return findings with title, severity, evidence and optional typed repairs. Use the diagnostic example as a working contract. The host previews actual actions, rechecks targets, and reports applied, failed and stale actions separately. A stale result is not success. Repairs offered together must not contradict each other.

Theme tokens accept six-digit hex colors. Supported tokens are background, foreground, surface, surface-foreground, primary, primary-foreground, accent, accent-foreground, border, muted, muted-foreground, and destructive. Unknown names and other color syntax are ignored by the renderer. Light and dark token maps apply only to the corresponding mode. Theme CSS never loads remote resources.

## Replacing a built-in screen

A plugin can offer to render one of Agora's own screens instead of adding a new one:

```json
"replacements": [
  {
    "id": "home",
    "title": "Compact",
    "description": "One dense list instead of cards.",
    "surface": "home",
    "view": { "kind": "host", "export": "home" }
  }
]
```

Declaring this does **not** take the screen over. It puts "Compact" on a list in Settings, and
Agora's own screen renders until a user chooses yours. Two plugins offering the same surface is a
list of two, not a race decided by install order, and the user can always go back. Write the
`title` to say what your version *is* rather than repeating the surface name — it appears beside
Agora's own entry.

`surface` is a closed set: `home` and `instance-overview`. A surface joins it when the launcher can
genuinely hand it over; naming one a plugin can declare but never render would be worse than not
offering it. Adding a surface later is additive, so a manifest written today keeps working.

`instance-overview` receives `{ instanceId }`, so one view serves every instance rather than the
plugin having to work out which is open.

Only `kind: "host"` may replace a surface. A replacement is the whole screen, and the host-rendered
path is themed, accessible and controller-navigable by construction. Declare `onView:<id>` with the
replacement's id so your plugin starts when the screen opens.

Expect to be fallen back on. If your plugin is disabled, removed, or fails to start, Agora renders
its own screen and tells the user why; the choice is remembered, so fixing the problem restores it.
`examples/plugins/home-replacement/` is a working example, including how to handle an optional
capability the user declined.

## Custom views were withdrawn

`view: { kind: "custom", html: "..." }` shipped as a prototype in an early 0.1 build and has been
**removed**. A manifest still declaring one is refused with a message pointing here.

It was withdrawn rather than finished because the script inside that frame ran in the WebView,
outside every bound the plugin runtime exists to impose. A QuickJS plugin gets a memory ceiling, an
interrupt handler and a deadline; a 512 KiB HTML document got none of them and could hang the
launcher with `while (true) {}`. A bounded file and a throttled command bridge do not make a
bounded view. Accessibility and controller navigation were also the author's problem inside the
frame, in an application where controller support is first-class — and the frame's isolation could
not be demonstrated for the packaged app, because Tauri documents that on some platforms it cannot
distinguish IPC from an embedded frame from IPC from the window containing it.

Use `{ kind: "host", export: "..." }` and return a view model. If you have an interaction the view
model genuinely cannot express — a graph, a map, a spatial editor — that is a good reason to open
an issue asking for a new block type. It is not a reason to reintroduce a second renderer: the
right fix adds a component every plugin can use, themed and navigable, rather than one plugin's
private HTML.

## Recovery and compatibility

Per-plugin disable removes contributions and subscriptions; uninstall asks separately about retained data. **Turn all off** is the recovery control. The global switch disables all currently enabled plugins when turned off, so re-enable individual plugins afterward. Built-in destinations remain available. Saved plugin destinations show an actionable fallback if unavailable.

API 0.1 is experimental. Pin its range and keep data migrations backward compatible. The implementation does not yet provide complete update rollback, signed package distribution, or a stable support window. Do not use package bytes alone as proof that plugin data can be rolled back. These limitations must be resolved before API v1 is advertised as stable.
