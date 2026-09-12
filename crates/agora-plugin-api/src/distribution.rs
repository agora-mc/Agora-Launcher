//! How a plugin says where its updates come from, and what a publisher signs.
//!
//! Agora has no servers. A plugin author publishes two static files anywhere
//! that serves HTTPS — a GitHub Pages site, a release asset, their own host —
//! and that is the whole distribution system:
//!
//! - an **update document** ([`UpdateDocument`]), listing the releases that
//!   exist and carrying an Ed25519 signature over itself;
//! - the **package** for each release, authenticated by the SHA-256 recorded
//!   *inside* that signed document.
//!
//! The package therefore does not need its own signature, and can live on a
//! different host from the document. The document is what is signed because
//! the document is what the client has to make decisions from: which release
//! is newest, whether a downgrade is being attempted, whether the bytes it is
//! about to fetch are the ones the author published. A bare package signature
//! authenticates bytes and answers none of those questions.
//!
//! # What the signature does and does not prove
//!
//! The trusted key arrives with the first install, in
//! [`UpdateSource`] inside the package. The launcher records it at the moment
//! the user agrees to install, and from then on reads it only from its own
//! database — never again from a downloaded file. That is the same rule the
//! capability grants follow, and it gives the same guarantee: **updates come
//! from whoever published the thing you installed.**
//!
//! It is emphatically *not* proof of who that publisher is. Whoever hands you
//! the first package chooses the key and the URL, so a package obtained from a
//! bad source is bad forever. The `publisher.plugin` namespace is not an
//! authorship claim either. This is continuity of authorship, not identity —
//! the same thing an Android signing key gives you, with the same limits, and
//! `docs/plugins/publishing.md` says so in those words.
//!
//! # Key rotation, and the thing that has no answer
//!
//! [`UpdateSource`] holds a *set* of keys and the signature block holds a
//! list, so an author rotates by publishing a release signed with the current
//! key whose package names both the current and the next one. Installing it
//! replaces the pinned set, and the following release may use the new key.
//! The chain is verifiable end to end and needs nothing hosted.
//!
//! Losing the only key ends the line: there is no authority that can vouch for
//! a replacement and no revocation list to publish it to, so users have to
//! uninstall and install the new plugin deliberately. That is a real cost of
//! having no servers, and it is written down rather than papered over.

use crate::error::{PluginError, PluginErrorCode, PluginResult};
use crate::manifest::PluginId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Schema version of both files described here.
pub const DISTRIBUTION_SCHEMA_VERSION: u32 = 1;

/// Filename of the update source, at the root of a package.
///
/// Deliberately *not* a field in `agora-plugin.json`. Where a plugin gets its
/// updates is a different question from what a plugin is allowed to do, it
/// changes on a different schedule, and keeping it out means the manifest
/// contract — which is pinned by compatibility fixtures — does not move every
/// time distribution grows a feature.
pub const UPDATE_SOURCE_FILENAME: &str = "agora-plugin-update.json";

/// Domain separator for the signed bytes.
///
/// Prefixed so a signature over an update document can never be replayed as a
/// signature over some other Agora structure that happened to canonicalise to
/// the same bytes, and versioned so the envelope itself can change without
/// ambiguity about which rule produced a given signature.
pub const SIGNING_CONTEXT: &str = "agora-plugin-update:v1:";

/// How many releases one document may list.
///
/// Generous for a plugin's whole history, small enough that the document stays
/// something a client can fetch and parse without care.
pub const MAX_RELEASES: usize = 200;

/// How many keys may be pinned for one plugin at once.
///
/// More than two means a rotation that never completed, or a key set nobody
/// audits. Two covers "current and next".
pub const MAX_KEYS: usize = 4;

// ---------------------------------------------------------------------------
// The package side: where updates come from
// ---------------------------------------------------------------------------

/// `agora-plugin-update.json`, shipped inside the package.
///
/// Read once, at install time, in front of the user. After that the launcher
/// uses its own stored copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateSource {
    pub schema: u32,
    /// HTTPS URL of the update document.
    pub url: String,
    /// Keys any of which may sign an update document for this plugin.
    pub keys: Vec<PublicKey>,
}

/// One Ed25519 public key a publisher signs with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicKey {
    /// Author-chosen label, echoed by a signature so verification can pick the
    /// right key without trying all of them. Not a security boundary.
    pub id: String,
    /// Always `ed25519` today. Present so a second algorithm can be added
    /// without a new schema, and so a file that names something else is
    /// refused rather than silently misread.
    pub algorithm: String,
    /// Standard base64 of the 32 raw key bytes.
    pub public_key: String,
}

impl UpdateSource {
    /// Parse and validate. Prefer this over `serde_json` directly.
    pub fn parse(json: &str) -> PluginResult<Self> {
        let source: UpdateSource = serde_json::from_str(json).map_err(|e| {
            PluginError::new(
                PluginErrorCode::InvalidPackage,
                format!("{UPDATE_SOURCE_FILENAME} is not valid: {e}"),
            )
        })?;
        source.validate()?;
        Ok(source)
    }

    pub fn validate(&self) -> PluginResult<()> {
        if self.schema != DISTRIBUTION_SCHEMA_VERSION {
            return Err(invalid(format!(
                "update schema {} is not supported; this Agora reads schema {}",
                self.schema, DISTRIBUTION_SCHEMA_VERSION
            )));
        }
        if !self.url.starts_with("https://") {
            return Err(invalid(format!(
                "`url` must be an https URL, got `{}`",
                self.url
            )));
        }
        if self.keys.is_empty() {
            return Err(invalid(
                "`keys` must name at least one key, or updates could never be verified",
            ));
        }
        if self.keys.len() > MAX_KEYS {
            return Err(invalid(format!(
                "`keys` lists {} keys, past the {MAX_KEYS} allowed",
                self.keys.len()
            )));
        }
        let mut seen = std::collections::BTreeSet::new();
        for key in &self.keys {
            key.validate()?;
            if !seen.insert(key.id.as_str()) {
                return Err(invalid(format!("two keys share the id `{}`", key.id)));
            }
        }
        Ok(())
    }

    /// The key with this id, if it is one of the pinned ones.
    pub fn key(&self, id: &str) -> Option<&PublicKey> {
        self.keys.iter().find(|key| key.id == id)
    }
}

impl PublicKey {
    pub fn validate(&self) -> PluginResult<()> {
        if self.id.trim().is_empty() {
            return Err(invalid("a key must have a non-empty `id`"));
        }
        if self.id.len() > 64 {
            return Err(invalid(format!("key id `{}` is too long", self.id)));
        }
        if !self.algorithm.eq_ignore_ascii_case("ed25519") {
            return Err(invalid(format!(
                "unsupported key algorithm `{}`; this Agora verifies ed25519",
                self.algorithm
            )));
        }
        if self.decoded_bytes().is_none() {
            return Err(invalid(format!(
                "key `{}` is not 32 bytes of standard base64",
                self.id
            )));
        }
        Ok(())
    }

    /// The 32 raw key bytes, or `None` if this is not a well-formed key.
    ///
    /// Decoding lives here rather than in the verifier so that a malformed key
    /// is rejected when the file is read — in front of the user, at install
    /// time — instead of months later when an update fails for a reason nobody
    /// can act on.
    pub fn decoded_bytes(&self) -> Option<[u8; 32]> {
        let raw = base64_decode(&self.public_key)?;
        raw.try_into().ok()
    }

    /// Short, comparable form for showing a user.
    ///
    /// Worth showing only if it can be compared against something the author
    /// published independently. On its own a fingerprint proves nothing, and
    /// the install prompt says so rather than implying otherwise.
    pub fn fingerprint(&self) -> String {
        let bytes = match self.decoded_bytes() {
            Some(bytes) => bytes,
            None => return "invalid".into(),
        };
        bytes
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    }
}

// ---------------------------------------------------------------------------
// The hosted side: what releases exist
// ---------------------------------------------------------------------------

/// The author-hosted, signed list of releases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateDocument {
    pub schema: u32,
    /// Must equal the id of the plugin this document is fetched for. Checked,
    /// so a signed document for one plugin cannot be served in place of
    /// another's.
    pub id: PluginId,
    /// Monotonic counter the publisher increments on every publication.
    ///
    /// Freshness cannot rest on a timestamp: a timestamp is a claim by whoever
    /// serves the file, and serving an old *validly signed* document back is
    /// exactly how an attacker keeps a client on a version with a known
    /// problem. A client remembers the highest sequence it has verified and
    /// refuses anything lower.
    pub sequence: u64,
    pub releases: Vec<Release>,
    /// Detached signatures over this document. One is enough; the list exists
    /// so rotation and future thresholds do not need a new schema.
    ///
    /// Defaulted so an author can write a document without one and have
    /// `agora plugin sign` fill it in. [`Self::parse`] still refuses an empty
    /// list, so nothing unsigned reaches a client.
    #[serde(default)]
    pub signatures: Vec<Signature>,
}

/// One published version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Release {
    pub version: semver::Version,
    /// HTTPS URL of the package.
    pub url: String,
    /// Lowercase hex SHA-256 of the package bytes. This is what actually
    /// authenticates the download; the URL is only where to look.
    pub sha256: String,
    /// Expected size in bytes, so an oversized response is abandoned before it
    /// is read rather than after.
    pub size: u64,
    /// Host API range this release supports. Must match the package's own
    /// manifest; carried here so an incompatible release can be skipped
    /// without downloading it.
    pub api_range: semver::VersionReq,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
}

/// A detached Ed25519 signature over the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Signature {
    /// Which pinned key produced this.
    pub key_id: String,
    pub algorithm: String,
    /// Standard base64 of the 64 raw signature bytes.
    pub value: String,
}

impl Signature {
    /// The 64 raw signature bytes, or `None` if malformed.
    pub fn decoded_bytes(&self) -> Option<[u8; 64]> {
        let raw = base64_decode(&self.value)?;
        raw.try_into().ok()
    }
}

impl UpdateDocument {
    /// Parse and check everything that does not require a key.
    ///
    /// Signature verification is deliberately *not* here: this crate has no
    /// cryptography dependency, and the key to check against lives in the
    /// launcher's database rather than in the document. `agora-core` does that
    /// half. What this does do is refuse a document that is malformed,
    /// oversized, or internally inconsistent before any of it is trusted.
    pub fn parse(json: &str) -> PluginResult<Self> {
        let document: UpdateDocument = serde_json::from_str(json).map_err(|e| {
            PluginError::new(
                PluginErrorCode::InvalidManifest,
                format!("the update document is not valid: {e}"),
            )
        })?;
        document.validate()?;
        Ok(document)
    }

    /// Parse a document that has not been signed yet.
    ///
    /// For authoring tools only. Everything except the signature block is
    /// checked, so `agora plugin sign` still refuses a document with a bad
    /// release in it — but an author does not have to supply a valid signature
    /// in order to produce one, which is a requirement that cannot be met.
    ///
    /// A client must never use this. Signature checking is the whole point on
    /// the reading side, which is why the lenient path is a separate,
    /// differently named function rather than a flag on [`Self::parse`].
    pub fn parse_draft(json: &str) -> PluginResult<Self> {
        let document: UpdateDocument = serde_json::from_str(json).map_err(|e| {
            PluginError::new(
                PluginErrorCode::InvalidManifest,
                format!("the update document is not valid: {e}"),
            )
        })?;
        document.validate_except_signatures()?;
        Ok(document)
    }

    pub fn validate(&self) -> PluginResult<()> {
        self.validate_except_signatures()?;
        if self.signatures.is_empty() {
            return Err(invalid("the document carries no signature"));
        }
        if self.signatures.len() > MAX_KEYS {
            return Err(invalid("the document carries implausibly many signatures"));
        }
        for signature in &self.signatures {
            if !signature.algorithm.eq_ignore_ascii_case("ed25519") {
                return Err(invalid(format!(
                    "unsupported signature algorithm `{}`",
                    signature.algorithm
                )));
            }
            if signature.decoded_bytes().is_none() {
                return Err(invalid(format!(
                    "the signature from key `{}` is not 64 bytes of standard base64",
                    signature.key_id
                )));
            }
        }
        Ok(())
    }

    fn validate_except_signatures(&self) -> PluginResult<()> {
        if self.schema != DISTRIBUTION_SCHEMA_VERSION {
            return Err(invalid(format!(
                "update schema {} is not supported; this Agora reads schema {}",
                self.schema, DISTRIBUTION_SCHEMA_VERSION
            )));
        }
        if self.releases.len() > MAX_RELEASES {
            return Err(invalid(format!(
                "the document lists {} releases, past the {MAX_RELEASES} allowed",
                self.releases.len()
            )));
        }
        let mut versions = std::collections::BTreeSet::new();
        for release in &self.releases {
            release.validate()?;
            if !versions.insert(release.version.to_string()) {
                return Err(invalid(format!(
                    "version {} is listed twice; a version must name exactly one set of bytes",
                    release.version
                )));
            }
        }
        Ok(())
    }

    /// The exact bytes a publisher signs and a client verifies.
    ///
    /// The `signatures` field is excluded — it cannot be part of what it signs
    /// — and everything else is canonicalised so that reserialising the
    /// document, or serving it through something that reorders keys or
    /// reindents it, does not invalidate a valid signature.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut value = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        if let Some(object) = value.as_object_mut() {
            object.remove("signatures");
        }
        let mut out = SIGNING_CONTEXT.as_bytes().to_vec();
        out.extend_from_slice(canonical_json(&value).as_bytes());
        out
    }

    /// The release with this exact version.
    pub fn release(&self, version: &semver::Version) -> Option<&Release> {
        self.releases
            .iter()
            .find(|release| &release.version == version)
    }

    /// The newest release this host can actually run.
    ///
    /// `host_api` is the running contract version. A release whose `apiRange`
    /// excludes it is skipped rather than offered and then refused after the
    /// download — being told "up to date" when the newest release does not fit
    /// this build is confusing, so callers distinguish the two by also asking
    /// [`Self::newest`].
    pub fn newest_compatible(&self, host_api: &semver::Version) -> Option<&Release> {
        self.releases
            .iter()
            .filter(|release| release.api_range.matches(host_api))
            .max_by(|a, b| a.version.cmp(&b.version))
    }

    /// The newest release listed, compatible or not.
    pub fn newest(&self) -> Option<&Release> {
        self.releases
            .iter()
            .max_by(|a, b| a.version.cmp(&b.version))
    }
}

impl Release {
    pub fn validate(&self) -> PluginResult<()> {
        if !self.url.starts_with("https://") {
            return Err(invalid(format!(
                "release {} must be served over https, got `{}`",
                self.version, self.url
            )));
        }
        let hash = self.sha256.trim();
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid(format!(
                "release {} does not carry a SHA-256",
                self.version
            )));
        }
        if hash.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err(invalid(format!(
                "release {}: the SHA-256 must be lowercase hex, so two documents \
                 describing the same bytes are byte-identical",
                self.version
            )));
        }
        if self.size == 0 {
            return Err(invalid(format!(
                "release {} declares no size",
                self.version
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Canonicalisation
// ---------------------------------------------------------------------------

/// Serialise a JSON value so the same document always produces the same bytes.
///
/// Object keys sorted, no insignificant whitespace, strings escaped the one way
/// `serde_json` escapes them. Every value in these documents is a string,
/// integer or array, so there is no float representation question to get wrong
/// — and [`UpdateDocument`] is a closed struct with `deny_unknown_fields`, so
/// there is no possibility of a field surviving into the signed bytes that the
/// reader did not understand.
pub fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: BTreeMap<&String, &serde_json::Value> = map.iter().collect();
            let inner: Vec<String> = sorted
                .iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::Value::String((*key).clone()),
                        canonical_json(value)
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn invalid(message: impl Into<String>) -> PluginError {
    PluginError::new(PluginErrorCode::InvalidManifest, message)
}

/// Standard base64 with padding, decoded without pulling in a dependency.
///
/// This crate's whole purpose is being the one thing a plugin author's tooling
/// and the launcher agree on, so it stays dependency-light on principle. The
/// inputs are two fixed-size keys and signatures; anything malformed returns
/// `None` and is reported as a malformed file.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let text = text.trim();
    if text.is_empty() || !text.len().is_multiple_of(4) {
        return None;
    }
    // Padding is stripped first so the decode loop never has to reason about
    // where a `=` is allowed to appear — after this, any `=` left is in the
    // middle of the input, which is simply invalid.
    let body = text.trim_end_matches('=');
    if text.len() - body.len() > 2 {
        return None;
    }

    let mut accumulator: u32 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    for byte in body.bytes() {
        let value = TABLE.iter().position(|c| *c == byte)? as u32;
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    // Whatever is left over must be zero padding bits. A non-zero remainder
    // means two different texts would decode to the same bytes, and a
    // canonical form is worth more here than leniency.
    if accumulator & ((1 << bits) - 1) != 0 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_json() -> serde_json::Value {
        serde_json::json!({
            "id": "2026-09",
            "algorithm": "ed25519",
            // 32 bytes of 0x01.
            "publicKey": "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE="
        })
    }

    fn document_json() -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "id": "acme.dashboard",
            "sequence": 3,
            "releases": [{
                "version": "1.2.0",
                "url": "https://example.com/p-1.2.0.zip",
                "sha256": "a".repeat(64),
                "size": 4096,
                "apiRange": ">=0.1, <0.2"
            }],
            "signatures": [{
                "keyId": "2026-09",
                "algorithm": "ed25519",
                "value": "A".repeat(86) + "=="
            }]
        })
    }

    #[test]
    fn base64_round_trips_a_known_vector() {
        assert_eq!(base64_decode("AQID").unwrap(), vec![1, 2, 3]);
        assert_eq!(base64_decode("//8=").unwrap(), vec![255, 255]);
        assert_eq!(base64_decode("TWFu").unwrap(), b"Man".to_vec());
        assert_eq!(base64_decode("TWE=").unwrap(), b"Ma".to_vec());
    }

    #[test]
    fn base64_refuses_what_is_not_base64() {
        assert!(base64_decode("").is_none());
        assert!(base64_decode("AQI").is_none(), "unpadded length");
        assert!(base64_decode("A*ID").is_none(), "illegal character");
        assert!(base64_decode("A===").is_none(), "too much padding");
    }

    #[test]
    fn a_key_decodes_to_exactly_thirty_two_bytes() {
        let key: PublicKey = serde_json::from_value(key_json()).unwrap();
        key.validate().unwrap();
        assert_eq!(key.decoded_bytes().unwrap(), [1u8; 32]);
    }

    #[test]
    fn a_key_of_the_wrong_length_is_refused_when_the_file_is_read() {
        let mut value = key_json();
        value["publicKey"] = serde_json::json!("AQID");
        let key: PublicKey = serde_json::from_value(value).unwrap();
        assert!(key.validate().is_err());
    }

    #[test]
    fn an_algorithm_this_build_cannot_verify_is_refused_rather_than_ignored() {
        let mut value = key_json();
        value["algorithm"] = serde_json::json!("rsa");
        let key: PublicKey = serde_json::from_value(value).unwrap();
        assert!(key.validate().is_err());
    }

    #[test]
    fn an_update_source_must_be_https_and_carry_a_key() {
        let source = UpdateSource::parse(
            &serde_json::json!({
                "schema": 1,
                "url": "https://example.com/u.json",
                "keys": [key_json()]
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(source.keys.len(), 1);
        assert!(source.key("2026-09").is_some());

        let insecure = serde_json::json!({
            "schema": 1, "url": "http://example.com/u.json", "keys": [key_json()]
        });
        assert!(UpdateSource::parse(&insecure.to_string()).is_err());

        let keyless = serde_json::json!({
            "schema": 1, "url": "https://example.com/u.json", "keys": []
        });
        assert!(UpdateSource::parse(&keyless.to_string()).is_err());
    }

    #[test]
    fn two_keys_cannot_share_an_id() {
        let value = serde_json::json!({
            "schema": 1,
            "url": "https://example.com/u.json",
            "keys": [key_json(), key_json()]
        });
        let error = UpdateSource::parse(&value.to_string()).unwrap_err();
        assert!(error.message.contains("share the id"), "{}", error.message);
    }

    /// The signature cannot cover itself, and must cover everything else.
    #[test]
    fn signing_bytes_exclude_the_signature_and_include_every_other_field() {
        let document = UpdateDocument::parse(&document_json().to_string()).unwrap();
        let bytes = String::from_utf8(document.signing_bytes()).unwrap();
        assert!(bytes.starts_with(SIGNING_CONTEXT));
        assert!(!bytes.contains("signatures"));
        for expected in ["schema", "acme.dashboard", "sequence", "1.2.0", "4096"] {
            assert!(bytes.contains(expected), "missing {expected} in {bytes}");
        }
    }

    /// A document reserialised with different key order or whitespace has to
    /// keep verifying, or every proxy and static host becomes a way to break
    /// signatures.
    #[test]
    fn signing_bytes_survive_reordering_and_reindentation() {
        let document = UpdateDocument::parse(&document_json().to_string()).unwrap();
        let reordered: serde_json::Value = serde_json::json!({
            "signatures": document_json()["signatures"],
            "releases": document_json()["releases"],
            "sequence": 3,
            "id": "acme.dashboard",
            "schema": 1
        });
        let pretty = serde_json::to_string_pretty(&reordered).unwrap();
        let same = UpdateDocument::parse(&pretty).unwrap();
        assert_eq!(document.signing_bytes(), same.signing_bytes());
    }

    /// Changing anything the client acts on must change the signed bytes.
    #[test]
    fn changing_a_release_changes_the_signed_bytes() {
        let document = UpdateDocument::parse(&document_json().to_string()).unwrap();
        let mut tampered = document_json();
        tampered["releases"][0]["sha256"] = serde_json::json!("b".repeat(64));
        let tampered = UpdateDocument::parse(&tampered.to_string()).unwrap();
        assert_ne!(document.signing_bytes(), tampered.signing_bytes());

        let mut rewound = document_json();
        rewound["sequence"] = serde_json::json!(2);
        let rewound = UpdateDocument::parse(&rewound.to_string()).unwrap();
        assert_ne!(document.signing_bytes(), rewound.signing_bytes());
    }

    #[test]
    fn a_document_without_a_signature_is_refused() {
        let mut value = document_json();
        value["signatures"] = serde_json::json!([]);
        assert!(UpdateDocument::parse(&value.to_string()).is_err());
    }

    #[test]
    fn a_release_must_be_https_and_carry_a_lowercase_sha256() {
        let mut value = document_json();
        value["releases"][0]["url"] = serde_json::json!("http://example.com/p.zip");
        assert!(UpdateDocument::parse(&value.to_string()).is_err());

        let mut value = document_json();
        value["releases"][0]["sha256"] = serde_json::json!("A".repeat(64));
        let error = UpdateDocument::parse(&value.to_string()).unwrap_err();
        assert!(error.message.contains("lowercase"), "{}", error.message);

        let mut value = document_json();
        value["releases"][0]["sha256"] = serde_json::json!("nothex");
        assert!(UpdateDocument::parse(&value.to_string()).is_err());
    }

    /// One version, one set of bytes. Without this a publisher could list the
    /// same version twice and leave the client choosing between them.
    #[test]
    fn a_version_cannot_be_listed_twice() {
        let mut value = document_json();
        let release = value["releases"][0].clone();
        value["releases"] = serde_json::json!([release.clone(), release]);
        let error = UpdateDocument::parse(&value.to_string()).unwrap_err();
        assert!(error.message.contains("twice"), "{}", error.message);
    }

    #[test]
    fn the_newest_compatible_release_skips_one_this_host_cannot_run() {
        let mut value = document_json();
        value["releases"] = serde_json::json!([
            {
                "version": "1.2.0", "url": "https://example.com/a.zip",
                "sha256": "a".repeat(64), "size": 1, "apiRange": ">=0.1, <0.2"
            },
            {
                "version": "2.0.0", "url": "https://example.com/b.zip",
                "sha256": "b".repeat(64), "size": 1, "apiRange": ">=0.2, <0.3"
            }
        ]);
        let document = UpdateDocument::parse(&value.to_string()).unwrap();
        let host = semver::Version::new(0, 1, 0);
        assert_eq!(
            document.newest_compatible(&host).unwrap().version,
            semver::Version::new(1, 2, 0)
        );
        // And the caller can still tell that something newer exists, so it can
        // say "needs a newer Agora" rather than "up to date".
        assert_eq!(
            document.newest().unwrap().version,
            semver::Version::new(2, 0, 0)
        );
    }

    #[test]
    fn a_document_for_another_schema_is_refused_before_anything_else() {
        let mut value = document_json();
        value["schema"] = serde_json::json!(2);
        assert!(UpdateDocument::parse(&value.to_string()).is_err());
    }

    #[test]
    fn canonical_json_sorts_keys_at_every_depth() {
        let value = serde_json::json!({ "b": 1, "a": { "d": 2, "c": [3, { "f": 4, "e": 5 }] } });
        assert_eq!(
            canonical_json(&value),
            r#"{"a":{"c":[3,{"e":5,"f":4}],"d":2},"b":1}"#
        );
    }
}
