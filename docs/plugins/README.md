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

## The manifest

`agora-plugin.json`, at the root of your folder or ZIP. Here is one with every field, so you do not
have to infer any of them:

```jsonc
{
  // The *schema* version of this file, always 1. Note the field is called
  // `manifest` — it is not called `schema`.
  "manifest": 1,

  // publisher.plugin — lowercase letters, digits and inner hyphens in each half.
  // Not verified by anyone; it is a namespace, not a claim of authorship.
  "id": "acme.census",
  "name": "Mod census",
  "version": "1.0.0",                 // semver, for your plugin
  "license": "MIT",                   // SPDX identifier. Required.
  "description": "Counts things.",    // optional
  "source": "https://github.com/...", // optional, must be https

  // Which plugin API versions you support. Not Agora's version — the two are
  // separate. Pin `>=0.1, <0.2` while the API is experimental.
  "apiRange": ">=0.1, <0.2",

  // Package-relative. No leading "./", no "..", no absolute paths.
  // Bundled JavaScript only: .js or .mjs. TypeScript is compiled by you.
  "entrypoint": "main.js",

  // When your script is loaded. Omit this and it loads at startup. Each entry
  // must name something you actually contribute, or it would never fire.
  //   onStartup | onInstanceOpened | onView:<id> | onCommand:<id> | onEvent:<name>
  // `onView:` covers pages, instance panels and replacements.
  "activation": ["onView:panel", "onCommand:recount"],

  // An object with two arrays — not a bare array.
  // `required` must all be granted or the plugin will not install.
  // `optional` are granted if the user agrees and simply absent if not, so
  // your code must cope without them.
  "capabilities": {
    "required": ["instance:read", "content:read"],
    "optional": ["launch:prepare"]
  },

  // Other plugins that must be installed and enabled, by version range.
  "dependencies": { "other.library": ">=1.2, <2" },

  // Bump when the shape of what you put in `storage` changes. Agora keeps a
  // copy of the old data before loading the new version, which the user can
  // restore if your migration goes wrong.
  "dataVersion": 1,

  // Only meaningful with the `network` capability. Exact hostnames: no
  // wildcards, no IP literals, no ports, no localhost. At most 10.
  "network": { "hosts": ["api.example.com"] },

  "contributions": {
    // A full-width page in the sidebar.
    "pages": [
      { "id": "overview", "title": "Overview", "icon": "LayoutDashboard",
        "view": { "kind": "host", "export": "overview" } }
    ],

    // A panel inside an opened instance. Your export receives { instanceId }.
    "instancePanels": [
      { "id": "panel", "title": "Mod census",
        "view": { "kind": "host", "export": "panel" } }
    ],

    // Command palette, and optionally an instance's context menu. An
    // instance-context command receives { instanceId }.
    "commands": [
      { "id": "recount", "title": "Recount", "export": "recount",
        "surfaces": ["palette", "instance-context"] }
    ],

    // Settings the host renders and validates. An ARRAY, and each entry has a
    // `key` — it is not an object keyed by setting name.
    "settings": [
      { "key": "warn", "title": "Warn before launching", "type": "boolean", "default": true },
      { "key": "label", "title": "Label", "type": "string", "default": "hi", "maxLength": 40 },
      { "key": "limit", "title": "Limit", "type": "number", "default": 5, "min": 1, "max": 10 },
      { "key": "density", "title": "Density", "type": "enum", "default": "compact",
        "options": [{ "value": "compact", "label": "Compact" },
                    { "value": "roomy", "label": "Roomy" }] }
    ],

    // Checks the user runs against an instance. Export returns a
    // DiagnosticReport; receives { instanceId }.
    "diagnostics": [
      { "id": "check", "title": "Check mods", "export": "check" }
    ],

    // Runs before a launch. Same DiagnosticReport shape, same { instanceId }.
    // `timeoutMs` is clamped to 5000 by the host. `onFailure` is "warn"
    // (default — Agora warns, the user decides) or "block".
    "launchChecks": [
      { "id": "preflight", "title": "Preflight", "export": "preflight",
        "timeoutMs": 1500, "onFailure": "warn" }
    ],

    // Offer to render one of Agora's own screens. See below.
    "replacements": [
      { "id": "home", "title": "Compact", "surface": "home",
        "view": { "kind": "host", "export": "home" } }
    ],

    // Colour tokens. Declarative — a theme needs no script at all.
    "theme": { "id": "forest", "title": "Forest", "light": {}, "dark": {} }
  }
}
```

Everything under `contributions` is optional; a theme-only plugin needs no `entrypoint` at all.
`examples/plugins/` has runnable versions of each of these. Where this document and
`crates/agora-plugin-api/src/manifest.rs` disagree, the code is right and this is a bug — please
report it.

### The runtime

Scripts run in QuickJS through `rquickjs 0.13`, with separate bounded runtimes on host-owned worker threads. This is **not an OS process sandbox**. QuickJS supports ES modules, promises, and async functions; the host adds the `agora` module. It does not add `require`, `process`, filesystem access, DOM APIs, native modules, or global `fetch`. Bundle third-party JavaScript and mark `agora` external. Use the standalone declarations in `sdk/` for TypeScript.

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

## What your exports receive, and what they must return

Import the types from `agora` and let `tsc` check this for you — the shapes below are declared in
`sdk/index.d.ts`. Unknown fields are **ignored, not rejected**, so a misspelled one produces a view
that renders with something quietly missing rather than an error.

| Contribution | Receives | Returns |
|---|---|---|
| Page | `null` | `ViewModel` |
| Instance panel | `{ instanceId }` | `ViewModel` |
| Replacement (`home`) | `null` | `ViewModel` |
| Replacement (`instance-overview`) | `{ instanceId }` | `ViewModel` |
| Palette command | `null` | anything serialisable |
| Instance-context command | `{ instanceId }` | anything serialisable |
| Action button | the button's `args`, else the view's args | anything serialisable |
| Diagnostic | `{ instanceId }` | `DiagnosticReport` |
| Launch check | `{ instanceId }` | `DiagnosticReport` |

### A view is a list of blocks

A `ViewModel` is `{ title?, subtitle?, blocks: [] }`. **The `blocks` array is required** — a bare
table is not a view. Each block is one of `heading`, `text`, `stats`, `table`, `list`, `status`,
`actions`, `divider`, discriminated by `type`.

Table rows are arrays of **cells**, in column order — not objects keyed by column. A cell is
`{ type: "text", text }`, `{ type: "badge", text, tone? }` or `{ type: "flag", value }`. Columns
carry a `label` and optional `align`; they have no id.

```js
export async function panel({ instanceId }) {
  // Positional, not an options object — see `sdk/index.d.ts`.
  const items = await content.list(instanceId);
  return {
    title: 'Mod census',
    blocks: [
      { type: 'stats', items: [{ label: 'Installed', value: String(items.length) }] },
      {
        type: 'table',
        columns: [{ label: 'Name' }, { label: 'Enabled' }],
        rows: items.map((item) => [
          { type: 'text', text: item.displayName ?? item.filename },
          { type: 'flag', value: item.enabled },
        ]),
        emptyMessage: 'Nothing installed yet.',
      },
    ],
  };
}
```

At most 200 blocks per view and 500 rows per table. Going over is an error, not a truncation, so a
view that loops while building itself says so rather than freezing the window.

### Values you will be comparing against

Two vocabularies are easy to guess wrong, so they are declared in `sdk/index.d.ts` as `Loader` and
`ContentType` and repeated here:

- **`Instance.loader`** is `vanilla`, `fabric`, `forge`, `neoforge` or `quilt`. `vanilla` is the
  sentinel for "no modloader"; an empty string also occurs on older instances, so test for both.
- **`ContentItem.contentType`**, and `content.list`'s optional filter, is **singular**: `mod`,
  `resourcepack`, `shader`, `datapack`, `world`. Not `mods` — a plural filter matches nothing and
  returns an empty list rather than an error.

Both are typed as a union widened with `string`, so a newer Agora adding a value does not break
your build.

### Diagnostics and launch checks

Both return a `DiagnosticReport`: `{ findings: [], incompleteReason? }`. An empty `findings` array
means "nothing wrong" — that is the success case, not `null`. Set `incompleteReason` when you could
not finish; a partial report with an honest note beats a clean report that silently checked nothing.

A finding needs a stable `id` (so the host can tell "still broken" from "broken again"), a `title`,
and optionally `severity` (`info` | `warning` | `error`, default `info`), a `summary`, `evidence`
label/value pairs, and `repairs`.

```js
export async function preflight({ instanceId }) {
  const items = await content.list(instanceId);
  const enabled = items.filter((item) => item.enabled);
  if (enabled.length > 0) return { findings: [] };
  return {
    findings: [{
      id: 'no-enabled-mods',
      title: 'No mods are enabled',
      severity: 'warning',
      summary: 'This instance has a modloader but every mod is switched off.',
      evidence: [{ label: 'Installed', value: String(items.length) }],
    }],
  };
}
```

Neither needs an activation event. Opening a diagnostic or starting a launch runs your script if it
is not already running, the same way opening a contributed page does. Declare `onView:`/`onCommand:`
for the surfaces that have them and leave `activation` empty otherwise.

A launch check that throws or times out is reported and the launch continues — `onFailure: "warn"`
is the default because Agora warns rather than vetoing. `onFailure: "block"` stops the launch.
`timeoutMs` is clamped to 5000.

Repairs are a **closed set** of actions the host knows how to perform: `disableContent`,
`enableContent`, `pinContentUpdate`, `unpinContentUpdate`, `setJvmMemory`, `resetJvmArgs`,
`createSnapshot`. You propose one; the host re-validates the target and performs it through the same
service the GUI uses. It reports applied, failed and stale actions separately, and a stale result is
not success. Repairs offered together must not contradict each other.

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
