# Layer Ownership & Dependency Boundaries

> **Canonical reference for which code belongs where.** All three frontends (Tauri GUI, CLI, MCP dispatcher) call the same `agora-core` library for business logic. No frontend contains business logic that is not available to the others.

---

## Architecture Overview

Canonical reference for which code belongs where.

## Ownership Rules

### Core and game packages — own everything below

`agora-core` is game-agnostic. Minecraft's behaviour lives in the `agora-game-minecraft` package,
which core never references; it registers into core at startup through `agora_core::game_hooks`
(MASTER_SPEC §26.12). Where a row below names the Minecraft package, a second game supplies its own
equivalent.

| Domain | Owner | Notes |
|---|---|---|
| AppPaths / data layout | `agora-core` | Canonical path derivation for runtimes, instances, cache, receipts |
| Database access (all SQLite) | `agora-core` | Both `registry.db` (the catalog) and `local_state.db`; parameterized queries only |
| Catalogs (runtime, modrinth, etc.) | Signed registry: `agora-core`; Java runtime, loader and Modrinth catalogs: `agora-game-minecraft` | Typed catalog sources; search, version resolution |
| LaunchService (spawn + orchestrate) | `agora-game-minecraft` | Includes process spawning, process identity verification, exit classification, PID tracking. The adapter provides only the raw `std::process::Command` or equivalent handle — core owns the lifecycle |
| InstallService (resolve + stage + apply) | `agora-game-minecraft` | The full install pipeline: `InstallIntent` → `ResolvedInstallPlan` → verified staging → atomic apply → health rollback |
| Dependency resolution | `agora-game-minecraft` (alias and version-range types in `agora-core`) | Required/optional/incompatible resolution; alias matching across sources |
| Import / export / clone | `agora-game-minecraft`; snapshots in `agora-core` | mrpack, Prism, directory import; zip snapshots; instance clone |
| Health / pre-launch checks | `agora-game-minecraft` | JAR metadata parsing, version matching, incompatibility classification |
| Crash diagnostics | `agora-core` | Regex signature matching, scoring algorithm, telemetry recording |
| Authentication (MSA + GitHub OAuth) | MSA: `agora-game-minecraft`; GitHub OAuth and the secret store: `agora-core` | Full device-flow chains; token storage via keyring or encrypted fallback |
| Network policy / security policy | `agora-core` | Host allowlists, redirect validation, hash verification, rate limiting |
| Java runtime operations | `agora-game-minecraft` | Managed runtime catalog, download, extraction, validation, promotion |
| Loader operations | `agora-game-minecraft` | Loader manifest resolution, installer execution, profile adoption |
| MCP dispatcher | `agora-game-minecraft` | Tool routing, argument deserialization, approval policy, system context generation. Adapter provides only transport framing |
| Locks / operation state | `agora-core` | Per-instance mutex, catalog read-writer lock, operation state machine |
| Process identity verification | `agora-core` | PID → executable path → start-time verification; os-identifier abstraction behind a core trait |
| Controller support policy | `agora-game-minecraft` | Whether to offer Controlify for an instance, which loaders it supports, and which instances the user declined. Gamepad *detection* is the Web Gamepad API and belongs to React — core never asks whether a pad is plugged in, only what to do about an instance |
| Content providers | `agora-core` (registry, authorization); `agora-game-minecraft` (Modrinth, Technic, browse, install) | Which providers exist (`providers::ProviderRegistry`) and whether a provider's install plan is permitted (`providers::authorize_plan`) are core's; browsing them alongside the curated catalog and installing a plan are Minecraft's (`agora_game_minecraft::providers`). Modrinth and Technic implement the same `ContentProvider` trait as plugin providers and are registered as compiled-in providers. Adapters build the registry and move data; React renders descriptors and never decides which providers exist |
| Plugin policy | `agora-core` | What is installed, what is enabled, which capabilities were granted, what order plugins activate in, which host method each call maps to, and what happens when a plugin misbehaves. Core does **not** own the script engine — see below |

### Plugin Layer — `agora-plugin-api` / `agora-plugin-host`

Community plugins add two crates that sit *beside* core rather than inside it.

| Crate | Owns | Must NOT own |
|---|---|---|
| `agora-plugin-api` | The public contract: manifest schema and validation, capability set, contribution types, DTOs, diagnostics and repair actions, the host-call protocol, and the `ScriptHost` / `HostBridge` traits | Any policy decision, any service call, any engine. It depends on `serde`, `semver` and `thiserror` and nothing else |
| `agora-plugin-host` | Running plugin JavaScript: QuickJS runtimes, module resolution confined to the package, deadlines, interrupts, memory ceilings, event queues | Anything about instances, mods, launching or the registry. It moves JSON between a plugin and a `HostBridge` |

`agora-core` holds an `Arc<dyn ScriptHost>` that an adapter supplies, exactly the way it holds a
`dyn Clock` or a `dyn EventSink`. **The engine is a mechanism the adapter provides; the policy is
core's.** This is the same trait-in-core rule applied one level out, and it is what keeps
"another runtime later" — a companion process speaking another language — a matter of writing a
new `ScriptHost` rather than reworking plugin policy.

The method table in `agora-core/src/plugins/dispatch.rs` is the **entire** surface a plugin can
reach. Every arm names its required capability next to its implementation, and calls the same
service the GUI and CLI call. A plugin cannot reach a code path the user could not reach
themselves.

### Adapter Layer — Tauri / CLI / MCP transport

| Adapter | Owns | Must NOT own |
|---|---|---|
| Desktop Rust (`desktop/src-tauri/`) | Tauri command registration, IPC event emission, DTO mapping, OS integration (file dialog, system tray, window), platform-specific launcher discovery behind core trait, **supplying the `ScriptHost` implementation** | Loader installation, launch command construction, process classification, MCP tool behavior, dependency resolution, any SQL queries, **any plugin policy** |
| CLI (`crates/agora/`) | Clap argument parsing, stdout/stderr formatting, `Ctrl+C` handling, progress reporter for terminal | Building its own install/launch plans, executing transactions directly, duplicating core domain logic |
| MCP transport (stdio/HTTP) | JSON-RPC framing and parsing, transport-level authorization, forwarding to core dispatcher | Any duplicate business logic, tool-specific validation beyond what the core dispatcher requires |

If a platform-specific primitive is needed (e.g., Windows registry lookup, macOS `CFBundle` detection):
1. Define a **trait in `agora-core`** with the required abstraction.
2. Implement the trait in the adapter.
3. The adapter registers the implementation via dependency injection or a static registry.

This ensures the core owns the **interface and policy** while the adapter provides only the **OS-specific mechanism**.

### Presentation Layer — React

| Concern | Owner | Notes |
|---|---|---|
| UI rendering | React | Components, Tailwind styling, page layout |
| Navigation | React | Tab routing, page transitions, history |
| Transient user-facing state | React | Form inputs, selected items, open dialogs |
| User decisions | React | `onConfirm`/`onCancel` callbacks only — never executes business operations |
| IPC calls to backend | React | `invoke()` calls Tauri commands — no direct SQL, filesystem, or MCP HTTP |
| MCP HTTP from browser | **FORBIDDEN** | React must NOT call `localhost:39741` directly. All MCP operations go through the core dispatcher via Tauri IPC |
| Rendering plugin views | React | A plugin returns a `ViewModel` — data, never markup — and React draws it with Agora's own components. There is deliberately no HTML string to hand to `dangerouslySetInnerHTML` |
| Plugin themes | React | Applies the semantic tokens a theme declares. A theme names a token, never arbitrary CSS |

## Dependency Direction

The game contract is in `agora-game-api`, below core and game packages. Core must not reference
`agora-game-minecraft`; packages register into core through `game_hooks` and the contract. Until
Phase 2, `agora-game-minecraft` may still use `agora-core` within a budget that only shrinks
(`scripts/game_package_core_budget.json`). See [game API and manifest v3](game-api.md).

```
agora-plugin-api  ←  agora-core
agora-plugin-api  ←  agora-plugin-host
agora-core        ←  agora-cli (crates/agora/)
agora-core        ←  agora-desktop (desktop/src-tauri/)
agora-plugin-host ←  agora-desktop (supplies the ScriptHost)
agora-core        ←  MCP dispatcher (future agora serve or desktop adapter)
```

- `agora-core` MUST NOT depend on `tauri`, `clap`, or any MCP-protocol crate.
- `agora-core` MUST NOT depend on `desktop/src-tauri/` or `crates/agora/`.
- `agora-core` MUST NOT depend on `agora-plugin-host` outside `[dev-dependencies]`. Core owns
  plugin policy and reaches the engine only through `dyn ScriptHost`; the dev-dependency exists
  so the end-to-end tests run against the runtime that actually ships.
- `agora-plugin-api` MUST NOT depend on a script engine, a transport, or `agora-core`. It is the
  contract both sides agree on, and adding anything to it is an API change with a version bump.
- Desktop adapter MAY depend on `tauri` and `serde` for command interfaces.
- CLI adapter MAY depend on `clap` and `serde` for CLI interfaces.

## Examples

### ✅ Allowed — Core owns LaunchService including spawn

```rust
// agora-game-minecraft/src/launch_service.rs
pub struct LaunchService {
    planner: LaunchPlanner,
    process_factory: Box<dyn ProcessFactory>,
}

impl LaunchService {
    pub async fn launch(&self, request: LaunchRequest) -> LauncherResult<RunningInstance> {
        let plan = self.planner.resolve(request).await?;
        let mut cmd = self.process_factory.spawn_command(&plan);
        let child = cmd.spawn().map_err(|e| LauncherError::SpawnFailed { .. })?;
        let identity = capture_process_identity(&child)?;
        Ok(RunningInstance { child, identity, plan })
    }
}
```

### ✅ Allowed — Desktop adapter provides platform Command factory

```rust
// desktop/src-tauri/src/process_factory.rs
pub struct TauriProcessFactory;

impl ProcessFactory for TauriProcessFactory {
    fn spawn_command(&self, plan: &MaterializedLaunchPlan) -> std::process::Command {
        let mut cmd = std::process::Command::new(&plan.java_path);
        cmd.args(&plan.jvm_args);
        cmd.args(&plan.game_args);
        // Tauri-specific env clean-up, working dir, etc.
        cmd
    }
}
```

### ✅ Allowed — CLI adapter maps install to core InstallService

```rust
// crates/agora/src/main.rs
"install" => {
    let intent = InstallIntent::from_cli_matches(&matches)?;
    let plan = core::InstallService::resolve_plan(&db, &intent)?;
    if !matches.get_flag("yes") {
        print_plan(&plan);
        if !confirm() { return Ok(()); }
    }
    let outcome = core::InstallService::apply_plan(&db, &instance_dir, plan).await?;
    println!("Installed {} mods", outcome.installed_count());
}
```

### ❌ Forbidden — Desktop adapter constructs launch command or classifies exit

```rust
// BAD: desktop/src-tauri/src/commands.rs
#[tauri::command]
async fn launch_game(instance_id: String, state: ...) -> ... {
    // These belong in agora_core::LaunchService:
    let plan = self_constructed_plan;     // ✗
    let cmd = Command::new("java");       // ✗
    let exit = cmd.wait();                // ✗
    classify_exit(exit);                  // ✗
}
```

### ❌ Forbidden — Desktop adapter owns loader installation logic

```rust
// BAD: desktop/src-tauri/src/instances.rs
pub fn inject_loader(instance_dir: &Path, loader: &str) -> ... {
    // This belongs in core — all three frontends need loader install
    let jar = download_loader_jar(loader);
    run_installer_jar(jar);
}
```

### ❌ Forbidden — CLI constructs its own install plan or transaction

```rust
// BAD: crates/agora/src/main.rs
"install" => {
    let mod = download_mod_from_url(url);  // ✗
    copy_to_mods_dir(mod);                  // ✗
    update_manifest_directly();             // ✗
    // Must go through core::InstallService
}
```

### ❌ Forbidden — React calls MCP HTTP or bypasses core operations

```ts
// BAD: SomeReactComponent.ts
const response = await fetch("http://127.0.0.1:39741/tools/call", { ... });
// Must use invoke() to Tauri backend, which delegates to core dispatcher
```

## Data Flow Patterns

### Read (query, browse, status)

```
React/CLI arg  →  Tauri command / CLI dispatch
                →  agora_core::registry::browse_items(...)
                →  returns data  →  adapter formats response
                →  React renders / CLI prints
```

### Write (install, update, remove) — via InstallService

```
User action  →  adapter builds InstallIntent
              →  core::InstallService::resolve_plan(intent)
              →  ResolvedInstallPlan returned
              →  adapter shows plan (if interactive)
              →  core::InstallService::apply_plan(db, dir, plan)
              →  complete / rollback
```

### Launch — via LaunchService

```
User clicks Play / CLI `launch`
              →  adapter builds LaunchRequest
              →  core::LaunchService::launch(request)
              →  core spawns process (via ProcessFactory trait)
              →  core tracks PID, verifies identity, classifies exit
              →  adapter receives RunningInstance handle
```

### MCP tool call

```
MCP client (e.g., Claude Desktop)
              →  MCP transport (stdio/HTTP)
              →  framing/parsing in adapter
              →  core::mcp_dispatcher::handle_call(tool, args, approval)
              →  response formatted by transport layer
```

## MCP Server Architecture

```
┌──────────┐    JSON-RPC     ┌─────────────────────┐
│  MCP     │ ──────────────► │  Transport Adapter   │
│  Client  │ ◄────────────── │  (stdio / HTTP)      │
└──────────┘                 │  Framing + Auth      │
                             └──────────┬──────────┘
                                        │ call
                             ┌──────────▼──────────┐
                             │  Core MCP Dispatcher │
                             │  (agora-core)         │
                             │  - tool routing       │
                             │  - arg deser          │
                             │  - approval policy    │
                             │  - sys context gen    │
                             │  - delegates to       │
                             │    LaunchService,     │
                             │    InstallService,    │
                             │    health, registry,  │
                             │    crash_diagnostics  │
                             └──────────────────────┘
```

The transport adapter owns only framing (JSON-RPC parse/serialize) and transport-level authorization. The core dispatcher owns all tool behavior, approval policy, and system context generation. This prevents duplicate business logic when adding new transports (e.g., `agora serve` stdio mode later).

## Interactive feature boundary (`desktop/src/features/interactive/`)

A stricter allowlist on top of the rules above, enforced fail-closed by
`desktop/scripts/check-interactive-boundaries.mjs`:

- `domain/`, `visual/` and `lab/` must not import `@tauri-apps/*`, `@/lib/tauri`, `live/`, or
  operation components. `live/` is the only app-boundary layer.
- Within `live/`: the read layer (readAdapters, liveScene, freshness) may call only the
  read-command allowlist; `core` may use Tauri *types* only; `operationBridges/` may host Standard
  controllers but not invoke Tauri. Unclassified files fail.
- Shared visuals are controlled components emitting `VisualIntent` only — no operation-shaped
  callback props (checked via AST, property and method signatures).

Every negative fixture in `desktop/scripts/boundary-fixtures/` must produce a violation:
`node scripts/check-interactive-boundaries.mjs --root scripts/boundary-fixtures/interactive --fixtures`
(from `desktop/`).
