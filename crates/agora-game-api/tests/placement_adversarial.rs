//! Adversarial tests for `suggest_placement`: marker checks must win over unwrapping, and odd
//! input must never panic or place content somewhere surprising.

use agora_game_api::{suggest_placement, ContentLayout, RelPath, Suggestion};

fn skyrim() -> ContentLayout {
    ContentLayout {
        data_path: RelPath::new("Data").unwrap(),
        data_markers: [
            "*.esp",
            "*.esm",
            "*.esl",
            "*.bsa",
            "textures",
            "meshes",
            "scripts",
            "skse",
            "interface",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        root_markers: ["*.exe", "*.dll", "enbseries", "enb*.ini", "reshade-shaders"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    }
}

fn paths(list: &[&str]) -> Vec<RelPath> {
    list.iter().map(|p| RelPath::new(*p).unwrap()).collect()
}

fn place(files: &[&str]) -> (String, String) {
    match suggest_placement(&paths(files), &skyrim()) {
        Suggestion::Place {
            source_path,
            mount_path,
            ..
        } => (
            source_path.as_str().to_string(),
            mount_path.as_str().to_string(),
        ),
        other => panic!("expected Place for {files:?}, got {other:?}"),
    }
}

#[test]
fn a_lone_data_marker_folder_is_not_unwrapped() {
    // `textures/` alone is data-folder content; unwrapping it would put `a.dds` at Data/a.dds.
    assert_eq!(place(&["textures/a.dds"]), ("".into(), "Data".into()));
    assert_eq!(place(&["SKSE/Plugins/x.dll"]), ("".into(), "Data".into()));
}

#[test]
fn a_lone_root_marker_folder_is_not_unwrapped() {
    assert_eq!(place(&["enbseries/effect.fx"]), ("".into(), "".into()));
}

#[test]
fn a_wrapper_around_data_goes_to_the_root() {
    assert_eq!(place(&["MyMod/Data/x.esp"]), ("MyMod".into(), "".into()));
}

#[test]
fn case_is_ignored() {
    assert_eq!(place(&["DATA/X.ESP"]), ("".into(), "".into()));
    assert_eq!(
        place(&["MyMod/TEXTURES/a.dds"]),
        ("MyMod".into(), "Data".into())
    );
}

#[test]
fn a_root_file_beside_a_readme_is_root_content() {
    assert_eq!(
        place(&["skse64_loader.exe", "readme.txt"]),
        ("".into(), "".into())
    );
}

#[test]
fn a_fomod_inside_a_wrapper_is_an_installer() {
    assert!(matches!(
        suggest_placement(
            &paths(&["MyMod/fomod/ModuleConfig.xml", "MyMod/textures/a.dds"]),
            &skyrim()
        ),
        Suggestion::Installer { .. }
    ));
}

#[test]
fn nothing_at_all_is_unknown_not_a_panic() {
    assert!(matches!(
        suggest_placement(&[], &skyrim()),
        Suggestion::Unknown { .. }
    ));
}

#[test]
fn a_wrapper_with_two_folders_and_no_markers_is_unknown() {
    assert!(matches!(
        suggest_placement(
            &paths(&["Wrap/OptionA/x.dds", "Wrap/OptionB/x.dds"]),
            &skyrim()
        ),
        Suggestion::Unknown { .. }
    ));
}

#[test]
fn an_empty_data_path_layout_never_unwraps_into_nothing() {
    let cyberpunk = ContentLayout {
        data_path: RelPath::new("").unwrap(),
        data_markers: vec![],
        root_markers: ["archive", "bin", "r6", "red4ext", "engine", "mods"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };
    match suggest_placement(&paths(&["MyMod v2/archive/pc/mod/x.archive"]), &cyberpunk) {
        Suggestion::Place {
            source_path,
            mount_path,
            ..
        } => {
            assert_eq!(source_path.as_str(), "MyMod v2");
            assert_eq!(mount_path.as_str(), "");
        }
        other => panic!("{other:?}"),
    }
}
