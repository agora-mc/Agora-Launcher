//! Deciding whether an update is real, and whether to offer it.
//!
//! Everything here is a pure function over data that has already been fetched.
//! No network, no filesystem, no database — those belong to the caller, so
//! that the part of the update path where being wrong is expensive can be
//! tested exhaustively without any of them.
//!
//! The order of checks is deliberate and is the whole security argument:
//!
//! 1. The document is well formed and for *this* plugin.
//! 2. It is signed by a key pinned when the user installed this plugin. Not a
//!    key from the document, not a key from the package that was just
//!    downloaded — the key already on record.
//! 3. Its sequence is not older than the newest one already verified, so a
//!    validly signed but stale document cannot be replayed to hold someone on
//!    a version with a known problem.
//! 4. Only then is any release considered, and only then is a byte fetched.
//!
//! The bytes are authenticated by the SHA-256 inside the signed document, so
//! the package host is untrusted: it can refuse to serve, and it can serve
//! something else, but something else fails the hash.

use agora_plugin_api::distribution::{PublicKey, Release, UpdateDocument};
use agora_plugin_api::manifest::PluginId;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::error::{LauncherError, LauncherResult};

fn refused(message: impl Into<String>) -> LauncherError {
    LauncherError::Generic {
        code: "ERR_PLUGIN_UPDATE_REFUSED".into(),
        message: message.into(),
    }
}

/// What the trust record says about one plugin, as verification needs it.
#[derive(Debug, Clone)]
pub struct PinnedTrust {
    /// Keys recorded when the user agreed to install this plugin.
    pub keys: Vec<PublicKey>,
    /// The highest sequence number already accepted from this publisher.
    pub highest_sequence: u64,
}

/// Check a fetched document against what is pinned.
///
/// Returns the id of the key that signed it, which the caller records. A
/// document is accepted if *any* pinned key verifies it: that is what makes
/// rotation work, and the threshold is one by design — requiring two would
/// mean a lone author with one key could never publish.
pub fn verify_document(
    trust: &PinnedTrust,
    expected_id: &PluginId,
    document: &UpdateDocument,
) -> LauncherResult<String> {
    document
        .validate()
        .map_err(|error| refused(error.message))?;

    if &document.id != expected_id {
        return Err(refused(format!(
            "this update document is for `{}`, not `{expected_id}`",
            document.id
        )));
    }

    // Authenticity first. Nothing about this document — not its sequence, not
    // its releases — is allowed to influence anything until a key the user
    // pinned has vouched for the whole of it.
    let message = document.signing_bytes();
    let mut rejected = Vec::new();
    let mut accepted_by = None;
    for signature in &document.signatures {
        match check_one(trust, signature, &message) {
            Ok(()) => {
                accepted_by = Some(signature.key_id.clone());
                break;
            }
            // Recorded and moved past rather than treated as fatal: a document
            // published during a key rotation carries a signature this install
            // has never heard of alongside the one it has, and refusing on the
            // first unknown one would break exactly the clients rotation is
            // meant to carry forward.
            Err(why) => rejected.push(why),
        }
    }

    let Some(key_id) = accepted_by else {
        return Err(refused(format!(
            "no signature on this update document matches a key recorded when \
             `{expected_id}` was installed ({}). Updates are only accepted from \
             whoever published the version you have.",
            if rejected.is_empty() {
                "it carried none".to_string()
            } else {
                rejected.join("; ")
            }
        )));
    };

    // Freshness second, and only now, so that an unsigned or forged document
    // can never move the recorded sequence forward.
    if document.sequence < trust.highest_sequence {
        return Err(refused(format!(
            "this update document is older than one already seen (sequence {} \
             against {}); it may be a stale copy being served in place of the \
             current one",
            document.sequence, trust.highest_sequence
        )));
    }

    Ok(key_id)
}

/// Whether one signature is a pinned key's signature over `message`.
fn check_one(
    trust: &PinnedTrust,
    signature: &agora_plugin_api::distribution::Signature,
    message: &[u8],
) -> Result<(), String> {
    let key = trust
        .keys
        .iter()
        .find(|key| key.id == signature.key_id)
        .ok_or_else(|| format!("`{}` is not a pinned key", signature.key_id))?;
    let key_bytes = key
        .decoded_bytes()
        .ok_or_else(|| format!("the pinned key `{}` is malformed", signature.key_id))?;
    let signature_bytes = signature
        .decoded_bytes()
        .ok_or_else(|| format!("the signature from `{}` is malformed", signature.key_id))?;
    let verifying = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|_| format!("`{}` is not a usable key", signature.key_id))?;
    verifying
        .verify(message, &Signature::from_bytes(&signature_bytes))
        .map_err(|_| format!("the signature from `{}` did not verify", signature.key_id))
}

/// What checking for an update concluded.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum UpdateVerdict {
    /// The newest release this host can run is the one already installed.
    UpToDate,
    /// There is something newer, and it fits.
    Available {
        from: String,
        to: String,
        notes: Option<String>,
        /// Bytes to fetch, authenticated by `sha256`.
        url: String,
        sha256: String,
        size: u64,
    },
    /// Something newer exists but needs a newer Agora. Distinguished from
    /// `UpToDate` because "you are up to date" is a lie that stops someone
    /// looking for the real reason their plugin is behind.
    NeedsNewerHost { latest: String, requires: String },
    /// The publisher only lists releases older than what is installed. Not an
    /// error — a development build, or a package installed by hand — but never
    /// applied automatically.
    InstalledIsNewer { installed: String, latest: String },
    /// The publisher lists nothing at all.
    NoReleases,
}

/// Decide what to offer, given a verified document.
///
/// Takes the document *after* [`verify_document`] has accepted it. Splitting
/// the two means this half can be reasoned about as ordinary version logic,
/// and there is no path where a decision is reached without the signature
/// having been checked first — the input type is the same either way, so the
/// discipline is the call order in [`crate::plugins::service`], which has one
/// caller and a test that its verdict is unreachable without verification.
pub fn decide(
    installed: &semver::Version,
    host_api: &semver::Version,
    document: &UpdateDocument,
) -> UpdateVerdict {
    if document.releases.is_empty() {
        return UpdateVerdict::NoReleases;
    }

    match document.newest_compatible(host_api) {
        Some(release) if &release.version > installed => UpdateVerdict::Available {
            from: installed.to_string(),
            to: release.version.to_string(),
            notes: release.notes.clone(),
            url: release.url.clone(),
            sha256: release.sha256.clone(),
            size: release.size,
        },
        Some(release) if &release.version == installed => UpdateVerdict::UpToDate,
        Some(release) => UpdateVerdict::InstalledIsNewer {
            installed: installed.to_string(),
            latest: release.version.to_string(),
        },
        None => match document.newest() {
            // Nothing compatible, but something exists: say which, and what it
            // wants, rather than reporting no update.
            Some(latest) => UpdateVerdict::NeedsNewerHost {
                latest: latest.version.to_string(),
                requires: latest.api_range.to_string(),
            },
            None => UpdateVerdict::NoReleases,
        },
    }
}

/// Confirm downloaded bytes are the ones the signed document described.
///
/// Size is checked first because it is free and because a mismatch there means
/// there is no point hashing several megabytes to reach the same conclusion.
pub fn verify_package_bytes(release: &Release, bytes: &[u8]) -> LauncherResult<()> {
    if bytes.len() as u64 != release.size {
        return Err(refused(format!(
            "the download is {} bytes; the signed release says {}",
            bytes.len(),
            release.size
        )));
    }
    let actual = crate::download::sha256_hex(bytes);
    if actual != release.sha256 {
        return Err(refused(
            "the downloaded package does not match the SHA-256 in the signed update document. \
             The bytes served are not the bytes the publisher signed.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_plugin_api::distribution::Signature as DocSignature;
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};

    fn engine() -> base64::engine::general_purpose::GeneralPurpose {
        base64::engine::general_purpose::STANDARD
    }

    fn keypair(seed: u8) -> (SigningKey, PublicKey) {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let public = PublicKey {
            id: format!("key-{seed}"),
            algorithm: "ed25519".into(),
            public_key: engine().encode(signing.verifying_key().to_bytes()),
        };
        (signing, public)
    }

    fn document(sequence: u64, releases: serde_json::Value) -> UpdateDocument {
        UpdateDocument::parse(
            &serde_json::json!({
                "schema": 1,
                "id": "acme.dashboard",
                "sequence": sequence,
                "releases": releases,
                "signatures": [{
                    "keyId": "unsigned",
                    "algorithm": "ed25519",
                    "value": engine().encode([0u8; 64])
                }]
            })
            .to_string(),
        )
        .unwrap()
    }

    fn one_release() -> serde_json::Value {
        serde_json::json!([{
            "version": "2.0.0",
            "url": "https://example.com/p-2.0.0.zip",
            "sha256": "a".repeat(64),
            "size": 1024,
            "apiRange": ">=0.1, <0.2"
        }])
    }

    /// Sign `document` with `key`, replacing whatever signatures it had.
    fn sign(document: &mut UpdateDocument, signing: &SigningKey, key_id: &str) {
        document.signatures = vec![DocSignature {
            key_id: key_id.into(),
            algorithm: "ed25519".into(),
            value: engine().encode(signing.sign(&document.signing_bytes()).to_bytes()),
        }];
    }

    fn id() -> PluginId {
        PluginId::parse("acme.dashboard").unwrap()
    }

    /// The base case, and incidentally an oracle for the contract crate's
    /// hand-written base64: the key and signature here are encoded by the
    /// `base64` crate and decoded by `agora-plugin-api`. If those two
    /// disagreed about a single byte, no signature would ever verify.
    #[test]
    fn a_document_signed_by_a_pinned_key_verifies() {
        let (signing, public) = keypair(7);
        let mut doc = document(1, one_release());
        sign(&mut doc, &signing, &public.id);

        let trust = PinnedTrust {
            keys: vec![public.clone()],
            highest_sequence: 0,
        };
        assert_eq!(verify_document(&trust, &id(), &doc).unwrap(), public.id);
    }

    #[test]
    fn a_document_signed_by_some_other_key_is_refused() {
        let (_, pinned) = keypair(7);
        let (attacker, _) = keypair(9);
        let mut doc = document(1, one_release());
        // Signed by the attacker but *claiming* the pinned key's id, which is
        // the shape an attacker would actually use.
        sign(&mut doc, &attacker, &pinned.id);

        let trust = PinnedTrust {
            keys: vec![pinned],
            highest_sequence: 0,
        };
        let error = verify_document(&trust, &id(), &doc).unwrap_err();
        assert!(error.to_string().contains("did not verify"), "{error}");
    }

    #[test]
    fn a_document_naming_a_key_nobody_pinned_is_refused() {
        let (signing, public) = keypair(9);
        let (_, pinned) = keypair(7);
        let mut doc = document(1, one_release());
        sign(&mut doc, &signing, &public.id);

        let trust = PinnedTrust {
            keys: vec![pinned],
            highest_sequence: 0,
        };
        let error = verify_document(&trust, &id(), &doc).unwrap_err();
        assert!(error.to_string().contains("not a pinned key"), "{error}");
    }

    /// Changing one byte of what the client acts on must break the signature.
    #[test]
    fn tampering_with_the_download_hash_invalidates_the_signature() {
        let (signing, public) = keypair(7);
        let mut doc = document(1, one_release());
        sign(&mut doc, &signing, &public.id);
        doc.releases[0].sha256 = "b".repeat(64);

        let trust = PinnedTrust {
            keys: vec![public],
            highest_sequence: 0,
        };
        assert!(verify_document(&trust, &id(), &doc).is_err());
    }

    #[test]
    fn a_signed_document_for_a_different_plugin_is_refused() {
        let (signing, public) = keypair(7);
        let mut doc = document(1, one_release());
        doc.id = PluginId::parse("evil.other").unwrap();
        sign(&mut doc, &signing, &public.id);

        let trust = PinnedTrust {
            keys: vec![public],
            highest_sequence: 0,
        };
        let error = verify_document(&trust, &id(), &doc).unwrap_err();
        assert!(error.to_string().contains("is for"), "{error}");
    }

    /// Replaying an old, validly signed document is how someone is held on a
    /// version with a known problem. The sequence is what stops it.
    #[test]
    fn a_validly_signed_but_stale_document_is_refused() {
        let (signing, public) = keypair(7);
        let mut doc = document(3, one_release());
        sign(&mut doc, &signing, &public.id);

        let trust = PinnedTrust {
            keys: vec![public],
            highest_sequence: 5,
        };
        let error = verify_document(&trust, &id(), &doc).unwrap_err();
        assert!(
            error.to_string().contains("older than one already seen"),
            "{error}"
        );
    }

    #[test]
    fn the_same_sequence_is_accepted_so_a_recheck_is_not_an_error() {
        let (signing, public) = keypair(7);
        let mut doc = document(5, one_release());
        sign(&mut doc, &signing, &public.id);

        let trust = PinnedTrust {
            keys: vec![public],
            highest_sequence: 5,
        };
        assert!(verify_document(&trust, &id(), &doc).is_ok());
    }

    /// Rotation: two keys pinned, the document signed by the newer one.
    #[test]
    fn either_pinned_key_may_sign() {
        let (_, old) = keypair(7);
        let (new_signing, new_public) = keypair(8);
        let mut doc = document(1, one_release());
        sign(&mut doc, &new_signing, &new_public.id);

        let trust = PinnedTrust {
            keys: vec![old, new_public.clone()],
            highest_sequence: 0,
        };
        assert_eq!(verify_document(&trust, &id(), &doc).unwrap(), new_public.id);
    }

    /// A document carrying a signature from a key this install does not know
    /// alongside one it does must still verify — otherwise a publisher mid
    /// rotation would break every client that had not caught up.
    #[test]
    fn an_unknown_signature_alongside_a_known_one_still_verifies() {
        let (signing, public) = keypair(7);
        let mut doc = document(1, one_release());
        sign(&mut doc, &signing, &public.id);
        doc.signatures.insert(
            0,
            DocSignature {
                key_id: "a-key-from-the-future".into(),
                algorithm: "ed25519".into(),
                value: engine().encode([0u8; 64]),
            },
        );
        // Re-sign, since inserting changed nothing signed — signatures are
        // excluded from the signing bytes, which is exactly why this works.
        let trust = PinnedTrust {
            keys: vec![public.clone()],
            highest_sequence: 0,
        };
        assert_eq!(verify_document(&trust, &id(), &doc).unwrap(), public.id);
    }

    // -- decide ------------------------------------------------------------

    fn host() -> semver::Version {
        semver::Version::new(0, 1, 0)
    }

    #[test]
    fn a_newer_compatible_release_is_offered_with_the_bytes_to_fetch() {
        let doc = document(1, one_release());
        let verdict = decide(&semver::Version::new(1, 0, 0), &host(), &doc);
        match verdict {
            UpdateVerdict::Available {
                from,
                to,
                sha256,
                size,
                ..
            } => {
                assert_eq!(from, "1.0.0");
                assert_eq!(to, "2.0.0");
                assert_eq!(sha256, "a".repeat(64));
                assert_eq!(size, 1024);
            }
            other => panic!("expected an update, got {other:?}"),
        }
    }

    #[test]
    fn the_installed_version_being_the_newest_is_up_to_date() {
        let doc = document(1, one_release());
        assert_eq!(
            decide(&semver::Version::new(2, 0, 0), &host(), &doc),
            UpdateVerdict::UpToDate
        );
    }

    /// "Up to date" would be a lie here, and the lie is what stops someone
    /// finding out why their plugin is behind.
    #[test]
    fn a_release_needing_a_newer_host_says_so_rather_than_up_to_date() {
        let doc = document(
            1,
            serde_json::json!([{
                "version": "3.0.0",
                "url": "https://example.com/p-3.0.0.zip",
                "sha256": "a".repeat(64),
                "size": 1024,
                "apiRange": ">=0.2, <0.3"
            }]),
        );
        match decide(&semver::Version::new(1, 0, 0), &host(), &doc) {
            UpdateVerdict::NeedsNewerHost { latest, .. } => assert_eq!(latest, "3.0.0"),
            other => panic!("expected NeedsNewerHost, got {other:?}"),
        }
    }

    #[test]
    fn a_development_build_ahead_of_the_publisher_is_reported_not_downgraded() {
        let doc = document(1, one_release());
        match decide(&semver::Version::new(9, 0, 0), &host(), &doc) {
            UpdateVerdict::InstalledIsNewer { installed, latest } => {
                assert_eq!(installed, "9.0.0");
                assert_eq!(latest, "2.0.0");
            }
            other => panic!("expected InstalledIsNewer, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_document_is_not_an_update() {
        let doc = document(1, serde_json::json!([]));
        assert_eq!(
            decide(&semver::Version::new(1, 0, 0), &host(), &doc),
            UpdateVerdict::NoReleases
        );
    }

    // -- bytes -------------------------------------------------------------

    #[test]
    fn package_bytes_must_match_the_signed_hash_and_size() {
        let bytes = b"a plugin package".to_vec();
        let mut release = Release {
            version: semver::Version::new(1, 0, 0),
            url: "https://example.com/p.zip".into(),
            sha256: crate::download::sha256_hex(&bytes),
            size: bytes.len() as u64,
            api_range: semver::VersionReq::parse(">=0.1, <0.2").unwrap(),
            notes: None,
            published: None,
        };
        verify_package_bytes(&release, &bytes).unwrap();

        let error = verify_package_bytes(&release, b"different bytes!").unwrap_err();
        assert!(error.to_string().contains("bytes"), "{error}");

        release.sha256 = "c".repeat(64);
        let error = verify_package_bytes(&release, &bytes).unwrap_err();
        assert!(error.to_string().contains("SHA-256"), "{error}");
    }
}
