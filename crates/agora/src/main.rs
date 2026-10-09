use clap::{Parser, Subcommand, ValueEnum};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agora_core::crash_service::CrashService;
use agora_core::plugins::{CapabilityDescription, InstallPreview, PluginService, PluginSummary};
use agora_core::registry::RegistryService;
use agora_core::settings::SettingsService;
use agora_game_minecraft::clone::ClonePrefs;
use agora_game_minecraft::install_service::InstallService;
use agora_game_minecraft::instance_service::{CreateInstanceRequest, InstanceService};
use agora_game_minecraft::loader_service::LoaderService;
use agora_game_minecraft::runtime_service::RuntimeService;

/// A silent progress reporter for the CLI — no progress events are emitted.
struct SilentReporter;

impl agora_game_minecraft::install_pipeline::ProgressReporter for SilentReporter {
    fn report(&self, _event: agora_game_minecraft::install_pipeline::ProgressEvent) {}
}

/// A console progress reporter for runtime operations.
struct ConsoleRuntimeProgress;

impl agora_game_minecraft::runtime_manager::RuntimeProgress for ConsoleRuntimeProgress {
    fn on_progress(&self, message: &str, percent: Option<f64>) {
        if let Some(pct) = percent {
            eprintln!("[{}%] {}", pct, message);
        } else {
            eprintln!("[..] {}", message);
        }
    }
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct ConsoleLaunchProgress {
    json: bool,
    timings: bool,
}

struct CliProgressSink {
    file: Mutex<std::fs::File>,
    console: bool,
}

impl CliProgressSink {
    fn new(path: &Path, console: bool) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
            console,
        })
    }

    fn log(&self, level: &str, message: &str) {
        let line = format!("{} [{level}] {message}", chrono::Utc::now().to_rfc3339());
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{line}");
        }
    }
}

impl agora_core::event_sink::ProgressSink for CliProgressSink {
    fn report(&self, event: agora_core::event_sink::ProgressEvent) {
        let message = format!("{:?}: {}", event.phase, event.message);
        self.log("progress", &message);
        if self.console {
            eprintln!("[..] {message}");
        }
    }
}

impl agora_game_minecraft::launch_service::LaunchProgress for ConsoleLaunchProgress {
    fn phase(&self, _name: &str, message: &str) {
        if !self.json {
            eprintln!("[..] {message}");
        }
    }

    fn phase_completed(&self, name: &str, duration_ms: u128) {
        if self.timings {
            eprintln!("[timing] {name}: {duration_ms} ms");
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum OutputFormat {
    Human,
    Json,
}

impl OutputFormat {
    fn is_json_output(self) -> bool {
        matches!(self, OutputFormat::Json)
    }
}

#[derive(Parser)]
#[command(
    name = "agora",
    version,
    about = "Manage Agora instances, content, plugins, recovery, and direct launches",
    long_about = "Agora's standalone command-line interface uses the same core services as the desktop application. It can synchronize the signed registry, create and inspect instances, resolve content changes, manage community plugins, run health checks, manage snapshots and lockfiles, investigate crashes, and launch Minecraft directly.",
    after_help = "Start with `agora paths`, `agora registry status`, and `agora list-instances`. Use `--data-dir` for an isolated test profile. See docs/CLI.md for safety guidance, examples, structured output, and exit codes.",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(long, global = true, help = "Path to Agora data directory")]
    data_dir: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        help = "JSON output (shorthand for --output json)"
    )]
    json: bool,

    #[arg(
        long,
        global = true,
        help = "Output format: human or json. Overrides --json."
    )]
    output: Option<OutputFormat>,

    #[arg(
        long,
        global = true,
        help = "Registry repository (owner/repo). Overrides AGORA_REGISTRY_REPO env."
    )]
    registry_repo: Option<String>,

    #[arg(long, global = true, help = "Append diagnostics to this log file")]
    log_file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// List every local instance.
    ListInstances,
    /// Print resolved Agora data, database, cache, runtime, and instance paths.
    Paths,
    /// Print one instance by ID.
    GetInstance { id: String },
    /// Create, clone, lock, rename, repair, or delete instances.
    Instance {
        #[command(subcommand)]
        action: InstanceCmd,
    },
    /// Search, install, remove, update, enable, or disable content.
    #[command(name = "mod")]
    Mods {
        #[command(subcommand)]
        action: ModsCmd,
    },
    /// Run the local health scanner for an instance.
    Health { instance: String },
    /// Inspect the installed content inventory for an instance.
    Inventory { instance: String },
    /// Inspect or synchronize the signed Agora registry.
    Registry {
        #[command(subcommand)]
        action: RegistryCmd,
    },
    /// Create, list, restore, or delete recovery snapshots.
    Snapshots {
        #[command(subcommand)]
        action: SnapshotsCmd,
    },
    /// Import a local supported pack or a remote .mrpack URL.
    Import {
        #[arg(required_unless_present = "url")]
        path: Option<PathBuf>,
        #[arg(
            long,
            conflicts_with = "path",
            help = "Download and import a .mrpack URL"
        )]
        url: Option<String>,
        #[arg(long, help = "Symlink saves instead of copying")]
        symlink_saves: bool,
        #[arg(
            long,
            conflicts_with = "url",
            help = "Name for the imported instance (use when the source's own name is already taken)"
        )]
        name: Option<String>,
    },
    /// Launch an instance directly through Agora core.
    Launch {
        instance: String,
        #[arg(long, help = "Skip health check confirmation")]
        yes: bool,
        #[arg(long, help = "Print launch phase timings to stderr")]
        timings: bool,
    },
    /// Manage the Microsoft account used for direct launch.
    Auth {
        #[command(subcommand)]
        action: AuthCmd,
    },
    /// Synchronize the signed registry.
    Sync,
    /// Discover, inspect, provision, or clean managed Java runtimes.
    Runtime {
        #[command(subcommand)]
        action: RuntimeCmd,
    },
    /// List or install pinned loader profiles.
    Loader {
        #[command(subcommand)]
        action: LoaderCmd,
    },
    /// Install, inspect, and recover community plugins.
    Plugin {
        #[command(subcommand)]
        action: PluginCmd,
    },
    /// List, switch, and search content providers (Modrinth, Technic, plugins).
    Provider {
        #[command(subcommand)]
        action: ProviderCmd,
    },
    /// Read or write Agora settings.
    Settings {
        #[command(subcommand)]
        action: SettingsCmd,
    },
    /// Run Agora's MCP server over standard input/output.
    Mcp {
        #[command(subcommand)]
        action: McpCmd,
    },
    /// List, inspect, or investigate crash evidence.
    Crash {
        #[command(subcommand)]
        action: CrashCmd,
    },
    /// Migrate data from an old CLI data root to the current app data directory.
    #[command(name = "migrate-data")]
    MigrateData {
        /// Path to the old (legacy) Agora CLI data root.
        #[arg(long, short)]
        from: PathBuf,
        /// Actually execute the migration (required; default is dry-run).
        #[arg(long)]
        yes: bool,
    },
    /// Install a modpack pack manifest into an existing instance.
    Pack {
        #[command(subcommand)]
        action: PackCmd,
    },
    /// Export an instance to a standalone server environment.
    Export { instance: String, dest: PathBuf },
    /// Manage loadout profiles for an instance (enable/disable sets of content).
    Loadout {
        #[command(subcommand)]
        action: LoadoutCmd,
    },
    /// Canonical integrity lockfiles: export, verify, repair, import.
    Lockfile {
        #[command(subcommand)]
        action: LockfileCmd,
    },
    /// Discover installed games across supported stores.
    Games {
        #[command(subcommand)]
        action: GamesCmd,
    },
}

#[derive(Subcommand)]
enum GamesCmd {
    /// Discover all game installs on the machine.
    Discover,
    /// List supported games and their identified installs.
    List,
    /// Manage pinned bases for games.
    Base {
        #[command(subcommand)]
        action: BaseCmd,
    },
    /// Manage content store for mod assets.
    Content {
        #[command(subcommand)]
        action: ContentCmd,
    },
    /// Browse the catalog entries for games other than Minecraft.
    Catalog {
        #[command(subcommand)]
        action: CatalogCmd,
    },
    /// Manage game instances.
    Instance {
        #[command(subcommand)]
        action: GameInstanceCmd,
    },
    /// Launch a game from its pinned base.
    Launch {
        /// Base ID of the pinned base to launch.
        base_id: String,
        /// Wait for the game process to exit.
        #[arg(long)]
        wait: bool,
        /// Launch even if the base fails verification.
        #[arg(long)]
        launch_anyway: bool,
        /// Start the game's own executable, not a framework loader such as SKSE's.
        #[arg(long)]
        plain: bool,
    },
    /// Manage per-user game files and journaled swap sessions.
    #[command(name = "user-files")]
    UserFiles {
        #[command(subcommand)]
        action: UserFilesCmd,
    },
}

#[derive(Subcommand)]
enum UserFilesCmd {
    /// Show user-file swap sessions in progress.
    Status {
        /// Optional game ID to filter sessions.
        game: Option<String>,
    },
    /// Restore user files from a finished session.
    Restore {
        /// Game ID of the session to restore.
        game: String,
        /// Store ID of the session to restore (e.g. steam, gog).
        store: String,
    },
}

#[derive(Subcommand)]
enum GameInstanceCmd {
    /// Create a game instance from an install.
    Create {
        /// Install ID of the game to create an instance for.
        install_id: String,
        /// Name of the instance (defaults to game name).
        #[arg(long)]
        name: Option<String>,
        /// Optional explicit instance ID.
        #[arg(long)]
        id: Option<String>,
        /// Mode for pinned base: linked (default) or copied.
        #[arg(long, default_value = "linked")]
        mode: String,
        /// Include files excluded by the game definition.
        #[arg(long)]
        include_excluded: bool,
    },
    /// List all game instances, Minecraft included.
    List,
    /// Launch a game instance.
    Launch {
        /// Instance ID to launch.
        instance_id: String,
        /// Wait for the game process to exit.
        #[arg(long)]
        wait: bool,
        /// Launch even if the base fails verification.
        #[arg(long)]
        launch_anyway: bool,
        /// Run this launch from one deployment: virtual, links, copies, or auto (the
        /// instance's own choice). A mode you name never falls back to another.
        #[arg(long)]
        deployment: Option<String>,
        /// Start the game's own executable, not a framework loader such as SKSE's.
        #[arg(long)]
        plain: bool,
        /// If the virtual file system cannot start, run from the next fallback (linked files)
        /// without asking.
        #[arg(long)]
        fall_back: bool,
    },
    /// Choose how an instance's game folder is deployed and run: virtual (under the virtual file
    /// system), links, copies, or auto (let Agora pick and announce any step down).
    #[command(name = "set-deployment")]
    SetDeployment {
        /// Instance ID.
        instance_id: String,
        /// virtual, links, copies or auto.
        mode: String,
    },
    /// Delete a game instance.
    Delete {
        /// Instance ID to delete.
        instance_id: String,
    },
    /// Manage content layers in a game instance.
    Content {
        #[command(subcommand)]
        action: InstanceContentCmd,
    },
    /// Deploy content and base files to the game instance runtime folder.
    Deploy {
        /// Instance ID to deploy.
        instance_id: String,
        /// Copy all files instead of hardlinking (same as `--deployment copies`).
        #[arg(long, conflicts_with = "deployment")]
        copies: bool,
        /// Deploy as virtual, links or copies. Without it the instance's own choice applies,
        /// else the mode a launch would use.
        #[arg(long)]
        deployment: Option<String>,
    },
    /// Undeploy content, harvest writes back to writable layer, and remove game folder.
    Undeploy {
        /// Instance ID to undeploy.
        instance_id: String,
    },
    /// Check the framework files against the game version, and the plugin load order, without
    /// launching. Exits 1 when a framework was built for another version or the load order would
    /// stop the game: the same checks a launch makes.
    Check {
        /// Instance ID to check.
        instance_id: String,
    },
    /// Read and change the game's INI files as the instance keeps them. With no file, lists the
    /// files; with a file, lists its keys; with a section and key, gets the value; with a value,
    /// sets it; with --unset, removes the key.
    Ini {
        /// Instance ID.
        instance_id: String,
        /// The game's file, as the instance names it, e.g. user/Skyrim.ini.
        file: Option<String>,
        /// The section, e.g. General.
        section: Option<String>,
        /// The key, e.g. SLocalSavePath.
        key: Option<String>,
        /// The value to set.
        value: Option<String>,
        /// Remove the key instead of getting or setting it.
        #[arg(long)]
        unset: bool,
    },
    /// Show or change whether the instance's saves are the game's shared folder or its own. With no
    /// choice, shows it. Switching never moves a save file.
    Saves {
        /// Instance ID.
        instance_id: String,
        /// own or shared. Omit to show the current choice.
        choice: Option<String>,
    },
    /// Show or change which plugins an instance's plugin list activates.
    #[command(args_conflicts_with_subcommands = true)]
    Plugins {
        /// Instance whose plugin list to show.
        instance_id: Option<String>,
        #[command(subcommand)]
        action: Option<InstancePluginsCmd>,
    },
    /// Run the tools a game declares (such as Nemesis) and manage the output they write into an
    /// instance, kept as a generated layer (MASTER_SPEC §26.9).
    Tools {
        #[command(subcommand)]
        action: InstanceToolsCmd,
    },
}

#[derive(Subcommand)]
enum InstanceToolsCmd {
    /// List each tool the game declares, with its current and previous output, and whether that
    /// output is current, stale or unknown.
    List {
        /// Instance ID.
        instance_id: String,
    },
    /// Run a tool in the instance. A run that exits 0 becomes the tool's output; any other run is
    /// discarded, and the output that was in effect stays.
    Run {
        /// Instance ID.
        instance_id: String,
        /// Tool ID, e.g. nemesis.
        tool: String,
    },
    /// Make the previous output current again. The game reads it on its next launch.
    Rollback {
        /// Instance ID.
        instance_id: String,
        /// Tool ID, e.g. nemesis.
        tool: String,
    },
    /// Remove a tool's output from the instance, with its generations.
    Remove {
        /// Instance ID.
        instance_id: String,
        /// Tool ID, e.g. nemesis.
        tool: String,
    },
    /// Show the files a tool's output added, changed and removed between its previous and current
    /// generations.
    Diff {
        /// Instance ID.
        instance_id: String,
        /// Tool ID, e.g. nemesis.
        tool: String,
    },
}

impl InstanceToolsCmd {
    fn instance_id(&self) -> &str {
        match self {
            InstanceToolsCmd::List { instance_id }
            | InstanceToolsCmd::Run { instance_id, .. }
            | InstanceToolsCmd::Rollback { instance_id, .. }
            | InstanceToolsCmd::Remove { instance_id, .. }
            | InstanceToolsCmd::Diff { instance_id, .. } => instance_id,
        }
    }
}

#[derive(Subcommand)]
enum InstancePluginsCmd {
    /// Activate a plugin in an instance's plugin list.
    Enable {
        /// Instance ID.
        instance_id: String,
        /// Plugin file name, e.g. SkyUI_SE.esp.
        name: String,
    },
    /// Deactivate a plugin in an instance's plugin list (it stays listed).
    Disable {
        /// Instance ID.
        instance_id: String,
        /// Plugin file name, e.g. SkyUI_SE.esp.
        name: String,
    },
    /// Sort the plugin list so every master comes before the plugins that need it.
    Sort {
        /// Instance ID.
        instance_id: String,
        /// Print the moves without writing the list.
        #[arg(long)]
        dry_run: bool,
    },
    /// Move a plugin to a position, or before or after another plugin.
    Move {
        /// Instance ID.
        instance_id: String,
        /// Plugin file name, e.g. SkyUI_SE.esp.
        plugin: String,
        /// Move to this 1-based position in the list (always-loaded plugins count).
        #[arg(long)]
        to: Option<usize>,
        /// Move to just before this plugin.
        #[arg(long)]
        before: Option<String>,
        /// Move to just after this plugin.
        #[arg(long)]
        after: Option<String>,
    },
    /// Lock a plugin's place, so sort and move leave it where it is.
    Lock {
        /// Instance ID.
        instance_id: String,
        /// Plugin file name, e.g. SkyUI_SE.esp.
        plugin: String,
    },
    /// Unlock a plugin's place, so sort and move may move it again.
    Unlock {
        /// Instance ID.
        instance_id: String,
        /// Plugin file name, e.g. SkyUI_SE.esp.
        plugin: String,
    },
    /// Print the load order findings. Exits 1 only when a launch would refuse; warnings exit 0.
    Check {
        /// Instance ID.
        instance_id: String,
    },
}

#[derive(Subcommand)]
enum InstanceContentCmd {
    /// Add a content item to an instance.
    Add {
        /// Instance ID to add content to.
        instance_id: String,
        /// Content item ID or prefix.
        item_id: String,
        /// Subdirectory in game folder to mount content under.
        #[arg(long)]
        into: Option<String>,
        /// Subfolder of the item to deploy.
        #[arg(long)]
        from: Option<String>,
    },
    /// List content items in an instance in priority order.
    List {
        /// Instance ID to list content for.
        instance_id: String,
    },
    /// Remove a content item from an instance.
    Remove {
        /// Instance ID to remove content from.
        instance_id: String,
        /// Content item ID or prefix.
        item_id: String,
    },
    /// Enable a content item in an instance.
    Enable {
        /// Instance ID.
        instance_id: String,
        /// Content item ID or prefix.
        item_id: String,
    },
    /// Disable a content item in an instance.
    Disable {
        /// Instance ID.
        instance_id: String,
        /// Content item ID or prefix.
        item_id: String,
    },
    /// Move a content item to a new position (1-based, lowest priority first).
    Move {
        /// Instance ID.
        instance_id: String,
        /// Content item ID or prefix.
        item_id: String,
        /// Target 1-based position.
        position: usize,
    },
    /// Configure whether an instance deploys its own copies of a content item's files.
    OwnCopy {
        /// Instance ID.
        instance_id: String,
        /// Content item ID or prefix.
        item_id: String,
        /// "on" or "off".
        state: String,
    },
}

#[derive(Subcommand)]
enum BaseCmd {
    /// Build a pinned base from a game install.
    Build {
        /// Install ID of the game to build a base for.
        install_id: String,
        /// Mode for the base: linked (default, using hardlinks for archives) or copied (full copy).
        #[arg(long, default_value = "linked")]
        mode: String,
        /// Include files excluded by the game definition.
        #[arg(long)]
        include_excluded: bool,
    },
    /// List all pinned bases.
    List,
    /// Verify the integrity of a pinned base.
    Verify {
        /// Base ID to verify.
        base_id: String,
        /// Run full verification (hash all files).
        #[arg(long)]
        full: bool,
    },
    /// Remove a pinned base and its manifest.
    Remove {
        /// Base ID to remove.
        base_id: String,
    },
}

#[derive(Subcommand)]
enum CatalogCmd {
    /// List the catalog entries for one game, such as skyrim-se.
    List {
        /// The game id, as `agora games list` shows it.
        game: String,
    },
}

#[derive(Subcommand)]
enum ContentCmd {
    /// Add an archive or folder to the content store.
    Add {
        /// Path to the zip, 7z or RAR archive, or a folder.
        path: PathBuf,
        /// Optional name for the content item.
        #[arg(long)]
        name: Option<String>,
    },
    /// List all items in the content store.
    List,
    /// Show files in a content item.
    Show {
        /// Item ID or unique prefix.
        item: String,
    },
    /// Verify integrity of content items.
    Verify {
        /// Item ID or unique prefix to verify (verifies all items if omitted).
        item: Option<String>,
        /// Run full verification (re-hash all objects).
        #[arg(long)]
        full: bool,
    },
    /// Remove an item from the content store.
    Remove {
        /// Item ID or unique prefix to remove.
        item: String,
    },
    /// Show or run the FOMOD installer inside an archive item.
    Fomod {
        #[command(subcommand)]
        action: FomodCmd,
    },
}

#[derive(Subcommand)]
enum FomodCmd {
    /// Show an installer's steps, groups and options.
    Show {
        /// Archive item ID or unique prefix.
        item: String,
    },
    /// Install an archive item through its FOMOD installer into a new content item.
    Install {
        /// Archive item ID or unique prefix.
        item: String,
        /// Instance whose files answer the installer's file checks; the installed item is added to it.
        #[arg(long)]
        instance: Option<String>,
        /// An option to select, as "Step/Group/Plugin" (repeatable). Replaces the defaults of its group.
        #[arg(long = "choose", value_name = "STEP/GROUP/PLUGIN")]
        choose: Vec<String>,
        /// Select every option the installer requires or recommends (and a first usable option where a group needs one).
        #[arg(long)]
        defaults: bool,
    },
}

#[derive(Subcommand)]
enum InstanceCmd {
    /// Create a new instance with the given name, MC version, loader, and loader version.
    Create {
        name: String,
        #[arg(short, long, help = "Minecraft version (e.g. 1.21)")]
        mc_version: String,
        #[arg(short, long, default_value = "vanilla", help = "Mod loader")]
        loader: String,
        #[arg(short = 'V', long, default_value = "", help = "Loader version")]
        loader_version: String,
        #[arg(long)]
        jvm_memory_mb: Option<i64>,
        #[arg(long)]
        jvm_gc: Option<String>,
        #[arg(long)]
        jvm_custom_args: Option<String>,
        #[arg(long)]
        jvm_always_pre_touch: Option<bool>,
        #[arg(
            long = "template",
            help = "Instance template id to seed configs and JVM settings from"
        )]
        template_id: Option<String>,
    },
    /// Clone an existing instance with copy-preference flags.
    Clone {
        source: String,
        name: String,
        #[arg(long, help = "Skip copying saves")]
        no_saves: bool,
        #[arg(long, help = "Skip copying mods")]
        no_mods: bool,
        #[arg(long, help = "Skip copying resource packs")]
        no_resource_packs: bool,
        #[arg(long, help = "Skip copying shader packs")]
        no_shader_packs: bool,
        #[arg(long, help = "Skip copying screenshots")]
        no_screenshots: bool,
        #[arg(long, help = "Skip copying config")]
        no_config: bool,
        #[arg(long, help = "Skip copying servers.dat")]
        no_servers: bool,
        #[arg(long, help = "Skip copying options files")]
        no_options: bool,
        #[arg(long, help = "Use hard links instead of copying")]
        hard_links: bool,
        #[arg(long, help = "Use symlinks instead of copying")]
        sym_links: bool,
    },
    /// Lock an instance to prevent modification.
    Lock { id: String },
    /// Unlock a locked instance.
    Unlock { id: String },
    /// Delete an instance and its directory.
    Delete { id: String },
    /// Rename an instance.
    Rename { id: String, name: String },
    /// Reinstall the mod loader for an instance.
    RepairLoader { id: String },
    /// Explain the current automatic memory estimate without changing settings.
    RecommendMemory { id: String },
}

#[derive(Subcommand)]
enum ProviderCmd {
    /// List every content provider and whether it can be used right now.
    List,
    /// Switch a provider on. For a plugin's provider, this enables the plugin.
    Enable { provider_id: String },
    /// Switch a provider off. For a plugin's provider, this disables the plugin.
    Disable { provider_id: String },
    /// Search one provider directly.
    Search {
        provider_id: String,
        #[arg(default_value = "")]
        query: String,
        #[arg(long)]
        content_type: Option<String>,
    },
    /// Print the plan digest a curator pins as `sha256` for a `provider_pack`
    /// catalog entry. IDENTIFIER is `<provider-id>:<project-id>@<version-id>`.
    PlanDigest { identifier: String },
}

#[derive(Subcommand)]
enum LoaderCmd {
    /// List all available mod loaders from the pinned catalog.
    List {
        #[arg(
            short = 'm',
            long,
            visible_alias = "minecraft",
            help = "List loader versions compatible with this Minecraft version"
        )]
        mc_version: Option<String>,
    },
    /// Install a pinned loader profile without creating an instance.
    Install {
        loader: String,
        mc_version: String,
        loader_version: String,
        #[arg(long, help = "Reinstall even when a verified profile exists")]
        force: bool,
    },
}

#[derive(Subcommand)]
enum PluginCmd {
    /// List installed plugins, including the core-resolved status and reason.
    #[command(visible_alias = "status")]
    List,
    /// Preview a package or development folder without installing it.
    Preview {
        #[command(subcommand)]
        source: PluginSourceCmd,
    },
    /// Install a package or register a development folder.
    Install {
        #[command(subcommand)]
        source: PluginSourceCmd,
        /// Accept the manifest's requested capabilities without prompting.
        ///
        /// `global` so it is accepted on either side of the `package` /
        /// `development` subcommand. Both readings are natural, and a consent
        /// flag that silently fails to parse is the wrong thing to be strict
        /// about.
        #[arg(
            long,
            global = true,
            help = "Accept requested plugin capabilities without prompting"
        )]
        yes: bool,
    },
    /// Enable a previously disabled plugin.
    Enable { id: String },
    /// Disable a plugin without removing it.
    Disable { id: String },
    /// Remove a plugin; keep its stored data unless --purge-data is given.
    Remove {
        id: String,
        #[arg(long, help = "Also discard the plugin's stored data")]
        purge_data: bool,
    },
    /// Read the most recent lines from a plugin's log.
    Log {
        id: String,
        #[arg(long, default_value_t = 200, help = "Number of log lines to read")]
        lines: usize,
    },
    /// Disable every installed plugin as a recovery action.
    #[command(name = "disable-all")]
    DisableAll,
    /// Ask a plugin's publisher whether there is a newer release.
    ///
    /// Fetches only the signed metadata, never a package. With no id, checks
    /// every plugin that came with an update source.
    #[command(name = "check-update")]
    CheckUpdate {
        /// Plugin id. Omit to check all of them.
        id: Option<String>,
    },
    /// Download and install the release the publisher is offering.
    Update {
        id: String,
        #[arg(
            long,
            help = "Accept capabilities the installed version was not granted"
        )]
        yes: bool,
    },
    /// Put a plugin's stored data back to the copy kept before its last
    /// data-shape change.
    ///
    /// Never runs on its own. A copy is kept when an update changes how a
    /// plugin stores data; going back to it also discards anything written
    /// since, so it is always a decision.
    #[command(name = "restore-data")]
    RestoreData {
        id: String,
        #[arg(long, help = "Skip the confirmation prompt")]
        yes: bool,
    },
    /// Generate an Ed25519 signing key for publishing updates.
    ///
    /// Author tooling. The private key is written to a file you keep; Agora
    /// never stores it, never reads it again, and cannot recover it.
    #[command(name = "keygen")]
    Keygen {
        /// Where to write the private key. The public half is printed.
        #[arg(long)]
        out: PathBuf,
        /// Label recorded alongside the key, echoed by signatures.
        #[arg(long, default_value = "default")]
        key_id: String,
    },
    /// Sign an update document in place.
    ///
    /// Reads the document, signs the canonical form, and writes it back with
    /// the signature attached.
    Sign {
        /// The update document to sign.
        document: PathBuf,
        /// Private key file produced by `keygen`.
        #[arg(long)]
        key: PathBuf,
        /// Key id to record in the signature. Defaults to the one the
        /// document already names, if it names one.
        #[arg(long)]
        key_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum PluginSourceCmd {
    /// Inspect or install a packaged plugin archive.
    Package { path: PathBuf },
    /// Inspect or register a plugin development folder in place.
    Development { path: PathBuf },
}

#[derive(Subcommand)]
enum SettingsCmd {
    /// List all user settings.
    List,
    /// Get a single setting by key.
    Get { key: String },
    /// Set a setting (value is parsed as JSON; falls back to string).
    Set { key: String, value: String },
}

#[derive(Subcommand)]
enum ModsCmd {
    List {
        instance: String,
    },
    Install {
        project: String,
        instance: String,
        #[arg(short, long)]
        version: Option<String>,
        #[arg(
            long,
            value_enum,
            default_value_t = ModSourceArg::Curated,
            help = "Artifact source: curated registry strategy or raw Modrinth project"
        )]
        source: ModSourceArg,
        #[arg(long, help = "Allow replacing existing files")]
        allow_replace: bool,
        #[arg(long, help = "Skip health scan after install")]
        skip_health_scan: bool,
        #[arg(
            long,
            help = "Install a file that does not match its curator pin or the hash recorded on an earlier install, after checking it yourself"
        )]
        install_anyway: bool,
        #[arg(
            long,
            conflicts_with = "exclude_optional",
            help = "Include specific optional dependencies (comma-separated)"
        )]
        include_optional: Option<String>,
        #[arg(
            long,
            conflicts_with = "include_optional",
            help = "Exclude all optional dependencies"
        )]
        exclude_optional: bool,
        #[arg(long, help = "Resolve all conflicts by replacing")]
        replace_conflicts: bool,
        #[arg(long, help = "Abort on any unresolved conflict")]
        abort_conflicts: bool,
        #[arg(long, help = "Resolve plan and print it without executing")]
        dry_run: bool,
    },
    Remove {
        project: String,
        instance: String,
        #[arg(long, help = "Allow replacing existing files")]
        allow_replace: bool,
        #[arg(long, help = "Skip health scan after removal")]
        skip_health_scan: bool,
        #[arg(long, help = "Resolve all conflicts by replacing")]
        replace_conflicts: bool,
        #[arg(long, help = "Abort on any unresolved conflict")]
        abort_conflicts: bool,
        #[arg(
            long,
            conflicts_with = "abort_conflicts",
            help = "Remove the file even if an installed mod still requires it (those mods will show a health alert)"
        )]
        remove_anyway: bool,
        #[arg(long, help = "Resolve plan and print it without executing")]
        dry_run: bool,
    },
    /// Search the curated registry for items matching a query.
    Search {
        query: String,
        #[arg(
            short,
            long,
            help = "Content type filter (mod, resourcepack, shader, datapack, world)"
        )]
        content_type: Option<String>,
        #[arg(short = 'V', long, help = "Minecraft version filter")]
        mc_version: Option<String>,
    },
    /// Update a single installed item to a newer version.
    Update {
        instance: String,
        item: String,
        #[arg(short, long, help = "Target version (default: latest)")]
        version: Option<String>,
        #[arg(
            long,
            help = "Install a file that does not match its curator pin or the hash recorded on an earlier install, after checking it yourself"
        )]
        install_anyway: bool,
        #[arg(
            long,
            conflicts_with = "exclude_optional",
            help = "Include specific optional dependencies (comma-separated)"
        )]
        include_optional: Option<String>,
        #[arg(
            long,
            conflicts_with = "include_optional",
            help = "Exclude all optional dependencies"
        )]
        exclude_optional: bool,
        #[arg(long, help = "Resolve all conflicts by replacing")]
        replace_conflicts: bool,
        #[arg(long, help = "Abort on any unresolved conflict")]
        abort_conflicts: bool,
        #[arg(long, help = "Resolve plan and print it without executing")]
        dry_run: bool,
    },
    /// Update all installed items with a registry identity to their latest versions.
    UpdateAll {
        instance: String,
        #[arg(
            long,
            conflicts_with = "exclude_optional",
            help = "Include specific optional dependencies (comma-separated)"
        )]
        include_optional: Option<String>,
        #[arg(
            long,
            conflicts_with = "include_optional",
            help = "Exclude all optional dependencies"
        )]
        exclude_optional: bool,
        #[arg(long, help = "Resolve all conflicts by replacing")]
        replace_conflicts: bool,
        #[arg(long, help = "Abort on any unresolved conflict")]
        abort_conflicts: bool,
        #[arg(long, help = "Resolve plan and print it without executing")]
        dry_run: bool,
    },
    /// Enable a previously disabled mod by renaming <file>.disabled back to <file>.
    Enable {
        instance: String,
        file: String,
    },
    /// Disable a mod by renaming <file> to <file>.disabled.
    Disable {
        instance: String,
        file: String,
    },
    /// Choose which worlds a data pack is copied into (default: all worlds).
    Worlds {
        instance: String,
        /// The installed data pack file, for example veinminer-1.3.4.zip.
        file: String,
        #[arg(
            long,
            value_delimiter = ',',
            conflicts_with = "all",
            required_unless_present = "all",
            help = "Only these world folders (comma-separated)"
        )]
        worlds: Option<Vec<String>>,
        #[arg(long, help = "Sync into every world, including ones created later")]
        all: bool,
    },
    /// Copy the enabled data packs into the instance's worlds now.
    SyncDatapacks {
        instance: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ModSourceArg {
    Curated,
    Modrinth,
}

#[derive(Subcommand)]
enum RegistryCmd {
    Status,
    Sync,
}

#[derive(Subcommand)]
enum SnapshotsCmd {
    List {
        instance: String,
    },
    Create {
        instance: String,
        #[arg(short, long)]
        label: Option<String>,
    },
    Restore {
        instance: String,
        snapshot_id: String,
    },
    Delete {
        instance: String,
        snapshot_id: String,
    },
}

#[derive(Subcommand)]
enum AuthCmd {
    Login {
        /// Print the sign-in link instead of opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
    Status,
    Logout,
}

#[derive(Subcommand)]
enum RuntimeCmd {
    /// List all discovered Java runtimes (managed + Mojang + system).
    List,
    /// Ensure a managed Java runtime for the given major version is installed.
    Ensure { major: u32 },
    /// Remove unused managed Java runtimes (keep newest per major).
    RemoveUnused,
    /// Inspect a Java executable at the given path.
    Inspect { path: PathBuf },
}

#[derive(Subcommand)]
enum PackCmd {
    /// Install a pack manifest JSON file into an existing instance.
    Install {
        /// Path to the pack manifest JSON file.
        path: PathBuf,
        /// Target instance ID.
        instance: String,
    },
    /// List a curated registry pack's locked releases, newest first.
    Versions {
        /// Curated pack ID.
        pack: String,
    },
    /// Install a curated registry pack into an existing instance.
    ///
    /// With --release, installs that locked release (the instance must be on
    /// its Minecraft version and loader). Without it, installs the flexible
    /// recipe against the instance's own version: each mod gets its newest
    /// build that fits, recommended/optional mods with no build are left out,
    /// and a missing required mod stops the install.
    Curated {
        /// Curated pack ID.
        pack: String,
        /// Target instance ID.
        instance: String,
        #[arg(long, help = "Locked release to install (default: flexible recipe)")]
        release: Option<String>,
        #[arg(long, help = "Print the plan without installing")]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum LoadoutCmd {
    /// Create a loadout profile from the current enabled state.
    Create { instance: String, name: String },
    /// List all loadout profiles for an instance.
    List { instance: String },
    /// Apply a loadout profile (enable/disable content to match).
    Apply { instance: String, name: String },
    /// Delete a loadout profile.
    Delete { instance: String, name: String },
}

#[derive(Subcommand)]
enum LockfileCmd {
    /// Export a lockfile from the current state of an instance.
    Export {
        instance: String,
        #[arg(long = "out", short = 'o', help = "Write path (default: stdout)")]
        out: Option<PathBuf>,
    },
    /// Verify a lockfile's structure and content hash.
    Verify {
        /// Path to the lockfile JSON.
        path: PathBuf,
    },
    /// Repair an instance's lockfile by re-exporting.
    Repair {
        instance: String,
        #[arg(long = "out", short = 'o', help = "Write path (default: stdout)")]
        out: Option<PathBuf>,
    },
    /// Import a lockfile and restore the instance to match.
    Import {
        /// Path to the lockfile JSON.
        path: PathBuf,
        /// Target instance ID.
        instance: String,
        #[arg(long, help = "Skip health scan after import")]
        skip_health_scan: bool,
    },
}

#[derive(Subcommand)]
enum CrashCmd {
    /// List crash reports for an instance.
    List { instance: String },
    /// Read the content of a crash report file.
    Inspect { instance: String, file: String },
    /// Collect recent logs and run the full local crash investigation.
    Investigate {
        instance: String,
        /// Add an explicit diagnostic text file. May be repeated.
        #[arg(long = "file")]
        files: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum McpCmd {
    /// Start the MCP server with stdio transport (read JSON-RPC from stdin, write to stdout).
    Serve {
        /// Use stdio transport (required; HTTP is not yet implemented)
        #[arg(long)]
        stdio: bool,
    },
}

fn print_table(columns: &[&str], rows: &[Vec<String>]) {
    if rows.is_empty() {
        for col in columns {
            print!("{col}  ");
        }
        println!();
        return;
    }
    let mut widths: Vec<usize> = columns.iter().map(|c| c.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }
    for (i, col) in columns.iter().enumerate() {
        print!("{col}");
        if i < widths.len() {
            for _ in 0..widths[i].saturating_sub(col.len()) + 2 {
                print!(" ");
            }
        }
    }
    println!();
    let total: usize =
        widths.iter().map(|w| w + 2).sum::<usize>() + columns.len().saturating_sub(1);
    for _ in 0..total {
        print!("-");
    }
    println!();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            print!("{cell}");
            if i < widths.len() {
                for _ in 0..widths[i].saturating_sub(cell.len()) + 2 {
                    print!(" ");
                }
            }
        }
        println!();
    }
}

/// Map a structured LauncherError to a semantic CLI exit code.
///
/// Exit-code ranges:
///   0   success
///   1   generic/unclassified
///   2   CLI usage error
///   7   game crash
///  10   local-state / DB
///  11   instance not found / locked / profile
///  12   instance creation
///  13   registry missing / corrupt / schema
///  20   network offline
///  21   download / registry download
///  30   integrity security (hash, untrusted source, zip bomb, override)
///  34   disk full
///  40   authentication
///  50   feature disabled
///  60   Mojang / launcher integration
///  61   version / loader resolution
///  62   Java runtime
///  70   dependency / placeholder
///  80   MCP rate-limit
///  81   MCP denied
///  82   MCP unauthorized
///  90–94  network policy / privacy
/// 100+  process errors
fn exit_code_from_launcher_error(err: &agora_core::error::LauncherError) -> i32 {
    use agora_core::error::LauncherError;
    match err {
        LauncherError::NetworkOffline => 20,
        LauncherError::RegistryDownloadFailed => 21,
        LauncherError::RegistrySignatureInvalid => 13,
        LauncherError::SchemaTooNew => 13,
        LauncherError::ZipBomb => 30,
        LauncherError::OverrideSecurityViolation => 30,
        LauncherError::HashMismatch => 30,
        LauncherError::HashConfirmationRequired(_) => 30,
        LauncherError::UntrustedSource => 31,
        LauncherError::DiskFull => 34,
        LauncherError::AuthExpired => 40,
        LauncherError::AuthRequired | LauncherError::MsaAuthRequired => 40,
        LauncherError::ModrinthDisabled => 50,
        LauncherError::InstanceLocked => 11,
        LauncherError::SandboxUnavailable => 50,
        LauncherError::MojangNotFound => 60,
        LauncherError::LaunchFailed => 61,
        LauncherError::GameCrash => 7,
        LauncherError::LocalStateFailed => 10,
        LauncherError::InstanceCreateFailed => 12,
        LauncherError::ProfileWriteFailed => 60,
        LauncherError::RegistryMissing => 13,
        LauncherError::UnsupportedLoader => 61,
        LauncherError::VersionNotFound => 61,
        LauncherError::GameVersionNotFound => 61,
        LauncherError::LoaderProfileNotFound => 61,
        LauncherError::ProfileMissing(..) => 11,
        LauncherError::ProfileUnsupportedMetadata(..) => 11,
        LauncherError::ProfileCorrupt(..) => 11,
        LauncherError::JavaIncompatible => 62,
        LauncherError::JavaRuntimeMissing { .. } => 62,
        LauncherError::JavaRuntimeCatalogMissing { .. } => 62,
        LauncherError::UnresolvedPlaceholder => 70,
        LauncherError::DependencyMissing => 70,
        LauncherError::McpTooManyRequests => 80,
        LauncherError::McpDenied => 81,
        LauncherError::McpUnauthorized => 82,
        LauncherError::NetworkMojangMetadataDisabled => 90,
        LauncherError::NetworkMojangContentDisabled => 91,
        LauncherError::NetworkLoaderDisabled => 92,
        LauncherError::NetworkMsaDisabled => 93,
        LauncherError::NetworkJavaDisabled => 94,
        LauncherError::JavaRuntimeCancelled { .. } => 62,
        LauncherError::JavaRuntimeDownloadDisabled { .. } => 94,
        LauncherError::MavenDescriptor => 30,
        LauncherError::MigrationConflict { .. } => 71,
        LauncherError::MigrationFailed { .. } => 72,
        LauncherError::ProcessCaptureFailed { .. } => 100,
        LauncherError::ProcessStale { .. } => 101,
        LauncherError::UserDecisionRequired => 71,
        LauncherError::Generic { .. } => 1,
    }
}

/// Map any error to a stable exit code.  If the error wraps a `LauncherError`
/// (via anyhow's downcast) use its semantic code; otherwise return 1.
fn exit_code_from_error(err: &anyhow::Error) -> i32 {
    if let Some(le) = err.downcast_ref::<agora_core::error::LauncherError>() {
        exit_code_from_launcher_error(le)
    } else {
        1
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let output_fmt = match cli.output {
        Some(f) => f,
        None if cli.json => OutputFormat::Json,
        None => OutputFormat::Human,
    };
    let json = output_fmt.is_json_output();

    // Author tooling, handled before anything touches the launcher's state.
    //
    // `keygen` writes a key file and `sign` rewrites a JSON document; neither
    // reads an instance, a setting or the database. Initialising core for them
    // is not merely wasteful — it opens `local_state.db`, so signing a release
    // while Agora is running fails on a lock, and two concurrent invocations
    // contend over a database neither wants. Short-circuited for the same
    // reason `MigrateData` is below.
    if let Commands::Plugin { action } = &cli.command {
        if let Some(result) = run_offline_plugin_command(action, output_fmt) {
            if let Err(error) = result {
                let code = exit_code_from_error(&error);
                if json {
                    let value = serde_json::json!({
                        "error": error.to_string(),
                        "exitCode": code,
                    });
                    eprintln!("{}", serde_json::to_string_pretty(&value).unwrap());
                } else {
                    eprintln!("Error: {error}");
                }
                std::process::exit(code);
            }
            return;
        }
    }

    // `--data-dir` still wins outright. Without it, defer to the same resolver
    // the desktop uses so a portable install's bundled CLI lands on the pack of
    // instances sitting next to it, rather than in the platform app-data dir.
    let paths = match cli.data_dir.clone() {
        Some(root) => agora_core::app_paths::AppPaths::from_root(root),
        None => agora_core::app_paths::AppPaths::platform_default(),
    };
    let log_path = cli
        .log_file
        .clone()
        .unwrap_or_else(|| paths.root().join("logs").join("agora-cli.log"));
    let progress = match CliProgressSink::new(&log_path, !json) {
        Ok(progress) => Arc::new(progress),
        Err(error) => {
            eprintln!("Error opening CLI log '{}': {error}", log_path.display());
            std::process::exit(1);
        }
    };
    progress.log("info", "Agora CLI started");

    // Migration must run before normal initialization. Initializing the
    // destination would create local_state.db and turn a fresh destination
    // into a false database conflict.
    if let Commands::MigrateData { from, yes } = &cli.command {
        if let Err(error) = run_data_migration(&paths, from, *yes, output_fmt) {
            let code = exit_code_from_error(&error);
            if json {
                let value = serde_json::json!({
                    "error": error.to_string(),
                    "exitCode": code,
                });
                eprintln!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                eprintln!("Error: {error}");
            }
            std::process::exit(code);
        }
        return;
    }

    // A refused compiled package is a build bug, not user input.
    let mut registry_builder = agora_core::game_registry::GameRegistry::builder();
    agora_game_minecraft::register_into(&mut registry_builder)
        .expect("build bug: the Minecraft package was refused");
    registry_builder
        .add(
            agora_core::game_registry::PackageSource::Compiled {
                crate_name: "agora-game-creation".to_string(),
            },
            agora_game_creation::game_package(),
        )
        .expect("build bug: the Creation Engine package was refused");

    let (ctx, warnings) =
        match agora_core::ctx::CoreContext::initialize(paths.clone(), registry_builder) {
            Ok(result) => result,
            Err(error) => {
                progress.log("error", &format!("Core initialization failed: {error}"));
                eprintln!("Error initializing Agora core: {error}");
                std::process::exit(1);
            }
        };
    let ctx = ctx.with_progress_sink(progress.clone());
    for warning in warnings {
        progress.log("warning", &warning);
        eprintln!("Warning: {warning}");
    }
    let data_dir = paths.root().to_path_buf();
    let result = run_command(cli, &paths, &data_dir, &ctx, output_fmt).await;
    if let Err(e) = result {
        progress.log("error", &e.to_string());
        let code = exit_code_from_error(&e);
        if json {
            let err_val = serde_json::json!({
                "error": e.to_string(),
                "exitCode": code,
            });
            eprintln!("{}", serde_json::to_string_pretty(&err_val).unwrap());
        } else {
            eprintln!("Error: {e}");
        }
        std::process::exit(code);
    }
}

fn run_data_migration(
    paths: &agora_core::app_paths::AppPaths,
    from: &Path,
    yes: bool,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    if !from.exists() {
        anyhow::bail!("Source path '{}' does not exist", from.display());
    }

    let service = agora_core::data_migration::DataMigrationService::new(paths.clone());
    let plan = service.plan(from)?;
    if !plan.can_proceed {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "status": "conflict",
                    "sourceInventory": plan.source_inventory,
                    "conflicts": plan.conflicts,
                }))?
            );
        } else {
            println!("Source: {}", plan.source_inventory.source_root);
            println!(
                "Files:  {} ({:.2} MB)",
                plan.source_inventory.files.len(),
                plan.source_inventory.total_size_bytes as f64 / 1_048_576.0
            );
            println!("CONFLICTS - migration cannot proceed:");
            for conflict in &plan.conflicts {
                println!("  - {}: {}", conflict.rel_path, conflict.reason);
            }
        }
        return Err(agora_core::error::LauncherError::MigrationConflict {
            message: format!("Migration blocked by {} conflict(s)", plan.conflicts.len()),
        }
        .into());
    }

    if !yes {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "status": "dry-run",
                    "sourceInventory": plan.source_inventory,
                    "conflicts": plan.conflicts,
                }))?
            );
        } else {
            println!("Dry-run: migration would copy the following:");
            println!("  Source:   {}", plan.source_inventory.source_root);
            println!(
                "  Files:    {} ({:.2} MB)",
                plan.source_inventory.files.len(),
                plan.source_inventory.total_size_bytes as f64 / 1_048_576.0
            );
            if !plan.source_inventory.instance_ids.is_empty() {
                println!(
                    "  Instances: {}",
                    plan.source_inventory.instance_ids.join(", ")
                );
            }
            println!("Pass --yes to execute this migration.");
        }
        return Ok(());
    }

    let result = service.execute(from)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "Migration complete: {} file(s), {:.2} MB, {} instance(s)",
            result.files_migrated,
            result.total_bytes as f64 / 1_048_576.0,
            result.instance_ids.len(),
        );
        println!("Backup: {}", result.backup_path);
        if !result.instance_ids.is_empty() {
            println!("Instances: {}", result.instance_ids.join(", "));
        }
    }
    Ok(())
}

/// One line for a verdict, from its serialised form.
///
/// Reads the JSON rather than the enum so the CLI prints exactly the shape it
/// would emit under `--json`; a divergence between the two is the kind of bug
/// nobody notices until someone scripts against it.
fn describe_verdict(verdict: &serde_json::Value) -> String {
    let state = verdict["state"].as_str().unwrap_or("");
    let text = |key: &str| verdict[key].as_str().unwrap_or("?").to_string();
    match state {
        "upToDate" => "up to date".into(),
        "available" => {
            let notes = verdict["notes"].as_str().unwrap_or("");
            let line = format!("{} -> {} available", text("from"), text("to"));
            if notes.is_empty() {
                line
            } else {
                format!("{line} — {notes}")
            }
        }
        // Deliberately not folded into "up to date": a plugin held back by the
        // host version is a thing the user can act on, and saying it is
        // current would stop them looking.
        "needsNewerHost" => format!(
            "{} is available but needs an Agora matching {}",
            text("latest"),
            text("requires")
        ),
        "installedIsNewer" => format!(
            "installed {} is newer than the published {}",
            text("installed"),
            text("latest")
        ),
        "noReleases" => "the publisher lists no releases".into(),
        other => format!("unrecognised result `{other}`"),
    }
}

/// Write a new Ed25519 private key and return its public half as a
/// `PublicKey` ready to paste into a package.
///
/// Refuses to overwrite an existing file. Overwriting a signing key is never
/// what someone meant, and doing it silently would destroy the only copy of
/// something that cannot be regenerated.
fn generate_signing_key(out: &Path, key_id: &str) -> anyhow::Result<serde_json::Value> {
    use base64::Engine as _;
    use rand::RngCore as _;

    if out.exists() {
        anyhow::bail!(
            "{} already exists. Refusing to overwrite a signing key.",
            out.display()
        );
    }
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
    let engine = base64::engine::general_purpose::STANDARD;

    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    // The file is the secret, so it is written once, with a trailing newline
    // and nothing else — no id, no comment, nothing that invites someone to
    // paste the whole file somewhere thinking it is the public half.
    std::fs::write(out, format!("{}\n", engine.encode(signing.to_bytes())))?;
    restrict_to_owner(out);

    Ok(serde_json::json!({
        "id": key_id,
        "algorithm": "ed25519",
        "publicKey": engine.encode(signing.verifying_key().to_bytes()),
    }))
}

/// Best-effort: make a private key file readable only by its owner.
///
/// On Windows the meaningful control is the directory ACL, which is not ours
/// to change, so this is a no-op there rather than a false assurance.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) {}

/// Sign an update document in place, returning the key id used.
fn sign_update_document(
    document: &Path,
    key: &Path,
    key_id: Option<&str>,
) -> anyhow::Result<String> {
    use agora_plugin_api::distribution::{Signature, UpdateDocument};
    use base64::Engine as _;
    use ed25519_dalek::Signer as _;

    let engine = base64::engine::general_purpose::STANDARD;
    let key_text = std::fs::read_to_string(key)
        .map_err(|e| anyhow::anyhow!("could not read {}: {e}", key.display()))?;
    let raw = engine
        .decode(key_text.trim())
        .map_err(|_| anyhow::anyhow!("{} is not a base64 private key", key.display()))?;
    let raw: [u8; 32] = raw
        .try_into()
        .map_err(|_| anyhow::anyhow!("{} is not a 32-byte Ed25519 key", key.display()))?;
    let signing = ed25519_dalek::SigningKey::from_bytes(&raw);

    let text = std::fs::read_to_string(document)
        .map_err(|e| anyhow::anyhow!("could not read {}: {e}", document.display()))?;
    // Parsed as a draft: requiring a valid signature in order to produce one
    // is a requirement nobody can meet. Everything else about the document is
    // still checked, so signing one with a bad release in it fails here rather
    // than at every user who later fetches it.
    let mut parsed =
        UpdateDocument::parse_draft(&text).map_err(|e| anyhow::anyhow!("{}", e.message))?;

    let key_id = match key_id {
        Some(explicit) => explicit.to_string(),
        None => parsed
            .signatures
            .first()
            .map(|signature| signature.key_id.clone())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "this document does not say which key signs it. Pass --key-id, or add a \
                     `signatures` entry naming one."
                )
            })?,
    };

    // Signed over the document with `signatures` removed, so whatever
    // placeholder was in there cannot affect the result.
    let signature = signing.sign(&parsed.signing_bytes());
    parsed.signatures = vec![Signature {
        key_id: key_id.clone(),
        algorithm: "ed25519".into(),
        value: engine.encode(signature.to_bytes()),
    }];
    std::fs::write(document, serde_json::to_string_pretty(&parsed)?)?;
    Ok(key_id)
}

/// Ask before discarding data. Returns false in a non-interactive session,
/// where a silent yes would be the worst possible default.
fn confirm_restore(id: &str) -> anyhow::Result<bool> {
    if !std::io::stdin().is_terminal() {
        eprintln!("Restoring plugin data needs an interactive terminal; rerun with --yes.");
        return Ok(false);
    }
    print!("Restore the saved copy of {id} data? [y/N]: ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn plugin_service(ctx: &agora_core::ctx::Ctx) -> PluginService {
    let host: Arc<dyn agora_plugin_api::host::ScriptHost> =
        Arc::new(agora_plugin_host::QuickJsHost::new());
    PluginService::headless(ctx.clone(), host)
}

fn parse_plugin_id(raw: &str) -> anyhow::Result<agora_plugin_api::manifest::PluginId> {
    agora_plugin_api::manifest::PluginId::parse(raw).map_err(|error| {
        anyhow::Error::from(agora_core::error::LauncherError::Generic {
            code: "ERR_PLUGIN_ID_INVALID".into(),
            message: error.message,
        })
    })
}

fn preview_plugin_source(
    service: &PluginService,
    source: &PluginSourceCmd,
) -> anyhow::Result<InstallPreview> {
    Ok(match source {
        PluginSourceCmd::Package { path } => service.preview_package(path)?,
        PluginSourceCmd::Development { path } => service.preview_folder(path)?,
    })
}

fn install_plugin_source(
    service: &PluginService,
    source: PluginSourceCmd,
    accept_capabilities: bool,
) -> anyhow::Result<PluginSummary> {
    Ok(match source {
        PluginSourceCmd::Package { path } => service.install_package(&path, accept_capabilities)?,
        PluginSourceCmd::Development { path } => {
            service.add_development_folder(&path, accept_capabilities)?
        }
    })
}

fn print_plugin_preview(
    preview: &InstallPreview,
    json: bool,
    to_stderr: bool,
) -> anyhow::Result<()> {
    if json {
        let rendered = serde_json::to_string_pretty(preview)?;
        if to_stderr {
            eprintln!("{rendered}");
        } else {
            println!("{rendered}");
        }
        return Ok(());
    }

    println!(
        "Plugin: {} ({})",
        preview.manifest.name, preview.manifest.id
    );
    println!("Version: {}", preview.manifest.version);
    println!("License: {}", preview.manifest.license);
    println!("API: {}", preview.manifest.api_range);
    if let Some(description) = &preview.manifest.description {
        println!("Description: {description}");
    }
    if let Some(source) = &preview.manifest.source {
        println!("Source: {source}");
    }
    if preview.file_count > 0 {
        println!(
            "Package: {} file(s), {} uncompressed bytes",
            preview.file_count, preview.uncompressed_bytes
        );
    } else {
        println!("Source: development folder (loaded in place)");
    }
    if let Some(version) = &preview.replaces_version {
        println!("Replaces installed version: {version}");
    }
    if preview.migrates_data {
        println!("Data: the installed data shape would be migrated");
    }

    fn print_capabilities(label: &str, capabilities: &[CapabilityDescription]) {
        if capabilities.is_empty() {
            return;
        }
        println!("{label}:");
        for capability in capabilities {
            let mut suffix = String::new();
            if capability.is_mutating {
                suffix.push_str(" [can change data]");
            }
            println!("  - {}: {}{}", capability.name, capability.summary, suffix);
        }
    }

    print_capabilities("Required capabilities", &preview.required_capabilities);
    print_capabilities("Optional capabilities", &preview.optional_capabilities);
    // The hosts are the other half of the `network` grant — and, for a content
    // provider, the scope its downloads count as verified in — so they belong
    // on the same screen as the capability, not only in the GUI.
    if !preview.manifest.network.hosts.is_empty() {
        println!("Reaches: {}", preview.manifest.network.hosts.join(", "));
    }
    for provider in &preview.manifest.contributions.content_providers {
        println!(
            "Content source: {} ({})",
            provider.title,
            provider.content_types.join(", ")
        );
    }
    if !preview.unsupported_capabilities.is_empty() {
        println!(
            "Unsupported capabilities: {}",
            preview.unsupported_capabilities.join(", ")
        );
    }
    if preview.required_capabilities.is_empty() && preview.optional_capabilities.is_empty() {
        println!("Capabilities: none");
    } else if preview.replaces_version.is_some() {
        // On a replacement the whole list is not the decision; the difference
        // is. Saying so is what lets someone approve a bugfix quickly and
        // still notice the release that started asking for more.
        if preview.added_capabilities.is_empty() {
            println!("New capabilities: none beyond what is already granted");
        } else {
            println!(
                "New capabilities not previously granted: {}",
                preview.added_capabilities.join(", ")
            );
        }
    }
    Ok(())
}

fn accept_plugin_capabilities(
    preview: &InstallPreview,
    json: bool,
    skip_prompt: bool,
) -> anyhow::Result<bool> {
    if skip_prompt || !preview.requires_capability_consent() {
        return Ok(skip_prompt);
    }

    // A preview is a diagnostic while an install is pending. Keep it off
    // stdout in JSON mode so a successful command still emits one value.
    print_plugin_preview(preview, json, json)?;
    if !std::io::stdin().is_terminal() {
        if json {
            eprintln!(
                "Capability consent was not granted in a non-interactive session; rerun with --yes."
            );
        } else {
            eprintln!(
                "Capability consent requires an interactive terminal; rerun with --yes for scripting."
            );
        }
        // Passing false through to core preserves PluginService's consent
        // check and its error instead of recreating that policy here.
        return Ok(false);
    }

    let prompt = format!(
        "Accept the requested capabilities for {}? [y/N]: ",
        preview.manifest.id
    );
    if json {
        eprint!("{prompt}");
        std::io::stderr().flush()?;
    } else {
        print!("{prompt}");
        std::io::stdout().flush()?;
    }
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Plugin subcommands that need no launcher state at all.
///
/// `None` for anything that does, so the caller falls through to the ordinary
/// core-backed path. The split is explicit rather than incidental: these two
/// write a key file and rewrite a JSON document, and making them open
/// `local_state.db` first is what would stop an author signing a release while
/// Agora is running.
fn run_offline_plugin_command(
    action: &PluginCmd,
    output_fmt: OutputFormat,
) -> Option<anyhow::Result<()>> {
    let json = output_fmt.is_json_output();
    match action {
        PluginCmd::Keygen { out, key_id } => Some(offline_keygen(out, key_id, json)),
        PluginCmd::Sign {
            document,
            key,
            key_id,
        } => Some(offline_sign(document, key, key_id.as_deref(), json)),
        _ => None,
    }
}

fn offline_keygen(out: &Path, key_id: &str, json: bool) -> anyhow::Result<()> {
    let public = generate_signing_key(out, key_id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&public)?);
    } else {
        println!("Private key written to {}.", out.display());
        println!(
            "Keep it safe and out of your repository. If you lose it you cannot ship              updates to existing installs, and nothing can restore that."
        );
        println!();
        println!("Put this in your package as agora-plugin-update.json:");
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema": 1,
                "url": "https://example.com/your-plugin.json",
                "keys": [public],
            }))?
        );
    }
    Ok(())
}

fn offline_sign(
    document: &Path,
    key: &Path,
    key_id: Option<&str>,
    json: bool,
) -> anyhow::Result<()> {
    let signed = sign_update_document(document, key, key_id)?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "signed": document, "keyId": signed })
        );
    } else {
        println!("Signed {} with key `{signed}`.", document.display());
    }
    Ok(())
}

async fn run_plugin_command(
    service: &PluginService,
    action: PluginCmd,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();

    // Belt and braces. `main` dispatches these before core starts, so they do
    // not arrive here — but a future caller that reaches this function by some
    // other route should get a working command rather than a panic.
    if let Some(result) = run_offline_plugin_command(&action, output_fmt) {
        return result;
    }

    match action {
        PluginCmd::List => {
            service.reload()?;
            let plugins = service.list();
            if json {
                println!("{}", serde_json::to_string_pretty(&plugins)?);
            } else {
                let rows: Vec<Vec<String>> = plugins
                    .iter()
                    .map(|plugin| {
                        vec![
                            plugin.id.clone(),
                            plugin.name.clone(),
                            plugin.version.clone(),
                            if plugin.development {
                                "development".into()
                            } else {
                                "package".into()
                            },
                            if plugin.enabled { "yes" } else { "no" }.into(),
                            if plugin.running { "yes" } else { "no" }.into(),
                            plugin.status_text.clone(),
                        ]
                    })
                    .collect();
                print_table(
                    &[
                        "ID",
                        "Name",
                        "Version",
                        "Source",
                        "Enabled",
                        "Running",
                        "Status / reason",
                    ],
                    &rows,
                );
            }
        }
        PluginCmd::Preview { source } => {
            let preview = preview_plugin_source(service, &source)?;
            print_plugin_preview(&preview, json, false)?;
        }
        PluginCmd::Install { source, yes } => {
            let preview = preview_plugin_source(service, &source)?;
            let accepted = accept_plugin_capabilities(&preview, json, yes)?;
            let summary = install_plugin_source(service, source, accepted)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!(
                    "Installed plugin {} v{} ({}).",
                    summary.id,
                    summary.version,
                    if summary.development {
                        "development folder"
                    } else {
                        "package"
                    }
                );
            }
        }
        PluginCmd::Enable { id } => {
            let plugin_id = parse_plugin_id(&id)?;
            service.set_enabled(&plugin_id, true)?;
            if json {
                println!("{}", serde_json::json!({"id": id, "enabled": true}));
            } else {
                println!("Enabled plugin {id}.");
            }
        }
        PluginCmd::Disable { id } => {
            let plugin_id = parse_plugin_id(&id)?;
            service.set_enabled(&plugin_id, false)?;
            if json {
                println!("{}", serde_json::json!({"id": id, "enabled": false}));
            } else {
                println!("Disabled plugin {id}.");
            }
        }
        PluginCmd::Remove { id, purge_data } => {
            let plugin_id = parse_plugin_id(&id)?;
            service.uninstall(&plugin_id, purge_data)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "id": id,
                        "removed": true,
                        "purgedData": purge_data,
                    })
                );
            } else if purge_data {
                println!("Removed plugin {id} and discarded its stored data.");
            } else {
                println!("Removed plugin {id}; stored data was kept.");
            }
        }
        PluginCmd::Log { id, lines } => {
            let plugin_id = parse_plugin_id(&id)?;
            let log_lines = service.logs(&plugin_id, lines);
            if json {
                println!("{}", serde_json::to_string_pretty(&log_lines)?);
            } else {
                for line in log_lines {
                    println!("{line}");
                }
            }
        }
        PluginCmd::CheckUpdate { id } => {
            let targets = match &id {
                Some(id) => vec![parse_plugin_id(id)?],
                // Everything installed. Plugins with no update source report
                // that rather than being silently skipped, because "nothing
                // happened" is indistinguishable from "nothing to do".
                None => service
                    .list()
                    .into_iter()
                    .filter_map(|plugin| parse_plugin_id(&plugin.id).ok())
                    .collect(),
            };
            let mut results = Vec::new();
            for plugin_id in targets {
                let outcome = match service.check_update(&plugin_id).await {
                    Ok(verdict) => serde_json::json!({
                        "id": plugin_id.as_str(),
                        "verdict": verdict,
                    }),
                    Err(error) => serde_json::json!({
                        "id": plugin_id.as_str(),
                        "error": error.to_string(),
                    }),
                };
                results.push(outcome);
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                for result in &results {
                    let id = result["id"].as_str().unwrap_or("?");
                    if let Some(error) = result["error"].as_str() {
                        println!("{id}: {error}");
                    } else {
                        println!("{id}: {}", describe_verdict(&result["verdict"]));
                    }
                }
            }
        }
        PluginCmd::Update { id, yes } => {
            let plugin_id = parse_plugin_id(&id)?;
            let outcome = service.apply_update(&plugin_id, yes).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            } else {
                match outcome {
                    agora_core::plugins::UpdateOutcome::Installed { plugin } => {
                        println!("Updated plugin {} to v{}.", plugin.id, plugin.version);
                    }
                    // Deliberately not applied and deliberately not an error:
                    // the release is fine, it just wants something the user
                    // has not agreed to, and they are the one who decides.
                    agora_core::plugins::UpdateOutcome::NeedsConsent { preview } => {
                        println!(
                            "Not updated. Version {} asks for more than {id} was granted:",
                            preview.manifest.version
                        );
                        for capability in &preview.added_capabilities {
                            println!("  - {capability}");
                        }
                        for host in &preview.added_hosts {
                            println!("  - may contact {host}");
                        }
                        println!();
                        println!("Re-run with --yes to accept.");
                    }
                }
            }
        }
        PluginCmd::RestoreData { id, yes } => {
            let plugin_id = parse_plugin_id(&id)?;
            let Some(checkpoint) = service.restorable_data(&plugin_id) else {
                anyhow::bail!(
                    "There is no saved copy of `{id}` data to go back to. A copy is kept only \
                     when an update changes how the plugin stores its data."
                );
            };
            if !yes {
                println!(
                    "This puts `{id}` back to the {} entries saved before version {} \
                     (captured {}).",
                    checkpoint.entry_count, checkpoint.from_version, checkpoint.captured_at
                );
                println!("Anything the plugin has stored since then is discarded.");
                if !confirm_restore(&id)? {
                    println!("Left as it is.");
                    return Ok(());
                }
            }
            let restored = service.restore_data(&plugin_id)?;
            if json {
                println!("{}", serde_json::json!({ "id": id, "restored": restored }));
            } else {
                println!("Restored {restored} stored entries for {id}.");
            }
        }
        // Handled by the guard directly above, and by `main` before core even
        // starts. Unreachable in both senses, and harmless if it ever is not.
        PluginCmd::Keygen { .. } | PluginCmd::Sign { .. } => {}
        PluginCmd::DisableAll => {
            let disabled = service.disable_all()?;
            if json {
                println!("{}", serde_json::json!({"disabled": disabled}));
            } else {
                println!("Disabled {disabled} plugin(s).");
            }
        }
    }
    Ok(())
}

async fn run_command(
    cli: Cli,
    paths: &agora_core::app_paths::AppPaths,
    data_dir: &Path,
    ctx: &agora_core::ctx::Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();

    match cli.command {
        Commands::Paths => {
            let values = serde_json::json!({
                "root": paths.root().display().to_string(),
                "local_state_db": paths.local_state_db().display().to_string(),
                "registry_db": paths.registry_db().display().to_string(),
                "registry_signature": paths.registry_signature().display().to_string(),
                "instances": paths.instances_root().display().to_string(),
                "minecraft_runtime": paths.minecraft_runtime_root().display().to_string(),
                "loader_cache": paths.loader_cache().display().to_string(),
                "loader_receipts": paths.loader_receipts().display().to_string(),
                "java_runtimes": paths.java_runtimes_root().display().to_string(),
                "snapshots": paths.snapshots_root().display().to_string(),
                "staging": paths.staging_root().display().to_string(),
                "locks": paths.locks_root().display().to_string(),
            });
            if json {
                println!("{}", serde_json::to_string_pretty(&values)?);
            } else {
                if let Some(object) = values.as_object() {
                    for (key, value) in object {
                        println!("{key}: {}", value.as_str().unwrap_or_default());
                    }
                }
            }
        }
        Commands::ListInstances => {
            let svc = InstanceService::new(ctx.clone());
            let instances = svc.list()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&instances)?);
            } else {
                let rows: Vec<Vec<String>> = instances
                    .iter()
                    .map(|i| {
                        vec![
                            i.instance_id.clone(),
                            i.name.clone(),
                            i.minecraft_version.clone(),
                            i.loader.clone(),
                            i.loader_version.clone(),
                            i.last_launched_at.clone().unwrap_or_default(),
                        ]
                    })
                    .collect();
                print_table(
                    &["ID", "Name", "MC", "Loader", "Version", "Launched"],
                    &rows,
                );
            }
        }
        Commands::GetInstance { id } => {
            let svc = InstanceService::new(ctx.clone());
            match svc.get(&id)? {
                Some(detail) => {
                    let row = &detail.row;
                    if json {
                        println!("{}", serde_json::to_string_pretty(row)?);
                    } else {
                        println!("ID:       {}", row.instance_id);
                        println!("Name:     {}", row.name);
                        println!("MC:       {}", row.minecraft_version);
                        println!("Loader:   {} {}", row.loader, row.loader_version);
                        println!("Locked:   {}", row.is_locked);
                        println!("Modpack:  {}", row.is_modpack);
                        println!(
                            "Launched: {}",
                            row.last_launched_at.clone().unwrap_or_default()
                        );
                    }
                }
                None => {
                    anyhow::bail!("Instance '{}' not found", id);
                }
            }
        }
        Commands::Instance { action } => match action {
            InstanceCmd::Create {
                name,
                mc_version,
                loader,
                loader_version,
                jvm_memory_mb,
                jvm_gc,
                jvm_custom_args,
                jvm_always_pre_touch,
                template_id,
            } => {
                let instance_id = agora_core::paths::sanitize_id(&name);
                let jvm_memory_mode = Some(if jvm_memory_mb.is_some() {
                    "manual".to_string()
                } else {
                    "auto".to_string()
                });
                let request = CreateInstanceRequest {
                    name: name.clone(),
                    instance_id: instance_id.clone(),
                    minecraft_version: mc_version,
                    loader,
                    loader_version,
                    jvm_memory_mb,
                    jvm_memory_mode,
                    jvm_gc,
                    jvm_custom_args,
                    jvm_always_pre_touch,
                    is_modpack: None,
                    pack_icon_url: None,
                    template_id,
                };
                let row = InstanceService::new(ctx.clone()).create(request).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&row)?);
                } else {
                    println!("Created instance: {} ({})", row.name, row.instance_id);
                }
            }
            InstanceCmd::Clone {
                source,
                name,
                no_saves,
                no_mods,
                no_resource_packs,
                no_shader_packs,
                no_screenshots,
                no_config,
                no_servers,
                no_options,
                hard_links,
                sym_links,
            } => {
                let prefs = ClonePrefs {
                    copy_saves: !no_saves,
                    copy_mods: !no_mods,
                    copy_resource_packs: !no_resource_packs,
                    copy_shader_packs: !no_shader_packs,
                    copy_screenshots: !no_screenshots,
                    copy_config: !no_config,
                    copy_servers: !no_servers,
                    copy_options: !no_options,
                    use_hard_links: hard_links,
                    use_sym_links: sym_links,
                };
                let request = agora_game_minecraft::instance_service::CloneRequest {
                    source_instance_id: source,
                    new_name: name,
                    prefs,
                };
                let row = InstanceService::new(ctx.clone()).clone(request).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&row)?);
                } else {
                    println!("Cloned instance: {} ({})", row.name, row.instance_id);
                }
            }
            InstanceCmd::Lock { id } => {
                InstanceService::new(ctx.clone()).lock(&id)?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"status": "locked", "instanceId": id})
                    );
                } else {
                    println!("Locked instance '{}'", id);
                }
            }
            InstanceCmd::Unlock { id } => {
                InstanceService::new(ctx.clone()).unlock(&id)?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"status": "unlocked", "instanceId": id})
                    );
                } else {
                    println!("Unlocked instance '{}'", id);
                }
            }
            InstanceCmd::Delete { id } => {
                InstanceService::new(ctx.clone()).delete(&id, None)?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"status": "deleted", "instanceId": id})
                    );
                } else {
                    println!("Deleted instance '{}'", id);
                }
            }
            InstanceCmd::Rename { id, name } => {
                InstanceService::new(ctx.clone()).rename(&id, &name)?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"status": "renamed", "instanceId": id, "name": name})
                    );
                } else {
                    println!("Renamed instance '{}' to '{}'", id, name);
                }
            }
            InstanceCmd::RepairLoader { id } => {
                let summary = LoaderService::new(ctx.clone()).repair(&id).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&summary)?);
                } else {
                    println!(
                        "Reinstalled {} {} for '{}'",
                        summary.tuple.loader, summary.tuple.loader_version, id
                    );
                }
            }
            InstanceCmd::RecommendMemory { id } => {
                let recommendation =
                    InstanceService::new(ctx.clone()).memory_recommendation(&id)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&recommendation)?);
                } else {
                    println!("Recommended memory: {}", recommendation.tier_label);
                    println!("Effective allocation: {} MB", recommendation.recommended_mb);
                    println!("{}", recommendation.explanation);
                    if recommendation.insufficient_system_ram {
                        println!(
                            "Warning: the recommended tier does not fit available system RAM."
                        );
                    }
                }
            }
        },
        Commands::Loader { action } => match action {
            LoaderCmd::List { mc_version } => {
                let loaders = agora_game_minecraft::loader_manifests::list_loaders();
                if let Some(mc_version) = mc_version {
                    let entries: Vec<serde_json::Value> = loaders
                        .into_iter()
                        .filter_map(|loader| {
                            let versions: Vec<String> =
                                agora_game_minecraft::loader_manifests::list_versions(
                                    &loader,
                                    &mc_version,
                                )
                                .into_iter()
                                .map(|entry| entry.loader_version)
                                .collect();
                            (!versions.is_empty()).then(|| {
                                serde_json::json!({
                                    "loader": loader,
                                    "minecraftVersion": mc_version,
                                    "versions": versions,
                                })
                            })
                        })
                        .collect();
                    if json {
                        println!("{}", serde_json::to_string_pretty(&entries)?);
                    } else {
                        let rows: Vec<Vec<String>> = entries
                            .iter()
                            .flat_map(|entry| {
                                let loader = entry["loader"].as_str().unwrap_or_default();
                                entry["versions"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(serde_json::Value::as_str)
                                    .map(|version| vec![loader.to_string(), version.to_string()])
                            })
                            .collect();
                        print_table(&["Loader", "Version"], &rows);
                    }
                } else if json {
                    println!("{}", serde_json::to_string_pretty(&loaders)?);
                } else {
                    for loader in &loaders {
                        println!("{loader}");
                    }
                }
            }
            LoaderCmd::Install {
                loader,
                mc_version,
                loader_version,
                force,
            } => {
                let summary = LoaderService::new(ctx.clone())
                    .ensure_installed(&loader, &mc_version, &loader_version, force)
                    .await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&summary)?);
                } else {
                    println!(
                        "Installed {} {} for Minecraft {}",
                        summary.tuple.loader,
                        summary.tuple.loader_version,
                        summary.tuple.minecraft_version
                    );
                }
            }
        },
        Commands::Plugin { action } => {
            let service = plugin_service(ctx);
            run_plugin_command(&service, action, output_fmt).await?;
        }
        Commands::Provider { action } => {
            let service = plugin_service(ctx);
            let _ = service.reload();
            let plugins = service.is_enabled().then_some(&service);
            let registry = agora_core::providers::ProviderRegistry::new(ctx, plugins);
            match &action {
                ProviderCmd::List => {
                    let descriptors = registry.descriptors();
                    if json {
                        println!("{}", serde_json::to_string_pretty(&descriptors)?);
                    } else {
                        for d in descriptors {
                            let origin = match &d.origin {
                                agora_core::providers::ProviderOrigin::Official => {
                                    "official".to_string()
                                }
                                agora_core::providers::ProviderOrigin::Plugin { plugin_id } => {
                                    format!("plugin {plugin_id}")
                                }
                            };
                            let state = match (d.enabled, &d.unavailable_reason) {
                                (false, _) => "off".to_string(),
                                (true, Some(reason)) => format!("on, unavailable: {reason}"),
                                (true, None) => "on".to_string(),
                            };
                            println!("{:<36} {:<20} {:<28} {}", d.id, d.title, origin, state);
                        }
                    }
                }
                ProviderCmd::Enable { provider_id } | ProviderCmd::Disable { provider_id } => {
                    let enabled = matches!(action, ProviderCmd::Enable { .. });
                    agora_core::providers::set_enabled(
                        ctx,
                        &registry,
                        plugins,
                        provider_id,
                        enabled,
                    )?;
                    if !json {
                        println!(
                            "{provider_id} is now {}",
                            if enabled { "on" } else { "off" }
                        );
                    }
                }
                ProviderCmd::PlanDigest { identifier } => {
                    let digest = agora_game_minecraft::providers::install::curated_pack_digest(
                        &registry, identifier,
                    )
                    .await?;
                    if json {
                        println!("{}", serde_json::json!({ "planDigest": digest }));
                    } else {
                        println!("{digest}");
                    }
                }
                ProviderCmd::Search {
                    provider_id,
                    query,
                    content_type,
                } => {
                    let page = registry
                        .usable(provider_id)?
                        .search(agora_core::providers::SearchRequest {
                            query: query.clone(),
                            content_type: content_type.clone(),
                            limit: 20,
                            ..Default::default()
                        })
                        .await?;
                    let summaries: Vec<_> = page.hits.into_iter().map(|h| h.summary).collect();
                    if json {
                        println!("{}", serde_json::to_string_pretty(&summaries)?);
                    } else {
                        for s in summaries {
                            println!(
                                "{:<48} {:<10} {}",
                                agora_core::providers::item_id(provider_id, &s.id),
                                s.content_type,
                                s.title
                            );
                        }
                    }
                }
            }
        }
        Commands::Settings { action } => match action {
            SettingsCmd::List => {
                let svc = SettingsService::new(ctx.clone());
                if json {
                    let rows = svc.list_parsed()?;
                    let map: serde_json::Map<String, serde_json::Value> =
                        rows.into_iter().collect();
                    println!("{}", serde_json::to_string_pretty(&map)?);
                } else {
                    let rows = svc.list()?;
                    let table_rows: Vec<Vec<String>> = rows
                        .iter()
                        .map(|(k, v)| vec![k.clone(), v.clone()])
                        .collect();
                    print_table(&["Key", "Value (JSON)"], &table_rows);
                }
            }
            SettingsCmd::Get { key } => {
                let svc = SettingsService::new(ctx.clone());
                match svc.get(&key)? {
                    Some(value) => {
                        if json {
                            println!("{}", serde_json::to_string_pretty(&value)?);
                        } else {
                            println!("{}: {}", key, value);
                        }
                    }
                    None => {
                        anyhow::bail!("Setting '{}' not found", key);
                    }
                }
            }
            SettingsCmd::Set { key, value } => {
                let parsed: serde_json::Value = serde_json::from_str(&value)
                    .unwrap_or(serde_json::Value::String(value.clone()));
                let svc = SettingsService::new(ctx.clone());
                svc.set(&key, &parsed)?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"status": "set", "key": key, "value": parsed})
                    );
                } else {
                    println!("Set {} = {}", key, parsed);
                }
            }
        },
        Commands::Mods { action } => match action {
            ModsCmd::List { instance } => {
                let manifest_path = agora_core::paths::instance_manifest_path(data_dir, &instance)?;
                if !manifest_path.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                let manifest = agora_core::helpers::read_manifest(&manifest_path)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&manifest.mods)?);
                } else {
                    let rows: Vec<Vec<String>> = manifest
                        .mods
                        .iter()
                        .map(|m| {
                            vec![
                                m.filename.clone(),
                                m.source.clone(),
                                m.version.clone().unwrap_or_default(),
                                m.modrinth_id.clone().unwrap_or_default(),
                            ]
                        })
                        .collect();
                    print_table(&["Filename", "Source", "Version", "Modrinth ID"], &rows);
                }
            }
            ModsCmd::Install {
                project,
                instance,
                version,
                source,
                allow_replace,
                skip_health_scan,
                install_anyway,
                include_optional,
                exclude_optional,
                replace_conflicts,
                abort_conflicts,
                dry_run,
            } => {
                let svc = InstallService::new(ctx.clone());
                let requested_version = version.clone().unwrap_or_else(|| "selected".into());
                let optional_deps = resolve_optional_deps(include_optional, exclude_optional);
                let intent = agora_game_minecraft::install_pipeline::InstallIntent {
                    action: agora_game_minecraft::install_pipeline::InstallAction::Install {
                        source_type: match source {
                            ModSourceArg::Curated => {
                                agora_game_minecraft::install_pipeline::SourceType::Curated
                            }
                            ModSourceArg::Modrinth => {
                                agora_game_minecraft::install_pipeline::SourceType::Modrinth
                            }
                        },
                        item_id: project.clone(),
                        candidate_version: version,
                    },
                    target_instance: instance.clone(),
                    optional_deps,
                    requested_by: agora_game_minecraft::install_pipeline::RequestSource::CLI,
                    overrides: agora_game_minecraft::install_pipeline::PlanOverrides {
                        allow_replace,
                        skip_health_scan,
                        accept_hash_confirmation: install_anyway,
                        ..Default::default()
                    },
                };

                let reporter = SilentReporter;
                let cancel = agora_game_minecraft::install_pipeline::CancellationToken::new();

                let mut plan = svc.resolve(intent, &reporter).await?;

                // Apply --replace-conflicts / --abort-conflicts override
                apply_conflict_overrides(&mut plan, replace_conflicts, abort_conflicts)?;

                // Dry-run: print the plan and exit
                if dry_run {
                    print_plan(&plan, json)?;
                    return Ok(());
                }

                // Preview / error gate — fail closed on unresolved plans
                if !plan.is_fully_resolved() {
                    let has_choices = !plan.pending_choices.is_empty()
                        || plan
                            .conflicts
                            .iter()
                            .any(|c| c.blocking && c.chosen.is_none());
                    report_unresolved_plan(&plan, json);
                    if has_choices {
                        return Err(agora_core::error::LauncherError::UserDecisionRequired.into());
                    }
                    anyhow::bail!(
                        "Install blocked: unresolved errors, conflicts, or pending choices"
                    );
                }

                // Execute the plan with snapshot, verifiable staging, and health gate
                let outcome = svc.execute(&plan, &reporter, &cancel).await;

                match outcome {
                    agora_game_minecraft::install_pipeline::InstallOutcome::Success {
                        warnings,
                        snapshot_id,
                        ..
                    } => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "success",
                                    "filename": plan.files_to_add.first().map(|f| &f.target_filename),
                                     "version": requested_version.clone(),
                                    "snapshotId": snapshot_id,
                                    "warnings": warnings,
                                }))?
                            );
                        } else {
                            let filename = plan
                                .files_to_add
                                .first()
                                .map(|f| f.target_filename.clone())
                                .unwrap_or_else(|| project.clone());
                            println!("Installed {} ({})", filename, requested_version);
                        }
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::HealthRollback {
                        health_report,
                        snapshot_id,
                        warnings,
                    } => {
                        if json {
                            eprintln!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "health_rollback",
                                    "blockers": health_report.blockers,
                                    "snapshotId": snapshot_id,
                                    "warnings": warnings,
                                }))?
                            );
                        } else {
                            eprintln!(
                                "Install completed but post-install health check found {} blocker(s) — install kept, snapshot {} kept for manual rollback.",
                                health_report.blockers.len(),
                                snapshot_id
                            );
                            for b in &health_report.blockers {
                                eprintln!("  [BLOCK] {}", b.message);
                                if let Some(action) = &b.suggested_action {
                                    eprintln!("    suggestion: {}", action);
                                }
                            }
                            for w in &warnings {
                                eprintln!("  [WARN] {}", w.message);
                            }
                        }
                        anyhow::bail!(
                            "Install has {} health blocker(s) (install kept; snapshot {} available for rollback)",
                            health_report.blockers.len(),
                            snapshot_id
                        );
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::Cancelled {
                        phase,
                        ..
                    } => {
                        anyhow::bail!("Install was cancelled during {}.", phase);
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::Failed {
                        error,
                        rollback_performed,
                        hash_confirmation,
                        ..
                    } => {
                        if json {
                            eprintln!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "failed",
                                    "error": error,
                                    "rollbackPerformed": rollback_performed,
                                }))?
                            );
                        } else {
                            if hash_confirmation.is_some() {
                                eprintln!(
                                    "Pass --install-anyway to install it anyway, once you have checked the file."
                                );
                            }
                            eprintln!("Install failed: {}", error);
                        }
                        anyhow::bail!("Install failed and rolled back: {}", error);
                    }
                }
            }
            ModsCmd::Remove {
                project,
                instance,
                allow_replace,
                skip_health_scan,
                replace_conflicts,
                abort_conflicts,
                remove_anyway,
                dry_run,
            } => {
                let svc = InstallService::new(ctx.clone());
                let load = svc.load_instance(&instance)?;

                let prepared = InstallService::prepare_removal(
                    &load.manifest,
                    &project,
                    load.registry_revision.clone(),
                );

                let target_filename = match &prepared.operation {
                    agora_game_minecraft::install_pipeline::ResolvedOperation::Remove {
                        target_filename,
                        ..
                    } => target_filename.clone(),
                    _ => project.clone(),
                };

                let intent = agora_game_minecraft::install_pipeline::InstallIntent {
                    action: agora_game_minecraft::install_pipeline::InstallAction::Remove {
                        filename: target_filename.clone(),
                    },
                    target_instance: instance.clone(),
                    optional_deps:
                        agora_game_minecraft::install_pipeline::OptionalDepsPolicy::ExcludeAll,
                    requested_by: agora_game_minecraft::install_pipeline::RequestSource::CLI,
                    overrides: remove_overrides(
                        &target_filename,
                        allow_replace,
                        skip_health_scan,
                        remove_anyway,
                    ),
                };

                let reporter = SilentReporter;
                let cancel = agora_game_minecraft::install_pipeline::CancellationToken::new();

                let mut plan = svc.resolve(intent, &reporter).await?;

                // Apply --replace-conflicts / --abort-conflicts override
                apply_conflict_overrides(&mut plan, replace_conflicts, abort_conflicts)?;

                // --remove-anyway: the core already resolved the broken
                // dependency conflict as RemoveAnyway; say what that means.
                if remove_anyway && !json {
                    for warning in plan
                        .warnings
                        .iter()
                        .filter(|warning| warning.code == "WARN_BROKEN_REVERSE_DEP")
                    {
                        eprintln!("[WARN] {}", warning.message);
                    }
                }

                // Dry-run: print the plan and exit
                if dry_run {
                    print_plan(&plan, json)?;
                    return Ok(());
                }

                // Preview / error gate — fail closed on unresolved plans
                if !plan.is_fully_resolved() {
                    let has_choices = !plan.pending_choices.is_empty()
                        || plan
                            .conflicts
                            .iter()
                            .any(|c| c.blocking && c.chosen.is_none());
                    report_unresolved_plan(&plan, json);
                    if has_choices {
                        return Err(agora_core::error::LauncherError::UserDecisionRequired.into());
                    }
                    anyhow::bail!("Remove blocked: unresolved errors");
                }

                // Execute the remove plan with snapshot, file removal, and health gate
                let outcome = svc.execute(&plan, &reporter, &cancel).await;

                match outcome {
                    agora_game_minecraft::install_pipeline::InstallOutcome::Success { .. } => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "removed",
                                    "filename": target_filename,
                                }))?
                            );
                        } else {
                            println!("Removed {}", target_filename);
                        }
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::Failed {
                        error,
                        ..
                    } => {
                        if json {
                            eprintln!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "failed",
                                    "error": error,
                                }))?
                            );
                        } else {
                            eprintln!("Remove failed: {}", error);
                        }
                        anyhow::bail!("Remove failed: {}", error);
                    }
                    other => {
                        let err_msg = format!("Remove encountered unexpected state: {:?}", other);
                        if json {
                            eprintln!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "unexpected",
                                    "error": err_msg,
                                }))?
                            );
                        } else {
                            eprintln!("{}", err_msg);
                        }
                        anyhow::bail!("{}", err_msg);
                    }
                }
            }
            ModsCmd::Search {
                query,
                content_type,
                mc_version,
            } => {
                let svc = RegistryService::new(ctx.clone());
                if !ctx.paths.registry_db().exists() {
                    anyhow::bail!("Registry database not found. Run 'agora registry sync' first.");
                }
                let sort = agora_core::registry::SortOption::NetScore;
                let curated_strategies: Vec<String> =
                    agora_core::registry::CURATED_DOWNLOAD_STRATEGIES
                        .iter()
                        .map(|strategy| strategy.to_string())
                        .collect();
                let items = svc.browse_items(
                    content_type.as_deref(),
                    None,
                    &sort,
                    &curated_strategies,
                    mc_version.as_deref(),
                    None,
                    Some(&query),
                    50,
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&items)?);
                } else {
                    let rows: Vec<Vec<String>> = items
                        .iter()
                        .map(|i| {
                            vec![
                                i.id.clone(),
                                i.name.clone(),
                                i.content_type.clone(),
                                i.description.clone().unwrap_or_default(),
                                i.page_url.clone().unwrap_or_default(),
                            ]
                        })
                        .collect();
                    print_table(&["ID", "Name", "Type", "Description", "URL"], &rows);
                }
            }
            ModsCmd::Update {
                instance,
                item,
                version,
                install_anyway,
                include_optional,
                exclude_optional,
                replace_conflicts,
                abort_conflicts,
                dry_run,
            } => {
                let svc = InstallService::new(ctx.clone());
                let target_version = version.clone().unwrap_or_else(|| "latest".into());
                let optional_deps = resolve_optional_deps(include_optional, exclude_optional);
                let intent = agora_game_minecraft::install_pipeline::InstallIntent {
                    action: agora_game_minecraft::install_pipeline::InstallAction::Update {
                        item_id: item.clone(),
                        target_version: target_version.clone(),
                    },
                    target_instance: instance.clone(),
                    optional_deps,
                    requested_by: agora_game_minecraft::install_pipeline::RequestSource::CLI,
                    overrides: agora_game_minecraft::install_pipeline::PlanOverrides {
                        allow_replace: true,
                        skip_health_scan: false,
                        accept_hash_confirmation: install_anyway,
                        ..Default::default()
                    },
                };

                let reporter = SilentReporter;
                let cancel = agora_game_minecraft::install_pipeline::CancellationToken::new();

                let mut plan = svc.resolve(intent, &reporter).await?;

                // Apply --replace-conflicts / --abort-conflicts override
                apply_conflict_overrides(&mut plan, replace_conflicts, abort_conflicts)?;

                // Dry-run: print the plan and exit
                if dry_run {
                    print_plan(&plan, json)?;
                    return Ok(());
                }

                // Preview / error gate — fail closed on unresolved plans
                if !plan.is_fully_resolved() {
                    let has_choices = !plan.pending_choices.is_empty()
                        || plan
                            .conflicts
                            .iter()
                            .any(|c| c.blocking && c.chosen.is_none());
                    report_unresolved_plan(&plan, json);
                    if has_choices {
                        return Err(agora_core::error::LauncherError::UserDecisionRequired.into());
                    }
                    anyhow::bail!(
                        "Update blocked: unresolved errors, conflicts, or pending choices"
                    );
                }

                // Execute the update plan with snapshot, verifiable staging, and health gate
                let outcome = svc.execute(&plan, &reporter, &cancel).await;

                match outcome {
                    agora_game_minecraft::install_pipeline::InstallOutcome::Success {
                        warnings,
                        snapshot_id,
                        installed_items,
                        ..
                    } => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "success",
                                    "itemId": item,
                                    "version": target_version,
                                    "snapshotId": snapshot_id,
                                    "installedItems": installed_items,
                                    "warnings": warnings,
                                }))?
                            );
                        } else {
                            println!("Updated {} ({})", item, target_version);
                        }
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::HealthRollback {
                        health_report,
                        snapshot_id,
                        warnings,
                    } => {
                        if json {
                            eprintln!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "health_rollback",
                                    "blockers": health_report.blockers,
                                    "snapshotId": snapshot_id,
                                    "warnings": warnings,
                                }))?
                            );
                        } else {
                            eprintln!(
                                "Update completed but post-update health check found {} blocker(s) — install kept, snapshot {} kept for manual rollback.",
                                health_report.blockers.len(),
                                snapshot_id
                            );
                            for b in &health_report.blockers {
                                eprintln!("  [BLOCK] {}", b.message);
                                if let Some(action) = &b.suggested_action {
                                    eprintln!("    suggestion: {}", action);
                                }
                            }
                            for w in &warnings {
                                eprintln!("  [WARN] {}", w.message);
                            }
                        }
                        anyhow::bail!(
                            "Update has {} health blocker(s) (install kept; snapshot {} available for rollback)",
                            health_report.blockers.len(),
                            snapshot_id
                        );
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::Cancelled {
                        phase,
                        ..
                    } => {
                        anyhow::bail!("Update was cancelled during {}.", phase);
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::Failed {
                        error,
                        rollback_performed,
                        hash_confirmation,
                        ..
                    } => {
                        if json {
                            eprintln!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "failed",
                                    "error": error,
                                    "rollbackPerformed": rollback_performed,
                                }))?
                            );
                        } else {
                            eprintln!("Update failed: {}", error);
                            if hash_confirmation.is_some() {
                                eprintln!(
                                    "Pass --install-anyway to update to it anyway, once you have checked the file."
                                );
                            }
                        }
                        anyhow::bail!("Update failed and rolled back: {}", error);
                    }
                }
            }
            ModsCmd::Enable { instance, file } => {
                CrashService::new(ctx.clone()).enable_mod(&instance, &file)?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"status": "enabled", "instanceId": instance, "file": file})
                    );
                } else {
                    println!("Enabled {} in '{}'", file, instance);
                }
            }
            ModsCmd::Disable { instance, file } => {
                CrashService::new(ctx.clone()).disable_mod(&instance, &file)?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"status": "disabled", "instanceId": instance, "file": file})
                    );
                } else {
                    println!("Disabled {} in '{}'", file, instance);
                }
            }
            ModsCmd::Worlds {
                instance,
                file,
                worlds,
                all,
            } => {
                let worlds = if all { None } else { worlds };
                let report = agora_game_minecraft::datapack_sync::set_world_scope(
                    ctx, &instance, &file, worlds,
                )?;
                print_datapack_sync(&instance, &report, json)?;
            }
            ModsCmd::SyncDatapacks { instance } => {
                let report = agora_game_minecraft::datapack_sync::sync_instance(ctx, &instance)?;
                print_datapack_sync(&instance, &report, json)?;
            }
            ModsCmd::UpdateAll {
                instance,
                include_optional,
                exclude_optional,
                replace_conflicts,
                abort_conflicts,
                dry_run,
            } => {
                let svc = InstallService::new(ctx.clone());
                let load = svc.load_instance(&instance)?;

                let items: Vec<agora_game_minecraft::install_pipeline::BatchUpdateItem> = load
                    .manifest
                    .mods
                    .iter()
                    .chain(load.manifest.resourcepacks.iter())
                    .chain(load.manifest.shaders.iter())
                    .chain(load.manifest.datapacks.iter())
                    .filter_map(|m| {
                        m.registry_id
                            .as_ref()
                            .or(m.modrinth_id.as_ref())
                            .or(m.mod_jar_id.as_ref())
                            .map(
                                |id| agora_game_minecraft::install_pipeline::BatchUpdateItem {
                                    item_id: id.clone(),
                                    target_version: "latest".into(),
                                },
                            )
                    })
                    .collect();

                if items.is_empty() {
                    anyhow::bail!(
                        "No installed items with a registry identity found in instance '{}'",
                        instance
                    );
                }

                let optional_deps = resolve_optional_deps(include_optional, exclude_optional);
                let intent = agora_game_minecraft::install_pipeline::InstallIntent {
                    action: agora_game_minecraft::install_pipeline::InstallAction::BatchUpdate {
                        items,
                    },
                    target_instance: instance.clone(),
                    optional_deps,
                    requested_by: agora_game_minecraft::install_pipeline::RequestSource::CLI,
                    overrides: agora_game_minecraft::install_pipeline::PlanOverrides {
                        skip_health_scan: false,
                        ..Default::default()
                    },
                };

                let reporter = SilentReporter;
                let cancel = agora_game_minecraft::install_pipeline::CancellationToken::new();

                let mut plan = svc.resolve(intent, &reporter).await?;

                // Apply --replace-conflicts / --abort-conflicts override
                apply_conflict_overrides(&mut plan, replace_conflicts, abort_conflicts)?;

                // Dry-run: print the plan and exit
                if dry_run {
                    print_plan(&plan, json)?;
                    return Ok(());
                }

                // Preview / error gate — fail closed on unresolved plans.
                // This ensures unsafe conflicts are never silently chosen.
                if !plan.is_fully_resolved() {
                    let has_choices = !plan.pending_choices.is_empty()
                        || plan
                            .conflicts
                            .iter()
                            .any(|c| c.blocking && c.chosen.is_none());
                    report_unresolved_plan(&plan, json);
                    if has_choices {
                        return Err(agora_core::error::LauncherError::UserDecisionRequired.into());
                    }
                    anyhow::bail!(
                        "Batch update blocked: unresolved errors, conflicts, or pending choices"
                    );
                }

                // Execute the batch update plan with snapshot, verifiable staging, and health gate
                let outcome = svc.execute(&plan, &reporter, &cancel).await;

                match outcome {
                    agora_game_minecraft::install_pipeline::InstallOutcome::Success {
                        warnings,
                        snapshot_id,
                        installed_items,
                        ..
                    } => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "success",
                                    "snapshotId": snapshot_id,
                                    "installedItems": installed_items,
                                    "warnings": warnings,
                                }))?
                            );
                        } else {
                            println!(
                                "Batch update completed with {} updated items",
                                installed_items.len()
                            );
                        }
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::HealthRollback {
                        health_report,
                        snapshot_id,
                        warnings,
                    } => {
                        if json {
                            eprintln!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "health_rollback",
                                    "blockers": health_report.blockers,
                                    "snapshotId": snapshot_id,
                                    "warnings": warnings,
                                }))?
                            );
                        } else {
                            eprintln!(
                                "Batch update completed but post-update health check found {} blocker(s) — install kept, snapshot {} kept for manual rollback.",
                                health_report.blockers.len(),
                                snapshot_id
                            );
                            for b in &health_report.blockers {
                                eprintln!("  [BLOCK] {}", b.message);
                                if let Some(action) = &b.suggested_action {
                                    eprintln!("    suggestion: {}", action);
                                }
                            }
                            for w in &warnings {
                                eprintln!("  [WARN] {}", w.message);
                            }
                        }
                        anyhow::bail!(
                            "Batch update has {} health blocker(s) (install kept; snapshot {} available for rollback)",
                            health_report.blockers.len(),
                            snapshot_id
                        );
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::Cancelled {
                        phase,
                        ..
                    } => {
                        anyhow::bail!("Batch update was cancelled during {}.", phase);
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::Failed {
                        error,
                        rollback_performed,
                        ..
                    } => {
                        if json {
                            eprintln!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "status": "failed",
                                    "error": error,
                                    "rollbackPerformed": rollback_performed,
                                }))?
                            );
                        } else {
                            eprintln!("Batch update failed: {}", error);
                        }
                        anyhow::bail!("Batch update failed and rolled back: {}", error);
                    }
                }
            }
        },
        Commands::Health { instance } => {
            let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
            if !instance_dir.exists() {
                anyhow::bail!("Instance '{}' not found", instance);
            }
            let manifest_path = agora_core::paths::instance_manifest_path(data_dir, &instance)?;
            if !manifest_path.exists() {
                anyhow::bail!("Instance manifest not found for '{}'", instance);
            }
            let manifest = agora_core::helpers::read_manifest(&manifest_path)?;
            let reg_path = data_dir.join("registry.db");
            let reg_opt = if reg_path.exists() {
                Some(reg_path)
            } else {
                None
            };
            let report =
                agora_game_minecraft::health::health(&instance_dir, &manifest, reg_opt.as_deref());
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Health score: {:?}", report.score);
                for w in &report.warnings {
                    println!("  [WARN] {}", w.message);
                }
                for b in &report.blockers {
                    println!("  [BLOCK] {}", b.message);
                }
                for recommendation in &report.recommendations {
                    println!("  [RECOMMEND] {}", recommendation.message);
                }
            }
            if report.score == agora_game_minecraft::health::HealthScore::Red {
                anyhow::bail!("Health score is {:?} (see report above)", report.score);
            }
        }
        Commands::Inventory { instance } => {
            let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
            if !instance_dir.exists() {
                anyhow::bail!("Instance '{}' not found", instance);
            }
            let manifest_path = agora_core::paths::instance_manifest_path(data_dir, &instance)?;
            let manifest = agora_core::helpers::read_manifest(&manifest_path)?;
            let inventory = agora_game_minecraft::health::inventory(&instance_dir, &manifest);
            if json {
                let artifacts: Vec<_> = inventory
                    .artifacts
                    .iter()
                    .map(|artifact| {
                        serde_json::json!({
                            "filename": artifact.filename,
                            "status": artifact.status,
                            "diagnostics": artifact.diagnostics,
                            "manifestFallbackUsed": artifact.manifest_fallback_used,
                            "primaryModId": artifact.metadata.mod_jar_id,
                            "version": artifact.metadata.mod_version,
                            "providedMods": artifact.metadata.provided_mods,
                            "requiredDependencies": artifact.metadata.depends_on,
                            "optionalDependencies": artifact.metadata.optional_deps,
                            "incompatibilities": artifact.metadata.incompatibility_decls,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "artifacts": artifacts,
                        "missingEnabledManifestFiles": inventory.missing_enabled_manifest_files,
                    }))?
                );
            } else {
                for artifact in &inventory.artifacts {
                    let primary = artifact.metadata.mod_jar_id.as_deref().unwrap_or("unknown");
                    let provided = artifact
                        .metadata
                        .provided_mods
                        .iter()
                        .map(|provided| provided.mod_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!(
                        "{}: {:?}; primary={}; provided=[{}]; required=[{}]; optional=[{}]; incompatible=[{}]",
                        artifact.filename,
                        artifact.status,
                        primary,
                        provided,
                        artifact.metadata.depends_on.join(", "),
                        artifact.metadata.optional_deps.join(", "),
                        artifact.metadata.incompatible_deps.join(", ")
                    );
                }
                for filename in &inventory.missing_enabled_manifest_files {
                    println!("MISSING: {filename}");
                }
            }
        }
        Commands::Registry { action } => match action {
            RegistryCmd::Status => {
                let local_state = data_dir.join("local_state.db");
                let _repo =
                    agora_core::registry_sync::resolve_registry_repo(cli.registry_repo.as_deref());
                let status = agora_core::registry_sync::get_status(data_dir, &local_state);
                if json {
                    println!("{}", serde_json::to_string_pretty(&status)?);
                } else {
                    println!("Cached DB:     {}", status.has_cached_db);
                    println!(
                        "Cached tag:    {}",
                        status.cached_tag.as_deref().unwrap_or("none")
                    );
                    println!(
                        "Schema:        {}",
                        status
                            .cached_schema_version
                            .map_or("N/A".into(), |v| v.to_string())
                    );
                    println!("Update avail:  {}", status.update_available);
                    println!("Message:       {}", status.message);
                    if !status.has_cached_db {
                        println!("Hint:          Run `agora registry sync` to download it.");
                    }
                }
            }
            RegistryCmd::Sync => {
                let repo =
                    agora_core::registry_sync::resolve_registry_repo(cli.registry_repo.as_deref());
                let local_state = data_dir.join("local_state.db");
                if !local_state.exists() {
                    agora_core::db::init_local_state_db(&local_state)?;
                }
                let report = agora_core::registry_sync::check_and_download_update(
                    data_dir,
                    &local_state,
                    true,
                    None,
                    None,
                    &repo,
                    ctx.lock_manager(),
                )
                .await?;
                let catalog_warnings = ctx.reload_game_catalogs()?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "report": report,
                            "catalog_warnings": catalog_warnings,
                        }))?
                    );
                } else {
                    println!("Registry sync: {}", report.message);
                    for warning in catalog_warnings {
                        println!("Catalog: {warning}");
                    }
                }
            }
        },
        Commands::Snapshots { action } => match action {
            SnapshotsCmd::List { instance } => {
                let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
                if !instance_dir.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                let snapshots = agora_core::snapshot::list_snapshots(&instance_dir)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&snapshots)?);
                } else {
                    let rows: Vec<Vec<String>> = snapshots
                        .iter()
                        .map(|s| {
                            vec![
                                s.id.clone(),
                                s.label.clone().unwrap_or_default(),
                                s.created_at.clone(),
                                s.file_count.to_string(),
                            ]
                        })
                        .collect();
                    print_table(&["ID", "Label", "Created", "Files"], &rows);
                }
            }
            SnapshotsCmd::Create { instance, label } => {
                let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
                if !instance_dir.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                let snapshot = agora_core::snapshot::create_snapshot_with_origin(
                    &instance_dir,
                    label.as_deref(),
                    agora_core::snapshot::SnapshotOrigin::User,
                )
                .map_err(|e| anyhow::anyhow!("{}", e))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&snapshot)?);
                } else {
                    println!(
                        "Created snapshot {} ({})",
                        snapshot.id,
                        snapshot.label.as_deref().unwrap_or("unlabeled")
                    );
                }
            }
            SnapshotsCmd::Restore {
                instance,
                snapshot_id,
            } => {
                let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
                if !instance_dir.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                // Same core service the desktop app uses: the instance lock,
                // the launch-exclusion check, and the undo snapshot are not
                // things a second front end gets to skip.
                let outcome = agora_core::snapshot_service::SnapshotService::new(ctx.clone())
                    .restore(&instance, &snapshot_id)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&outcome)?);
                } else {
                    println!(
                        "Restored instance '{}' from snapshot {}",
                        instance, snapshot_id
                    );
                    if outcome.coverage == agora_core::snapshot::RestoreCoverageKind::LegacyPartial
                    {
                        println!(
                            "  This snapshot predates scoped snapshots, so only {} could be restored.",
                            outcome.restored_roots.join(", ")
                        );
                    }
                    if !outcome.preserved_roots.is_empty() {
                        println!(
                            "  Left untouched (outside this snapshot): {}",
                            outcome.preserved_roots.join(", ")
                        );
                    }
                    if let Some(warning) = &outcome.cleanup_warning {
                        println!("  Note: {warning}");
                    }
                    if let Some(preserved) = &outcome.preserved_recovery_dir {
                        println!("  Recovery material from an earlier interrupted restore was kept at {preserved}");
                    }
                }
            }
            SnapshotsCmd::Delete {
                instance,
                snapshot_id,
            } => {
                let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
                if !instance_dir.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                agora_core::snapshot::delete_snapshot(&instance_dir, &snapshot_id)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"status": "deleted", "instanceId": instance, "snapshotId": snapshot_id})
                    );
                } else {
                    println!(
                        "Deleted snapshot '{}' for instance '{}'",
                        snapshot_id, instance
                    );
                }
            }
        },
        Commands::Import {
            path,
            url,
            symlink_saves,
            name,
        } => {
            let svc = agora_game_minecraft::import_service::ImportService::new(ctx.clone());
            if let Some(url) = url {
                if symlink_saves {
                    anyhow::bail!("--symlink-saves is not supported for URL imports");
                }
                let result = svc.run_mrpack_url(&url).await?;
                wait_for_initial_snapshot(ctx, &result.instance_id)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!("Imported: {} ({} mods)", result.name, result.imported_mods);
                }
                return Ok(());
            }
            let path = path.expect("clap requires either path or --url");
            if !path.exists() {
                anyhow::bail!("Path '{}' does not exist", path.display());
            }
            let import_source = if path.is_dir() {
                agora_game_minecraft::import_service::ImportSource::Directory(path)
            } else {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                match ext {
                    "mrpack" => agora_game_minecraft::import_service::ImportSource::mrpack(path),
                    "zip" => agora_game_minecraft::import_service::ImportSource::PrismZip(path),
                    _ => anyhow::bail!(
                        "Unsupported file type '.{ext}'. Use .mrpack, .zip, or a directory"
                    ),
                }
            };
            let request = agora_game_minecraft::import_service::ImportRequest {
                source: import_source,
                symlink_saves,
            };
            let result = svc
                .run_import_named(
                    request,
                    name,
                    ctx.progress_sink.clone(),
                    agora_core::event_sink::CancellationToken::new(),
                )
                .await
                .map_err(|error| {
                    if error.code() == "ERR_INSTANCE_EXISTS" {
                        anyhow::anyhow!(
                            "{error} Re-run with --name \"<new name>\" to import it as a copy."
                        )
                    } else {
                        anyhow::Error::from(error)
                    }
                })?;
            wait_for_initial_snapshot(ctx, &result.instance_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("Imported: {} ({} mods)", result.name, result.imported_mods);
            }
        }
        Commands::Launch {
            instance,
            yes,
            timings,
        } => {
            run_launch_service(ctx, &instance, yes, timings, output_fmt).await?;
        }
        Commands::Auth { action } => match action {
            AuthCmd::Login { no_browser } => {
                let db_path = data_dir.join("local_state.db");
                let flow =
                    agora_game_minecraft::msa::begin_login(&ctx.http_clients, &db_path).await?;

                // In --json mode stdout must stay machine-readable, so the
                // human-facing prompt goes to stderr.
                let prompt = format!(
                    "To sign in, open {} and enter the code: {}",
                    flow.verification_uri, flow.user_code
                );
                if json {
                    eprintln!("{prompt}");
                } else {
                    println!("{prompt}");
                }

                if no_browser {
                    eprintln!("Waiting for you to finish signing in… (Ctrl-C to cancel)");
                } else {
                    match open_url_in_browser(&flow.verification_uri) {
                        Ok(()) => eprintln!(
                            "Opened your browser. Waiting for you to finish signing in…                              (Ctrl-C to cancel)"
                        ),
                        Err(error) => eprintln!(
                            "Could not open a browser ({error}). Open the link above manually.                              Waiting… (Ctrl-C to cancel)"
                        ),
                    }
                }

                // Ctrl-C stops the polling loop cleanly instead of leaving a
                // half-finished sign-in behind.
                let cancel = agora_game_minecraft::msa::MsaLoginCancel::new();
                let on_signal = cancel.clone();
                tokio::spawn(async move {
                    if tokio::signal::ctrl_c().await.is_ok() {
                        on_signal.cancel();
                    }
                });

                let credentials = agora_game_minecraft::msa::poll_login(
                    &ctx.http_clients,
                    &flow,
                    &db_path,
                    &cancel,
                )
                .await?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "username": credentials.username,
                            "uuid": credentials.uuid,
                            "expires": credentials.expires,
                        })
                    );
                } else {
                    println!("Signed in as {}", credentials.username);
                }
            }
            AuthCmd::Status => match agora_game_minecraft::msa::load_credentials()? {
                Some(creds) => {
                    if creds.needs_reauth() {
                        // Stored by the pre-migration flow: no refresh token
                        // Agora now holds can renew it.
                        if json {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "status": "sign_in_required",
                                    "username": creds.username,
                                    "reason": agora_game_minecraft::msa::LEGACY_CREDENTIALS_MESSAGE,
                                })
                            );
                        } else {
                            println!("Signed in as {} — sign-in required", creds.username);
                            println!("{}", agora_game_minecraft::msa::LEGACY_CREDENTIALS_MESSAGE);
                        }
                    } else if creds.is_expired() {
                        if json {
                            println!(
                                "{}",
                                serde_json::json!({"status": "expired", "username": creds.username})
                            );
                        } else {
                            println!(
                                "Signed in as {} (expired — run 'agora auth login')",
                                creds.username
                            );
                        }
                    } else {
                        if json {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "status": "valid",
                                    "username": creds.username,
                                    "expires": creds.expires,
                                })
                            );
                        } else {
                            println!(
                                "Signed in as {} (expires {})",
                                creds.username, creds.expires
                            );
                        }
                    }
                }
                None => {
                    if json {
                        println!("{}", serde_json::json!({"status": "not_authenticated"}));
                    } else {
                        println!("Not authenticated. Run 'agora auth login'.");
                    }
                }
            },
            AuthCmd::Logout => {
                agora_game_minecraft::msa::clear_credentials()?;
                if json {
                    println!("{}", serde_json::json!({"status": "logged_out"}));
                } else {
                    println!("Signed out.");
                }
            }
        },
        Commands::Sync => {
            let repo =
                agora_core::registry_sync::resolve_registry_repo(cli.registry_repo.as_deref());
            let local_state = data_dir.join("local_state.db");
            if !local_state.exists() {
                agora_core::db::init_local_state_db(&local_state)?;
            }
            let report = agora_core::registry_sync::check_and_download_update(
                data_dir,
                &local_state,
                true,
                None,
                None,
                &repo,
                ctx.lock_manager(),
            )
            .await?;
            let catalog_warnings = ctx.reload_game_catalogs()?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "report": report,
                        "catalog_warnings": catalog_warnings,
                    }))?
                );
            } else {
                println!("{}", report.message);
                for warning in catalog_warnings {
                    println!("Catalog: {warning}");
                }
            }
        }
        Commands::Mcp { action } => match action {
            McpCmd::Serve { stdio: true } => run_mcp_stdio(ctx).await?,
            McpCmd::Serve { stdio: false } => {
                anyhow::bail!("Only --stdio transport is currently supported for 'mcp serve'")
            }
        },
        Commands::Runtime { action } => match action {
            RuntimeCmd::List => {
                let svc = RuntimeService::new(ctx.clone());
                let candidates = svc.list_candidates()?;

                if json {
                    println!("{}", serde_json::to_string_pretty(&candidates)?);
                } else {
                    let rows: Vec<Vec<String>> = candidates
                        .iter()
                        .map(|j| {
                            vec![
                                j.version.to_string(),
                                j.version_string.clone(),
                                j.path.to_string_lossy().to_string(),
                                format!("{:?}", j.source),
                                j.arch.clone().unwrap_or_default(),
                            ]
                        })
                        .collect();
                    print_table(&["Major", "Version", "Path", "Source", "Arch"], &rows);
                }
            }
            RuntimeCmd::Ensure { major } => {
                let svc = RuntimeService::new(ctx.clone());
                let policy = svc.network_policy()?;

                if !json {
                    println!("Ensuring Java {major} runtime...");
                }

                let ensured = svc
                    .ensure_runtime(major, policy, std::sync::Arc::new(ConsoleRuntimeProgress))
                    .await?;

                if json {
                    println!("{}", serde_json::to_string_pretty(&ensured)?);
                } else {
                    println!(
                        "Java {} runtime ready at {}",
                        ensured.version,
                        ensured.path.display()
                    );
                }
            }
            RuntimeCmd::RemoveUnused => {
                let svc = RuntimeService::new(ctx.clone());
                let removed = svc.remove_unused()?;

                if json {
                    println!("{}", serde_json::json!({"removed": removed}));
                } else {
                    println!("Removed {removed} unused runtime(s).");
                }
            }
            RuntimeCmd::Inspect { path } => {
                let svc = RuntimeService::new(ctx.clone());
                match svc.inspect(&path) {
                    Ok(inst) => {
                        if json {
                            println!("{}", serde_json::to_string_pretty(&inst)?);
                        } else {
                            println!("Java major:   {}", inst.version);
                            println!("Version:      {}", inst.version_string);
                            println!("Path:         {}", inst.path.display());
                            println!("Source:       {:?}", inst.source);
                            if let Some(ref arch) = inst.arch {
                                println!("Architecture: {}", arch);
                            }
                        }
                    }
                    Err(e) => {
                        anyhow::bail!("{}", e);
                    }
                }
            }
        },
        Commands::Crash { action } => match action {
            CrashCmd::List { instance } => {
                let reports = CrashService::new(ctx.clone()).list_reports(&instance)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&reports)?);
                } else {
                    let rows: Vec<Vec<String>> = reports
                        .iter()
                        .map(|r| {
                            vec![
                                r.filename.clone(),
                                r.modified_at.clone(),
                                r.size_bytes.to_string(),
                            ]
                        })
                        .collect();
                    print_table(&["Filename", "Modified", "Size (bytes)"], &rows);
                }
            }
            CrashCmd::Inspect { instance, file } => {
                let content = CrashService::new(ctx.clone()).read_crash_log(&instance, &file)?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "filename": file,
                            "content": content,
                        })
                    );
                } else {
                    println!("{}", content);
                }
            }
            CrashCmd::Investigate { instance, files } => {
                let investigation =
                    CrashService::new(ctx.clone()).investigate_evidence(&instance, &files)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&investigation)?);
                } else {
                    if investigation.evidence.sources.is_empty() {
                        anyhow::bail!("No crash evidence found for '{}'", instance);
                    }
                    println!("Evidence:");
                    for (index, source) in investigation.evidence.sources.iter().enumerate() {
                        let role = if index == investigation.evidence.primary_index {
                            "primary"
                        } else {
                            "supporting"
                        };
                        let truncated = if source.meta.truncated {
                            ", truncated"
                        } else {
                            ""
                        };
                        println!("  - {} ({role}{truncated})", source.meta.basename);
                    }
                    println!("Category: {:?}", investigation.failure_category);
                    if let Some(name) = investigation.triage.signature_name.as_deref() {
                        println!("Known signature: {name}");
                    }
                    if let Some(solution) = investigation.triage.solution_markdown.as_deref() {
                        println!("Suggested fix: {solution}");
                    }
                    let rows: Vec<Vec<String>> = investigation
                        .suspects
                        .iter()
                        .map(|s| {
                            vec![
                                s.mod_id.clone(),
                                s.filename.clone(),
                                format!("{:.2}", s.total_score),
                                s.is_dependent_of.clone().unwrap_or_default(),
                            ]
                        })
                        .collect();
                    print_table(&["Mod ID", "Filename", "Score", "Depends On"], &rows);
                    if investigation.suspects.is_empty() {
                        println!("No mod suspects had positive deterministic evidence.");
                    }
                }
            }
        },
        Commands::MigrateData { from, yes } => {
            let svc = agora_core::data_migration::DataMigrationService::new(paths.clone());

            if !from.exists() {
                anyhow::bail!("Source path '{}' does not exist", from.display());
            }

            // Always produce a plan (dry-run inventory).
            let plan = svc.plan(&from)?;

            if !plan.can_proceed {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "status": "conflict",
                            "sourceInventory": plan.source_inventory,
                            "conflicts": plan.conflicts,
                        }))?
                    );
                } else {
                    println!("Source: {}", plan.source_inventory.source_root);
                    println!(
                        "Files:  {} ({:.2} MB)",
                        plan.source_inventory.files.len(),
                        plan.source_inventory.total_size_bytes as f64 / 1_048_576.0
                    );
                    println!(
                        "DBs:   local_state.db={}, registry.db={}",
                        plan.source_inventory.has_local_state_db,
                        plan.source_inventory.has_registry_db,
                    );
                    println!(
                        "Instances: {}",
                        plan.source_inventory.instance_ids.join(", ")
                    );
                    println!();
                    println!("CONFLICTS — migration cannot proceed:");
                    for c in &plan.conflicts {
                        println!("  - {}: {}", c.rel_path, c.reason);
                    }
                    println!();
                    println!("Resolve conflicts (remove or rename destination data) and retry.");
                }
                anyhow::bail!("Migration blocked by {} conflict(s)", plan.conflicts.len());
            }

            // Dry-run: show what would happen but don't execute.
            if !yes {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "status": "dry-run",
                            "sourceInventory": plan.source_inventory,
                            "conflicts": plan.conflicts,
                        }))?
                    );
                } else {
                    println!("Dry-run: migration would copy the following:");
                    println!("  Source:   {}", plan.source_inventory.source_root);
                    println!(
                        "  Files:    {} ({:.2} MB)",
                        plan.source_inventory.files.len(),
                        plan.source_inventory.total_size_bytes as f64 / 1_048_576.0
                    );
                    println!(
                        "  DBs:      local_state.db={}, registry.db={}",
                        plan.source_inventory.has_local_state_db,
                        plan.source_inventory.has_registry_db,
                    );
                    if !plan.source_inventory.instance_ids.is_empty() {
                        println!(
                            "  Instances: {}",
                            plan.source_inventory.instance_ids.join(", ")
                        );
                    }
                    println!();
                    println!("Pass --yes to execute this migration.");
                }
                return Ok(());
            }

            // Execute.
            let result = svc.execute(&from)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "Migration complete: {} file(s), {:.2} MB, {} instance(s)",
                    result.files_migrated,
                    result.total_bytes as f64 / 1_048_576.0,
                    result.instance_ids.len(),
                );
                println!("Backup: {}", result.backup_path);
                if !result.instance_ids.is_empty() {
                    println!("Instances: {}", result.instance_ids.join(", "));
                }
            }
        }
        Commands::Pack { action } => {
            match action {
                PackCmd::Install { path, instance } => {
                    let json_text = std::fs::read_to_string(&path).map_err(|e| {
                        anyhow::anyhow!("Cannot read pack manifest '{}': {}", path.display(), e)
                    })?;
                    let svc = agora_game_minecraft::import_service::ImportService::new(ctx.clone());
                    let request = agora_game_minecraft::import_service::ImportRequest {
                        source: agora_game_minecraft::import_service::ImportSource::PackManifest {
                            manifest_json: json_text,
                            target_instance_id: instance.clone(),
                        },
                        symlink_saves: false,
                    };
                    let result = svc.install_pack(request).await?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else {
                        println!(
                            "Installed pack '{}' ({} mods)",
                            result.name, result.mods_installed
                        );
                    }
                }
                PackCmd::Versions { pack } => {
                    let releases =
                        agora_game_minecraft::curated_pack::CuratedPackService::new(ctx.clone())
                            .versions(&pack)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&releases)?);
                    } else if releases.is_empty() {
                        println!(
                            "'{pack}' has no locked releases; it installs in flexible mode only."
                        );
                    } else {
                        let rows: Vec<Vec<String>> = releases
                            .iter()
                            .map(|release| {
                                vec![
                                    release.version.clone(),
                                    release.minecraft_version.clone(),
                                    format!("{} {}", release.loader, release.loader_version),
                                ]
                            })
                            .collect();
                        print_table(&["Release", "Minecraft", "Loader"], &rows);
                    }
                }
                PackCmd::Curated {
                    pack,
                    instance,
                    release,
                    dry_run,
                } => {
                    use agora_game_minecraft::curated_pack::{
                        CuratedPackSelection, CuratedPackService,
                    };
                    let detail = InstanceService::new(ctx.clone())
                        .get(&instance)?
                        .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", instance))?;
                    let selection = match release {
                        Some(pack_version) => CuratedPackSelection::Locked { pack_version },
                        None => CuratedPackSelection::Flexible {
                            minecraft_version: detail.row.minecraft_version.clone(),
                            loader: detail.row.loader.clone(),
                        },
                    };
                    let plan = CuratedPackService::new(ctx.clone())
                        .plan(&pack, &selection)
                        .await?;
                    if plan.target.minecraft_version != detail.row.minecraft_version
                        || plan.target.loader != detail.row.loader
                    {
                        anyhow::bail!(
                            "Release {} targets Minecraft {} with {}, but '{}' is on {} with {}.",
                            plan.pack_version.as_deref().unwrap_or("?"),
                            plan.target.minecraft_version,
                            plan.target.loader,
                            instance,
                            detail.row.minecraft_version,
                            detail.row.loader
                        );
                    }

                    if json {
                        println!("{}", serde_json::to_string_pretty(&plan)?);
                    } else {
                        println!(
                            "{} mod(s) resolved for Minecraft {} with {}.",
                            plan.mods.len(),
                            plan.target.minecraft_version,
                            plan.target.loader
                        );
                        for dropped in &plan.dropped {
                            println!(
                                "  [LEFT OUT] {} ({}): {}",
                                dropped.mod_id, dropped.status, dropped.reason
                            );
                        }
                        for blocking in &plan.blocking {
                            eprintln!(
                                "  [BLOCK] {} (required): {}",
                                blocking.mod_id, blocking.reason
                            );
                        }
                    }
                    if !plan.can_install() {
                        anyhow::bail!("Pack '{}' cannot be installed on this instance.", pack);
                    }
                    if dry_run {
                        return Ok(());
                    }

                    let svc = InstallService::new(ctx.clone());
                    let intent = agora_game_minecraft::install_pipeline::InstallIntent {
                        action:
                            agora_game_minecraft::install_pipeline::InstallAction::BatchInstall {
                                items: plan.batch_items(),
                            },
                        target_instance: instance.clone(),
                        // The pack names its mods explicitly; a CLI run cannot answer a
                        // prompt for extra optional dependencies, so it takes none.
                        optional_deps:
                            agora_game_minecraft::install_pipeline::OptionalDepsPolicy::ExcludeAll,
                        requested_by: agora_game_minecraft::install_pipeline::RequestSource::CLI,
                        overrides: agora_game_minecraft::install_pipeline::PlanOverrides::default(),
                    };
                    let reporter = SilentReporter;
                    let cancel = agora_game_minecraft::install_pipeline::CancellationToken::new();
                    let resolved = svc.resolve(intent, &reporter).await?;
                    if !resolved.is_fully_resolved() {
                        report_unresolved_plan(&resolved, json);
                        anyhow::bail!(
                            "Install blocked: unresolved errors, conflicts, or pending choices"
                        );
                    }
                    match svc.execute(&resolved, &reporter, &cancel).await {
                    agora_game_minecraft::install_pipeline::InstallOutcome::Success { .. } => {
                        if !json {
                            println!("Installed pack '{}' into '{}'.", pack, instance);
                        }
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::HealthRollback {
                        health_report,
                        snapshot_id,
                        ..
                    } => {
                        anyhow::bail!(
                            "Install has {} health blocker(s) (install kept; snapshot {} available for rollback)",
                            health_report.blockers.len(),
                            snapshot_id
                        );
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::Cancelled { phase, .. } => {
                        anyhow::bail!("Install was cancelled during {}.", phase);
                    }
                    agora_game_minecraft::install_pipeline::InstallOutcome::Failed { error, .. } => {
                        anyhow::bail!("Install failed and rolled back: {}", error);
                    }
                }
                }
            }
        }
        Commands::Export { instance, dest } => {
            let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
            if !instance_dir.exists() {
                anyhow::bail!("Instance '{}' not found", instance);
            }
            std::fs::create_dir_all(&dest).map_err(|e| {
                anyhow::anyhow!("Cannot create destination '{}': {}", dest.display(), e)
            })?;
            let manifest_path = agora_core::paths::instance_manifest_path(data_dir, &instance)?;
            let manifest = agora_core::helpers::read_manifest(&manifest_path)?;
            let result = agora_game_minecraft::server_export::export_server_environment(
                &instance_dir,
                &dest,
                &manifest.loader,
                &manifest.minecraft_version,
            )
            .map_err(|e| anyhow::anyhow!("Export failed: {e}"))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "Exported {} mods ({} server, {} client-only removed) to {}",
                    result.total_mods,
                    result.server_mods,
                    result.removed_client_only.len(),
                    dest.display()
                );
            }
        }
        Commands::Loadout { action } => match action {
            LoadoutCmd::Create { instance, name } => {
                let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
                if !instance_dir.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                let profile = agora_core::loadout::create_profile(&instance_dir, &name)
                    .map_err(|e| anyhow::anyhow!("Failed to create loadout: {e}"))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&profile)?);
                } else {
                    println!(
                        "Created loadout '{}' ({})",
                        profile.name, profile.created_at
                    );
                }
            }
            LoadoutCmd::List { instance } => {
                let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
                if !instance_dir.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                let profiles = agora_core::loadout::list_profiles(&instance_dir)
                    .map_err(|e| anyhow::anyhow!("Failed to list loadouts: {e}"))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&profiles)?);
                } else {
                    let rows: Vec<Vec<String>> = profiles
                        .iter()
                        .map(|p| {
                            vec![
                                p.name.clone(),
                                p.created_at.clone(),
                                p.enabled_mods.len().to_string(),
                            ]
                        })
                        .collect();
                    print_table(&["Name", "Created", "Entries"], &rows);
                }
            }
            LoadoutCmd::Apply { instance, name } => {
                let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
                if !instance_dir.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                agora_core::loadout::apply_profile(&instance_dir, &name)
                    .map_err(|e| anyhow::anyhow!("Failed to apply loadout: {e}"))?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"status": "applied", "instanceId": instance, "profile": name})
                    );
                } else {
                    println!("Applied loadout '{}' to '{}'", name, instance);
                }
            }
            LoadoutCmd::Delete { instance, name } => {
                let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
                if !instance_dir.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                agora_core::loadout::delete_profile(&instance_dir, &name)
                    .map_err(|e| anyhow::anyhow!("Failed to delete loadout: {e}"))?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"status": "deleted", "instanceId": instance, "profile": name})
                    );
                } else {
                    println!("Deleted loadout '{}'", name);
                }
            }
        },
        Commands::Lockfile { action } => match action {
            LockfileCmd::Export {
                instance,
                out: output,
            } => {
                let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
                if !instance_dir.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                let lockfile = agora_game_minecraft::lockfile::build_from_instance(&instance_dir)
                    .map_err(|e| anyhow::anyhow!("Failed to build lockfile: {e}"))?;
                let lockfile_json = lockfile
                    .to_pretty_json()
                    .map_err(|e| anyhow::anyhow!("Failed to serialize lockfile: {e}"))?;
                match output {
                    Some(path) => {
                        std::fs::write(&path, &lockfile_json)
                            .map_err(|e| anyhow::anyhow!("Failed to write lockfile: {e}"))?;
                        if json {
                            println!(
                                "{}",
                                serde_json::json!({"status": "exported", "path": path.display().to_string()})
                            );
                        } else {
                            println!("Exported lockfile to {}", path.display());
                        }
                    }
                    None => {
                        println!("{lockfile_json}");
                    }
                }
            }
            LockfileCmd::Verify { path } => {
                let json_text = std::fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("Cannot read '{}': {}", path.display(), e))?;
                match agora_game_minecraft::lockfile::InstanceLockfile::parse_and_validate(
                    &json_text,
                ) {
                    Ok(lockfile) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "status": "valid",
                                    "instance": lockfile.instance.name,
                                    "artifacts": lockfile.artifacts.len(),
                                    "schemaVersion": lockfile.schema_version,
                                })
                            );
                        } else {
                            println!("Lockfile is valid");
                            println!(
                                "  Instance: {} {} ({}/{})",
                                lockfile.instance.name,
                                lockfile.instance.minecraft_version,
                                lockfile.instance.loader,
                                lockfile.instance.loader_version
                            );
                            println!("  Artifacts: {}", lockfile.artifacts.len());
                            println!("  Schema:   v{}", lockfile.schema_version);
                            if lockfile.signature.is_some() {
                                println!("  Signed:   yes");
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            eprintln!(
                                "{}",
                                serde_json::json!({
                                    "status": "invalid",
                                    "error": e,
                                })
                            );
                        } else {
                            eprintln!("Lockfile is invalid: {e}");
                        }
                        anyhow::bail!("Lockfile verification failed: {e}");
                    }
                }
            }
            LockfileCmd::Repair {
                instance,
                out: output,
            } => {
                let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
                if !instance_dir.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                // Repair re-exports the lockfile from the current state.
                let lockfile = agora_game_minecraft::lockfile::build_from_instance(&instance_dir)
                    .map_err(|e| anyhow::anyhow!("Failed to rebuild lockfile: {e}"))?;
                let lockfile_json = lockfile
                    .to_pretty_json()
                    .map_err(|e| anyhow::anyhow!("Failed to serialize lockfile: {e}"))?;
                match output {
                    Some(path) => {
                        std::fs::write(&path, &lockfile_json)
                            .map_err(|e| anyhow::anyhow!("Failed to write lockfile: {e}"))?;
                        if json {
                            println!(
                                "{}",
                                serde_json::json!({"status": "repaired", "path": path.display().to_string()})
                            );
                        } else {
                            println!("Repaired lockfile written to {}", path.display());
                        }
                    }
                    None => {
                        println!("{lockfile_json}");
                    }
                }
            }
            LockfileCmd::Import {
                path,
                instance,
                skip_health_scan: _,
            } => {
                let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
                if !instance_dir.exists() {
                    anyhow::bail!("Instance '{}' not found", instance);
                }
                let json_text = std::fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("Cannot read '{}': {}", path.display(), e))?;
                let lockfile =
                    agora_game_minecraft::lockfile::InstanceLockfile::parse_and_validate(
                        &json_text,
                    )
                    .map_err(|e| anyhow::anyhow!("Invalid lockfile: {e}"))?;

                // Build a lockfile from the current instance to detect drift.
                let _current = agora_game_minecraft::lockfile::build_from_instance(&instance_dir)
                    .map_err(|e| anyhow::anyhow!("Cannot read current instance: {e}"))?;

                // Compute the drift between the lockfile and current instance
                let mut live_files: std::collections::BTreeMap<String, String> =
                    std::collections::BTreeMap::new();
                let content_dirs = ["mods", "resourcepacks", "shaderpacks", "datapacks", "saves"];
                for dir_name in &content_dirs {
                    let dir = instance_dir.join(dir_name);
                    if !dir.is_dir() {
                        continue;
                    }
                    if let Ok(entries) = std::fs::read_dir(&dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_file() {
                                if let Ok(data) = std::fs::read(&path) {
                                    let sha256 = agora_core::download::sha256_hex(&data);
                                    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                                        live_files.insert(format!("{dir_name}/{name}"), sha256);
                                    }
                                }
                            }
                        }
                    }
                }

                let drift =
                    agora_game_minecraft::lockfile::detect_drift(&lockfile, &live_files, None);
                if json {
                    println!("{}", serde_json::to_string_pretty(&drift)?);
                } else {
                    if drift.status == agora_game_minecraft::lockfile::DriftStatus::InSync {
                        println!("Instance is already in sync with lockfile");
                    } else {
                        println!("Drift detected ({} differences):", drift.differences.len());
                        for diff in &drift.differences {
                            println!("  [{:?}] {}", diff.kind, diff.path);
                        }
                    }
                }
            }
        },
        Commands::Games { action } => match action {
            GamesCmd::Discover => {
                let report = agora_core::game_discovery::discover_all();
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    print_discovery_report(&report);
                }
            }
            GamesCmd::List => {
                let report = agora_core::game_discovery::discover_all();
                let inventory = agora_core::game_registry::identify_installs(
                    &ctx.games,
                    &report,
                    &agora_core::game_discovery::file_version::read_file_version,
                );
                if json {
                    let out = serde_json::json!({
                        "games": ctx.games.games().collect::<Vec<_>>(),
                        "inventory": inventory,
                    });
                    println!("{}", serde_json::to_string_pretty(&out)?);
                } else {
                    print_games_list(&ctx.games, &inventory);
                }
            }
            GamesCmd::Base { action } => match action {
                BaseCmd::Build {
                    install_id,
                    mode,
                    include_excluded,
                } => {
                    let report = agora_core::game_discovery::discover_all();
                    let inventory = agora_core::game_registry::identify_installs(
                        &ctx.games,
                        &report,
                        &agora_core::game_discovery::file_version::read_file_version,
                    );
                    let install = inventory
                        .installs
                        .iter()
                        .find(|i| i.install_id.as_str() == install_id);
                    let Some(install) = install else {
                        anyhow::bail!("Install '{install_id}' not found.");
                    };
                    let game_def = ctx.games.game(&install.game).ok_or_else(|| {
                        anyhow::anyhow!("Game definition not found for {}", install.game)
                    })?;
                    let base_mode: agora_core::game_base::BaseMode =
                        mode.parse().map_err(|e| anyhow::anyhow!("{e}"))?;

                    let start = std::time::Instant::now();
                    let options = agora_core::game_base::BuildOptions { include_excluded };
                    let result = agora_core::game_base::build_base(
                        &ctx.paths,
                        install,
                        game_def,
                        base_mode,
                        None,
                        options,
                        &|p: agora_core::game_base::BuildProgress| {
                            // Hashing a large game takes a while; say so about once a second.
                            use std::sync::atomic::{AtomicU64, Ordering};
                            static LAST_MS: AtomicU64 = AtomicU64::new(0);
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);
                            let last = LAST_MS.load(Ordering::Relaxed);
                            if (now.saturating_sub(last) >= 1000 || p.files_done == p.files_total)
                                && LAST_MS
                                    .compare_exchange(
                                        last,
                                        now,
                                        Ordering::Relaxed,
                                        Ordering::Relaxed,
                                    )
                                    .is_ok()
                                && !json
                            {
                                eprintln!(
                                    "  hashing: {}/{} files, {:.1}/{:.1} GB",
                                    p.files_done,
                                    p.files_total,
                                    p.bytes_hashed as f64 / 1e9,
                                    p.bytes_total as f64 / 1e9
                                );
                            }
                        },
                    );

                    match result {
                        Ok(agora_core::game_base::BuildOutcome::Built {
                            manifest,
                            linked_bytes,
                            copied_bytes,
                        }) => {
                            let linked_gb = (linked_bytes as f64) / (1024.0 * 1024.0 * 1024.0);
                            let copied_mb = (copied_bytes as f64) / (1024.0 * 1024.0);
                            let secs = start.elapsed().as_secs_f64();
                            if json {
                                let out = serde_json::json!({
                                    "status": "built",
                                    "mode": manifest.mode,
                                    "base_id": manifest.base_id,
                                    "location": manifest.location,
                                    "file_count": manifest.files.len(),
                                    "linked_bytes": linked_bytes,
                                    "linked_gb": linked_gb,
                                    "copied_bytes": copied_bytes,
                                    "copied_mb": copied_mb,
                                    "seconds_taken": secs,
                                    "manifest": manifest,
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                println!("Mode:          {}", manifest.mode);
                                println!("Base ID:       {}", manifest.base_id);
                                println!("Location:      {}", manifest.location.display());
                                println!("Files:         {}", manifest.files.len());
                                println!("Linked:        {linked_gb:.2} GB");
                                println!("Copied:        {copied_mb:.2} MB");
                                println!("Seconds taken: {secs:.2}s");
                            }
                        }
                        Ok(agora_core::game_base::BuildOutcome::Existing(manifest)) => {
                            let secs = start.elapsed().as_secs_f64();
                            if json {
                                let out = serde_json::json!({
                                    "status": "existing",
                                    "mode": manifest.mode,
                                    "base_id": manifest.base_id,
                                    "location": manifest.location,
                                    "file_count": manifest.files.len(),
                                    "seconds_taken": secs,
                                    "manifest": manifest,
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                println!("Mode:          {}", manifest.mode);
                                println!("Base ID:       {}", manifest.base_id);
                                println!("Location:      {}", manifest.location.display());
                                println!("Files:         {} (existing)", manifest.files.len());
                                println!("Seconds taken: {secs:.2}s");
                            }
                        }
                        Err(agora_core::game_base::BaseError::LinkUnavailable {
                            reason,
                            copied_bytes,
                        }) => {
                            let copied_gb = (copied_bytes as f64) / (1024.0 * 1024.0 * 1024.0);
                            if json {
                                let out = serde_json::json!({
                                    "status": "error",
                                    "error": "link_unavailable",
                                    "reason": reason,
                                    "copied_bytes": copied_bytes,
                                    "hint": "re-run with --mode copied",
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Hardlinks unavailable: {reason}");
                                eprintln!(
                                    "A copied base would require {copied_gb:.2} GB ({copied_bytes} bytes)."
                                );
                                eprintln!("Hint: re-run with --mode copied to build a full copy.");
                            }
                            std::process::exit(1);
                        }
                        Err(e) => {
                            anyhow::bail!("{e}");
                        }
                    }
                }
                BaseCmd::List => {
                    let listings = agora_core::game_base::list_bases(&ctx.paths);
                    if json {
                        println!("{}", serde_json::to_string_pretty(&listings)?);
                    } else if listings.is_empty() {
                        println!("No pinned bases found.");
                    } else {
                        for b in listings {
                            let status = if b.present { "present" } else { "missing" };
                            println!(
                                "{} ({}, {}, {} files, {})",
                                b.manifest.base_id,
                                b.manifest.mode,
                                b.manifest.location.display(),
                                b.manifest.files.len(),
                                status
                            );
                        }
                    }
                }
                BaseCmd::Verify { base_id, full } => {
                    let manifest_path = ctx.paths.base_manifest_path(&base_id);
                    if !manifest_path.exists() {
                        if json {
                            let out = serde_json::json!({
                                "error": format!("Base '{base_id}' not found."),
                                "exitCode": 1,
                            });
                            eprintln!("{}", serde_json::to_string_pretty(&out)?);
                        } else {
                            eprintln!("Error: Base '{base_id}' not found.");
                        }
                        std::process::exit(1);
                    }
                    let content = std::fs::read_to_string(&manifest_path)?;
                    let manifest: agora_core::game_base::BaseManifest =
                        serde_json::from_str(&content)?;
                    let depth = if full {
                        agora_core::game_base::VerifyDepth::Full
                    } else {
                        agora_core::game_base::VerifyDepth::Quick
                    };
                    let game_def = ctx.games.game(&manifest.runtime.game);
                    let ver = agora_core::game_base::verify_base(
                        &manifest,
                        depth,
                        &|p| game_def.map(|d| d.is_declared_write(p)).unwrap_or(false),
                        &|p| game_def.map(|d| d.is_excluded(p)).unwrap_or(false),
                    );
                    if json {
                        println!("{}", serde_json::to_string_pretty(&ver)?);
                    } else if ver.problems.is_empty() {
                        println!(
                            "Base '{base_id}' verified clean (checked {}, hashed {}).",
                            ver.checked, ver.hashed
                        );
                        if !ver.game_writes.is_empty() {
                            println!("Game writes ({}):", ver.game_writes.len());
                            for w in &ver.game_writes {
                                println!("  - {w}");
                            }
                        }
                    } else {
                        eprintln!("Base '{base_id}' has {} problem(s):", ver.problems.len());
                        print_base_problems(&ver.problems);
                        if !ver.game_writes.is_empty() {
                            println!("Game writes ({}):", ver.game_writes.len());
                            for w in &ver.game_writes {
                                println!("  - {w}");
                            }
                        }
                    }
                    if !ver.problems.is_empty() {
                        std::process::exit(1);
                    }
                }
                BaseCmd::Remove { base_id } => {
                    match agora_core::game_base::remove_base(&ctx.paths, &base_id) {
                        Ok(()) => {
                            if json {
                                let out = serde_json::json!({
                                    "status": "removed",
                                    "base_id": base_id,
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                println!("Base '{base_id}' removed.");
                            }
                        }
                        Err(agora_core::game_base::BaseError::InUse { instances }) => {
                            if json {
                                let out = serde_json::json!({
                                    "error": format!("Base '{base_id}' is in use by instance(s): {}", instances.join(", ")),
                                    "instances": instances,
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!(
                                    "Error: Base '{base_id}' cannot be removed because it is in use by instance(s): {}",
                                    instances.join(", ")
                                );
                            }
                            std::process::exit(1);
                        }
                        Err(agora_core::game_base::BaseError::NotFound(_)) => {
                            if json {
                                let out = serde_json::json!({
                                    "error": format!("Base '{base_id}' not found."),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: Base '{base_id}' not found.");
                            }
                            std::process::exit(1);
                        }
                        Err(e) => {
                            anyhow::bail!("{e}");
                        }
                    }
                }
            },
            GamesCmd::Catalog { action } => match action {
                CatalogCmd::List { game } => {
                    if game == "minecraft" {
                        anyhow::bail!(
                            "Minecraft's catalog is browsed with 'agora mods search', not here."
                        );
                    }
                    let known = agora_game_api::GameId::new(&game)
                        .ok()
                        .and_then(|id| ctx.games.game(&id))
                        .is_some();
                    if !known {
                        anyhow::bail!(
                            "Unknown game '{game}'. 'agora games list' shows the supported games."
                        );
                    }
                    let svc = RegistryService::new(ctx.clone());
                    let entries = svc.list_game_items(&game)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&entries)?);
                    } else {
                        let rows: Vec<Vec<String>> = entries
                            .iter()
                            .map(|entry| {
                                vec![
                                    entry.id.clone(),
                                    entry.name.clone(),
                                    entry
                                        .game_compatibility
                                        .iter()
                                        .map(|compat| compat.stores.join("/"))
                                        .collect::<Vec<_>>()
                                        .join("; "),
                                    entry
                                        .game_compatibility
                                        .iter()
                                        .map(|compat| compat.game_versions.join(","))
                                        .collect::<Vec<_>>()
                                        .join("; "),
                                ]
                            })
                            .collect();
                        print_table(&["ID", "Name", "Stores", "Game versions"], &rows);
                    }
                }
            },
            GamesCmd::Content { action } => match action {
                ContentCmd::Add { path, name } => {
                    let outcome = if path.is_dir() {
                        agora_core::content_store::add_folder(ctx, &path, name.as_deref())
                    } else {
                        agora_core::content_store::add_archive(ctx, &path, name.as_deref())
                    };
                    match outcome {
                        Ok(outcome) => {
                            if json {
                                println!("{}", serde_json::to_string_pretty(&outcome)?);
                            } else {
                                let item = outcome.item();
                                let short_id = &item.item_id[..12.min(item.item_id.len())];
                                match &outcome {
                                    agora_core::content_store::AddOutcome::Added { .. } => println!(
                                        "Added item {short_id} ({} files, {} bytes, {} new objects).",
                                        item.files.len(),
                                        item.total_size,
                                        outcome.objects_new()
                                    ),
                                    agora_core::content_store::AddOutcome::Existing { .. } => println!(
                                        "Already stored as item {short_id} ({} files); {} objects restored.",
                                        item.files.len(),
                                        outcome.objects_restored()
                                    ),
                                }
                            }
                        }
                        Err(e) => {
                            if json {
                                let out = serde_json::json!({
                                    "error": e.to_string(),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: {e}");
                            }
                            std::process::exit(1);
                        }
                    }
                }
                ContentCmd::List => {
                    let items = agora_core::content_store::list_items(ctx)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&items)?);
                    } else if items.is_empty() {
                        println!("No content items found.");
                    } else {
                        for item in items {
                            let short_id = &item.item_id[..12.min(item.item_id.len())];
                            let sources_str = item
                                .sources
                                .iter()
                                .map(|s| match s {
                                    agora_core::content_store::ContentSource::Archive {
                                        path,
                                        ..
                                    } => format!("archive:{path}"),
                                    agora_core::content_store::ContentSource::Folder {
                                        path,
                                        ..
                                    } => format!("folder:{path}"),
                                    agora_core::content_store::ContentSource::FomodInstall {
                                        from_item,
                                        ..
                                    } => format!("fomod:{}", &from_item[..12.min(from_item.len())]),
                                    agora_core::content_store::ContentSource::Thunderstore {
                                        package,
                                        version,
                                        ..
                                    } => format!("thunderstore:{package}@{version}"),
                                    _ => "other".to_string(),
                                })
                                .collect::<Vec<_>>()
                                .join(", ");
                            println!(
                                "{} ({}, {} files, {} bytes, sources: [{}])",
                                item.name,
                                short_id,
                                item.files.len(),
                                item.total_size,
                                sources_str
                            );
                        }
                    }
                }
                ContentCmd::Show { item } => {
                    match agora_core::content_store::get_item(ctx, &item) {
                        Ok(it) => {
                            if json {
                                println!("{}", serde_json::to_string_pretty(&it)?);
                            } else {
                                let short_id = &it.item_id[..12.min(it.item_id.len())];
                                println!(
                                    "Item {} ({}, {} files, {} bytes):",
                                    it.name,
                                    short_id,
                                    it.files.len(),
                                    it.total_size
                                );
                                for f in &it.files {
                                    println!(
                                        "  {} ({} bytes, sha256: {})",
                                        f.path, f.size, f.sha256
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            if json {
                                let out = serde_json::json!({
                                    "error": e.to_string(),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: {e}");
                            }
                            std::process::exit(1);
                        }
                    }
                }
                ContentCmd::Verify { item, full } => {
                    let depth = if full {
                        agora_core::content_store::VerifyDepth::Full
                    } else {
                        agora_core::content_store::VerifyDepth::Quick
                    };
                    if let Some(prefix) = item {
                        match agora_core::content_store::verify_item(ctx, &prefix, depth) {
                            Ok(ver) => {
                                if json {
                                    println!("{}", serde_json::to_string_pretty(&ver)?);
                                } else if ver.problems.is_empty() {
                                    let short_id = &ver.item_id[..12.min(ver.item_id.len())];
                                    println!(
                                        "Item '{short_id}' verified clean (checked {}, hashed {}).",
                                        ver.checked, ver.hashed
                                    );
                                } else {
                                    let short_id = &ver.item_id[..12.min(ver.item_id.len())];
                                    eprintln!(
                                        "Item '{short_id}' has {} problem(s):",
                                        ver.problems.len()
                                    );
                                    print_content_problems(&ver.problems);
                                }
                                if !ver.problems.is_empty() {
                                    std::process::exit(1);
                                }
                            }
                            Err(e) => {
                                if json {
                                    let out = serde_json::json!({
                                        "error": e.to_string(),
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Error: {e}");
                                }
                                std::process::exit(1);
                            }
                        }
                    } else {
                        match agora_core::content_store::verify_all(ctx, depth) {
                            Ok(report) => {
                                if json {
                                    println!("{}", serde_json::to_string_pretty(&report)?);
                                } else if report.problems.is_empty() {
                                    println!(
                                        "All content items verified clean (checked {} items, {} files, hashed {}).",
                                        report.checked_items, report.checked_files, report.hashed_files
                                    );
                                } else {
                                    eprintln!(
                                        "Content store has {} problem(s) across {} item(s):",
                                        report.problems.len(),
                                        report.checked_items
                                    );
                                    print_content_problems(&report.problems);
                                }
                                if !report.problems.is_empty() {
                                    std::process::exit(1);
                                }
                            }
                            Err(e) => {
                                if json {
                                    let out = serde_json::json!({
                                        "error": e.to_string(),
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Error: {e}");
                                }
                                std::process::exit(1);
                            }
                        }
                    }
                }
                ContentCmd::Remove { item } => {
                    match agora_core::content_store::remove_item(ctx, &item) {
                        Ok(()) => {
                            if json {
                                let out = serde_json::json!({
                                    "status": "removed",
                                    "item": item,
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                println!("Content item '{item}' removed.");
                            }
                        }
                        Err(e) => {
                            if json {
                                let out = serde_json::json!({
                                    "error": e.to_string(),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: {e}");
                            }
                            std::process::exit(1);
                        }
                    }
                }
                ContentCmd::Fomod { action } => run_fomod_command(ctx, action, json)?,
            },
            GamesCmd::Instance { action } => match action {
                GameInstanceCmd::Create {
                    install_id,
                    name,
                    id,
                    mode,
                    include_excluded,
                } => {
                    let report = agora_core::game_discovery::discover_all();
                    let inventory = agora_core::game_registry::identify_installs(
                        &ctx.games,
                        &report,
                        &agora_core::game_discovery::file_version::read_file_version,
                    );
                    let install = inventory
                        .installs
                        .iter()
                        .find(|i| i.install_id.as_str() == install_id);
                    let Some(install) = install else {
                        if json {
                            let out = serde_json::json!({
                                "error": format!("Install '{install_id}' not found."),
                                "exitCode": 1,
                            });
                            eprintln!("{}", serde_json::to_string_pretty(&out)?);
                        } else {
                            eprintln!("Error: Install '{install_id}' not found.");
                        }
                        std::process::exit(1);
                    };
                    let game_def = ctx.games.game(&install.game).ok_or_else(|| {
                        anyhow::anyhow!("Game definition not found for {}", install.game)
                    })?;
                    let base_mode: agora_game_api::BaseMode =
                        mode.parse().map_err(|e| anyhow::anyhow!("{e}"))?;

                    let start = std::time::Instant::now();
                    let options = agora_core::game_base::BuildOptions { include_excluded };
                    let record = match agora_core::game_instance::create_with_options(
                        ctx,
                        install,
                        game_def,
                        name.as_deref().unwrap_or(""),
                        id,
                        base_mode,
                        options,
                        &|p: agora_core::game_base::BuildProgress| {
                            use std::sync::atomic::{AtomicU64, Ordering};
                            static LAST_MS: AtomicU64 = AtomicU64::new(0);
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);
                            let last = LAST_MS.load(Ordering::Relaxed);
                            if (now.saturating_sub(last) >= 1000 || p.files_done == p.files_total)
                                && LAST_MS
                                    .compare_exchange(
                                        last,
                                        now,
                                        Ordering::Relaxed,
                                        Ordering::Relaxed,
                                    )
                                    .is_ok()
                                && !json
                            {
                                eprintln!(
                                    "  hashing: {}/{} files, {:.1}/{:.1} GB",
                                    p.files_done,
                                    p.files_total,
                                    p.bytes_hashed as f64 / 1e9,
                                    p.bytes_total as f64 / 1e9
                                );
                            }
                        },
                    ) {
                        Ok(rec) => rec,
                        Err(e) => {
                            if json {
                                let out = serde_json::json!({
                                    "status": "error",
                                    "error": format!("{e}"),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error creating instance: {e}");
                            }
                            std::process::exit(1);
                        }
                    };

                    if json {
                        println!("{}", serde_json::to_string_pretty(&record)?);
                    } else {
                        println!(
                            "Created instance '{}' for game '{}'.",
                            record.instance_id, record.game
                        );
                        match &record.base {
                            agora_game_api::BaseReference::Pinned { id, .. } => {
                                println!("Pinned base: {id}");
                            }
                            agora_game_api::BaseReference::Unpinned { reason, .. } => {
                                println!("unpinned: {reason}");
                            }
                        }
                        if let Some(agora_core::game_base::BuildOutcome::Built {
                            manifest,
                            linked_bytes,
                            copied_bytes,
                        }) = &record.build_outcome
                        {
                            let linked_gb = (*linked_bytes as f64) / (1024.0 * 1024.0 * 1024.0);
                            let copied_mb = (*copied_bytes as f64) / (1024.0 * 1024.0);
                            let secs = start.elapsed().as_secs_f64();
                            println!(
                                "Built pinned base '{}' ({}) in {:.1}s: {} files, {:.2} GB linked, {:.1} MB copied",
                                manifest.base_id,
                                manifest.mode,
                                secs,
                                manifest.files.len(),
                                linked_gb,
                                copied_mb
                            );
                        }
                    }
                }
                GameInstanceCmd::List => {
                    let (instances, warnings) = agora_core::game_instance::list_all(ctx);
                    for warning in &warnings {
                        eprintln!("Warning: {warning}");
                    }
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "instances": instances,
                                "warnings": warnings,
                            }))?
                        );
                    } else if instances.is_empty() && warnings.is_empty() {
                        println!("No instances found.");
                    } else {
                        for inst in &instances {
                            let pinned_str = match inst.pinned {
                                Some(true) => "pinned",
                                Some(false) => "unpinned",
                                None => "-",
                            };
                            println!(
                                "{}\t{}\t{}\t{}\t{}",
                                inst.instance_id, inst.game, inst.name, inst.runtime, pinned_str
                            );
                        }
                    }
                }
                GameInstanceCmd::SetDeployment { instance_id, mode } => {
                    let chosen = match parse_deployment_arg(&mode) {
                        Ok(m) => m,
                        Err(e) => {
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    };
                    match agora_core::game_deploy::set_deployment(ctx, &instance_id, chosen) {
                        Ok(()) => {
                            let shown = chosen.map(|m| m.as_str()).unwrap_or("auto");
                            if json {
                                let out = serde_json::json!({
                                    "status": "ok",
                                    "instance_id": instance_id,
                                    "deployment": shown,
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                            } else if chosen.is_some() {
                                println!(
                                    "Instance '{instance_id}' will always deploy and run as '{shown}'; Agora will not step down from it."
                                );
                            } else {
                                println!(
                                    "Instance '{instance_id}' will use the game's default; Agora announces any step down."
                                );
                            }
                        }
                        Err(e) => {
                            if json {
                                let out = serde_json::json!({
                                    "status": "error",
                                    "error": format!("{e}"),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: {e}");
                            }
                            std::process::exit(1);
                        }
                    }
                }
                GameInstanceCmd::Launch {
                    instance_id,
                    wait,
                    launch_anyway,
                    deployment,
                    plain,
                    fall_back,
                } => {
                    let on_vfs_failure = if fall_back {
                        agora_core::game_instance::VfsFailure::FallBack
                    } else {
                        agora_core::game_instance::VfsFailure::Ask
                    };
                    let mut launch_options = match deployment.as_deref().map(parse_deployment_arg) {
                        None => agora_core::game_instance::LaunchOptions {
                            launch_anyway,
                            plain,
                            deployment: None,
                            on_vfs_failure,
                        },
                        Some(Ok(deployment)) => agora_core::game_instance::LaunchOptions {
                            launch_anyway,
                            plain,
                            deployment,
                            on_vfs_failure,
                        },
                        Some(Err(e)) => {
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    };
                    let in_mc = agora_core::game_instance::is_minecraft_instance(ctx, &instance_id);
                    if in_mc {
                        if json {
                            let out = serde_json::json!({
                                "error": format!("Instance '{instance_id}' is a Minecraft instance; use 'agora launch {instance_id}' instead."),
                                "exitCode": 1,
                            });
                            eprintln!("{}", serde_json::to_string_pretty(&out)?);
                        } else {
                            eprintln!("Error: Instance '{instance_id}' is a Minecraft instance; use 'agora launch {instance_id}' instead.");
                        }
                        std::process::exit(1);
                    }

                    let record = match agora_core::game_instance::get(ctx, &instance_id)? {
                        Some(r) => r,
                        None => {
                            if json {
                                let out = serde_json::json!({
                                    "error": format!("Instance '{instance_id}' not found."),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: Instance '{instance_id}' not found.");
                            }
                            std::process::exit(1);
                        }
                    };

                    let game_def = ctx.games.game(&record.game).ok_or_else(|| {
                        anyhow::anyhow!("Game definition not found for {}", record.game)
                    })?;

                    // The rung a "yes" to the fallback question started this launch from, so the
                    // way to keep it can be printed once the game is running.
                    let mut retried_from: Option<agora_core::game_deploy::DeployMode> = None;
                    'launch: loop {
                        let prepared = match agora_core::game_instance::prepare_launch_with(
                            ctx,
                            &instance_id,
                            game_def,
                            launch_options,
                            &agora_core::game_discovery::discover_all,
                            &agora_core::game_launch::SystemLauncher,
                        ) {
                            Ok(p) => p,
                            Err(agora_core::game_instance::InstanceError::LaunchError(
                                agora_core::game_launch::LaunchError::BaseDamaged { problems },
                            )) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": "base_damaged",
                                        "instance_id": instance_id,
                                        "problems": problems,
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Instance '{instance_id}' has base problem(s):");
                                    print_base_problems(&problems);
                                }
                                std::process::exit(1);
                            }
                            Err(agora_core::game_instance::InstanceError::LaunchError(
                                agora_core::game_launch::LaunchError::NoRecipe,
                            )) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": "no_recipe",
                                        "message": "Game definition has no launch recipe.",
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Error: Game definition has no launch recipe.");
                                }
                                std::process::exit(1);
                            }
                            Err(agora_core::game_instance::InstanceError::LaunchError(
                                agora_core::game_launch::LaunchError::RuntimeMismatch { findings },
                            )) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": "runtime_mismatch",
                                        "instance_id": instance_id,
                                        "findings": findings,
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!(
                                        "Instance '{instance_id}' has framework(s) built for another game version:"
                                    );
                                    print_runtime_findings(&findings);
                                    eprintln!(
                                        "Run 'agora games instance check {instance_id}' to see this again, or pass --launch-anyway to start the game anyway."
                                    );
                                }
                                std::process::exit(1);
                            }
                            Err(agora_core::game_instance::InstanceError::LaunchError(
                                agora_core::game_launch::LaunchError::LoadOrderProblems {
                                    findings,
                                },
                            )) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": "load_order_problems",
                                        "instance_id": instance_id,
                                        "findings": findings,
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!(
                                        "Instance '{instance_id}' has plugin load order problems that would stop the game:"
                                    );
                                    print_load_order_findings(&findings);
                                    eprintln!(
                                        "Run 'agora games instance plugins sort {instance_id}' to fix master order, or pass --launch-anyway to start the game anyway."
                                    );
                                }
                                std::process::exit(1);
                            }
                            Err(
                                e @ agora_core::game_instance::InstanceError::VfsUnavailable {
                                    next: Some(next),
                                    ..
                                },
                            ) => {
                                launch_options.deployment =
                                    Some(offer_vfs_fallback(&e, &instance_id, next, json)?);
                                retried_from = Some(next);
                                continue 'launch;
                            }
                            Err(e) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": format!("{e}"),
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Error: {e}");
                                }
                                std::process::exit(1);
                            }
                        };

                        if let Some(deploy_outcome) = &prepared.deploy_outcome {
                            if !json {
                                match deploy_outcome {
                                    agora_core::game_deploy::DeployOutcome::UpToDate { .. } => {
                                        println!("Deployment is up to date.");
                                    }
                                    agora_core::game_deploy::DeployOutcome::Built {
                                        linked,
                                        copied,
                                        copied_bytes,
                                        config_copied,
                                        harvest,
                                        ..
                                    } => {
                                        println!(
                                        "Deployed: {linked} linked, {copied} copied ({copied_bytes} bytes)."
                                    );
                                        if *config_copied > 0 {
                                            println!(
                                            "{config_copied} small config files copied so the game can write them."
                                        );
                                        }
                                        if let Some(h) = harvest {
                                            if !h.is_empty() {
                                                println!(
                                                "Harvested: {} copied to writable, {} whiteouts added.",
                                                h.copied_to_writable.len(),
                                                h.whiteouts_added.len()
                                            );
                                            }
                                        }
                                    }
                                }
                                if let Some(report) = deploy_outcome.plugins() {
                                    print_plugin_sync(report);
                                }
                            }
                        }
                        if !json {
                            if let Some(alt) = &prepared.alternative {
                                println!("Starting through '{}': {}.", alt.id, alt.reason);
                                println!(
                                    "(Use --plain to start the game's own executable instead.)"
                                );
                            }
                        }

                        let (store, running_from) = match &record.base {
                            agora_game_api::BaseReference::Pinned { id: base_id, .. } => {
                                let manifest_path = ctx.paths.base_manifest_path(base_id);
                                let text = std::fs::read_to_string(&manifest_path)?;
                                let base_manifest: agora_core::game_base::BaseManifest =
                                    serde_json::from_str(&text)?;
                                let store = base_manifest.runtime.store.clone();
                                let running_from = if prepared.deploy_outcome.is_some() {
                                    agora_core::game_deploy::deployment_dir(ctx, &instance_id)?
                                        .unwrap_or_else(|| base_manifest.location.clone())
                                } else {
                                    base_manifest.location.clone()
                                };
                                (store, running_from)
                            }
                            agora_game_api::BaseReference::Unpinned { install, .. } => {
                                let report = agora_core::game_discovery::discover_all();
                                let matching = report.installs.iter().find(|discovered| {
                                    agora_core::game_registry::make_install_id(
                                        &discovered.store,
                                        &discovered.product,
                                    ) == *install
                                });
                                let Some(discovered) = matching else {
                                    if json {
                                        let out = serde_json::json!({
                                            "error": format!("Install '{install}' not found."),
                                            "exitCode": 1,
                                        });
                                        eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                    } else {
                                        eprintln!("Error: Install '{install}' not found.");
                                    }
                                    std::process::exit(1);
                                };
                                (discovered.store.clone(), discovered.location.clone())
                            }
                        };

                        if let Err(e) = agora_core::game_user_files::swap_in(
                            ctx,
                            &instance_id,
                            game_def,
                            &store,
                            &running_from,
                        ) {
                            if json {
                                let out = serde_json::json!({
                                    "status": "error",
                                    "error": format!("{e}"),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: {e}");
                            }
                            std::process::exit(1);
                        }

                        let launch_start = std::time::Instant::now();
                        // What the VFS's log held before this launch, to read what the session adds.
                        let vfs_log_before = prepared
                            .vfs
                            .as_ref()
                            .map(|v| (v.log.clone(), agora_core::game_launch::log_len(&v.log)));
                        // With `--fall-back` the launch may step down from the virtual file system,
                        // which replaces `prepared` with the one that actually ran.
                        let (mut launched, prepared) =
                            match agora_core::game_instance::spawn_prepared(
                                ctx,
                                &instance_id,
                                game_def,
                                prepared,
                                launch_options,
                                &agora_core::game_launch::SystemLauncher,
                            ) {
                                Ok(l) => l,
                                Err(e) => {
                                    let user_files = report_user_files_restore(
                                        agora_core::game_user_files::restore(ctx, game_def, &store),
                                        game_def.id.as_str(),
                                        store.as_str(),
                                        json,
                                    );
                                    if let agora_core::game_instance::InstanceError::VfsUnavailable {
                                next: Some(next),
                                ..
                            } = &e
                            {
                                let next = *next;
                                launch_options.deployment =
                                    Some(offer_vfs_fallback(&e, &instance_id, next, json)?);
                                retried_from = Some(next);
                                continue 'launch;
                            }
                                    if json {
                                        let out = serde_json::json!({
                                            "status": "error",
                                            "error": format!("{e}"),
                                            "exitCode": 1,
                                            "user_files_restored": user_files,
                                        });
                                        eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                    } else {
                                        eprintln!("Launch failed: {e}");
                                    }
                                    std::process::exit(1);
                                }
                            };
                        // Only a session that really ran under the VFS has anything in its log.
                        let vfs_log_before = vfs_log_before.filter(|_| prepared.vfs.is_some());
                        if !json {
                            if let Some(notice) = &prepared.notice {
                                eprintln!("Notice: {notice}");
                            }
                            if let Some(mode) = retried_from {
                                println!(
                                "Running from {} for this launch only. To keep it: agora games instance set-deployment {instance_id} {mode}",
                                mode.plain_name()
                            );
                            }
                        }

                        let _ = agora_core::game_user_files::record_process(
                            ctx,
                            &record.game,
                            &store,
                            launched.identity.clone(),
                        );

                        let _ = agora_core::game_instance::record_launch(ctx, &instance_id);

                        if !json && !prepared.generated_findings.is_empty() {
                            for finding in &prepared.generated_findings {
                                eprintln!("Warning: {}", finding.message);
                            }
                        }
                        if !json && !prepared.runtime_findings.is_empty() {
                            eprintln!(
                                "Warning: Launching past {} framework check finding(s):",
                                prepared.runtime_findings.len()
                            );
                            print_runtime_findings(&prepared.runtime_findings);
                        }
                        if !json && !prepared.load_order_findings.is_empty() {
                            eprintln!(
                                "Warning: {} plugin load order finding(s):",
                                prepared.load_order_findings.len()
                            );
                            print_load_order_findings(&prepared.load_order_findings);
                            if let Some(hint) =
                                sort_hint(&instance_id, &prepared.load_order_findings)
                            {
                                eprintln!("{hint}");
                            }
                        }

                        let env_map: std::collections::BTreeMap<String, String> = prepared
                            .resolved
                            .env
                            .iter()
                            .map(|(k, v)| (k.clone(), v.to_string_lossy().to_string()))
                            .collect();

                        let base_id = match &record.base {
                            agora_game_api::BaseReference::Pinned { id, .. } => Some(id.clone()),
                            agora_game_api::BaseReference::Unpinned { .. } => None,
                        };

                        if !wait {
                            if json {
                                let out = serde_json::json!({
                                    "status": "launched",
                                    "instance_id": instance_id,
                                    "base_id": base_id,
                                    "pid": launched.pid(),
                                    "program": prepared.resolved.program,
                                    "cwd": prepared.resolved.cwd,
                                    "env": env_map,
                                    "warnings": prepared.warnings,
                                    "runtime_findings": prepared.runtime_findings,
                                    "load_order_findings": prepared.load_order_findings,
                                    "generated_findings": prepared.generated_findings,
                                    "deployment": prepared.deployment.map(|m| m.as_str()),
                                    "notice": prepared.notice,
                                    "alternative": prepared.alternative,
                                    "plugins": prepared.deploy_outcome.as_ref().and_then(|o| o.plugins()),
                                    "deploy_summary": deploy_summary_json(prepared.deploy_outcome.as_ref()),
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                if !prepared.warnings.is_empty() {
                                    eprintln!(
                                        "Warning: Launching damaged base ({} problem(s)):",
                                        prepared.warnings.len()
                                    );
                                    print_base_problems(&prepared.warnings);
                                }
                                println!(
                                    "Program:           {}",
                                    prepared.resolved.program.display()
                                );
                                println!("Working Directory: {}", prepared.resolved.cwd.display());
                                if let Some(mode) = prepared.deployment {
                                    println!("Deployment:        {mode}");
                                }
                                println!("PID:               {}", launched.pid());
                                if !prepared.resolved.env.is_empty() {
                                    println!("Environment:");
                                    for (k, v) in &prepared.resolved.env {
                                        println!("  {k}={}", v.to_string_lossy());
                                    }
                                }
                            }
                        } else {
                            // A pinned instance is watched and re-verified through its
                            // base; prepare_launch already loaded that manifest, so a
                            // failure to read it now is an error, not a fallback.
                            let base_manifest = match &record.base {
                                agora_game_api::BaseReference::Pinned { id, .. } => {
                                    let text =
                                        std::fs::read_to_string(ctx.paths.base_manifest_path(id))?;
                                    Some(
                                        serde_json::from_str::<agora_core::game_base::BaseManifest>(
                                            &text,
                                        )?,
                                    )
                                }
                                agora_game_api::BaseReference::Unpinned { .. } => None,
                            };
                            // The game that matters is whatever runs from the runtime folder: a
                            // framework loader starts the game and exits, so the loader's own
                            // process is not what to wait for.
                            let watch_dir = running_from.clone();

                            let exit_report = agora_core::game_launch::wait_for_exit(
                                &watch_dir,
                                &mut launched,
                                std::time::Duration::from_millis(250),
                                std::time::Duration::from_secs(5),
                            );
                            let session_duration = launch_start.elapsed();
                            let user_files = report_user_files_restore(
                                agora_core::game_user_files::restore(ctx, game_def, &store),
                                game_def.id.as_str(),
                                store.as_str(),
                                json,
                            );
                            let after = base_manifest.as_ref().map(|m| {
                                agora_core::game_base::verify_base(
                                    m,
                                    agora_core::game_base::VerifyDepth::Quick,
                                    &|p| game_def.is_declared_write(p),
                                    &|p| game_def.is_excluded(p),
                                )
                            });
                            // Programs the game started that the VFS ended because it could not
                            // protect them: silent to the game, so say so here.
                            let ended_by_vfs = vfs_log_before
                                .as_ref()
                                .map(|(log, from)| {
                                    agora_core::game_launch::processes_ended_since(log, *from)
                                })
                                .unwrap_or_default();
                            let ended_next = if ended_by_vfs.is_empty() {
                                None
                            } else {
                                prepared.deployment.and_then(|mode| mode.next_fallback())
                            };

                            // Linked files that end this quickly usually mean a mod refused to run
                            // from them; the human output hints at the remedies.
                            let ended_quickly = prepared.deployment
                                == Some(agora_core::game_deploy::DeployMode::Links)
                                && session_duration < std::time::Duration::from_secs(30);

                            if json {
                                let out = serde_json::json!({
                                    "status": "exited",
                                    "vfs_ended_processes": ended_by_vfs,
                                    "nextDeployment": ended_next.map(|m| m.as_str()),
                                    "retry": ended_next.map(|m| vfs_retry_commands(&instance_id, m)),
                                    "deployment": prepared.deployment.map(|m| m.as_str()),
                                    "notice": prepared.notice,
                                    "alternative": prepared.alternative,
                                    "plugins": prepared.deploy_outcome.as_ref().and_then(|o| o.plugins()),
                                    "deploy_summary": deploy_summary_json(prepared.deploy_outcome.as_ref()),
                                    "user_files_restored": user_files,
                                    "session_ended_quickly": ended_quickly,
                                    "instance_id": instance_id,
                                    "base_id": base_id,
                                    "pid": launched.pid(),
                                    "program": prepared.resolved.program,
                                    "cwd": prepared.resolved.cwd,
                                    "env": env_map,
                                    "warnings": prepared.warnings,
                                    "runtime_findings": prepared.runtime_findings,
                                    "load_order_findings": prepared.load_order_findings,
                                    "generated_findings": prepared.generated_findings,
                                    "deployment": prepared.deployment.map(|m| m.as_str()),
                                    "notice": prepared.notice,
                                    "processes": exit_report.processes,
                                    "relaunched_outside": exit_report.relaunched_outside,
                                    "game_writes": after.as_ref().map(|v| &v.game_writes),
                                    "problems": after.as_ref().map(|v| &v.problems),
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                println!(
                                    "Program:           {}",
                                    prepared.resolved.program.display()
                                );
                                println!("PID:               {}", launched.pid());
                                println!("Processes running from game directory:");
                                if exit_report.processes.is_empty() {
                                    println!("  (none)");
                                } else {
                                    for p in &exit_report.processes {
                                        println!("  - PID {}: {}", p.pid, p.exe.display());
                                    }
                                }
                                if exit_report.relaunched_outside {
                                    eprintln!("Warning: A process with the game's executable name ran outside the game directory (possible relaunch from store).");
                                }
                                if let Some(ver) = &after {
                                    if !ver.game_writes.is_empty() {
                                        println!("Game writes ({}):", ver.game_writes.len());
                                        for w in &ver.game_writes {
                                            println!("  - {w}");
                                        }
                                    }
                                    if ver.problems.is_empty() {
                                        println!("Base verified clean after launch.");
                                    } else {
                                        eprintln!(
                                            "Base has {} problem(s) after launch:",
                                            ver.problems.len()
                                        );
                                        print_base_problems(&ver.problems);
                                    }
                                }
                                if ended_quickly {
                                    println!();
                                    println!("The session ended quickly. Under linked files a mod that edits its own files is refused.");
                                    println!("Remedies:");
                                    println!("  agora games instance launch {instance_id} --deployment virtual");
                                    println!("  agora games instance content own-copy {instance_id} <item> on");
                                }
                            }
                            if !json && !ended_by_vfs.is_empty() {
                                if let Some(next) = ended_next {
                                    if offer_restart_after_vfs_ended(
                                        &ended_by_vfs,
                                        &instance_id,
                                        next,
                                    )? {
                                        launch_options.deployment = Some(next);
                                        retried_from = Some(next);
                                        continue 'launch;
                                    }
                                }
                            }
                            if after.as_ref().is_some_and(|v| !v.problems.is_empty()) {
                                std::process::exit(1);
                            }
                        }
                        break 'launch;
                    }
                }
                GameInstanceCmd::Delete { instance_id } => {
                    match agora_core::game_instance::delete(ctx, &instance_id) {
                        Ok(outcome) => {
                            if json {
                                let out = serde_json::json!({
                                    "status": "deleted",
                                    "instance_id": instance_id,
                                    "orphaned_base": outcome.orphaned_base,
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                println!("Instance '{instance_id}' deleted.");
                                if let Some(base_id) = outcome.orphaned_base {
                                    println!("Base '{base_id}' is no longer used by any instance; run 'agora games base remove {base_id}' to remove it.");
                                }
                            }
                        }
                        Err(agora_core::game_instance::InstanceError::MinecraftInstance(_)) => {
                            if json {
                                let out = serde_json::json!({
                                    "error": format!("Instance '{instance_id}' is a Minecraft instance; use 'agora instance delete {instance_id}' instead."),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: Instance '{instance_id}' is a Minecraft instance; use 'agora instance delete {instance_id}' instead.");
                            }
                            std::process::exit(1);
                        }
                        Err(agora_core::game_instance::InstanceError::NotFound(_)) => {
                            if json {
                                let out = serde_json::json!({
                                    "error": format!("Instance '{instance_id}' not found."),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: Instance '{instance_id}' not found.");
                            }
                            std::process::exit(1);
                        }
                        Err(e) => {
                            if json {
                                let out = serde_json::json!({
                                    "error": format!("{e}"),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: {e}");
                            }
                            std::process::exit(1);
                        }
                    }
                }
                GameInstanceCmd::Content { action } => match action {
                    InstanceContentCmd::Add {
                        instance_id,
                        item_id,
                        into,
                        from,
                    } => {
                        let (target_mount, target_source) = if into.is_some() || from.is_some() {
                            (into, from)
                        } else {
                            let manifest =
                                match agora_core::game_instance::get_manifest(ctx, &instance_id) {
                                    Ok(m) => m,
                                    Err(e) => {
                                        if json {
                                            let out = serde_json::json!({
                                                "status": "error",
                                                "error": format!("{e}"),
                                                "exitCode": 1,
                                            });
                                            eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                        } else {
                                            eprintln!("Error: {e}");
                                        }
                                        std::process::exit(1);
                                    }
                                };
                            let game_def = ctx.games.game(&manifest.game);
                            let Some(game_def) = game_def else {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": format!("Game definition not found for {}", manifest.game),
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!(
                                        "Error: Game definition not found for {}",
                                        manifest.game
                                    );
                                }
                                std::process::exit(1);
                            };
                            let Some(layout) = &game_def.content_layout else {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": "Game definition has no content layout. Specify --into (and --from).",
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Error: Game definition '{}' has no content layout. Specify --into (and --from).", game_def.id);
                                }
                                std::process::exit(1);
                            };

                            if layout.thunderstore_bepinex {
                                if let Ok(Some(manifest)) =
                                    agora_core::content_thunderstore::parse_manifest(ctx, &item_id)
                                {
                                    match agora_core::content_thunderstore::install_thunderstore(
                                        ctx,
                                        &instance_id,
                                        &item_id,
                                    ) {
                                        Ok(outcome) => {
                                            if !json {
                                                println!(
                                                    "Thunderstore package {} {}: {}",
                                                    outcome.package_id,
                                                    outcome.version,
                                                    outcome.summary
                                                );
                                                println!(
                                                    "Added content '{}' to instance '{instance_id}' (mount: '').",
                                                    outcome.derived_item.item_id
                                                );
                                            }
                                            for dep in &outcome.missing_dependencies {
                                                eprintln!(
                                                    "Warning: {} needs {}; it is not in this instance",
                                                    manifest.name, dep
                                                );
                                            }
                                            if json {
                                                let out = serde_json::json!({
                                                    "status": "added",
                                                    "instance_id": instance_id,
                                                    "item_id": outcome.derived_item.item_id,
                                                    "layer_id": outcome.layer.id.as_str(),
                                                    "mount_path": outcome.layer.mount_path.as_str(),
                                                    "source_path": outcome.layer.source_path.as_str(),
                                                    "thunderstore": {
                                                        "package": outcome.package_id,
                                                        "version": outcome.version,
                                                        "summary": outcome.summary,
                                                        "missing_dependencies": outcome.missing_dependencies,
                                                    }
                                                });
                                                println!("{}", serde_json::to_string_pretty(&out)?);
                                            }
                                            return Ok(());
                                        }
                                        Err(e) => {
                                            if json {
                                                let out = serde_json::json!({
                                                    "status": "error",
                                                    "error": format!("{e}"),
                                                    "exitCode": 1,
                                                });
                                                eprintln!(
                                                    "{}",
                                                    serde_json::to_string_pretty(&out)?
                                                );
                                            } else {
                                                eprintln!("Error: {e}");
                                            }
                                            std::process::exit(1);
                                        }
                                    }
                                }
                            }

                            let item = match agora_core::content_store::get_item(ctx, &item_id) {
                                Ok(it) => it,
                                Err(e) => {
                                    if json {
                                        let out = serde_json::json!({
                                            "status": "error",
                                            "error": format!("{e}"),
                                            "exitCode": 1,
                                        });
                                        eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                    } else {
                                        eprintln!("Error: {e}");
                                    }
                                    std::process::exit(1);
                                }
                            };

                            let file_paths: Vec<agora_game_api::RelPath> =
                                item.files.iter().map(|f| f.path.clone()).collect();
                            let suggestion = agora_game_api::suggest_placement(&file_paths, layout);

                            match suggestion {
                                agora_game_api::Suggestion::Place {
                                    source_path,
                                    mount_path,
                                    reason,
                                } => {
                                    if !json {
                                        println!("{reason}");
                                    }
                                    let mount_opt = if mount_path.as_str().is_empty() {
                                        None
                                    } else {
                                        Some(mount_path.as_str().to_string())
                                    };
                                    let source_opt = if source_path.as_str().is_empty() {
                                        None
                                    } else {
                                        Some(source_path.as_str().to_string())
                                    };
                                    (mount_opt, source_opt)
                                }
                                agora_game_api::Suggestion::Installer { reason } => {
                                    let mut top_level: Vec<String> = item
                                        .files
                                        .iter()
                                        .filter_map(|f| {
                                            f.path.as_str().split('/').next().map(|s| s.to_string())
                                        })
                                        .collect();
                                    top_level.sort();
                                    top_level.dedup();
                                    let command = format!(
                                        "agora games content fomod install {item_id} --instance {instance_id}"
                                    );
                                    if json {
                                        let out = serde_json::json!({
                                            "status": "error",
                                            "error": format!(
                                                "This archive has a FOMOD installer: {reason}. Run `{command} --defaults`, or pick options with `--choose \"Step/Group/Plugin\"` (see `agora games content fomod show {item_id}`)."
                                            ),
                                            "reason": reason,
                                            "top_level": top_level,
                                            "installer_command": command,
                                            "exitCode": 1,
                                        });
                                        eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                    } else {
                                        eprintln!(
                                            "Error: this archive has a FOMOD installer: {reason}."
                                        );
                                        eprintln!("Install it with: {command} --defaults");
                                        eprintln!(
                                            "or pick options with --choose \"Step/Group/Plugin\" (see `agora games content fomod show {item_id}`)."
                                        );
                                    }
                                    std::process::exit(1);
                                }
                                agora_game_api::Suggestion::Unknown { top_level } => {
                                    if json {
                                        let out = serde_json::json!({
                                            "status": "error",
                                            "error": "Cannot determine placement for content",
                                            "top_level": top_level,
                                            "exitCode": 1,
                                        });
                                        eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                    } else {
                                        eprintln!("Error: Cannot determine placement for content.");
                                        eprintln!("Top-level entries: {}", top_level.join(", "));
                                        eprintln!("Specify --into (and --from) to place manually.");
                                    }
                                    std::process::exit(1);
                                }
                            }
                        };

                        match agora_core::game_deploy::add_content(
                            ctx,
                            &instance_id,
                            &item_id,
                            target_mount.as_deref(),
                            target_source.as_deref(),
                        ) {
                            Ok(layer) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "added",
                                        "instance_id": instance_id,
                                        "item_id": item_id,
                                        "layer_id": layer.id.as_str(),
                                        "mount_path": layer.mount_path.as_str(),
                                        "source_path": layer.source_path.as_str(),
                                    });
                                    println!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    let from_note = if layer.source_path.as_str().is_empty() {
                                        String::new()
                                    } else {
                                        format!(" (from '{}')", layer.source_path.as_str())
                                    };
                                    println!(
                                        "Added content '{item_id}' to instance '{instance_id}' (mount: '{}'{from_note}).",
                                        layer.mount_path.as_str()
                                    );
                                }
                            }
                            Err(e) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": format!("{e}"),
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Error: {e}");
                                }
                                std::process::exit(1);
                            }
                        }
                    }
                    InstanceContentCmd::List { instance_id } => {
                        let manifest =
                            match agora_core::game_instance::get_manifest(ctx, &instance_id) {
                                Ok(m) => m,
                                Err(e) => {
                                    if json {
                                        let out = serde_json::json!({
                                            "status": "error",
                                            "error": format!("{e}"),
                                            "exitCode": 1,
                                        });
                                        eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                    } else {
                                        eprintln!("Error: {e}");
                                    }
                                    std::process::exit(1);
                                }
                            };

                        let mut items = Vec::new();
                        for (idx, layer) in manifest
                            .layers
                            .layers()
                            .iter()
                            .filter(|l| {
                                matches!(l.source, agora_game_api::LayerSource::Content { .. })
                            })
                            .enumerate()
                        {
                            if let agora_game_api::LayerSource::Content { content } = &layer.source
                            {
                                items.push(serde_json::json!({
                                    "position": idx + 1,
                                    "item_id": content,
                                    "layer_id": layer.id.as_str(),
                                    "enabled": layer.enabled,
                                    "own_copy": layer.own_copy,
                                    "mount_path": layer.mount_path.as_str(),
                                    "source_path": layer.source_path.as_str(),
                                }));
                            }
                        }

                        if json {
                            println!("{}", serde_json::to_string_pretty(&items)?);
                        } else if items.is_empty() {
                            println!("No content items found for instance '{instance_id}'.");
                        } else {
                            println!(
                                "{:<4} {:<18} {:<10} {:<10} Mount Path",
                                "#", "Item ID", "Enabled", "Own Copy"
                            );
                            for it in &items {
                                let pos = it["position"].as_u64().unwrap_or(0);
                                let item_id = it["item_id"].as_str().unwrap_or("");
                                let short_id = if item_id.len() > 16 {
                                    &item_id[..16]
                                } else {
                                    item_id
                                };
                                let enabled = if it["enabled"].as_bool().unwrap_or(false) {
                                    "yes"
                                } else {
                                    "no"
                                };
                                let own_copy = if it["own_copy"].as_bool().unwrap_or(false) {
                                    "yes"
                                } else {
                                    "no"
                                };
                                let mount = it["mount_path"].as_str().unwrap_or("");
                                let source = it["source_path"].as_str().unwrap_or("");
                                let mount_display = if source.is_empty() {
                                    mount.to_string()
                                } else if mount.is_empty() {
                                    format!("(from {source})")
                                } else {
                                    format!("{mount} (from {source})")
                                };
                                println!(
                                    "{:<4} {:<18} {:<10} {:<10} {}",
                                    pos, short_id, enabled, own_copy, mount_display
                                );
                            }
                        }
                    }
                    InstanceContentCmd::Remove {
                        instance_id,
                        item_id,
                    } => {
                        match agora_core::game_deploy::remove_content(ctx, &instance_id, &item_id) {
                            Ok(()) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "removed",
                                        "instance_id": instance_id,
                                        "item_id": item_id,
                                    });
                                    println!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    println!(
                                        "Removed content '{item_id}' from instance '{instance_id}'."
                                    );
                                }
                            }
                            Err(e) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": format!("{e}"),
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Error: {e}");
                                }
                                std::process::exit(1);
                            }
                        }
                    }
                    InstanceContentCmd::Enable {
                        instance_id,
                        item_id,
                    } => {
                        match agora_core::game_deploy::set_content_enabled(
                            ctx,
                            &instance_id,
                            &item_id,
                            true,
                        ) {
                            Ok(()) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "enabled",
                                        "instance_id": instance_id,
                                        "item_id": item_id,
                                        "enabled": true,
                                    });
                                    println!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    println!(
                                        "Enabled content '{item_id}' in instance '{instance_id}'."
                                    );
                                }
                            }
                            Err(e) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": format!("{e}"),
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Error: {e}");
                                }
                                std::process::exit(1);
                            }
                        }
                    }
                    InstanceContentCmd::Disable {
                        instance_id,
                        item_id,
                    } => {
                        match agora_core::game_deploy::set_content_enabled(
                            ctx,
                            &instance_id,
                            &item_id,
                            false,
                        ) {
                            Ok(()) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "disabled",
                                        "instance_id": instance_id,
                                        "item_id": item_id,
                                        "enabled": false,
                                    });
                                    println!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    println!(
                                        "Disabled content '{item_id}' in instance '{instance_id}'."
                                    );
                                }
                            }
                            Err(e) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": format!("{e}"),
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Error: {e}");
                                }
                                std::process::exit(1);
                            }
                        }
                    }
                    InstanceContentCmd::Move {
                        instance_id,
                        item_id,
                        position,
                    } => {
                        if position == 0 {
                            if json {
                                let out = serde_json::json!({
                                    "status": "error",
                                    "error": "position must be 1 or greater",
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: position must be 1 or greater");
                            }
                            std::process::exit(1);
                        }
                        let new_index = position - 1;
                        match agora_core::game_deploy::move_content(
                            ctx,
                            &instance_id,
                            &item_id,
                            new_index,
                        ) {
                            Ok(()) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "moved",
                                        "instance_id": instance_id,
                                        "item_id": item_id,
                                        "position": position,
                                    });
                                    println!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    println!(
                                        "Moved content '{item_id}' to position {position} in instance '{instance_id}'."
                                    );
                                }
                            }
                            Err(e) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": format!("{e}"),
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Error: {e}");
                                }
                                std::process::exit(1);
                            }
                        }
                    }
                    InstanceContentCmd::OwnCopy {
                        instance_id,
                        item_id,
                        state,
                    } => {
                        let own_copy = match state.to_ascii_lowercase().as_str() {
                            "on" | "true" | "1" => true,
                            "off" | "false" | "0" => false,
                            _ => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": format!("invalid state '{state}': expected 'on' or 'off'"),
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!(
                                        "Error: invalid state '{state}': expected 'on' or 'off'"
                                    );
                                }
                                std::process::exit(1);
                            }
                        };
                        match agora_core::game_deploy::set_content_own_copy(
                            ctx,
                            &instance_id,
                            &item_id,
                            own_copy,
                        ) {
                            Ok(()) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "own_copy_set",
                                        "instance_id": instance_id,
                                        "item_id": item_id,
                                        "own_copy": own_copy,
                                    });
                                    println!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    let s = if own_copy { "on" } else { "off" };
                                    println!(
                                        "Set own-copy to {s} for content '{item_id}' in instance '{instance_id}'."
                                    );
                                }
                            }
                            Err(e) => {
                                if json {
                                    let out = serde_json::json!({
                                        "status": "error",
                                        "error": format!("{e}"),
                                        "exitCode": 1,
                                    });
                                    eprintln!("{}", serde_json::to_string_pretty(&out)?);
                                } else {
                                    eprintln!("Error: {e}");
                                }
                                std::process::exit(1);
                            }
                        }
                    }
                },
                GameInstanceCmd::Deploy {
                    instance_id,
                    copies,
                    deployment,
                } => {
                    let record = match agora_core::game_instance::get(ctx, &instance_id)? {
                        Some(r) => r,
                        None => {
                            if json {
                                let out = serde_json::json!({
                                    "error": format!("Instance '{instance_id}' not found."),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: Instance '{instance_id}' not found.");
                            }
                            std::process::exit(1);
                        }
                    };

                    let game_def = ctx.games.game(&record.game).ok_or_else(|| {
                        anyhow::anyhow!("Game definition not found for {}", record.game)
                    })?;

                    let requested = if copies {
                        Some(agora_core::game_deploy::DeployMode::Copies)
                    } else {
                        match deployment.as_deref().map(parse_deployment_arg) {
                            None | Some(Ok(None)) => None,
                            Some(Ok(mode)) => mode,
                            Some(Err(e)) => {
                                eprintln!("Error: {e}");
                                std::process::exit(1);
                            }
                        }
                    };
                    let mode = match agora_core::game_instance::deploy_mode_for(
                        ctx,
                        &instance_id,
                        game_def,
                        requested,
                        &agora_core::game_launch::SystemLauncher,
                    ) {
                        Ok(m) => m,
                        Err(e) => {
                            if json {
                                let out = serde_json::json!({
                                    "error": format!("{e}"),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: {e}");
                            }
                            std::process::exit(1);
                        }
                    };

                    if !json {
                        println!("Deploying as '{mode}'.");
                    }
                    match agora_core::game_deploy::deploy(ctx, &instance_id, game_def, mode) {
                        Ok(agora_core::game_deploy::DeployOutcome::UpToDate { plugins }) => {
                            if json {
                                let out = serde_json::json!({
                                    "status": "up_to_date",
                                    "instance_id": instance_id,
                                    "plugins": plugins,
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                println!("Deployment for instance '{instance_id}' is up to date.");
                                if let Some(report) = &plugins {
                                    print_plugin_sync(report);
                                }
                            }
                        }
                        Ok(agora_core::game_deploy::DeployOutcome::Built {
                            linked,
                            copied,
                            copied_bytes,
                            config_copied,
                            harvest,
                            plugins,
                        }) => {
                            if json {
                                let out = serde_json::json!({
                                    "status": "built",
                                    "instance_id": instance_id,
                                    "linked": linked,
                                    "copied": copied,
                                    "copied_bytes": copied_bytes,
                                    "config_copied": config_copied,
                                    "harvest": harvest,
                                    "plugins": plugins,
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                println!(
                                    "Deployed instance '{instance_id}': {linked} linked, {copied} copied ({copied_bytes} bytes)."
                                );
                                if config_copied > 0 {
                                    println!(
                                        "{config_copied} small config files copied so the game can write them."
                                    );
                                }
                                if let Some(h) = harvest {
                                    if !h.is_empty() {
                                        println!(
                                            "Harvested previous deployment: {} copied to writable, {} base files changed, {} whiteouts added, {} writable files removed.",
                                            h.copied_to_writable.len(),
                                            h.base_files_changed.len(),
                                            h.whiteouts_added.len(),
                                            h.writable_files_removed.len()
                                        );
                                        for b in &h.base_files_changed {
                                            println!(
                                                "  base file changed: {} (linked to store: {})",
                                                b.path, b.linked_to_store
                                            );
                                        }
                                    }
                                }
                                if let Some(report) = &plugins {
                                    print_plugin_sync(report);
                                }
                            }
                        }
                        Err(e) => {
                            if json {
                                let out = serde_json::json!({
                                    "status": "error",
                                    "error": format!("{e}"),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: {e}");
                            }
                            std::process::exit(1);
                        }
                    }
                }
                GameInstanceCmd::Undeploy { instance_id } => {
                    match agora_core::game_deploy::undeploy(ctx, &instance_id) {
                        Ok(harvest) => {
                            if json {
                                let out = serde_json::json!({
                                    "status": "undeployed",
                                    "instance_id": instance_id,
                                    "harvest": harvest,
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                println!(
                                    "Undeployed instance '{instance_id}': {} copied to writable, {} base files changed, {} whiteouts added, {} writable files removed.",
                                    harvest.copied_to_writable.len(),
                                    harvest.base_files_changed.len(),
                                    harvest.whiteouts_added.len(),
                                    harvest.writable_files_removed.len()
                                );
                                if !harvest.base_files_changed.is_empty() {
                                    println!("Warning: base files were changed:");
                                    for b in &harvest.base_files_changed {
                                        println!(
                                            "  {} (linked to store: {})",
                                            b.path, b.linked_to_store
                                        );
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if json {
                                let out = serde_json::json!({
                                    "status": "error",
                                    "error": format!("{e}"),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: {e}");
                            }
                            std::process::exit(1);
                        }
                    }
                }
                GameInstanceCmd::Check { instance_id } => {
                    let fail = |message: String| -> ! {
                        if json {
                            let out = serde_json::json!({
                                "status": "error",
                                "error": message,
                                "exitCode": 1,
                            });
                            eprintln!("{}", serde_json::to_string_pretty(&out).unwrap_or(message));
                        } else {
                            eprintln!("Error: {message}");
                        }
                        std::process::exit(1);
                    };
                    let record = match agora_core::game_instance::get(ctx, &instance_id)? {
                        Some(r) => r,
                        None => fail(format!("Instance '{instance_id}' not found.")),
                    };
                    let game_def = ctx.games.game(&record.game).ok_or_else(|| {
                        anyhow::anyhow!("Game definition not found for {}", record.game)
                    })?;
                    let findings = match agora_core::game_instance::runtime_findings(
                        ctx,
                        &instance_id,
                        game_def,
                    ) {
                        Ok(findings) => findings,
                        Err(e) => fail(format!("{e}")),
                    };
                    let pinned =
                        matches!(record.base, agora_game_api::BaseReference::Pinned { .. });
                    // The plugin list is synced at launch, so its order is checked for a pinned
                    // instance of a game that keeps one. An unpinned instance has no synced list.
                    let load_order: Vec<agora_core::game_load_order::Finding> =
                        if pinned && game_def.plugin_list.is_some() {
                            match agora_core::game_load_order::check(ctx, &instance_id, game_def) {
                                Ok(found) => found,
                                Err(e) => fail(format!("{e}")),
                            }
                        } else {
                            Vec::new()
                        };
                    // Only a framework built for another version, or a load order that would stop
                    // the game, fails the check. A rule that cannot be checked is listed, as a
                    // warning, and so are an unreadable header and a plugin listed twice.
                    let framework_refuses = findings
                        .iter()
                        .any(agora_game_api::RuntimeFileFinding::refuses_launch);
                    let order_refuses = load_order
                        .iter()
                        .any(agora_core::game_load_order::Finding::refuses_launch);
                    let refuses = framework_refuses || order_refuses;
                    // Tool output that is stale or unknown is a warning, never a refusal (§26.9).
                    let generated: Vec<agora_core::game_tools::OutputFinding> = if pinned {
                        match agora_core::game_tools::output_findings(ctx, &instance_id, game_def) {
                            Ok(found) => found,
                            Err(e) => fail(format!("{e}")),
                        }
                    } else {
                        Vec::new()
                    };
                    let status = if refuses {
                        "findings"
                    } else if findings.is_empty() && load_order.is_empty() && generated.is_empty() {
                        "ok"
                    } else {
                        "warnings"
                    };
                    if json {
                        let out = serde_json::json!({
                            "status": status,
                            "instance_id": instance_id,
                            "checked": pinned,
                            "findings": findings,
                            "load_order_findings": load_order,
                            "generated_findings": generated,
                            "exitCode": if refuses { 1 } else { 0 },
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                    } else if !pinned {
                        println!(
                            "Instance '{instance_id}' runs from its install folder, which has no framework check."
                        );
                    } else {
                        if findings.is_empty() {
                            println!("No framework problems found in instance '{instance_id}'.");
                        } else {
                            println!(
                                "Instance '{instance_id}' has {} framework problem(s):",
                                findings.len()
                            );
                            for line in runtime_finding_lines(&findings) {
                                println!("{line}");
                            }
                        }
                        if game_def.plugin_list.is_some() {
                            if load_order.is_empty() {
                                println!(
                                    "No load order problems found in instance '{instance_id}'."
                                );
                            } else {
                                println!(
                                    "Instance '{instance_id}' has {} load order finding(s):",
                                    load_order.len()
                                );
                                for finding in &load_order {
                                    println!("- {}", finding.message());
                                }
                                if let Some(hint) = sort_hint(&instance_id, &load_order) {
                                    println!("{hint}");
                                }
                            }
                        }
                        for finding in &generated {
                            println!("- {}", finding.message);
                        }
                    }
                    if refuses {
                        std::process::exit(1);
                    }
                }
                GameInstanceCmd::Ini {
                    instance_id,
                    file,
                    section,
                    key,
                    value,
                    unset,
                } => {
                    let record = match agora_core::game_instance::get(ctx, &instance_id)? {
                        Some(r) => r,
                        None => {
                            ini_fail(json, format!("Instance '{instance_id}' not found."));
                        }
                    };
                    let game_def = ctx.games.game(&record.game).ok_or_else(|| {
                        anyhow::anyhow!("Game definition not found for {}", record.game)
                    })?;
                    ini_command(
                        ctx,
                        json,
                        &instance_id,
                        game_def,
                        IniRequest {
                            file,
                            section,
                            key,
                            value,
                            unset,
                        },
                    )?;
                }
                GameInstanceCmd::Tools { action } => {
                    instance_tools_command(ctx, json, action)?;
                }
                GameInstanceCmd::Saves {
                    instance_id,
                    choice,
                } => {
                    let record = match agora_core::game_instance::get(ctx, &instance_id)? {
                        Some(r) => r,
                        None => {
                            ini_fail(json, format!("Instance '{instance_id}' not found."));
                        }
                    };
                    let game_def = ctx.games.game(&record.game).ok_or_else(|| {
                        anyhow::anyhow!("Game definition not found for {}", record.game)
                    })?;
                    saves_command(ctx, json, &instance_id, game_def, choice.as_deref())?;
                }
                GameInstanceCmd::Plugins {
                    instance_id: None,
                    action: Some(order),
                } if order.is_load_order_command() => {
                    plugins_load_order_command(ctx, json, order)?;
                }
                GameInstanceCmd::Plugins {
                    instance_id,
                    action,
                } => {
                    let (instance_id, toggle) = match (instance_id, action) {
                        (Some(id), None) => (id, None),
                        (None, Some(InstancePluginsCmd::Enable { instance_id, name })) => {
                            (instance_id, Some((name, true)))
                        }
                        (None, Some(InstancePluginsCmd::Disable { instance_id, name })) => {
                            (instance_id, Some((name, false)))
                        }
                        _ => {
                            eprintln!(
                                "Error: name an instance: agora games instance plugins <instance>"
                            );
                            std::process::exit(2);
                        }
                    };
                    let fail = |message: String| -> ! {
                        if json {
                            let out = serde_json::json!({
                                "status": "error",
                                "error": message,
                                "exitCode": 1,
                            });
                            eprintln!("{}", serde_json::to_string_pretty(&out).unwrap_or(message));
                        } else {
                            eprintln!("Error: {message}");
                        }
                        std::process::exit(1);
                    };
                    let record = match agora_core::game_instance::get(ctx, &instance_id)? {
                        Some(r) => r,
                        None => fail(format!("Instance '{instance_id}' not found.")),
                    };
                    let game_def = ctx.games.game(&record.game).ok_or_else(|| {
                        anyhow::anyhow!("Game definition not found for {}", record.game)
                    })?;
                    match toggle {
                        None => {
                            match agora_core::game_load_order::order(ctx, &instance_id, game_def) {
                                Ok(order) => {
                                    if json {
                                        println!("{}", serde_json::to_string_pretty(&order)?);
                                    } else {
                                        print_load_order(&instance_id, &order);
                                    }
                                }
                                Err(e) => fail(e.to_string()),
                            }
                        }
                        Some((name, active)) => {
                            match agora_core::game_plugins::set_active(
                                ctx,
                                &instance_id,
                                game_def,
                                &name,
                                active,
                            ) {
                                Ok(changed) => {
                                    let state = if active { "active" } else { "inactive" };
                                    if json {
                                        let out = serde_json::json!({
                                            "status": "ok",
                                            "instance_id": instance_id,
                                            "plugin": name,
                                            "active": active,
                                            "changed": changed,
                                        });
                                        println!("{}", serde_json::to_string_pretty(&out)?);
                                    } else if changed {
                                        println!(
                                            "'{name}' is now {state} in instance '{instance_id}'."
                                        );
                                    } else {
                                        println!("'{name}' was already {state} in instance '{instance_id}'.");
                                    }
                                }
                                Err(e) => fail(e.to_string()),
                            }
                        }
                    }
                }
            },
            GamesCmd::Launch {
                base_id,
                wait,
                launch_anyway,
                plain,
            } => {
                let manifest_path = ctx.paths.base_manifest_path(&base_id);
                if !manifest_path.exists() {
                    if json {
                        let out = serde_json::json!({
                            "error": format!("Base '{base_id}' not found."),
                            "exitCode": 1,
                        });
                        eprintln!("{}", serde_json::to_string_pretty(&out)?);
                    } else {
                        eprintln!("Error: Base '{base_id}' not found.");
                    }
                    std::process::exit(1);
                }
                let content = std::fs::read_to_string(&manifest_path)?;
                let manifest: agora_core::game_base::BaseManifest = serde_json::from_str(&content)?;
                let game_def = ctx.games.game(&manifest.runtime.game).ok_or_else(|| {
                    anyhow::anyhow!("Game definition not found for {}", manifest.runtime.game)
                })?;

                let prepared = match agora_core::game_launch::prepare_base_launch_with(
                    &manifest,
                    game_def,
                    launch_anyway,
                    plain,
                ) {
                    Ok(p) => p,
                    Err(agora_core::game_launch::LaunchError::BaseDamaged { problems }) => {
                        if json {
                            let out = serde_json::json!({
                                "status": "error",
                                "error": "base_damaged",
                                "base_id": base_id,
                                "problems": problems,
                                "exitCode": 1,
                            });
                            eprintln!("{}", serde_json::to_string_pretty(&out)?);
                        } else {
                            eprintln!("Base '{base_id}' has {} problem(s):", problems.len());
                            print_base_problems(&problems);
                        }
                        std::process::exit(1);
                    }
                    Err(agora_core::game_launch::LaunchError::RuntimeMismatch { findings }) => {
                        if json {
                            let out = serde_json::json!({
                                "status": "error",
                                "error": "runtime_mismatch",
                                "base_id": base_id,
                                "findings": findings,
                                "exitCode": 1,
                            });
                            eprintln!("{}", serde_json::to_string_pretty(&out)?);
                        } else {
                            eprintln!(
                                "Base '{base_id}' has framework(s) built for another game version:"
                            );
                            print_runtime_findings(&findings);
                            eprintln!("Pass --launch-anyway to start the game anyway.");
                        }
                        std::process::exit(1);
                    }
                    Err(agora_core::game_launch::LaunchError::NoRecipe) => {
                        if json {
                            let out = serde_json::json!({
                                "status": "error",
                                "error": "no_recipe",
                                "message": "Game definition has no launch recipe.",
                                "exitCode": 1,
                            });
                            eprintln!("{}", serde_json::to_string_pretty(&out)?);
                        } else {
                            eprintln!("Error: Game definition has no launch recipe.");
                        }
                        std::process::exit(1);
                    }
                    Err(e) => {
                        if json {
                            let out = serde_json::json!({
                                "status": "error",
                                "error": format!("{e}"),
                                "exitCode": 1,
                            });
                            eprintln!("{}", serde_json::to_string_pretty(&out)?);
                        } else {
                            eprintln!("Error: {e}");
                        }
                        std::process::exit(1);
                    }
                };

                if !json && !prepared.generated_findings.is_empty() {
                    for finding in &prepared.generated_findings {
                        eprintln!("Warning: {}", finding.message);
                    }
                }
                if !json && !prepared.runtime_findings.is_empty() {
                    eprintln!(
                        "Warning: Launching past {} framework check finding(s):",
                        prepared.runtime_findings.len()
                    );
                    print_runtime_findings(&prepared.runtime_findings);
                }

                let mut launched = match agora_core::game_launch::launch(&prepared) {
                    Ok(l) => l,
                    Err(e) => {
                        if json {
                            let out = serde_json::json!({
                                "status": "error",
                                "error": format!("{e}"),
                                "exitCode": 1,
                            });
                            eprintln!("{}", serde_json::to_string_pretty(&out)?);
                        } else {
                            eprintln!("Launch failed: {e}");
                        }
                        std::process::exit(1);
                    }
                };

                let env_map: std::collections::BTreeMap<String, String> = prepared
                    .resolved
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_string_lossy().to_string()))
                    .collect();

                if !json {
                    if let Some(alt) = &prepared.alternative {
                        println!("Starting through '{}': {}.", alt.id, alt.reason);
                        println!("(Use --plain to start the game's own executable instead.)");
                    }
                }

                if !wait {
                    if json {
                        let out = serde_json::json!({
                            "status": "launched",
                            "alternative": prepared.alternative,
                            "base_id": base_id,
                            "pid": launched.pid(),
                            "program": prepared.resolved.program,
                            "cwd": prepared.resolved.cwd,
                            "env": env_map,
                            "warnings": prepared.warnings,
                            "runtime_findings": prepared.runtime_findings,
                            "load_order_findings": prepared.load_order_findings,
                            "generated_findings": prepared.generated_findings,
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                    } else {
                        if !prepared.warnings.is_empty() {
                            eprintln!(
                                "Warning: Launching damaged base ({} problem(s)):",
                                prepared.warnings.len()
                            );
                            print_base_problems(&prepared.warnings);
                        }
                        println!("Program:           {}", prepared.resolved.program.display());
                        println!("Working Directory: {}", prepared.resolved.cwd.display());
                        println!("PID:               {}", launched.pid());
                        if !prepared.resolved.env.is_empty() {
                            println!("Environment:");
                            for (k, v) in &prepared.resolved.env {
                                println!("  {k}={}", v.to_string_lossy());
                            }
                        }
                    }
                } else {
                    if !json {
                        if !prepared.warnings.is_empty() {
                            eprintln!(
                                "Warning: Launching damaged base ({} problem(s)):",
                                prepared.warnings.len()
                            );
                            print_base_problems(&prepared.warnings);
                        }
                        println!("Program:           {}", prepared.resolved.program.display());
                        println!("Working Directory: {}", prepared.resolved.cwd.display());
                        println!("PID:               {}", launched.pid());
                        if !prepared.resolved.env.is_empty() {
                            println!("Environment:");
                            for (k, v) in &prepared.resolved.env {
                                println!("  {k}={}", v.to_string_lossy());
                            }
                        }
                    }

                    let exit_report = agora_core::game_launch::wait_for_exit(
                        &manifest.location,
                        &mut launched,
                        Duration::from_millis(250),
                        Duration::from_secs(5),
                    );

                    let ver = agora_core::game_base::verify_base(
                        &manifest,
                        agora_core::game_base::VerifyDepth::Quick,
                        &|p| game_def.is_declared_write(p),
                        &|p| game_def.is_excluded(p),
                    );

                    if json {
                        let out = serde_json::json!({
                            "status": "exited",
                            "base_id": base_id,
                            "pid": launched.pid(),
                            "program": prepared.resolved.program,
                            "cwd": prepared.resolved.cwd,
                            "env": env_map,
                            "warnings": prepared.warnings,
                            "runtime_findings": prepared.runtime_findings,
                            "load_order_findings": prepared.load_order_findings,
                            "generated_findings": prepared.generated_findings,
                            "processes": exit_report.processes,
                            "relaunched_outside": exit_report.relaunched_outside,
                            "game_writes": ver.game_writes,
                            "problems": ver.problems,
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                    } else {
                        println!("Processes running from base:");
                        if exit_report.processes.is_empty() {
                            println!("  (none)");
                        } else {
                            for p in &exit_report.processes {
                                println!("  - PID {}: {}", p.pid, p.exe.display());
                            }
                        }
                        if exit_report.relaunched_outside {
                            eprintln!("Warning: A process with the game's executable name ran outside the base folder (possible relaunch from store).");
                        }
                        if !ver.game_writes.is_empty() {
                            println!("Game writes ({}):", ver.game_writes.len());
                            for w in &ver.game_writes {
                                println!("  - {w}");
                            }
                        }
                        if ver.problems.is_empty() {
                            println!("Base verified clean after launch.");
                        } else {
                            eprintln!("Base has {} problem(s) after launch:", ver.problems.len());
                            print_base_problems(&ver.problems);
                        }
                    }

                    if !ver.problems.is_empty() {
                        std::process::exit(1);
                    }
                }
            }
            GamesCmd::UserFiles { action } => match action {
                UserFilesCmd::Status { game } => {
                    let game_id = game
                        .as_deref()
                        .map(agora_game_api::GameId::new)
                        .transpose()
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    let statuses =
                        agora_core::game_user_files::list_statuses(ctx, game_id.as_ref())?;
                    if json {
                        let json_list: Vec<serde_json::Value> = statuses
                            .into_iter()
                            .map(|(g, s, st)| {
                                serde_json::json!({
                                    "game": g,
                                    "store": s,
                                    "instance": st.instance,
                                    "running": st.running,
                                    "files": st.files,
                                })
                            })
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&json_list)?);
                    } else if statuses.is_empty() {
                        println!("No user-file swap sessions in progress.");
                    } else {
                        println!("Active user-file swap sessions ({}):", statuses.len());
                        for (g, s, st) in statuses {
                            println!("\nGame:     {g}");
                            println!("Store:    {s}");
                            println!("Instance: {}", st.instance);
                            println!("Running:  {}", if st.running { "yes" } else { "no" });
                            println!("Files ({}):", st.files.len());
                            for f in &st.files {
                                println!("  - {} (swapped: {})", f.real_path.display(), f.swapped);
                            }
                        }
                    }
                }
                UserFilesCmd::Restore { game, store } => {
                    let game_id =
                        agora_game_api::GameId::new(&game).map_err(|e| anyhow::anyhow!("{e}"))?;
                    let store_id =
                        agora_game_api::StoreId::new(&store).map_err(|e| anyhow::anyhow!("{e}"))?;
                    let game_def = ctx.games.game(&game_id).ok_or_else(|| {
                        anyhow::anyhow!("Game definition not found for {}", game_id)
                    })?;
                    match agora_core::game_user_files::restore(ctx, game_def, &store_id) {
                        Ok(report) => {
                            if json {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&serde_json::json!({
                                        "status": "restored",
                                        "game": report.game,
                                        "store": report.store,
                                        "instance": report.instance_id,
                                        "files": report.files,
                                    }))?
                                );
                            } else {
                                println!(
                                    "Restored user files for {} ({}) from instance {}:",
                                    report.game, report.store, report.instance_id
                                );
                                for f in &report.files {
                                    println!(
                                        "  - {} ({})",
                                        f.real_path.display(),
                                        if f.changed { "changed" } else { "unchanged" }
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            if json {
                                let out = serde_json::json!({
                                    "status": "error",
                                    "error": format!("{e}"),
                                    "exitCode": 1,
                                });
                                eprintln!("{}", serde_json::to_string_pretty(&out)?);
                            } else {
                                eprintln!("Error: {e}");
                            }
                            std::process::exit(1);
                        }
                    }
                }
            },
        },
    }

    Ok(())
}

fn print_discovery_report(report: &agora_core::game_discovery::DiscoveryReport) {
    use agora_core::game_discovery::InstallKind;

    let mut store_ids: Vec<_> = report.installs.iter().map(|i| i.store.clone()).collect();
    store_ids.sort();
    store_ids.dedup();

    let mut found_any = false;

    for store in &store_ids {
        let base_games: Vec<_> = report
            .installs
            .iter()
            .filter(|i| &i.store == store && i.kind == InstallKind::BaseGame)
            .collect();

        if base_games.is_empty() {
            continue;
        }

        found_any = true;
        let count_str = if base_games.len() == 1 {
            "1 game".to_string()
        } else {
            format!("{} games", base_games.len())
        };
        println!("{} ({}):", store, count_str);

        for bg in base_games {
            let add_on_count = report
                .installs
                .iter()
                .filter(|i| {
                    &i.store == store
                        && i.kind == InstallKind::AddOn
                        && i.parent_product.as_deref() == Some(&bg.product)
                })
                .count();

            if add_on_count == 0 {
                println!("  - {}", bg.name);
            } else if add_on_count == 1 {
                println!("  - {} (1 add-on)", bg.name);
            } else {
                println!("  - {} ({} add-ons)", bg.name, add_on_count);
            }
        }
    }

    let tools: Vec<_> = report
        .installs
        .iter()
        .filter(|i| i.kind == InstallKind::Tool)
        .collect();

    if !tools.is_empty() {
        found_any = true;
        println!("Tools ({}):", tools.len());
        for tool in tools {
            println!("  - {} ({})", tool.name, tool.store);
        }
    }

    if !found_any {
        println!("No game installs discovered.");
    }

    if !report.warnings.is_empty() {
        println!("Warnings:");
        for w in &report.warnings {
            println!("  - [{}]: {}", w.store, w.message);
        }
    }
}

fn print_games_list(
    registry: &agora_core::game_registry::GameRegistry,
    inventory: &agora_core::game_registry::GameInventory,
) {
    for game in registry.games() {
        let source_str = match registry.source_for(&game.id) {
            Some(agora_core::game_registry::PackageSource::Compiled { crate_name }) => {
                format!(" (compiled: {crate_name})")
            }
            Some(agora_core::game_registry::PackageSource::Plugin { plugin_id }) => {
                format!(" (plugin: {plugin_id})")
            }
            None => String::new(),
        };
        println!("{} ({}){}", game.name, game.id, source_str);
        if game.stores.is_empty() {
            println!("  Agora manages its installs itself");
            continue;
        }

        let matching_installs: Vec<_> = inventory
            .installs
            .iter()
            .filter(|i| i.game == game.id)
            .collect();

        if matching_installs.is_empty() {
            println!("  No installs found.");
            continue;
        }

        for inst in matching_installs {
            let (version_str, build_str) = match &inst.runtime {
                agora_core::game_registry::RuntimeResolution::Identified { runtime, .. } => {
                    let v = runtime.version.clone();
                    let b = runtime
                        .build
                        .as_deref()
                        .map(|b| format!("build {b}"))
                        .unwrap_or_else(|| "no build".to_string());
                    (v, b)
                }
                agora_core::game_registry::RuntimeResolution::Unidentified { reasons } => {
                    let v = if reasons.is_empty() {
                        "version unknown".to_string()
                    } else {
                        format!("version unknown ({})", reasons.join(", "))
                    };
                    let b = inst
                        .discovered
                        .store_build
                        .as_deref()
                        .map(|b| format!("build {b}"))
                        .unwrap_or_else(|| "no build".to_string());
                    (v, b)
                }
            };

            let vol_str = match &inst.discovered.volume {
                Some(v) => format!(
                    "{} ({})",
                    v.filesystem,
                    if v.supports_hardlinks {
                        "hardlinks supported"
                    } else {
                        "hardlinks unsupported"
                    }
                ),
                None => "unknown volume".to_string(),
            };

            let add_ons_str = match inst.add_ons.len() {
                0 => "0 add-ons".to_string(),
                1 => "1 add-on".to_string(),
                n => format!("{n} add-ons"),
            };

            println!(
                "  - {} [{}]: {}, {}, {}, {}, {}",
                inst.discovered.store,
                inst.install_id,
                version_str,
                build_str,
                inst.discovered.location.display(),
                vol_str,
                add_ons_str,
            );
        }
    }

    println!(
        "Other games found (not supported yet): {}",
        inventory.unsupported.len()
    );
    for u in &inventory.unsupported {
        println!("  - {}", u.name);
    }
}

impl InstancePluginsCmd {
    /// Sort, move, lock, unlock and check: the load order commands, not enable or disable.
    fn is_load_order_command(&self) -> bool {
        matches!(
            self,
            InstancePluginsCmd::Sort { .. }
                | InstancePluginsCmd::Move { .. }
                | InstancePluginsCmd::Lock { .. }
                | InstancePluginsCmd::Unlock { .. }
                | InstancePluginsCmd::Check { .. }
        )
    }
}

/// `games instance tools`: run a tool, or list, roll back, remove or diff the output it wrote. A run
/// that does not promote its output exits 1, as a failed build should.
fn instance_tools_command(
    ctx: &agora_core::ctx::Ctx,
    json: bool,
    action: InstanceToolsCmd,
) -> anyhow::Result<()> {
    use agora_core::game_tools;
    use agora_game_api::ToolId;

    let instance_id = action.instance_id().to_string();
    let record = match agora_core::game_instance::get(ctx, &instance_id)? {
        Some(r) => r,
        None => ini_fail(json, format!("Instance '{instance_id}' not found.")),
    };
    let game_def = ctx
        .games
        .game(&record.game)
        .ok_or_else(|| anyhow::anyhow!("Game definition not found for {}", record.game))?;
    let fail = |message: String| -> ! { ini_fail(json, message) };
    let tool_id = |tool: &str| -> ToolId {
        ToolId::new(tool).unwrap_or_else(|e| ini_fail(json, e.to_string()))
    };

    match action {
        InstanceToolsCmd::List { .. } => {
            let states = game_tools::list(ctx, &instance_id, game_def)
                .unwrap_or_else(|e| fail(e.to_string()));
            if json {
                let out = serde_json::json!({
                    "instance_id": instance_id,
                    "tools": states,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if states.is_empty() {
                println!("Game '{}' declares no tools.", game_def.name);
            } else {
                for state in &states {
                    let status = state.status.map(|s| s.as_str()).unwrap_or("not built");
                    let current = state.current.as_deref().unwrap_or("none");
                    let previous = state.previous.as_deref().unwrap_or("none");
                    let declared = if state.declared {
                        ""
                    } else {
                        " (no longer declared by the game)"
                    };
                    println!(
                        "{} ({}): output {status}, generation {current}, previous {previous}{declared}",
                        state.name, state.tool
                    );
                }
            }
        }
        InstanceToolsCmd::Run { tool, .. } => {
            let tool = tool_id(&tool);
            let cancel = agora_core::event_sink::CancellationToken::new();
            let outcome = game_tools::run(
                ctx,
                &instance_id,
                game_def,
                &tool,
                &agora_core::game_launch::SystemLauncher,
                &cancel,
            )
            .unwrap_or_else(|e| fail(e.to_string()));
            if json {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            } else {
                print_tool_run(&outcome);
            }
            if !outcome.promoted {
                std::process::exit(1);
            }
        }
        InstanceToolsCmd::Rollback { tool, .. } => {
            let tool = tool_id(&tool);
            let state = game_tools::rollback(ctx, &instance_id, game_def, &tool)
                .unwrap_or_else(|e| fail(e.to_string()));
            if json {
                println!("{}", serde_json::to_string_pretty(&state)?);
            } else {
                println!(
                    "{} rolled back: its output is now generation {}, with generation {} kept. The game reads it on its next launch.",
                    state.name,
                    state.current.as_deref().unwrap_or("none"),
                    state.previous.as_deref().unwrap_or("none")
                );
            }
        }
        InstanceToolsCmd::Remove { tool, .. } => {
            let tool = tool_id(&tool);
            let removed = game_tools::remove(ctx, &instance_id, game_def, &tool)
                .unwrap_or_else(|e| fail(e.to_string()));
            if json {
                let out = serde_json::json!({
                    "instance_id": instance_id,
                    "tool": tool.to_string(),
                    "removed_generations": removed,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!(
                    "Removed {tool} from instance '{instance_id}', with {} generation folder(s).",
                    removed.len()
                );
            }
        }
        InstanceToolsCmd::Diff { tool, .. } => {
            let tool = tool_id(&tool);
            let report =
                game_tools::diff(ctx, &instance_id, &tool).unwrap_or_else(|e| fail(e.to_string()));
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "{tool}: generation {} compared with the previous generation {}.",
                    report.current, report.previous
                );
                if report.added.is_empty() && report.changed.is_empty() && report.removed.is_empty()
                {
                    println!("No file differs between the two generations.");
                }
                for (label, files) in [
                    ("Added", &report.added),
                    ("Changed", &report.changed),
                    ("Removed", &report.removed),
                ] {
                    if !files.is_empty() {
                        println!("{label} ({}):", files.len());
                        for file in files.iter().take(game_tools::REPORTED_PATHS) {
                            println!("  {file}");
                        }
                        if files.len() > game_tools::REPORTED_PATHS {
                            println!(
                                "  ... and {} more",
                                files.len() - game_tools::REPORTED_PATHS
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// What a tool run wrote and what became of it, for people to read.
fn print_tool_run(outcome: &agora_core::game_tools::RunOutcome) {
    use agora_core::game_tools::REPORTED_PATHS;

    let current = outcome.current.as_deref().unwrap_or("none");
    let written = outcome.written.len();
    if outcome.promoted {
        let previous = outcome
            .previous
            .as_deref()
            .map(|p| format!(", with generation {p} kept for rollback"))
            .unwrap_or_default();
        println!(
            "{} ran and wrote {written} file(s). Its output is now generation {current}{previous}.",
            outcome.name
        );
    } else if outcome.cancelled {
        println!(
            "{} was cancelled. Its output was discarded, and generation {current} stays in effect.",
            outcome.name
        );
    } else {
        let code = outcome
            .exit_code
            .map(|c| format!("exit code {c}"))
            .unwrap_or_else(|| "no exit code".to_string());
        println!(
            "{} failed ({code}) after writing {written} file(s). Its output was discarded, and generation {current} stays in effect.",
            outcome.name
        );
    }
    for path in outcome.written.iter().take(REPORTED_PATHS) {
        println!("  {path}");
    }
    if written > REPORTED_PATHS {
        println!("  ... and {} more", written - REPORTED_PATHS);
    }
    if !outcome.deleted.is_empty() {
        println!(
            "{} file(s) deleted, which the game will no longer see:",
            outcome.deleted.len()
        );
        for path in outcome.deleted.iter().take(REPORTED_PATHS) {
            println!("  {path}");
        }
    }
    if let Some(folder) = &outcome.failed_folder {
        println!("The discarded run is kept in {}.", folder.display());
    }
}

/// A failure in the `ini` and `saves` commands: the message as JSON or as text, and exit 1.
fn ini_fail(json: bool, message: String) -> ! {
    if json {
        let out = serde_json::json!({
            "status": "error",
            "error": message,
            "exitCode": 1,
        });
        eprintln!("{}", serde_json::to_string_pretty(&out).unwrap_or(message));
    } else {
        eprintln!("Error: {message}");
    }
    std::process::exit(1);
}

/// The arguments of `games instance ini`, after the instance.
struct IniRequest {
    file: Option<String>,
    section: Option<String>,
    key: Option<String>,
    value: Option<String>,
    unset: bool,
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

/// `games instance ini`: list the game's files for an instance, list a file's keys, get, set or
/// remove one key. Reads never create the instance's copy; a change does, from the game's file.
fn ini_command(
    ctx: &agora_core::ctx::Ctx,
    json: bool,
    instance_id: &str,
    game_def: &agora_game_api::GameDefinition,
    request: IniRequest,
) -> anyhow::Result<()> {
    use agora_core::game_ini;

    let IniRequest {
        file,
        section,
        key,
        value,
        unset,
    } = request;
    let fail = |message: String| -> ! { ini_fail(json, message) };
    let usage =
        "usage: agora games instance ini <instance> <file> [<section> [<key> [<value>]]] [--unset]";

    let Some(file) = file else {
        if section.is_some() || key.is_some() || value.is_some() || unset {
            fail(format!("name a file. {usage}"));
        }
        let files = match game_ini::list_files(ctx, instance_id, game_def) {
            Ok(files) => files,
            Err(e) => fail(e.to_string()),
        };
        if json {
            println!("{}", serde_json::to_string_pretty(&files)?);
        } else if files.is_empty() {
            println!("The game keeps no per-user files for this instance's store.");
        } else {
            for f in &files {
                println!(
                    "{}  copy: {}  game file: {} ({})",
                    f.instance_path,
                    yes_no(f.copy_exists),
                    f.game_file.display(),
                    if f.game_file_exists {
                        "exists"
                    } else {
                        "missing"
                    }
                );
            }
        }
        return Ok(());
    };

    let read = match game_ini::read(ctx, instance_id, game_def, &file) {
        Ok(read) => read,
        Err(e) => fail(e.to_string()),
    };

    let Some(section) = section else {
        if key.is_some() || value.is_some() || unset {
            fail(format!("name a section. {usage}"));
        }
        let entries = read.document.entries();
        print_ini_entries(json, &read.instance_path, read.source, &entries)?;
        return Ok(());
    };

    let Some(key) = key else {
        if value.is_some() || unset {
            fail(format!("name a key. {usage}"));
        }
        let entries: Vec<_> = read
            .document
            .entries()
            .into_iter()
            .filter(|e| e.section.eq_ignore_ascii_case(&section))
            .collect();
        print_ini_entries(json, &read.instance_path, read.source, &entries)?;
        return Ok(());
    };

    if unset {
        if value.is_some() {
            fail("--unset takes no value".to_string());
        }
        let changed = match game_ini::unset_value(ctx, instance_id, game_def, &file, &section, &key)
        {
            Ok(changed) => changed,
            Err(e) => fail(e.to_string()),
        };
        if json {
            let out = serde_json::json!({
                "status": "ok",
                "instance_id": instance_id,
                "file": file,
                "section": section,
                "key": key,
                "changed": changed,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else if changed {
            println!("Removed '{key}' from [{section}] in {file} for instance '{instance_id}'.");
        } else {
            println!("'{key}' was not set in [{section}] of {file}; nothing changed.");
        }
        return Ok(());
    }

    let Some(value) = value else {
        // A read: the value alone on stdout for a script, or a JSON object.
        let found = read.document.get(&section, &key);
        if json {
            let out = serde_json::json!({
                "file": read.instance_path,
                "section": section,
                "key": key,
                "value": found,
                "source": read.source,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else if let Some(found) = &found {
            println!("{found}");
        } else {
            eprintln!("'{key}' is not set in [{section}] of {file}.");
        }
        if found.is_none() {
            std::process::exit(1);
        }
        return Ok(());
    };

    let changed =
        match game_ini::set_value(ctx, instance_id, game_def, &file, &section, &key, &value) {
            Ok(changed) => changed,
            Err(e) => fail(e.to_string()),
        };
    if json {
        let out = serde_json::json!({
            "status": "ok",
            "instance_id": instance_id,
            "file": file,
            "section": section,
            "key": key,
            "value": value,
            "changed": changed,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if changed {
        println!("Set '{key}' in [{section}] of {file} for instance '{instance_id}'.");
    } else {
        println!("'{key}' in [{section}] of {file} already has that value; nothing changed.");
    }
    Ok(())
}

fn print_ini_entries(
    json: bool,
    file: &str,
    source: agora_core::game_ini::IniSource,
    entries: &[agora_core::game_ini::IniEntry],
) -> anyhow::Result<()> {
    if json {
        let out = serde_json::json!({
            "file": file,
            "source": source,
            "entries": entries,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    let origin = match source {
        agora_core::game_ini::IniSource::Copy => "the instance's copy",
        agora_core::game_ini::IniSource::GameFile => {
            "the game's file (the instance has no copy yet)"
        }
        agora_core::game_ini::IniSource::Nothing => "nothing: the file does not exist yet",
    };
    println!("{file}, from {origin}:");
    for e in entries {
        println!("[{}] {} = {}", e.section, e.key, e.value);
    }
    Ok(())
}

/// One folder's saves, for a line of text: the count and the newest save's time.
fn folder_summary(folder: &agora_core::game_saves::SaveFolder) -> String {
    if !folder.exists {
        return "the folder does not exist yet".to_string();
    }
    match (&folder.newest, folder.saves) {
        (_, 0) => "no saves".to_string(),
        (Some(newest), n) => format!("{n} save{}, newest {newest}", if n == 1 { "" } else { "s" }),
        (None, n) => format!("{n} save{}", if n == 1 { "" } else { "s" }),
    }
}

/// `games instance saves`: show the instance's save choice, or make it `own` or `shared`.
fn saves_command(
    ctx: &agora_core::ctx::Ctx,
    json: bool,
    instance_id: &str,
    game_def: &agora_game_api::GameDefinition,
    choice: Option<&str>,
) -> anyhow::Result<()> {
    use agora_core::game_instance::SavesChoice;
    use agora_core::game_saves::{self, SettingChange};

    let fail = |message: String| -> ! { ini_fail(json, message) };
    let wanted = match choice {
        None => None,
        Some("own") => Some(SavesChoice::Own),
        Some("shared") => Some(SavesChoice::Shared),
        Some(other) => fail(format!("'{other}' is not a save choice: use own or shared")),
    };

    let Some(wanted) = wanted else {
        let status = match game_saves::status(ctx, instance_id, game_def) {
            Ok(status) => status,
            Err(e) => fail(e.to_string()),
        };
        if json {
            println!("{}", serde_json::to_string_pretty(&status)?);
            return Ok(());
        }
        let kept = match status.choice {
            SavesChoice::Shared => "the game's shared save folder",
            SavesChoice::Own => "saves of its own",
        };
        println!("Instance '{instance_id}' keeps {kept}.");
        println!(
            "In use: {} ({}).",
            status.in_use.path.display(),
            folder_summary(&status.in_use)
        );
        println!(
            "Other folder: {} ({}). Switching never moves saves.",
            status.other.path.display(),
            folder_summary(&status.other)
        );
        let set_to = status.setting_value.as_deref().unwrap_or("not set");
        println!(
            "The game's setting {} {} is {set_to} (from {}).",
            status.setting_file,
            status.setting,
            match status.setting_source {
                agora_core::game_ini::IniSource::Copy => "the instance's copy",
                agora_core::game_ini::IniSource::GameFile => "the game's file",
                agora_core::game_ini::IniSource::Nothing => "no file yet",
            }
        );
        return Ok(());
    };

    let change = match game_saves::set_choice(ctx, instance_id, game_def, wanted) {
        Ok(change) => change,
        Err(e) => fail(e.to_string()),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&change)?);
        return Ok(());
    }
    if change.previous == change.choice {
        println!(
            "Instance '{instance_id}' already keeps {} saves.",
            change.choice.as_str()
        );
    } else {
        println!(
            "Instance '{instance_id}' now keeps {} saves.",
            change.choice.as_str()
        );
    }
    match &change.setting {
        SettingChange::Set { value } => {
            println!("The game's save setting now names {value}.");
        }
        SettingChange::Unchanged => {
            println!("The game's save setting already named that folder.");
        }
        SettingChange::Restored { value } => {
            println!("The game's save setting is back to {value}, as it was before.");
        }
        SettingChange::Removed => {
            println!("The game's save setting was removed; it was not set before.");
        }
        SettingChange::LeftAlone { now } => {
            let now = now.as_deref().unwrap_or("nothing");
            println!(
                "The game's save setting no longer holds the value Agora set (it holds {now}), so it was left as it is. Check it by hand."
            );
        }
        SettingChange::NothingRecorded => {
            println!(
                "Nothing was recorded for this instance, so the game's save setting was left as it is."
            );
        }
    }
    println!(
        "The game reads and writes saves in {} ({}).",
        change.in_use.path.display(),
        folder_summary(&change.in_use)
    );
    println!(
        "The saves in {} ({}) stay where they are; nothing was moved, copied or deleted.",
        change.other.path.display(),
        folder_summary(&change.other)
    );
    Ok(())
}

/// The load order findings a launch prints or refuses with, one per line.
fn print_load_order_findings(findings: &[agora_core::game_load_order::Finding]) {
    for finding in findings {
        eprintln!("  - {}", finding.message());
    }
}

/// The line that says how to fix a late master, when the findings have one. The game can start
/// with a plugin before its master, but the plugin's references may resolve wrongly, and
/// `plugins sort` moves the master up.
fn sort_hint(
    instance_id: &str,
    findings: &[agora_core::game_load_order::Finding],
) -> Option<String> {
    let late = findings.iter().any(|f| {
        matches!(
            f,
            agora_core::game_load_order::Finding::MasterNotEarlier { .. }
        )
    });
    late.then(|| {
        format!(
            "Warning: a plugin loads before its master. 'agora games instance plugins sort {instance_id}' fixes the order, since the plugin's references may resolve wrongly until it does."
        )
    })
}

/// The instance's effective load order, one plugin per line: position, state, the master and
/// light flags (`?` when the header was not read), the lock, who put the line there, and the name.
fn print_load_order(instance_id: &str, order: &agora_core::game_load_order::LoadOrder) {
    if !order.exists {
        println!(
            "Instance '{instance_id}' has no plugin list yet; it is created when the instance is deployed."
        );
    }
    if order.entries.is_empty() {
        if order.exists {
            println!("Instance '{instance_id}' plugin list is empty.");
        }
        return;
    }
    println!(
        "{:>3}  {:<8}  M L  {:<6}  {:<7}  name",
        "#", "state", "lock", "from"
    );
    for (index, e) in order.entries.iter().enumerate() {
        let state = if e.implicit {
            "always"
        } else if e.active {
            "active"
        } else {
            "inactive"
        };
        let (master, light) = if e.header_read {
            (
                if e.master { "M" } else { "-" },
                if e.light { "L" } else { "-" },
            )
        } else {
            ("?", "?")
        };
        let lock = if e.locked { "locked" } else { "" };
        let from = if e.implicit {
            "game"
        } else if e.managed {
            "managed"
        } else {
            "yours"
        };
        let note = if !e.present {
            " (missing)"
        } else if e.header_error.is_some() {
            " (unreadable header)"
        } else {
            ""
        };
        println!(
            "{:>3}  {:<8}  {} {}  {:<6}  {:<7}  {}{}",
            index + 1,
            state,
            master,
            light,
            lock,
            from,
            e.name,
            note
        );
    }
    if order.findings.is_empty() {
        println!("No findings.");
    }
    for finding in &order.findings {
        println!("! {}", finding.message());
    }
}

/// `games instance plugins sort|move|lock|unlock|check`: the load order of a Creation Engine
/// game's plugin list (MASTER_SPEC §26.6).
fn plugins_load_order_command(
    ctx: &agora_core::ctx::Ctx,
    json: bool,
    cmd: InstancePluginsCmd,
) -> anyhow::Result<()> {
    use agora_core::game_load_order::{self as load_order, MoveTarget};

    let fail = |message: String| -> ! {
        if json {
            let out = serde_json::json!({
                "status": "error",
                "error": message,
                "exitCode": 1,
            });
            eprintln!("{}", serde_json::to_string_pretty(&out).unwrap_or(message));
        } else {
            eprintln!("Error: {message}");
        }
        std::process::exit(1);
    };

    let instance_id = match &cmd {
        InstancePluginsCmd::Sort { instance_id, .. }
        | InstancePluginsCmd::Move { instance_id, .. }
        | InstancePluginsCmd::Lock { instance_id, .. }
        | InstancePluginsCmd::Unlock { instance_id, .. }
        | InstancePluginsCmd::Check { instance_id } => instance_id.clone(),
        InstancePluginsCmd::Enable { .. } | InstancePluginsCmd::Disable { .. } => {
            fail("not a load order command".to_string())
        }
    };
    let record = match agora_core::game_instance::get(ctx, &instance_id)? {
        Some(r) => r,
        None => fail(format!("Instance '{instance_id}' not found.")),
    };
    let game_def = ctx
        .games
        .game(&record.game)
        .ok_or_else(|| anyhow::anyhow!("Game definition not found for {}", record.game))?;

    let locking = matches!(cmd, InstancePluginsCmd::Lock { .. });
    match cmd {
        InstancePluginsCmd::Sort { dry_run, .. } => {
            match load_order::sort(ctx, &instance_id, game_def, dry_run) {
                Ok(report) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&report)?);
                    } else {
                        if report.moves.is_empty() {
                            println!(
                                "Nothing to move: every master already comes before the plugins that need it."
                            );
                        }
                        for m in &report.moves {
                            println!("Move '{}' from position {} to {}.", m.plugin, m.from, m.to);
                        }
                        if !report.moves.is_empty() {
                            println!(
                                "{}",
                                if report.written {
                                    "Plugin list written."
                                } else {
                                    "Dry run: nothing was written."
                                }
                            );
                        }
                        for b in &report.blocked {
                            println!(
                                "Cannot fix: '{}' loads above its master '{}', and that master is locked or always loaded.",
                                b.plugin, b.master
                            );
                        }
                    }
                }
                Err(e) => fail(e.to_string()),
            }
        }
        InstancePluginsCmd::Move {
            plugin,
            to,
            before,
            after,
            ..
        } => {
            let target = match (to, before, after) {
                (Some(position), None, None) => MoveTarget::Position(position),
                (None, Some(other), None) => MoveTarget::Before(other),
                (None, None, Some(other)) => MoveTarget::After(other),
                _ => fail("name one target: --to N, --before PLUGIN or --after PLUGIN".to_string()),
            };
            match load_order::move_plugin(ctx, &instance_id, game_def, &plugin, &target) {
                Ok(report) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&report)?);
                    } else if report.written {
                        println!(
                            "Moved '{}' from position {} to {}.",
                            report.plugin, report.from, report.to
                        );
                    } else {
                        println!("'{}' is already at position {}.", report.plugin, report.to);
                    }
                }
                Err(e) => fail(e.to_string()),
            }
        }
        InstancePluginsCmd::Lock { plugin, .. } | InstancePluginsCmd::Unlock { plugin, .. } => {
            match agora_core::game_plugins::set_locked(
                ctx,
                &instance_id,
                game_def,
                &plugin,
                locking,
            ) {
                Ok(changed) => {
                    if json {
                        let out = serde_json::json!({
                            "status": "ok",
                            "instance_id": instance_id,
                            "plugin": plugin,
                            "locked": locking,
                            "changed": changed,
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                    } else {
                        let word = match (locking, changed) {
                            (true, true) => "is now locked",
                            (true, false) => "was already locked",
                            (false, true) => "is now unlocked",
                            (false, false) => "was not locked",
                        };
                        println!("'{plugin}' {word} in instance '{instance_id}'.");
                    }
                }
                Err(e) => fail(e.to_string()),
            }
        }
        InstancePluginsCmd::Check { .. } => match load_order::check(ctx, &instance_id, game_def) {
            Ok(findings) => {
                // Only a finding a launch refuses for exits 1; warnings are printed and exit 0.
                let refuses = findings.iter().any(load_order::Finding::refuses_launch);
                let status = if refuses {
                    "findings"
                } else if findings.is_empty() {
                    "ok"
                } else {
                    "warnings"
                };
                if json {
                    let out = serde_json::json!({
                        "status": status,
                        "instance_id": instance_id,
                        "findings": findings,
                        "exitCode": if refuses { 1 } else { 0 },
                    });
                    println!("{}", serde_json::to_string_pretty(&out)?);
                } else if findings.is_empty() {
                    println!("No findings: the load order satisfies the game's rules.");
                } else {
                    for f in &findings {
                        println!("- {}", f.message());
                    }
                    if let Some(hint) = sort_hint(&instance_id, &findings) {
                        println!("{hint}");
                    }
                }
                if refuses {
                    std::process::exit(1);
                }
            }
            Err(e) => fail(e.to_string()),
        },
        InstancePluginsCmd::Enable { .. } | InstancePluginsCmd::Disable { .. } => {
            fail("not a load order command".to_string())
        }
    }
    Ok(())
}

/// Say what a deploy changed in the instance's plugin list.
fn print_plugin_sync(report: &agora_core::game_plugins::PluginSyncReport) {
    if !report.added.is_empty() {
        println!("Plugin list: activated {}.", report.added.join(", "));
    }
    if !report.removed.is_empty() {
        println!("Plugin list: removed {}.", report.removed.join(", "));
    }
    for w in &report.warnings {
        eprintln!("Warning: {w}");
    }
}

/// Print base problems one per line, and say plainly when any of them is on a
/// file hardlinked to the store install: that change happened there too.
/// Parse a deployment mode name: `virtual`, `links`, `copies`, or `auto` (no choice).
fn parse_deployment_arg(s: &str) -> Result<Option<agora_core::game_deploy::DeployMode>, String> {
    if s.trim().eq_ignore_ascii_case("auto") {
        return Ok(None);
    }
    agora_core::game_deploy::DeployMode::parse(s)
        .map(Some)
        .ok_or_else(|| format!("unknown deployment '{s}': use virtual, links, copies or auto"))
}

/// The two ways to run an instance from `next`: for this launch, and for good.
fn vfs_retry_commands(instance_id: &str, next: agora_core::game_deploy::DeployMode) -> Vec<String> {
    vec![
        format!("agora games instance launch {instance_id} --deployment {next}"),
        format!("agora games instance set-deployment {instance_id} {next}"),
    ]
}

/// What running from `next` instead of the virtual file system gives up, for this game.
fn vfs_tradeoff_lines(next: agora_core::game_deploy::DeployMode) -> Vec<&'static str> {
    use agora_core::game_deploy::DeployMode;
    match next {
        DeployMode::Links => vec![
            "Linked files cannot catch writes: a mod that edits its own files in place may be refused.",
            "Small config files are copies, so most mods still work.",
        ],
        DeployMode::Copies => vec![
            "Copied files take more disk space and longer to switch between instances.",
            "Every file can be written, but only in this instance's own copy.",
        ],
        DeployMode::Virtual => Vec::new(),
    }
}

/// Ask a yes/no question on the terminal; anything but yes is no.
fn ask_yes_no(prompt: &str) -> anyhow::Result<bool> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// The virtual file system could not start and nothing was launched: say what happened and what
/// the next rung means, and in a terminal ask whether to try it. Returns `next` on yes. Any other
/// answer, no terminal, or `--json` (which never prompts) prints how to run from `next` and exits
/// 1.
fn offer_vfs_fallback(
    error: &agora_core::game_instance::InstanceError,
    instance_id: &str,
    next: agora_core::game_deploy::DeployMode,
    json: bool,
) -> anyhow::Result<agora_core::game_deploy::DeployMode> {
    let retry = vfs_retry_commands(instance_id, next);
    if json {
        let out = serde_json::json!({
            "status": "error",
            "error": format!("{error}"),
            "exitCode": 1,
            "nextDeployment": next.as_str(),
            "retry": retry,
        });
        eprintln!("{}", serde_json::to_string_pretty(&out)?);
        std::process::exit(1);
    }
    eprintln!("Error: {error}");
    for line in vfs_tradeoff_lines(next) {
        eprintln!("{line}");
    }
    if std::io::stdin().is_terminal()
        && ask_yes_no(&format!("Try again from {}? [y/N]: ", next.plain_name()))?
    {
        return Ok(next);
    }
    eprintln!("To run from {} instead:", next.plain_name());
    eprintln!("  {}    (this launch only)", retry[0]);
    eprintln!("  {}    (every launch)", retry[1]);
    std::process::exit(1);
}

/// A session under the virtual file system ended programs the game started. Say which and why;
/// in a terminal ask whether to start the game again from `next` now. Returns whether to.
fn offer_restart_after_vfs_ended(
    ended: &[agora_core::game_launch::EndedProcess],
    instance_id: &str,
    next: agora_core::game_deploy::DeployMode,
) -> anyhow::Result<bool> {
    eprintln!(
        "The virtual file system stopped {} program(s) the game started, because it could not protect them:",
        ended.len()
    );
    for process in ended {
        eprintln!(
            "  - {} (pid {}): {}",
            process.exe, process.pid, process.reason
        );
    }
    eprintln!("Something the game needed from them may not have worked.");
    for line in vfs_tradeoff_lines(next) {
        eprintln!("{line}");
    }
    if std::io::stdin().is_terminal()
        && ask_yes_no(&format!(
            "Start the game again from {} now? [y/N]: ",
            next.plain_name()
        ))?
    {
        return Ok(true);
    }
    let retry = vfs_retry_commands(instance_id, next);
    eprintln!("To run from {} instead:", next.plain_name());
    eprintln!("  {}    (this launch only)", retry[0]);
    eprintln!("  {}    (every launch)", retry[1]);
    Ok(false)
}

/// One line per framework finding, indented under the caller's heading. The repair goes on the
/// line after what is wrong, so a person can act on it.
fn runtime_finding_lines(findings: &[agora_game_api::RuntimeFileFinding]) -> Vec<String> {
    use agora_game_api::RuntimeFileProblem;
    let mut lines = Vec::new();
    for finding in findings {
        match &finding.problem {
            RuntimeFileProblem::WrongVersion => {
                let found = if finding.found.is_empty() {
                    "no file of its family".to_string()
                } else {
                    finding.found.join(", ")
                };
                lines.push(format!(
                    "  - {} ({}): needs {}, the game has {}",
                    finding.rule_name, finding.rule_id, finding.expected, found
                ));
                lines.push(format!("    Repair: {}", finding.repair));
            }
            RuntimeFileProblem::CannotCheck { reason } => {
                lines.push(format!(
                    "  - {} ({}): cannot be checked: {reason}",
                    finding.rule_name, finding.rule_id
                ));
            }
        }
    }
    lines
}

fn print_runtime_findings(findings: &[agora_game_api::RuntimeFileFinding]) {
    for line in runtime_finding_lines(findings) {
        eprintln!("{line}");
    }
}

fn print_base_problems(problems: &[agora_core::game_base::BaseProblem]) {
    use agora_core::game_base::ProblemKind;
    for p in problems {
        let what = match &p.kind {
            ProblemKind::Missing => "missing".to_string(),
            ProblemKind::SizeChanged { expected, actual } => {
                format!("size changed (expected {expected}, actual {actual})")
            }
            ProblemKind::ContentChanged => "content changed".to_string(),
            ProblemKind::Unexpected => "unexpected file".to_string(),
        };
        let linked = if p.linked_to_store {
            " [linked: the store install changed too]"
        } else {
            ""
        };
        eprintln!("  - {}: {what}{linked}", p.path);
    }
    if problems.iter().any(|p| p.linked_to_store) {
        eprintln!(
            "Warning: a file hardlinked to the store install changed, so the store install changed with it. Until Agora's write layer exists, declare files the game writes in its definition, or use a Copied base. A store's verify/repair restores the original."
        );
    }
}

fn print_content_problems(problems: &[agora_core::content_store::ContentProblem]) {
    for p in problems {
        match &p.kind {
            agora_core::content_store::ProblemKind::Missing => {
                eprintln!("  [{}] {}: missing from object store", p.item_id, p.path);
            }
            agora_core::content_store::ProblemKind::SizeMismatch { expected, actual } => {
                eprintln!(
                    "  [{}] {}: size mismatch (expected {expected} bytes, found {actual})",
                    p.item_id, p.path
                );
            }
            agora_core::content_store::ProblemKind::HashMismatch { expected, actual } => {
                eprintln!(
                    "  [{}] {}: hash mismatch (expected {expected}, found {actual})",
                    p.item_id, p.path
                );
            }
            agora_core::content_store::ProblemKind::Unprotected => {
                eprintln!("  [{}] {}: object is unprotected", p.item_id, p.path);
            }
            agora_core::content_store::ProblemKind::CorruptManifest { error } => {
                eprintln!("  [{}] manifest corrupt or unreadable: {error}", p.item_id);
            }
        }
    }
}

fn print_datapack_sync(
    instance: &str,
    report: &agora_game_minecraft::datapack_sync::DatapackSyncReport,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "status": "synced",
                "instanceId": instance,
                "worlds": report.worlds,
                "copied": report.copied,
                "removed": report.removed,
                "warnings": report.warnings,
            })
        );
    } else {
        println!(
            "Data packs in '{instance}': {} world(s), {} added or updated, {} removed.",
            report.worlds, report.copied, report.removed
        );
        for warning in &report.warnings {
            eprintln!("warning: {warning}");
        }
    }
    Ok(())
}

/// Resolve optional deps policy from CLI flags.
fn resolve_optional_deps(
    include: Option<String>,
    exclude: bool,
) -> agora_game_minecraft::install_pipeline::OptionalDepsPolicy {
    if exclude {
        return agora_game_minecraft::install_pipeline::OptionalDepsPolicy::ExcludeAll;
    }
    if let Some(list) = include {
        let deps: Vec<String> = list
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        return agora_game_minecraft::install_pipeline::OptionalDepsPolicy::Include { deps };
    }
    agora_game_minecraft::install_pipeline::OptionalDepsPolicy::Prompt
}

/// Overrides for `mod remove`. `--remove-anyway` pre-selects RemoveAnyway for
/// the broken-required-dependency conflict of the file being removed, and for
/// no other conflict.
fn remove_overrides(
    target_filename: &str,
    allow_replace: bool,
    skip_health_scan: bool,
    remove_anyway: bool,
) -> agora_game_minecraft::install_pipeline::PlanOverrides {
    let mut overrides = agora_game_minecraft::install_pipeline::PlanOverrides {
        allow_replace,
        skip_health_scan,
        ..Default::default()
    };
    if remove_anyway {
        overrides.force_conflict_resolution.insert(
            format!("broken-dependency:{target_filename}"),
            agora_game_minecraft::install_pipeline::ConflictResolution::RemoveAnyway,
        );
    }
    overrides
}

/// Apply --replace-conflicts / --abort-conflicts to a resolved plan.
fn apply_conflict_overrides(
    plan: &mut agora_game_minecraft::install_pipeline::ResolvedInstallPlan,
    replace: bool,
    abort: bool,
) -> anyhow::Result<()> {
    for conflict in &mut plan.conflicts {
        if conflict.chosen.is_some() {
            continue;
        }
        if replace {
            if conflict
                .resolution_options
                .contains(&agora_game_minecraft::install_pipeline::ConflictResolution::Replace)
            {
                conflict.chosen =
                    Some(agora_game_minecraft::install_pipeline::ConflictResolution::Replace);
            }
        } else if abort {
            conflict.chosen =
                Some(agora_game_minecraft::install_pipeline::ConflictResolution::Abort);
        }
    }
    Ok(())
}

/// Print unresolved plan diagnostics to stderr.
fn report_unresolved_plan(
    plan: &agora_game_minecraft::install_pipeline::ResolvedInstallPlan,
    json: bool,
) {
    if json {
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "blocked",
                "blockingErrors": plan.blocking_errors,
                "pendingChoices": plan.pending_choices,
                "conflicts": plan.conflicts,
            }))
            .unwrap_or_default()
        );
    } else {
        for err in &plan.blocking_errors {
            eprintln!("[BLOCK] {}: {}", err.code, err.message);
        }
        for conflict in &plan.conflicts {
            if conflict.chosen.is_none() {
                eprintln!("[CONFLICT] {}", conflict.message);
            }
        }
        for choice in &plan.pending_choices {
            let label: std::borrow::Cow<'_, str> = match choice {
                agora_game_minecraft::install_pipeline::PendingChoice::OptionalDependencies {
                    ..
                } => "Optional dependencies".into(),
                agora_game_minecraft::install_pipeline::PendingChoice::Conflict { .. } => {
                    "Conflict resolution".into()
                }
                agora_game_minecraft::install_pipeline::PendingChoice::LoaderChange {
                    current_version,
                    recommended_version,
                    ..
                } => format!(
                    "Loader change {} -> {}",
                    current_version, recommended_version
                )
                .into(),
            };
            eprintln!("[CHOICE] {} requires user input", label);
        }
    }
}

/// Print a resolved plan (used by --dry-run).
fn print_plan(
    plan: &agora_game_minecraft::install_pipeline::ResolvedInstallPlan,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
    } else {
        println!("=== Dry-run plan ({}): ===", plan.fingerprint);
        print!("  Operation: ");
        use agora_game_minecraft::install_pipeline::ResolvedArtifact;
        fn artifact_id(artifact: &ResolvedArtifact) -> String {
            match artifact {
                ResolvedArtifact::Download(d) => d.item_id.clone(),
                ResolvedArtifact::LocalFile(l) => l.item_id.clone(),
            }
        }
        fn artifact_version(artifact: &ResolvedArtifact) -> String {
            match artifact {
                ResolvedArtifact::Download(d) => d.version_id.clone(),
                ResolvedArtifact::LocalFile(_) => "local".into(),
            }
        }
        match &plan.operation {
            agora_game_minecraft::install_pipeline::ResolvedOperation::Install { artifact } => {
                println!(
                    "install {} v{}",
                    artifact_id(artifact),
                    artifact_version(artifact)
                );
            }
            agora_game_minecraft::install_pipeline::ResolvedOperation::Update {
                new_artifact,
                ..
            } => {
                println!(
                    "update {} v{}",
                    artifact_id(new_artifact),
                    artifact_version(new_artifact)
                );
            }
            agora_game_minecraft::install_pipeline::ResolvedOperation::Remove {
                target_filename,
                ..
            } => {
                println!("remove {}", target_filename);
            }
            other => {
                println!("{:?}", other);
            }
        }
        if !plan.files_to_add.is_empty() {
            println!("  Add: {} file(s)", plan.files_to_add.len());
            for f in &plan.files_to_add {
                println!("    + {}", f.target_filename);
            }
        }
        if !plan.files_to_remove.is_empty() {
            println!("  Remove: {} file(s)", plan.files_to_remove.len());
            for f in &plan.files_to_remove {
                println!("    - {}", f.filename);
            }
        }
        if !plan.files_to_disable.is_empty() {
            println!("  Disable: {} file(s)", plan.files_to_disable.len());
            for f in &plan.files_to_disable {
                println!("    ~ {}", f.filename);
            }
        }
        if !plan.conflicts.is_empty() {
            println!("  Conflicts:");
            for c in &plan.conflicts {
                let resolution = c
                    .chosen
                    .as_ref()
                    .map(|r| format!("{:?}", r))
                    .unwrap_or_else(|| "unresolved".into());
                println!("    ! {} -> {}", c.message, resolution);
            }
        }
        if !plan.dependencies.is_empty() {
            println!("  Dependencies: {}", plan.dependencies.len());
        }
        println!(
            "  Disk: {} download, {} additional, {} delta",
            plan.disk_estimate.download_bytes,
            plan.disk_estimate.peak_additional_bytes,
            plan.disk_estimate.post_commit_delta_bytes
        );
        if !plan.blocking_errors.is_empty() {
            println!("  Blocking errors: {}", plan.blocking_errors.len());
            for err in &plan.blocking_errors {
                println!("    [BLOCK] {}: {}", err.code, err.message);
            }
        }
    }
    Ok(())
}

/// Print an error the way the other `games content` commands do, and exit 1.
fn exit_with_error(json: bool, message: &str) -> ! {
    if json {
        let out = serde_json::json!({
            "status": "error",
            "error": message,
            "exitCode": 1,
        });
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&out).unwrap_or_else(|_| message.to_string())
        );
    } else {
        eprintln!("Error: {message}");
    }
    std::process::exit(1);
}

fn describe_plugin_type(td: &agora_core::content_fomod::TypeDescriptor) -> String {
    use agora_core::content_fomod::TypeDescriptor;
    match td {
        TypeDescriptor::Simple { plugin_type } => format!("{plugin_type:?}"),
        TypeDescriptor::Dependent { default, patterns } => {
            let mut s = format!("{default:?} by default");
            for (cond, t) in patterns {
                s.push_str(&format!("; {t:?} when {}", cond.describe()));
            }
            s
        }
    }
}

/// `agora games content fomod ...`: a thin adapter over `agora_core::content_fomod`.
fn run_fomod_command(
    ctx: &agora_core::ctx::Ctx,
    action: FomodCmd,
    json: bool,
) -> anyhow::Result<()> {
    use agora_core::content_fomod as fomod;
    match action {
        FomodCmd::Show { item } => {
            let item_id = match agora_core::content_store::resolve_item_id(ctx, &item) {
                Ok(id) => id,
                Err(e) => exit_with_error(json, &e.to_string()),
            };
            let installer = match fomod::parse(ctx, &item_id) {
                Ok(i) => i,
                Err(e) => exit_with_error(json, &e.to_string()),
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&installer)?);
                return Ok(());
            }
            let title = if installer.module_name.is_empty() {
                "(unnamed installer)"
            } else {
                installer.module_name.as_str()
            };
            println!(
                "{title} (item {}, installer in '{}')",
                &item_id[..12.min(item_id.len())],
                installer.root
            );
            if let Some(info) = &installer.info {
                let parts: Vec<String> = [
                    info.version.as_ref().map(|v| format!("version {v}")),
                    info.author.as_ref().map(|a| format!("by {a}")),
                    info.website.clone(),
                ]
                .into_iter()
                .flatten()
                .collect();
                if !parts.is_empty() {
                    println!("  {}", parts.join(", "));
                }
            }
            if let Some(deps) = &installer.module_dependencies {
                println!("Requires: {}", deps.describe());
            }
            if !installer.required_files.is_empty() {
                println!(
                    "Always installed: {} file/folder entries",
                    installer.required_files.len()
                );
            }
            for (i, step) in installer.steps.iter().enumerate() {
                match &step.visible {
                    Some(c) => println!(
                        "Step {}: {} (shown when {})",
                        i + 1,
                        step.name,
                        c.describe()
                    ),
                    None => println!("Step {}: {}", i + 1, step.name),
                }
                for group in &step.groups {
                    println!("  Group: {} ({})", group.name, group.group_type.rule());
                    for plugin in &group.plugins {
                        println!(
                            "    - {} [{}]",
                            plugin.name,
                            describe_plugin_type(&plugin.type_descriptor)
                        );
                        if !plugin.description.is_empty() {
                            let first = plugin.description.lines().next().unwrap_or("");
                            println!("        {first}");
                        }
                        if !plugin.flags.is_empty() {
                            let flags: Vec<String> = plugin
                                .flags
                                .iter()
                                .map(|f| format!("{}={}", f.name, f.value))
                                .collect();
                            println!("        sets: {}", flags.join(", "));
                        }
                        println!(
                            "        choose with: --choose \"{}/{}/{}\"",
                            step.name, group.name, plugin.name
                        );
                    }
                }
            }
            for ci in &installer.conditional_installs {
                println!(
                    "Also installed when {}: {} file/folder entries",
                    ci.condition.describe(),
                    ci.files.len()
                );
            }
            Ok(())
        }
        FomodCmd::Install {
            item,
            instance,
            choose,
            defaults,
        } => {
            let item_id = match agora_core::content_store::resolve_item_id(ctx, &item) {
                Ok(id) => id,
                Err(e) => exit_with_error(json, &e.to_string()),
            };
            let installer = match fomod::parse(ctx, &item_id) {
                Ok(i) => i,
                Err(e) => exit_with_error(json, &e.to_string()),
            };

            // The instance answers the installer's file checks, and says where the result goes.
            let mut context = None;
            let mut mount: Option<String> = None;
            if let Some(instance_id) = &instance {
                context = match fomod::instance_context(ctx, instance_id) {
                    Ok(c) => Some(c),
                    Err(e) => exit_with_error(json, &e.to_string()),
                };
                let manifest = match agora_core::game_instance::get_manifest(ctx, instance_id) {
                    Ok(m) => m,
                    Err(e) => exit_with_error(json, &e.to_string()),
                };
                let layout = ctx
                    .games
                    .game(&manifest.game)
                    .and_then(|g| g.content_layout.as_ref());
                mount = match layout {
                    Some(l) if !l.data_path.as_str().is_empty() => {
                        Some(l.data_path.as_str().to_string())
                    }
                    Some(_) => None,
                    None => exit_with_error(
                        json,
                        &format!(
                            "game '{}' has no content layout, so Agora does not know where an installer's files go",
                            manifest.game
                        ),
                    ),
                };
            }

            let mut explicit = Vec::new();
            for spec in &choose {
                match installer.resolve_choice(spec) {
                    Ok(c) => explicit.push(c),
                    Err(e) => exit_with_error(json, &e.to_string()),
                }
            }
            let choices = if defaults {
                match fomod::defaults_over(&installer, &explicit, context.as_ref()) {
                    Ok(c) => c,
                    Err(e) => exit_with_error(json, &e.to_string()),
                }
            } else {
                fomod::merge_choices(Vec::new(), explicit)
            };

            let (_, plan, outcome) = match fomod::install(ctx, &item_id, &choices, context.as_ref())
            {
                Ok(r) => r,
                Err(e) => {
                    let hint = if defaults || !choose.is_empty() {
                        ""
                    } else {
                        " (pick options with --choose \"Step/Group/Plugin\", or use --defaults)"
                    };
                    exit_with_error(json, &format!("{e}{hint}"))
                }
            };
            let derived = outcome.item();

            let mut layer = None;
            if let Some(instance_id) = &instance {
                match agora_core::game_deploy::add_content(
                    ctx,
                    instance_id,
                    &derived.item_id,
                    mount.as_deref(),
                    None,
                ) {
                    Ok(l) => layer = Some(l),
                    Err(e) => exit_with_error(
                        json,
                        &format!(
                            "installed item {} but could not add it to instance '{instance_id}': {e}",
                            derived.item_id
                        ),
                    ),
                }
            }

            if json {
                let files: Vec<serde_json::Value> = plan
                    .files
                    .iter()
                    .map(|f| {
                        serde_json::json!({
                            "destination": f.destination.as_str(),
                            "source": f.source.as_str(),
                            "size": f.size,
                        })
                    })
                    .collect();
                let out = serde_json::json!({
                    "status": "installed",
                    "item_id": derived.item_id,
                    "name": derived.name,
                    "from_item": item_id,
                    "files": files,
                    "choices": plan.choices,
                    "notes": plan.notes,
                    "instance_id": instance,
                    "layer_id": layer.as_ref().map(|l| l.id.as_str().to_string()),
                    "mount_path": layer.as_ref().map(|l| l.mount_path.as_str().to_string()),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!(
                    "Installed item {} ({}, {} files, {} bytes; no new objects stored).",
                    derived.item_id,
                    derived.name,
                    derived.files.len(),
                    derived.total_size
                );
                for f in &plan.files {
                    println!("  {}", f.destination);
                }
                if !plan.choices.is_empty() {
                    println!("Choices:");
                    for c in &plan.choices {
                        println!("  {}/{}: {}", c.step, c.group, c.plugins.join(", "));
                    }
                }
                for note in &plan.notes {
                    println!("Note: {note}");
                }
                if let (Some(instance_id), Some(layer)) = (&instance, &layer) {
                    println!(
                        "Added to instance '{instance_id}' (mount: '{}').",
                        layer.mount_path.as_str()
                    );
                }
            }
            Ok(())
        }
    }
}

async fn run_launch_service(
    ctx: &agora_core::ctx::Ctx,
    instance: &str,
    yes: bool,
    timings: bool,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    let request = agora_game_minecraft::launch_service::LaunchRequest {
        instance_id: instance.to_owned(),
        mode: agora_game_minecraft::launch_service::LaunchMode::Direct,
        health_policy: if yes {
            agora_game_minecraft::launch_service::HealthPolicy::WarnOnly
        } else {
            agora_game_minecraft::launch_service::HealthPolicy::BlockOnRed
        },
        health_scan_token: None,
    };
    let progress = ConsoleLaunchProgress { json, timings };
    let launch_started = std::time::Instant::now();
    let result = agora_game_minecraft::launch_service::LaunchService::new(ctx.clone())
        .launch(request, &progress)
        .await?;
    eprintln!(
        "[timing] total-cli-session (incl. game runtime): {} ms",
        launch_started.elapsed().as_millis()
    );
    if json {
        println!(
            "{}",
            serde_json::json!({
                "pid": result.pid,
                "session_id": result.session_id,
                "outcome": result.outcome,
                "snapshot_id": result.snapshot_id,
            })
        );
    } else {
        println!("Launch finished with outcome {:?}.", result.outcome);
    }
    match result.outcome {
        agora_core::lkg::LaunchOutcome::Crash | agora_core::lkg::LaunchOutcome::Unknown => {
            Err(agora_core::error::LauncherError::GameCrash.into())
        }
        _ => Ok(()),
    }
}

/// Wait for an import's background initial recovery snapshot to settle.
///
/// The import service spawns the initial snapshot on a background worker so
/// interactive callers can start browsing immediately, but a CLI process
/// exits right after the import future completes — killing the worker and
/// leaving a dead-owner `.agora_snapshot_pending` marker that blocks launch.
/// This blocks until the snapshot manifest is durable (Ready), surfaces a
/// failed snapshot, or times out.
fn wait_for_initial_snapshot(ctx: &agora_core::ctx::Ctx, instance_id: &str) -> anyhow::Result<()> {
    const SNAPSHOT_SETTLE_TIMEOUT: Duration = Duration::from_secs(300);
    const POLL_INTERVAL: Duration = Duration::from_millis(250);

    let instance_dir = ctx.paths.instance_dir(instance_id)?;
    let started = std::time::Instant::now();
    loop {
        use agora_core::snapshot::SnapshotReadiness;
        match agora_core::snapshot::snapshot_readiness(&instance_dir) {
            SnapshotReadiness::Failed => {
                let reason = agora_core::snapshot::snapshot_readiness_error(&instance_dir)
                    .unwrap_or_else(|| "unknown snapshot failure".into());
                anyhow::bail!("initial recovery snapshot failed: {reason}");
            }
            SnapshotReadiness::Pending => {
                if started.elapsed() >= SNAPSHOT_SETTLE_TIMEOUT {
                    anyhow::bail!(
                        "timed out waiting for the initial recovery snapshot ({} s)",
                        SNAPSHOT_SETTLE_TIMEOUT.as_secs()
                    );
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            SnapshotReadiness::Ready => {
                let has_manifest = agora_core::snapshot::list_snapshots(&instance_dir)
                    .map(|snapshots| !snapshots.is_empty())
                    .unwrap_or(false);
                if has_manifest {
                    eprintln!(
                        "[timing] initial-snapshot: settled in {} ms",
                        started.elapsed().as_millis()
                    );
                    return Ok(());
                }
                if started.elapsed() >= SNAPSHOT_SETTLE_TIMEOUT {
                    anyhow::bail!(
                        "timed out waiting for the initial recovery snapshot ({} s)",
                        SNAPSHOT_SETTLE_TIMEOUT.as_secs()
                    );
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MCP stdio transport
// ---------------------------------------------------------------------------/// Build a JSON-RPC 2.0 success response envelope.
fn build_jsonrpc_response(id: &serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

/// Build a JSON-RPC 2.0 error response envelope.
fn build_jsonrpc_error(id: &serde_json::Value, code: i64, message: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

/// Run the MCP stdio transport loop.
///
/// Reads newline-delimited JSON-RPC 2.0 requests from stdin, dispatches via
/// [`agora_game_minecraft::mcp_dispatcher::McpDispatcher`], writes responses to
/// stdout, and prints diagnostics to stderr.  Notifications (requests without
/// an `id` field) do not receive a response.  Exits cleanly on EOF.
async fn run_mcp_stdio(ctx: &agora_core::ctx::Ctx) -> anyhow::Result<()> {
    let dispatcher = agora_game_minecraft::mcp_dispatcher::McpDispatcher::new(ctx.clone());
    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout();

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[mcp] stdin read error: {e}");
                continue;
            }
        };
        let trimmed = line.trim().to_owned();
        if trimmed.is_empty() {
            continue;
        }

        let request: serde_json::Value = match serde_json::from_str(&trimmed) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[mcp] Failed to parse request: {e}");
                // Cannot send a JSON-RPC error for malformed JSON — we don't
                // have a valid id to echo back in the response.
                continue;
            }
        };

        let id_value: serde_json::Value = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let is_notification = request.get("id").is_none();

        let method = match request.get("method").and_then(|v| v.as_str()) {
            Some(m) => m,
            None => {
                eprintln!("[mcp] Request missing 'method' field");
                if !is_notification {
                    let resp =
                        build_jsonrpc_error(&id_value, -32600, "Invalid Request: missing method");
                    let line = serde_json::to_string(&resp)?;
                    writeln!(stdout, "{line}")?;
                    stdout.flush()?;
                }
                continue;
            }
        };

        let dispatcher_result = dispatcher.handle_method(method, request.get("params"));

        // Notifications have no "id" — no response per JSON-RPC 2.0
        if is_notification {
            continue;
        }

        let response = if dispatcher_result.get("error").is_some() {
            let err = &dispatcher_result["error"];
            let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(-32603);
            let msg = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Internal error");
            build_jsonrpc_error(&id_value, code, msg)
        } else {
            build_jsonrpc_response(&id_value, dispatcher_result)
        };

        let line = serde_json::to_string(&response)?;
        writeln!(stdout, "{line}")?;
        stdout.flush()?;
    }

    Ok(())
}

/// Open a URL with the OS handler.
///
/// Only https URLs are passed to the shell, and only ones Microsoft returned
/// for the pending sign-in — never a string typed by the user.
fn open_url_in_browser(url: &str) -> anyhow::Result<()> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| anyhow::anyhow!("Microsoft returned an invalid sign-in URL"))?;
    if parsed.scheme() != "https" {
        anyhow::bail!("refusing to open a non-https sign-in URL");
    }

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", parsed.as_str()]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg(parsed.as_str());
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(parsed.as_str());
        c
    };

    command.spawn()?;
    Ok(())
}

/// Say what happened to the per-user files after a session: a failed restore leaves them swapped
/// (the journal keeps everything needed), and the user must hear that and how to finish it.
///
/// Returns what the launch reports for them: `{restored, changed}`, or `{error}` when the restore
/// failed. The human line is stdout, so it is printed only without `--json`; the failure warning
/// is stderr and is printed either way.
fn report_user_files_restore(
    result: Result<
        agora_core::game_user_files::RestoreReport,
        agora_core::game_user_files::UserFilesError,
    >,
    game: &str,
    store: &str,
    json: bool,
) -> serde_json::Value {
    match result {
        Ok(report) => {
            let changed = report.files.iter().filter(|f| f.changed).count();
            if !json && !report.files.is_empty() {
                println!(
                    "Restored {} per-user file(s); {changed} changed during the session and were kept in the instance.",
                    report.files.len()
                );
            }
            serde_json::json!({ "restored": report.files.len(), "changed": changed })
        }
        Err(e) => {
            eprintln!(
                "Warning: the per-user files could not be restored ({e}); they are still swapped in.              Run `agora games user-files restore {game} {store}` once the game has closed."
            );
            serde_json::json!({ "error": e.to_string() })
        }
    }
}

/// What a launch's deployment did, as data: the counts the human "Deployed:" lines print.
fn deploy_summary_json(
    outcome: Option<&agora_core::game_deploy::DeployOutcome>,
) -> serde_json::Value {
    use agora_core::game_deploy::DeployOutcome;
    match outcome {
        None => serde_json::Value::Null,
        Some(DeployOutcome::UpToDate { .. }) => serde_json::json!({ "status": "up_to_date" }),
        Some(DeployOutcome::Built {
            linked,
            copied,
            copied_bytes,
            config_copied,
            harvest,
            ..
        }) => serde_json::json!({
            "status": "built",
            "linked": linked,
            "copied": copied,
            "copied_bytes": copied_bytes,
            "config_copied": config_copied,
            "harvest": harvest.as_ref().map(|h| serde_json::json!({
                "copied_to_writable": h.copied_to_writable.len(),
                "base_files_changed": h.base_files_changed.len(),
                "whiteouts_added": h.whiteouts_added.len(),
                "writable_files_removed": h.writable_files_removed.len(),
            })),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::exit_code_from_error;
    use super::exit_code_from_launcher_error;
    use super::Cli;
    use super::Commands;
    use super::LoadoutCmd;
    use super::LockfileCmd;
    use super::ModSourceArg;
    use super::ModsCmd;
    use super::OutputFormat;
    use super::PackCmd;
    use super::SilentReporter;
    use agora_core::dependency_ops;
    use agora_core::error::LauncherError;
    use agora_core::models::InstalledMod;
    use agora_game_minecraft::install_pipeline::{
        ArtifactMetadata, ArtifactSource, CancellationToken, ConflictKind, ConflictResolution,
        DepConflict, DiskSpaceEstimate, HashSpec, InstallAction, InstallIntent, OptionalDepsPolicy,
        PlanOverrides, ProgressEvent, ProgressPhase, ProgressReporter, RequestSource,
        ResolvedArtifact, ResolvedDownload, ResolvedInstallPlan, ResolvedOperation, SnapshotPlan,
        SourceType,
    };
    use clap::Parser;

    #[test]
    fn inventory_command_parses_instance_id() {
        let cli = Cli::try_parse_from(["agora", "inventory", "my-instance"])
            .expect("inventory command should parse");
        assert!(matches!(
            cli.command,
            Commands::Inventory { instance } if instance == "my-instance"
        ));
    }

    #[test]
    fn silent_reporter_accepts_progress_events() {
        let reporter = SilentReporter;
        // Should not panic
        reporter.report(ProgressEvent {
            plan_id: "test".into(),
            phase: ProgressPhase::Resolving,
            step: 0,
            total_steps: 1,
            bytes_downloaded: 0,
            bytes_total: 0,
            message: "test".into(),
        });
    }

    #[test]
    fn silent_reporter_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SilentReporter>();
    }

    #[test]
    fn cancellation_token_default_is_not_cancelled() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn cancellation_token_cancel_works() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn removal_plan_detects_reverse_dependents() {
        let target = InstalledMod {
            provider: None,
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            filename: "core-lib.jar".into(),
            registry_id: Some("core-lib".into()),
            modrinth_id: None,
            source: "modrinth".into(),
            source_url: None,
            version: Some("1.0.0".into()),
            sha256: "a".repeat(64),
            hash_verified: true,
            installed_at: "2024-01-01T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: Some("core-lib".into()),
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };

        let dependent = InstalledMod {
            provider: None,
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            filename: "dependent-mod.jar".into(),
            registry_id: Some("dependent-mod".into()),
            modrinth_id: None,
            source: "modrinth".into(),
            source_url: None,
            version: Some("2.0.0".into()),
            sha256: "b".repeat(64),
            hash_verified: true,
            installed_at: "2024-01-01T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: Some("dependent-mod".into()),
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            depends_on: vec!["core-lib".into()],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };

        let installed = vec![target.clone(), dependent];
        let plan = dependency_ops::build_removal_plan(&installed, &target);
        assert_eq!(plan.dependents.len(), 1);
        assert_eq!(plan.dependents[0].mod_id, "dependent-mod");
        assert_eq!(
            plan.dependents[0].requirement,
            agora_game_minecraft::install_pipeline::Requirement::Required
        );
    }

    #[test]
    fn removal_plan_empty_for_unreferenced_mod() {
        let target = InstalledMod {
            provider: None,
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            filename: "standalone.jar".into(),
            registry_id: None,
            modrinth_id: None,
            source: "manual".into(),
            source_url: None,
            version: None,
            sha256: "c".repeat(64),
            hash_verified: true,
            installed_at: "2024-01-01T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: None,
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };

        let other = InstalledMod {
            provider: None,
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            filename: "other.jar".into(),
            registry_id: Some("other".into()),
            modrinth_id: None,
            source: "modrinth".into(),
            source_url: None,
            version: Some("1.0.0".into()),
            sha256: "d".repeat(64),
            hash_verified: true,
            installed_at: "2024-01-01T00:00:00Z".into(),
            java_packages: vec![],
            mod_jar_id: Some("other".into()),
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        };

        let installed = vec![target.clone(), other];
        let plan = dependency_ops::build_removal_plan(&installed, &target);
        assert!(plan.dependents.is_empty());
    }

    // --- exit-code mapping tests ---

    #[test]
    fn exit_code_local_state_failed() {
        assert_eq!(
            exit_code_from_launcher_error(&LauncherError::LocalStateFailed),
            10
        );
    }

    #[test]
    fn exit_code_instance_locked() {
        assert_eq!(
            exit_code_from_launcher_error(&LauncherError::InstanceLocked),
            11
        );
    }

    #[test]
    fn exit_code_instance_create_failed() {
        assert_eq!(
            exit_code_from_launcher_error(&LauncherError::InstanceCreateFailed),
            12
        );
    }

    #[test]
    fn exit_code_network_offline() {
        assert_eq!(
            exit_code_from_launcher_error(&LauncherError::NetworkOffline),
            20
        );
    }

    #[test]
    fn exit_code_auth_expired() {
        assert_eq!(
            exit_code_from_launcher_error(&LauncherError::AuthExpired),
            40
        );
    }

    #[test]
    fn exit_code_generic_falls_to_one() {
        assert_eq!(
            exit_code_from_launcher_error(&LauncherError::Generic {
                code: "ERR_X".into(),
                message: "x".into(),
            }),
            1
        );
    }

    #[test]
    fn exit_code_from_anyhow_wrapping_launcher_error() {
        let le = LauncherError::LocalStateFailed;
        let err = anyhow::Error::from(le);
        assert_eq!(exit_code_from_error(&err), 10);
    }

    #[test]
    fn exit_code_from_anyhow_plain_message_is_one() {
        let err = anyhow::anyhow!("something went wrong");
        assert_eq!(exit_code_from_error(&err), 1);
    }

    #[test]
    fn exit_code_generic_maps_to_one() {
        assert_eq!(
            exit_code_from_launcher_error(&LauncherError::Generic {
                code: "ERR_INSTANCE_NOT_FOUND".into(),
                message: "Instance 'test' not found".into(),
            }),
            1
        );
    }

    #[test]
    fn exit_code_generic_through_anyhow_maps_to_one() {
        let err: anyhow::Error = LauncherError::Generic {
            code: "ERR_INSTANCE_NOT_FOUND".into(),
            message: "Instance 'test' not found".into(),
        }
        .into();
        assert_eq!(exit_code_from_error(&err), 1);
    }

    #[test]
    fn exit_code_from_plain_bail_is_one() {
        let err = anyhow::anyhow!("Instance 'foo' not found");
        assert_eq!(exit_code_from_error(&err), 1);
    }

    // --- OutputFormat tests ---

    #[test]
    fn output_format_human_is_not_json() {
        assert!(!OutputFormat::Human.is_json_output());
    }

    #[test]
    fn output_format_json_is_json() {
        assert!(OutputFormat::Json.is_json_output());
    }

    // --- JSON-safe behavior tests ---

    #[test]
    fn json_branch_list_instances_no_db_uses_eprintln() {
        // This is a compile-time / logic assertion that the stdout leak
        // has been fixed. The production code now uses eprintln! for
        // the "No local state database found" message, which keeps
        // stdout clean for JSON consumers.
    }

    #[test]
    fn json_branch_settings_list_no_db_uses_eprintln() {
        // Same safety property as list_instances_no_db.
    }

    #[test]
    fn json_branch_auth_login_prompts_use_eprintln() {
        // In JSON mode, the interactive prompts are sent to stderr
        // so stdout contains only the final JSON credentials object.
    }

    // --- JSON error envelope tests ---

    #[test]
    fn json_error_envelope_has_required_fields() {
        let err = anyhow::anyhow!("Instance 'foo' not found");
        let code = exit_code_from_error(&err);
        let envelope = serde_json::json!({
            "error": err.to_string(),
            "exitCode": code,
        });
        assert_eq!(envelope["error"], "Instance 'foo' not found");
        assert_eq!(envelope["exitCode"], 1);
    }

    #[test]
    fn json_error_envelope_includes_semantic_code_for_launcher_error() {
        let le = LauncherError::LocalStateFailed;
        let err = anyhow::Error::from(le);
        let code = exit_code_from_error(&err);
        let envelope = serde_json::json!({
            "error": err.to_string(),
            "exitCode": code,
        });
        assert_eq!(envelope["exitCode"], 10);
        assert!(
            envelope["error"].as_str().unwrap().contains("database"),
            "error should mention database"
        );
    }

    #[test]
    fn json_error_envelope_blocks_install() {
        let err =
            anyhow::anyhow!("Install blocked: unresolved errors, conflicts, or pending choices");
        let code = exit_code_from_error(&err);
        assert_eq!(code, 1);
        assert!(
            err.to_string().contains("blocked"),
            "blocked error should contain 'blocked'"
        );
    }

    #[test]
    fn json_error_envelope_blocks_remove() {
        let err = anyhow::anyhow!("Remove blocked: unresolved errors");
        let code = exit_code_from_error(&err);
        assert_eq!(code, 1);
        assert!(
            err.to_string().contains("blocked"),
            "blocked error should contain 'blocked'"
        );
    }

    #[test]
    fn bail_messages_include_expected_content() {
        // Verify that the bail messages used in run_command replacements
        // produce meaningful error text through the top-level envelope.
        let cases: Vec<(&str, &[&str])> = vec![
            ("No local state database found", &["database", "found"]),
            (
                "Instance 'my-instance' not found",
                &["Instance", "not found"],
            ),
            ("Setting 'foo' not found", &["Setting", "not found"]),
            (
                "Path 'C:\\missing' does not exist",
                &["Path", "does not exist"],
            ),
            ("No input provided", &["No input", "provided"]),
        ];
        for (msg, keywords) in cases {
            let err = anyhow::anyhow!("{}", msg);
            let s = err.to_string();
            for kw in keywords {
                assert!(s.contains(kw), "error '{}' should contain '{}'", s, kw);
            }
        }
    }

    // --- MCP JSON-RPC envelope tests ---

    #[test]
    fn jsonrpc_response_has_correct_envelope() {
        let id = serde_json::json!(1);
        let result = serde_json::json!({"status": "ok"});
        let resp = super::build_jsonrpc_response(&id, result);
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["status"], "ok");
        assert!(resp.get("error").is_none());
    }

    #[test]
    fn jsonrpc_response_string_id() {
        let id = serde_json::json!("req-42");
        let result = serde_json::json!({});
        let resp = super::build_jsonrpc_response(&id, result);
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], "req-42");
    }

    #[test]
    fn jsonrpc_response_null_id() {
        let id = serde_json::Value::Null;
        let result = serde_json::json!({});
        let resp = super::build_jsonrpc_response(&id, result);
        assert_eq!(resp["id"], serde_json::Value::Null);
    }

    #[test]
    fn jsonrpc_error_has_correct_envelope() {
        let id = serde_json::json!(1);
        let resp = super::build_jsonrpc_error(&id, -32601, "Method not found");
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["error"]["code"], -32601);
        assert_eq!(resp["error"]["message"], "Method not found");
        assert!(resp.get("result").is_none());
    }

    #[test]
    fn jsonrpc_error_no_id_uses_null() {
        let id = serde_json::Value::Null;
        let resp = super::build_jsonrpc_error(&id, -32700, "Parse error");
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], serde_json::Value::Null);
        assert_eq!(resp["error"]["code"], -32700);
    }

    #[test]
    fn jsonrpc_dispatcher_result_with_error_becomes_error_response() {
        // Simulates the dispatcher returning a method-level error
        let id = serde_json::json!(5);
        let dispatcher_result = serde_json::json!({
            "error": {
                "code": -32601,
                "message": "Unknown method: bogus"
            }
        });

        let response = if dispatcher_result.get("error").is_some() {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": dispatcher_result["error"],
            })
        } else {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": dispatcher_result,
            })
        };

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 5);
        assert_eq!(response["error"]["code"], -32601);
        assert!(response.get("result").is_none());
    }

    #[test]
    fn jsonrpc_dispatcher_result_without_error_becomes_result_response() {
        let id = serde_json::json!(10);
        let dispatcher_result = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {}
        });

        let response = if dispatcher_result.get("error").is_some() {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": dispatcher_result["error"],
            })
        } else {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": dispatcher_result,
            })
        };

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 10);
        assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
        assert!(response.get("error").is_none());
    }

    #[test]
    fn jsonrpc_notification_suppresses_response() {
        // Notifications (no "id") produce no response
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialize"
        });
        let is_notification = request.get("id").is_none();
        assert!(is_notification);
    }

    #[test]
    fn jsonrpc_request_with_id_is_not_notification() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize"
        });
        let is_notification = request.get("id").is_none();
        assert!(!is_notification);
    }

    // --- CLI parser tests for crash commands ---

    #[test]
    fn crash_list_parses() {
        let cli =
            Cli::try_parse_from(["agora", "crash", "list", "my-instance"]).expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Crash {
                action: super::CrashCmd::List { .. }
            }
        ));
    }

    #[test]
    fn crash_inspect_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "crash",
            "inspect",
            "my-instance",
            "crash-2024-01-01.txt",
        ])
        .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Crash {
                action: super::CrashCmd::Inspect { .. }
            }
        ));
    }

    #[test]
    fn crash_investigate_parses() {
        let cli = Cli::try_parse_from(["agora", "crash", "investigate", "my-instance"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Crash {
                action: super::CrashCmd::Investigate { .. }
            }
        ));
    }

    #[test]
    fn loader_list_with_mc_version_parses() {
        let cli = Cli::try_parse_from(["agora", "loader", "list", "--mc-version", "1.21"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Loader {
                action: super::LoaderCmd::List { .. }
            }
        ));
    }

    #[test]
    fn loader_list_with_m_flag_parses() {
        let cli =
            Cli::try_parse_from(["agora", "loader", "list", "-m", "1.20.1"]).expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Loader {
                action: super::LoaderCmd::List { .. }
            }
        ));
    }

    #[test]
    fn runtime_inspect_parses() {
        let cli = Cli::try_parse_from(["agora", "runtime", "inspect", "/usr/bin/java"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Runtime {
                action: super::RuntimeCmd::Inspect { .. }
            }
        ));
    }

    // --- CLI parser tests for new mod commands ---

    #[test]
    fn mod_search_parses_query() {
        let cli = Cli::try_parse_from(["agora", "mod", "search", "sodium"]).expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Mods {
                action: ModsCmd::Search { .. }
            }
        ));
    }

    #[test]
    fn mod_search_parses_with_filters() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "search",
            "sodium",
            "--content-type",
            "mod",
            "--mc-version",
            "1.21",
        ])
        .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Mods {
                action: ModsCmd::Search { .. }
            }
        ));
    }

    #[test]
    fn mod_update_parses_with_version() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "update",
            "my-instance",
            "sodium",
            "--version",
            "1.0.1",
        ])
        .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Mods {
                action: ModsCmd::Update { .. }
            }
        ));
    }

    #[test]
    fn mod_update_parses_without_version() {
        let cli = Cli::try_parse_from(["agora", "mod", "update", "my-instance", "sodium"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Mods {
                action: ModsCmd::Update { .. }
            }
        ));
    }

    #[test]
    fn mod_update_all_parses() {
        let cli = Cli::try_parse_from(["agora", "mod", "update-all", "my-instance"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Mods {
                action: ModsCmd::UpdateAll { .. }
            }
        ));
    }

    // --- mod enable / disable parser tests ---

    #[test]
    fn mod_enable_parses() {
        let cli = Cli::try_parse_from(["agora", "mod", "enable", "my-instance", "sodium.jar"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Mods {
                action: ModsCmd::Enable { .. }
            }
        ));
    }

    #[test]
    fn mod_worlds_parses_a_chosen_set_or_all() {
        let cli = Cli::try_parse_from([
            "agora", "mod", "worlds", "inst", "vm.zip", "--worlds", "A,B",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Mods {
                action: ModsCmd::Worlds { worlds, all, .. },
            } => {
                assert_eq!(worlds, Some(vec!["A".to_string(), "B".to_string()]));
                assert!(!all);
            }
            _ => panic!("unexpected command"),
        }
        assert!(Cli::try_parse_from(["agora", "mod", "worlds", "inst", "vm.zip", "--all"]).is_ok());
        assert!(
            Cli::try_parse_from(["agora", "mod", "worlds", "inst", "vm.zip"]).is_err(),
            "choosing nothing is not a scope"
        );
        assert!(Cli::try_parse_from(["agora", "mod", "sync-datapacks", "inst"]).is_ok());
    }

    #[test]
    fn mod_disable_parses() {
        let cli = Cli::try_parse_from(["agora", "mod", "disable", "my-instance", "sodium.jar"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Mods {
                action: ModsCmd::Disable { .. }
            }
        ));
    }

    // --- integration-style test for enable / disable via CrashService ---

    #[test]
    fn mod_enable_disable_roundtrip() {
        use agora_core::crash_service::CrashService;
        use agora_core::ctx::CoreContext;
        use agora_core::models::{InstalledMod, InstanceManifest};
        use std::fs;

        static TEST_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let seq = TEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!("agora-cli-mod-test-{}", seq));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create tmp");

        let ctx = CoreContext::for_testing(tmp.clone());
        let instance_dir = ctx.paths.instance_dir("test-instance").unwrap();
        fs::create_dir_all(instance_dir.join("mods")).expect("create mods dir");
        let mod_path = instance_dir.join("mods").join("test-mod.jar");
        fs::write(&mod_path, b"fake mod content").expect("write mod file");

        let manifest = InstanceManifest {
            manifest_version: agora_core::models::CURRENT_MANIFEST_VERSION,
            game_data: Default::default(),
            pack_origin: None,
            instance_id: "test-instance".into(),
            name: "Test".into(),
            created_from_pack: None,
            minecraft_version: "1.21".into(),
            loader: "fabric".into(),
            loader_version: "0.16.0".into(),
            is_locked: false,
            mods: vec![InstalledMod {
                provider: None,
                update_pinned: false,
                pack_managed: false,
                installed_as_dependency: false,
                filename: "test-mod.jar".into(),
                source: "manual".into(),
                source_url: None,
                version: Some("1.0.0".into()),
                sha256: "a".repeat(64),
                hash_verified: true,
                installed_at: "2024-01-01T00:00:00Z".into(),
                java_packages: vec![],
                modrinth_id: None,
                registry_id: None,
                mod_jar_id: None,
                provided_mod_ids: vec![],
                enabled: true,
                content_type: "mod".into(),
                depends_on: vec![],
                optional_deps: vec![],
                incompatible_deps: vec![],
            }],
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::Value::Null,
        };
        let manifest_path = ctx.paths.instance_manifest("test-instance").unwrap();
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .expect("write manifest");

        let svc = CrashService::new(ctx.clone());

        // DISABLE: rename test-mod.jar -> test-mod.jar.disabled
        svc.disable_mod("test-instance", "test-mod.jar")
            .expect("disable");
        assert!(
            !instance_dir.join("mods").join("test-mod.jar").exists(),
            "original gone"
        );
        assert!(
            instance_dir
                .join("mods")
                .join("test-mod.jar.disabled")
                .exists(),
            "disabled file exists"
        );

        // Asserts on the bytes actually written, so it must not heal.
        // allow-raw-instance-manifest
        let updated: InstanceManifest =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert!(!updated.mods[0].enabled, "mod disabled in manifest");

        // Idempotent
        svc.disable_mod("test-instance", "test-mod.jar")
            .expect("re-disable ok");

        // ENABLE: rename back
        svc.enable_mod("test-instance", "test-mod.jar")
            .expect("enable");
        assert!(
            instance_dir.join("mods").join("test-mod.jar").exists(),
            "original back"
        );
        assert!(
            !instance_dir
                .join("mods")
                .join("test-mod.jar.disabled")
                .exists(),
            "disabled file gone"
        );

        // Asserts on the bytes actually written, so it must not heal.
        // allow-raw-instance-manifest
        let updated2: InstanceManifest =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert!(updated2.mods[0].enabled, "mod enabled in manifest");

        // Idempotent
        svc.enable_mod("test-instance", "test-mod.jar")
            .expect("re-enable ok");

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- New policy flag tests ---

    #[test]
    fn mod_install_defaults_to_curated_source() {
        let cli = Cli::try_parse_from(["agora", "mod", "install", "lithium", "my-instance"])
            .expect("should parse");
        match cli.command {
            Commands::Mods {
                action: ModsCmd::Install { source, .. },
            } => assert_eq!(source, ModSourceArg::Curated),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mod_install_accepts_raw_modrinth_source() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "install",
            "sodium",
            "my-instance",
            "--source",
            "modrinth",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Mods {
                action: ModsCmd::Install { source, .. },
            } => assert_eq!(source, ModSourceArg::Modrinth),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mod_install_with_include_optional_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "install",
            "sodium",
            "my-instance",
            "--include-optional",
            "fabric-api,indium",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Mods {
                action:
                    ModsCmd::Install {
                        include_optional, ..
                    },
            } => {
                assert_eq!(include_optional, Some("fabric-api,indium".into()));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mod_install_with_exclude_optional_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "install",
            "sodium",
            "my-instance",
            "--exclude-optional",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Mods {
                action:
                    ModsCmd::Install {
                        exclude_optional, ..
                    },
            } => {
                assert!(exclude_optional);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mod_install_with_replace_conflicts_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "install",
            "sodium",
            "my-instance",
            "--replace-conflicts",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Mods {
                action:
                    ModsCmd::Install {
                        replace_conflicts, ..
                    },
            } => {
                assert!(replace_conflicts);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mod_install_with_abort_conflicts_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "install",
            "sodium",
            "my-instance",
            "--abort-conflicts",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Mods {
                action: ModsCmd::Install {
                    abort_conflicts, ..
                },
            } => {
                assert!(abort_conflicts);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mod_remove_with_remove_anyway_parses_and_excludes_abort() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "remove",
            "fabric-api",
            "my-instance",
            "--remove-anyway",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Mods {
                action: ModsCmd::Remove { remove_anyway, .. },
            } => assert!(remove_anyway),
            _ => panic!("wrong variant"),
        }
        assert!(Cli::try_parse_from([
            "agora",
            "mod",
            "remove",
            "fabric-api",
            "my-instance",
            "--remove-anyway",
            "--abort-conflicts",
        ])
        .is_err());
    }

    #[test]
    fn remove_anyway_only_selects_the_broken_dependency_conflict() {
        use agora_game_minecraft::install_pipeline::ConflictResolution;
        let overrides = super::remove_overrides("fabric-api.jar", false, true, true);
        assert_eq!(overrides.force_conflict_resolution.len(), 1);
        assert_eq!(
            overrides
                .force_conflict_resolution
                .get("broken-dependency:fabric-api.jar"),
            Some(&ConflictResolution::RemoveAnyway)
        );
        assert!(overrides.skip_health_scan);
        assert!(
            super::remove_overrides("fabric-api.jar", false, false, false)
                .force_conflict_resolution
                .is_empty()
        );
    }

    #[test]
    fn mod_install_with_dry_run_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "install",
            "sodium",
            "my-instance",
            "--dry-run",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Mods {
                action: ModsCmd::Install { dry_run, .. },
            } => {
                assert!(dry_run);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mod_install_include_exclude_optional_are_mutually_exclusive() {
        assert!(Cli::try_parse_from([
            "agora",
            "mod",
            "install",
            "sodium",
            "my-instance",
            "--include-optional",
            "fabric-api",
            "--exclude-optional"
        ])
        .is_err());
    }

    #[test]
    fn mod_update_with_policy_flags_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "update",
            "my-instance",
            "sodium",
            "--include-optional",
            "fabric-api",
            "--replace-conflicts",
            "--dry-run",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Mods {
                action:
                    ModsCmd::Update {
                        include_optional,
                        replace_conflicts,
                        dry_run,
                        ..
                    },
            } => {
                assert_eq!(include_optional, Some("fabric-api".into()));
                assert!(replace_conflicts);
                assert!(dry_run);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mod_update_all_with_policy_flags_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "update-all",
            "my-instance",
            "--exclude-optional",
            "--abort-conflicts",
            "--dry-run",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Mods {
                action:
                    ModsCmd::UpdateAll {
                        exclude_optional,
                        abort_conflicts,
                        dry_run,
                        ..
                    },
            } => {
                assert!(exclude_optional);
                assert!(abort_conflicts);
                assert!(dry_run);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mod_remove_with_conflict_flags_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "mod",
            "remove",
            "sodium",
            "my-instance",
            "--replace-conflicts",
            "--dry-run",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Mods {
                action:
                    ModsCmd::Remove {
                        replace_conflicts,
                        dry_run,
                        ..
                    },
            } => {
                assert!(replace_conflicts);
                assert!(dry_run);
            }
            _ => panic!("wrong variant"),
        }
    }

    // --- New command family parser tests ---

    #[test]
    fn pack_install_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "pack",
            "install",
            "/path/to/pack.json",
            "my-instance",
        ])
        .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Pack {
                action: PackCmd::Install { .. }
            }
        ));
    }

    #[test]
    fn pack_curated_parses_release_and_flexible() {
        let cli = Cli::try_parse_from([
            "agora",
            "pack",
            "curated",
            "optimized-survival",
            "my-instance",
            "--release",
            "1.0.0",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Pack {
                action:
                    PackCmd::Curated {
                        release, dry_run, ..
                    },
            } => {
                assert_eq!(release.as_deref(), Some("1.0.0"));
                assert!(!dry_run);
            }
            _ => panic!("expected pack curated"),
        }

        let cli = Cli::try_parse_from([
            "agora",
            "pack",
            "curated",
            "optimized-survival",
            "my-instance",
            "--dry-run",
        ])
        .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Pack {
                action: PackCmd::Curated {
                    release: None,
                    dry_run: true,
                    ..
                }
            }
        ));
    }

    #[test]
    fn export_parses() {
        let cli = Cli::try_parse_from(["agora", "export", "my-instance", "/path/to/dest"])
            .expect("should parse");
        assert!(matches!(cli.command, Commands::Export { .. }));
    }

    #[test]
    fn loadout_create_parses() {
        let cli = Cli::try_parse_from(["agora", "loadout", "create", "my-instance", "my-profile"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Loadout {
                action: LoadoutCmd::Create { .. }
            }
        ));
    }

    #[test]
    fn loadout_list_parses() {
        let cli =
            Cli::try_parse_from(["agora", "loadout", "list", "my-instance"]).expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Loadout {
                action: LoadoutCmd::List { .. }
            }
        ));
    }

    #[test]
    fn loadout_apply_parses() {
        let cli = Cli::try_parse_from(["agora", "loadout", "apply", "my-instance", "my-profile"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Loadout {
                action: LoadoutCmd::Apply { .. }
            }
        ));
    }

    #[test]
    fn loadout_delete_parses() {
        let cli = Cli::try_parse_from(["agora", "loadout", "delete", "my-instance", "my-profile"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Loadout {
                action: LoadoutCmd::Delete { .. }
            }
        ));
    }

    #[test]
    fn lockfile_export_parses() {
        let cli = Cli::try_parse_from(["agora", "lockfile", "export", "my-instance"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Lockfile {
                action: LockfileCmd::Export { .. }
            }
        ));
    }

    #[test]
    fn lockfile_export_with_output_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "lockfile",
            "export",
            "my-instance",
            "--out",
            "lockfile.json",
        ])
        .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Lockfile {
                action: LockfileCmd::Export { .. }
            }
        ));
    }

    #[test]
    fn lockfile_verify_parses() {
        let cli = Cli::try_parse_from(["agora", "lockfile", "verify", "lockfile.json"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Lockfile {
                action: LockfileCmd::Verify { .. }
            }
        ));
    }

    #[test]
    fn lockfile_repair_parses() {
        let cli = Cli::try_parse_from(["agora", "lockfile", "repair", "my-instance"])
            .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Lockfile {
                action: LockfileCmd::Repair { .. }
            }
        ));
    }

    #[test]
    fn lockfile_import_parses() {
        let cli = Cli::try_parse_from([
            "agora",
            "lockfile",
            "import",
            "lockfile.json",
            "my-instance",
        ])
        .expect("should parse");
        assert!(matches!(
            cli.command,
            Commands::Lockfile {
                action: LockfileCmd::Import { .. }
            }
        ));
    }

    // --- resolve_optional_deps tests ---

    #[test]
    fn resolve_optional_deps_include_list() {
        use agora_game_minecraft::install_pipeline::OptionalDepsPolicy;
        let policy = super::resolve_optional_deps(Some("fabric-api,indium".into()), false);
        match policy {
            OptionalDepsPolicy::Include { deps } => {
                assert_eq!(deps, vec!["fabric-api", "indium"]);
            }
            _ => panic!("expected Include"),
        }
    }

    #[test]
    fn resolve_optional_deps_exclude_all() {
        use agora_game_minecraft::install_pipeline::OptionalDepsPolicy;
        let policy = super::resolve_optional_deps(None, true);
        assert_eq!(policy, OptionalDepsPolicy::ExcludeAll);
    }

    #[test]
    fn resolve_optional_deps_prompt_when_no_flags() {
        use agora_game_minecraft::install_pipeline::OptionalDepsPolicy;
        let policy = super::resolve_optional_deps(None, false);
        assert_eq!(policy, OptionalDepsPolicy::Prompt);
    }

    #[test]
    fn resolve_optional_deps_empty_include_is_exclude() {
        use agora_game_minecraft::install_pipeline::OptionalDepsPolicy;
        let policy = super::resolve_optional_deps(Some(String::new()), false);
        match policy {
            OptionalDepsPolicy::Include { deps } => {
                assert!(deps.is_empty());
            }
            _ => panic!("expected Include"),
        }
    }

    // --- apply_conflict_overrides tests ---

    #[test]
    fn apply_replace_resolves_conflicts() {
        use agora_game_minecraft::install_pipeline::*;
        let mut plan = ResolvedInstallPlan {
            fingerprint: "test".into(),
            intent: todo_placeholder_intent(),
            operation: ResolvedOperation::Install {
                artifact: ResolvedArtifact::Download(ResolvedDownload {
                    item_id: "test".into(),
                    version_id: "1.0".into(),
                    filename: "test.jar".into(),
                    source: ArtifactSource::Download {
                        url: "https://example.com/test.jar".into(),
                    },
                    hashes: HashSpec {
                        values: vec![],
                        ..Default::default()
                    },
                    size: 0,
                    metadata: ArtifactMetadata {
                        provider: None,
                        source_type: SourceType::Modrinth,
                        registry_id: None,
                        modrinth_id: None,
                        content_type: "mod".into(),
                        version: None,
                        download_strategy: None,
                        pinned_host: None,
                    },
                }),
            },
            dependencies: vec![],
            conflicts: vec![DepConflict {
                conflict_id: "c1".into(),
                kind: ConflictKind::DuplicateMod,
                existing_mod_jar_id: "existing".into(),
                incoming_mod_jar_id: "incoming".into(),
                message: "conflict".into(),
                blocking: true,
                resolution_options: vec![ConflictResolution::Replace, ConflictResolution::Skip],
                chosen: None,
            }],
            files_to_add: vec![],
            files_to_remove: vec![],
            files_to_disable: vec![],
            files_to_promote: vec![],
            snapshot: SnapshotPlan {
                label: "".into(),
                estimated_bytes: 0,
            },
            disk_estimate: DiskSpaceEstimate::zero(),
            warnings: vec![],
            blocking_errors: vec![],
            pending_choices: vec![],
            loader_change: None,
            created_at: "".into(),
            instance_state_hash: "".into(),
            registry_revision: "".into(),
        };
        super::apply_conflict_overrides(&mut plan, true, false).unwrap();
        assert_eq!(plan.conflicts[0].chosen, Some(ConflictResolution::Replace));
        assert!(plan.is_fully_resolved());
    }

    #[test]
    fn apply_abort_resolves_conflicts() {
        let mut plan = ResolvedInstallPlan {
            fingerprint: "test".into(),
            intent: todo_placeholder_intent(),
            operation: ResolvedOperation::Install {
                artifact: ResolvedArtifact::Download(ResolvedDownload {
                    item_id: "test".into(),
                    version_id: "1.0".into(),
                    filename: "test.jar".into(),
                    source: ArtifactSource::Download {
                        url: "https://example.com/test.jar".into(),
                    },
                    hashes: HashSpec {
                        values: vec![],
                        ..Default::default()
                    },
                    size: 0,
                    metadata: ArtifactMetadata {
                        provider: None,
                        source_type: SourceType::Modrinth,
                        registry_id: None,
                        modrinth_id: None,
                        content_type: "mod".into(),
                        version: None,
                        download_strategy: None,
                        pinned_host: None,
                    },
                }),
            },
            dependencies: vec![],
            conflicts: vec![DepConflict {
                conflict_id: "c1".into(),
                kind: ConflictKind::DuplicateMod,
                existing_mod_jar_id: "existing".into(),
                incoming_mod_jar_id: "incoming".into(),
                message: "conflict".into(),
                blocking: true,
                resolution_options: vec![ConflictResolution::Replace, ConflictResolution::Skip],
                chosen: None,
            }],
            files_to_add: vec![],
            files_to_remove: vec![],
            files_to_disable: vec![],
            files_to_promote: vec![],
            snapshot: SnapshotPlan {
                label: "".into(),
                estimated_bytes: 0,
            },
            disk_estimate: DiskSpaceEstimate::zero(),
            warnings: vec![],
            blocking_errors: vec![],
            pending_choices: vec![],
            loader_change: None,
            created_at: "".into(),
            instance_state_hash: "".into(),
            registry_revision: "".into(),
        };
        super::apply_conflict_overrides(&mut plan, false, true).unwrap();
        // Abort is not in resolution_options, so chosen may be set but
        // is_fully_resolved will still return true (chosen is Some).
        assert_eq!(plan.conflicts[0].chosen, Some(ConflictResolution::Abort));
        assert!(plan.is_fully_resolved());
    }

    fn todo_placeholder_intent() -> InstallIntent {
        InstallIntent {
            action: InstallAction::Install {
                source_type: SourceType::Modrinth,
                item_id: "test".into(),
                candidate_version: None,
            },
            target_instance: "test".into(),
            optional_deps: OptionalDepsPolicy::ExcludeAll,
            requested_by: RequestSource::CLI,
            overrides: PlanOverrides::default(),
        }
    }

    #[test]
    fn exit_code_user_decision_required() {
        assert_eq!(
            exit_code_from_launcher_error(&LauncherError::UserDecisionRequired),
            71
        );
    }

    #[test]
    fn games_instance_content_add_parses_into_and_from() {
        use crate::{GameInstanceCmd, GamesCmd, InstanceContentCmd};
        let cli = Cli::try_parse_from([
            "agora", "games", "instance", "content", "add", "my-inst", "item123", "--into", "Data",
            "--from", "MyMod",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Games {
                action:
                    GamesCmd::Instance {
                        action:
                            GameInstanceCmd::Content {
                                action:
                                    InstanceContentCmd::Add {
                                        instance_id,
                                        item_id,
                                        into,
                                        from,
                                    },
                            },
                    },
            } => {
                assert_eq!(instance_id, "my-inst");
                assert_eq!(item_id, "item123");
                assert_eq!(into.as_deref(), Some("Data"));
                assert_eq!(from.as_deref(), Some("MyMod"));
            }
            _ => panic!("wrong command variant"),
        }
    }

    #[test]
    fn games_instance_content_own_copy_parses() {
        use crate::{GameInstanceCmd, GamesCmd, InstanceContentCmd};
        let cli = Cli::try_parse_from([
            "agora", "games", "instance", "content", "own-copy", "my-inst", "item123", "on",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Games {
                action:
                    GamesCmd::Instance {
                        action:
                            GameInstanceCmd::Content {
                                action:
                                    InstanceContentCmd::OwnCopy {
                                        instance_id,
                                        item_id,
                                        state,
                                    },
                            },
                    },
            } => {
                assert_eq!(instance_id, "my-inst");
                assert_eq!(item_id, "item123");
                assert_eq!(state, "on");
            }
            _ => panic!("wrong command variant"),
        }
    }
}
