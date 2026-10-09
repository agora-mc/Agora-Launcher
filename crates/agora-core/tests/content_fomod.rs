//! FOMOD installer tests (MASTER_SPEC §26.6): parsing, evaluation, defaults and derivation.
//!
//! Every fixture is an archive built here: a wrapper folder, `fomod/ModuleConfig.xml` (UTF-16 LE
//! with a byte order mark unless a test says otherwise) and the content folders it names.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use agora_core::content_fomod::{
    self as fomod, Choice, Condition, FileState, FomodContext, FomodError,
};
use agora_core::content_store::{
    add_archive, derive_item, get_item, remove_item, verify_item, AddOutcome, ContentError,
    ContentSource, VerifyDepth,
};
use agora_core::ctx::Ctx;
use agora_game_api::RelPath;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn create_zip(path: &Path, entries: &[(String, Vec<u8>)]) {
    let file = File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in entries {
        zip.start_file(name.as_str(), options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap();
}

fn utf16le_bom(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    out.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
    out
}

const CONTENT: &[(&str, &[u8])] = &[
    ("Core/CoolMod.esp", b"core esp"),
    ("Core/textures/a.dds", b"a"),
    ("Core/textures/sub/b.dds", b"b"),
    ("Heavy/meshes/armor.nif", b"heavy armor"),
    ("Light/meshes/armor.nif", b"light armor"),
    ("Extras/glow.nif", b"glow"),
    ("Extras/spikes.nif", b"spikes"),
    ("Patches/Heavy.esp", b"patch heavy"),
    ("Patches/Light.esp", b"patch light"),
    ("fomod/images/heavy.png", b"png"),
];

/// A config as the archive's `fomod/ModuleConfig.xml` bytes, inside a wrapper folder.
fn store_archive(ctx: &Ctx, work: &Path, config: Vec<u8>, extra: &[(&str, &[u8])]) -> String {
    let mut entries: Vec<(String, Vec<u8>)> = CONTENT
        .iter()
        .chain(extra.iter())
        .map(|(p, c)| (format!("Cool Mod 1.0/{p}"), c.to_vec()))
        .collect();
    entries.push(("Cool Mod 1.0/fomod/ModuleConfig.xml".to_string(), config));
    let zip_path = work.join(format!("cool-{}.zip", entries.len()));
    create_zip(&zip_path, &entries);
    add_archive(ctx, &zip_path, Some("Cool Mod 1.0"))
        .unwrap()
        .item()
        .item_id
        .clone()
}

struct Fx {
    _tmp: TempDir,
    work: TempDir,
    ctx: Ctx,
}

impl Fx {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = Ctx::for_testing(tmp.path().join("data"));
        Fx {
            _tmp: tmp,
            work: tempfile::tempdir().unwrap(),
            ctx,
        }
    }

    fn item(&self, config: &str) -> String {
        store_archive(&self.ctx, self.work.path(), utf16le_bom(config), &[])
    }

    fn installer(&self, config: &str) -> fomod::FomodInstaller {
        let id = self.item(config);
        fomod::parse(&self.ctx, &id).unwrap()
    }
}

const HEADER: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:noNamespaceSchemaLocation="http://qconsulting.ca/fo3/ModConfig5.0.xsd">"#;

/// The installer used by most tests.
fn main_config() -> String {
    format!(
        r#"{HEADER}
  <moduleName>Cool Mod</moduleName>
  <requiredInstallFiles>
    <file source="Core\CoolMod.esp" destination="CoolMod.esp"/>
    <folder source="Core\textures" destination="textures"/>
  </requiredInstallFiles>
  <installSteps order="Explicit">
    <installStep name="Options">
      <optionalFileGroups order="Explicit">
        <group name="Armor" type="SelectExactlyOne">
          <plugins order="Explicit">
            <plugin name="Heavy">
              <description>Heavy armor</description>
              <image path="fomod\images\heavy.png"/>
              <files><folder source="Heavy" destination="" priority="0"/></files>
              <conditionFlags><flag name="armor">heavy</flag></conditionFlags>
              <typeDescriptor><type name="Recommended"/></typeDescriptor>
            </plugin>
            <plugin name="Light">
              <description>Light armor</description>
              <files><folder source="Light" destination=""/></files>
              <conditionFlags><flag name="armor">light</flag></conditionFlags>
              <typeDescriptor><type name="Optional"/></typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
    <installStep name="Heavy Extras">
      <visible><dependencies operator="And"><flagDependency flag="armor" value="heavy"/></dependencies></visible>
      <optionalFileGroups>
        <group name="Extras" type="SelectAny">
          <plugins>
            <plugin name="Glow">
              <files><file source="Extras\glow.nif" destination="meshes\glow.nif"/></files>
              <typeDescriptor><type name="Optional"/></typeDescriptor>
            </plugin>
            <plugin name="Spikes">
              <files><file source="Extras\spikes.nif" destination="meshes\spikes.nif"/></files>
              <typeDescriptor><type name="Optional"/></typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
  </installSteps>
  <conditionalFileInstalls>
    <patterns>
      <pattern>
        <dependencies operator="And"><flagDependency flag="armor" value="heavy"/></dependencies>
        <files><file source="Patches\Heavy.esp" destination="Heavy.esp"/></files>
      </pattern>
      <pattern>
        <dependencies operator="And"><flagDependency flag="armor" value="light"/></dependencies>
        <files><file source="Patches\Light.esp" destination="Light.esp"/></files>
      </pattern>
    </patterns>
  </conditionalFileInstalls>
</config>"#
    )
}

fn pick(step: &str, group: &str, plugins: &[&str]) -> Choice {
    Choice {
        step: step.into(),
        group: group.into(),
        plugins: plugins.iter().map(|p| p.to_string()).collect(),
    }
}

fn dests(plan: &fomod::InstallPlan) -> Vec<String> {
    plan.files
        .iter()
        .map(|f| f.destination.as_str().to_string())
        .collect()
}

fn object_count(ctx: &Ctx) -> usize {
    let mut n = 0;
    for shard in std::fs::read_dir(ctx.paths.content_objects_dir()).unwrap() {
        n += std::fs::read_dir(shard.unwrap().path()).unwrap().count();
    }
    n
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[test]
fn parses_the_whole_model() {
    let fx = Fx::new();
    let inst = fx.installer(&main_config());
    assert_eq!(inst.module_name, "Cool Mod");
    assert_eq!(inst.root, "Cool Mod 1.0");
    assert_eq!(inst.required_files.len(), 2);
    assert_eq!(inst.required_files[1].kind, fomod::EntryKind::Folder);
    assert_eq!(inst.required_files[1].source, "Core/textures");
    assert_eq!(inst.steps.len(), 2);
    let heavy = &inst.steps[0].groups[0].plugins[0];
    assert_eq!(heavy.name, "Heavy");
    assert_eq!(heavy.description, "Heavy armor");
    assert_eq!(heavy.image.as_deref(), Some("fomod/images/heavy.png"));
    assert_eq!(heavy.flags[0].name, "armor");
    assert_eq!(heavy.flags[0].value, "heavy");
    assert_eq!(
        heavy.type_descriptor,
        fomod::TypeDescriptor::Simple {
            plugin_type: fomod::PluginType::Recommended
        }
    );
    assert_eq!(
        inst.steps[0].groups[0].group_type,
        fomod::GroupType::SelectExactlyOne
    );
    assert!(inst.steps[0].visible.is_none());
    assert_eq!(
        inst.steps[1].visible.as_ref().unwrap().describe(),
        "option flag 'armor' is 'heavy'"
    );
    assert_eq!(inst.conditional_installs.len(), 2);
}

#[test]
fn utf16_and_utf8_configs_give_the_same_model() {
    let fx = Fx::new();
    let config = main_config();
    let id16 = store_archive(&fx.ctx, fx.work.path(), utf16le_bom(&config), &[]);
    let mut bom8 = vec![0xEF, 0xBB, 0xBF];
    bom8.extend(config.replace("UTF-16", "UTF-8").as_bytes());
    let id8 = store_archive(&fx.ctx, fx.work.path(), bom8, &[]);
    // A UTF-8 file that still claims UTF-16 in its declaration, with no byte order mark.
    let id_liar = store_archive(&fx.ctx, fx.work.path(), config.as_bytes().to_vec(), &[]);
    assert_ne!(id16, id8);
    assert_ne!(id16, id_liar);

    let json = |id: &str| {
        let mut v = serde_json::to_value(fomod::parse(&fx.ctx, id).unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("item_id");
        v
    };
    assert_eq!(json(&id16), json(&id8));
    assert_eq!(json(&id16), json(&id_liar));
}

#[test]
fn order_attribute_sorts_steps_groups_and_plugins() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}
<installSteps order="Descending">
  <installStep name="b step"><optionalFileGroups order="Ascending">
    <group name="Zed" type="SelectAny"><plugins order="Descending">
      <plugin name="a"/><plugin name="C"/><plugin name="b"/>
    </plugins></group>
    <group name="alpha" type="SelectAny"><plugins order="Explicit">
      <plugin name="z"/><plugin name="y"/>
    </plugins></group>
  </optionalFileGroups></installStep>
  <installStep name="A step"><optionalFileGroups/></installStep>
  <installStep name="c step"><optionalFileGroups/></installStep>
</installSteps></config>"#
    );
    let inst = fx.installer(&xml);
    let steps: Vec<_> = inst.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(steps, ["c step", "b step", "A step"]);
    let groups: Vec<_> = inst.steps[1]
        .groups
        .iter()
        .map(|g| g.name.as_str())
        .collect();
    assert_eq!(groups, ["alpha", "Zed"]);
    let plugins = |g: usize| -> Vec<String> {
        inst.steps[1].groups[g]
            .plugins
            .iter()
            .map(|p| p.name.clone())
            .collect()
    };
    assert_eq!(plugins(0), ["z", "y"]);
    assert_eq!(plugins(1), ["C", "b", "a"]);
}

#[test]
fn nested_dependencies_and_all_condition_kinds_parse() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}
<moduleDependencies operator="And">
  <gameDependency version="1.5.97.0"/>
  <fileDependency file="Data\Skyrim.esm" state="Active"/>
  <dependencies operator="Or">
    <fileDependency file="SkyUI_SE.esp" state="Active"/>
    <flagDependency flag="x" value="On"/>
    <dependencies operator="And"><fommDependency version="1.0"/></dependencies>
  </dependencies>
</moduleDependencies>
<installSteps><installStep name="S">
  <visible><flagDependency flag="a" value="1"/></visible>
  <optionalFileGroups><group name="G" type="SelectAny"><plugins>
    <plugin name="P"><typeDescriptor><dependencyType>
      <defaultType name="Optional"/>
      <patterns><pattern>
        <dependencies operator="Or"><fileDependency file="SkyUI_SE.esp" state="Missing"/></dependencies>
        <type name="NotUsable"/>
      </pattern></patterns>
    </dependencyType></typeDescriptor></plugin>
  </plugins></group></optionalFileGroups>
</installStep></installSteps></config>"#
    );
    let inst = fx.installer(&xml);
    let deps = inst.module_dependencies.clone().unwrap();
    let Condition::All { conditions } = &deps else {
        panic!("expected And");
    };
    assert_eq!(conditions.len(), 3);
    assert!(
        matches!(&conditions[0], Condition::Version { kind, version }
        if kind == "game" && version == "1.5.97.0")
    );
    assert!(matches!(&conditions[1], Condition::File { file, state }
        if file == "Data/Skyrim.esm" && *state == FileState::Active));
    let Condition::Any { conditions: inner } = &conditions[2] else {
        panic!("expected nested Or");
    };
    assert_eq!(inner.len(), 3);
    assert!(matches!(&inner[2], Condition::All { conditions } if conditions.len() == 1));
    // `visible` holding its dependencies directly, without a <dependencies> wrapper.
    assert_eq!(
        inst.steps[0].visible,
        Some(Condition::All {
            conditions: vec![Condition::Flag {
                flag: "a".into(),
                value: "1".into()
            }]
        })
    );
    let fomod::TypeDescriptor::Dependent { default, patterns } =
        &inst.steps[0].groups[0].plugins[0].type_descriptor
    else {
        panic!("expected a dependent type");
    };
    assert_eq!(*default, fomod::PluginType::Optional);
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns[0].1, fomod::PluginType::NotUsable);
    assert!(deps.describe().contains("any of"));
}

fn expect_parse_err(config: &str) -> FomodError {
    let fx = Fx::new();
    let id = fx.item(config);
    fomod::parse(&fx.ctx, &id).unwrap_err()
}

#[test]
fn a_source_that_escapes_the_root_is_refused() {
    for (element, attr) in [
        (
            r#"<file source="..\..\evil.dll" destination="x.dll"/>"#,
            "evil",
        ),
        (r#"<folder source="Core\..\..\"/>"#, "source"),
        (
            r#"<file source="Core\CoolMod.esp" destination="..\..\evil.esp"/>"#,
            "evil",
        ),
        (
            r#"<file source="C:\Windows\x.dll" destination="x.dll"/>"#,
            "C:",
        ),
    ] {
        let xml =
            format!("{HEADER}<requiredInstallFiles>{element}</requiredInstallFiles></config>");
        let err = expect_parse_err(&xml);
        let FomodError::UnsafePath { element: el, .. } = &err else {
            panic!("{element}: expected UnsafePath, got {err}");
        };
        assert!(el.starts_with('<'), "the error names the element: {err}");
        assert!(err.to_string().contains(attr) || el.contains(attr), "{err}");
    }
    // An image path counts too.
    let xml = format!(
        r#"{HEADER}<installSteps><installStep name="S"><optionalFileGroups>
        <group name="G" type="SelectAny"><plugins><plugin name="P">
        <image path="..\..\secret.png"/></plugin></plugins></group>
        </optionalFileGroups></installStep></installSteps></config>"#
    );
    assert!(matches!(
        expect_parse_err(&xml),
        FomodError::UnsafePath { .. }
    ));
}

#[test]
fn an_unknown_group_type_is_refused_by_name() {
    let xml = format!(
        r#"{HEADER}<installSteps><installStep name="S"><optionalFileGroups>
        <group name="Armor" type="SelectEverything"><plugins/></group>
        </optionalFileGroups></installStep></installSteps></config>"#
    );
    let err = expect_parse_err(&xml);
    let FomodError::Invalid { element, reason } = &err else {
        panic!("expected Invalid, got {err}");
    };
    assert!(
        element.contains("group") && element.contains("Armor"),
        "{err}"
    );
    assert!(reason.contains("SelectEverything"), "{err}");
}

#[test]
fn unknown_plugin_types_states_and_duplicate_names_are_refused() {
    let bad_type = format!(
        r#"{HEADER}<installSteps><installStep name="S"><optionalFileGroups>
        <group name="G" type="SelectAny"><plugins><plugin name="P">
        <typeDescriptor><type name="Maybe"/></typeDescriptor></plugin></plugins></group>
        </optionalFileGroups></installStep></installSteps></config>"#
    );
    assert!(matches!(
        expect_parse_err(&bad_type),
        FomodError::Invalid { .. }
    ));

    let bad_state = format!(
        r#"{HEADER}<installSteps><installStep name="S">
        <visible><fileDependency file="a.esp" state="Sleeping"/></visible>
        <optionalFileGroups/></installStep></installSteps></config>"#
    );
    assert!(matches!(
        expect_parse_err(&bad_state),
        FomodError::Invalid { .. }
    ));

    let dup = format!(
        r#"{HEADER}<installSteps><installStep name="S"><optionalFileGroups>
        <group name="G" type="SelectAny"><plugins><plugin name="P"/><plugin name="p"/></plugins></group>
        </optionalFileGroups></installStep></installSteps></config>"#
    );
    assert!(matches!(expect_parse_err(&dup), FomodError::Invalid { .. }));

    assert!(matches!(
        expect_parse_err(&format!(
            "{HEADER}<requiredInstallFiles><thing/></requiredInstallFiles></config>"
        )),
        FomodError::Invalid { .. }
    ));
    assert!(matches!(
        expect_parse_err("<notconfig/>"),
        FomodError::Invalid { .. }
    ));
    assert!(matches!(expect_parse_err("<config>"), FomodError::Xml(_)));
}

#[test]
fn an_oversized_document_is_refused() {
    let fx = Fx::new();
    let padding = "x".repeat(fomod::MAX_DOCUMENT_BYTES + 1);
    let xml = format!("<config><!-- {padding} --></config>");
    let id = store_archive(&fx.ctx, fx.work.path(), xml.into_bytes(), &[]);
    let err = fomod::parse(&fx.ctx, &id).unwrap_err();
    assert!(matches!(err, FomodError::TooLarge { .. }), "{err}");
}

#[test]
fn excessive_nesting_is_refused() {
    let ok = format!(
        "<config>{}{}</config>",
        "<dependencies>".repeat(fomod::MAX_DEPTH - 2),
        "</dependencies>".repeat(fomod::MAX_DEPTH - 2)
    );
    let fx = Fx::new();
    let id = store_archive(&fx.ctx, fx.work.path(), ok.into_bytes(), &[]);
    fomod::parse(&fx.ctx, &id).expect("nesting at the limit is accepted");

    let deep = format!(
        "<config><moduleDependencies>{}{}</moduleDependencies></config>",
        "<dependencies>".repeat(fomod::MAX_DEPTH),
        "</dependencies>".repeat(fomod::MAX_DEPTH)
    );
    assert!(matches!(
        expect_parse_err(&deep),
        FomodError::TooDeep { .. }
    ));
}

#[test]
fn a_doctype_and_huge_counts_are_refused() {
    let doctype =
        r#"<!DOCTYPE config [<!ENTITY a "aaaa"><!ENTITY b "&a;&a;&a;&a;">]><config>&b;</config>"#;
    assert!(matches!(expect_parse_err(doctype), FomodError::Xml(_)));

    let steps: String = (0..300)
        .map(|i| format!(r#"<installStep name="s{i}"><optionalFileGroups/></installStep>"#))
        .collect();
    let xml = format!("{HEADER}<installSteps>{steps}</installSteps></config>");
    assert!(matches!(expect_parse_err(&xml), FomodError::TooMany { .. }));
}

#[test]
fn an_archive_without_an_installer_says_so() {
    let fx = Fx::new();
    let zip_path = fx.work.path().join("plain.zip");
    create_zip(&zip_path, &[("Data/a.esp".to_string(), b"a".to_vec())]);
    let id = add_archive(&fx.ctx, &zip_path, None)
        .unwrap()
        .item()
        .item_id
        .clone();
    assert!(matches!(
        fomod::parse(&fx.ctx, &id).unwrap_err(),
        FomodError::NoInstaller(_)
    ));
}

#[test]
fn the_installer_at_the_top_of_an_archive_is_found_too() {
    let fx = Fx::new();
    let zip_path = fx.work.path().join("flat.zip");
    create_zip(
        &zip_path,
        &[
            (
                "FOMOD/moduleconfig.XML".to_string(),
                main_config().into_bytes(),
            ),
            ("Core/CoolMod.esp".to_string(), b"x".to_vec()),
            (
                "Deeper/fomod/ModuleConfig.xml".to_string(),
                b"<config/>".to_vec(),
            ),
        ],
    );
    let id = add_archive(&fx.ctx, &zip_path, None)
        .unwrap()
        .item()
        .item_id
        .clone();
    let inst = fomod::parse(&fx.ctx, &id).unwrap();
    assert_eq!(inst.root, "");
    assert_eq!(inst.module_name, "Cool Mod");
}

#[test]
fn info_xml_is_read_when_present() {
    let fx = Fx::new();
    let info = r#"<fomod><Name>Cool</Name><Author>Someone</Author><Version MachineVersion="1.0">1.0</Version><Website>https://example.com</Website></fomod>"#;
    let id = store_archive(
        &fx.ctx,
        fx.work.path(),
        utf16le_bom(&main_config()),
        &[("fomod/info.xml", info.as_bytes())],
    );
    let inst = fomod::parse(&fx.ctx, &id).unwrap();
    let info = inst.info.unwrap();
    assert_eq!(info.author.as_deref(), Some("Someone"));
    assert_eq!(info.version.as_deref(), Some("1.0"));
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

#[test]
fn required_files_are_always_installed_and_folders_recurse() {
    let fx = Fx::new();
    let inst = fx.installer(&main_config());
    let plan = fomod::evaluate(&inst, &[pick("Options", "Armor", &["Light"])], None).unwrap();
    assert_eq!(
        dests(&plan),
        [
            "CoolMod.esp",
            "Light.esp",
            "meshes/armor.nif",
            "textures/a.dds",
            "textures/sub/b.dds"
        ]
    );
    // Each planned file names the archive file it comes from.
    let armor = plan
        .files
        .iter()
        .find(|f| f.destination.as_str() == "meshes/armor.nif")
        .unwrap();
    assert_eq!(armor.source.as_str(), "Light/meshes/armor.nif");
    assert_eq!(armor.size, b"light armor".len() as u64);
}

#[test]
fn a_config_with_only_required_files_needs_no_choices() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<requiredInstallFiles>
        <file source="core\COOLMOD.ESP" destination="x.esp"/>
        <folder source="CORE/Textures/" destination="tex\"/>
        </requiredInstallFiles></config>"#
    );
    let plan = fomod::evaluate(&fx.installer(&xml), &[], None).unwrap();
    assert_eq!(dests(&plan), ["tex/a.dds", "tex/sub/b.dds", "x.esp"]);
}

#[test]
fn select_exactly_one_refuses_zero_and_two() {
    let fx = Fx::new();
    let inst = fx.installer(&main_config());
    let none = fomod::evaluate(&inst, &[], None).unwrap_err();
    let two = fomod::evaluate(
        &inst,
        &[pick("Options", "Armor", &["Heavy", "Light"])],
        None,
    )
    .unwrap_err();
    for (err, chosen) in [(none, 0), (two, 2)] {
        let FomodError::GroupRule {
            step,
            group,
            rule,
            chosen: n,
        } = &err
        else {
            panic!("expected GroupRule, got {err}");
        };
        assert_eq!((step.as_str(), group.as_str()), ("Options", "Armor"));
        assert_eq!(*rule, "select exactly one");
        assert_eq!(*n, chosen);
        assert!(err.to_string().contains("Options/Armor"));
    }
}

#[test]
fn the_other_group_types_are_enforced() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<installSteps><installStep name="S"><optionalFileGroups>
        <group name="AtMost" type="SelectAtMostOne"><plugins><plugin name="a"/><plugin name="b"/></plugins></group>
        <group name="AtLeast" type="SelectAtLeastOne"><plugins><plugin name="a"/><plugin name="b"/></plugins></group>
        <group name="All" type="SelectAll"><plugins>
          <plugin name="a"><files><file source="Extras\glow.nif"/></files></plugin>
          <plugin name="b"><files><file source="Extras\spikes.nif"/></files></plugin>
        </plugins></group>
        </optionalFileGroups></installStep></installSteps></config>"#
    );
    let inst = fx.installer(&xml);
    let ok = [pick("S", "AtLeast", &["a"])];
    let plan = fomod::evaluate(&inst, &ok, None).unwrap();
    // SelectAll installs everything without being told, and the choices record it.
    assert_eq!(dests(&plan), ["Extras/glow.nif", "Extras/spikes.nif"]);
    assert!(plan.choices.contains(&pick("S", "All", &["a", "b"])));

    let too_many = [
        pick("S", "AtLeast", &["a"]),
        pick("S", "AtMost", &["a", "b"]),
    ];
    assert!(matches!(
        fomod::evaluate(&inst, &too_many, None).unwrap_err(),
        FomodError::GroupRule {
            rule: "select at most one",
            chosen: 2,
            ..
        }
    ));
    assert!(matches!(
        fomod::evaluate(&inst, &[], None).unwrap_err(),
        FomodError::GroupRule {
            rule: "select at least one",
            chosen: 0,
            ..
        }
    ));
}

#[test]
fn unknown_names_are_errors() {
    let fx = Fx::new();
    let inst = fx.installer(&main_config());
    assert!(matches!(
        fomod::evaluate(&inst, &[pick("Nope", "Armor", &["Heavy"])], None).unwrap_err(),
        FomodError::UnknownStep(_)
    ));
    assert!(matches!(
        fomod::evaluate(&inst, &[pick("Options", "Nope", &["Heavy"])], None).unwrap_err(),
        FomodError::UnknownGroup { .. }
    ));
    assert!(matches!(
        fomod::evaluate(&inst, &[pick("Options", "Armor", &["Nope"])], None).unwrap_err(),
        FomodError::UnknownPlugin { .. }
    ));
    // Names match case-insensitively, and the plan records the canonical ones.
    let plan = fomod::evaluate(&inst, &[pick("options", "ARMOR", &["heavy"])], None).unwrap();
    assert_eq!(plan.choices, [pick("Options", "Armor", &["Heavy"])]);
}

#[test]
fn a_hidden_step_is_skipped_and_its_choices_ignored_with_a_note() {
    let fx = Fx::new();
    let inst = fx.installer(&main_config());
    let choices = [
        pick("Options", "Armor", &["Light"]),
        pick("Heavy Extras", "Extras", &["Glow"]),
    ];
    let plan = fomod::evaluate(&inst, &choices, None).unwrap();
    assert!(!dests(&plan).contains(&"meshes/glow.nif".to_string()));
    assert!(plan
        .notes
        .iter()
        .any(|n| n.contains("Heavy Extras") && n.contains("hidden")));
    assert!(plan.choices.iter().all(|c| c.step != "Heavy Extras"));

    // With heavy armor the step shows and the choice counts.
    let choices = [
        pick("Options", "Armor", &["Heavy"]),
        pick("Heavy Extras", "Extras", &["Glow"]),
    ];
    let plan = fomod::evaluate(&inst, &choices, None).unwrap();
    assert!(dests(&plan).contains(&"meshes/glow.nif".to_string()));
    assert!(!dests(&plan).contains(&"meshes/spikes.nif".to_string()));
}

#[test]
fn a_conditional_install_fires_only_when_its_flags_match() {
    let fx = Fx::new();
    let inst = fx.installer(&main_config());
    let heavy = fomod::evaluate(&inst, &[pick("Options", "Armor", &["Heavy"])], None).unwrap();
    let light = fomod::evaluate(&inst, &[pick("Options", "Armor", &["Light"])], None).unwrap();
    assert!(dests(&heavy).contains(&"Heavy.esp".to_string()));
    assert!(!dests(&heavy).contains(&"Light.esp".to_string()));
    assert!(dests(&light).contains(&"Light.esp".to_string()));
    assert!(!dests(&light).contains(&"Heavy.esp".to_string()));
}

#[test]
fn priority_decides_a_destination_conflict_and_install_order_breaks_ties() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<installSteps><installStep name="S"><optionalFileGroups>
        <group name="G" type="SelectAny"><plugins>
          <plugin name="High"><files><file source="Extras\glow.nif" destination="x.nif" priority="5"/></files></plugin>
          <plugin name="Low"><files><file source="Extras\spikes.nif" destination="X.NIF" priority="1"/></files></plugin>
          <plugin name="TieA"><files><file source="Heavy\meshes\armor.nif" destination="t.nif"/></files></plugin>
          <plugin name="TieB"><files><file source="Light\meshes\armor.nif" destination="t.nif"/></files></plugin>
        </plugins></group></optionalFileGroups></installStep></installSteps></config>"#
    );
    let inst = fx.installer(&xml);
    let plan = fomod::evaluate(
        &inst,
        &[pick("S", "G", &["High", "Low", "TieA", "TieB"])],
        None,
    )
    .unwrap();
    assert_eq!(plan.files.len(), 2, "{:?}", dests(&plan));
    let by_dest = |d: &str| {
        plan.files
            .iter()
            .find(|f| f.destination.as_str().eq_ignore_ascii_case(d))
            .unwrap()
    };
    // The higher priority wins even though the other comes later; the case of the destination
    // does not make two files.
    assert_eq!(by_dest("x.nif").source.as_str(), "Extras/glow.nif");
    // Equal priorities: the later plugin wins.
    assert_eq!(by_dest("t.nif").source.as_str(), "Light/meshes/armor.nif");
    assert!(plan.notes.iter().any(|n| n.contains("wins")));

    // Choosing only TieA and Low changes nothing but who is left.
    let plan = fomod::evaluate(&inst, &[pick("S", "G", &["TieB", "TieA"])], None).unwrap();
    assert_eq!(
        plan.files[0].source.as_str(),
        "Light/meshes/armor.nif",
        "install order is the config's order, not the order of the choices"
    );
}

#[test]
fn file_dependencies_fail_without_context_and_follow_it_with_one() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<installSteps>
        <installStep name="Compat">
          <visible><fileDependency file="SkyUI_SE.esp" state="Active"/></visible>
          <optionalFileGroups><group name="G" type="SelectAny"><plugins>
            <plugin name="UI"><files><file source="Extras\glow.nif" destination="ui.nif"/></files></plugin>
          </plugins></group></optionalFileGroups>
        </installStep>
        <installStep name="Gone">
          <visible><fileDependency file="data\Old.esp" state="Missing"/></visible>
          <optionalFileGroups><group name="H" type="SelectAny"><plugins>
            <plugin name="X"><files><file source="Extras\spikes.nif" destination="x.nif"/></files></plugin>
          </plugins></group></optionalFileGroups>
        </installStep>
        </installSteps></config>"#
    );
    let inst = fx.installer(&xml);
    let choices = [pick("Compat", "G", &["UI"]), pick("Gone", "H", &["X"])];

    // No context: every fileDependency fails (even "Missing"), and the plan says why.
    let plan = fomod::evaluate(&inst, &choices, None).unwrap();
    assert!(plan.files.is_empty());
    assert!(plan
        .notes
        .iter()
        .any(|n| n.contains("SkyUI_SE.esp") && n.contains("cannot be known")));

    let mut context = FomodContext::from_active(["skyui_se.ESP"]);
    context.set("data/Old.esp", FileState::Inactive);
    let plan = fomod::evaluate(&inst, &choices, Some(&context)).unwrap();
    assert_eq!(dests(&plan), ["ui.nif"]);

    let empty = FomodContext::new();
    let plan = fomod::evaluate(&inst, &choices, Some(&empty)).unwrap();
    assert_eq!(dests(&plan), ["x.nif"]);
}

#[test]
fn version_dependencies_count_as_satisfied_and_say_so() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<moduleDependencies><gameDependency version="9.9"/></moduleDependencies>
        <installSteps><installStep name="S">
          <visible><fommDependency version="1.0"/></visible>
          <optionalFileGroups><group name="G" type="SelectAny"><plugins>
            <plugin name="P"><files><file source="Extras\glow.nif"/></files></plugin>
          </plugins></group></optionalFileGroups></installStep></installSteps></config>"#
    );
    let inst = fx.installer(&xml);
    let plan = fomod::evaluate(&inst, &[pick("S", "G", &["P"])], None).unwrap();
    assert_eq!(dests(&plan), ["Extras/glow.nif"]);
    assert!(plan
        .notes
        .iter()
        .any(|n| n.contains("game version 9.9") && n.contains("satisfied")));
}

#[test]
fn not_usable_plugins_cannot_be_chosen_and_required_ones_always_are() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<installSteps><installStep name="S"><optionalFileGroups>
        <group name="G" type="SelectAny"><plugins>
          <plugin name="Dead"><files><file source="Extras\glow.nif"/></files>
            <typeDescriptor><type name="NotUsable"/></typeDescriptor></plugin>
          <plugin name="Must"><files><file source="Extras\spikes.nif"/></files>
            <typeDescriptor><type name="Required"/></typeDescriptor></plugin>
        </plugins></group></optionalFileGroups></installStep></installSteps></config>"#
    );
    let inst = fx.installer(&xml);
    assert!(matches!(
        fomod::evaluate(&inst, &[pick("S", "G", &["Dead"])], None).unwrap_err(),
        FomodError::NotUsable { .. }
    ));
    let plan = fomod::evaluate(&inst, &[], None).unwrap();
    assert_eq!(dests(&plan), ["Extras/spikes.nif"]);
    assert_eq!(plan.choices, [pick("S", "G", &["Must"])]);
}

#[test]
fn a_dependent_plugin_type_follows_flags_and_files() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<installSteps><installStep name="S"><optionalFileGroups>
        <group name="G" type="SelectExactlyOne"><plugins>
          <plugin name="Patch">
            <files><file source="Patches\Heavy.esp" destination="p.esp"/></files>
            <typeDescriptor><dependencyType>
              <defaultType name="NotUsable"/>
              <patterns><pattern>
                <dependencies operator="Or"><fileDependency file="SkyUI_SE.esp" state="Active"/></dependencies>
                <type name="Recommended"/>
              </pattern></patterns>
            </dependencyType></typeDescriptor>
          </plugin>
          <plugin name="Plain"><files><file source="Patches\Light.esp" destination="l.esp"/></files></plugin>
        </plugins></group></optionalFileGroups></installStep></installSteps></config>"#
    );
    let inst = fx.installer(&xml);
    let yes = FomodContext::from_active(["SkyUI_SE.esp"]);
    let no = FomodContext::new();

    assert_eq!(
        fomod::defaults(&inst, Some(&yes)),
        [pick("S", "G", &["Patch"])]
    );
    assert_eq!(
        fomod::defaults(&inst, Some(&no)),
        [pick("S", "G", &["Plain"])]
    );
    assert!(fomod::evaluate(&inst, &[pick("S", "G", &["Patch"])], Some(&yes)).is_ok());
    assert!(matches!(
        fomod::evaluate(&inst, &[pick("S", "G", &["Patch"])], Some(&no)).unwrap_err(),
        FomodError::NotUsable { .. }
    ));
}

#[test]
fn always_install_and_install_if_usable_apply_to_unchosen_plugins() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<installSteps><installStep name="S"><optionalFileGroups>
        <group name="G" type="SelectAny"><plugins>
          <plugin name="A"><files>
            <file source="Extras\glow.nif" destination="always.nif" alwaysInstall="true"/>
            <file source="Extras\spikes.nif" destination="usable.nif" installIfUsable="true"/>
            <file source="Heavy\meshes\armor.nif" destination="only-if-chosen.nif"/>
          </files></plugin>
          <plugin name="Dead"><files>
            <file source="Light\meshes\armor.nif" destination="dead-usable.nif" installIfUsable="true"/>
            <file source="Patches\Heavy.esp" destination="dead-always.esp" alwaysInstall="1"/>
          </files><typeDescriptor><type name="NotUsable"/></typeDescriptor></plugin>
        </plugins></group></optionalFileGroups></installStep></installSteps></config>"#
    );
    let inst = fx.installer(&xml);
    let plan = fomod::evaluate(&inst, &[], None).unwrap();
    assert_eq!(
        dests(&plan),
        ["always.nif", "dead-always.esp", "usable.nif"]
    );
    let plan = fomod::evaluate(&inst, &[pick("S", "G", &["A"])], None).unwrap();
    assert!(dests(&plan).contains(&"only-if-chosen.nif".to_string()));
}

#[test]
fn files_missing_from_the_archive_are_skipped_with_a_note() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<requiredInstallFiles>
        <file source="Nope\gone.esp" destination="gone.esp"/>
        <folder source="NopeFolder" destination="x"/>
        <folder source="Extras\glow.nif" destination="meshes\"/>
        </requiredInstallFiles></config>"#
    );
    let plan = fomod::evaluate(&fx.installer(&xml), &[], None).unwrap();
    // A <folder> that names one file puts that file in the destination folder.
    assert_eq!(dests(&plan), ["meshes/glow.nif"]);
    assert_eq!(plan.notes.len(), 2, "{:?}", plan.notes);
    assert!(plan.notes.iter().any(|n| n.contains("Nope/gone.esp")));
}

#[test]
fn a_plan_that_would_put_a_file_where_a_folder_is_refused() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<requiredInstallFiles>
        <file source="Extras\glow.nif" destination="a"/>
        <file source="Extras\spikes.nif" destination="a\b"/>
        </requiredInstallFiles></config>"#
    );
    let err = fomod::evaluate(&fx.installer(&xml), &[], None).unwrap_err();
    assert!(
        matches!(err, FomodError::Content(ContentError::InvalidPath { .. })),
        "{err}"
    );
}

#[test]
fn a_choice_spec_may_contain_slashes_and_must_be_unambiguous() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<installSteps><installStep name="S/1"><optionalFileGroups>
        <group name="G" type="SelectAny"><plugins>
          <plugin name="a/b"/><plugin name="dup"/>
        </plugins></group></optionalFileGroups></installStep></installSteps></config>"#
    );
    let inst = fx.installer(&xml);
    assert_eq!(
        inst.resolve_choice("s/1/g/A/B").unwrap(),
        pick("S/1", "G", &["a/b"])
    );
    assert!(matches!(
        inst.resolve_choice("S/1/G/zzz").unwrap_err(),
        FomodError::UnknownChoice { .. }
    ));
    assert_eq!(
        fomod::merge_choices(
            vec![pick("S", "G", &["a"]), pick("S", "H", &["x"])],
            vec![pick("s", "g", &["b"])]
        ),
        [pick("S", "H", &["x"]), pick("s", "g", &["b"])]
    );
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

#[test]
fn defaults_choose_recommended_plugins_and_reveal_the_steps_they_unlock() {
    let fx = Fx::new();
    let inst = fx.installer(&main_config());
    // Heavy is Recommended, so it is chosen; that shows "Heavy Extras", where nothing is
    // recommended and a SelectAny group takes nothing.
    assert_eq!(
        fomod::defaults(&inst, None),
        [pick("Options", "Armor", &["Heavy"])]
    );
    let plan = fomod::evaluate(&inst, &fomod::defaults(&inst, None), None).unwrap();
    assert!(dests(&plan).contains(&"Heavy.esp".to_string()));
}

#[test]
fn a_select_exactly_one_group_with_no_recommendation_takes_its_first_usable_plugin() {
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<installSteps><installStep name="S"><optionalFileGroups>
        <group name="One" type="SelectExactlyOne"><plugins>
          <plugin name="Dead"><typeDescriptor><type name="NotUsable"/></typeDescriptor></plugin>
          <plugin name="First"/><plugin name="Second"/>
        </plugins></group>
        <group name="Two" type="SelectExactlyOne"><plugins>
          <plugin name="Opt"/><plugin name="Rec"><typeDescriptor><type name="Recommended"/></typeDescriptor></plugin>
          <plugin name="Rec2"><typeDescriptor><type name="Recommended"/></typeDescriptor></plugin>
        </plugins></group>
        <group name="Any" type="SelectAny"><plugins>
          <plugin name="R1"><typeDescriptor><type name="Recommended"/></typeDescriptor></plugin>
          <plugin name="R2"><typeDescriptor><type name="Recommended"/></typeDescriptor></plugin>
          <plugin name="O"/>
        </plugins></group>
        </optionalFileGroups></installStep></installSteps></config>"#
    );
    let inst = fx.installer(&xml);
    let d = fomod::defaults(&inst, None);
    assert_eq!(
        d,
        [
            pick("S", "One", &["First"]),
            pick("S", "Two", &["Rec"]),
            pick("S", "Any", &["R1", "R2"])
        ]
    );
    assert!(fomod::evaluate(&inst, &d, None).is_ok());

    // Explicit choices replace the defaults of their group only.
    let over = fomod::defaults_over(&inst, &[pick("S", "One", &["Second"])], None).unwrap();
    assert_eq!(over[0], pick("S", "One", &["Second"]));
    assert_eq!(over.len(), 3);
}

// ---------------------------------------------------------------------------
// Derivation
// ---------------------------------------------------------------------------

#[test]
fn the_derived_item_reuses_the_objects_records_the_choices_and_replays_to_the_same_id() {
    let fx = Fx::new();
    let archive = fx.item(&main_config());
    let objects_before = object_count(&fx.ctx);
    let choices = [
        pick("Options", "Armor", &["Heavy"]),
        pick("Heavy Extras", "Extras", &["Glow"]),
    ];

    let (_, plan, outcome) = fomod::install(&fx.ctx, &archive, &choices, None).unwrap();
    let AddOutcome::Added { item, objects_new } = &outcome else {
        panic!("expected a new item");
    };
    assert_eq!(*objects_new, 0);
    assert_eq!(object_count(&fx.ctx), objects_before, "no new objects");
    assert_ne!(item.item_id, archive);
    assert_eq!(item.name, "Cool Mod (FOMOD)");
    let paths: Vec<_> = item
        .files
        .iter()
        .map(|f| f.path.as_str().to_string())
        .collect();
    assert_eq!(paths, dests(&plan));
    assert!(paths.contains(&"meshes/armor.nif".to_string()));
    assert!(
        !paths.iter().any(|p| p.starts_with("Cool Mod")),
        "paths are data-folder relative"
    );
    let ContentSource::FomodInstall {
        from_item,
        choices: recorded,
        ..
    } = &item.sources[0]
    else {
        panic!("expected a FomodInstall source");
    };
    assert_eq!(from_item, &archive);
    assert_eq!(recorded, &plan.choices);

    // The manifest is on disk and verifies against the shared objects.
    let stored = get_item(&fx.ctx, &item.item_id).unwrap();
    assert_eq!(&stored, item);
    let report = verify_item(&fx.ctx, &item.item_id, VerifyDepth::Full).unwrap();
    assert!(report.problems.is_empty(), "{:?}", report.problems);

    // Replaying the recorded choices yields the same item (found as existing, source not added
    // twice).
    let (from, replay) = fomod::recorded_install(&stored).unwrap();
    assert_eq!(from, archive);
    let (_, _, again) = fomod::install(&fx.ctx, from, replay, None).unwrap();
    let AddOutcome::Existing {
        item: again,
        objects_new,
        ..
    } = &again
    else {
        panic!("expected the same item");
    };
    assert_eq!(*objects_new, 0);
    assert_eq!(again.item_id, item.item_id);
    assert_eq!(again.sources.len(), 1);
    assert_eq!(object_count(&fx.ctx), objects_before);

    // Different choices are a different item that shares the same objects.
    let (_, _, other) = fomod::install(
        &fx.ctx,
        &archive,
        &[pick("Options", "Armor", &["Light"])],
        None,
    )
    .unwrap();
    assert_ne!(other.item().item_id, item.item_id);
    assert!(object_count(&fx.ctx) == objects_before);
}

#[test]
fn removing_the_archive_keeps_the_derived_items_objects() {
    let fx = Fx::new();
    let archive = fx.item(&main_config());
    let (_, _, outcome) = fomod::install(
        &fx.ctx,
        &archive,
        &[pick("Options", "Armor", &["Heavy"])],
        None,
    )
    .unwrap();
    let derived = outcome.item().item_id.clone();

    remove_item(&fx.ctx, &archive).unwrap();
    let report = verify_item(&fx.ctx, &derived, VerifyDepth::Full).unwrap();
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    assert_eq!(report.checked, outcome.item().files.len());
    // Objects only the archive used are gone; the derived item's are not.
    assert_eq!(object_count(&fx.ctx), 5);
    // Replaying now says the archive is gone instead of guessing.
    assert!(fomod::install(&fx.ctx, &archive, &[], None).is_err());
}

#[test]
fn derive_item_refuses_what_the_source_item_does_not_contain() {
    let fx = Fx::new();
    let archive = fx.item(&main_config());
    let other_zip = fx.work.path().join("other.zip");
    create_zip(&other_zip, &[("o.txt".to_string(), b"other".to_vec())]);
    let other = add_archive(&fx.ctx, &other_zip, None)
        .unwrap()
        .item()
        .clone();
    let foreign = other.files[0].sha256.clone();

    let source = || ContentSource::Folder {
        path: "x".into(),
        added_at_unix_ms: 0,
    };
    let err = derive_item(
        &fx.ctx,
        &archive,
        vec![(RelPath::new("a.txt").unwrap(), foreign.clone())],
        "x",
        source(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("does not contain"), "{err}");

    let own = get_item(&fx.ctx, &archive).unwrap().files[0].sha256.clone();
    for bad in ["../x.txt", "a/../b", "CON.txt", "a/", ""] {
        let Ok(path) = RelPath::new(bad) else {
            continue;
        };
        assert!(
            derive_item(&fx.ctx, &archive, vec![(path, own.clone())], "x", source()).is_err(),
            "{bad:?} should be refused"
        );
    }
    assert!(matches!(
        derive_item(&fx.ctx, &archive, vec![], "x", source()).unwrap_err(),
        ContentError::EmptyItem
    ));
    let dup = vec![
        (RelPath::new("a.txt").unwrap(), own.clone()),
        (RelPath::new("A.TXT").unwrap(), own),
    ];
    assert!(derive_item(&fx.ctx, &archive, dup, "x", source()).is_err());
}

#[test]
fn an_install_that_selects_nothing_is_an_error() {
    let fx = Fx::new();
    let xml = format!("{HEADER}<moduleName>Empty</moduleName></config>");
    let archive = fx.item(&xml);
    assert!(matches!(
        fomod::install(&fx.ctx, &archive, &[], None).unwrap_err(),
        FomodError::NothingToInstall
    ));
}

// ---------------------------------------------------------------------------
// The instance as a context
// ---------------------------------------------------------------------------

mod instance {
    use super::*;
    use agora_core::ctx::CoreContext;
    use agora_core::game_base::BaseMode;
    use agora_core::game_deploy::{add_content, set_content_enabled};
    use agora_core::game_discovery::{DiscoveredInstall, InstallCapabilities};
    use agora_core::game_instance::create;
    use agora_core::game_registry::{
        GameRegistry, IdentifiedInstall, PackageSource, RuntimeResolution,
    };
    use agora_game_api::{
        ContentLayout, DeploymentStrategy, GameDefinition, GameId, GamePackage, InstallId,
        InstallKind, LaunchRecipe, PackageDefinition, RuntimeIdentity, StoreId, StoreIdentifier,
    };

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
                executable: agora_game_api::GamePath::Runtime {
                    path: RelPath::new("Game.exe").unwrap(),
                },
                arguments: vec![],
                environment: Default::default(),
                working_directory: agora_game_api::GamePath::Runtime {
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
                data_markers: vec![],
                root_markers: vec![],
                thunderstore_bepinex: false,
            }),
            copy_patterns: Vec::new(),
        }
    }

    #[test]
    fn active_and_inactive_files_come_from_the_instances_layers_and_base() {
        let tmp = tempfile::tempdir().unwrap();
        let def = definition();
        let ctx = CoreContext::for_testing(tmp.path().join("app_data"));
        agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        let mut builder = GameRegistry::builder();
        builder
            .add(
                PackageSource::Compiled {
                    crate_name: "test".into(),
                },
                Arc::new(TestPackage(PackageDefinition {
                    id: "test.game".into(),
                    version: semver::Version::new(0, 1, 0),
                    api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
                    parents: vec![],
                    games: vec![def.clone()],
                    frameworks: vec![],
                    tools: vec![],
                })),
            )
            .unwrap();
        let ctx = ctx.with_games(Arc::new(builder.build()));

        let install_dir = tmp.path().join("install");
        std::fs::create_dir_all(install_dir.join("Data")).unwrap();
        std::fs::write(install_dir.join("Game.exe"), b"exe").unwrap();
        std::fs::write(install_dir.join("Data").join("Skyrim.esm"), b"master").unwrap();
        let store = StoreId::new("steam").unwrap();
        let runtime = RuntimeIdentity {
            game: def.id.clone(),
            store: store.clone(),
            version: "1.0.0".into(),
            build: None,
        };
        let volume =
            agora_core::game_discovery::volume::VolumeDetector::new().get_volume_info(&install_dir);
        let install = IdentifiedInstall {
            game: def.id.clone(),
            install_id: InstallId::new("steam:12345").unwrap(),
            discovered: DiscoveredInstall {
                store,
                product: "12345".into(),
                name: "Test Game".into(),
                kind: InstallKind::BaseGame,
                parent_product: None,
                location: install_dir.clone(),
                store_version: Some(runtime.version.clone()),
                store_build: None,
                executables: vec!["Game.exe".into()],
                capabilities: InstallCapabilities {
                    executables_readable: true,
                    accepts_new_files: true,
                    relocatable: true,
                },
                volume,
            },
            add_ons: vec![],
            runtime: RuntimeResolution::Identified {
                runtime,
                source: "executable".into(),
            },
        };
        let inst = create(&ctx, &install, &def, "Ctx", None, BaseMode::Linked, &|_| {}).unwrap();

        // A mod whose archive wraps its Data folder, added with the layout's own rules.
        let mod_dir = tmp.path().join("skyui");
        std::fs::create_dir_all(mod_dir.join("Wrapper").join("Data")).unwrap();
        std::fs::write(
            mod_dir.join("Wrapper").join("Data").join("SkyUI_SE.esp"),
            b"ui",
        )
        .unwrap();
        let ui = agora_core::content_store::add_folder(&ctx, &mod_dir, Some("SkyUI"))
            .unwrap()
            .item()
            .item_id
            .clone();
        add_content(&ctx, &inst.instance_id, &ui, None, Some("Wrapper")).unwrap();

        let context = fomod::instance_context(&ctx, &inst.instance_id).unwrap();
        assert_eq!(context.state_of("skyui_se.esp"), FileState::Active);
        assert_eq!(
            context.state_of("Skyrim.esm"),
            FileState::Active,
            "from the pinned base"
        );
        assert_eq!(context.state_of("Other.esp"), FileState::Missing);

        set_content_enabled(&ctx, &inst.instance_id, &ui, false).unwrap();
        let context = fomod::instance_context(&ctx, &inst.instance_id).unwrap();
        assert_eq!(context.state_of("SkyUI_SE.esp"), FileState::Inactive);
    }
}

// ---------------------------------------------------------------------------
// Review fixes
// ---------------------------------------------------------------------------

#[test]
fn a_folder_entry_never_installs_the_installers_own_folder() {
    for src in ["", ".", "\\"] {
        let fx = Fx::new();
        let xml = format!(
            r#"{HEADER}<requiredInstallFiles><folder source="{src}" destination=""/></requiredInstallFiles></config>"#
        );
        let plan = fomod::evaluate(&fx.installer(&xml), &[], None).unwrap();
        let d = dests(&plan);
        assert!(d.contains(&"Core/CoolMod.esp".to_string()), "{d:?}");
        assert!(
            !d.iter()
                .any(|p| p.to_ascii_lowercase().starts_with("fomod/")),
            "{src:?}: {d:?}"
        );
        assert!(
            plan.notes.iter().any(|n| n.contains("own fomod folder")),
            "{:?}",
            plan.notes
        );
    }
    // Naming something inside fomod/ on purpose still works, as a file or as a folder.
    let fx = Fx::new();
    let xml = format!(
        r#"{HEADER}<requiredInstallFiles>
        <file source="fomod\images\heavy.png" destination="icon.png"/>
        <folder source="FOMOD\images" destination="imgs"/>
        </requiredInstallFiles></config>"#
    );
    let plan = fomod::evaluate(&fx.installer(&xml), &[], None).unwrap();
    assert_eq!(dests(&plan), ["icon.png", "imgs/heavy.png"]);
}

#[test]
fn destinations_differing_only_in_case_keep_the_higher_priority_and_its_spelling() {
    let fx = Fx::new();
    for (first, second, winner) in [
        (("Data\\Patch.esp", 1), ("data\\PATCH.ESP", 0), "Heavy.esp"),
        (("data\\PATCH.ESP", 0), ("Data\\Patch.esp", 1), "Light.esp"),
        (("Data\\Patch.esp", 0), ("data\\PATCH.ESP", 0), "Light.esp"),
    ] {
        let xml = format!(
            r#"{HEADER}<requiredInstallFiles>
            <file source="Patches\Heavy.esp" destination="{}" priority="{}"/>
            <file source="Patches\Light.esp" destination="{}" priority="{}"/>
            </requiredInstallFiles></config>"#,
            first.0, first.1, second.0, second.1
        );
        let plan = fomod::evaluate(&fx.installer(&xml), &[], None).unwrap();
        assert_eq!(plan.files.len(), 1, "{:?}", dests(&plan));
        assert!(
            plan.files[0].source.as_str().ends_with(winner),
            "{:?}",
            plan.files[0]
        );
    }
    // The winner's spelling is the one kept.
    let xml = format!(
        r#"{HEADER}<requiredInstallFiles>
        <file source="Patches\Heavy.esp" destination="Data\Patch.esp" priority="1"/>
        <file source="Patches\Light.esp" destination="data\PATCH.ESP"/>
        </requiredInstallFiles></config>"#
    );
    let plan = fomod::evaluate(&fx.installer(&xml), &[], None).unwrap();
    assert_eq!(dests(&plan), ["Data/Patch.esp"]);
}
