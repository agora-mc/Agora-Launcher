# Layer Ownership & Dependency Boundaries

> **Canonical reference for which code belongs where.** All three frontends (Tauri GUI, CLI, MCP dispatcher) call the same `agora-core` library for business logic. No frontend contains business logic that is not available to the others.

---

## Architecture Overview

Canonical reference for which code belongs where.

## Ownership Rules

### Core Layer (`agora-core`) — owns everything below

| Domain | Owner | Notes |
|---|---|---|
| AppPaths / data layout | `agora-core` | Canonical path derivation for runtimes, instances, cache, receipts |
| Database access (all SQLite) | `agora-core` | Both `registry.db` and `local_state.db`; parameterized queries only |
| Catalogs (runtime, modrinth, etc.) | `agora-core` | Typed catalog sources; search, version resolution |
| LaunchService (spawn + orchestrate) | `agora-core` | Includes process spawning, process identity verification, exit classification, PID tracking. The adapter provides only the raw `std::process::Command` or equivalent handle — core owns the lifecycle |
| InstallService (resolve + stage + apply) | `agora-core` | The full install pipeline: `InstallIntent` → `ResolvedInstallPlan` → verified staging → atomic apply → health rollback |
| Dependency resolution | `agora-core` | Required/optional/incompatible resolution; alias matching across sources |
| Import / export / snapshot / clone | `agora-core` | mrpack, Prism, directory import; zip snapshots; instance clone |
| Health / pre-launch checks | `agora-core` | JAR metadata parsing, version matching, incompatibility classification |
| Crash diagnostics | `agora-core` | Regex signature matching, scoring algorithm, telemetry recording |
| Authentication (MSA + GitHub OAuth) | `agora-core` | Full device-flow chains; token storage via keyring or encrypted fallback |
| Network policy / security policy | `agora-core` | Host allowlists, redirect validation, hash verification, rate limiting |
| Java runtime operations | `agora-core` | Managed runtime catalog, download, extraction, validation, promotion |
| Loader operations | `agora-core` | Loader manifest resolution, installer execution, profile adoption |
| MCP dispatcher | `agora-core` | Tool routing, argument deserialization, approval policy, system context generation. Adapter provides only transport framing |
| Locks / operation state | `agora-core` | Per-instance mutex, registry read-writer lock, operation state machine |
| Process identity verification | `agora-core` | PID → executable path → start-time verification; os-identifier abstraction behind a core trait |

### Adapter Layer — Tauri / CLI / MCP transport

| Adapter | Owns | Must NOT own |
|---|---|---|
| Desktop Rust (`desktop/src-tauri/`) | Tauri command registration, IPC event emission, DTO mapping, OS integration (file dialog, system tray, window), platform-specific launcher discovery behind core trait | Loader installation, launch command construction, process classification, MCP tool behavior, dependency resolution, any SQL queries |
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

## Dependency Direction

```
agora-core  ←  agora-cli (crates/agora/)
agora-core  ←  agora-desktop (desktop/src-tauri/)
agora-core  ←  MCP dispatcher (future agora serve or desktop adapter)
```

- `agora-core` MUST NOT depend on `tauri`, `clap`, or any MCP-protocol crate.
- `agora-core` MUST NOT depend on `desktop/src-tauri/` or `crates/agora/`.
- Desktop adapter MAY depend on `tauri` and `serde` for command interfaces.
- CLI adapter MAY depend on `clap` and `serde` for CLI interfaces.

## Examples

### ✅ Allowed — Core owns LaunchService including spawn

```rust
// agora-core/src/launch_service.rs
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
