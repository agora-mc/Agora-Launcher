//! Installing a curated catalog entry into an instance (MASTER_SPEC §26.8). The network is a fake
//! transport: no test reaches GitHub or any other host.

use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agora_core::artifact_hash::HashOrigin;
use agora_core::catalog_install::{
    install, CatalogPlan, CatalogTransport, InstallReport, InstallRequest, InstallState,
};
use agora_core::content_store::{self, ContentSource};
use agora_core::ctx::Ctx;
use agora_core::download::sha256_hex;
use agora_core::error::{LauncherError, LauncherResult};
use agora_core::game_base::BaseMode;
use agora_core::game_deploy::{self, PlacementDecision};
use agora_core::game_discovery::{DiscoveredInstall, InstallCapabilities};
use agora_core::game_instance::{self, get_manifest, GameInstanceRecord};
use agora_core::game_registry::{
    GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_core::github_release::{GitHubRelease, GitHubReleaseAsset};
use agora_core::registry::{
    DownloadSource, FrameworkRequirement, GameCatalogItem, GameCompatibility, SourcePin,
};
use agora_game_api::{
    ContentLayout, DeploymentStrategy, FrameworkDefinition, FrameworkDetection, FrameworkId,
    FrameworkVersionSource, GameDefinition, GameId, GamePackage, GamePath, InstallId, InstallKind,
    LaunchRecipe, LaunchValue, LayerSource, PackageDefinition, RelPath, RuntimeIdentity, StoreId,
    StoreIdentifier,
};
use async_trait::async_trait;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fake network
// ---------------------------------------------------------------------------

/// Releases by page (page 1 first), and the bytes behind each download URL.
struct FakeTransport {
    pages: Vec<Vec<GitHubRelease>>,
    files: HashMap<String, Vec<u8>>,
    fetches: Mutex<Vec<String>>,
}

impl FakeTransport {
    fn new(pages: Vec<Vec<GitHubRelease>>, files: Vec<(String, Vec<u8>)>) -> Self {
        Self {
            pages,
            files: files.into_iter().collect(),
            fetches: Mutex::new(Vec::new()),
        }
    }

    /// A transport that serves no releases and no files: for direct downloads that must not run.
    fn silent() -> Self {
        Self::new(Vec::new(), Vec::new())
    }

    fn fetch_count(&self) -> usize {
        self.fetches.lock().unwrap().len()
    }
}

#[async_trait]
impl CatalogTransport for FakeTransport {
    async fn releases_page(
        &self,
        _repo: &str,
        page: u32,
    ) -> LauncherResult<(Vec<GitHubRelease>, u32)> {
        let total = self.pages.len().max(1) as u32;
        let releases = self
            .pages
            .get(page as usize - 1)
            .cloned()
            .unwrap_or_default();
        Ok((releases, total))
    }

    async fn fetch(&self, url: &str, _pinned_host: Option<&str>) -> LauncherResult<Vec<u8>> {
        self.fetches.lock().unwrap().push(url.to_string());
        self.files
            .get(url)
            .cloned()
            .ok_or_else(|| LauncherError::Generic {
                code: "TEST_NO_FILE".into(),
                message: format!("no fake file for {url}"),
            })
    }
}

const REPO: &str = "example-author/crash-logger";

fn asset_url(tag: &str, name: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{tag}/{name}")
}

/// How a fake asset's digest is published.
#[derive(Clone, Copy)]
enum Digest {
    /// GitHub's digest for the bytes.
    Matching,
    /// No digest at all.
    Absent,
    /// A digest that does not describe these bytes.
    Other,
}

struct Asset<'a> {
    name: &'a str,
    bytes: &'a [u8],
    digest: Digest,
}

fn release(
    tag: &str,
    published_at: &str,
    draft: bool,
    prerelease: bool,
    assets: &[Asset],
) -> GitHubRelease {
    GitHubRelease {
        tag_name: tag.to_string(),
        published_at: Some(published_at.to_string()),
        draft,
        prerelease,
        assets: assets
            .iter()
            .map(|asset| GitHubReleaseAsset {
                name: asset.name.to_string(),
                browser_download_url: asset_url(tag, asset.name),
                size: Some(asset.bytes.len() as u64),
                digest: match asset.digest {
                    Digest::Matching => Some(format!("sha256:{}", sha256_hex(asset.bytes))),
                    Digest::Absent => None,
                    Digest::Other => Some(format!("sha256:{}", "e".repeat(64))),
                },
            })
            .collect(),
    }
}

/// The downloadable bytes for every asset of `releases`, keyed by URL.
fn files_of(releases: &[(&str, &[Asset])]) -> Vec<(String, Vec<u8>)> {
    releases
        .iter()
        .flat_map(|(tag, assets)| {
            assets
                .iter()
                .map(|asset| (asset_url(tag, asset.name), asset.bytes.to_vec()))
                .collect::<Vec<_>>()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Archives and PE files
// ---------------------------------------------------------------------------

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in entries {
        writer
            .start_file(*name, zip::write::FileOptions::default())
            .unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// A minimal PE32 file whose version resource says `components`, for the framework version reader.
fn pe_with_version(components: [u16; 4]) -> Vec<u8> {
    let mut file = vec![0u8; 0x400];
    let put_u16 =
        |file: &mut Vec<u8>, at: usize, v: u16| file[at..at + 2].copy_from_slice(&v.to_le_bytes());
    let put_u32 =
        |file: &mut Vec<u8>, at: usize, v: u32| file[at..at + 4].copy_from_slice(&v.to_le_bytes());
    file[0..2].copy_from_slice(b"MZ");
    put_u32(&mut file, 0x3C, 0x40);
    file[0x40..0x44].copy_from_slice(b"PE\0\0");
    put_u16(&mut file, 0x46, 1);
    put_u16(&mut file, 0x44 + 16, 224);
    let optional = 0x58;
    put_u16(&mut file, optional, 0x010B);
    put_u32(&mut file, optional + 96 + 16, 0x1000);
    put_u32(&mut file, optional + 96 + 20, 0x200);
    let section = optional + 224;
    file[section..section + 6].copy_from_slice(b".rsrc\0");
    put_u32(&mut file, section + 8, 0x200);
    put_u32(&mut file, section + 12, 0x1000);
    put_u32(&mut file, section + 16, 0x200);
    put_u32(&mut file, section + 20, 0x200);
    let rsrc = 0x200;
    put_u16(&mut file, rsrc + 14, 1);
    put_u32(&mut file, rsrc + 16, 16);
    put_u32(&mut file, rsrc + 20, 0x8000_0000 | 0x18);
    put_u16(&mut file, rsrc + 0x18 + 14, 1);
    put_u32(&mut file, rsrc + 0x18 + 16, 1);
    put_u32(&mut file, rsrc + 0x18 + 20, 0x8000_0000 | 0x28);
    put_u16(&mut file, rsrc + 0x28 + 14, 1);
    put_u32(&mut file, rsrc + 0x28 + 16, 0x409);
    put_u32(&mut file, rsrc + 0x28 + 20, 0x48);
    put_u32(&mut file, rsrc + 0x48, 0x1060);
    put_u32(&mut file, rsrc + 0x48 + 4, 188);
    let fixed = rsrc + 0x60 + 136;
    put_u32(&mut file, fixed, 0xFEEF_04BD);
    put_u32(&mut file, fixed + 4, 0x0001_0000);
    put_u32(
        &mut file,
        fixed + 8,
        ((components[0] as u32) << 16) | components[1] as u32,
    );
    put_u32(
        &mut file,
        fixed + 12,
        ((components[2] as u32) << 16) | components[3] as u32,
    );
    file
}

// ---------------------------------------------------------------------------
// Game, package and instance
// ---------------------------------------------------------------------------

struct TestPackage(PackageDefinition);

impl GamePackage for TestPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

fn test_definition() -> GameDefinition {
    GameDefinition {
        id: GameId::new("test-game").unwrap(),
        name: "Test Game".into(),
        stores: vec![StoreIdentifier {
            store: StoreId::new("steam").unwrap(),
            product: "12345".into(),
        }],
        version_sources: vec![],
        deployment: DeploymentStrategy::VirtualFileSystem,
        content_rules: vec![],
        native_code_patterns: vec![],
        framework_ids: vec![FrameworkId::new("skse").unwrap()],
        tool_ids: vec![],
        launch: Some(LaunchRecipe {
            executable: GamePath::Runtime {
                path: RelPath::new("Game.exe").unwrap(),
            },
            arguments: vec![LaunchValue::Literal {
                value: "-test".into(),
            }],
            environment: Default::default(),
            working_directory: GamePath::Runtime {
                path: RelPath::default(),
            },
        }),
        log_paths: vec![],
        crash_paths: vec![],
        user_files: vec![],
        save_paths: vec![],
        linked_archive_patterns: vec![],
        declared_writes: vec![],
        excluded_paths: vec![],
        plugin_list: None,
        runtime_files: Vec::new(),
        save_location: Vec::new(),
        launch_alternatives: Vec::new(),
        content_layout: Some(ContentLayout {
            data_path: RelPath::new("Data").unwrap(),
            data_markers: vec!["*.esp".into(), "*.bsa".into()],
            root_markers: vec![],
            thunderstore_bepinex: false,
        }),
        copy_patterns: Vec::new(),
    }
}

fn skse(game: &GameId) -> FrameworkDefinition {
    FrameworkDefinition {
        id: FrameworkId::new("skse").unwrap(),
        game: game.clone(),
        name: "Script Extender".into(),
        version: "2.2.6".into(),
        supported_runtimes: vec![],
        required_frameworks: vec![],
        content: vec![],
        launch: None,
        detection: Some(FrameworkDetection {
            marker: RelPath::new("skse64_loader.exe").unwrap(),
            version: Some(FrameworkVersionSource::PeFileVersion),
        }),
    }
}

fn register(def: GameDefinition, frameworks: Vec<FrameworkDefinition>) -> Arc<GameRegistry> {
    let mut builder = GameRegistry::builder();
    let package = PackageDefinition {
        id: "test.catalog".into(),
        version: semver::Version::new(0, 1, 0),
        api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
        parents: vec![],
        games: vec![def],
        frameworks,
        tools: vec![],
    };
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "test".into(),
            },
            Arc::new(TestPackage(package)),
        )
        .expect("register the test package");
    Arc::new(builder.build())
}

/// A game install on disk, with a base file set and optionally a framework loader.
struct Fixture {
    _tmp: TempDir,
    ctx: Ctx,
    install_dir: PathBuf,
    definition: GameDefinition,
}

/// `loader` is the bytes of `skse64_loader.exe` in the install, or `None` for no framework.
fn fixture(loader: Option<Vec<u8>>, frameworks: bool) -> Fixture {
    let tmp = TempDir::new().unwrap();
    let definition = test_definition();
    let declared = if frameworks {
        vec![skse(&definition.id)]
    } else {
        vec![]
    };
    let registry = register(definition.clone(), declared);
    let ctx = Ctx::for_testing(tmp.path().join("app_data")).with_games(registry);
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(install_dir.join("Data")).unwrap();
    std::fs::write(install_dir.join("Game.exe"), b"fake game binary").unwrap();
    std::fs::write(install_dir.join("Data").join("Skyrim.bsa"), b"BSA DATA").unwrap();
    if let Some(bytes) = loader {
        std::fs::write(install_dir.join("skse64_loader.exe"), bytes).unwrap();
    }
    Fixture {
        _tmp: tmp,
        ctx,
        install_dir,
        definition,
    }
}

impl Fixture {
    fn identified_install(&self) -> IdentifiedInstall {
        let store = StoreId::new("steam").unwrap();
        let runtime = RuntimeIdentity {
            game: GameId::new("test-game").unwrap(),
            store: store.clone(),
            version: "1.0.0".into(),
            build: None,
        };
        let detector = agora_core::game_discovery::volume::VolumeDetector::new();
        let discovered = DiscoveredInstall {
            store,
            product: "12345".into(),
            name: "Test Game".into(),
            kind: InstallKind::BaseGame,
            parent_product: None,
            location: self.install_dir.clone(),
            store_version: Some(runtime.version.clone()),
            store_build: None,
            executables: vec!["Game.exe".into()],
            capabilities: InstallCapabilities {
                executables_readable: true,
                accepts_new_files: true,
                relocatable: true,
            },
            volume: detector.get_volume_info(&self.install_dir),
        };
        IdentifiedInstall {
            game: runtime.game.clone(),
            install_id: InstallId::new("steam:12345").unwrap(),
            discovered,
            add_ons: vec![],
            runtime: RuntimeResolution::Identified {
                runtime,
                source: "executable".into(),
            },
        }
    }

    /// A new pinned instance of the test game, named `name`.
    fn instance(&self, name: &str) -> GameInstanceRecord {
        game_instance::create(
            &self.ctx,
            &self.identified_install(),
            &self.definition,
            name,
            None,
            BaseMode::Linked,
            &|_| {},
        )
        .expect("create instance")
    }

    fn content_items(&self) -> Vec<agora_core::content_store::ContentItem> {
        content_store::list_items(&self.ctx).unwrap()
    }

    fn layer_count(&self, instance_id: &str) -> usize {
        get_manifest(&self.ctx, instance_id)
            .unwrap()
            .layers
            .layers()
            .iter()
            .filter(|layer| matches!(layer.source, LayerSource::Content { .. }))
            .count()
    }
}

// ---------------------------------------------------------------------------
// Catalog entries
// ---------------------------------------------------------------------------

fn github_source(pins: Vec<SourcePin>) -> DownloadSource {
    DownloadSource {
        strategy: "github_release".into(),
        identifier: REPO.into(),
        pins,
    }
}

fn compat(
    stores: &[&str],
    versions: &[&str],
    asset: Option<&str>,
    requires: Vec<FrameworkRequirement>,
) -> GameCompatibility {
    GameCompatibility {
        stores: stores.iter().map(|s| s.to_string()).collect(),
        game_versions: versions.iter().map(|v| v.to_string()).collect(),
        requires,
        asset: asset.map(str::to_string),
    }
}

fn skse_requirement(min: Option<&str>) -> Vec<FrameworkRequirement> {
    vec![FrameworkRequirement {
        framework: "skse".into(),
        min_version: min.map(str::to_string),
    }]
}

fn entry(
    id: &str,
    game: &str,
    sources: Vec<DownloadSource>,
    compats: Vec<GameCompatibility>,
) -> GameCatalogItem {
    GameCatalogItem {
        id: id.into(),
        game: game.into(),
        name: "CrashLogger".into(),
        author: Some("example-author".into()),
        content_type: "mod".into(),
        download_strategy: sources
            .first()
            .map(|s| s.strategy.clone())
            .unwrap_or_default(),
        source_identifier: REPO.into(),
        sha256: None,
        download_sources: sources,
        game_compatibility: compats,
        description: None,
        license_id: Some("MIT".into()),
        page_url: None,
        icon_url: None,
        status: "active".into(),
        date_added: None,
    }
}

/// The standard entry: GitHub, `steam`, `1.0.*`, the asset `CrashLogger-*.zip`.
fn standard_entry(pins: Vec<SourcePin>) -> GameCatalogItem {
    entry(
        "crash-logger",
        "test-game",
        vec![github_source(pins)],
        vec![compat(
            &["steam"],
            &["1.0.*"],
            Some("CrashLogger-*.zip"),
            vec![],
        )],
    )
}

async fn run(
    fixture: &Fixture,
    transport: &dyn CatalogTransport,
    item: &GameCatalogItem,
    instance_id: &str,
    install_anyway: bool,
    dry_run: bool,
) -> LauncherResult<InstallReport> {
    install(
        &fixture.ctx,
        transport,
        InstallRequest {
            instance_id,
            item,
            install_anyway,
            dry_run,
        },
        &mut |_: &CatalogPlan| {},
    )
    .await
}

fn plain_archive() -> Vec<u8> {
    zip_bytes(&[
        ("CrashLogger.esp", b"plugin"),
        ("CrashLogger.bsa", b"archive"),
    ])
}

// ---------------------------------------------------------------------------
// Compatibility
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_store_mismatch_refuses_and_says_what_the_entry_supports() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("store");
    let item = entry(
        "crash-logger",
        "test-game",
        vec![github_source(vec![])],
        vec![compat(
            &["gog"],
            &["1.0.*"],
            Some("CrashLogger-*.zip"),
            vec![],
        )],
    );
    let transport = FakeTransport::silent();
    let error = run(&f, &transport, &item, &instance.instance_id, false, true)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_CATALOG_NO_COMPATIBLE_ENTRY");
    let message = error.to_string();
    assert!(
        message.contains("1.0.*") && message.contains("gog"),
        "{message}"
    );
    assert!(message.contains("steam"), "{message}");
}

#[tokio::test]
async fn a_version_mismatch_refuses_and_says_what_the_entry_supports() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("version");
    let item = entry(
        "crash-logger",
        "test-game",
        vec![github_source(vec![])],
        vec![compat(
            &["steam"],
            &["2.*"],
            Some("CrashLogger-*.zip"),
            vec![],
        )],
    );
    let transport = FakeTransport::silent();
    let message = run(&f, &transport, &item, &instance.instance_id, false, true)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("2.*") && message.contains("1.0.0"),
        "{message}"
    );
}

#[tokio::test]
async fn a_glob_version_matches_and_the_first_of_two_matching_entries_is_chosen() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("first");
    let item = entry(
        "crash-logger",
        "test-game",
        vec![github_source(vec![])],
        vec![
            compat(&["steam"], &["1.0.*"], Some("CrashLogger-A-*.zip"), vec![]),
            compat(&["steam"], &["1.0.0"], Some("CrashLogger-B-*.zip"), vec![]),
        ],
    );
    let a = b"a".as_slice();
    let releases = vec![release(
        "v1",
        "2026-01-01T00:00:00Z",
        false,
        false,
        &[
            Asset {
                name: "CrashLogger-A-1.zip",
                bytes: a,
                digest: Digest::Matching,
            },
            Asset {
                name: "CrashLogger-B-1.zip",
                bytes: b"b",
                digest: Digest::Matching,
            },
        ],
    )];
    let transport = FakeTransport::new(
        vec![releases],
        files_of(&[(
            "v1",
            &[
                Asset {
                    name: "CrashLogger-A-1.zip",
                    bytes: a,
                    digest: Digest::Matching,
                },
                Asset {
                    name: "CrashLogger-B-1.zip",
                    bytes: b"b",
                    digest: Digest::Matching,
                },
            ],
        )]),
    );
    let report = run(&f, &transport, &item, &instance.instance_id, false, true)
        .await
        .unwrap();
    assert_eq!(report.plan.compatibility_index, 0);
    assert_eq!(report.plan.source.file_name(), "CrashLogger-A-1.zip");
}

#[tokio::test]
async fn the_asset_pattern_is_case_insensitive() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("case");
    let item = entry(
        "crash-logger",
        "test-game",
        vec![github_source(vec![])],
        vec![compat(
            &["steam"],
            &["1.0.*"],
            Some("crashlogger-*.ZIP"),
            vec![],
        )],
    );
    let bytes = plain_archive();
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    let report = run(&f, &transport, &item, &instance.instance_id, false, true)
        .await
        .unwrap();
    assert_eq!(report.plan.source.file_name(), "CrashLogger-1.zip");
}

// ---------------------------------------------------------------------------
// Frameworks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_missing_framework_is_refused_and_names_what_to_install() {
    let f = fixture(None, true);
    let instance = f.instance("missing");
    let item = entry(
        "crash-logger",
        "test-game",
        vec![github_source(vec![])],
        vec![compat(
            &["steam"],
            &["1.0.*"],
            Some("CrashLogger-*.zip"),
            skse_requirement(Some("2.2.6")),
        )],
    );
    let transport = FakeTransport::silent();
    let message = run(&f, &transport, &item, &instance.instance_id, true, true)
        .await
        .unwrap_err()
        .to_string();
    assert!(message.contains("Script Extender"), "{message}");
    assert!(message.contains("2.2.6"), "{message}");
    assert_eq!(transport.fetch_count(), 0);
}

#[tokio::test]
async fn a_framework_below_the_minimum_is_refused_naming_both_versions_even_with_install_anyway() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 5])), true);
    let instance = f.instance("old");
    let item = entry(
        "crash-logger",
        "test-game",
        vec![github_source(vec![])],
        vec![compat(
            &["steam"],
            &["1.0.*"],
            Some("CrashLogger-*.zip"),
            skse_requirement(Some("2.2.6")),
        )],
    );
    let transport = FakeTransport::silent();
    for anyway in [false, true] {
        let error = run(&f, &transport, &item, &instance.instance_id, anyway, false)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_CATALOG_FRAMEWORK_TOO_OLD");
        let message = error.to_string();
        assert!(
            message.contains("2.2.6") && message.contains("2.2.5"),
            "{message}"
        );
    }
    assert_eq!(transport.fetch_count(), 0);
}

#[tokio::test]
async fn a_framework_equal_or_newer_than_the_minimum_passes_and_reports_its_version() {
    for (components, expected) in [([0, 2, 2, 6], "2.2.6"), ([0, 2, 3, 0], "2.3.0")] {
        let f = fixture(Some(pe_with_version(components)), true);
        let instance = f.instance(expected);
        let item = entry(
            "crash-logger",
            "test-game",
            vec![github_source(vec![])],
            vec![compat(
                &["steam"],
                &["1.0.*"],
                Some("CrashLogger-*.zip"),
                skse_requirement(Some("2.2.6")),
            )],
        );
        let bytes = plain_archive();
        let assets = [Asset {
            name: "CrashLogger-1.zip",
            bytes: &bytes,
            digest: Digest::Matching,
        }];
        let transport = FakeTransport::new(
            vec![vec![release(
                "v1",
                "2026-01-01T00:00:00Z",
                false,
                false,
                &assets,
            )]],
            files_of(&[("v1", &assets)]),
        );
        let report = run(&f, &transport, &item, &instance.instance_id, false, true)
            .await
            .unwrap();
        assert_eq!(
            report.plan.frameworks[0].found_version.as_deref(),
            Some(expected)
        );
        assert!(report.plan.warnings.is_empty());
    }
}

#[tokio::test]
async fn an_unreadable_framework_version_warns_and_proceeds() {
    let f = fixture(Some(b"not a pe file".to_vec()), true);
    let instance = f.instance("unreadable");
    let item = entry(
        "crash-logger",
        "test-game",
        vec![github_source(vec![])],
        vec![compat(
            &["steam"],
            &["1.0.*"],
            Some("CrashLogger-*.zip"),
            skse_requirement(Some("2.2.6")),
        )],
    );
    let bytes = plain_archive();
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    let report = run(&f, &transport, &item, &instance.instance_id, false, true)
        .await
        .unwrap();
    assert_eq!(report.plan.frameworks[0].found_version, None);
    assert_eq!(report.plan.warnings.len(), 1, "{:?}", report.plan.warnings);
    assert!(report.plan.warnings[0].contains("Script Extender"));
}

// ---------------------------------------------------------------------------
// GitHub releases
// ---------------------------------------------------------------------------

fn github_fixture_item() -> GameCatalogItem {
    standard_entry(vec![])
}

#[tokio::test]
async fn the_newest_matching_asset_wins() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("newest");
    let older = b"older".as_slice();
    let newer = b"newer".as_slice();
    let old_assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: older,
        digest: Digest::Matching,
    }];
    let new_assets = [Asset {
        name: "CrashLogger-2.zip",
        bytes: newer,
        digest: Digest::Matching,
    }];
    // Listed oldest first on purpose: the order of the listing must not decide.
    let transport = FakeTransport::new(
        vec![vec![
            release("v1", "2026-01-01T00:00:00Z", false, false, &old_assets),
            release("v2", "2026-03-01T00:00:00Z", false, false, &new_assets),
        ]],
        files_of(&[("v1", &old_assets), ("v2", &new_assets)]),
    );
    let report = run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        true,
    )
    .await
    .unwrap();
    assert_eq!(report.plan.source.release(), Some("v2"));
    assert_eq!(report.plan.source.file_name(), "CrashLogger-2.zip");
}

#[tokio::test]
async fn drafts_and_prereleases_are_skipped() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("drafts");
    let stable = b"stable".as_slice();
    let draft = b"draft".as_slice();
    let pre = b"pre".as_slice();
    let stable_assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: stable,
        digest: Digest::Matching,
    }];
    let draft_assets = [Asset {
        name: "CrashLogger-3.zip",
        bytes: draft,
        digest: Digest::Matching,
    }];
    let pre_assets = [Asset {
        name: "CrashLogger-2.zip",
        bytes: pre,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![vec![
            release("v3", "2026-05-01T00:00:00Z", true, false, &draft_assets),
            release("v2", "2026-04-01T00:00:00Z", false, true, &pre_assets),
            release("v1", "2026-01-01T00:00:00Z", false, false, &stable_assets),
        ]],
        files_of(&[
            ("v1", &stable_assets),
            ("v2", &pre_assets),
            ("v3", &draft_assets),
        ]),
    );
    let report = run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        true,
    )
    .await
    .unwrap();
    assert_eq!(report.plan.source.release(), Some("v1"));
}

#[tokio::test]
async fn no_matching_asset_is_a_clear_error() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("nomatch");
    let other = b"other".as_slice();
    let assets = [Asset {
        name: "Something-Else.zip",
        bytes: other,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        vec![],
    );
    let error = run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        true,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "ERR_CATALOG_NO_ASSET");
    assert!(error.to_string().contains("CrashLogger-*.zip"), "{error}");
}

#[tokio::test]
async fn a_match_on_a_later_page_is_found() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("pages");
    let bytes = plain_archive();
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![
            vec![release(
                "v2",
                "2026-02-01T00:00:00Z",
                false,
                false,
                &[Asset {
                    name: "Other.zip",
                    bytes: b"x",
                    digest: Digest::Matching,
                }],
            )],
            vec![release("v1", "2026-01-01T00:00:00Z", false, false, &assets)],
        ],
        files_of(&[("v1", &assets)]),
    );
    let report = run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        true,
    )
    .await
    .unwrap();
    assert_eq!(report.plan.source.release(), Some("v1"));
}

// ---------------------------------------------------------------------------
// Hashes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_published_digest_that_matches_verifies_and_is_recorded_as_verified() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("digest-match");
    let bytes = plain_archive();
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    let report = run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        false,
    )
    .await
    .unwrap();
    let InstallState::Placed { hash, .. } = &report.state else {
        panic!("expected a placed install, got {:?}", report.state);
    };
    assert!(hash.verified);
    assert!(hash.basis.contains("GitHub published"), "{}", hash.basis);
    let recorded = f.content_items();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0]
        .sources
        .iter()
        .any(|s| matches!(s, ContentSource::Catalog { verified: true, .. })));
}

#[tokio::test]
async fn a_published_digest_that_differs_is_refused_even_with_install_anyway() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("digest-mismatch");
    let bytes = plain_archive();
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Other,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    for anyway in [false, true] {
        let error = run(
            &f,
            &transport,
            &github_fixture_item(),
            &instance.instance_id,
            anyway,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), "ERR_HASH_MISMATCH", "install_anyway={anyway}");
    }
    assert_eq!(f.layer_count(&instance.instance_id), 0);
    assert!(
        f.content_items().is_empty(),
        "a refused download is not stored"
    );
}

#[tokio::test]
async fn no_published_digest_installs_as_unverified_records_the_hash_and_says_so() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("no-digest");
    let bytes = plain_archive();
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Absent,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    let report = run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        false,
    )
    .await
    .unwrap();
    let InstallState::Placed { hash, .. } = &report.state else {
        panic!("expected a placed install, got {:?}", report.state);
    };
    assert!(!hash.verified);
    assert!(
        hash.basis
            .contains("GitHub published no checksum for CrashLogger-1.zip"),
        "{}",
        hash.basis
    );
    assert_eq!(hash.sha256, sha256_hex(&bytes));
    let recorded = f.content_items();
    assert!(recorded[0].sources.iter().any(|s| matches!(
        s,
        ContentSource::Catalog { verified: false, sha256, release: Some(r), .. }
            if sha256 == &sha256_hex(&bytes) && r == "v1"
    )));
}

#[tokio::test]
async fn a_second_install_whose_bytes_differ_from_the_remembered_hash_stops_until_confirmed() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let first = f.instance("first");
    let second = f.instance("second");
    let bytes_one = plain_archive();
    let bytes_two = zip_bytes(&[("CrashLogger.esp", b"a changed plugin")]);
    let assets_one = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes_one,
        digest: Digest::Absent,
    }];
    let assets_two = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes_two,
        digest: Digest::Absent,
    }];
    let transport_one = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets_one,
        )]],
        files_of(&[("v1", &assets_one)]),
    );
    run(
        &f,
        &transport_one,
        &github_fixture_item(),
        &first.instance_id,
        false,
        false,
    )
    .await
    .unwrap();

    let transport_two = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets_two,
        )]],
        files_of(&[("v1", &assets_two)]),
    );
    let error = run(
        &f,
        &transport_two,
        &github_fixture_item(),
        &second.instance_id,
        false,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "ERR_HASH_CONFIRMATION_REQUIRED");
    assert_eq!(f.layer_count(&second.instance_id), 0);

    let report = run(
        &f,
        &transport_two,
        &github_fixture_item(),
        &second.instance_id,
        true,
        false,
    )
    .await
    .unwrap();
    let InstallState::Placed { hash, .. } = &report.state else {
        panic!("expected a placed install, got {:?}", report.state);
    };
    assert_eq!(hash.confirmed_past, Some(HashOrigin::PreviousInstall));
    assert_eq!(f.layer_count(&second.instance_id), 1);
}

#[tokio::test]
async fn a_curator_pin_that_matches_passes_as_unverified() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("pin-match");
    let bytes = plain_archive();
    let pin = SourcePin {
        tag: "v1".into(),
        asset: "CrashLogger-1.zip".into(),
        sha256: sha256_hex(&bytes),
    };
    let item = entry(
        "crash-logger",
        "test-game",
        vec![github_source(vec![pin])],
        vec![compat(
            &["steam"],
            &["1.0.*"],
            Some("CrashLogger-*.zip"),
            vec![],
        )],
    );
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Absent,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    let report = run(&f, &transport, &item, &instance.instance_id, false, false)
        .await
        .unwrap();
    let InstallState::Placed { hash, .. } = &report.state else {
        panic!("expected a placed install, got {:?}", report.state);
    };
    assert!(!hash.verified);
    assert!(hash.basis.contains("curator's pin"), "{}", hash.basis);
}

#[tokio::test]
async fn a_curator_pin_that_differs_stops_until_confirmed() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("pin-differs");
    let bytes = plain_archive();
    let pin = SourcePin {
        tag: "v1".into(),
        asset: "CrashLogger-1.zip".into(),
        sha256: "f".repeat(64),
    };
    let item = entry(
        "crash-logger",
        "test-game",
        vec![github_source(vec![pin])],
        vec![compat(
            &["steam"],
            &["1.0.*"],
            Some("CrashLogger-*.zip"),
            vec![],
        )],
    );
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Absent,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    let error = run(&f, &transport, &item, &instance.instance_id, false, false)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_HASH_CONFIRMATION_REQUIRED");
    let report = run(&f, &transport, &item, &instance.instance_id, true, false)
        .await
        .unwrap();
    let InstallState::Placed { hash, .. } = &report.state else {
        panic!("expected a placed install, got {:?}", report.state);
    };
    assert_eq!(hash.confirmed_past, Some(HashOrigin::CuratorPin));
}

#[tokio::test]
async fn a_direct_hash_that_matches_verifies_and_one_that_differs_is_refused_even_with_install_anyway(
) {
    let bytes = plain_archive();
    let url = "https://files.example-author.example/crash-logger/CrashLogger-1.zip".to_string();
    let direct = |sha: String| {
        let mut item = entry(
            "crash-logger",
            "test-game",
            vec![DownloadSource {
                strategy: "direct_hash".into(),
                identifier: url.clone(),
                pins: vec![],
            }],
            vec![compat(&["steam"], &["1.0.*"], None, vec![])],
        );
        item.sha256 = Some(sha);
        item
    };

    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("direct-ok");
    let transport = FakeTransport::new(vec![], vec![(url.clone(), bytes.clone())]);
    let report = run(
        &f,
        &transport,
        &direct(sha256_hex(&bytes)),
        &instance.instance_id,
        false,
        false,
    )
    .await
    .unwrap();
    let InstallState::Placed { hash, .. } = &report.state else {
        panic!("expected a placed install, got {:?}", report.state);
    };
    assert!(hash.verified);
    assert!(hash.basis.contains("curator's manifest"), "{}", hash.basis);

    let other = f.instance("direct-bad");
    let error = run(
        &f,
        &transport,
        &direct("d".repeat(64)),
        &other.instance_id,
        true,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "ERR_HASH_MISMATCH");
}

// ---------------------------------------------------------------------------
// Placement and the instance
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_plain_archive_is_placed_the_way_content_add_would_place_it() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("plain");
    let bytes = plain_archive();
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    let report = run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        false,
    )
    .await
    .unwrap();
    let InstallState::Placed {
        mount_path,
        source_path,
        content_item_id,
        ..
    } = &report.state
    else {
        panic!("expected a placed install, got {:?}", report.state);
    };
    // Root-level .esp and .bsa files go under the data folder, as `content add` places them.
    assert_eq!(mount_path, "Data");
    assert_eq!(source_path, "");
    let manifest = get_manifest(&f.ctx, &instance.instance_id).unwrap();
    let layer = manifest
        .layers
        .layers()
        .iter()
        .find(
            |l| matches!(&l.source, LayerSource::Content { content } if content == content_item_id),
        )
        .expect("the content layer is in the instance");
    assert_eq!(layer.mount_path.as_str(), "Data");
    let item = content_store::get_item(&f.ctx, content_item_id).unwrap();
    let layout = f.definition.content_layout.as_ref().unwrap();
    assert!(matches!(
        game_deploy::decide_placement(&item, layout),
        PlacementDecision::Place { ref mount_path, ref source_path, .. }
            if mount_path == "Data" && source_path.is_empty()
    ));
}

#[tokio::test]
async fn a_fomod_archive_is_imported_but_not_added() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("fomod");
    let bytes = zip_bytes(&[
        ("fomod/ModuleConfig.xml", b"<config/>"),
        ("Data/CrashLogger.esp", b"plugin"),
    ]);
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    let report = run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        false,
    )
    .await
    .unwrap();
    let InstallState::NeedsInstaller {
        content_item_id,
        installer_command,
        ..
    } = &report.state
    else {
        panic!("expected an installer state, got {:?}", report.state);
    };
    assert!(installer_command.contains("agora games content fomod install"));
    assert!(installer_command.contains(content_item_id.as_str()));
    assert_eq!(f.layer_count(&instance.instance_id), 0);
    assert_eq!(f.content_items().len(), 1, "the archive is imported");
}

#[tokio::test]
async fn an_archive_no_rule_places_is_imported_and_refused_with_where_to_put_it() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("unknown");
    let bytes = zip_bytes(&[("readme.txt", b"hello")]);
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    let error = run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "ERR_CATALOG_UNPLACEABLE");
    assert!(error.to_string().contains("--into"), "{error}");
    assert_eq!(f.layer_count(&instance.instance_id), 0);
}

#[tokio::test]
async fn a_reinstall_of_the_same_item_is_a_no_op() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("reinstall");
    let bytes = plain_archive();
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        false,
    )
    .await
    .unwrap();
    let fetches_after_first = transport.fetch_count();
    let again = run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        again.state,
        InstallState::AlreadyInstalled {
            installed_release: Some("v1".into()),
            same_release: true,
        }
    );
    assert_eq!(
        transport.fetch_count(),
        fetches_after_first,
        "nothing is downloaded again"
    );
    assert_eq!(f.layer_count(&instance.instance_id), 1);
}

#[tokio::test]
async fn an_installed_item_with_a_newer_release_changes_nothing_and_says_so() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("update");
    let old = plain_archive();
    let new = zip_bytes(&[("CrashLogger.esp", b"newer plugin")]);
    let old_assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &old,
        digest: Digest::Matching,
    }];
    let new_assets = [Asset {
        name: "CrashLogger-2.zip",
        bytes: &new,
        digest: Digest::Matching,
    }];
    let first = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &old_assets,
        )]],
        files_of(&[("v1", &old_assets)]),
    );
    run(
        &f,
        &first,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        false,
    )
    .await
    .unwrap();
    let second = FakeTransport::new(
        vec![vec![release(
            "v2",
            "2026-03-01T00:00:00Z",
            false,
            false,
            &new_assets,
        )]],
        files_of(&[("v2", &new_assets)]),
    );
    let report = run(
        &f,
        &second,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        report.state,
        InstallState::AlreadyInstalled {
            installed_release: Some("v1".into()),
            same_release: false,
        }
    );
    assert_eq!(second.fetch_count(), 0);
    assert_eq!(f.layer_count(&instance.instance_id), 1);
}

#[tokio::test]
async fn dry_run_downloads_and_stores_nothing() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("dry");
    let bytes = plain_archive();
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    let report = run(
        &f,
        &transport,
        &github_fixture_item(),
        &instance.instance_id,
        false,
        true,
    )
    .await
    .unwrap();
    assert_eq!(report.state, InstallState::DryRun);
    assert_eq!(report.plan.source.file_name(), "CrashLogger-1.zip");
    assert_eq!(transport.fetch_count(), 0);
    assert!(f.content_items().is_empty());
    assert_eq!(f.layer_count(&instance.instance_id), 0);
}

#[tokio::test]
async fn an_item_for_another_game_is_refused() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("wrong-game");
    let item = entry(
        "crash-logger",
        "valheim",
        vec![github_source(vec![])],
        vec![compat(
            &["steam"],
            &["1.0.*"],
            Some("CrashLogger-*.zip"),
            vec![],
        )],
    );
    let transport = FakeTransport::silent();
    let error = run(&f, &transport, &item, &instance.instance_id, false, true)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ERR_CATALOG_WRONG_GAME");
    assert_eq!(transport.fetch_count(), 0);
}

#[tokio::test]
async fn the_install_names_its_source_before_a_download_starts() {
    let f = fixture(Some(pe_with_version([0, 2, 2, 6])), true);
    let instance = f.instance("announce");
    let bytes = plain_archive();
    let assets = [Asset {
        name: "CrashLogger-1.zip",
        bytes: &bytes,
        digest: Digest::Matching,
    }];
    let transport = FakeTransport::new(
        vec![vec![release(
            "v1",
            "2026-01-01T00:00:00Z",
            false,
            false,
            &assets,
        )]],
        files_of(&[("v1", &assets)]),
    );
    let mut announced: Vec<String> = Vec::new();
    install(
        &f.ctx,
        &transport,
        InstallRequest {
            instance_id: &instance.instance_id,
            item: &github_fixture_item(),
            install_anyway: false,
            dry_run: false,
        },
        &mut |plan: &CatalogPlan| {
            announced.push(format!(
                "{} from {}",
                plan.source.file_name(),
                plan.source.describe()
            ))
        },
    )
    .await
    .unwrap();
    assert_eq!(announced.len(), 1);
    assert!(announced[0].contains("CrashLogger-1.zip"), "{announced:?}");
    assert!(announced[0].contains(REPO), "{announced:?}");
}
