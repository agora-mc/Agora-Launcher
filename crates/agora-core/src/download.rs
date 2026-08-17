use crate::error::{LauncherError, LauncherResult};
use crate::http_client::{self, ClientCategory, HttpClients};
use crate::loader_manifests;
use sha1::{Digest as Sha1Digest, Sha1};
use sha2::Sha256;

/// Raw SHA-256 hex digest of a byte slice.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Lowercase SHA-1 hex digest of a byte slice.
pub fn sha1_hex(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Produce a canonical serialization of a JSON value for stable hashing.
///
/// Sorts object keys recursively, drops `time` and `releaseTime` fields (which
/// Mojang changes when re-releasing the same version), and uses a deterministic
/// serializer with no extra whitespace. This is the same algorithm the compiler
/// generator uses when computing `version_json_sha256`.
pub fn canonical_version_json(value: &serde_json::Value) -> String {
    fn strip_and_sort(val: &serde_json::Value) -> serde_json::Value {
        match val {
            serde_json::Value::Object(map) => {
                let mut entries: Vec<_> = map
                    .iter()
                    .filter(|(key, _)| key.as_str() != "time" && key.as_str() != "releaseTime")
                    .map(|(key, value)| (key.clone(), strip_and_sort(value)))
                    .collect();
                entries.sort_by(|(a, _), (b, _)| a.cmp(b));
                serde_json::Value::Object(entries.into_iter().collect())
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(strip_and_sort).collect())
            }
            other => other.clone(),
        }
    }
    let cleaned = strip_and_sort(value);
    serde_json::to_string(&cleaned).expect("canonical_version_json serialization must not fail")
}

/// Download a mod artifact using the dedicated GitHub/Modrinth allowlist.
/// Both the initial URL and every redirect must remain HTTPS on port 443.
///
/// Uses the [`HttpClients`] category system: Modrinth URLs use the Modrinth
/// category; all others (GitHub, objects.githubusercontent, etc.) use GitHub.
pub async fn download_mod_bytes(clients: &HttpClients, url: &str) -> LauncherResult<Vec<u8>> {
    let parsed = reqwest::Url::parse(url).map_err(|_| LauncherError::UntrustedSource)?;
    let category = if parsed
        .host_str()
        .is_some_and(|h| h.contains("modrinth.com"))
    {
        ClientCategory::Modrinth
    } else {
        ClientCategory::GitHub
    };
    http_client::checked_get_bytes(clients, category, url).await
}

/// Download a registry-pinned artifact (SHA-256 pinned by the curator in the
/// signed manifest) from an arbitrary HTTPS host.
///
/// Host authorization comes from the pinned URL itself via
/// [`http_client::HostPolicy::SignedManifest`], so the host does not need a
/// compile-time allowlist entry. Every other gate — HTTPS scheme, port 443,
/// no userinfo, no IP literals, non-private DNS resolution — still applies,
/// as does same-host-only redirect re-validation.
pub async fn download_pinned_bytes(clients: &HttpClients, url: &str) -> LauncherResult<Vec<u8>> {
    let host = reqwest::Url::parse(url)
        .map_err(|_| LauncherError::UntrustedSource)?
        .host_str()
        .ok_or(LauncherError::UntrustedSource)?
        .to_string();
    http_client::checked_get_bytes_with_policy(
        clients,
        ClientCategory::PinnedArtifact,
        url,
        http_client::HostPolicy::SignedManifest(&host),
    )
    .await
}

/// Convenience: download registry-pinned bytes without an explicit
/// [`HttpClients`] instance. Prefer [`download_pinned_bytes`] when a shared
/// clients instance is available.
pub async fn download_pinned_bytes_standalone(url: &str) -> LauncherResult<Vec<u8>> {
    let clients = HttpClients::new().map_err(|e| LauncherError::Generic {
        code: "ERR_HTTP_CLIENT_INIT".into(),
        message: format!("Failed to initialize HTTP clients: {e}"),
    })?;
    download_pinned_bytes(&clients, url).await
}

/// Download content the user explicitly opted into (Technic tiers S/Z).
///
/// This is the ONLY path that may use
/// [`ClientCategory::ConsentedContent`](http_client::ClientCategory::ConsentedContent)
/// with [`HostPolicy::UserConsented`](http_client::HostPolicy::UserConsented).
/// Callers must have already checked the user's consent settings for the
/// content's tier in core (a frontend bug cannot widen this because the check
/// lives in core). Every fetch is logged with its resolved host so a network
/// audit can prove the category is never used without consent. The floor still
/// applies: private/loopback/link-local destinations are refused, redirects
/// are re-validated per hop, and per-category size caps are enforced.
pub async fn download_consented_bytes(clients: &HttpClients, url: &str) -> LauncherResult<Vec<u8>> {
    let host = reqwest::Url::parse(url)
        .map_err(|_| LauncherError::UntrustedSource)?
        .host_str()
        .unwrap_or("")
        .to_string();
    eprintln!(
        "[consented-content] fetch host={host} url={}",
        crate::network::sanitized_url_for_log(url)
    );
    http_client::checked_get_bytes_with_policy(
        clients,
        ClientCategory::ConsentedContent,
        url,
        http_client::HostPolicy::UserConsented,
    )
    .await
}

/// Convenience: download consented content without an explicit
/// [`HttpClients`] instance. Prefer [`download_consented_bytes`] when a shared
/// clients instance is available.
pub async fn download_consented_bytes_standalone(url: &str) -> LauncherResult<Vec<u8>> {
    let clients = HttpClients::new().map_err(|e| LauncherError::Generic {
        code: "ERR_HTTP_CLIENT_INIT".into(),
        message: format!("Failed to initialize HTTP clients: {e}"),
    })?;
    download_consented_bytes(&clients, url).await
}

/// Download a complete modpack archive through the stricter pack host
/// allowlist and the 500 MiB archive limit used by the importer.
pub async fn download_modpack_bytes(clients: &HttpClients, url: &str) -> LauncherResult<Vec<u8>> {
    http_client::checked_get_bytes(clients, ClientCategory::Modpack, url).await
}

/// Download a modpack archive and report cumulative body progress.
pub async fn download_modpack_bytes_with_progress<F>(
    clients: &HttpClients,
    url: &str,
    on_progress: F,
) -> LauncherResult<Vec<u8>>
where
    F: FnMut(u64, Option<u64>),
{
    http_client::checked_get_bytes_with_progress(clients, ClientCategory::Modpack, url, on_progress)
        .await
}

/// Convenience: download mod bytes without an explicit HttpClients instance.
/// Creates a properly-configured [`HttpClients`] internally.
/// Prefer [`download_mod_bytes`] when a shared clients instance is available.
pub async fn download_mod_bytes_standalone(url: &str) -> LauncherResult<Vec<u8>> {
    let clients = HttpClients::new().map_err(|e| LauncherError::Generic {
        code: "ERR_HTTP_CLIENT_INIT".into(),
        message: format!("Failed to initialize HTTP clients: {e}"),
    })?;
    download_mod_bytes(&clients, url).await
}

/// Deterministic SHA-256 of a JSON payload after stripping volatile keys.
///
/// Fabric rewrites `time`/`releaseTime` on every request, so the raw response hash
/// is unstable. This mirrors `_stable_json_sha256` in `scripts/fetch_loader_manifests.py`.
pub fn stable_json_sha256(data: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(data).ok()?;
    let mut obj = value;
    if let serde_json::Value::Object(map) = &mut obj {
        map.remove("time");
        map.remove("releaseTime");
    }
    let canonical = serde_json::to_string(&obj).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    Some(hex::encode(hasher.finalize()))
}

/// Compute the expected hash for a loader file. Profile JSONs (Fabric/Quilt)
/// use the stable normalized hash; installer jars use the raw hash.
pub fn compute_loader_hash(loader: &str, _file_name: &str, file_type: &str, data: &[u8]) -> String {
    if file_type == "profile_json" && (loader == "fabric" || loader == "quilt") {
        if let Some(stable) = stable_json_sha256(data) {
            return stable;
        }
    }
    sha256_hex(data)
}

/// Download bytes from a URL using a redirect-safe client.
///
/// Redirects are only followed when the target host is on the embedded loader
/// domain allowlist, preventing SSRF via compromised/malicious pinned hosts.
pub async fn download_bytes(url: &str) -> LauncherResult<Vec<u8>> {
    loader_manifests::ensure_allowed_domain(url).inspect_err(|_error| {
        eprintln!(
            "[loader-download] rejected stage=initial-allowlist url={}",
            crate::network::sanitized_url_for_log(url)
        );
    })?;
    let clients = HttpClients::new()?;
    http_client::checked_get_bytes(&clients, ClientCategory::Loader, url).await
}

/// Download bytes through an already initialized category-aware client set.
pub async fn download_bytes_with_clients(
    clients: &HttpClients,
    url: &str,
) -> LauncherResult<Vec<u8>> {
    loader_manifests::ensure_allowed_domain(url).inspect_err(|_error| {
        eprintln!(
            "[loader-download] rejected stage=initial-allowlist url={}",
            crate::network::sanitized_url_for_log(url)
        );
    })?;
    http_client::checked_get_bytes(clients, ClientCategory::Loader, url).await
}

/// Download a loader file and verify its hash against the pinned value.
pub async fn download_verified(
    loader: &str,
    file_name: &str,
    file_type: &str,
    url: &str,
    expected_sha: &str,
) -> LauncherResult<Vec<u8>> {
    loader_manifests::ensure_allowed_domain(url).inspect_err(|_error| {
        eprintln!(
            "[loader-download] rejected stage=verified-initial loader={loader} file={file_name} url={}",
            crate::network::sanitized_url_for_log(url)
        );
    })?;
    let data = download_bytes(url).await?;
    let actual = compute_loader_hash(loader, file_name, file_type, &data);

    if actual != loader_manifests::strip_sha_prefix(expected_sha) {
        return Err(LauncherError::HashMismatch);
    }
    Ok(data)
}

/// Download and verify a pinned loader file with shared core clients.
pub async fn download_verified_with_clients(
    clients: &HttpClients,
    loader: &str,
    file_name: &str,
    file_type: &str,
    url: &str,
    expected_sha: &str,
) -> LauncherResult<Vec<u8>> {
    let data = download_bytes_with_clients(clients, url).await?;
    let actual = compute_loader_hash(loader, file_name, file_type, &data);
    if actual != loader_manifests::strip_sha_prefix(expected_sha) {
        return Err(LauncherError::HashMismatch);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn stable_json_sha256_is_order_independent() {
        // Verify that reordering keys does not change the hash.
        let input1 = br#"{"a":1,"b":2}"#;
        let input2 = br#"{"b":2,"a":1}"#;
        let hash1 = stable_json_sha256(input1).unwrap();
        let hash2 = stable_json_sha256(input2).unwrap();
        assert_eq!(hash1, hash2, "reordering keys must produce the same hash");
    }

    #[test]
    fn stable_json_sha256_matches_python_canonicalization() {
        // This test vector must correspond to what the Python script
        // (`scripts/fetch_loader_manifests.py`, `_stable_json_sha256`)
        // computes for the same input.
        let input = br#"{"id":"1.21","mainClass":"net.minecraft.client.main.Main","time":"2024-06-13T15:00:00+00:00","releaseTime":"2024-06-13T15:00:00+00:00","type":"release"}"#;
        // Python sorts keys with `sort_keys=True` after dropping time/releaseTime.
        // The expected canonical is: {"id":"1.21","mainClass":"net.minecraft.client.main.Main","type":"release"}
        let hash = stable_json_sha256(input).unwrap();
        // Compute expected: SHA-256 of '{"id":"1.21","mainClass":"net.minecraft.client.main.Main","type":"release"}'
        let canonical =
            br#"{"id":"1.21","mainClass":"net.minecraft.client.main.Main","type":"release"}"#;
        let mut hasher = Sha256::new();
        hasher.update(canonical);
        let expected = hex::encode(hasher.finalize());
        assert_eq!(
            hash, expected,
            "stable_json_sha256 must match Python-style canonicalization"
        );
    }

    #[test]
    fn canonical_version_json_rust_python_parity() {
        // This hardcoded test vector ensures Rust and Python produce the
        // same canonical JSON hash for a realistic version.json.
        // Python (scripts/fetch_loader_manifests.py _stable_json_sha256):
        //   1. Parse JSON
        //   2. Drop "time" and "releaseTime" keys
        //   3. Serialize with sort_keys=True, separators=(',', ':'), ensure_ascii=True
        //   4. SHA-256 of UTF-8 bytes
        //
        // Rust (crate::download::canonical_version_json):
        //   1. Parse JSON
        //   2. Recursively strip "time" and "releaseTime"
        //   3. Sort object keys recursively
        //   4. Serialize with serde_json::to_string (compact, no whitespace)
        //   5. SHA-256 of UTF-8 bytes
        let version_json = serde_json::json!({
            "id": "forge-1.21-47.1.0",
            "inheritsFrom": "1.21",
            "time": "2024-07-01T12:00:00Z",
            "releaseTime": "2024-07-01T12:00:00Z",
            "mainClass": "net.minecraftforge.Main",
            "libraries": [{"name": "net.minecraftforge:forge:47.1.0"}],
            "type": "release"
        });
        let canonical = super::canonical_version_json(&version_json);
        let hash = super::sha256_hex(canonical.as_bytes());

        // Verify structure: no time/releaseTime, keys sorted
        assert!(!canonical.contains("\"time\""), "must drop time");
        assert!(
            !canonical.contains("\"releaseTime\""),
            "must drop releaseTime"
        );
        // id should appear before inheritsFrom (sorted)
        assert!(
            canonical.find("\"id\"").unwrap() < canonical.find("\"inheritsFrom\"").unwrap(),
            "id must sort before inheritsFrom"
        );

        // Verify this is deterministic
        let canonical2 = super::canonical_version_json(&version_json);
        let hash2 = super::sha256_hex(canonical2.as_bytes());
        assert_eq!(hash, hash2, "canonical JSON hash must be deterministic");

        // Cross-language parity: the Python script with the same input
        // should produce the same hash. This is the expected value from
        // running: _stable_json_sha256(json.dumps({"id":"forge-...","inheritsFrom":"1.21","mainClass":"net.minecraftforge.Main","libraries":[{"name":"net.minecraftforge:forge:47.1.0"}],"type":"release"}))
        // with time/releaseTime removed and sort_keys=True.
        let expected_python_canonical = r#"{"id":"forge-1.21-47.1.0","inheritsFrom":"1.21","libraries":[{"name":"net.minecraftforge:forge:47.1.0"}],"mainClass":"net.minecraftforge.Main","type":"release"}"#;
        assert_eq!(
            canonical, expected_python_canonical,
            "Rust canonical JSON must match Python"
        );
    }
}
