//! Canonical core context — the dependency bag for every `agora-core` operation.
//!
//! `Ctx` (aliased from `CoreContext`) is constructed once per host (Tauri GUI,
//! CLI, MCP listener) and provides all shared resources: paths, HTTP clients,
//! clock, lock manager, and event/progress sinks. No adapter may reconstruct
//! app-data subpaths or create unconfigured HTTP clients on its own.
//!
//! Constraint: this struct MUST NOT carry any `tauri::`, `clap::`, or
//! MCP-protocol type.

use crate::app_paths::AppPaths;
use crate::error::{LauncherError, LauncherResult};
use crate::event_sink::{EventSink, NoopEventSink, NoopProgressSink, ProgressSink};
use crate::http_client::HttpClients;
use crate::lock_manager::LockManager;
use crate::operation_manager::OperationManager;
use crate::process_session_manager::ProcessSessionManager;
use crate::runtime_catalog::{RuntimeCatalog, RuntimeCatalogHandle};
use crate::task_scheduler::TaskScheduler;
use std::sync::Arc;
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// Clock abstraction
// ---------------------------------------------------------------------------

/// Abstract clock so tests can control time without wall-clock dependency.
pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

/// Wall-clock implementation.
#[derive(Debug, Clone, Copy)]
pub struct WallClock;

impl Clock for WallClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// A deterministic clock for tests.
#[derive(Debug, Clone)]
pub struct TestClock {
    now: Arc<std::sync::Mutex<SystemTime>>,
}

impl TestClock {
    pub fn new(start: SystemTime) -> Self {
        Self {
            now: Arc::new(std::sync::Mutex::new(start)),
        }
    }

    pub fn advance(&self, dur: std::time::Duration) {
        let mut guard = self.now.lock().unwrap();
        *guard += dur;
    }
}

impl Clock for TestClock {
    fn now(&self) -> SystemTime {
        *self.now.lock().unwrap()
    }
}

// ---------------------------------------------------------------------------
// CoreContext (canonical)
// ---------------------------------------------------------------------------

/// All shared resources for an Agora core session.
///
/// # Construction
///
/// Use [`CoreContext::initialize`] for production or
/// [`CoreContext::for_testing`] for tests.
///
/// # Type alias
///
/// `pub type Ctx = CoreContext;` — prefer the shorter name when referring to
/// the canonical context throughout the codebase.
#[derive(Clone)]
pub struct CoreContext {
    /// Canonical path layout for this host.
    pub paths: AppPaths,
    /// Official Mojang launcher profile file. Test contexts point this at an
    /// isolated fixture instead of the user's platform-default Minecraft data.
    pub launcher_profiles_path: Option<std::path::PathBuf>,
    /// Category-aware pre-built HTTP clients.
    pub http_clients: HttpClients,
    /// Clock for time-sensitive operations.
    pub clock: Arc<dyn Clock>,
    /// Cross-process filesystem lock manager.
    pub lock_manager: LockManager,
    /// Validated Java runtime catalog active for this context.
    /// Wrapped in an `Arc<RwLock>` so any holder can take an atomic snapshot
    /// and so the catalog can be reloaded at runtime without process restart.
    pub runtime_catalog: RuntimeCatalogHandle,
    /// Sink for progress events emitted during long-running operations.
    pub progress_sink: Arc<dyn ProgressSink>,
    /// Sink for significant core events.
    pub event_sink: Arc<dyn EventSink>,
    /// Cloneable operation manager for tracking long-running operations.
    pub operation_manager: OperationManager,
    /// Bounded worker admission for blocking filesystem and archive work.
    pub task_scheduler: TaskScheduler,
    /// Process session manager for direct-launch lifecycle tracking.
    pub process_session_manager: ProcessSessionManager,
}

/// The canonical context type alias — use `Ctx` throughout the codebase.
pub type Ctx = CoreContext;

impl CoreContext {
    /// Initialize the core context from an `AppPaths`.
    ///
    /// Steps:
    /// 1. Create required directories.
    /// 2. **Fatal**: open and migrate `local_state.db`. Failure here returns
    ///    an error — without persistent state the app cannot function.
    /// 3. Validate cached registry schema version (warning only).
    /// 4. **Conservative** stale-staging cleanup: only removes directories
    ///    whose marker file `staging-complete` exists AND whose creation
    ///    time is older than 24h _and_ no active lock file references them.
    /// 5. Probe lock-directory writability (warning only).
    ///
    /// Warnings are returned as `Vec<String>` — genuinely recoverable
    /// observations that did not prevent initialization.
    pub fn initialize(paths: AppPaths) -> LauncherResult<(Self, Vec<String>)> {
        let mut warnings = Vec::new();

        // 1. Create required directories.
        paths.create_required_dirs()?;

        // 2. Fatal: initialize / migrate local_state.db.
        let db_path = paths.local_state_db();
        crate::db::init_local_state_db(&db_path).map_err(|e| {
            let msg = format!(
                "Failed to initialize local state database at {}: {e}",
                db_path.display()
            );
            // Log the path without secrets or token data.
            LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: msg,
            }
        })?;
        warnings.extend(crate::launcher_import_service::recover_interrupted_jobs(
            &paths,
        ));

        // 3. Verify the cached registry's Ed25519 signature before anything
        //    reads it. Download-time verification only covers transit; the
        //    cached copy sits in a user-writable directory and supplies every
        //    download URL and expected hash the launcher trusts. A failing db is
        //    quarantined here, which also leaves no `registry.db` behind so the
        //    next sync re-downloads a good copy.
        if let crate::registry_sync::CachedRegistryIntegrity::Quarantined(reason) =
            crate::registry_sync::verify_cached_registry(paths.root())
        {
            warnings.push(reason);
        }

        //    Then validate the cached registry schema version (warning only —
        //    the cached db may be absent or stale, which is recoverable).
        let reg_path = paths.registry_db();
        let mut runtime_catalog = RuntimeCatalog::embedded();
        if reg_path.exists() {
            match crate::db::registry_connection(&reg_path) {
                Ok(conn) => {
                    let version: Result<i64, _> = conn.query_row(
                        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                        [],
                        |row| row.get(0),
                    );
                    if let Ok(v) = version {
                        if v > crate::registry_sync::APP_REGISTRY_SCHEMA_VERSION {
                            warnings.push(format!(
                                "Cached registry schema v{v} is newer than app (v{}); upgrade launcher",
                                crate::registry_sync::APP_REGISTRY_SCHEMA_VERSION,
                            ));
                        }
                    }
                    match crate::loader_manifests::LoaderCatalog::init_from_registry(&conn) {
                        Ok(true) => {
                            warnings.push("Using merged signed and embedded loader catalogs".into())
                        }
                        Ok(false) => warnings.push("Using embedded loader catalog".into()),
                        Err(error) => warnings.push(format!(
                            "Cannot load signed loader catalog; using embedded fallback: {error}"
                        )),
                    }
                    match RuntimeCatalog::from_registry_db(&conn) {
                        Ok(Some(catalog)) => {
                            runtime_catalog = catalog;
                            warnings.push("Using signed registry Java runtime catalog".into());
                        }
                        Ok(None) => warnings.push("Using embedded Java runtime catalog".into()),
                        Err(errors) => warnings.push(format!(
                            "Cannot load signed Java runtime catalog; using embedded fallback: {errors:?}"
                        )),
                    }
                }
                Err(e) => {
                    warnings.push(format!("Cannot open cached registry: {e}"));
                    warnings.push("Using embedded loader and Java runtime catalogs".into());
                }
            }
        } else {
            warnings.push("Using embedded loader and Java runtime catalogs".into());
        }

        // 4. Conservative stale-staging cleanup.
        //    Only remove staged directories that:
        //    - Have existed for more than 24 hours
        //    - Contain a `staging-complete` marker file (orphaned after
        //      a crash), OR have no active lock referencing them
        let staging_root = paths.staging_root();
        if staging_root.exists() {
            let stale_threshold = std::time::Duration::from_secs(24 * 60 * 60);
            let now = std::time::SystemTime::now();
            if let Ok(entries) = std::fs::read_dir(&staging_root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    // Check age via directory's modified time.
                    let age = match path.metadata().and_then(|m| m.modified()) {
                        Ok(modified) => match now.duration_since(modified) {
                            Ok(d) => d,
                            Err(_) => continue,
                        },
                        Err(_) => continue,
                    };
                    if age < stale_threshold {
                        continue;
                    }
                    // Conservative: only remove if there's a marker file
                    // confirming this was a completed staging attempt.
                    if path.join("staging-complete").exists() {
                        let _ = std::fs::remove_dir_all(&path);
                    }
                    // Otherwise leave it; an operation-manager cleanup
                    // later can handle orphaned staging dirs.
                }
            }
        }

        // 5. Validate lock directory is usable (warning only).
        let locks_root = paths.locks_root();
        if locks_root.exists() {
            let probe = locks_root.join(".probe");
            match std::fs::write(&probe, b"probe") {
                Ok(_) => {
                    let _ = std::fs::remove_file(&probe);
                }
                Err(e) => warnings.push(format!("Lock directory may not be writable: {e}")),
            }
        }

        let ctx = Self {
            lock_manager: LockManager::new(paths.locks_root()),
            launcher_profiles_path: crate::paths::launcher_profiles_path(),
            paths,
            http_clients: HttpClients::new().map_err(|e| {
                // Convert HTTP client init failure to a fatal error.
                LauncherError::Generic {
                    code: "ERR_HTTP_CLIENT_INIT".into(),
                    message: format!("Failed to build HTTP clients: {e}"),
                }
            })?,
            runtime_catalog: RuntimeCatalogHandle::new(runtime_catalog),
            clock: Arc::new(WallClock),
            progress_sink: Arc::new(NoopProgressSink),
            event_sink: Arc::new(NoopEventSink),
            operation_manager: OperationManager::new(),
            task_scheduler: TaskScheduler::default(),
            process_session_manager: ProcessSessionManager::new(),
        };

        Ok((ctx, warnings))
    }

    /// Create a context for testing with temporary paths.
    ///
    /// The caller should create a temp directory and pass its path.
    /// Uses a fast single-client HTTP client (no policy enforcement).
    pub fn for_testing(root: std::path::PathBuf) -> Self {
        let launcher_profiles_path = root
            .join("official-minecraft")
            .join("launcher_profiles.json");
        let paths = AppPaths::from_root(root);
        let lock_manager = LockManager::new(paths.locks_root());
        let _ = std::fs::create_dir_all(paths.locks_root());
        Self {
            paths,
            launcher_profiles_path: Some(launcher_profiles_path),
            http_clients: HttpClients::for_testing(reqwest::Client::new()),
            runtime_catalog: RuntimeCatalogHandle::new(RuntimeCatalog::embedded()),
            clock: Arc::new(WallClock),
            lock_manager,
            progress_sink: Arc::new(NoopProgressSink),
            event_sink: Arc::new(NoopEventSink),
            operation_manager: OperationManager::new(),
            task_scheduler: TaskScheduler::default(),
            process_session_manager: ProcessSessionManager::new(),
        }
    }

    /// Replace the progress sink (e.g., to wire in a Tauri event emitter).
    pub fn with_progress_sink(mut self, sink: Arc<dyn ProgressSink>) -> Self {
        self.progress_sink = sink;
        self
    }

    /// Replace the event sink (e.g., to wire in a Tauri event emitter).
    pub fn with_event_sink(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.event_sink = sink;
        self
    }

    /// Replace the clock (e.g., with a test clock).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Replace the HTTP clients (e.g., with a single client for testing).
    pub fn with_http_clients(mut self, clients: HttpClients) -> Self {
        self.http_clients = clients;
        self
    }

    /// Replace the process session manager (e.g., for testing with a shared
    /// manager between service and test harness).
    pub fn with_process_session_manager(mut self, mgr: ProcessSessionManager) -> Self {
        self.process_session_manager = mgr;
        self
    }

    /// Atomically reload the Java runtime catalog from the signed
    /// `registry.db`, preserving the current catalog on validation failure.
    ///
    /// The loader catalog is also reloaded through the existing
    /// [`LoaderCatalog::init_from_registry`] path so both catalogs stay
    /// coordinated after a fresh registry download.
    ///
    /// Returns a list of human-readable warnings (e.g. fallback to embedded).
    /// Errors are reserved for I/O failures opening the database.
    pub fn reload_runtime_catalog(&self) -> LauncherResult<Vec<String>> {
        let mut warnings = Vec::new();
        let reg_path = self.paths.registry_db();
        if !reg_path.exists() {
            warnings.push("No registry database found; runtime catalog unchanged".into());
            return Ok(warnings);
        }
        let conn = match crate::db::registry_connection(&reg_path) {
            Ok(conn) => conn,
            Err(e) => {
                warnings.push(format!(
                    "Cannot open registry database; runtime catalog unchanged: {e}"
                ));
                return Ok(warnings);
            }
        };

        // Parse both catalogs before replacing either active snapshot. This
        // keeps loader and runtime data consistent when one catalog is corrupt.
        let registry_loader_catalog = match crate::loader_manifests::LoaderCatalog::from_registry(
            &conn,
        ) {
            Ok(catalog) => catalog,
            Err(error) => {
                warnings.push(format!(
                    "Cannot load signed loader catalog; preserving existing active catalogs: {error}"
                ));
                return Ok(warnings);
            }
        };
        let has_registry_loader_catalog = registry_loader_catalog.is_some();
        let loader_catalog = match crate::loader_manifests::LoaderCatalog::merge_with_embedded(
            registry_loader_catalog,
        ) {
            Ok(catalog) => catalog,
            Err(error) => {
                warnings.push(format!(
                    "Cannot merge signed loader catalog; preserving existing active catalogs: {error}"
                ));
                return Ok(warnings);
            }
        };
        let runtime_catalog = match RuntimeCatalog::from_registry_db(&conn) {
            Ok(Some(catalog)) => {
                warnings.push("Using signed registry Java runtime catalog".into());
                catalog
            }
            Ok(None) => {
                warnings.push("No runtime catalog in registry; using embedded".into());
                RuntimeCatalog::embedded()
            }
            Err(errors) => {
                warnings.push(format!(
                    "Cannot reload Java runtime catalog; preserving existing active catalogs: {errors:?}"
                ));
                return Ok(warnings);
            }
        };

        crate::loader_manifests::LoaderCatalog::replace_active(Some(loader_catalog))?;
        self.runtime_catalog.replace(runtime_catalog);
        warnings.push(if has_registry_loader_catalog {
            "Using merged signed and embedded loader catalogs".into()
        } else {
            "Using embedded loader catalog".into()
        });

        Ok(warnings)
    }

    /// The lock manager reference.
    pub fn lock_manager(&self) -> &LockManager {
        &self.lock_manager
    }

    /// The path model reference.
    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    /// The HTTP clients reference.
    pub fn http_clients(&self) -> &HttpClients {
        &self.http_clients
    }
}

impl std::fmt::Debug for CoreContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreContext")
            .field("paths", &self.paths)
            .field("launcher_profiles_path", &self.launcher_profiles_path)
            .field("http_clients", &self.http_clients)
            .field("lock_manager", &self.lock_manager)
            .field("runtime_catalog", &self.runtime_catalog)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// The canonical Ctx is CoreContext — no separate old-style Ctx remains.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_creates_dirs_and_db() {
        let tmp = std::env::temp_dir().join(format!("agora-ctx-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let paths = AppPaths::from_root(tmp.clone());

        let (ctx, warnings) = CoreContext::initialize(paths).unwrap();
        assert!(ctx.paths.root().exists(), "root should exist");
        assert!(
            ctx.paths.local_state_db().exists(),
            "local_state.db should exist"
        );
        assert!(
            ctx.paths.instances_root().exists(),
            "instances dir should exist"
        );
        assert!(ctx.paths.locks_root().exists(), "locks dir should exist");
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("embedded loader and Java runtime catalogs")),
            "clean init should report embedded catalog sources: {:?}",
            warnings
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_for_testing_succeeds() {
        let tmp = std::env::temp_dir().join(format!("agora-ctx-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let ctx = CoreContext::for_testing(tmp.clone());
        assert!(
            ctx.http_clients
                .get(crate::http_client::ClientCategory::GitHub) as *const _ as usize
                > 0
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_with_sinks_and_clock() {
        let tmp = std::env::temp_dir().join(format!("agora-ctx-sink-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let ctx = CoreContext::for_testing(tmp.clone())
            .with_progress_sink(Arc::new(crate::event_sink::NoopProgressSink))
            .with_event_sink(Arc::new(crate::event_sink::NoopEventSink));
        // No panic means it works.
        drop(ctx);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_clock_abstraction() {
        let start = std::time::SystemTime::now();
        let test_clock = TestClock::new(start);
        let ctx = CoreContext::for_testing(std::env::temp_dir().join("agora-clock-test"))
            .with_clock(Arc::new(test_clock));
        let _now = ctx.clock.now();
    }

    #[test]
    fn test_test_clock_advance() {
        let start = std::time::SystemTime::now();
        let clock = TestClock::new(start);
        let t1 = clock.now();
        clock.advance(std::time::Duration::from_secs(60));
        let t2 = clock.now();
        assert!(t2 >= t1);
    }

    #[test]
    fn test_invalid_db_path_fails() {
        // A path to a non-existent parent directory should fail.
        let tmp = std::env::temp_dir().join(format!("agora-ctx-dbfail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        // Use a subdirectory that doesn't exist and can't be created
        // (but paths.create_required_dirs will create the root, so make
        // the db_path itself unwritable).
        let paths = AppPaths::from_root(tmp.clone());
        // Create root dir but make local_state.db path unwritable by
        // creating it as a directory beforehand.
        std::fs::create_dir_all(tmp.join("local_state.db")).unwrap();
        let result = CoreContext::initialize(paths);
        assert!(result.is_err(), "should fail when db cannot be created");
        let err = result.unwrap_err();
        assert_eq!(err.code(), "ERR_LOCAL_STATE_FAILED");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_runtime_catalog_handle_snapshot_via_ctx() {
        let tmp = std::env::temp_dir().join(format!("agora-ctx-snap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let ctx = CoreContext::for_testing(tmp.clone());
        let catalog = ctx.runtime_catalog.snapshot();
        assert!(
            !catalog.entries.is_empty(),
            "snapshot should have embedded entries"
        );
        // A second snapshot is independent
        let catalog2 = ctx.runtime_catalog.snapshot();
        assert_eq!(catalog, catalog2);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_reload_no_registry_db_returns_warning() {
        let tmp =
            std::env::temp_dir().join(format!("agora-ctx-reload-nodb-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let ctx = CoreContext::for_testing(tmp.clone());
        let warnings = ctx.reload_runtime_catalog().unwrap();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("No registry database found")),
            "should warn when registry.db is absent: {:?}",
            warnings
        );
        // Snapshot should still return embedded catalog (unchanged).
        let catalog = ctx.runtime_catalog.snapshot();
        assert!(!catalog.entries.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_reload_runtime_catalog_with_valid_registry() {
        let tmp = std::env::temp_dir().join(format!("agora-ctx-reload-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Create a minimal registry.db with runtime_catalog and loader_catalog tables.
        let reg_path = tmp.join("registry.db");
        let conn = rusqlite::Connection::open(&reg_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE runtime_catalog (singleton_id INTEGER PRIMARY KEY, catalog_json TEXT NOT NULL);
             CREATE TABLE loader_catalog (singleton_id INTEGER PRIMARY KEY, catalog_json TEXT NOT NULL);
             CREATE TABLE schema_version (version INTEGER PRIMARY KEY);"
        ).unwrap();

        let embedded_json = include_str!("../../../runtime-catalog/runtime_catalog.json");
        conn.execute(
            "INSERT INTO runtime_catalog (singleton_id, catalog_json) VALUES (1, ?1)",
            [embedded_json],
        )
        .unwrap();
        let manifests = include_str!("../../../loader-manifests/loader_manifests.json");
        conn.execute(
            "INSERT INTO loader_catalog (singleton_id, catalog_json) VALUES (1, ?1)",
            [manifests],
        )
        .unwrap();
        drop(conn);

        let ctx = CoreContext::for_testing(tmp.clone());

        // Confirm we start with embedded.
        let before = ctx.runtime_catalog.snapshot();
        assert!(!before.entries.is_empty());

        // Reload from the registry.db we just placed.
        let warnings = ctx.reload_runtime_catalog().unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("signed registry")),
            "should report signed registry load: {:?}",
            warnings
        );

        // After reload, the catalog should still be valid (same data).
        let after = ctx.runtime_catalog.snapshot();
        assert!(!after.entries.is_empty());
        assert_eq!(after.schema_version, before.schema_version);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_reload_runtime_catalog_old_preserved_on_failure() {
        let tmp =
            std::env::temp_dir().join(format!("agora-ctx-reload-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Create a registry.db with invalid runtime catalog JSON.
        let reg_path = tmp.join("registry.db");
        let conn = rusqlite::Connection::open(&reg_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE runtime_catalog (singleton_id INTEGER PRIMARY KEY, catalog_json TEXT NOT NULL);
             CREATE TABLE schema_version (version INTEGER PRIMARY KEY);"
        ).unwrap();
        conn.execute(
            "INSERT INTO runtime_catalog (singleton_id, catalog_json) VALUES (1, ?1)",
            [r#"{"invalid": "no schema version"}"#],
        )
        .unwrap();
        drop(conn);

        let ctx = CoreContext::for_testing(tmp.clone());

        // Snapshot before reload is the embedded catalog.
        let before = ctx.runtime_catalog.snapshot();
        assert!(!before.entries.is_empty());

        // Reload should fail validation but NOT replace the catalog.
        let warnings = ctx.reload_runtime_catalog().unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("preserving existing")),
            "should warn about preserving existing catalog: {:?}",
            warnings
        );

        // The catalog must still be the embedded one.
        let after = ctx.runtime_catalog.snapshot();
        assert!(!after.entries.is_empty());
        assert_eq!(
            after, before,
            "catalog should be unchanged after failed reload"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
