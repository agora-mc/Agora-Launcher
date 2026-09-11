//! The compatibility fixtures: what plugin API 0.1 accepts, and what it refuses.
//!
//! These are the regression net for the *public contract*. The unit tests in
//! `manifest.rs` check the rules in isolation; this checks the shipped surface
//! as an author actually encounters it — whole manifest files, on disk, in
//! `docs/plugins/fixtures/`, where a plugin author can read them.
//!
//! The point is the future. When API 0.2 lands, `v0.1/accepted/` is the set of
//! manifests that must *still* load, and `v0.1/rejected/` is the set that must
//! still be refused for the same reason. Without a pinned set, "we kept
//! backward compatibility" is an assertion nobody can check.
//!
//! Adding a fixture is cheap and is the right reflex after fixing a contract
//! bug. Changing one is a deliberate compatibility decision and should be
//! visible in review as exactly that.

use agora_plugin_api::capability::CapabilitySet;
use agora_plugin_api::distribution::UpdateSource;
use agora_plugin_api::{PluginErrorCode, PluginManifest};
use std::path::{Path, PathBuf};

fn fixtures_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `crates/agora-plugin-api`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/plugins/fixtures/v0.1")
        .canonicalize()
        .expect("the v0.1 fixture directory should exist")
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name != "expectations.json")
        })
        .collect();
    // Sorted so a failure names the same fixture on every machine.
    files.sort();
    files
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().to_string()
}

/// Parse, then resolve capabilities — the same two steps an install performs.
enum Outcome {
    Accepted,
    RejectedAtManifest(agora_plugin_api::PluginError),
    RejectedAtCapabilityGrant(agora_plugin_api::PluginError),
}

fn evaluate(json: &str) -> Outcome {
    let manifest = match PluginManifest::parse(json) {
        Ok(manifest) => manifest,
        Err(error) => return Outcome::RejectedAtManifest(error),
    };
    match CapabilitySet::resolve(&manifest.capabilities) {
        Ok(_) => Outcome::Accepted,
        Err(error) => Outcome::RejectedAtCapabilityGrant(error),
    }
}

#[test]
fn every_accepted_fixture_still_loads() {
    let dir = fixtures_root().join("accepted");
    let files = json_files(&dir);
    assert!(
        files.len() >= 4,
        "the accepted fixture set looks truncated: {} file(s)",
        files.len()
    );

    for path in files {
        let json = std::fs::read_to_string(&path).expect("fixture is readable");
        match evaluate(&json) {
            Outcome::Accepted => {}
            Outcome::RejectedAtManifest(error) => panic!(
                "`{}` no longer parses, which is a backward-compatibility break: {}",
                name_of(&path),
                error.message
            ),
            Outcome::RejectedAtCapabilityGrant(error) => panic!(
                "`{}` parses but its capabilities no longer resolve, which is a \
                 backward-compatibility break: {}",
                name_of(&path),
                error.message
            ),
        }
    }
}

#[test]
fn every_rejected_fixture_is_still_refused_for_the_same_reason() {
    let dir = fixtures_root().join("rejected");
    let expectations: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("expectations.json")).expect("expectations.json"),
    )
    .expect("expectations.json is valid JSON");

    let files = json_files(&dir);
    assert!(
        files.len() >= 10,
        "the rejected fixture set looks truncated: {} file(s)",
        files.len()
    );

    for path in files {
        let name = name_of(&path);
        let expected = expectations
            .get(&name)
            .unwrap_or_else(|| panic!("`{name}` has no entry in expectations.json"));
        let expected_stage = expected["stage"].as_str().expect("stage");
        let expected_code = expected["code"].as_str().expect("code");
        let needle = expected["messageContains"]
            .as_str()
            .expect("messageContains");

        let json = std::fs::read_to_string(&path).expect("fixture is readable");
        let (stage, error) = match evaluate(&json) {
            Outcome::Accepted => panic!(
                "`{name}` is now ACCEPTED. It used to be refused ({expected_code}), so either a \
                 check was lost or this is a deliberate loosening that should update the fixture."
            ),
            Outcome::RejectedAtManifest(error) => ("manifest", error),
            Outcome::RejectedAtCapabilityGrant(error) => ("capability-grant", error),
        };

        // Where a refusal happens is part of the contract: an author debugging a
        // capability-grant failure is not looking at a syntax error.
        assert_eq!(
            stage, expected_stage,
            "`{name}` is now refused at the {stage} stage rather than {expected_stage}"
        );
        assert_eq!(
            format!("{:?}", error.code),
            to_variant(expected_code),
            "`{name}` is refused with {:?}, not the expected {expected_code}",
            error.code
        );
        assert!(
            error
                .message
                .to_lowercase()
                .contains(&needle.to_lowercase()),
            "`{name}` no longer explains itself: expected the message to mention `{needle}`, got `{}`",
            error.message
        );
    }
}

/// `SCREAMING_SNAKE_CASE` wire name back to the Rust variant name.
fn to_variant(code: &str) -> String {
    code.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(|c| c.to_lowercase()))
                    .collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

#[test]
fn every_rejection_expectation_names_a_fixture_that_exists() {
    // The other direction: an expectation left behind after its fixture was
    // deleted would silently stop testing anything.
    let dir = fixtures_root().join("rejected");
    let expectations: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("expectations.json")).unwrap())
            .unwrap();
    let present: Vec<String> = json_files(&dir).iter().map(|p| name_of(p)).collect();

    for key in expectations.as_object().expect("object").keys() {
        if key.starts_with('$') {
            continue;
        }
        assert!(
            present.contains(key),
            "expectations.json describes `{key}`, but no such fixture file exists"
        );
    }
}

#[test]
fn every_rejection_expectation_explains_why_the_rule_exists() {
    // A fixture whose reason nobody wrote down is a fixture someone will delete
    // in two years because it looks arbitrary.
    let dir = fixtures_root().join("rejected");
    let expectations: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("expectations.json")).unwrap())
            .unwrap();

    for (key, value) in expectations.as_object().expect("object") {
        if key.starts_with('$') {
            continue;
        }
        let why = value["why"].as_str().unwrap_or("");
        assert!(
            why.len() > 30,
            "`{key}` needs a `why` saying what the rule protects against"
        );
    }
}

// ---------------------------------------------------------------------------
// Distribution
// ---------------------------------------------------------------------------
//
// The same discipline, applied to the other half of the public contract. How a
// plugin is published and what a plugin may do are separate agreements that
// change on separate schedules, so they get separate fixture sets rather than
// one that has to be edited whenever either moves.

fn distribution_root() -> PathBuf {
    fixtures_root().join("distribution")
}

#[test]
fn every_accepted_update_source_still_loads() {
    let dir = distribution_root().join("accepted");
    let files = json_files(&dir);
    assert!(
        !files.is_empty(),
        "there should be accepted distribution fixtures in {}",
        dir.display()
    );
    for path in files {
        let json = std::fs::read_to_string(&path).unwrap();
        if let Err(error) = UpdateSource::parse(&json) {
            panic!(
                "`{}` is a published example and must keep loading, but was refused: {}",
                name_of(&path),
                error.message
            );
        }
    }
}

#[test]
fn every_rejected_update_source_is_still_refused_for_the_same_reason() {
    let dir = distribution_root().join("rejected");
    let expectations: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("expectations.json")).unwrap())
            .unwrap();

    let files = json_files(&dir);
    assert!(!files.is_empty(), "there should be rejected fixtures");

    for path in files {
        let name = name_of(&path);
        let expected = &expectations[&name];
        assert!(
            !expected.is_null(),
            "`{name}` has no entry in expectations.json. Every rejected fixture has to say \
             what it is pinning, or a refusal that changes reason silently still passes."
        );
        let json = std::fs::read_to_string(&path).unwrap();
        let error = match UpdateSource::parse(&json) {
            Ok(_) => panic!("`{name}` must keep being refused, but it loaded"),
            Err(error) => error,
        };
        let needle = expected["messageContains"].as_str().unwrap_or_default();
        assert!(
            error
                .message
                .to_lowercase()
                .contains(&needle.to_lowercase()),
            "`{name}` is still refused, but for a different reason.\n  expected to mention: \
             {needle}\n  actual: {}",
            error.message
        );
    }
}

#[test]
fn every_distribution_rejection_explains_why_the_rule_exists() {
    let dir = distribution_root().join("rejected");
    let expectations: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("expectations.json")).unwrap())
            .unwrap();

    for (key, value) in expectations.as_object().expect("object") {
        if key.starts_with('$') {
            continue;
        }
        let why = value["why"].as_str().unwrap_or("");
        assert!(
            why.len() > 30,
            "`{key}` needs a `why` saying what the rule protects against"
        );
    }
}

/// A key set with two entries is the shape a rotation takes, so it has to keep
/// being accepted — refusing it would strand every author mid-rotation.
#[test]
fn a_source_may_pin_more_than_one_key() {
    let json =
        std::fs::read_to_string(distribution_root().join("accepted/source-mid-rotation.json"))
            .unwrap();
    let source = UpdateSource::parse(&json).expect("a rotating source must load");
    assert_eq!(source.keys.len(), 2);
    assert!(source.key("2026-03").is_some());
    assert!(source.key("2026-09").is_some());
}

#[test]
fn the_error_code_spelling_in_expectations_round_trips() {
    // Guards the `to_variant` helper itself: if `PluginErrorCode`'s serde naming
    // ever changes, this fails here rather than making every fixture look broken.
    for (wire, variant) in [
        ("INVALID_MANIFEST", PluginErrorCode::InvalidManifest),
        ("DEPENDENCY_CYCLE", PluginErrorCode::DependencyCycle),
        (
            "DUPLICATE_CONTRIBUTION",
            PluginErrorCode::DuplicateContribution,
        ),
        ("INCOMPATIBLE_API", PluginErrorCode::IncompatibleApi),
    ] {
        assert_eq!(
            to_variant(wire),
            format!("{variant:?}"),
            "`{wire}` no longer maps to {variant:?}"
        );
        assert_eq!(
            serde_json::to_value(variant).unwrap(),
            serde_json::Value::String(wire.to_string()),
            "{variant:?} no longer serialises as `{wire}`"
        );
    }
}
