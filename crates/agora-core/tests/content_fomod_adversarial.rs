#![allow(unused_imports, dead_code)]
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

// ---------------------------------------------------------------------------
// Adversarial probes
// ---------------------------------------------------------------------------

fn only_required(body: &str) -> String {
    format!("{HEADER}<moduleName>Probe</moduleName><requiredInstallFiles>{body}</requiredInstallFiles></config>")
}

#[test]
fn probe_a_root_folder_source_never_installs_the_installer_itself() {
    // `<folder source="">` (or ".") means "the installer root"; the fomod folder holds the
    // installer's own files (ModuleConfig.xml, images) and must not land in the game's Data.
    for src in ["", "."] {
        let fx = Fx::new();
        let inst = fx.installer(&only_required(&format!(
            r#"<folder source="{src}" destination=""/>"#
        )));
        let plan = fomod::evaluate(&inst, &[], None);
        if let Ok(plan) = plan {
            let d = dests(&plan);
            assert!(
                !d.iter()
                    .any(|p| p.to_ascii_lowercase().starts_with("fomod/")),
                "source {src:?} installed the installer's own files: {d:?}"
            );
        }
    }
}

#[test]
fn probe_utf16_big_endian_configs_parse() {
    let fx = Fx::new();
    let text = only_required(r#"<file source="Core\CoolMod.esp" destination="CoolMod.esp"/>"#);
    let mut be = vec![0xFE, 0xFF];
    be.extend(text.encode_utf16().flat_map(|u| u.to_be_bytes()));
    let id = store_archive(&fx.ctx, fx.work.path(), be, &[]);
    let inst = fomod::parse(&fx.ctx, &id).expect("a UTF-16 BE config with a BOM must parse");
    let plan = fomod::evaluate(&inst, &[], None).unwrap();
    assert_eq!(dests(&plan), vec!["CoolMod.esp".to_string()]);
}

#[test]
fn probe_destinations_differing_only_in_case_resolve_to_one_file() {
    // Two sources onto one destination spelled differently: Windows sees one file, so the plan
    // must hold one, the higher priority's, rather than fail or install both.
    let fx = Fx::new();
    let inst = fx.installer(&only_required(
        r#"<file source="Patches\Heavy.esp" destination="Data\Patch.esp" priority="1"/>
           <file source="Patches\Light.esp" destination="data\PATCH.ESP" priority="0"/>"#,
    ));
    let plan = fomod::evaluate(&inst, &[], None)
        .expect("a case-only clash is a priority decision, not an error");
    let lower: Vec<String> = dests(&plan)
        .iter()
        .map(|d| d.to_ascii_lowercase())
        .collect();
    assert_eq!(lower, vec!["data/patch.esp".to_string()]);
    let file = plan
        .files
        .iter()
        .find(|f| {
            f.destination
                .as_str()
                .eq_ignore_ascii_case("data/patch.esp")
        })
        .unwrap();
    assert!(
        file.source
            .as_str()
            .eq_ignore_ascii_case("Patches/Heavy.esp"),
        "the higher priority must win"
    );
}

#[test]
fn probe_choosing_something_that_does_not_exist_is_an_error() {
    let fx = Fx::new();
    let inst = fx.installer(&main_config());
    for c in [
        pick("No Such Step", "Armor", &["Heavy"]),
        pick("Options", "No Such Group", &["Heavy"]),
        pick("Options", "Armor", &["No Such Plugin"]),
        pick("", "", &[]),
    ] {
        assert!(
            fomod::evaluate(&inst, std::slice::from_ref(&c), None).is_err(),
            "accepted {c:?}"
        );
    }
}

#[test]
fn probe_derive_item_refuses_an_object_from_another_item() {
    let fx = Fx::new();
    let id = fx.item(&main_config());
    // An object the store has, but not in this item: add an unrelated archive first.
    let other_zip = fx.work.path().join("other.zip");
    create_zip(
        &other_zip,
        &[("secret.txt".to_string(), b"not yours".to_vec())],
    );
    let other = add_archive(&fx.ctx, &other_zip, None)
        .unwrap()
        .item()
        .clone();
    let foreign = other.files[0].sha256.clone();
    let r = derive_item(
        &fx.ctx,
        &id,
        vec![(RelPath::new("secret.txt").unwrap(), foreign)],
        "Probe",
        ContentSource::FomodInstall {
            from_item: id.clone(),
            choices: vec![],
            added_at_unix_ms: 0,
        },
    );
    assert!(
        r.is_err(),
        "derive_item accepted an object that is not in the source item"
    );
}

#[test]
fn probe_dependency_nesting_is_bounded() {
    let fx = Fx::new();
    let mut deps = String::new();
    for _ in 0..200 {
        deps.push_str(r#"<dependencies operator="And">"#);
    }
    deps.push_str(r#"<flagDependency flag="x" value="y"/>"#);
    for _ in 0..200 {
        deps.push_str("</dependencies>");
    }
    let config = format!(
        "{HEADER}<moduleName>Deep</moduleName><conditionalFileInstalls><patterns><pattern>{deps}<files><file source=\"Core\\CoolMod.esp\"/></files></pattern></patterns></conditionalFileInstalls></config>"
    );
    let id = fx.item(&config);
    assert!(
        fomod::parse(&fx.ctx, &id).is_err(),
        "200 levels of nesting must be refused, not recursed"
    );
}
