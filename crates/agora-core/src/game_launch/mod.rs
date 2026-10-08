//! Generic game launch recipe resolution, launch preparation, process execution,
//! and process watching (MASTER_SPEC §26.3, §26.4, §26.13).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agora_game_api::{
    check_runtime_files, describe_runtime_findings, GameDefinition, GamePath, LaunchRecipe,
    LaunchValue, RuntimeFileFinding, UserDataLocation,
};
use serde::{Deserialize, Serialize};

use crate::game_base::{verify_base, BaseManifest, BaseProblem, VerifyDepth};
use crate::game_load_order::{describe_findings, Finding};
use crate::process_identity::{self, ProcessIdentity};

mod vfs;
pub use vfs::{
    launch_under_vfs, locate_dll, log_len, processes_ended_by_vfs, processes_ended_since,
    EndedProcess, VfsLaunch,
};

/// Host-resolved roots for recipe resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRoots {
    pub runtime: PathBuf,
    pub install: Option<PathBuf>,
    pub base: Option<PathBuf>,
}

/// A fully resolved launch configuration ready to be executed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLaunch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: BTreeMap<String, OsString>,
    pub cwd: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("unsupported launch root: {0}")]
    UnsupportedRoot(String),
    #[error("program missing or not a file: {path}")]
    ProgramMissing { path: PathBuf },
    #[error(
        "the launch recipe's executable must be inside the game's runtime, base or install folder"
    )]
    ExecutableOutsideGame,
    #[error("game definition has no launch recipe")]
    NoRecipe,
    #[error("base is damaged ({} problem(s))", problems.len())]
    BaseDamaged { problems: Vec<BaseProblem> },
    /// A framework in the game's files was built for another runtime version (MASTER_SPEC §26.6).
    #[error("frameworks do not match this game's version: {}", describe_runtime_findings(.findings))]
    RuntimeMismatch { findings: Vec<RuntimeFileFinding> },
    /// The plugin list breaks the game's load order rules (MASTER_SPEC §26.6): the game would crash
    /// or refuse to load. `agora games instance plugins sort <instance>` fixes a master's order.
    #[error(
        "the plugin load order would stop the game: {}. `agora games instance plugins sort <instance>` fixes master order",
        describe_findings(.findings)
    )]
    LoadOrderProblems { findings: Vec<Finding> },
    #[error("root '{0}' is not configured")]
    RootNotConfigured(String),
    #[error("user data location '{0:?}' could not be resolved")]
    UserDataNotFound(UserDataLocation),
    #[error("process capture failed: {0}")]
    ProcessCapture(String),
    /// The game was not started: the virtual file system could not be put under it.
    #[error("the virtual file system could not start: {reason}")]
    VfsUnavailable { reason: String },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A prepared launch from a base with warnings (if launched anyway).
#[derive(Debug, Clone)]
pub struct PreparedLaunch {
    pub resolved: ResolvedLaunch,
    pub warnings: Vec<BaseProblem>,
    pub deploy_outcome: Option<crate::game_deploy::DeployOutcome>,
    /// How the instance was deployed, when it was.
    pub deployment: Option<crate::game_deploy::DeployMode>,
    /// The user chose `deployment` (for the instance or for this launch), so it is never
    /// replaced by a fallback.
    pub deployment_chosen: bool,
    /// Set when the game is to run under the virtual file system.
    pub vfs: Option<VfsLaunch>,
    /// Something the user should be told about how this launch will run, such as a step down
    /// from the virtual file system.
    pub notice: Option<String>,
    /// The launch alternative that replaced the recipe's executable, if one did.
    pub alternative: Option<AppliedAlternative>,
    /// Framework findings that `launch_anyway` let through. Empty when the launch was clean.
    pub runtime_findings: Vec<RuntimeFileFinding>,
    /// Load order findings to show the user: the warnings always, and the refusing findings too
    /// when `launch_anyway` let them through. Empty when the load order is clean.
    pub load_order_findings: Vec<Finding>,
}

impl PreparedLaunch {
    /// A launch with no deployment: the game runs from the folder the recipe names.
    pub fn undeployed(
        resolved: ResolvedLaunch,
        warnings: Vec<BaseProblem>,
        alternative: Option<AppliedAlternative>,
    ) -> Self {
        Self {
            resolved,
            warnings,
            deploy_outcome: None,
            deployment: None,
            deployment_chosen: false,
            vfs: None,
            notice: None,
            alternative,
            runtime_findings: Vec::new(),
            load_order_findings: Vec::new(),
        }
    }
}

/// Refuse a launch whose plugin list breaks the game's load order rules, unless `launch_anyway` is
/// set. Only the refusing findings refuse (a missing or inactive master, a master loop, too many
/// plugins); the others (a late master, an unreadable header, a plugin listed twice) come back as
/// warnings.
/// `findings` come from [`crate::game_load_order::check`] after the plugin list is synced.
pub fn refuse_load_order(
    findings: Vec<Finding>,
    launch_anyway: bool,
) -> Result<Vec<Finding>, LaunchError> {
    let refuses = findings.iter().any(Finding::refuses_launch);
    if refuses && !launch_anyway {
        Err(LaunchError::LoadOrderProblems { findings })
    } else {
        Ok(findings)
    }
}

/// Refuse a launch that has a framework built for another version, unless `launch_anyway` is set.
/// Any other finding (a rule that cannot be checked) never refuses: it comes back as a warning.
/// `findings` come from [`check_runtime_files`] over the files the game will see.
pub fn refuse_runtime_mismatch(
    findings: Vec<RuntimeFileFinding>,
    launch_anyway: bool,
) -> Result<Vec<RuntimeFileFinding>, LaunchError> {
    let refuses = findings.iter().any(RuntimeFileFinding::refuses_launch);
    if refuses && !launch_anyway {
        Err(LaunchError::RuntimeMismatch { findings })
    } else {
        Ok(findings)
    }
}

/// The runtime-file findings for a runtime at `version` over `paths` (the files the game will
/// see). One function for launch and for `games instance check`, so they cannot disagree.
pub fn runtime_findings<S: AsRef<str>>(
    definition: &GameDefinition,
    version: &str,
    paths: &[S],
) -> Vec<RuntimeFileFinding> {
    check_runtime_files(&definition.runtime_files, version, paths)
}

/// A launch alternative (a framework's loader) that was used instead of the recipe's own
/// executable, and the reason the game definition gives for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedAlternative {
    pub id: String,
    pub reason: String,
}

/// How a prepared launch is started. The real implementation is [`SystemLauncher`]; tests
/// substitute their own so no process has to be injected.
pub trait Launcher {
    /// Find `agora_vfs.dll`, or say why it cannot be used.
    fn locate_vfs_dll(&self) -> Result<PathBuf, String>;
    /// Start the game: under the virtual file system when `prepared.vfs` is set.
    fn launch(&self, prepared: &PreparedLaunch) -> Result<LaunchedGame, LaunchError>;
}

/// Starts real processes.
pub struct SystemLauncher;

impl Launcher for SystemLauncher {
    fn locate_vfs_dll(&self) -> Result<PathBuf, String> {
        locate_dll()
    }

    fn launch(&self, prepared: &PreparedLaunch) -> Result<LaunchedGame, LaunchError> {
        launch(prepared)
    }
}

/// A spawned game process and its captured OS identity.
pub struct LaunchedGame {
    pub child: std::process::Child,
    pub identity: ProcessIdentity,
    pub program: PathBuf,
}

impl LaunchedGame {
    pub fn pid(&self) -> u32 {
        self.identity.pid
    }
}

/// A running process whose executable lies inside a target directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunningGameProcess {
    pub pid: u32,
    pub exe: PathBuf,
}

/// Exit and process monitoring summary for a game launch session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionExitReport {
    pub processes: Vec<RunningGameProcess>,
    pub relaunched_outside: bool,
}

// ---------------------------------------------------------------------------
// Path and recipe resolution
// ---------------------------------------------------------------------------

/// Join a `/`-separated relative path one component at a time, so the result
/// uses the platform's separator throughout.
fn join_rel(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn resolve_game_path(path: &GamePath, roots: &LaunchRoots) -> Result<PathBuf, LaunchError> {
    match path {
        GamePath::Runtime { path } => {
            let rel = path.as_str();
            if rel.is_empty() {
                Ok(roots.runtime.clone())
            } else {
                Ok(join_rel(&roots.runtime, rel))
            }
        }
        GamePath::Base { path, .. } => {
            let Some(base_root) = &roots.base else {
                return Err(LaunchError::RootNotConfigured("base".to_string()));
            };
            let rel = path.as_str();
            if rel.is_empty() {
                Ok(base_root.clone())
            } else {
                Ok(join_rel(base_root, rel))
            }
        }
        GamePath::Install { path, .. } => {
            let Some(install_root) = &roots.install else {
                return Err(LaunchError::RootNotConfigured("install".to_string()));
            };
            let rel = path.as_str();
            if rel.is_empty() {
                Ok(install_root.clone())
            } else {
                Ok(join_rel(install_root, rel))
            }
        }
        GamePath::UserData { location, path } => {
            let Some(base) = crate::game_user_files::user_data_root(location) else {
                return Err(LaunchError::UserDataNotFound(location.clone()));
            };
            let rel = path.as_str();
            if rel.is_empty() {
                Ok(base)
            } else {
                Ok(join_rel(&base, rel))
            }
        }
        GamePath::Instance { .. } => Err(LaunchError::UnsupportedRoot("instance".to_string())),
        GamePath::Layer { .. } => Err(LaunchError::UnsupportedRoot("layer".to_string())),
        GamePath::RuntimeComponent { .. } => Err(LaunchError::UnsupportedRoot(
            "runtime_component".to_string(),
        )),
        GamePath::Artifact { .. } => Err(LaunchError::UnsupportedRoot("artifact".to_string())),
    }
}

fn resolve_launch_value(val: &LaunchValue, roots: &LaunchRoots) -> Result<OsString, LaunchError> {
    match val {
        LaunchValue::Literal { value } => Ok(OsString::from(value)),
        LaunchValue::Path {
            path,
            prefix,
            suffix,
        } => {
            let resolved = resolve_game_path(path, roots)?;
            let mut s = OsString::from(prefix);
            s.push(resolved.as_os_str());
            s.push(suffix);
            Ok(s)
        }
        LaunchValue::PathList { paths, prefix } => {
            #[cfg(windows)]
            const SEP: &str = ";";
            #[cfg(not(windows))]
            const SEP: &str = ":";

            let mut s = OsString::from(prefix);
            for (i, p) in paths.iter().enumerate() {
                let resolved = resolve_game_path(p, roots)?;
                if i > 0 {
                    s.push(SEP);
                }
                s.push(resolved.as_os_str());
            }
            Ok(s)
        }
    }
}

/// Resolve a declarative launch recipe against available roots.
pub fn resolve_recipe(
    recipe: &LaunchRecipe,
    roots: &LaunchRoots,
) -> Result<ResolvedLaunch, LaunchError> {
    // A game's executable lives in the game's own folders. A definition comes
    // from a package (possibly a community plugin holding `game:define`), and
    // consenting to "define a game" is not consenting to run any program in
    // the user's Documents or AppData.
    if !matches!(
        recipe.executable,
        GamePath::Runtime { .. } | GamePath::Base { .. } | GamePath::Install { .. }
    ) {
        return Err(LaunchError::ExecutableOutsideGame);
    }
    let program = resolve_game_path(&recipe.executable, roots)?;
    if !program.is_file() {
        return Err(LaunchError::ProgramMissing { path: program });
    }

    let cwd = resolve_game_path(&recipe.working_directory, roots)?;

    let mut args = Vec::with_capacity(recipe.arguments.len());
    for arg in &recipe.arguments {
        args.push(resolve_launch_value(arg, roots)?);
    }

    let mut env = BTreeMap::new();
    for (k, v) in &recipe.environment {
        env.insert(k.clone(), resolve_launch_value(v, roots)?);
    }

    Ok(ResolvedLaunch {
        program,
        args,
        env,
        cwd,
    })
}

/// Replace the recipe's executable with the first launch alternative of the game whose
/// `when_present` file exists in the runtime root (a framework's loader, for example).
///
/// Arguments, environment and working directory stay the recipe's, and the executable rule of
/// [`resolve_recipe`] still applies: the program must be a file inside the runtime, base or
/// install folder. An alternative that matches but cannot be started is an error, never a
/// quiet fall back to the plain executable. Callers skip this entirely for a plain launch.
pub fn apply_launch_alternative(
    definition: &GameDefinition,
    roots: &LaunchRoots,
    resolved: &mut ResolvedLaunch,
) -> Result<Option<AppliedAlternative>, LaunchError> {
    for alternative in &definition.launch_alternatives {
        let marker = join_rel(&roots.runtime, alternative.when_present.as_str());
        if !marker.is_file() {
            continue;
        }
        if !matches!(
            alternative.executable,
            GamePath::Runtime { .. } | GamePath::Base { .. } | GamePath::Install { .. }
        ) {
            return Err(LaunchError::ExecutableOutsideGame);
        }
        let program = resolve_game_path(&alternative.executable, roots)?;
        if !program.is_file() {
            return Err(LaunchError::ProgramMissing { path: program });
        }
        resolved.program = program;
        return Ok(Some(AppliedAlternative {
            id: alternative.id.clone(),
            reason: alternative.reason.clone(),
        }));
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Base launch preparation
// ---------------------------------------------------------------------------

/// Prepare a launch from a pinned base, verifying base integrity and setting up store environment.
pub fn prepare_base_launch(
    manifest: &BaseManifest,
    definition: &GameDefinition,
    launch_anyway: bool,
) -> Result<PreparedLaunch, LaunchError> {
    prepare_base_launch_with(manifest, definition, launch_anyway, false)
}

/// Like [`prepare_base_launch`]; `plain` skips the game's launch alternatives and starts
/// the recipe's own executable.
pub fn prepare_base_launch_with(
    manifest: &BaseManifest,
    definition: &GameDefinition,
    launch_anyway: bool,
    plain: bool,
) -> Result<PreparedLaunch, LaunchError> {
    let Some(recipe) = &definition.launch else {
        return Err(LaunchError::NoRecipe);
    };

    let ver = verify_base(
        manifest,
        VerifyDepth::Quick,
        &|p| definition.is_declared_write(p),
        &|p| definition.is_excluded(p),
    );
    if !ver.problems.is_empty() && !launch_anyway {
        return Err(LaunchError::BaseDamaged {
            problems: ver.problems,
        });
    }
    let warnings = ver.problems;

    let base_paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    let findings = refuse_runtime_mismatch(
        runtime_findings(definition, &manifest.runtime.version, &base_paths),
        launch_anyway,
    )?;

    let roots = LaunchRoots {
        runtime: manifest.location.clone(),
        install: Some(manifest.source_location.clone()),
        base: Some(manifest.location.clone()),
    };

    let mut resolved = resolve_recipe(recipe, &roots)?;
    let alternative = if plain {
        None
    } else {
        apply_launch_alternative(definition, &roots, &mut resolved)?
    };

    // Store launch environment:
    // for runtime.store == "steam", set SteamAppId and SteamGameId to the store product.
    if manifest.runtime.store.as_str() == "steam" {
        let product = manifest.source_product.as_deref().or_else(|| {
            definition
                .stores
                .iter()
                .find(|s| s.store == manifest.runtime.store)
                .map(|s| s.product.as_str())
        });
        if let Some(prod) = product {
            resolved
                .env
                .insert("SteamAppId".to_string(), OsString::from(prod));
            resolved
                .env
                .insert("SteamGameId".to_string(), OsString::from(prod));
        }
    }

    let mut prepared = PreparedLaunch::undeployed(resolved, warnings, alternative);
    prepared.runtime_findings = findings;
    Ok(prepared)
}

// ---------------------------------------------------------------------------
// Spawning and process watching
// ---------------------------------------------------------------------------

/// Spawn the game process according to a prepared launch, under the virtual file system when the
/// preparation asked for it.
pub fn launch(prepared: &PreparedLaunch) -> Result<LaunchedGame, LaunchError> {
    if let Some(vfs) = &prepared.vfs {
        return launch_under_vfs(&prepared.resolved, vfs);
    }
    let mut cmd = std::process::Command::new(&prepared.resolved.program);
    cmd.args(&prepared.resolved.args);
    cmd.current_dir(&prepared.resolved.cwd);
    cmd.envs(&prepared.resolved.env);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    let child = cmd.spawn().map_err(LaunchError::Io)?;
    let pid = child.id();
    let identity =
        process_identity::capture(pid).map_err(|e| LaunchError::ProcessCapture(format!("{e}")))?;

    Ok(LaunchedGame {
        child,
        identity,
        program: prepared.resolved.program.clone(),
    })
}

fn clean_path_for_comparison(p: &Path) -> PathBuf {
    let canon = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let s = canon.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        canon
    }
}

/// Check whether `path` is inside `dir` (canonicalised, case-insensitive on Windows).
pub fn is_subpath(path: &Path, dir: &Path) -> bool {
    let norm_path = clean_path_for_comparison(path);
    let norm_dir = clean_path_for_comparison(dir);

    #[cfg(windows)]
    {
        let p_str = norm_path
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        let mut d_str = norm_dir
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        if !d_str.ends_with('/') {
            d_str.push('/');
        }
        p_str.starts_with(&d_str)
    }
    #[cfg(not(windows))]
    {
        norm_path.starts_with(&norm_dir)
    }
}

/// List all running processes whose executable path lies inside `dir`.
pub fn processes_running_from(dir: &Path) -> Vec<RunningGameProcess> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    let mut running = Vec::new();
    for (pid, process) in sys.processes() {
        if let Some(exe) = process.exe() {
            if is_subpath(exe, dir) {
                running.push(RunningGameProcess {
                    pid: pid.as_u32(),
                    exe: exe.to_path_buf(),
                });
            }
        }
    }
    running.sort_by_key(|p| p.pid);
    running
}

/// Wait for a launched game to exit and ensure no processes are running from `dir` for `grace`.
pub fn wait_for_exit(
    dir: &Path,
    launched: &mut LaunchedGame,
    poll: Duration,
    grace: Duration,
) -> SessionExitReport {
    let main_exe_name = launched
        .program
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let mut seen_processes: Vec<RunningGameProcess> = Vec::new();
    if is_subpath(&launched.program, dir) {
        seen_processes.push(RunningGameProcess {
            pid: launched.pid(),
            exe: launched.program.clone(),
        });
    }

    let mut relaunched_outside = false;
    let mut child_exited = false;
    let mut last_activity = std::time::Instant::now();

    loop {
        // 1. Check child status
        if !child_exited {
            match launched.child.try_wait() {
                Ok(Some(_status)) => {
                    child_exited = true;
                    last_activity = std::time::Instant::now();
                }
                Ok(None) => {}
                Err(_) => {
                    child_exited = true;
                    last_activity = std::time::Instant::now();
                }
            }
        }

        // 2. Poll processes on system
        let mut sys = sysinfo::System::new();
        sys.refresh_processes();

        let mut current_in_dir_count = 0;
        for (pid, process) in sys.processes() {
            if let Some(exe) = process.exe() {
                if is_subpath(exe, dir) {
                    current_in_dir_count += 1;
                    let entry = RunningGameProcess {
                        pid: pid.as_u32(),
                        exe: exe.to_path_buf(),
                    };
                    if !seen_processes.contains(&entry) {
                        seen_processes.push(entry);
                    }
                } else if !main_exe_name.is_empty()
                    // Only a process started by this launch counts: a copy of the
                    // game the user already had running elsewhere is not a relaunch.
                    && process.start_time() >= launched.identity.start_time
                {
                    if let Some(name) = exe.file_name().and_then(|n| n.to_str()) {
                        #[cfg(windows)]
                        let matches_name = name.eq_ignore_ascii_case(&main_exe_name);
                        #[cfg(not(windows))]
                        let matches_name = name == main_exe_name;
                        if matches_name {
                            relaunched_outside = true;
                        }
                    }
                }
            }
        }

        if current_in_dir_count > 0 || !child_exited {
            last_activity = std::time::Instant::now();
        }

        // 3. Exit condition: child exited AND no processes in dir for >= grace
        if child_exited && current_in_dir_count == 0 && last_activity.elapsed() >= grace {
            break;
        }

        std::thread::sleep(poll);
    }

    seen_processes.sort_by_key(|p| p.pid);

    SessionExitReport {
        processes: seen_processes,
        relaunched_outside,
    }
}
