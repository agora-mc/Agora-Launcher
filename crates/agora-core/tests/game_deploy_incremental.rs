//! Incremental deploys (MASTER_SPEC §26.5): a plan change updates the farm in place, and the farm
//! must come out exactly as a full rebuild of the same plan leaves it. Each step applies one change
//! to two instances of the same base and layers. `X` is deployed in place; `Y` is undeployed and
//! deployed again, which is the full rebuild. The two farms must then match file for file.

#[allow(dead_code)]
mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agora_core::ctx::CoreContext;
use agora_core::game_base::{get_file_identity, BaseManifest, BaseMode, FileIdentity};
use agora_core::game_deploy::{
    add_content, deploy, deployment_dir, move_content, remove_content, set_content_enabled,
    set_content_own_copy, set_update_fault_after, undeploy, DeployError, DeployMode, DeployOutcome,
    DeploymentRecord, Placement,
};
use agora_core::game_discovery::{DiscoveredInstall, InstallCapabilities};
use agora_core::game_instance::{create, get_manifest, GameInstanceRecord};
use agora_core::game_registry::{
    GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
};
use agora_game_api::{
    BaseReference, DeploymentStrategy, GameDefinition, GameId, GamePackage, GamePath,
    InputFingerprint, InstallId, InstallKind, LaunchRecipe, LaunchValue, Layer, LayerId,
    LayerSource, LayerStack, PackageDefinition, RelPath, RuntimeIdentity, StoreId, StoreIdentifier,
    ToolId,
};
use common::data_root::DataRoot;

struct TestPackage(PackageDefinition);

impl GamePackage for TestPackage {
    fn definition(&self) -> &PackageDefinition {
        &self.0
    }
}

fn definition() -> GameDefinition {
    GameDefinition {
        mo2_game_name: None,
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
        framework_ids: vec![],
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
        linked_archive_patterns: vec!["Data/*.bsa".into()],
        declared_writes: vec!["writeable_base.txt".into()],
        excluded_paths: vec![],
        plugin_list: None,
        runtime_files: Vec::new(),
        save_location: Vec::new(),
        launch_alternatives: Vec::new(),
        content_layout: None,
        copy_patterns: Vec::new(),
    }
}

fn register(def: GameDefinition) -> Arc<GameRegistry> {
    let mut builder = GameRegistry::builder();
    let pkg_def = PackageDefinition {
        id: format!("test.{}", def.id),
        version: semver::Version::new(0, 1, 0),
        api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
        parents: vec![],
        games: vec![def],
        frameworks: vec![],
        tools: vec![],
    };
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "test".into(),
            },
            Arc::new(TestPackage(pkg_def)),
        )
        .expect("register test package");
    Arc::new(builder.build())
}

fn make_install(install_dir: &Path, def: &GameDefinition) -> IdentifiedInstall {
    let store_id = StoreId::new("steam").unwrap();
    let runtime = RuntimeIdentity {
        game: def.id.clone(),
        store: store_id.clone(),
        version: "1.0.0".into(),
        build: None,
    };
    let detector = agora_core::game_discovery::volume::VolumeDetector::new();
    let volume = detector.get_volume_info(install_dir);
    let discovered = DiscoveredInstall {
        store: store_id,
        product: "12345".into(),
        name: "Test Game".into(),
        kind: InstallKind::BaseGame,
        parent_product: None,
        location: install_dir.to_path_buf(),
        store_version: Some(runtime.version.clone()),
        store_build: runtime.build.clone(),
        executables: vec!["Game.exe".into()],
        capabilities: InstallCapabilities {
            executables_readable: true,
            accepts_new_files: true,
            relocatable: true,
        },
        volume,
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

/// Two instances of one base, with the same changes applied to both.
struct Fixture {
    root: DataRoot,
    ctx: CoreContext,
    def: GameDefinition,
    /// Deployed in place.
    x: String,
    /// Deployed from scratch: the reference.
    y: String,
}

fn fixture() -> Fixture {
    let root = DataRoot::new();
    let def = definition();
    let install_dir = root.path().join("install");
    std::fs::create_dir_all(install_dir.join("Data")).unwrap();
    std::fs::write(install_dir.join("Game.exe"), b"fake game binary").unwrap();
    std::fs::write(install_dir.join("base_file.txt"), b"initial base file").unwrap();
    std::fs::write(
        install_dir.join("writeable_base.txt"),
        b"initial writeable base",
    )
    .unwrap();
    std::fs::write(install_dir.join("Data").join("Skyrim.bsa"), b"BSA DATA").unwrap();

    let ctx = CoreContext::for_testing(root.path().join("app_data"));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
    let ctx = ctx.with_games(register(def.clone()));
    let install = make_install(&install_dir, &def);
    let x = create_named(&ctx, &install, &def, "X");
    let y = create_named(&ctx, &install, &def, "Y");
    Fixture {
        root,
        ctx,
        def,
        x,
        y,
    }
}

fn create_named(
    ctx: &CoreContext,
    install: &IdentifiedInstall,
    def: &GameDefinition,
    name: &str,
) -> String {
    let record: GameInstanceRecord =
        create(ctx, install, def, name, None, BaseMode::Linked, &|_| {}).expect("create instance");
    record.instance_id
}

/// A content item made of the given files.
fn item(f: &Fixture, name: &str, files: &[(&str, &[u8])]) -> String {
    let mod_dir = f.root.path().join(format!("mod-{name}"));
    for (rel, content) in files {
        let path = mod_dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    let outcome = agora_core::content_store::add_folder(&f.ctx, &mod_dir, Some(name)).unwrap();
    outcome.item().item_id.clone()
}

/// Apply one change to both instances.
fn both<T>(f: &Fixture, change: impl Fn(&CoreContext, &str) -> T) {
    change(&f.ctx, &f.x);
    change(&f.ctx, &f.y);
}

/// The game folder of an instance that has a record.
fn game_dir(f: &Fixture, id: &str) -> PathBuf {
    deployment_dir(&f.ctx, id)
        .unwrap()
        .expect("the instance is deployed")
}

/// The deployment folder of an instance, whether or not it has a record.
fn deployment_folder(f: &Fixture, id: &str) -> PathBuf {
    let manifest = get_manifest(&f.ctx, id).unwrap();
    let base_id = match manifest.base {
        BaseReference::Pinned { id, .. } => id,
        other => panic!("the instance is pinned: {other:?}"),
    };
    let text = std::fs::read_to_string(f.ctx.paths.base_manifest_path(&base_id)).unwrap();
    let base: BaseManifest = serde_json::from_str(&text).unwrap();
    base.location.parent().unwrap().join("deployments").join(id)
}

fn read_record(folder: &Path) -> DeploymentRecord {
    let text = std::fs::read_to_string(folder.join("deployment.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Rewrite an instance's layer stack, through its manifest file.
fn edit_layers(f: &Fixture, id: &str, edit: impl FnOnce(&mut Vec<Layer>)) {
    let mut manifest = get_manifest(&f.ctx, id).unwrap();
    let mut layers = manifest.layers.layers().to_vec();
    edit(&mut layers);
    manifest.layers = LayerStack::new(layers).unwrap();
    let dir = f.ctx.paths.instance_dir(id).unwrap();
    std::fs::write(
        dir.join("instance_manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

/// A generated layer of the test tool, at one generation.
fn generated_layer(generation: &str) -> Layer {
    Layer {
        id: LayerId::new("generated-gen").unwrap(),
        enabled: true,
        mount_path: RelPath::default(),
        source_path: RelPath::default(),
        source: LayerSource::Generated {
            tool: ToolId::new("gen").unwrap(),
            generation: generation.into(),
            inputs: InputFingerprint::Unknown,
        },
        whiteouts: Vec::new(),
        own_copy: false,
    }
}

/// Write one generation's files into its folder.
fn write_generation(folder: &Path, files: &[(&str, &[u8])]) {
    for (rel, content) in files {
        let path = folder.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

/// One node of a farm: a folder, or a file with its bytes and its file identity (a link to a
/// content object has the object's identity; a copy has its own).
#[derive(Debug, PartialEq)]
struct Node {
    dir: bool,
    bytes: Vec<u8>,
    identity: Option<FileIdentity>,
}

fn snapshot(root: &Path) -> BTreeMap<String, Node> {
    fn walk(root: &Path, rel: &str, out: &mut BTreeMap<String, Node>) {
        let folder = if rel.is_empty() {
            root.to_path_buf()
        } else {
            root.join(rel)
        };
        for entry in std::fs::read_dir(&folder).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            let child = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            let path = root.join(&child);
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                out.insert(
                    child.clone(),
                    Node {
                        dir: true,
                        bytes: Vec::new(),
                        identity: None,
                    },
                );
                walk(root, &child, out);
            } else {
                out.insert(
                    child,
                    Node {
                        dir: false,
                        bytes: std::fs::read(&path).unwrap(),
                        identity: get_file_identity(&path),
                    },
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, "", &mut out);
    out
}

/// The farm of `x` must be the farm of `y`: the same folders and files, the same bytes, and each
/// file placed the same way (a link shares its content object's identity; a copy has its own). The
/// two records name the same files, with the same placement, size and source, apart from times.
fn assert_same_farm(f: &Fixture) {
    let gx = game_dir(f, &f.x);
    let gy = game_dir(f, &f.y);
    let farm_x = snapshot(&gx);
    let farm_y = snapshot(&gy);
    assert_eq!(
        farm_x.keys().collect::<Vec<_>>(),
        farm_y.keys().collect::<Vec<_>>(),
        "the folders and files differ"
    );

    let rec_x = read_record(gx.parent().unwrap());
    let rec_y = read_record(gy.parent().unwrap());
    assert_eq!(rec_x.mode, rec_y.mode);
    assert_eq!(rec_x.files.len(), rec_y.files.len());
    let normalized = |text: String| text.replace(&f.x, "<instance>").replace(&f.y, "<instance>");
    for (x_rec, y_rec) in rec_x.files.iter().zip(&rec_y.files) {
        assert_eq!(x_rec.path, y_rec.path, "record order");
        assert_eq!(
            x_rec.placement, y_rec.placement,
            "placement of {}",
            x_rec.path
        );
        assert_eq!(x_rec.size, y_rec.size, "size of {}", x_rec.path);
        assert_eq!(
            normalized(format!("{:?}", x_rec.source)),
            normalized(format!("{:?}", y_rec.source)),
            "source of {}",
            x_rec.path
        );
    }
    let placement_y: BTreeMap<&str, Placement> = rec_y
        .files
        .iter()
        .map(|rec| (rec.path.as_str(), rec.placement))
        .collect();

    for (path, node_x) in &farm_x {
        let node_y = &farm_y[path];
        assert_eq!(node_x.dir, node_y.dir, "{path}");
        if node_x.dir {
            continue;
        }
        assert_eq!(node_x.bytes, node_y.bytes, "bytes of {path}");
        match placement_y.get(path.as_str()) {
            Some(Placement::Link) => {
                assert_eq!(
                    node_x.identity, node_y.identity,
                    "{path} is not linked alike"
                );
            }
            Some(Placement::Copy) => {
                assert_ne!(
                    node_x.identity, node_y.identity,
                    "{path} is linked, not copied"
                );
            }
            None => panic!("{path} is in the farm but not in the reference record"),
        }
    }
}

/// `y` is undeployed and deployed again, from scratch, and `x` must match it.
fn reference_matches(f: &Fixture, mode: DeployMode) {
    undeploy(&f.ctx, &f.y).unwrap();
    let reference = deploy(&f.ctx, &f.y, &f.def, mode).unwrap();
    assert!(
        matches!(reference, DeployOutcome::Built { kept: 0, .. }),
        "{reference:?}"
    );
    assert_same_farm(f);
}

/// Deploy `x` (in place, when it can) and check it against the reference. Returns `x`'s outcome.
fn deploy_and_check(f: &Fixture, mode: DeployMode) -> DeployOutcome {
    let outcome = deploy(&f.ctx, &f.x, &f.def, mode).unwrap();
    reference_matches(f, mode);
    outcome
}

/// (files placed, files kept, files removed) of a built outcome.
fn built_counts(outcome: &DeployOutcome) -> (usize, usize, usize) {
    match outcome {
        DeployOutcome::Built {
            linked,
            copied,
            kept,
            removed,
            ..
        } => (linked + copied, *kept, *removed),
        other => panic!("expected a built deployment, got {other:?}"),
    }
}

/// The bytes the game sees at `rel` in an instance's farm.
fn farm_bytes(f: &Fixture, id: &str, rel: &str) -> Vec<u8> {
    std::fs::read(game_dir(f, id).join(rel)).unwrap()
}

#[test]
fn an_update_in_place_matches_a_full_rebuild_through_a_series_of_plan_changes() {
    let f = fixture();
    let a = item(
        &f,
        "ModA",
        &[
            ("Data/a/one.txt", b"a1"),
            ("Data/a/two.txt", b"a2"),
            ("Data/common.txt", b"from A"),
        ],
    );
    let b = item(
        &f,
        "ModB",
        &[("Data/b/one.txt", b"b1"), ("Data/common.txt", b"from B")],
    );
    let c = item(
        &f,
        "ModC",
        &[
            ("Data/c/big.dat", &[7u8; 4096]),
            ("Data/c/config.ini", b"[c]\nvalue=1\n"),
        ],
    );
    both(&f, |ctx, id| add_content(ctx, id, &a, None, None).unwrap());
    both(&f, |ctx, id| add_content(ctx, id, &b, None, None).unwrap());
    deploy_and_check(&f, DeployMode::Links);
    assert_eq!(farm_bytes(&f, &f.x, "Data/common.txt"), b"from B");

    // A layer added: its two files are placed, and nothing is removed.
    both(&f, |ctx, id| add_content(ctx, id, &c, None, None).unwrap());
    let outcome = deploy_and_check(&f, DeployMode::Links);
    let (placed, kept, removed) = built_counts(&outcome);
    assert_eq!((placed, removed), (2, 0), "{outcome:?}");
    assert!(kept > 0, "{outcome:?}");

    // A layer removed.
    both(&f, |ctx, id| remove_content(ctx, id, &c).unwrap());
    let outcome = deploy_and_check(&f, DeployMode::Links);
    assert!(built_counts(&outcome).1 > 0, "{outcome:?}");

    // A layer disabled, then enabled again: A's common.txt shows while B is off.
    both(&f, |ctx, id| {
        set_content_enabled(ctx, id, &b, false).unwrap()
    });
    deploy_and_check(&f, DeployMode::Links);
    assert_eq!(farm_bytes(&f, &f.x, "Data/common.txt"), b"from A");
    both(&f, |ctx, id| {
        set_content_enabled(ctx, id, &b, true).unwrap()
    });
    deploy_and_check(&f, DeployMode::Links);
    assert_eq!(farm_bytes(&f, &f.x, "Data/common.txt"), b"from B");

    // Two layers reordered: the conflict's winner changes, both ways.
    both(&f, |ctx, id| move_content(ctx, id, &b, 0).unwrap());
    deploy_and_check(&f, DeployMode::Links);
    assert_eq!(farm_bytes(&f, &f.x, "Data/common.txt"), b"from A");
    both(&f, |ctx, id| move_content(ctx, id, &a, 0).unwrap());
    deploy_and_check(&f, DeployMode::Links);
    assert_eq!(farm_bytes(&f, &f.x, "Data/common.txt"), b"from B");

    // An own-copy toggle: the layer's files become copies, then links again.
    both(&f, |ctx, id| {
        set_content_own_copy(ctx, id, &a, true).unwrap()
    });
    let outcome = deploy_and_check(&f, DeployMode::Links);
    assert!(built_counts(&outcome).1 > 0, "{outcome:?}");
    both(&f, |ctx, id| {
        set_content_own_copy(ctx, id, &a, false).unwrap()
    });
    deploy_and_check(&f, DeployMode::Links);

    // A switch from links to virtual mode, and back.
    let outcome = deploy_and_check(&f, DeployMode::Virtual);
    assert!(built_counts(&outcome).1 > 0, "{outcome:?}");
    let outcome = deploy_and_check(&f, DeployMode::Links);
    assert!(built_counts(&outcome).1 > 0, "{outcome:?}");

    // A generated layer added, then its generation replaced by a new one.
    for id in [f.x.clone(), f.y.clone()] {
        let tool_dir = f
            .ctx
            .paths
            .instance_dir(&id)
            .unwrap()
            .join("generated")
            .join("gen");
        write_generation(&tool_dir.join("1"), &[("Data/gen/out.txt", b"gen 1")]);
    }
    both(&f, |_ctx, id| {
        edit_layers(&f, id, |layers| layers.push(generated_layer("1")));
    });
    let outcome = deploy_and_check(&f, DeployMode::Links);
    assert!(built_counts(&outcome).1 > 0, "{outcome:?}");

    for id in [f.x.clone(), f.y.clone()] {
        let tool_dir = f
            .ctx
            .paths
            .instance_dir(&id)
            .unwrap()
            .join("generated")
            .join("gen");
        write_generation(
            &tool_dir.join("2"),
            &[
                ("Data/gen/out.txt", b"gen 2"),
                ("Data/gen/extra.txt", b"extra"),
            ],
        );
    }
    both(&f, |_ctx, id| {
        edit_layers(&f, id, |layers| {
            for layer in layers.iter_mut() {
                if let LayerSource::Generated { generation, .. } = &mut layer.source {
                    *generation = "2".into();
                }
            }
        });
    });
    let outcome = deploy_and_check(&f, DeployMode::Links);
    assert!(built_counts(&outcome).1 > 0, "{outcome:?}");
    assert_eq!(farm_bytes(&f, &f.x, "Data/gen/out.txt"), b"gen 2");
}

#[test]
fn a_game_edit_and_a_game_deletion_in_the_farm_survive_an_update_in_place() {
    let f = fixture();
    let a = item(
        &f,
        "ModA",
        &[("Data/a/one.txt", b"a1"), ("Data/a/two.txt", b"a2")],
    );
    both(&f, |ctx, id| add_content(ctx, id, &a, None, None).unwrap());
    deploy_and_check(&f, DeployMode::Links);

    // In each farm the game edits a copied file, deletes a linked one, and replaces another linked
    // one with a file of its own. Then the plan changes.
    both(&f, |ctx, id| {
        let game = deployment_dir(ctx, id).unwrap().unwrap();
        std::fs::write(game.join("writeable_base.txt"), b"edited by the game").unwrap();
        std::fs::remove_file(game.join("Data/a/one.txt")).unwrap();
        std::fs::remove_file(game.join("Data/a/two.txt")).unwrap();
        std::fs::write(game.join("Data/a/two.txt"), b"replaced by the game").unwrap();
    });
    let c = item(&f, "ModC", &[("Data/c/two.txt", b"c2")]);
    both(&f, |ctx, id| add_content(ctx, id, &c, None, None).unwrap());
    let outcome = deploy_and_check(&f, DeployMode::Links);
    assert!(built_counts(&outcome).1 > 0, "{outcome:?}");

    // The copied edit is in the writable layer, and the farm shows it; the deletion stays deleted.
    assert_eq!(
        farm_bytes(&f, &f.x, "writeable_base.txt"),
        b"edited by the game"
    );
    let writable = f.ctx.paths.instance_dir(&f.x).unwrap().join("writable");
    assert_eq!(
        std::fs::read(writable.join("writeable_base.txt")).unwrap(),
        b"edited by the game"
    );
    assert!(!game_dir(&f, &f.x).join("Data/a/one.txt").exists());
    // A linked file the game replaced is put back as the content it links to, as a rebuild does.
    assert_eq!(farm_bytes(&f, &f.x, "Data/a/two.txt"), b"a2");
}

#[test]
fn a_deleted_game_folder_is_built_again_and_the_writable_layer_is_kept() {
    let f = fixture();
    let a = item(&f, "ModA", &[("Data/a/one.txt", b"a1")]);
    both(&f, |ctx, id| add_content(ctx, id, &a, None, None).unwrap());
    deploy_and_check(&f, DeployMode::Links);
    // The game writes a file, which a deploy harvests into the writable layer.
    let game = game_dir(&f, &f.x);
    std::fs::write(game.join("writeable_base.txt"), b"edited by the game").unwrap();
    let c = item(&f, "ModC", &[("Data/c/two.txt", b"c2")]);
    add_content(&f.ctx, &f.x, &c, None, None).unwrap();
    deploy(&f.ctx, &f.x, &f.def, DeployMode::Links).unwrap();
    let writable = f.ctx.paths.instance_dir(&f.x).unwrap().join("writable");
    assert!(writable.join("writeable_base.txt").is_file());

    // Someone deletes the deployment's game folder to free space. The next deploy rebuilds it, and
    // neither deletes the writable file nor whites out the instance's files.
    agora_core::content_store::protect::unprotect(&game).ok();
    let mut stack = vec![game.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let _ = agora_core::content_store::protect::unprotect(&p);
            }
        }
    }
    std::fs::remove_dir_all(&game).unwrap();
    deploy(&f.ctx, &f.x, &f.def, DeployMode::Links).unwrap();
    assert_eq!(
        std::fs::read(writable.join("writeable_base.txt")).unwrap(),
        b"edited by the game"
    );
    assert_eq!(farm_bytes(&f, &f.x, "Data/a/one.txt"), b"a1");
    assert_eq!(
        farm_bytes(&f, &f.x, "writeable_base.txt"),
        b"edited by the game"
    );
}

#[test]
fn a_stop_part_way_leaves_a_marker_and_the_next_deploy_builds_the_farm_again() {
    let f = fixture();
    let a = item(
        &f,
        "ModA",
        &[("Data/a/one.txt", b"a1"), ("Data/common.txt", b"from A")],
    );
    let b = item(
        &f,
        "ModB",
        &[("Data/b/one.txt", b"b1"), ("Data/common.txt", b"from B")],
    );
    both(&f, |ctx, id| add_content(ctx, id, &a, None, None).unwrap());
    both(&f, |ctx, id| add_content(ctx, id, &b, None, None).unwrap());
    deploy_and_check(&f, DeployMode::Links);

    // Removing B removes two files and places one. The stop comes after the first removal.
    both(&f, |ctx, id| remove_content(ctx, id, &b).unwrap());
    set_update_fault_after(Some(1));
    let err = deploy(&f.ctx, &f.x, &f.def, DeployMode::Links).unwrap_err();
    set_update_fault_after(None);
    assert!(matches!(err, DeployError::Other(_)), "{err:?}");

    // The stop leaves the record set aside, the marker in its place, and the farm part updated.
    let folder = deployment_folder(&f, &f.x);
    assert!(folder.join("deployment.json.updating").exists());
    assert!(!folder.join("deployment.json").exists());
    assert!(folder.join("game").exists());
    assert!(!folder.join("game").join("Data/b/one.txt").exists());

    // The next deploy does not trust that farm: it builds it again, and it matches the reference.
    let outcome = deploy(&f.ctx, &f.x, &f.def, DeployMode::Links).unwrap();
    assert!(
        matches!(outcome, DeployOutcome::Built { kept: 0, .. }),
        "{outcome:?}"
    );
    assert!(!folder.join("deployment.json.updating").exists());
    reference_matches(&f, DeployMode::Links);
}

#[test]
fn a_marker_left_behind_fails_the_up_to_date_check_and_the_farm_is_built_again() {
    let f = fixture();
    let a = item(&f, "ModA", &[("Data/a/one.txt", b"a1")]);
    both(&f, |ctx, id| add_content(ctx, id, &a, None, None).unwrap());
    deploy_and_check(&f, DeployMode::Links);

    // The record was copied to the marker, as an interrupted update leaves it.
    let folder = deployment_folder(&f, &f.x);
    std::fs::copy(
        folder.join("deployment.json"),
        folder.join("deployment.json.updating"),
    )
    .unwrap();

    let outcome = deploy(&f.ctx, &f.x, &f.def, DeployMode::Links).unwrap();
    assert!(
        matches!(outcome, DeployOutcome::Built { kept: 0, .. }),
        "{outcome:?}"
    );
    assert!(!folder.join("deployment.json.updating").exists());
    reference_matches(&f, DeployMode::Links);
}

#[test]
fn a_record_naming_a_path_outside_the_farm_is_refused_and_nothing_outside_is_touched() {
    for escape in ["../outside.txt", "/outside.txt"] {
        let f = fixture();
        let a = item(&f, "ModA", &[("Data/a/one.txt", b"a1")]);
        both(&f, |ctx, id| add_content(ctx, id, &a, None, None).unwrap());
        deploy(&f.ctx, &f.x, &f.def, DeployMode::Links).unwrap();

        // A file beside the farm, where `game/../outside.txt` would reach.
        let folder = deployment_folder(&f, &f.x);
        let sentinel = folder.join("outside.txt");
        std::fs::write(&sentinel, b"keep me").unwrap();

        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(folder.join("deployment.json")).unwrap())
                .unwrap();
        value["files"][0]["path"] = serde_json::Value::String(escape.into());
        std::fs::write(
            folder.join("deployment.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();

        let err = deploy(&f.ctx, &f.x, &f.def, DeployMode::Links).unwrap_err();
        assert!(
            matches!(err, DeployError::CorruptDeployment(_)),
            "{escape}: {err:?}"
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep me", "{escape}");
        assert!(
            folder.join("game").join("Data/a/one.txt").exists(),
            "{escape}"
        );
    }
}

#[test]
fn a_record_of_another_instance_is_refused_and_its_farm_is_left_alone() {
    let f = fixture();
    let a = item(&f, "ModA", &[("Data/a/one.txt", b"a1")]);
    both(&f, |ctx, id| add_content(ctx, id, &a, None, None).unwrap());
    deploy(&f.ctx, &f.x, &f.def, DeployMode::Links).unwrap();
    deploy(&f.ctx, &f.y, &f.def, DeployMode::Links).unwrap();

    let folder_x = deployment_folder(&f, &f.x);
    let folder_y = deployment_folder(&f, &f.y);
    std::fs::copy(
        folder_y.join("deployment.json"),
        folder_x.join("deployment.json"),
    )
    .unwrap();

    let err = deploy(&f.ctx, &f.x, &f.def, DeployMode::Links).unwrap_err();
    assert!(
        matches!(err, DeployError::InvalidDeploymentDir(_)),
        "{err:?}"
    );
    assert!(folder_x.join("game").join("Data/a/one.txt").exists());
}
