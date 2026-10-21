//! What a downloaded file is checked against, and how an unverified file is
//! represented.
//!
//! Agora never hashes files by hand for the catalog. A file is checked against
//! the hash its source published (GitHub's asset digest, Modrinth's version
//! file hashes, or the manifest hash of a `direct_hash` entry, which identifies
//! its one file). Two more expectations ask the user before they are overridden:
//!
//! * a **curator pin**: a `pins` entry on a download source, naming the exact
//!   release tag and asset it is for;
//! * a **previous install**: the SHA-256 Agora recorded when the same release
//!   file was installed before, and the source published no checksum for it.
//!
//! A file whose source published nothing is installed only after the user is
//! told, and its bytes' SHA-256 is recorded with `hash_verified: false`.

use serde::{Deserialize, Serialize};

/// Where a hash expectation came from. Shown to the user when it is not met.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashOrigin {
    /// The curator's `pins` entry for this release and asset.
    CuratorPin,
    /// The SHA-256 recorded when this release file was installed before.
    PreviousInstall,
}

impl HashOrigin {
    /// The wire name, also used in the error details.
    pub fn as_str(self) -> &'static str {
        match self {
            HashOrigin::CuratorPin => "curator_pin",
            HashOrigin::PreviousInstall => "previous_install",
        }
    }
}

/// A SHA-256 expectation the user can override with an explicit confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmableHash {
    pub origin: HashOrigin,
    pub sha256: String,
}

/// Whether `value` is a well-formed SHA-256 hex digest (64 hex characters).
pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The first expectation the downloaded bytes do not meet, if any.
///
/// `actual_sha256` is the lowercase or uppercase hex SHA-256 of the bytes.
/// A malformed expectation never matches, so it is reported rather than
/// silently treated as verified.
pub fn first_unmet<'a>(
    actual_sha256: &str,
    expectations: &'a [ConfirmableHash],
) -> Option<&'a ConfirmableHash> {
    expectations.iter().find(|expected| {
        !(is_sha256_hex(&expected.sha256)
            && expected.sha256.eq_ignore_ascii_case(actual_sha256.trim()))
    })
}

/// What a download was checked against, once it has passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadCheck {
    /// `true` when a hash the source published matched the bytes; `false` when
    /// the source published none, so the bytes are recorded as not verified.
    pub verified: bool,
    /// The lowercase SHA-256 of the bytes, which is what the install records.
    pub sha256: String,
}

/// Decide whether downloaded bytes may be installed.
///
/// * A hash the source published (`published_sha1` or `published_sha256`) must
///   match the bytes. A mismatch fails with `HashMismatch`, and nothing overrides
///   it. An empty value counts as nothing published; a malformed one never
///   matches.
/// * Every `expectations` entry (a curator pin, a hash remembered from an
///   earlier install) must match too. A mismatch fails with
///   `HashConfirmationRequired`, unless `accept_confirmation` is set.
/// * When the source published no hash at all, the bytes are accepted and the
///   result is `verified: false`, so the caller can tell the user.
pub fn verify_download(
    bytes: &[u8],
    published_sha1: Option<&str>,
    published_sha256: Option<&str>,
    expectations: &[ConfirmableHash],
    accept_confirmation: bool,
    file: &str,
    release: Option<&str>,
) -> crate::error::LauncherResult<DownloadCheck> {
    use crate::error::{HashConfirmation, LauncherError};

    let sha1 = published_sha1.unwrap_or("").trim().to_lowercase();
    let sha256 = published_sha256.unwrap_or("").trim().to_lowercase();
    let actual_sha256 = crate::download::sha256_hex(bytes);

    // Every hash the source published must match: a SHA-1 alone is weak, so when the source also
    // published a SHA-256 that is checked too, never one in place of the other.
    if !sha256.is_empty() && actual_sha256 != sha256 {
        return Err(LauncherError::HashMismatch);
    }
    if !sha1.is_empty() && crate::download::sha1_hex(bytes) != sha1 {
        return Err(LauncherError::HashMismatch);
    }
    let verified = !sha256.is_empty() || !sha1.is_empty();

    if !accept_confirmation {
        if let Some(unmet) = first_unmet(&actual_sha256, expectations) {
            return Err(LauncherError::HashConfirmationRequired(HashConfirmation {
                file: file.to_string(),
                release: release.map(str::to_string),
                expected: unmet.sha256.clone(),
                actual: actual_sha256,
                expected_from: unmet.origin,
            }));
        }
    }

    Ok(DownloadCheck {
        verified,
        sha256: actual_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(sha256: &str) -> ConfirmableHash {
        ConfirmableHash {
            origin: HashOrigin::CuratorPin,
            sha256: sha256.to_string(),
        }
    }

    #[test]
    fn a_matching_expectation_is_met_in_either_case() {
        let actual = "ab".repeat(32);
        assert!(first_unmet(&actual, &[pin(&actual.to_uppercase())]).is_none());
        assert!(first_unmet(&actual, &[pin(&actual)]).is_none());
    }

    #[test]
    fn a_different_expectation_is_reported_with_its_origin() {
        let actual = "ab".repeat(32);
        let expected = ConfirmableHash {
            origin: HashOrigin::PreviousInstall,
            sha256: "cd".repeat(32),
        };
        let unmet = first_unmet(&actual, std::slice::from_ref(&expected)).expect("unmet");
        assert_eq!(unmet, &expected);
        assert_eq!(unmet.origin.as_str(), "previous_install");
    }

    #[test]
    fn empty_and_malformed_expectations_are_never_met() {
        let actual = "ab".repeat(32);
        assert!(first_unmet(&actual, &[pin("")]).is_some());
        assert!(first_unmet(&actual, &[pin("ab")]).is_some());
        assert!(first_unmet(&actual, &[pin(&"zz".repeat(32))]).is_some());
    }

    fn digest_of(bytes: &[u8]) -> String {
        crate::download::sha256_hex(bytes)
    }

    #[test]
    fn a_published_digest_that_matches_verifies() {
        let bytes = b"mod bytes";
        let check = verify_download(
            bytes,
            None,
            Some(&digest_of(bytes)),
            &[],
            false,
            "a.jar",
            Some("v1"),
        )
        .expect("matches");
        assert!(check.verified);
        assert_eq!(check.sha256, digest_of(bytes));
    }

    #[test]
    fn a_matching_sha1_does_not_cover_for_a_wrong_sha256() {
        let bytes = b"mod bytes";
        let sha1 = crate::download::sha1_hex(bytes);
        let wrong = "cd".repeat(32);
        let error = verify_download(bytes, Some(&sha1), Some(&wrong), &[], true, "a.jar", None)
            .expect_err("both published hashes must match");
        assert_eq!(error.code(), "ERR_HASH_MISMATCH");
        let both = verify_download(
            bytes,
            Some(&sha1),
            Some(&digest_of(bytes)),
            &[],
            false,
            "a.jar",
            None,
        )
        .expect("both match");
        assert!(both.verified);
    }

    #[test]
    fn a_published_digest_that_differs_is_a_hard_mismatch_even_when_accepted() {
        let bytes = b"mod bytes";
        let wrong = "cd".repeat(32);
        let error = verify_download(bytes, None, Some(&wrong), &[], true, "a.jar", Some("v1"))
            .expect_err("mismatch");
        assert_eq!(error.code(), "ERR_HASH_MISMATCH");
    }

    #[test]
    fn no_published_hash_installs_as_not_verified_with_the_bytes_hash_recorded() {
        let bytes = b"old release file";
        let check =
            verify_download(bytes, None, None, &[], false, "a.jar", Some("v1")).expect("accepted");
        assert!(!check.verified);
        assert_eq!(check.sha256, digest_of(bytes));
    }

    #[test]
    fn an_empty_published_hash_is_nothing_published_not_a_match() {
        let check = verify_download(b"x", Some(""), Some("   "), &[], false, "a.jar", None)
            .expect("accepted");
        assert!(!check.verified);
    }

    #[test]
    fn a_malformed_published_hash_is_refused_not_treated_as_verified() {
        let error = verify_download(b"x", None, Some("not-hex"), &[], false, "a.jar", None)
            .expect_err("refused");
        assert_eq!(error.code(), "ERR_HASH_MISMATCH");
    }

    #[test]
    fn a_curator_pin_that_differs_asks_the_user_and_is_installed_when_accepted() {
        let bytes = b"new bytes";
        let pin = ConfirmableHash {
            origin: HashOrigin::CuratorPin,
            sha256: "ef".repeat(32),
        };
        let error = verify_download(
            bytes,
            None,
            None,
            std::slice::from_ref(&pin),
            false,
            "a.jar",
            Some("v1.2.0"),
        )
        .expect_err("asks");
        match error {
            crate::error::LauncherError::HashConfirmationRequired(detail) => {
                assert_eq!(detail.file, "a.jar");
                assert_eq!(detail.release.as_deref(), Some("v1.2.0"));
                assert_eq!(detail.expected, pin.sha256);
                assert_eq!(detail.actual, digest_of(bytes));
                assert_eq!(detail.expected_from, HashOrigin::CuratorPin);
            }
            other => panic!("expected a confirmation, got {other:?}"),
        }
        let accepted = verify_download(
            bytes,
            None,
            None,
            std::slice::from_ref(&pin),
            true,
            "a.jar",
            Some("v1.2.0"),
        )
        .expect("accepted");
        assert!(!accepted.verified);
    }

    #[test]
    fn a_remembered_hash_that_differs_asks_the_user() {
        let remembered = ConfirmableHash {
            origin: HashOrigin::PreviousInstall,
            sha256: digest_of(b"first download"),
        };
        let error = verify_download(
            b"second download",
            None,
            None,
            std::slice::from_ref(&remembered),
            false,
            "a.jar",
            Some("v1"),
        )
        .expect_err("asks");
        assert_eq!(error.code(), "ERR_HASH_CONFIRMATION_REQUIRED");
    }

    #[test]
    fn a_pin_that_matches_passes_without_confirmation() {
        let bytes = b"the file";
        let pin = ConfirmableHash {
            origin: HashOrigin::CuratorPin,
            sha256: digest_of(bytes),
        };
        let check =
            verify_download(bytes, None, None, &[pin], false, "a.jar", Some("v1")).expect("passes");
        assert!(!check.verified);
    }
}
