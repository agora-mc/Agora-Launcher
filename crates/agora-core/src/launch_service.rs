//! Core-owned launch orchestration for both direct (attached Java) and
//! delegated (external launcher) modes.

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::java::JavaInstallation;
use crate::launch::LoaderInfo;
use crate::launch_planner::{BuildCommandRequest, LaunchFeatures, LaunchIdentity, ResolveRequest};
use crate::lkg::LaunchOutcome;
use crate::models::InstanceManifest;
use crate::network::NetworkPolicy;
use crate::process_identity::ProcessIdentity;
use crate::process_session_manager::ProcessSession;
use crate::runtime_manager::RuntimeProgress;
use crate::task_scheduler::BlockingPriority;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);
type JavaDiscoveryCache = std::collections::HashMap<String, (Instant, Vec<JavaInstallation>)>;
static JAVA_DISCOVERY_CACHE: LazyLock<Mutex<JavaDiscoveryCache>> =
    LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));
const JAVA_DISCOVERY_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// Whether the service should directly execute Java or hand off to an
/// external launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Direct,
    Delegated,
}

/// Coarse recovery action requested by the frontend before a retry launch.
/// The action is performed in the same backend operation; if it fails the
/// launch is aborted and the error is returned to the caller.
///
/// Internally tagged (`{ "type": ... }`) so the desktop frontend's existing
/// discriminated-union payloads (`{type:'RepairLoader'}`, `{type:
/// 'ProvisionJava', major}`, `{type:'SwitchLoader', target_version}`) are
/// accepted verbatim at the IPC boundary.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum LaunchRecoveryAction {
    /// No recovery — plain launch.
    None,
    /// Provision a managed Java runtime for the given major version, then
    /// retry the launch.
    ProvisionJava { major: u32 },
    /// Force-reinstall the instance's loader (repair), then retry the launch.
    RepairLoader,
    /// Switch the instance's loader to a signed catalog version, then retry
    /// the launch. The switch and the launch run in one backend-owned
    /// operation.
    SwitchLoader { target_version: String },
}

/// Health behavior selected by the frontend or CLI policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthPolicy {
    BlockOnRed,
    WarnOnly,
}

/// Runtime provisioning policy used when the exact Java major is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaRuntimeMode {
    Automatic,
    Manual,
}

/// UI-neutral launch progress hook. Adapters receive lifecycle callbacks
/// during both direct and delegated launches. The [`handoff`] method is
/// called in Delegated mode so the adapter can invoke the external launcher.
pub trait LaunchProgress: Send + Sync {
    fn phase(&self, _name: &str, _message: &str) {}
    fn phase_completed(&self, _name: &str, _duration_ms: u128) {}
    fn started(&self, _started: &LaunchStarted) {}
    fn log(&self, _stream: &str, _line: &str) {}
    fn finished(&self, _result: &LaunchResult) {}
    /// Called when a delegated launch is ready. The adapter must invoke
    /// the external Mojang launcher and return `Ok(())`. The default
    /// returns an error indicating delegated launch is unsupported.
    fn handoff(&self, _identity: &LaunchIdentity) -> LauncherResult<()> {
        Err(LauncherError::Generic {
            code: "ERR_DELEGATED_LAUNCH_ADAPTER_REQUIRED".into(),
            message: "Delegated launch not supported by this adapter.".into(),
        })
    }
}

/// No-op progress implementation for callers that only need the result.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopLaunchProgress;

impl LaunchProgress for NoopLaunchProgress {}

/// Frontend-neutral request for a complete launch operation.
#[derive(Debug, Clone)]
pub struct LaunchRequest {
    pub instance_id: String,
    pub mode: LaunchMode,
    pub health_policy: HealthPolicy,
    pub health_scan_token: Option<String>,
}

/// State loaded and normalized by [`LaunchService`] before execution.
#[derive(Debug, Clone)]
struct LaunchInputs {
    mode: LaunchMode,
    instance_id: String,
    health_policy: HealthPolicy,
    health_scan_token: Option<String>,
    java_runtime_mode: JavaRuntimeMode,
    manifest: InstanceManifest,
    game_dir: PathBuf,
    minecraft_root: PathBuf,
    assets_dir: PathBuf,
    runtimes_root: PathBuf,
    java_inventory_path: PathBuf,
    receipts_root: PathBuf,
    registry_db: Option<PathBuf>,
    network_policy: NetworkPolicy,
    identity: LaunchIdentity,
    java_override: Option<PathBuf>,
    allow_incompatible_java_override: bool,
    java_candidates: Vec<JavaInstallation>,
    jvm_memory_mb: i64,
    jvm_gc_profile: Option<crate::gc::GcProfile>,
    jvm_custom_args: String,
    jvm_always_pre_touch: bool,
    extra_game_args: Vec<String>,
}

/// Process-start result delivered before launch monitoring begins. Delegated
/// handoffs use `pid == 0` and an empty `java_path` because the official
/// launcher owns the eventual Java process.
#[derive(Debug, Clone)]
pub struct LaunchStarted {
    pub pid: u32,
    pub session_id: u64,
    pub snapshot_id: String,
    pub java_path: PathBuf,
    pub process_identity: ProcessIdentity,
}

/// Result of a launch operation. Delegated handoffs return immediately with
/// `pid == 0`, an empty `java_path`, and [`LaunchOutcome::Unknown`]; their
/// eventual outcome is produced by [`LaunchService::wait_delegated`].
#[derive(Debug, Clone)]
pub struct LaunchResult {
    pub pid: u32,
    pub session_id: u64,
    pub outcome: LaunchOutcome,
    pub snapshot_id: String,
    pub java_path: PathBuf,
    pub process_identity: ProcessIdentity,
}

/// Core-owned launch lifecycle service.
#[derive(Clone)]
pub struct LaunchService {
    ctx: Ctx,
}

impl LaunchService {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    /// Execute the complete launch lifecycle once. Both modes validate health
    /// and retain a pre-launch recovery snapshot. Direct mode additionally
    /// resolves and materializes the Java launch plan before spawning the game;
    /// delegated mode leaves that work to the official launcher, calls
    /// [`LaunchProgress::handoff`], and returns promptly. Callers of Delegated
    /// mode should spawn a background task calling [`Self::wait_delegated`] for
    /// monitoring.
    pub async fn launch(
        &self,
        request: LaunchRequest,
        progress: &dyn LaunchProgress,
    ) -> LauncherResult<LaunchResult> {
        validate_instance_id(&request.instance_id)?;
        let _lock = self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(request.instance_id.clone()),
            "launch",
        )?;
        self.launch_locked(request, progress).await
    }

    async fn launch_locked(
        &self,
        request: LaunchRequest,
        progress: &dyn LaunchProgress,
    ) -> LauncherResult<LaunchResult> {
        progress.phase("loading-inputs", "Loading instance and account state");
        let started = Instant::now();
        let inputs = self.load_inputs(request).await?;
        progress.phase_completed("loading-inputs", started.elapsed().as_millis());
        // The complete direct-launch state machine contains several large
        // resolver/materializer futures. Keep it heap-backed so CLI hosts with
        // the default Windows main-thread stack do not have to inline that
        // state into their command-dispatch future.
        Box::pin(self.launch_inputs(inputs, progress)).await
    }

    /// Execute a launch with an optional recovery step performed before
    /// the actual launch. If the recovery action fails the launch is aborted
    /// and the error is returned. [`LaunchRecoveryAction::None`] behaves
    /// identically to [`Self::launch`].
    pub async fn launch_with_recovery(
        &self,
        request: LaunchRequest,
        action: LaunchRecoveryAction,
        progress: &dyn LaunchProgress,
    ) -> LauncherResult<LaunchResult> {
        validate_instance_id(&request.instance_id)?;
        let _lock = self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Instance(request.instance_id.clone()),
            "launch-recovery",
        )?;
        match action {
            LaunchRecoveryAction::None => {}
            LaunchRecoveryAction::ProvisionJava { major } => {
                progress.phase("recovery", "Provisioning the required Java runtime");
                let policy = crate::network::NetworkPolicy::from_ctx(&self.ctx)?;
                policy.check(crate::network::NetworkCategory::JavaRuntime)?;
                let runtimes_root = self.ctx.paths.java_runtimes_root();
                let catalog = self.ctx.runtime_catalog.snapshot();
                let lock_manager = self.ctx.lock_manager.clone();
                self.ctx
                    .task_scheduler
                    .run_blocking(BlockingPriority::Launch, move || {
                        crate::runtime_manager::ensure_runtime(
                            &runtimes_root,
                            major,
                            &catalog,
                            &policy,
                            None::<&dyn crate::runtime_manager::RuntimeProgress>,
                            Some(&lock_manager),
                        )
                    })
                    .await
                    .map_err(|error| LauncherError::Generic {
                        code: "ERR_JAVA_PROVISION".into(),
                        message: format!("Java provisioning task failed: {error}"),
                    })??;
            }
            LaunchRecoveryAction::RepairLoader => {
                progress.phase("recovery", "Repairing loader installation");
                let loader_svc = crate::loader_service::LoaderService::new(self.ctx.clone());
                loader_svc.repair(&request.instance_id).await?;
            }
            LaunchRecoveryAction::SwitchLoader { target_version } => {
                progress.phase("recovery", "Switching loader version");
                let loader_svc = crate::loader_service::LoaderService::new(self.ctx.clone());
                loader_svc
                    .change_loader_version_locked(&request.instance_id, &target_version, false)
                    .await?;
            }
        }
        self.launch_locked(request, progress).await
    }

    async fn load_inputs(&self, request: LaunchRequest) -> LauncherResult<LaunchInputs> {
        let conn = crate::db::local_state_connection(&self.ctx.paths.local_state_db()).map_err(
            |error| LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            },
        )?;
        let row = crate::db::get_instance(&conn, &request.instance_id)
            .map_err(|error| LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: error.to_string(),
            })?
            .ok_or_else(|| LauncherError::Generic {
                code: "ERR_INSTANCE_NOT_FOUND".into(),
                message: format!("Instance '{}' not found.", request.instance_id),
            })?;
        let manifest_path = self.ctx.paths.instance_manifest(&request.instance_id)?;
        let manifest_text =
            std::fs::read_to_string(&manifest_path).map_err(|error| LauncherError::Generic {
                code: "ERR_INSTANCE_MANIFEST".into(),
                message: error.to_string(),
            })?;
        let manifest: InstanceManifest =
            serde_json::from_str(&manifest_text).map_err(|error| LauncherError::Generic {
                code: "ERR_INSTANCE_MANIFEST".into(),
                message: error.to_string(),
            })?;
        let network_policy = NetworkPolicy::from_db(&conn);
        let identity = if request.mode == LaunchMode::Direct {
            network_policy.check(crate::network::NetworkCategory::MicrosoftAuthentication)?;
            match crate::msa::get_valid_credentials(
                self.ctx
                    .http_clients
                    .get(crate::http_client::ClientCategory::Microsoft),
            )
            .await
            {
                crate::msa::MsaCredentialOutcome::Valid(credentials) => LaunchIdentity {
                    username: credentials.username,
                    access_token: credentials.access_token,
                    uuid: credentials.uuid,
                    user_type: "msa".into(),
                    client_id: String::new(),
                    xuid: String::new(),
                    user_properties: "{}".into(),
                },
                crate::msa::MsaCredentialOutcome::SignInRequired => {
                    return Err(LauncherError::MsaAuthRequired)
                }
                crate::msa::MsaCredentialOutcome::RefreshFailed(error) => return Err(error),
            }
        } else {
            LaunchIdentity {
                username: "Player".into(),
                access_token: String::new(),
                uuid: "00000000000000000000000000000000".into(),
                user_type: "legacy".into(),
                client_id: String::new(),
                xuid: String::new(),
                user_properties: "{}".into(),
            }
        };
        let java_runtime_mode = match crate::db::get_setting(&conn, "java_runtime_mode")
            .ok()
            .flatten()
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
        {
            Some("manual") => JavaRuntimeMode::Manual,
            _ => JavaRuntimeMode::Automatic,
        };
        let java_override = row
            .java_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                crate::db::get_setting(&conn, "java_path")
                    .ok()
                    .flatten()
                    .and_then(|value| value.as_str().map(PathBuf::from))
            });
        let minecraft_root = self.ctx.paths.minecraft_runtime_root();
        let layout = crate::minecraft_runtime::ensure_runtime_layout(&minecraft_root)?;
        let jvm_gc_profile = match row.jvm_gc.to_ascii_lowercase().as_str() {
            "zgc" | "low_latency" => Some(crate::gc::GcProfile::LowLatency),
            // `g1gc` was the implicit default before Auto was persisted.
            "high_efficiency" => Some(crate::gc::GcProfile::HighEfficiency),
            "manual" => Some(crate::gc::GcProfile::Manual),
            _ => None,
        };
        let global_pre_touch = crate::db::get_setting(&conn, "jvm_always_pre_touch")
            .ok()
            .flatten()
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let jvm_always_pre_touch = row.jvm_always_pre_touch && global_pre_touch;

        let game_dir = self.ctx.paths.instance_dir(&row.instance_id)?;
        let jvm_memory_mb = if request.mode == LaunchMode::Direct && row.jvm_memory_mode == "auto" {
            let summary = crate::memory_recommendation::summarize_instance(&game_dir, &manifest);
            crate::memory_recommendation::detect_and_recommend(&summary).recommended_mb
        } else {
            row.jvm_memory_mb
        };

        Ok(LaunchInputs {
            mode: request.mode,
            instance_id: request.instance_id,
            health_policy: request.health_policy,
            health_scan_token: request.health_scan_token,
            java_runtime_mode,
            manifest,
            game_dir,
            minecraft_root,
            assets_dir: layout.assets,
            runtimes_root: self.ctx.paths.java_runtimes_root(),
            java_inventory_path: self.ctx.paths.java_inventory(),
            receipts_root: self.ctx.paths.loader_receipts(),
            registry_db: self
                .ctx
                .paths
                .registry_db()
                .exists()
                .then(|| self.ctx.paths.registry_db()),
            network_policy,
            identity,
            java_override,
            allow_incompatible_java_override: row.java_incompatible_override,
            java_candidates: Vec::new(),
            jvm_memory_mb,
            jvm_gc_profile,
            jvm_custom_args: row.jvm_custom_args,
            jvm_always_pre_touch,
            extra_game_args: Vec::new(),
        })
    }

    async fn launch_inputs(
        &self,
        request: LaunchInputs,
        progress: &dyn LaunchProgress,
    ) -> LauncherResult<LaunchResult> {
        let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let _op_handle = self.ctx.operation_manager.register_for_instance(
            &format!("launch '{}'", request.instance_id),
            &request.instance_id,
        );
        match crate::snapshot::snapshot_readiness(&request.game_dir) {
            crate::snapshot::SnapshotReadiness::Ready => {}
            crate::snapshot::SnapshotReadiness::Pending => {
                return Err(LauncherError::Generic {
                    code: "ERR_SNAPSHOT_PENDING".into(),
                    message: "The instance is still finalizing its initial recovery snapshot. Try again shortly.".into(),
                });
            }
            crate::snapshot::SnapshotReadiness::Failed => {
                return Err(LauncherError::Generic {
                    code: "ERR_SNAPSHOT_FAILED".into(),
                    message: crate::snapshot::snapshot_readiness_error(&request.game_dir)
                        .unwrap_or_else(|| "The initial recovery snapshot failed.".into()),
                });
            }
        }
        progress.phase("checking-health", "Checking instance health");
        let health_started = Instant::now();

        if request.health_policy == HealthPolicy::BlockOnRed {
            let health_dir = request.game_dir.clone();
            let health_manifest = request.manifest.clone();
            let health_registry = request.registry_db.clone();
            let health_scan_token = request.health_scan_token.clone();
            let report = self
                .ctx
                .task_scheduler
                .run_blocking(BlockingPriority::Launch, move || {
                    crate::health::cached_health(
                        &health_dir,
                        &health_manifest,
                        health_registry.as_deref(),
                        health_scan_token.as_deref(),
                    )
                })
                .await
                .map_err(|error| LauncherError::Generic {
                    code: "ERR_HEALTH_TASK".into(),
                    message: format!("Health check task failed: {error}"),
                })?;
            if report.score == crate::health::HealthScore::Red {
                return Err(LauncherError::Generic {
                    code: "ERR_HEALTH_BLOCKED".into(),
                    message: "Health checks found blockers that prevent launch.".into(),
                });
            }
        }
        progress.phase_completed("checking-health", health_started.elapsed().as_millis());

        // Delegated mode is intentionally a small handoff pipeline.  The
        // official launcher owns Java discovery, artifact materialization,
        // native extraction, and command construction; doing that work here
        // only delays the handoff and duplicates the official launcher's work.
        if request.mode == LaunchMode::Delegated {
            progress.phase("snapshot", "Creating the pre-launch snapshot");
            let snapshot_started = Instant::now();
            let snapshot_dir = request.game_dir.clone();
            let snapshot_id = self
                .ctx
                .task_scheduler
                .run_blocking(BlockingPriority::Launch, move || {
                    create_or_reuse_snapshot(&snapshot_dir)
                })
                .await
                .map_err(|error| LauncherError::Generic {
                    code: "ERR_SNAPSHOT_TASK".into(),
                    message: format!("Pre-launch snapshot task failed: {error}"),
                })??;
            progress.phase_completed("snapshot", snapshot_started.elapsed().as_millis());
            let operation_id = _op_handle.id().clone();

            progress.phase("handoff", "Handing off to external launcher");
            progress.handoff(&request.identity)?;
            // Do not publish a session until the external launcher was
            // actually spawned. A snapshot or handoff failure must not make a
            // non-existent session supersede an already monitored launch.
            self.ctx
                .process_session_manager
                .note_latest(&request.instance_id, session_id);
            record_launch_started(&self.ctx, &request.instance_id);

            let process_identity = ProcessIdentity {
                pid: 0,
                start_time: 0,
                expected_exe: None,
            };
            let started = LaunchStarted {
                pid: 0,
                session_id,
                snapshot_id: snapshot_id.clone(),
                java_path: PathBuf::new(),
                process_identity: process_identity.clone(),
            };
            progress.started(&started);
            self.ctx
                .event_sink
                .emit(crate::event_sink::CoreEvent::Launch {
                    operation_id,
                    instance_id: request.instance_id.clone(),
                    status: crate::event_sink::EventStatus::Started,
                    pid: None,
                });

            let result = LaunchResult {
                pid: 0,
                session_id,
                outcome: LaunchOutcome::Unknown,
                snapshot_id,
                java_path: PathBuf::new(),
                process_identity,
            };
            progress.finished(&result);
            _op_handle.complete();
            return Ok(result);
        }

        progress.phase("resolving", "Resolving Minecraft metadata and Java");
        let resolve_started = Instant::now();
        // An explicit Java override is authoritative. Do not scan unrelated
        // system/Mojang runtimes first: on macOS, Java shims can block while
        // waiting for installation or GUI approval even though the override
        // is ready to use.
        let java_candidates = if request.java_override.is_some() {
            Vec::new()
        } else if request.java_candidates.is_empty() {
            let runtimes_root = request.runtimes_root.clone();
            let java_inventory_path = request.java_inventory_path.clone();
            self.ctx
                .task_scheduler
                .run_blocking(BlockingPriority::Launch, move || {
                    cached_java_candidates(&runtimes_root, &java_inventory_path)
                })
                .await
                .map_err(|error| LauncherError::Generic {
                    code: "ERR_JAVA_DISCOVERY".into(),
                    message: format!("Java discovery task failed: {error}"),
                })?
        } else {
            request.java_candidates.clone()
        };

        let loader = loader_info(&request.manifest);
        let resolve_request = || ResolveRequest {
            instance_id: request.instance_id.clone(),
            base_version_id: request.manifest.minecraft_version.clone(),
            loader: loader.clone(),
            game_dir: request.game_dir.clone(),
            assets_dir: request.assets_dir.clone(),
            cache_dir: request.minecraft_root.clone(),
            java_override: request.java_override.clone(),
            java_candidates: java_candidates.clone(),
            network_policy: request.network_policy.clone(),
            allow_incompatible_java_override: request.allow_incompatible_java_override,
            minecraft_dir: Some(request.minecraft_root.clone()),
            receipts_root: Some(request.receipts_root.clone()),
        };

        let resolved = match crate::launch_planner::resolve_cached(resolve_request()).await {
            Ok(plan) => plan,
            Err(LauncherError::JavaRuntimeMissing { major, .. })
                if request.java_runtime_mode == JavaRuntimeMode::Automatic =>
            {
                progress.phase(
                    "provisioning-java",
                    "Provisioning the required Java runtime",
                );
                request
                    .network_policy
                    .check(crate::network::NetworkCategory::JavaRuntime)?;
                let runtime_root = request.runtimes_root.clone();
                let network_policy = request.network_policy.clone();
                let catalog = self.ctx.runtime_catalog.snapshot();
                let lock_manager = self.ctx.lock_manager.clone();
                let ensured = self
                    .ctx
                    .task_scheduler
                    .run_blocking(BlockingPriority::Launch, move || {
                        crate::runtime_manager::ensure_runtime(
                            &runtime_root,
                            major,
                            &catalog,
                            &network_policy,
                            None::<&dyn RuntimeProgress>,
                            Some(&lock_manager),
                        )
                    })
                    .await
                    .map_err(|error| LauncherError::Generic {
                        code: "ERR_JAVA_PROVISION".into(),
                        message: format!("Java provisioning task failed: {error}"),
                    })??;
                let mut refreshed = java_candidates.clone();
                refreshed.push(JavaInstallation {
                    path: ensured.path,
                    version: ensured.version,
                    version_string: ensured.version_string,
                    source: crate::java::JavaSource::Managed,
                    arch: ensured.arch,
                });
                crate::launch_planner::resolve_cached(ResolveRequest {
                    java_candidates: refreshed,
                    ..resolve_request()
                })
                .await?
            }
            Err(error) => return Err(error),
        };
        progress.phase_completed("resolving", resolve_started.elapsed().as_millis());

        // Record this session as the latest for its instance (used by
        // delegated monitoring to detect same-instance replacement without
        // cross-instance interference).
        self.ctx
            .process_session_manager
            .note_latest(&request.instance_id, session_id);

        progress.phase("materializing", "Materializing verified launch artifacts");
        let materialize_started = Instant::now();
        let _materialization_lock = self.ctx.lock_manager.acquire(
            crate::lock_manager::LockResource::Materialization,
            "launch-materialize",
        )?;
        let materialized = crate::launch_planner::materialize(resolved).await?;
        progress.phase_completed("materializing", materialize_started.elapsed().as_millis());
        let java_path = materialized.resolved.java.path.clone();
        let gc_args = crate::gc::compute_gc(
            materialized.resolved.java.major_version,
            request.jvm_memory_mb,
            &request.jvm_custom_args,
            request.jvm_gc_profile,
        );
        let gc_args = crate::gc::apply_pre_touch(&gc_args.jvm_args, request.jvm_always_pre_touch);
        let user_jvm_args = crate::launch_planner::parse_argument_string(&gc_args)?;
        let prepared = crate::launch_planner::build_command(BuildCommandRequest {
            plan: &materialized,
            identity: &request.identity,
            features: &LaunchFeatures::default(),
            user_jvm_args: &user_jvm_args,
            extra_game_args: &request.extra_game_args,
        })?;

        progress.phase("snapshot", "Creating the pre-launch snapshot");
        let snapshot_started = Instant::now();
        let snapshot_dir = request.game_dir.clone();
        let snapshot_id = self
            .ctx
            .task_scheduler
            .run_blocking(BlockingPriority::Launch, move || {
                create_or_reuse_snapshot(&snapshot_dir)
            })
            .await
            .map_err(|error| LauncherError::Generic {
                code: "ERR_SNAPSHOT_TASK".into(),
                message: format!("Pre-launch snapshot task failed: {error}"),
            })??;
        progress.phase_completed("snapshot", snapshot_started.elapsed().as_millis());
        let operation_id = _op_handle.id().clone();

        // -- Direct mode: spawn Java and attach --
        progress.phase("launching", "Starting Minecraft");
        let child = crate::launch_planner::spawn(&prepared)?;
        let pid = child.id().ok_or_else(|| LauncherError::Generic {
            code: "ERR_NO_PID".into(),
            message: "Spawned process has no PID.".into(),
        })?;
        let process_identity = crate::process_identity::capture(pid)?;
        record_launch_started(&self.ctx, &request.instance_id);

        // Register the session with the core-owned process session manager.
        // Non-fatal: a duplicate registration should never happen in practice
        // and we continue regardless.
        let _ = self.ctx.process_session_manager.register(ProcessSession {
            instance_id: request.instance_id.clone(),
            session_id,
            pid,
            process_identity: process_identity.clone(),
            snapshot_id: snapshot_id.clone(),
            start_time: std::time::SystemTime::now(),
            attached: true,
            user_cancelled: false,
        });

        let started = LaunchStarted {
            pid,
            session_id,
            snapshot_id: snapshot_id.clone(),
            java_path: java_path.clone(),
            process_identity: process_identity.clone(),
        };
        progress.started(&started);
        self.ctx
            .event_sink
            .emit(crate::event_sink::CoreEvent::Launch {
                operation_id: operation_id.clone(),
                instance_id: request.instance_id.clone(),
                status: crate::event_sink::EventStatus::Started,
                pid: Some(pid),
            });

        progress.phase("running", "Waiting for Minecraft to exit");
        let secret = request.identity.access_token.as_str();
        let output_progress = |stream: &str, line: &str| progress.log(stream, line);
        let outcome = crate::launch_planner::wait_and_classify_with_progress(
            child,
            &request.game_dir,
            &[secret],
            Some(&output_progress),
        )
        .await
        .inspect_err(|_| {
            self.ctx.process_session_manager.remove(session_id);
        })?;

        // The game already ran to completion, so a failed LKG write must not
        // retroactively fail the launch: that would skip `finished` and leave
        // the adapter's running-process state set. Log and continue, matching
        // the delegated path.
        if let Err(error) = crate::lkg::record_launch_outcome(
            &request.game_dir,
            Some(&snapshot_id),
            &format!("session-{session_id}"),
            outcome.clone(),
        ) {
            eprintln!(
                "[launch] could not record launch outcome for {}: {error}",
                request.instance_id
            );
        }
        // Retention is core-owned and runs for both modes: a direct launch
        // promotes its pre-launch snapshot exactly like a delegated one, so it
        // must prune under the same policy. Running it here also covers the
        // CLI, which has no adapter-side retention of its own.
        if let Err(error) = crate::lkg::run_retention(&request.game_dir) {
            eprintln!(
                "[launch] snapshot retention failed after launch for {}: {error}",
                request.instance_id
            );
        }
        if outcome == LaunchOutcome::Success {
            let _ = crate::runtime_manager::mark_successful_use(&request.runtimes_root, &java_path);
        }

        // Session completed successfully — remove from manager.
        self.ctx.process_session_manager.remove(session_id);

        self.ctx
            .event_sink
            .emit(crate::event_sink::CoreEvent::Launch {
                operation_id,
                instance_id: request.instance_id,
                status: crate::event_sink::EventStatus::Completed,
                pid: Some(pid),
            });
        _op_handle.complete();

        let result = LaunchResult {
            pid,
            session_id,
            outcome,
            snapshot_id,
            java_path,
            process_identity,
        };
        progress.finished(&result);
        Ok(result)
    }

    /// Return all currently running process sessions.
    pub fn running_processes(&self) -> Vec<ProcessSession> {
        self.ctx.process_session_manager.list()
    }

    /// Monitor a delegated launch by polling for crash reports, log markers,
    /// and staleness (per-instance, never global). Unless the monitor was
    /// superseded by a newer session for the same instance, it then records
    /// the LKG outcome and runs snapshot retention. The desktop adapter only
    /// needs to emit the Tauri event from the return value.
    ///
    /// Staleness is checked against the instance's latest session in
    /// [`ProcessSessionManager`]: a newer launch for a *different* instance
    /// does NOT end monitoring; only a same-instance replacement does.
    pub async fn wait_delegated(
        ctx: &Ctx,
        instance_id: &str,
        game_dir: &Path,
        snapshot_id: &str,
        session_id: u64,
        launched_at: SystemTime,
    ) -> LaunchOutcome {
        const MAX_CAPTURED_LAUNCH_LOG_BYTES: usize = 1_048_576;
        let started = std::time::Instant::now();

        let mut superseded = false;
        let outcome = loop {
            // Per-instance staleness: only a newer session for the SAME
            // instance ends this monitor. Different instances are independent.
            if !ctx
                .process_session_manager
                .is_latest_for_instance(instance_id, session_id)
            {
                superseded = true;
                break LaunchOutcome::Unknown;
            }

            // Crash report check
            let crash_dir = game_dir.join("crash-reports");
            let has_crash = std::fs::read_dir(&crash_dir)
                .ok()
                .map(|entries| {
                    entries.flatten().any(|entry| {
                        entry
                            .metadata()
                            .ok()
                            .filter(|m| m.is_file())
                            .and_then(|m| m.modified().ok())
                            .map(|modified| modified >= launched_at)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            if has_crash {
                break LaunchOutcome::Crash;
            }

            // Log tail triage
            let log_path = game_dir.join("logs").join("latest.log");
            let log_tail = std::fs::metadata(&log_path)
                .ok()
                .filter(|m| m.modified().ok().is_some_and(|t| t >= launched_at))
                .and_then(|metadata| {
                    let mut file = std::fs::File::open(&log_path).ok()?;
                    let keep = metadata.len().min(MAX_CAPTURED_LAUNCH_LOG_BYTES as u64);
                    file.seek(SeekFrom::End(-(keep as i64))).ok()?;
                    let mut bytes = Vec::with_capacity(keep as usize);
                    file.read_to_end(&mut bytes).ok()?;
                    Some(String::from_utf8_lossy(&bytes).into_owned())
                });

            if let Some(ref log) = log_tail {
                if crate::crash_diagnostics::triage(log).matched {
                    break LaunchOutcome::Crash;
                }
                // The delegated launcher does not expose the game PID or exit code.
                // A clean-shutdown log marker is the only safe success signal.
                if log.lines().any(|line| line.contains("Stopping!")) {
                    break crate::lkg::classify_launch(&crate::lkg::LaunchEvents {
                        exit_code: Some(0),
                        runtime_ms: started.elapsed().as_millis() as u64,
                        was_user_cancelled: false,
                        crash_report_found: false,
                        log_crash_signature_matched: false,
                    });
                }
            }

            if started.elapsed() >= std::time::Duration::from_secs(12 * 60 * 60) {
                break LaunchOutcome::Unknown;
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        };

        // A newer session for this instance already owns the LKG state.
        // Recording here would overwrite that session's outcome with this
        // stale monitor's `Unknown`.
        if superseded {
            return outcome;
        }

        // Record LKG outcome and run retention — core-owned, not desktop.
        let _ = crate::lkg::record_launch_outcome(
            game_dir,
            Some(snapshot_id),
            &format!("delegated-{session_id}"),
            outcome.clone(),
        );
        let _ = crate::lkg::run_retention(game_dir);

        outcome
    }
}

fn cached_java_candidates(
    runtimes_root: &Path,
    java_inventory_path: &Path,
) -> Vec<JavaInstallation> {
    let minecraft_dir = crate::paths::minecraft_dir();
    let key = format!(
        "{}|{}",
        runtimes_root.display(),
        minecraft_dir
            .as_deref()
            .map(|path| path.to_string_lossy())
            .unwrap_or_default()
    );
    if let Ok(cache) = JAVA_DISCOVERY_CACHE.lock() {
        if let Some((fetched_at, candidates)) = cache.get(&key) {
            if fetched_at.elapsed() < JAVA_DISCOVERY_CACHE_TTL {
                eprintln!(
                    "[launch-timing] java-discovery: in-memory cache hit ({} candidates)",
                    candidates.len()
                );
                return candidates.clone();
            }
        }
    }
    // The persistent inventory is validated against executable metadata and
    // runtime-root directory state, so warm launches reuse the last discovery
    // without spawning `java -version` probes for every JRE on the machine.
    let discovery_started = Instant::now();
    let candidates = crate::java::java_candidates_cached(
        java_inventory_path,
        runtimes_root,
        minecraft_dir.as_deref(),
    );
    eprintln!(
        "[launch-timing] java-discovery: {} candidates in {} ms",
        candidates.len(),
        discovery_started.elapsed().as_millis()
    );
    if let Ok(mut cache) = JAVA_DISCOVERY_CACHE.lock() {
        cache.insert(key, (Instant::now(), candidates.clone()));
    }
    candidates
}

fn record_launch_started(ctx: &Ctx, instance_id: &str) {
    let result = crate::db::local_state_connection(&ctx.paths.local_state_db()).and_then(|conn| {
        crate::db::touch_last_launched(&conn, instance_id, &chrono::Utc::now().to_rfc3339())
    });
    if let Err(error) = result {
        eprintln!("[launch] could not record launch start for {instance_id}: {error}");
    }
}

fn validate_instance_id(instance_id: &str) -> LauncherResult<()> {
    crate::app_paths::validate_path_component(instance_id)
}

fn loader_info(manifest: &InstanceManifest) -> Option<LoaderInfo> {
    if matches!(manifest.loader.as_str(), "" | "vanilla") {
        None
    } else {
        Some(LoaderInfo {
            loader_type: manifest.loader.clone(),
            version: manifest.loader_version.clone(),
            version_url: String::new(),
        })
    }
}

fn create_or_reuse_snapshot(instance_dir: &Path) -> LauncherResult<String> {
    let started = Instant::now();
    let lkg = crate::lkg::read_lkg_state(instance_dir).map_err(|error| LauncherError::Generic {
        code: "ERR_LKG_READ".into(),
        message: error.to_string(),
    })?;
    // Pre-launch snapshots cover the mod/config/layout state that causes
    // launch failures; world data (`saves/`) is handled by install/import and
    // explicit backup snapshots instead of being re-walked and re-hashed on
    // every launch.
    let scope = crate::snapshot::prelaunch_tracked_entries();
    let Some(snapshot_id) = lkg.current_lkg_snapshot_id else {
        let snapshot_id = create_fresh_prelaunch_snapshot(instance_dir, scope)?;
        eprintln!(
            "[launch-timing] snapshot: no LKG snapshot, created fresh in {} ms",
            started.elapsed().as_millis()
        );
        return Ok(snapshot_id);
    };

    // O(1) reuse: the mutation journal is untouched since the snapshot was
    // verified, so no filesystem walk is needed to prove the state is current.
    if crate::snapshot::prelaunch_snapshot_is_reusable(instance_dir, &snapshot_id) {
        eprintln!(
            "[launch-timing] snapshot: O(1) journal reuse in {} ms",
            started.elapsed().as_millis()
        );
        return Ok(snapshot_id);
    }
    eprintln!(
        "[launch-timing] snapshot: journal mismatch/missing, falling back to fingerprint verification"
    );

    let fingerprint_started = Instant::now();
    let metadata_fingerprint =
        crate::snapshot::live_metadata_fingerprint_scoped(instance_dir, scope).ok();
    eprintln!(
        "[launch-timing] snapshot: metadata fingerprint walk took {} ms",
        fingerprint_started.elapsed().as_millis()
    );
    if metadata_fingerprint.as_deref().is_some_and(|fingerprint| {
        crate::snapshot::read_snapshot_metadata_fingerprint_scoped(
            instance_dir,
            &snapshot_id,
            scope,
        )
        .as_deref()
            == Some(fingerprint)
    }) {
        if let Some(fingerprint) = metadata_fingerprint.as_deref() {
            let _ = crate::snapshot::write_snapshot_metadata_fingerprint_scoped(
                instance_dir,
                &snapshot_id,
                fingerprint,
                scope,
            );
        }
        eprintln!(
            "[launch-timing] snapshot: fingerprint verified reuse in {} ms",
            started.elapsed().as_millis()
        );
        return Ok(snapshot_id);
    }

    // Legacy snapshots and receipts written before this optimization still
    // receive the exact content comparison once.  A successful comparison
    // upgrades the snapshot with a metadata receipt for future launches.
    let incremental_started = Instant::now();
    if crate::snapshot::snapshot_matches_live_incremental_scoped(instance_dir, &snapshot_id, scope)
        .unwrap_or(false)
    {
        if let Some(fingerprint) = metadata_fingerprint.as_deref() {
            let _ = crate::snapshot::write_snapshot_metadata_fingerprint_scoped(
                instance_dir,
                &snapshot_id,
                fingerprint,
                scope,
            );
        }
        eprintln!(
            "[launch-timing] snapshot: incremental verified reuse in {} ms",
            incremental_started.elapsed().as_millis()
        );
        return Ok(snapshot_id);
    }

    let create_started = Instant::now();
    let snapshot_id = create_fresh_prelaunch_snapshot(instance_dir, scope)?;
    eprintln!(
        "[launch-timing] snapshot: created new snapshot in {} ms (total {} ms)",
        create_started.elapsed().as_millis(),
        started.elapsed().as_millis()
    );
    Ok(snapshot_id)
}

fn create_fresh_prelaunch_snapshot(instance_dir: &Path, scope: &[&str]) -> LauncherResult<String> {
    let snapshot = crate::snapshot::create_snapshot_scoped(instance_dir, Some("pre-launch"), scope)
        .map_err(|error| LauncherError::Generic {
            code: "ERR_SNAPSHOT_CREATE".into(),
            message: error.to_string(),
        })?;
    // create_snapshot_scoped writes the scoped reuse receipt from the same
    // metadata traversal that created the snapshot.
    Ok(snapshot.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal_instance_ids() {
        assert!(validate_instance_id("../outside").is_err());
        assert!(validate_instance_id("safe-instance").is_ok());
    }

    #[test]
    fn session_ids_are_monotonic() {
        let first = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let second = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        assert!(second > first);
    }

    #[test]
    fn validate_instance_id_rejects_empty() {
        assert!(validate_instance_id("").is_err());
    }

    #[test]
    fn launch_recovery_action_serde_roundtrip() {
        for action in &[
            LaunchRecoveryAction::None,
            LaunchRecoveryAction::ProvisionJava { major: 21 },
            LaunchRecoveryAction::RepairLoader,
            LaunchRecoveryAction::SwitchLoader {
                target_version: "0.19.0".into(),
            },
        ] {
            let json = serde_json::to_string(action).unwrap();
            let back: LaunchRecoveryAction = serde_json::from_str(&json).unwrap();
            assert_eq!(*action, back);
        }
    }

    #[test]
    fn launch_recovery_action_serde_matches_frontend_discriminated_shape() {
        // The desktop frontend sends and receives `{type: ...}` payloads;
        // the exact serialized forms below are the contract at the IPC
        // boundary.
        assert_eq!(
            serde_json::to_string(&LaunchRecoveryAction::None).unwrap(),
            r#"{"type":"None"}"#
        );
        assert_eq!(
            serde_json::to_string(&LaunchRecoveryAction::ProvisionJava { major: 21 }).unwrap(),
            r#"{"type":"ProvisionJava","major":21}"#
        );
        assert_eq!(
            serde_json::to_string(&LaunchRecoveryAction::RepairLoader).unwrap(),
            r#"{"type":"RepairLoader"}"#
        );
        assert_eq!(
            serde_json::to_string(&LaunchRecoveryAction::SwitchLoader {
                target_version: "0.19.0".into(),
            })
            .unwrap(),
            r#"{"type":"SwitchLoader","target_version":"0.19.0"}"#
        );
    }

    #[test]
    fn launch_recovery_action_none_is_noop() {
        // None should serialize/deserialize without error
        let json = serde_json::to_string(&LaunchRecoveryAction::None).unwrap();
        assert_eq!(json, r#"{"type":"None"}"#);
        let back: LaunchRecoveryAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, LaunchRecoveryAction::None);
    }

    #[test]
    fn launch_recovery_action_provision_java_carries_major() {
        let action = LaunchRecoveryAction::ProvisionJava { major: 17 };
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, r#"{"type":"ProvisionJava","major":17}"#);
        let back: LaunchRecoveryAction = serde_json::from_str(&json).unwrap();
        match back {
            LaunchRecoveryAction::ProvisionJava { major } => assert_eq!(major, 17),
            _ => panic!("expected ProvisionJava"),
        }
    }

    #[test]
    fn launch_recovery_action_repair_loader_roundtrips() {
        let json = serde_json::to_string(&LaunchRecoveryAction::RepairLoader).unwrap();
        assert_eq!(json, r#"{"type":"RepairLoader"}"#);
        let back: LaunchRecoveryAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, LaunchRecoveryAction::RepairLoader);
    }

    #[test]
    fn launch_recovery_action_switch_loader_roundtrips() {
        let action = LaunchRecoveryAction::SwitchLoader {
            target_version: "0.19.0".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, r#"{"type":"SwitchLoader","target_version":"0.19.0"}"#);
        let back: LaunchRecoveryAction = serde_json::from_str(&json).unwrap();
        match back {
            LaunchRecoveryAction::SwitchLoader { target_version } => {
                assert_eq!(target_version, "0.19.0")
            }
            _ => panic!("expected SwitchLoader"),
        }
    }

    #[test]
    fn prelaunch_snapshot_excludes_saves_and_reuses_in_o1() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path().join("instance");
        std::fs::create_dir_all(inst.join("mods")).unwrap();
        std::fs::write(inst.join("mods").join("test.jar"), b"mod content").unwrap();
        std::fs::create_dir_all(inst.join("saves").join("world1")).unwrap();
        std::fs::write(
            inst.join("saves").join("world1").join("level.dat"),
            b"world data",
        )
        .unwrap();
        std::fs::write(inst.join("options.txt"), b"render_distance=12").unwrap();
        std::fs::write(inst.join("instance_manifest.json"), b"{}").unwrap();

        let first = create_or_reuse_snapshot(&inst).unwrap();
        let index = crate::snapshot::snapshot_file_index(&inst, &first).unwrap();
        assert!(index.iter().all(|entry| !entry.path.starts_with("saves/")));

        // Promote it as the LKG so the next launch takes the reuse path.
        let lkg = crate::lkg::LkgState {
            current_lkg_snapshot_id: Some(first.clone()),
            ..Default::default()
        };
        std::fs::write(inst.join("lkg.json"), serde_json::to_vec(&lkg).unwrap()).unwrap();

        let second = create_or_reuse_snapshot(&inst).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            crate::snapshot::list_snapshots(&inst).unwrap().len(),
            1,
            "an unchanged instance must reuse its snapshot instead of creating a new one"
        );

        // A launcher-driven mutation invalidates journal reuse but the
        // fingerprint verification path still reuses the same snapshot.
        crate::snapshot::mark_instance_mutated(&inst).unwrap();
        let third = create_or_reuse_snapshot(&inst).unwrap();
        assert_eq!(first, third);
        assert_eq!(crate::snapshot::list_snapshots(&inst).unwrap().len(), 1);
    }
}
