use agora_game_api::{
    extract_thunderstore_package_id, map_thunderstore_bepinex, RelPath, ThunderstoreMappingError,
};

fn p(s: &str) -> RelPath {
    RelPath::new(s).unwrap()
}

fn to_paths(list: &[&str]) -> Vec<RelPath> {
    list.iter().map(|s| p(s)).collect()
}

#[test]
fn test_bepinex_pack_goes_to_root_without_top_level_readme() {
    let files = to_paths(&[
        "manifest.json",
        "icon.png",
        "README.md",
        "CHANGELOG.md",
        "BepInExPack_Valheim/winhttp.dll",
        "BepInExPack_Valheim/doorstop_config.ini",
        "BepInExPack_Valheim/.doorstop_version",
        "BepInExPack_Valheim/BepInEx/core/BepInEx.Preloader.dll",
        "BepInExPack_Valheim/BepInEx/core/BepInEx.dll",
        "BepInExPack_Valheim/BepInEx/config/BepInEx.cfg",
        "BepInExPack_Valheim/start_game_bepinex.sh",
    ]);

    let pkg_id = extract_thunderstore_package_id(
        "denikson-BepInExPack_Valheim-5.4.2351.zip",
        "BepInExPack_Valheim",
        "5.4.2351",
    );
    assert_eq!(pkg_id, "denikson-BepInExPack_Valheim");

    let mappings = map_thunderstore_bepinex(&files, &pkg_id).unwrap();

    let dests: Vec<String> = mappings
        .iter()
        .map(|(_, dest)| dest.as_str().to_string())
        .collect();

    // The pack's folder contents go to root
    assert!(dests.contains(&"winhttp.dll".to_string()));
    assert!(dests.contains(&"doorstop_config.ini".to_string()));
    assert!(dests.contains(&".doorstop_version".to_string()));
    assert!(dests.contains(&"BepInEx/core/BepInEx.Preloader.dll".to_string()));
    assert!(dests.contains(&"BepInEx/core/BepInEx.dll".to_string()));
    assert!(dests.contains(&"BepInEx/config/BepInEx.cfg".to_string()));
    assert!(dests.contains(&"start_game_bepinex.sh".to_string()));

    // Top-level files beside it (README, manifest, icon, changelog) are not installed
    assert!(!dests.contains(&"README.md".to_string()));
    assert!(!dests.contains(&"manifest.json".to_string()));
    assert!(!dests.contains(&"icon.png".to_string()));
    assert!(!dests.contains(&"CHANGELOG.md".to_string()));
    assert!(!dests.iter().any(|d| d.contains("BepInExPack_Valheim")));
}

#[test]
fn test_jotunn_plugins_mapping() {
    let files = to_paths(&[
        "manifest.json",
        "icon.png",
        "README.md",
        "CHANGELOG.md",
        "plugins/Jotunn.dll",
        "plugins/Jotunn.xml",
        "plugins/Jotunn.pdb",
        "plugins/Jotunn.dll.mdb",
    ]);

    let pkg_id =
        extract_thunderstore_package_id("ValheimModding-Jotunn-2.30.2.zip", "Jotunn", "2.30.2");
    assert_eq!(pkg_id, "ValheimModding-Jotunn");

    let mappings = map_thunderstore_bepinex(&files, &pkg_id).unwrap();

    // plugins/Jotunn.dll -> BepInEx/plugins/ValheimModding-Jotunn/Jotunn.dll
    let jotunn_dll = mappings
        .iter()
        .find(|(src, _)| src.as_str() == "plugins/Jotunn.dll")
        .map(|(_, dest)| dest.as_str())
        .unwrap();
    assert_eq!(
        jotunn_dll,
        "BepInEx/plugins/ValheimModding-Jotunn/Jotunn.dll"
    );

    let jotunn_xml = mappings
        .iter()
        .find(|(src, _)| src.as_str() == "plugins/Jotunn.xml")
        .map(|(_, dest)| dest.as_str())
        .unwrap();
    assert_eq!(
        jotunn_xml,
        "BepInEx/plugins/ValheimModding-Jotunn/Jotunn.xml"
    );

    // Metadata at top-level goes into BepInEx/plugins/<pkg>/
    let readme = mappings
        .iter()
        .find(|(src, _)| src.as_str() == "README.md")
        .map(|(_, dest)| dest.as_str())
        .unwrap();
    assert_eq!(readme, "BepInEx/plugins/ValheimModding-Jotunn/README.md");
}

#[test]
fn test_azuclock_top_level_dll() {
    let files = to_paths(&[
        "manifest.json",
        "icon.png",
        "README.md",
        "CHANGELOG.md",
        "AzuClock.dll",
    ]);

    let pkg_id = extract_thunderstore_package_id("Azumatt-AzuClock-1.1.0.zip", "AzuClock", "1.1.0");
    assert_eq!(pkg_id, "Azumatt-AzuClock");

    let mappings = map_thunderstore_bepinex(&files, &pkg_id).unwrap();

    // AzuClock's top-level AzuClock.dll -> BepInEx/plugins/Azumatt-AzuClock/AzuClock.dll
    let azu_dll = mappings
        .iter()
        .find(|(src, _)| src.as_str() == "AzuClock.dll")
        .map(|(_, dest)| dest.as_str())
        .unwrap();
    assert_eq!(azu_dll, "BepInEx/plugins/Azumatt-AzuClock/AzuClock.dll");
}

#[test]
fn test_plant_everything_top_level_dll() {
    let files = to_paths(&[
        "manifest.json",
        "icon.png",
        "README.md",
        "CHANGELOG.md",
        "PlantEverything.dll",
    ]);

    let pkg_id = extract_thunderstore_package_id(
        "Advize-PlantEverything-1.21.3.zip",
        "PlantEverything",
        "1.21.3",
    );
    assert_eq!(pkg_id, "Advize-PlantEverything");

    let mappings = map_thunderstore_bepinex(&files, &pkg_id).unwrap();

    let plant_dll = mappings
        .iter()
        .find(|(src, _)| src.as_str() == "PlantEverything.dll")
        .map(|(_, dest)| dest.as_str())
        .unwrap();
    assert_eq!(
        plant_dll,
        "BepInEx/plugins/Advize-PlantEverything/PlantEverything.dll"
    );
}

#[test]
fn test_config_mapping() {
    let files = to_paths(&["manifest.json", "config/x.cfg"]);

    let mappings = map_thunderstore_bepinex(&files, "SomeAuthor-SomeMod").unwrap();

    // config/x.cfg -> BepInEx/config/x.cfg
    let cfg = mappings
        .iter()
        .find(|(src, _)| src.as_str() == "config/x.cfg")
        .map(|(_, dest)| dest.as_str())
        .unwrap();
    assert_eq!(cfg, "BepInEx/config/x.cfg");
}

#[test]
fn test_case_plugins_mapping() {
    let files = to_paths(&["manifest.json", "Plugins/Jotunn.dll"]);

    let mappings = map_thunderstore_bepinex(&files, "ValheimModding-Jotunn").unwrap();

    // Plugins/ (capital P) -> BepInEx/plugins/<pkg>/Jotunn.dll
    let dll = mappings
        .iter()
        .find(|(src, _)| src.as_str() == "Plugins/Jotunn.dll")
        .map(|(_, dest)| dest.as_str())
        .unwrap();
    assert_eq!(dll, "BepInEx/plugins/ValheimModding-Jotunn/Jotunn.dll");
}

#[test]
fn test_no_manifest_is_not_a_package() {
    // Missing manifest.json in paths is NotPackage
    let files_no_manifest = to_paths(&["plugins/Mod.dll"]);
    assert_eq!(
        map_thunderstore_bepinex(&files_no_manifest, "Author-Mod"),
        Err(ThunderstoreMappingError::NotPackage)
    );
}

#[test]
fn a_file_name_namespace_that_is_not_a_thunderstore_name_is_dropped() {
    assert_eq!(
        extract_thunderstore_package_id("Author-Mod-1.0.0.zip", "Mod", "1.0.0"),
        "Author-Mod"
    );
    for bad in ["..-Mod-1.0.0.zip", "a b-Mod-1.0.0.zip", "x/y-Mod-1.0.0.zip"] {
        assert_eq!(
            extract_thunderstore_package_id(bad, "Mod", "1.0.0"),
            "Mod",
            "{bad}"
        );
    }
}

#[test]
fn a_pack_without_a_wrapper_folder_goes_to_the_root() {
    let paths = to_paths(&[
        "manifest.json",
        "icon.png",
        "README.md",
        "winhttp.dll",
        "BepInEx/core/BepInEx.Preloader.dll",
        "BepInEx/config/BepInEx.cfg",
    ]);
    let mut got: Vec<(String, String)> = map_thunderstore_bepinex(&paths, "denikson-BepInExPack")
        .unwrap()
        .into_iter()
        .map(|(s, d)| (s.as_str().to_string(), d.as_str().to_string()))
        .collect();
    got.sort();
    let want: Vec<(String, String)> = [
        "BepInEx/config/BepInEx.cfg",
        "BepInEx/core/BepInEx.Preloader.dll",
        "winhttp.dll",
    ]
    .iter()
    .map(|p| (p.to_string(), p.to_string()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn a_package_id_that_could_leave_its_folder_is_refused() {
    let paths = to_paths(&["manifest.json", "Mod.dll"]);
    for bad in ["..", "../x", "a/b", "", "a-b-c", "x y", "-Mod"] {
        assert!(
            matches!(
                map_thunderstore_bepinex(&paths, bad),
                Err(ThunderstoreMappingError::InvalidPath(_))
            ),
            "{bad:?}"
        );
    }
    assert!(map_thunderstore_bepinex(&paths, "Author-Mod").is_ok());
    assert!(map_thunderstore_bepinex(&paths, "Mod").is_ok());
}
