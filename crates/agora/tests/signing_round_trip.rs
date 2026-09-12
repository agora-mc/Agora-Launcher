//! The author tooling and the launcher have to agree, byte for byte.
//!
//! `agora plugin keygen` and `agora plugin sign` are the only things that
//! produce a signature, and `agora-core` is the only thing that checks one. If
//! those two ever disagree about canonical form, base64, or the domain
//! separator, every plugin update in existence silently stops verifying — and
//! nothing else in the test suite would notice, because each half is tested
//! against itself.
//!
//! So this drives the real binary, end to end, and then verifies the result
//! with the real verifier.

use agora_plugin_api::distribution::{PublicKey, UpdateDocument};
use agora_plugin_api::manifest::PluginId;
use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    // The integration-test binary sits next to the one under test.
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join(format!("agora{}", std::env::consts::EXE_SUFFIX))
}

fn run(args: &[&str]) -> String {
    let output = Command::new(binary())
        .args(args)
        .output()
        .expect("could not run the agora binary");
    assert!(
        output.status.success(),
        "agora {} failed:\n{}\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn tempdir() -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "agora-signing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).unwrap();
    base
}

/// The document an author would write before signing: no signature at all.
fn draft(path: &Path) {
    std::fs::write(
        path,
        r#"{
  "schema": 1,
  "id": "acme.dashboard",
  "sequence": 7,
  "releases": [
    {
      "version": "1.2.0",
      "url": "https://example.com/acme.dashboard-1.2.0.zip",
      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "size": 4096,
      "apiRange": ">=0.1, <0.2",
      "notes": "Nothing in particular."
    }
  ]
}"#,
    )
    .unwrap();
}

/// Pull the public key out of `keygen --json`.
fn public_key(json: &str, id: &str) -> PublicKey {
    let value: serde_json::Value = serde_json::from_str(json).expect("keygen emits JSON");
    PublicKey {
        id: id.to_string(),
        algorithm: "ed25519".into(),
        public_key: value["publicKey"]
            .as_str()
            .expect("keygen reports a public key")
            .to_string(),
    }
}

#[test]
fn a_document_signed_by_the_cli_verifies_against_the_pinned_key() {
    let dir = tempdir();
    let key = dir.join("signing.key");
    let document = dir.join("updates.json");

    let generated = run(&[
        "plugin",
        "keygen",
        "--out",
        key.to_str().unwrap(),
        "--key-id",
        "2026-09",
        "--json",
    ]);
    let pinned = public_key(&generated, "2026-09");

    draft(&document);
    run(&[
        "plugin",
        "sign",
        document.to_str().unwrap(),
        "--key",
        key.to_str().unwrap(),
        "--key-id",
        "2026-09",
    ]);

    let signed = std::fs::read_to_string(&document).unwrap();
    let parsed = UpdateDocument::parse(&signed).expect("the signed document is a valid one");
    let trust = agora_core::plugins::PinnedTrust {
        keys: vec![pinned],
        highest_sequence: 0,
    };
    let used = agora_core::plugins::updates::verify_document(
        &trust,
        &PluginId::parse("acme.dashboard").unwrap(),
        &parsed,
    )
    .expect("the launcher accepts what the CLI produced");
    assert_eq!(used, "2026-09");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The other half of the guarantee: a signature is over *this* document, not
/// any document. Editing a signed one must break it.
#[test]
fn editing_a_signed_document_breaks_the_signature() {
    let dir = tempdir();
    let key = dir.join("signing.key");
    let document = dir.join("updates.json");

    let generated = run(&[
        "plugin",
        "keygen",
        "--out",
        key.to_str().unwrap(),
        "--key-id",
        "k1",
        "--json",
    ]);
    let pinned = public_key(&generated, "k1");

    draft(&document);
    run(&[
        "plugin",
        "sign",
        document.to_str().unwrap(),
        "--key",
        key.to_str().unwrap(),
        "--key-id",
        "k1",
    ]);

    // Swap the download for one the publisher never signed — the attack the
    // whole scheme exists to stop.
    let tampered = std::fs::read_to_string(&document)
        .unwrap()
        .replace("https://example.com/", "https://evil.example.net/");
    let parsed = UpdateDocument::parse(&tampered).unwrap();
    let trust = agora_core::plugins::PinnedTrust {
        keys: vec![pinned],
        highest_sequence: 0,
    };
    let error = agora_core::plugins::updates::verify_document(
        &trust,
        &PluginId::parse("acme.dashboard").unwrap(),
        &parsed,
    )
    .unwrap_err();
    assert!(error.to_string().contains("did not verify"), "{error}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Two different keys must not be interchangeable, which sounds obvious and is
/// exactly the kind of thing a wrong base64 decoder would break silently.
#[test]
fn a_document_signed_by_one_key_does_not_verify_against_another() {
    let dir = tempdir();
    let mine = dir.join("mine.key");
    let theirs = dir.join("theirs.key");
    let document = dir.join("updates.json");

    run(&[
        "plugin",
        "keygen",
        "--out",
        mine.to_str().unwrap(),
        "--key-id",
        "k1",
        "--json",
    ]);
    let other = run(&[
        "plugin",
        "keygen",
        "--out",
        theirs.to_str().unwrap(),
        "--key-id",
        "k1",
        "--json",
    ]);

    draft(&document);
    run(&[
        "plugin",
        "sign",
        document.to_str().unwrap(),
        "--key",
        mine.to_str().unwrap(),
        "--key-id",
        "k1",
    ]);

    let signed = std::fs::read_to_string(&document).unwrap();
    let parsed = UpdateDocument::parse(&signed).unwrap();
    let trust = agora_core::plugins::PinnedTrust {
        // Same id, different key: the id is a label for choosing which key to
        // try, never a credential in itself.
        keys: vec![public_key(&other, "k1")],
        highest_sequence: 0,
    };
    assert!(agora_core::plugins::updates::verify_document(
        &trust,
        &PluginId::parse("acme.dashboard").unwrap(),
        &parsed,
    )
    .is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

/// Signing must not require a signature to already be there — an author
/// producing their first release has nothing to put in that field.
#[test]
fn an_unsigned_draft_can_be_signed_without_a_placeholder() {
    let dir = tempdir();
    let key = dir.join("signing.key");
    let document = dir.join("updates.json");

    run(&[
        "plugin",
        "keygen",
        "--out",
        key.to_str().unwrap(),
        "--key-id",
        "first",
        "--json",
    ]);
    draft(&document);
    assert!(
        !std::fs::read_to_string(&document)
            .unwrap()
            .contains("signatures"),
        "the draft deliberately has no signature block"
    );

    run(&[
        "plugin",
        "sign",
        document.to_str().unwrap(),
        "--key",
        key.to_str().unwrap(),
        "--key-id",
        "first",
    ]);
    assert!(UpdateDocument::parse(&std::fs::read_to_string(&document).unwrap()).is_ok());

    let _ = std::fs::remove_dir_all(&dir);
}

/// A signing key cannot be regenerated and overwriting one destroys the only
/// copy, so the tool refuses rather than asking.
#[test]
fn keygen_refuses_to_overwrite_an_existing_key() {
    let dir = tempdir();
    let key = dir.join("signing.key");
    run(&["plugin", "keygen", "--out", key.to_str().unwrap(), "--json"]);
    let before = std::fs::read_to_string(&key).unwrap();

    let output = Command::new(binary())
        .args(["plugin", "keygen", "--out", key.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success(), "a second keygen must fail");
    assert_eq!(
        std::fs::read_to_string(&key).unwrap(),
        before,
        "and must not have touched the key"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
