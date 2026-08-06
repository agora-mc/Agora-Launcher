# Agora Launcher Architecture, Direct Launch, and CLI Parity Plan

> Archived 2026-08-05. Status: mandatory architecture, direct-launch, CLI parity, and stdio MCP scope completed. This is a historical implementation plan, not current product documentation. Optional daemon commands, generated TypeScript bindings, and further HTTP MCP hardening remain future work. Use [`docs/CLI.md`](../CLI.md), [`layer-ownership.md`](../architecture/layer-ownership.md), current source, and tests for live behavior.

## 1. Objective

Refactor Agora so that:

1. `agora-core` owns all reusable launcher behavior.
2. Desktop Rust is a thin Tauri and operating-system adapter.
3. TypeScript owns presentation, user interaction, and transient UI state.
4. The CLI calls the same core services as the desktop.
5. MCP tools call those same services rather than maintaining a third implementation.
6. Direct launch is reliable for:

   * Vanilla
   * Fabric
   * Quilt
   * Forge
   * NeoForge
   * Newly required Java versions such as Java 25
7. Signed loader and Java catalogs can update independently of the application executable.
8. Desktop and CLI share the same data, behavior, validation, and transaction semantics.
9. No frontend is able to bypass dependency resolution, integrity checks, snapshots, health checks, or rollback.
10. There is only one authoritative implementation for each launcher operation.

The final dependency direction must be:

```text
TypeScript / React
        ↓ Tauri IPC
Desktop Rust adapter
        ↓ direct Rust calls
agora-core services
        ↓
filesystem, SQLite, processes, keyring, and external HTTPS
```

For external AI clients:

```text
MCP HTTP or stdio transport
        ↓
core MCP dispatcher
        ↓
the same agora-core services
```

The normal desktop frontend must not communicate with the backend through local HTTP. It should continue using Tauri IPC.

---

# 2. Non-negotiable ownership rules

## 2.1 TypeScript owns

TypeScript and React should own:

* Rendering
* Navigation
* Form state
* Dialog visibility
* User selections
* Search-filter state
* UI-only sorting
* UI-only pagination state
* Transient progress presentation
* Bounded display log buffers
* Toasts and error presentation
* Asking the user to approve warnings
* Choosing among backend-provided recovery actions
* Native-looking visual experiences
* Accessibility behavior
* Theme and layout behavior

TypeScript may decide:

> “The user selected Download Java and Retry.”

It should not independently implement:

```text
download Java
→ update instance configuration
→ retry launch
→ classify failure
→ update persistent state
```

That sequence must be one core-owned operation.

## 2.2 Desktop Rust owns

Desktop Rust should own only things that require Tauri or direct frontend integration:

* `#[tauri::command]` wrappers
* Constructing core context from `AppHandle`
* Emitting Tauri events or channels
* Receiving frontend command parameters
* Mapping frontend DTOs into core request types
* Returning core results to the frontend
* Native file and folder pickers
* Opening URLs through the operating system
* Window lifecycle
* Tauri updater integration
* Tauri plugin integration
* Windows accent-color integration
* Desktop notifications
* Application-close callbacks
* Browser-window or WebView-specific behavior

Desktop Rust must not own:

* Loader installation
* Instance transactions
* Registry queries
* Modrinth behavior
* Dependency resolution
* Conflict resolution
* Snapshot policy
* Pack import transactions
* Java selection
* Launch command construction
* Minecraft process classification
* MCP tool behavior
* Authentication workflows other than opening or presenting OS UI
* Business validation

## 2.3 `agora-core` owns

Core must own:

* Paths and data-directory layout
* Database creation and migrations
* Registry access
* Registry synchronization
* Dynamic runtime and loader catalogs
* Instance creation, mutation, and deletion
* Loader installation and repair
* Java discovery, validation, and provisioning
* Direct launch
* Delegated-launch preparation
* Process spawning and identity verification
* Process exit classification
* LKG promotion
* Snapshot creation and restoration
* Health scanning
* Crash analysis
* Install-plan resolution
* Dependency resolution
* Conflict detection
* Reverse-dependency detection
* Install-plan execution and rollback
* Modrinth networking
* Mojang networking
* Microsoft authentication
* GitHub authentication logic
* Import and export
* Pack installation
* Lockfiles
* Loadouts
* Settings
* MCP tool behavior and approval policy
* External HTTP communication
* Security policies
* URL validation
* Hash validation
* Cross-process locking
* Cancellation and operation tracking

Core must not depend on:

* `tauri`
* `clap`
* React
* TypeScript
* An MCP transport library unless isolated behind a transport-specific crate or feature
* WebView-specific types

---

# 3. Agent operating rules

The agent must follow these rules throughout implementation.

## 3.1 Do not duplicate behavior

When moving logic into core:

1. Move or rewrite it in core.
2. Change desktop Rust to call core.
3. Change CLI to call core.
4. Change MCP to call core.
5. Delete the old duplicate implementations.

Do not leave old and new paths active simultaneously except during a tightly bounded migration commit.

## 3.2 Do not create frontend-specific forks

Do not introduce APIs such as:

```rust
create_instance_for_desktop(...)
create_instance_for_cli(...)
create_instance_for_mcp(...)
```

Create one service:

```rust
InstanceService::create(...)
```

Use adapters for progress, events, prompts, and host-specific behavior.

## 3.3 Do not use panics for user-controlled conditions

Remove or avoid:

* `unwrap()` on user-controlled input
* `expect()` on data files
* `panic!()` when no compatible version exists
* panic-prone nested Tokio runtimes
* one-shot globals that panic when initialized twice

All expected failures must return structured errors.

## 3.4 Preserve exact Java-major behavior

Minecraft metadata is authoritative.

If metadata requires Java 25:

* Java 21 must not be silently substituted.
* Java 26 must not be silently substituted.
* An explicit user override may bypass compatibility only when the user has enabled the incompatible override.
* The error must name the required major.

## 3.5 Preserve security boundaries

Do not weaken:

* HTTPS requirements
* Domain allowlists
* Redirect validation
* Path traversal checks
* Archive extraction checks
* Artifact size limits
* Metadata-provided hashes
* Loader profile or installer pins
* Java runtime hashes
* Forge-generated artifact receipt hashes
* Atomic file writes
* Snapshot and rollback behavior
* Authentication-token redaction
* Process identity verification

The removed transitive-library deep audit must not be accidentally restored as a launch requirement.

---

# 4. Phase 0: Establish a verified baseline

## 4.1 Create a dedicated branch

Create a branch such as:

```text
architecture/core-services-cli-parity
```

Do not perform the complete migration in one commit.

Recommended commit groups:

1. Shared context and paths
2. Catalog lifecycle
3. Launch service
4. Instance and loader service
5. Install resolver service
6. Import service
7. CLI parity
8. MCP transport separation
9. Desktop cleanup
10. Tests and CI enforcement

## 4.2 Run and record the current baseline

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p agora-core --lib
cargo test -p agora-core --test launch_planner_integration
cargo test -p agora-cli
cargo build -p agora-cli
cargo check -p agora-desktop
```

Also run the Python generator tests and the frontend tests currently used in CI.

Record:

* Existing failures
* Existing warnings
* Platform-specific compilation issues
* Tests that are missing
* Current CLI help output
* Current default data directories

Do not silently classify preexisting failures as regressions introduced by the migration.

## 4.3 Add an architecture document

Add a repository document such as:

```text
docs/architecture/layer-ownership.md
```

It should state the ownership rules from this plan and include examples of allowed and forbidden dependencies.

---

# 5. Phase 1: Build the shared core application context

The current `Ctx` is too small. Expand it into the foundation for every frontend.

## 5.1 Introduce `AppPaths`

Create a core-owned path model:

```rust
pub struct AppPaths {
    pub app_data: PathBuf,
    pub local_state_db: PathBuf,
    pub registry_db: PathBuf,
    pub registry_signature: PathBuf,
    pub instances: PathBuf,
    pub minecraft_runtime: PathBuf,
    pub loader_cache: PathBuf,
    pub loader_receipts: PathBuf,
    pub java_runtimes: PathBuf,
    pub snapshots: PathBuf,
    pub operation_state: PathBuf,
    pub locks: PathBuf,
}
```

Construct all paths from one app-data root.

No frontend should manually reproduce:

```text
app_data.join("minecraft-runtime")
app_data.join("loader_cache")
app_data.join("instances")
```

Use typed helper methods where a path requires an identifier:

```rust
impl AppPaths {
    pub fn instance_dir(&self, instance_id: &str) -> LauncherResult<PathBuf>;
    pub fn instance_manifest(&self, instance_id: &str) -> LauncherResult<PathBuf>;
    pub fn runtime_lock(&self, major: u32) -> PathBuf;
    pub fn instance_lock(&self, instance_id: &str) -> LauncherResult<PathBuf>;
}
```

All identifier sanitization must happen in core.

## 5.2 Expand `Ctx`

Recommended structure:

```rust
pub struct Ctx {
    pub paths: AppPaths,
    pub clients: HttpClients,
    pub clock: Arc<dyn Clock>,
    pub locks: LockManager,
}
```

A test context must be constructible with:

* Temporary directories
* Fake HTTP clients or mock-server configuration
* Test clock
* Test lock directory

Do not place Tauri types in `Ctx`.

## 5.3 Create category-aware HTTP clients

Consolidate duplicate `reqwest::Client` construction.

Use a shared type:

```rust
pub struct HttpClients {
    pub mojang_metadata: reqwest::Client,
    pub mojang_content: reqwest::Client,
    pub loader_content: reqwest::Client,
    pub registry: reqwest::Client,
    pub modrinth: reqwest::Client,
    pub microsoft: reqwest::Client,
    pub github: reqwest::Client,
}
```

Each client or request wrapper must enforce the relevant:

* HTTPS-only rule
* Host allowlist
* Redirect limit
* Local-network prohibition
* Timeout
* Size limit
* User agent

Avoid creating plain, unconfigured clients inside individual modules.

## 5.4 Add progress and event abstractions

Create UI-neutral interfaces:

```rust
pub trait ProgressSink: Send + Sync {
    fn emit(&self, event: ProgressEvent);
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: CoreEvent);
}
```

Provide:

* `NoopProgressSink`
* `CollectingProgressSink` for tests
* Desktop/Tauri adapter
* CLI human-output adapter
* CLI JSON/NDJSON adapter
* MCP adapter where needed

Events should be strongly typed and serializable.

## 5.5 Add cross-process locking

The existing process-local mutexes are insufficient once desktop and CLI share data.

Introduce locks for:

```text
locks/registry-update.lock
locks/loader-install.lock
locks/java-runtime-<major>.lock
instances/<id>/.agora/instance.lock
locks/minecraft-runtime-materialization.lock
```

Use an OS-backed or filesystem lock library.

Requirements:

* Same-process serialization remains efficient.
* Separate Agora processes cannot concurrently mutate the same shared resources.
* Lock acquisition supports cancellation or a timeout.
* Error messages identify the conflicting operation.
* Locks are released on normal errors and panics through RAII.

---

# 6. Phase 2: Unify app-data and startup initialization

## 6.1 Use the same default data directory

Desktop and CLI must resolve to the same platform-specific Agora directory.

Do not use:

```text
CLI → data_local_dir()/agora
Desktop → Tauri com.agoramc.app app-data
```

Create one platform resolver in core.

Recommended priority for CLI:

1. Explicit `--data-dir`
2. Explicit `AGORA_DATA_DIR`
3. Core platform default matching desktop
4. Error when a valid platform location cannot be resolved

Add:

```bash
agora paths
```

It should print all resolved paths in human-readable or JSON form.

## 6.2 Centralize database initialization

Every host must call:

```rust
CoreRuntime::initialize()
```

This should:

1. Create required directories.
2. Initialize or migrate `local_state.db`.
3. Validate the cached registry.
4. Load the signed runtime catalog.
5. Load the signed loader catalog.
6. Clean safe stale staging directories.
7. Validate lock-directory permissions.
8. Return warnings instead of silently ignoring recoverable issues.

Desktop setup, CLI startup, and MCP standalone startup must call the same method.

## 6.3 Make registry repository configuration runtime-safe

Do not require the CLI user to set `AGORA_REGISTRY_REPO` at compile time.

Use one of:

* A built-in official repository identifier
* A signed runtime configuration
* A runtime environment override for development
* A CLI option for test registries

Recommended priority:

```text
--registry-repo
AGORA_REGISTRY_REPO
built-in official repository
```

Do not allow an unsigned local registry in release builds unless explicitly running a development command.

---

# 7. Phase 3: Fix dynamic catalog lifecycle

Both Java runtime and loader catalogs must be updateable through the signed registry and remain active after restart.

## 7.1 Replace one-shot loader-catalog state

Remove a `OnceLock` design that:

* Can only be initialized once
* Panics on replacement
* Loses registry-only data after restart

Prefer either:

### Explicit service-owned catalog

```rust
pub struct CatalogService {
    loader_catalog: ArcSwap<LoaderCatalog>,
    runtime_catalog: ArcSwap<RuntimeCatalog>,
}
```

or:

```rust
RwLock<Arc<LoaderCatalog>>
RwLock<Arc<RuntimeCatalog>>
```

Explicitly passing the catalog through services is preferable to hidden global state.

## 7.2 Load catalogs at startup

During core initialization:

1. Open and verify cached `registry.db`.
2. Load `loader_catalog` from it.
3. Load `runtime_catalog` from it.
4. Validate catalog schema.
5. Use the embedded catalog only when the signed registry lacks that catalog or is unavailable.
6. Record which source is active:

   * Signed registry
   * Embedded fallback

Expose this in:

```bash
agora registry status
```

## 7.3 Reload after registry update

After an atomic registry update:

1. Open the new database.
2. Validate it.
3. Parse both catalogs.
4. Replace the active catalog state atomically.
5. Keep the previous active catalogs if parsing fails.
6. Notify desktop and daemon hosts that catalog data changed.

Do not require process restart.

## 7.4 Retry missing Java or loader entries once

When a requested entry is absent:

1. Check the active catalog.
2. If registry sync is allowed, force one update check.
3. Reload catalogs.
4. Retry lookup once.
5. Return a structured error if still unavailable.

Never retry indefinitely.

## 7.5 Keep loader refresh lightweight

The loader refresh system should pin:

* Fabric/Quilt profile JSON
* Forge/NeoForge installer JAR
* Forge/NeoForge embedded version metadata where applicable

Do not require a deep SHA-256 audit of every transitive loader dependency.

Ordinary dependency verification order:

1. Metadata SHA-256
2. Metadata SHA-1
3. Approved profile-declared HTTPS source
4. Reject unapproved or missing source

## 7.6 Generate Java majors dynamically

The Java catalog generator should:

1. Keep baseline majors:

   * 8
   * 16
   * 17
   * 21
2. Discover required majors from supported Mojang metadata.
3. Include newly required majors such as 25.
4. Query approved runtime vendors.
5. Record exact URL, archive type, size, SHA-256, Java major, OS, architecture, and executable path expectations.
6. Validate changed entries before publication.
7. Include the catalog in signed `registry.db`.

Keep embedded coverage for common baseline majors as an offline fallback.

---

# 8. Phase 4: Finish direct-launch reliability

## 8.1 Create a core `LaunchService`

Move the full launch orchestration into core.

Recommended interface:

```rust
pub struct LaunchRequest {
    pub instance_id: String,
    pub mode: LaunchMode,
    pub health_policy: HealthPolicy,
}

pub enum LaunchMode {
    Direct,
    Delegated,
}

pub struct LaunchStarted {
    pub pid: u32,
    pub session_id: u64,
}

pub struct LaunchExecution {
    pub started: LaunchStarted,
    pub outcome: Option<LaunchOutcome>,
}
```

Core should perform:

1. Validate instance ID.
2. Acquire the instance launch lock.
3. Load `InstanceRow`.
4. Load and validate `InstanceManifest`.
5. Load global and instance settings.
6. Run health checks.
7. Authenticate MSA where direct launch requires it.
8. Resolve the exact Java requirement.
9. Apply per-instance Java override.
10. Apply global Java override.
11. Respect incompatible-override settings.
12. Discover managed, Mojang, and system Java.
13. Provision exact Java when permitted.
14. Refresh signed catalog once when necessary.
15. Adopt installed loader profile for every non-vanilla loader.
16. Materialize client, libraries, natives, assets, and logging files.
17. Validate all required files.
18. Construct JVM arguments.
19. Apply configured memory.
20. Apply configured GC policy.
21. Apply custom JVM arguments.
22. Create launch snapshot.
23. Spawn Java.
24. Capture OS process identity.
25. Emit `ProcessStarted`.
26. Stream logs.
27. Classify exit.
28. Update LKG.
29. Update `last_launched_at`.
30. Emit `ProcessExited`.
31. Release locks and reservations.

No frontend should recreate this sequence.

## 8.2 Fix dynamic loader-catalog persistence

Acceptance test:

1. Start with an embedded catalog lacking a test loader version.
2. Install a signed registry containing it.
3. Create or repair an instance using that version.
4. Restart the application.
5. Launch the same instance.
6. Confirm the registry-provided loader remains available.

## 8.3 Verify locally observed hashes for hashless libraries

For an ordinary dependency without a metadata hash:

1. Download only from an approved HTTPS repository.
2. Compute SHA-256.
3. Store the observed hash in a local sidecar or cache record.
4. Atomically commit the artifact and observation.
5. On subsequent cache use, verify the artifact against the observation.
6. On mismatch:

   * Delete or quarantine it.
   * Redownload when network policy permits.
   * Otherwise return a structured corruption error.

Never accept a hashless cached file merely because it exists.

Do not mistake the locally observed hash for an independently curated trust root.

## 8.4 Keep generated Forge artifacts receipt-bound

Forge/NeoForge processor-generated files must:

* Have a receipt SHA-256
* Be adopted only from the validated installed profile
* Never receive a guessed network fallback
* Cause profile repair to be suggested when missing or corrupt

## 8.5 Fix fast-exit frontend races

Every launch must have a monotonically increasing `session_id`.

Include `session_id` in:

* Launch-start response
* Log events
* Exit events
* Kill requests
* Query-state responses

The desktop controller must match events by:

```text
instance_id + session_id
```

not only instance ID.

Backend state remains authoritative.

The frontend must be able to receive an exit event while launch is still in:

* `checking-health`
* `launching`
* `running`
* `stopping`
* `delegated`

A process that exits immediately must never leave the UI showing “running.”

## 8.6 Use Tauri channels for high-volume output

Use channels for:

* Game stdout/stderr
* Loader installer output
* Java download progress where frequent
* Long import progress
* AI streaming

Use ordinary events for:

* Game started
* Game exited
* Registry changed
* Authentication completed
* Operation completed

## 8.7 Direct-launch acceptance matrix

Test clean-machine behavior without an official `.minecraft` directory:

| Loader   | First online launch | Second offline launch |
| -------- | ------------------: | --------------------: |
| Vanilla  |            Required |              Required |
| Fabric   |            Required |              Required |
| Quilt    |            Required |              Required |
| Forge    |            Required |              Required |
| NeoForge |            Required |              Required |

Also test:

* Minecraft 26.2 with Java 25
* Runtime archive with nested Java executable path
* Managed Java tampering
* Loader profile tampering
* Missing generated Forge file
* Corrupt asset
* Corrupt hashless dependency
* Network disabled on first launch
* Network disabled after a successful first launch
* Immediate Java exit
* User-requested stop
* Unknown nonzero exit
* Launcher restart while game is running
* Paths containing spaces and Unicode

---

# 9. Phase 5: Move instance and loader ownership into core

## 9.1 Create `InstanceService`

Recommended operations:

```rust
impl InstanceService {
    pub async fn list(&self) -> LauncherResult<Vec<InstanceRow>>;
    pub async fn get(&self, id: &str) -> LauncherResult<InstanceDetail>;
    pub async fn create(
        &self,
        request: CreateInstanceRequest,
        progress: &dyn ProgressSink,
        cancel: &CancellationToken,
    ) -> LauncherResult<InstanceRow>;
    pub async fn delete(...);
    pub async fn rename(...);
    pub async fn clone(...);
    pub async fn lock(...);
    pub async fn unlock(...);
    pub async fn repair_loader(...);
}
```

## 9.2 Move loader installation out of desktop Rust

Move into core:

* Loader manifest lookup
* Loader cache path resolution
* Loader download
* Hash validation
* Fabric/Quilt profile writing
* Receipt creation
* Forge/NeoForge installer staging
* Installer Java resolution
* Installer process execution
* Installer timeout
* Output truncation
* Backup and rollback
* Receipt validation
* Cache-hit adoption
* Force-reinstall behavior

Replace Tauri progress emission with `ProgressSink`.

## 9.3 Make instance creation transactional

Required sequence:

1. Validate request.
2. Acquire instance and loader locks.
3. Create temporary instance staging directory.
4. Write a valid manifest.
5. Initialize runtime layout.
6. Bootstrap Mojang metadata.
7. Install loader when non-vanilla.
8. Validate loader adoption.
9. Persist database row.
10. Update launcher profile if still required.
11. Atomically promote instance directory.
12. Release locks.
13. Emit completion.

On failure:

* Remove temporary instance state.
* Do not remove valid shared runtime or loader files.
* Roll back partially persisted database state.
* Return the original structured error.

## 9.4 Make repair use the same loader service

`repair-loader` must call:

```rust
LoaderService::ensure_installed(..., force_reinstall = true)
```

It must not have a second repair-specific implementation.

---

# 10. Phase 6: Move install-plan preparation into core

The core currently owns plan normalization and execution, but desktop Rust owns important preparation behavior. Move the full process into a core `InstallService`.

## 10.1 Required responsibilities

Core must resolve:

* Curated registry item versions
* Raw Modrinth versions
* Manual local files
* Required dependencies
* Optional dependencies
* Existing reusable dependencies
* Version conflicts
* Duplicate mods
* Loader mismatches
* Minecraft-version mismatches
* Incompatibilities
* Filename collisions
* Reverse dependencies
* Batch installs
* Batch updates
* Removal impact
* Lockfile repair
* Registry revision
* Instance state fingerprint

## 10.2 Recommended service interface

```rust
impl InstallService {
    pub async fn resolve(
        &self,
        intent: InstallIntent,
        progress: &dyn ProgressSink,
        cancel: &CancellationToken,
    ) -> LauncherResult<ResolvedInstallPlan>;

    pub async fn execute(
        &self,
        plan_id: &str,
        progress: &dyn ProgressSink,
        cancel: &CancellationToken,
    ) -> LauncherResult<InstallOutcome>;
}
```

For long-lived hosts, core may maintain an operation store.

For a one-shot CLI, provide:

```rust
resolve_and_execute(...)
```

or permit execution of an internally verified plan in the same process.

Do not let clients execute arbitrary plan bodies.

## 10.3 Preserve review integrity

Desktop workflow:

```text
resolve
→ display plan
→ user decides
→ apply by opaque plan ID
```

CLI interactive workflow:

```text
resolve
→ print plan
→ prompt
→ execute backend-owned plan
```

CLI noninteractive workflow:

```text
resolve
→ fail on unresolved choices unless flags fully specify them
→ execute
```

## 10.4 Add explicit terminal policies

CLI flags should map to core policies:

```text
--include-optional <id,id>
--exclude-optional
--replace-conflicts
--abort-conflicts
--skip-health-scan
--allow-replace
--dry-run
--yes
```

Do not make CLI commands silently choose unsafe conflict resolutions.

## 10.5 Remove current CLI shortcuts

Delete CLI behavior that creates plans with:

```text
dependencies = []
conflicts = []
reverse_dependents = []
skip_health_scan = true
```

unless the user explicitly selected the corresponding policy and core verified it was permissible.

---

# 11. Phase 7: Make import and pack installation transactional

## 11.1 Create `ImportService`

Supported sources:

* `.mrpack`
* Prism instance
* Local Agora export
* Modrinth pack URL
* Curated pack
* Lockfile-based reconstruction where supported

## 11.2 Import sequence

1. Validate source.
2. Determine pack format.
3. Parse metadata asynchronously.
4. Validate Minecraft and loader versions.
5. Resolve destination ID.
6. Acquire destination lock.
7. Extract into a staging directory.
8. Validate all archive paths.
9. Download remote files asynchronously.
10. Verify available hashes.
11. Construct a valid `InstanceManifest`.
12. Populate all required arrays:

    * Mods
    * Resource packs
    * Shaders
    * Data packs
    * Worlds
13. Bootstrap runtime metadata.
14. Install the loader.
15. Register the database row.
16. Run health validation.
17. Atomically promote the instance.
18. Create initial snapshot.
19. Return complete import result.

On any failure:

* Delete staging.
* Roll back database state.
* Keep valid shared runtime artifacts.
* Never leave an unregistered filesystem-only instance.

## 11.3 Remove nested Tokio runtime usage

Do not create a Tokio runtime and call `block_on()` from within an existing Tokio runtime.

Make remote import operations genuinely async.

Use `spawn_blocking` only for CPU-heavy or synchronous archive operations.

## 11.4 Validate imported manifests

Add schema-level tests proving every import source produces a manifest that deserializes into the canonical `InstanceManifest`.

---

# 12. Phase 8: Achieve practical CLI parity

The CLI should become a real frontend to core rather than a partial alternate implementation.

## 12.1 Recommended command hierarchy

```text
agora paths

agora registry status
agora registry sync

agora settings list
agora settings get <key>
agora settings set <key> <value>

agora instance list
agora instance show <id>
agora instance create
agora instance delete <id>
agora instance rename <id>
agora instance clone <id>
agora instance lock <id>
agora instance unlock <id>
agora instance repair-loader <id>

agora loader list
agora loader list --minecraft <version>

agora mod search <query>
agora mod list <instance>
agora mod install <instance> <item>
agora mod update <instance> <item>
agora mod update-all <instance>
agora mod remove <instance> <item-or-file>
agora mod enable <instance> <file>
agora mod disable <instance> <file>

agora pack install <pack>
agora import <path-or-url>
agora export <instance>

agora health <instance>

agora launch <instance>
agora launch <instance> --detach
agora status
agora stop <instance-or-session>

agora snapshot list <instance>
agora snapshot create <instance>
agora snapshot restore <instance> <snapshot>
agora snapshot delete <instance> <snapshot>

agora loadout ...
agora lockfile ...

agora runtime list
agora runtime ensure <major>
agora runtime remove-unused
agora runtime inspect <path>

agora auth status
agora auth login
agora auth logout

agora crash list <instance>
agora crash inspect <instance> <file>
agora crash investigate <instance>

agora mcp serve --stdio
agora mcp serve --http
```

## 12.2 Use the shared data root

All commands must initialize the same core runtime used by desktop.

`--data-dir` remains available for:

* Tests
* Portable installs
* Development
* Recovery

## 12.3 Add settings support

The CLI must be able to manage settings that currently gate behavior, including:

* Modrinth enabled
* Network categories
* Global Java path
* MCP enabled
* Launch mode where relevant
* Privacy controls

A fresh CLI installation must not require editing SQLite manually.

## 12.4 Make launch honor all instance configuration

CLI launch must use:

* Per-instance Java path
* Global Java path
* Incompatible override
* Configured memory
* GC policy
* Always-pre-touch option
* Custom JVM arguments
* Health policy
* Authentication state
* Network policy

It must update:

* `last_launched_at`
* LKG
* Process outcome
* Crash state

## 12.5 Use meaningful exit codes

Define and document stable exit-code categories.

Example:

```text
0  Success
2  Invalid CLI usage
3  User decision required
4  Network disabled or unavailable
5  Authentication required
6  Health blocked
7  Launch or game crash
8  Integrity failure
9  Operation cancelled
10 Internal failure
```

Exact values may differ, but they must be:

* Stable
* Tested
* Documented
* Reflected in JSON output

A crashed game must not return exit code zero.

## 12.6 Make JSON output dependable

`--json` should produce one complete structured result.

For streamed operations, support:

```text
--output json
--output ndjson
--output human
```

Rules:

* Human text never appears on stdout in JSON mode.
* Diagnostics go to stderr.
* Secret values are redacted.
* Error responses use a stable envelope.
* Events contain operation IDs.
* Exit codes still carry semantic meaning.

## 12.7 Remove all user-input panic paths

Examples:

* No compatible Modrinth version
* Empty result set
* Missing primary file
* Invalid instance
* Invalid snapshot
* Unknown setting
* Missing registry
* Authentication cancellation

Return structured errors instead.

## 12.8 Implement or remove `serve`

Do not retain a command that prints “not implemented” and exits successfully.

Replace it with the actual MCP server or remove the command until implemented.

## 12.9 CLI command tests

Use temporary data roots and fake services.

Test at minimum:

* `paths`
* Fresh database initialization
* Settings get/set
* Instance create/list/show/delete
* Import registration
* Loader repair
* Mod dependency install
* Reverse-dependency removal block
* Dry-run plan
* JSON output
* Stable error output
* Launch with fake Java success
* Launch with fake Java crash
* Exit codes
* Registry configuration fallback
* `mcp serve --stdio` handshake

CI must run:

```bash
cargo test -p agora-cli
```

Building alone is not adequate.

---

# 13. Phase 9: Reduce desktop Rust to adapters

After core services exist, remove reusable logic from desktop modules.

## 13.1 Target command shape

A desktop command should generally be no more complicated than:

```rust
#[tauri::command]
async fn repair_instance_loader(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<RepairResult, LauncherError> {
    let runtime = runtime_from_app(&app)?;
    runtime
        .instances()
        .repair_loader(
            &instance_id,
            &TauriProgressSink::new(app),
            &CancellationToken::new(),
        )
        .await
}
```

## 13.2 Modules to shrink or remove

Review and refactor:

```text
desktop/src-tauri/src/instances.rs
desktop/src-tauri/src/install_pipeline.rs
desktop/src-tauri/src/mod_install.rs
desktop/src-tauri/src/modrinth_raw.rs
desktop/src-tauri/src/registry.rs
desktop/src-tauri/src/registry_sync.rs
desktop/src-tauri/src/crash_investigator.rs
desktop/src-tauri/src/mcp.rs
desktop/src-tauri/src/auth.rs
```

Keep desktop-specific adapters where justified.

## 13.3 Generate TypeScript bindings

Generate frontend models and command wrappers from Rust.

The generated output should include:

* Requests
* Responses
* Errors
* Events
* Install-plan types
* Instance models
* Health reports
* Launch results
* Runtime progress
* MCP status where exposed to the desktop

Check generated files into the repository or generate deterministically in CI.

Add a CI check that fails when generated bindings are stale.

## 13.4 Simplify frontend orchestration

Replace multi-command correctness workflows with coarse commands.

Examples:

```text
download Java + retry launch
repair loader + retry launch
import + register + install loader
restore snapshot + create undo point
```

The frontend still controls whether to initiate them, but core owns their atomic sequence.

---

# 14. Phase 10: Separate MCP behavior from MCP transport

## 14.1 Create a core MCP dispatcher

Core should define:

```rust
pub struct McpDispatcher {
    services: Arc<AgoraServices>,
}

impl McpDispatcher {
    pub async fn list_tools(...);
    pub async fn call_tool(...);
    pub async fn list_resources(...);
    pub async fn read_resource(...);
}
```

The dispatcher should call:

* `InstanceService`
* `InstallService`
* `CrashService`
* `RegistryService`
* `LaunchService`

It must not call desktop modules.

## 14.2 Move approval policy into core

Approval grants are business and security policy.

Core should own:

* Reading grants
* Deciding whether a tool is destructive
* Grant scope
* Session grants
* Always allow/deny
* Audit logging
* Rate limits

Desktop settings should only provide a UI for changing grants.

## 14.3 Support stdio transport

Implement:

```bash
agora mcp serve --stdio
```

Requirements:

* JSON-RPC over stdin/stdout
* No normal logs on stdout
* Diagnostics on stderr
* Client owns process lifetime
* No TCP port
* No bearer token required when the process transport itself is trusted
* Clean EOF and signal handling

## 14.4 Retain Streamable HTTP where useful

Desktop-managed MCP may continue to use loopback HTTP for persistent external access.

Requirements:

* Bind to loopback only
* Bearer authentication
* Validate `Origin`
* Avoid token in query parameters where possible
* Correct persistent rate limiting
* Body-size limits
* Header-size limits
* Request timeout
* Connection limits
* Clean lifecycle restart
* No handwritten parser unless extensively justified and tested

Prefer an established HTTP framework or MCP transport implementation.

## 14.5 Do not use MCP HTTP internally

The React frontend must not call Agora through MCP HTTP.

The CLI must not call Agora through MCP HTTP when linked directly to core.

Use:

```text
React → Tauri IPC → core
CLI → core
MCP client → MCP transport → core
```

---

# 15. Phase 11: Optional daemon for detached CLI parity

A daemon is optional for initial CLI parity but recommended for persistent operations.

## 15.1 Features requiring persistent state

Without a daemon, the CLI can support attached launch by remaining open.

A daemon is needed for robust:

* Detached launch
* Querying a launch from another terminal
* Stopping a process from another invocation
* Persistent MCP
* Cross-command cancellation
* Reconnecting to game logs
* Long-running background installs
* Operation history

## 15.2 Prefer local OS IPC over HTTP

For an internal daemon, prefer:

* Unix domain socket on Linux/macOS
* Named pipe on Windows

Do not introduce local HTTP merely because it is familiar.

The daemon protocol can use framed JSON or another simple typed protocol.

## 15.3 Keep daemon optional

Recommended progression:

1. Finish attached CLI launch.
2. Finish all stateless operations.
3. Add daemon only for persistent features.
4. Keep `agora launch` usable without a daemon.
5. Let `--detach` start or connect to the daemon.

---

# 16. Phase 12: Fix task and state ownership

## 16.1 Core owns operation state

Create an operation manager for:

* Install plans
* Install cancellation
* Java runtime provisioning
* Imports
* Registry sync
* Loader installation
* Launch reservations
* Active process sessions

Recommended structure:

```rust
pub struct OperationManager {
    installs: HashMap<OperationId, InstallOperation>,
    runtimes: HashMap<OperationId, RuntimeOperation>,
    imports: HashMap<OperationId, ImportOperation>,
    launches: HashMap<SessionId, LaunchOperation>,
}
```

Do not store operation state separately in:

* Desktop React
* Desktop Rust
* CLI
* MCP

Frontends may mirror presentation state, but core remains authoritative.

## 16.2 Use operation IDs consistently

Every long operation should have an ID included in:

* Progress events
* Cancellation calls
* Completion events
* Error events
* Logs where useful

## 16.3 Define cancellation ownership

Cancellation should be cooperative and checked during:

* Network download
* Staging
* Archive extraction
* Java provisioning
* Loader installation where safely possible
* Plan resolution
* Import
* Health scanning

Do not cancel during an unsafe atomic commit without either completing or rolling back.

Forge/NeoForge installer cancellation needs explicit policy because it is a child process modifying a shared runtime root.

## 16.4 Define process ownership

For every launched process, core must track:

* Instance ID
* Session ID
* PID
* Captured OS process identity
* Launch snapshot
* User-cancelled state
* Start time
* Current ownership host
* Whether attached or detached

Kill operations must verify process identity before signaling.

## 16.5 Define database ownership

Only core opens or mutates launcher databases.

Desktop and CLI may not issue independent SQL for business operations.

Expose typed repository or service APIs.

---

# 17. Phase 13: Testing and CI enforcement

## 17.1 Core unit tests

Add coverage for:

* Path resolution
* Data migrations
* Catalog fallback and replacement
* Catalog reload after update
* Exact Java selection
* Managed runtime validation
* Hashless cache observations
* Loader receipts
* Generated Forge artifacts
* Instance transactions
* Dependency resolution
* Reverse dependencies
* Conflict policies
* Plan fingerprinting
* Snapshot rollback
* Import manifest generation
* MCP approval policy
* Process session matching

## 17.2 Core integration tests

Use fake HTTP servers and fake Java executables.

Test:

* Vanilla launch
* Fabric launch
* Quilt launch
* Forge launch using a fake installer
* NeoForge launch using a fake installer
* Java 25 provisioning
* Offline second launch
* Catalog update and restart
* Immediate process exit
* Process cancellation
* Corrupt cache recovery
* Paths with spaces and Unicode
* Cross-process lock contention

## 17.3 CLI integration tests

Run the compiled binary against a temporary data root.

Verify:

* stdout
* stderr
* exit code
* JSON schema
* filesystem results
* database results

## 17.4 Desktop contract tests

The desktop tests should mock only the Tauri adapter boundary.

Ensure:

* Commands map parameters correctly
* Events are translated correctly
* Frontend state handles early exits
* Session IDs prevent stale events
* Generated TypeScript types match Rust
* No business decisions occur in frontend mocks

## 17.5 MCP transport tests

Test both transports against the same dispatcher fixtures.

Stdio:

* Initialize
* List tools
* Call read-only tool
* Call destructive tool without approval
* Notification without response
* Clean EOF

HTTP:

* Loopback binding
* Missing bearer token
* Invalid bearer token
* Invalid origin
* Valid Streamable HTTP call
* Legacy SSE only if retained
* Rate limiting
* Body-size rejection
* Restart after stop

## 17.6 CI matrix

At minimum:

```text
Windows latest
Ubuntu latest
macOS latest
```

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p agora-core --lib
cargo test -p agora-core --tests
cargo test -p agora-cli
cargo build -p agora-cli
cargo check -p agora-desktop
```

Add architecture checks:

* `agora-core` must not depend on Tauri.
* Desktop business modules must not duplicate core services.
* Generated TypeScript bindings must be current.
* CLI tests must execute, not merely compile.

---

# 18. Migration strategy

## 18.1 Preserve existing data

Do not change the app-data root without migration.

When the old CLI directory exists and differs from the unified directory:

1. Detect it.
2. Report its contents.
3. Offer an explicit migration command.
4. Do not automatically merge conflicting databases.
5. Create backups before migration.

Suggested command:

```bash
agora migrate-data --from <old-path>
```

## 18.2 Preserve database compatibility

Use additive migrations where possible.

Every migration must:

* Be versioned
* Be idempotent
* Run inside a transaction
* Preserve a backup or recovery strategy
* Be tested against representative old databases

## 18.3 Preserve embedded fallback catalogs

Do not remove embedded catalogs immediately.

They remain useful for:

* First startup
* Offline installation
* Registry recovery
* Testing
* Older registry schema compatibility

The signed registry should override them when valid.

---

# 19. Deletion checklist

Once migration is complete, remove:

* Duplicate desktop instance creation logic
* Duplicate desktop loader installation logic
* Duplicate desktop install-plan resolver
* Duplicate desktop Modrinth request implementations
* Duplicate CLI mod install/remove plan construction
* CLI panic paths
* CLI placeholder `serve`
* One-shot catalog initialization
* Hashless cache acceptance without local validation
* Manual TypeScript copies superseded by generated bindings
* MCP handlers that directly call desktop modules
* Handwritten HTTP parsing if replaced by a proper transport
* Compile-time-only registry repository configuration
* CLI-specific default data root

Do not retain dead code behind comments such as “legacy path” unless a documented compatibility mode still uses it.

---

# 20. Completion criteria

The work is complete when all of the following are true.

## 20.1 Ownership

* Core owns all launcher business behavior.
* Desktop Rust contains only Tauri and OS adapters.
* TypeScript contains no persistent launcher business decisions.
* CLI contains no duplicate resolver or transaction implementation.
* MCP contains no duplicate launcher behavior.

## 20.2 Direct launch

* Vanilla, Fabric, Quilt, Forge, and NeoForge launch without Mojang Launcher setup.
* Java 25 is provisioned when Minecraft metadata requires it.
* Signed catalog updates remain active after restart.
* Offline second launch succeeds after complete first materialization.
* Immediate child exit is reflected correctly in the UI.
* Hashless cached libraries are checked against local observations.
* Generated Forge artifacts remain receipt-bound.

## 20.3 CLI

* CLI and desktop share the same data by default.
* CLI can create, import, repair, configure, and launch an instance.
* CLI mod operations resolve dependencies and reverse dependencies.
* CLI honors Java and JVM settings.
* CLI returns meaningful exit codes.
* JSON mode is machine-safe.
* No normal user path panics.
* `cargo test -p agora-cli` runs in CI.

## 20.4 Transport

* React uses Tauri IPC.
* Desktop Rust calls core directly.
* CLI calls core directly.
* MCP supports a core-owned dispatcher.
* Stdio MCP works.
* HTTP MCP is optional, loopback-only, authenticated, origin-validated, and transport-only.
* Internal app behavior does not depend on local HTTP.

## 20.5 Reliability

* Shared resources use cross-process locking.
* Instance operations are transactional.
* Imports do not leave partial instances.
* Install operations snapshot and roll back.
* Process operations verify OS identity.
* Long operations support cancellation safely.
* Secrets are never emitted through logs or debug output.

---

# 21. Recommended implementation order

Use this exact sequence to reduce rework:

1. Baseline tests and architecture document.
2. `AppPaths`, expanded `Ctx`, shared HTTP clients, sinks, and lock manager.
3. Unified data directory and core initialization.
4. Dynamic loader/runtime catalog lifecycle.
5. Core `LaunchService` and remaining launch fixes.
6. Core loader and `InstanceService`.
7. Core install-plan preparation and `InstallService`.
8. Transactional `ImportService`.
9. CLI command parity and tests.
10. Core MCP dispatcher and stdio transport.
11. Hardened optional HTTP MCP transport.
12. Desktop adapter cleanup.
13. Generated TypeScript bindings.
14. Optional daemon for detached CLI operations.
15. Full cross-platform smoke testing and release gating.

Do not begin broad CLI command expansion before moving the corresponding desktop orchestration into core. Otherwise the CLI will either duplicate behavior or remain functionally weaker.

---

# 22. Final target

The final system should behave as one launcher engine with multiple presentations:

```text
                         agora-core

        InstanceService      InstallService
        LaunchService        ImportService
        RuntimeService       RegistryService
        AuthService          CrashService
        SnapshotService      McpDispatcher

             ▲                    ▲
             │                    │
      Desktop/Tauri adapter    CLI/Clap adapter
             │                    │
       React interface       Terminal / scripts

                         ▲
                         │
              MCP stdio or HTTP transport
```

A bug fix made in core must automatically benefit:

* Desktop users
* CLI users
* MCP clients
* Future frontends

No operation should have to be fixed independently in three different implementations.
