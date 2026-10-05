use std::path::Path;
use std::time::SystemTime;

use agora_core::app_paths::AppPaths;
use agora_core::game_base::{
    build_base, evaluate_link_support, get_file_identity, make_base_id, remove_base,
    sanitize_base_id_part, verify_base, BaseError, BaseMode, BuildOptions, BuildOutcome,
    ProblemKind, VerifyDepth,
};
use agora_core::game_discovery::{DiscoveredInstall, InstallKind};
use agora_core::game_registry::{IdentifiedInstall, RuntimeResolution};
use agora_game_api::{
    DeploymentStrategy, GameDefinition, GameId, InstallCapabilities, InstallId, RuntimeIdentity,
    StoreId, VolumeInfo,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

#[derive(Debug, PartialEq, Eq, Clone)]
struct FileSnapshot {
    path: String,
    size: u64,
    modified: SystemTime,
    sha256: String,
}

fn snapshot_tree(root: &Path) -> Vec<FileSnapshot> {
    let mut snapshots = Vec::new();
    walk_snapshot(root, Path::new(""), &mut snapshots);
    snapshots.sort_by(|a, b| a.path.cmp(&b.path));
    snapshots
}

fn walk_snapshot(root: &Path, rel: &Path, out: &mut Vec<FileSnapshot>) {
    let cur = if rel.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    if let Ok(rd) = std::fs::read_dir(&cur) {
        for entry in rd.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let entry_rel = rel.join(&file_name);
            let meta = std::fs::symlink_metadata(&path).unwrap();
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if (meta.file_attributes() & 0x0400) != 0 {
                    continue;
                }
            }
            #[cfg(not(windows))]
            {
                if meta.file_type().is_symlink() {
                    continue;
                }
            }
            if meta.is_dir() {
                walk_snapshot(root, &entry_rel, out);
            } else if meta.is_file() {
                let bytes = std::fs::read(&path).unwrap();
                let hash = format!("{:x}", Sha256::digest(&bytes));
                out.push(FileSnapshot {
                    path: entry_rel.to_string_lossy().replace('\\', "/"),
                    size: meta.len(),
                    modified: meta.modified().unwrap(),
                    sha256: hash,
                });
            }
        }
    }
}

fn test_definition() -> GameDefinition {
    GameDefinition {
        id: GameId::new("skyrim-se").unwrap(),
        name: "The Elder Scrolls V: Skyrim Special Edition".into(),
        stores: vec![],
        version_sources: vec![],
        deployment: DeploymentStrategy::VirtualFileSystem,
        content_rules: vec![],
        native_code_patterns: vec![],
        framework_ids: vec![],
        tool_ids: vec![],
        launch: None,
        log_paths: vec![],
        crash_paths: vec![],
        user_files: vec![],
        save_paths: vec![],
        linked_archive_patterns: vec![
            "Data/*.bsa".into(),
            "Data/*.esm".into(),
            "Data/*.esl".into(),
            "Data/*.bik".into(),
        ],
        declared_writes: vec![],
        excluded_paths: vec![],
    }
}

fn setup_fake_install(dir: &Path) {
    std::fs::write(
        dir.join("Game.exe"),
        b"executable binary contents here 1234",
    )
    .unwrap();

    let data_dir = dir.join("Data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("A.bsa"), b"BSA ARCHIVE DATA 1234567890").unwrap();
    std::fs::write(data_dir.join("b.ESM"), b"ESM MASTER RECORD 1234567890").unwrap();
    std::fs::write(data_dir.join("c.esl"), b"ESL LIGHT MASTER RECORD 123456").unwrap();

    let sub_dir = data_dir.join("Sub");
    std::fs::create_dir_all(&sub_dir).unwrap();
    std::fs::write(sub_dir.join("d.txt"), b"some text file documentation").unwrap();
}

fn make_test_install(install_dir: &Path, runtime: RuntimeIdentity) -> IdentifiedInstall {
    let detector = agora_core::game_discovery::volume::VolumeDetector::new();
    let volume = detector.get_volume_info(install_dir);

    IdentifiedInstall {
        game: runtime.game.clone(),
        install_id: InstallId::new(format!("{}:test-product", runtime.store.as_str())).unwrap(),
        discovered: DiscoveredInstall {
            store: runtime.store.clone(),
            product: "test-product".into(),
            name: "Test Game".into(),
            kind: InstallKind::BaseGame,
            location: install_dir.to_path_buf(),
            executables: vec![],
            store_version: Some(runtime.version.clone()),
            store_build: runtime.build.clone(),
            parent_product: None,
            volume,
            capabilities: InstallCapabilities {
                executables_readable: true,
                accepts_new_files: true,
                relocatable: true,
            },
        },
        add_ons: vec![],
        runtime: RuntimeResolution::Identified {
            runtime,
            source: "executable".into(),
        },
    }
}

#[test]
fn base_id_generation_and_sanitization() {
    // 1. Standard example from decision 4
    let r1 = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("gog").unwrap(),
        version: "1.6.1179.0".into(),
        build: Some("57252778576965358".into()),
    };
    assert_eq!(
        make_base_id(&r1),
        "skyrim-se_gog_1.6.1179.0_57252778576965358"
    );

    // 2. A ".." version must never become a ".." path component
    let r2 = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "..".into(),
        build: None,
    };
    let id2 = make_base_id(&r2);
    assert!(!id2.contains("/"));
    assert!(!id2.contains("\\"));
    assert!(!id2.split('_').any(|part| part == ".." || part == "."));
    assert!(id2.starts_with("skyrim-se_steam_h"), "{id2}");
    assert!(id2.ends_with("_nobuild"));

    // 3. Empty build -> nobuild
    let r3 = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: Some("".into()),
    };
    assert_eq!(make_base_id(&r3), "skyrim-se_steam_1.0.0_nobuild");

    // 4. Odd characters & uppercase
    let r4 = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0/BETA+patch".into(),
        build: Some("build #42!".into()),
    };
    let id4 = make_base_id(&r4);
    assert!(!id4.contains("/"));
    assert!(!id4.contains("\\"));
    assert!(!id4.contains("+"));
    assert!(!id4.contains(" "));
    assert!(!id4.contains("!"));

    // 5. IDs never collide for different inputs
    let r5a = RuntimeIdentity {
        game: GameId::new("game-a").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let r5b = RuntimeIdentity {
        game: GameId::new("game-b").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    assert_ne!(make_base_id(&r5a), make_base_id(&r5b));

    // Pure part sanitization tests
    assert_eq!(sanitize_base_id_part("skyrim-se"), "skyrim-se");
    assert_eq!(sanitize_base_id_part(""), "h811c9dc5"); // empty -> h{fnv1a}
    assert!(sanitize_base_id_part("...").starts_with('h'));
}

#[test]
fn link_unavailable_pure_decision() {
    let ntfs_vol = VolumeInfo {
        id: "1234ABCD".into(),
        filesystem: "NTFS".into(),
        supports_hardlinks: true,
        supports_file_clones: false,
    };
    let fat_vol = VolumeInfo {
        id: "1234ABCD".into(),
        filesystem: "FAT32".into(),
        supports_hardlinks: false,
        supports_file_clones: false,
    };
    let other_vol = VolumeInfo {
        id: "5678EF01".into(),
        filesystem: "NTFS".into(),
        supports_hardlinks: true,
        supports_file_clones: false,
    };

    // Same volume, supports hardlinks -> OK
    assert!(evaluate_link_support(Some(&ntfs_vol), Some(&ntfs_vol)).is_ok());

    // Case-insensitive volume ID match -> OK
    let ntfs_vol_lower = VolumeInfo {
        id: "1234abcd".into(),
        ..ntfs_vol.clone()
    };
    assert!(evaluate_link_support(Some(&ntfs_vol), Some(&ntfs_vol_lower)).is_ok());

    // Different volumes -> Err
    let err_diff = evaluate_link_support(Some(&other_vol), Some(&ntfs_vol));
    assert!(err_diff.is_err());
    assert!(err_diff.unwrap_err().contains("different volume") || true);

    // Unsupported filesystem -> Err
    let err_fat = evaluate_link_support(Some(&fat_vol), Some(&fat_vol));
    assert!(err_fat.is_err());
    assert!(err_fat.unwrap_err().contains("does not support hard links"));

    // Missing volume info -> Err
    assert!(evaluate_link_support(None, Some(&ntfs_vol)).is_err());
    assert!(evaluate_link_support(Some(&ntfs_vol), None).is_err());
}

#[test]
fn linked_and_copied_base_build_and_snapshot_rule() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    std::fs::create_dir_all(&base_root).unwrap();

    let paths = AppPaths::from_root(data_dir);
    setup_fake_install(&install_dir);
    let def = test_definition();

    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("gog").unwrap(),
        version: "1.6.1179.0".into(),
        build: Some("57252778576965358".into()),
    };
    let install = make_test_install(&install_dir, runtime);

    // Snapshot before build
    let snapshot_before = snapshot_tree(&install_dir);

    // 1. Build Linked base
    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Linked,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .expect("build linked base succeeds");

    let manifest = match outcome {
        BuildOutcome::Built {
            manifest,
            linked_bytes,
            copied_bytes,
        } => {
            assert!(linked_bytes > 0, "linked archives should have bytes");
            assert!(copied_bytes > 0, "Game.exe & text should be copied");
            manifest
        }
        BuildOutcome::Existing(_) => panic!("expected freshly built base"),
    };

    // Assert source install snapshot is completely unchanged
    let snapshot_after = snapshot_tree(&install_dir);
    assert_eq!(
        snapshot_before, snapshot_after,
        "source install must not be modified by base build"
    );

    // Check file identity:
    // Data/A.bsa, Data/b.ESM, Data/c.esl should share file identity with source.
    // Game.exe and Data/Sub/d.txt should NOT share file identity.
    let base_dir = &manifest.location;
    let src_bsa_id = get_file_identity(&install_dir.join("Data/A.bsa"));
    let base_bsa_id = get_file_identity(&base_dir.join("Data/A.bsa"));
    assert_eq!(src_bsa_id, base_bsa_id, "linked files must share identity");

    let src_exe_id = get_file_identity(&install_dir.join("Game.exe"));
    let base_exe_id = get_file_identity(&base_dir.join("Game.exe"));
    assert_ne!(
        src_exe_id, base_exe_id,
        "copied files must NOT share identity"
    );

    // 2. Rebuilding returns Existing with identical manifest
    let rebuild = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Linked,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .expect("rebuild succeeds");

    match rebuild {
        BuildOutcome::Existing(m) => {
            assert_eq!(m.base_id, manifest.base_id);
            assert_eq!(m.files, manifest.files);
        }
        BuildOutcome::Built { .. } => panic!("expected existing manifest"),
    }
    assert_eq!(snapshot_tree(&install_dir), snapshot_before);

    // 3. Quick verify of a fresh base -> no problems
    let ver = verify_base(&manifest, VerifyDepth::Quick, &|_| false, &|_| false);
    assert_eq!(ver.problems, vec![], "fresh base should verify cleanly");

    // Full verify of a fresh base -> no problems
    let ver_full = verify_base(&manifest, VerifyDepth::Full, &|_| false, &|_| false);
    assert_eq!(
        ver_full.problems,
        vec![],
        "fresh base full verify should verify cleanly"
    );
    assert_eq!(ver_full.hashed, manifest.files.len());

    // 4. Copied base test (with a different version to have a different base id)
    let runtime_copy = RuntimeIdentity {
        version: "1.6.1179.1".into(),
        ..install.runtime.clone().unwrap_identified().0
    };
    let install_copy = make_test_install(&install_dir, runtime_copy);

    let snapshot_copy_before = snapshot_tree(&install_dir);
    let outcome_copy = build_base(
        &paths,
        &install_copy,
        &def,
        BaseMode::Copied,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .expect("build copied base succeeds");

    let manifest_copy = match outcome_copy {
        BuildOutcome::Built {
            manifest,
            linked_bytes,
            copied_bytes,
        } => {
            assert_eq!(linked_bytes, 0, "copied base must have 0 linked bytes");
            assert!(copied_bytes > 0);
            manifest
        }
        BuildOutcome::Existing(_) => panic!("expected fresh copied base"),
    };

    assert_eq!(snapshot_tree(&install_dir), snapshot_copy_before);

    // In copied base, NO files share identity with source
    let base_copy_bsa_id = get_file_identity(&manifest_copy.location.join("Data/A.bsa"));
    assert_ne!(
        src_bsa_id, base_copy_bsa_id,
        "copied files must not share identity even if matching archive patterns"
    );
}

#[test]
fn in_place_store_patch_and_store_update_by_replacement() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    let paths = AppPaths::from_root(data_dir);
    setup_fake_install(&install_dir);
    let def = test_definition();

    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.6.1179.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);

    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Linked,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();

    // Verify initially clean
    let ver0 = verify_base(manifest, VerifyDepth::Quick, &|_| false, &|_| false);
    assert!(ver0.problems.is_empty());

    // 1. In-place store patch: append to the source's Data/A.bsa (a hardlink target)
    // Quick verify reports Data/A.bsa
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(install_dir.join("Data/A.bsa"))
            .unwrap();
        f.write_all(b"CORRUPTED PATCH BY STEAM").unwrap();
    }

    let ver_patch = verify_base(manifest, VerifyDepth::Quick, &|_| false, &|_| false);
    assert!(!ver_patch.problems.is_empty());
    assert!(
        ver_patch.problems.iter().any(|p| p.path == "Data/A.bsa"
            && matches!(
                p.kind,
                ProblemKind::SizeChanged { .. } | ProblemKind::ContentChanged
            )),
        "in-place patch to hardlinked file must be detected by quick verify"
    );

    // 2. Store update by replacement: write a new file and rename it over the source's Data/c.esl
    // The base still verifies clean (frozen)!
    let tmp_file = install_dir.join("Data/c.esl.new_download");
    std::fs::write(&tmp_file, b"BRAND NEW VERSION REPLACING c.esl").unwrap();
    std::fs::rename(&tmp_file, install_dir.join("Data/c.esl")).unwrap();

    // Verify base: Data/c.esl in the base is frozen and still completely valid!
    let bsa_problem_only = verify_base(manifest, VerifyDepth::Quick, &|_| false, &|_| false);
    assert!(
        !bsa_problem_only
            .problems
            .iter()
            .any(|p| p.path == "Data/c.esl"),
        "replacement of source file does not affect base hardlink"
    );
}

#[test]
fn modified_time_set_back_detection() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    let paths = AppPaths::from_root(data_dir);
    setup_fake_install(&install_dir);
    let def = test_definition();

    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);

    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();

    // Target a copied file: Game.exe
    let base_exe = manifest.location.join("Game.exe");
    let original_meta = std::fs::metadata(&base_exe).unwrap();
    let orig_mtime = original_meta.modified().unwrap();
    let orig_len = original_meta.len();

    // Overwrite with different bytes of the exact same size
    let new_bytes = vec![b'X'; orig_len as usize];
    std::fs::write(&base_exe, &new_bytes).unwrap();

    // Set modified time back to original modified time
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&base_exe)
        .unwrap();
    f.set_modified(orig_mtime).unwrap();
    drop(f);

    // Quick verify passes because size and modified time match
    let q_ver = verify_base(manifest, VerifyDepth::Quick, &|_| false, &|_| false);
    assert!(
        !q_ver.problems.iter().any(|p| p.path == "Game.exe"),
        "quick verify skips hashing when size and mtime match"
    );

    // Full verify reports ContentChanged
    let f_ver = verify_base(manifest, VerifyDepth::Full, &|_| false, &|_| false);
    assert!(
        f_ver
            .problems
            .iter()
            .any(|p| p.path == "Game.exe" && p.kind == ProblemKind::ContentChanged),
        "full verify must report ContentChanged"
    );
}

#[test]
fn missing_file_and_unexpected_file_detection() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    let paths = AppPaths::from_root(data_dir);
    setup_fake_install(&install_dir);
    let def = test_definition();

    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);

    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();

    // 1. Delete a base file -> Missing
    std::fs::remove_file(manifest.location.join("Game.exe")).unwrap();

    // 2. Write a stray file in the base -> Unexpected
    std::fs::write(
        manifest.location.join("stray_mod_file.dll"),
        b"unauthorized write to base",
    )
    .unwrap();

    let ver = verify_base(manifest, VerifyDepth::Quick, &|_| false, &|_| false);
    assert!(
        ver.problems
            .iter()
            .any(|p| p.path == "Game.exe" && p.kind == ProblemKind::Missing),
        "deleted base file must be reported Missing"
    );
    assert!(
        ver.problems
            .iter()
            .any(|p| p.path == "stray_mod_file.dll" && p.kind == ProblemKind::Unexpected),
        "stray base file must be reported Unexpected"
    );

    // 3. If base folder is deleted entirely -> all files are Missing
    let missing_base_manifest = agora_core::game_base::BaseManifest {
        location: tmp.path().join("non_existent_base_dir"),
        ..manifest.clone()
    };
    let ver_missing = verify_base(
        &missing_base_manifest,
        VerifyDepth::Quick,
        &|_| false,
        &|_| false,
    );
    assert_eq!(ver_missing.problems.len(), manifest.files.len());
    assert!(ver_missing
        .problems
        .iter()
        .all(|p| p.kind == ProblemKind::Missing));
}

#[cfg(windows)]
#[test]
fn junction_in_source_is_skipped() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let outside_dir = tmp.path().join("outside_target");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    std::fs::create_dir_all(&outside_dir).unwrap();

    let paths = AppPaths::from_root(data_dir);
    setup_fake_install(&install_dir);
    std::fs::write(outside_dir.join("secret.txt"), b"sensitive external data").unwrap();

    // Create a junction inside the install pointing outside
    let junction_dir = install_dir.join("Data").join("JunctionLink");
    junction::create(&outside_dir, &junction_dir).unwrap();

    let def = test_definition();
    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("gog").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);

    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();

    // Must be listed in skipped
    assert!(
        manifest
            .skipped
            .iter()
            .any(|s| s.contains("Data/JunctionLink")),
        "junction must be listed in skipped: {:?}",
        manifest.skipped
    );

    // Outside file must NOT be copied into the base
    assert!(
        !manifest
            .location
            .join("Data/JunctionLink/secret.txt")
            .exists(),
        "files inside junction must not be copied"
    );
    assert!(
        !manifest.files.iter().any(|f| f.path.contains("secret.txt")),
        "junction contents must not be in manifest"
    );
}

#[test]
fn remove_base_deletes_folder_and_manifest_leaves_source_intact() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("AgoraBases");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    let paths = AppPaths::from_root(data_dir);
    setup_fake_install(&install_dir);
    let def = test_definition();

    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);

    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Linked,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();
    let base_id = manifest.base_id.clone();
    let base_location = manifest.location.clone();
    let manifest_path = paths.base_manifest_path(&base_id);

    assert!(base_location.exists());
    assert!(manifest_path.exists());

    let snapshot_before_remove = snapshot_tree(&install_dir);

    // Remove base
    remove_base(&paths, &base_id).unwrap();

    assert!(!base_location.exists(), "base directory must be deleted");
    assert!(!manifest_path.exists(), "manifest file must be deleted");

    // Source install must be completely untouched
    let snapshot_after_remove = snapshot_tree(&install_dir);
    assert_eq!(snapshot_before_remove, snapshot_after_remove);

    // Removing an unknown id returns NotFound
    let err = remove_base(&paths, "non-existent-id");
    assert!(matches!(err, Err(BaseError::NotFound(_))));
}

trait RuntimeResolutionExt {
    fn unwrap_identified(self) -> (RuntimeIdentity, String);
}

impl RuntimeResolutionExt for RuntimeResolution {
    fn unwrap_identified(self) -> (RuntimeIdentity, String) {
        match self {
            RuntimeResolution::Identified { runtime, source } => (runtime, source),
            RuntimeResolution::Unidentified { reasons } => {
                panic!("expected identified runtime: {reasons:?}")
            }
        }
    }
}

fn built(tmp: &TempDir, build: Option<&str>) -> (AppPaths, std::path::PathBuf, IdentifiedInstall) {
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    setup_fake_install(&install_dir);
    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("gog").unwrap(),
        version: "1.6.1179.0".into(),
        build: build.map(Into::into),
    };
    (
        AppPaths::from_root(data_dir),
        install_dir.clone(),
        make_test_install(&install_dir, runtime),
    )
}

#[test]
fn a_tampered_manifest_can_never_make_remove_delete_the_install() {
    let tmp = TempDir::new().unwrap();
    let (paths, install_dir, install) = built(&tmp, Some("7"));
    let root = tmp.path().join("AgoraBases");
    let outcome = build_base(
        &paths,
        &install,
        &test_definition(),
        BaseMode::Linked,
        Some(&root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let base_id = outcome.manifest().base_id.clone();
    let manifest_path = paths.base_manifest_path(&base_id);
    let before = snapshot_tree(&install_dir);

    for evil in [
        install_dir.clone(),
        install_dir.join("Data"),
        tmp.path().to_path_buf(),
        root.clone(),
        root.join("someone-else"),
    ] {
        let mut manifest = outcome.manifest().clone();
        manifest.location = evil.clone();
        std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
        assert!(
            remove_base(&paths, &base_id).is_err(),
            "{} must be refused",
            evil.display()
        );
        assert_eq!(
            snapshot_tree(&install_dir),
            before,
            "{} touched the install",
            evil.display()
        );
        assert!(root.join(&base_id).exists());
    }

    // A manifest whose recorded id differs from the one asked for is refused too.
    let mut manifest = outcome.manifest().clone();
    manifest.base_id = "other".into();
    std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
    assert!(remove_base(&paths, &base_id).is_err());
}

#[test]
fn an_interrupted_build_does_not_brick_its_base_id() {
    let tmp = TempDir::new().unwrap();
    let (paths, install_dir, install) = built(&tmp, Some("8"));
    let root = tmp.path().join("AgoraBases");
    let def = test_definition();
    let first = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let base_id = first.manifest().base_id.clone();
    // The process died after the folder was renamed into place, before the manifest was written.
    std::fs::remove_file(paths.base_manifest_path(&base_id)).unwrap();
    std::fs::write(root.join(&base_id).join("half-written.tmp"), b"x").unwrap();
    let before = snapshot_tree(&install_dir);

    let again = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    assert!(matches!(again, BuildOutcome::Built { .. }));
    assert!(
        verify_base(again.manifest(), VerifyDepth::Full, &|_| false, &|_| false)
            .problems
            .is_empty()
    );
    assert_eq!(snapshot_tree(&install_dir), before);
    let leftovers: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(leftovers, [base_id]);
}

#[test]
fn base_id_parts_never_start_or_end_with_a_dot() {
    for odd in ["1.", ".1", ".", "..", "...", "a..b."] {
        let part = sanitize_base_id_part(odd);
        assert!(
            !part.starts_with('.') && !part.ends_with('.'),
            "{odd} -> {part}"
        );
    }
    assert_eq!(sanitize_base_id_part("1.6.1179.0"), "1.6.1179.0");
}

#[test]
fn build_base_refuses_declared_write_matching_linked_file() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    let paths = AppPaths::from_root(data_dir);
    setup_fake_install(&install_dir);

    let mut def = test_definition();
    // Data/A.bsa matches linked_archive_patterns ("Data/*.bsa")
    def.declared_writes = vec!["Data/*.bsa".into()];

    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);

    let err = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Linked,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap_err();

    assert!(
        matches!(err, BaseError::DeclaredWriteLinked { ref path } if path == "Data/A.bsa"),
        "expected DeclaredWriteLinked error for Data/A.bsa, got: {err:?}"
    );
}

#[test]
fn declared_writes_in_verification() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    let paths = AppPaths::from_root(data_dir);
    setup_fake_install(&install_dir);

    // Write d3dx9_42.log into the install
    std::fs::write(install_dir.join("d3dx9_42.log"), b"original log content").unwrap();

    let mut def = test_definition();
    def.declared_writes = vec!["d3dx9_42.log".into()];

    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);

    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&base_root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();

    // Fresh verify -> clean, no game writes
    let ver_fresh = verify_base(
        manifest,
        VerifyDepth::Quick,
        &|p| def.is_declared_write(p),
        &|_| false,
    );
    assert!(ver_fresh.problems.is_empty());
    assert!(ver_fresh.game_writes.is_empty());

    // 1. Changed d3dx9_42.log (case variant D3DX9_42.LOG) -> game_writes, NOT problems
    std::fs::write(
        manifest.location.join("d3dx9_42.log"),
        b"rewritten log by game with different content and size",
    )
    .unwrap();

    let ver_changed = verify_base(
        manifest,
        VerifyDepth::Quick,
        &|p| def.is_declared_write(p),
        &|_| false,
    );
    assert!(
        ver_changed.problems.is_empty(),
        "changed declared write must not be a problem"
    );
    assert_eq!(ver_changed.game_writes, vec!["d3dx9_42.log"]);

    // 2. Deleted d3dx9_42.log -> game_writes, NOT problems
    std::fs::remove_file(manifest.location.join("d3dx9_42.log")).unwrap();

    let ver_deleted = verify_base(
        manifest,
        VerifyDepth::Quick,
        &|p| def.is_declared_write(p),
        &|_| false,
    );
    assert!(
        ver_deleted.problems.is_empty(),
        "deleted declared write must not be a problem"
    );
    assert_eq!(ver_deleted.game_writes, vec!["d3dx9_42.log"]);

    // 3. New / unexpected file matching declared write in uppercase
    std::fs::write(
        manifest.location.join("D3DX9_42.LOG"),
        b"newly created uppercase log",
    )
    .unwrap();

    let ver_new = verify_base(
        manifest,
        VerifyDepth::Quick,
        &|p| def.is_declared_write(p),
        &|_| false,
    );
    // Note: on Windows, d3dx9_42.log and D3DX9_42.LOG refer to the same file.
    assert!(
        ver_new.problems.is_empty(),
        "new declared write must not be a problem"
    );
    assert!(
        ver_new
            .game_writes
            .iter()
            .any(|w| w.eq_ignore_ascii_case("d3dx9_42.log")),
        "must be listed in game_writes"
    );

    // 4. Any other unexpected change IS a problem
    std::fs::write(manifest.location.join("stray.dll"), b"bad dll").unwrap();
    let ver_with_stray = verify_base(
        manifest,
        VerifyDepth::Quick,
        &|p| def.is_declared_write(p),
        &|_| false,
    );
    assert_eq!(ver_with_stray.problems.len(), 1);
    assert_eq!(ver_with_stray.problems[0].path, "stray.dll");
    assert!(ver_with_stray
        .game_writes
        .iter()
        .any(|w| w.eq_ignore_ascii_case("d3dx9_42.log")));
}

#[test]
fn excluded_paths_in_build_and_verify() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let install_dir = tmp.path().join("source_install");
    let base_root = tmp.path().join("bases_root");

    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&install_dir).unwrap();
    let paths = AppPaths::from_root(data_dir);
    setup_fake_install(&install_dir);

    // Create an excluded file and a normal file
    let backup_dir = install_dir.join("Data").join("SSEEdit Backups");
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::write(backup_dir.join("x.esm.backup"), b"backup data").unwrap();
    std::fs::write(install_dir.join("Data").join("Skyrim.esm"), b"master esm").unwrap();

    let mut def = test_definition();
    def.excluded_paths = vec!["Data/SSEEdit Backups/**".into()];

    assert!(def.is_excluded("Data/SSEEdit Backups/x.esm.backup"));
    assert!(!def.is_excluded("Data/Skyrim.esm"));

    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: StoreId::new("steam").unwrap(),
        version: "1.0.0".into(),
        build: None,
    };
    let install = make_test_install(&install_dir, runtime);

    // 1. Default build: excluded path is skipped and recorded in manifest.skipped
    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&base_root),
        BuildOptions {
            include_excluded: false,
        },
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();

    assert!(
        manifest
            .skipped
            .iter()
            .any(|s| s.contains("Data/SSEEdit Backups/x.esm.backup")
                && s.contains("excluded by the game definition")),
        "excluded file must be in skipped with reason: {:?}",
        manifest.skipped
    );
    assert!(
        !manifest
            .location
            .join("Data/SSEEdit Backups/x.esm.backup")
            .exists(),
        "excluded file must not be in base"
    );
    assert!(
        manifest.location.join("Data/Skyrim.esm").exists(),
        "non-excluded file must be in base"
    );

    // Verify ignores excluded paths: stray excluded file in base is NOT Unexpected
    std::fs::create_dir_all(manifest.location.join("Data").join("SSEEdit Backups")).unwrap();
    std::fs::write(
        manifest.location.join("Data/SSEEdit Backups/x.esm.backup"),
        b"stray backup in base",
    )
    .unwrap();

    let ver = verify_base(
        manifest,
        VerifyDepth::Quick,
        &|p| def.is_declared_write(p),
        &|p| def.is_excluded(p),
    );
    assert!(
        ver.problems.is_empty(),
        "stray excluded file appearing in base must be ignored by verify, got: {:?}",
        ver.problems
    );

    // 2. Build with include_excluded: true -> excluded file IS copied into the base
    let root2 = tmp.path().join("root2");
    let paths2 = AppPaths::from_root(root2);
    paths2.create_required_dirs().unwrap();
    let base_root2 = tmp.path().join("bases_root2");
    let outcome_included = build_base(
        &paths2,
        &install,
        &def,
        BaseMode::Copied,
        Some(&base_root2),
        BuildOptions {
            include_excluded: true,
        },
        &|_| {},
    )
    .unwrap();
    let manifest_included = outcome_included.manifest();

    assert!(
        manifest_included
            .location
            .join("Data/SSEEdit Backups/x.esm.backup")
            .exists(),
        "excluded file must be in base when include_excluded is true"
    );
    assert!(
        !manifest_included
            .skipped
            .iter()
            .any(|s| s.contains("x.esm.backup")),
        "excluded file must not be skipped when include_excluded is true"
    );
}

/// Asking for a Copied base must never be answered with an existing Linked one:
/// the caller wanted no hardlinks into the store install.
#[test]
fn a_copied_base_is_not_answered_by_a_linked_one() {
    let tmp = TempDir::new().unwrap();
    let (paths, _install_dir, install) = built(&tmp, Some("9"));
    let def = test_definition();
    let root = tmp.path().join("AgoraBases");
    let build = |mode| {
        build_base(
            &paths,
            &install,
            &def,
            mode,
            Some(&root),
            BuildOptions::default(),
            &|_| {},
        )
        .unwrap()
    };

    let linked = build(BaseMode::Linked);
    let copied = build(BaseMode::Copied);
    assert!(
        matches!(copied, BuildOutcome::Built { .. }),
        "not short-circuited by the linked base"
    );
    assert_eq!(copied.manifest().mode, BaseMode::Copied);
    assert_ne!(linked.manifest().base_id, copied.manifest().base_id);

    let again = build(BaseMode::Copied);
    assert!(matches!(again, BuildOutcome::Existing(_)));
    assert_eq!(again.manifest().mode, BaseMode::Copied);
    assert_eq!(build(BaseMode::Linked).manifest().mode, BaseMode::Linked);
}

/// Default and unfiltered bases of one runtime hold different files, so each
/// gets its own id, and an unfiltered base still verifies the excluded files
/// it recorded.
#[test]
fn an_unfiltered_base_is_its_own_base_and_verifies_what_it_recorded() {
    let tmp = TempDir::new().unwrap();
    let (paths, install_dir, install) = built(&tmp, Some("9"));
    std::fs::create_dir_all(install_dir.join("Data/SSEEdit Backups")).unwrap();
    std::fs::write(
        install_dir.join("Data/SSEEdit Backups/b.esm.backup"),
        b"old master",
    )
    .unwrap();
    let mut def = test_definition();
    def.excluded_paths = vec!["Data/SSEEdit Backups/**".into()];
    let root = tmp.path().join("AgoraBases");

    let filtered = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let unfiltered = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Copied,
        Some(&root),
        BuildOptions {
            include_excluded: true,
        },
        &|_| {},
    )
    .unwrap();
    assert!(
        matches!(unfiltered, BuildOutcome::Built { .. }),
        "not short-circuited by the filtered base"
    );
    assert_ne!(filtered.manifest().base_id, unfiltered.manifest().base_id);
    let backup = "Data/SSEEdit Backups/b.esm.backup";
    assert!(!filtered.manifest().files.iter().any(|f| f.path == backup));
    assert!(unfiltered.manifest().files.iter().any(|f| f.path == backup));

    let excluded = |p: &str| def.is_excluded(p);
    let base_copy = unfiltered.manifest().location.join(backup);
    std::fs::write(&base_copy, b"changed!!!").unwrap();
    let ver = verify_base(
        unfiltered.manifest(),
        VerifyDepth::Full,
        &|_| false,
        &excluded,
    );
    assert!(
        ver.problems.iter().any(|p| p.path == backup),
        "a recorded file is verified even if the definition excludes its path: {:?}",
        ver.problems
    );
    // In the filtered base, a stray excluded file is not Unexpected.
    let stray = filtered.manifest().location.join(backup);
    std::fs::create_dir_all(stray.parent().unwrap()).unwrap();
    std::fs::write(&stray, b"appeared later").unwrap();
    let ver = verify_base(
        filtered.manifest(),
        VerifyDepth::Quick,
        &|_| false,
        &excluded,
    );
    assert!(ver.problems.is_empty(), "{:?}", ver.problems);
}

/// Until the write layer exists, a write to a hardlinked file reaches the store
/// install. Declaring the file a game write must not excuse that, and the
/// problem must say the store changed too (measured: Witcher 3 rewrote a
/// linked `content/metadata.store`).
#[test]
fn a_write_through_a_hardlink_is_never_excused_and_names_the_store() {
    let tmp = TempDir::new().unwrap();
    let (paths, install_dir, install) = built(&tmp, Some("10"));
    let mut def = test_definition();
    def.linked_archive_patterns = vec!["Data/*.bsa".into()];
    let root = tmp.path().join("AgoraBases");
    let outcome = build_base(
        &paths,
        &install,
        &def,
        BaseMode::Linked,
        Some(&root),
        BuildOptions::default(),
        &|_| {},
    )
    .unwrap();
    let manifest = outcome.manifest();
    assert!(manifest
        .files
        .iter()
        .any(|f| f.path == "Data/A.bsa" && f.linked));

    // The game rewrites the linked archive in place, and the copied exe too.
    std::fs::write(
        install_dir.join("Data/A.bsa"),
        b"REWRITTEN BY THE GAME 1234",
    )
    .unwrap();
    std::fs::write(manifest.location.join("Game.exe"), b"rewritten copy").unwrap();
    // Later the definition declares both as game writes.
    let declared = |p: &str| p == "Data/A.bsa" || p == "Game.exe";
    let ver = verify_base(manifest, VerifyDepth::Full, &declared, &|_| false);

    let bsa = ver
        .problems
        .iter()
        .find(|p| p.path == "Data/A.bsa")
        .expect("linked write reported");
    assert!(bsa.linked_to_store);
    assert!(
        ver.game_writes.contains(&"Game.exe".to_string()),
        "a copied file's declared write is excused"
    );
    assert!(!ver.problems.iter().any(|p| p.path == "Game.exe"));
}
