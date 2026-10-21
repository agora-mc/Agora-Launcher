//! Downloading pinned mod-loader files: only from hosts on the loader
//! catalog's allowlist, and verified against the catalog's hash.

use crate::loader_manifests;
use agora_core::download::{sha256_hex, stable_json_sha256};
use agora_core::error::{LauncherError, LauncherResult};
use agora_core::http_client::{self, ClientCategory, HttpClients};

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
            agora_core::network::sanitized_url_for_log(url)
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
            agora_core::network::sanitized_url_for_log(url)
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
            agora_core::network::sanitized_url_for_log(url)
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
