//! Small, durable receipts for artifacts that have already passed integrity
//! verification.
//!
//! A receipt is only a warm-path optimization.  It is accepted when the file's
//! size, high-resolution modification time, and platform file identity still
//! match the values captured immediately after verification.  Missing or
//! malformed receipts always fall back to the caller's full hash check.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

const RECEIPT_SCHEMA_VERSION: u32 = 1;
static RECEIPT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactReceipt {
    schema_version: u32,
    algorithm: String,
    expected_hash: String,
    expected_size: Option<u64>,
    file_size: u64,
    modified_ns: u128,
    file_identity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileState {
    file_size: u64,
    modified_ns: u128,
    file_identity: Option<String>,
}

/// Return whether a previously verified artifact can be accepted without
/// reopening and hashing its complete contents.
pub fn is_verified(
    path: &Path,
    algorithm: &str,
    expected_hash: &str,
    expected_size: Option<i64>,
) -> bool {
    if expected_size.is_some_and(|size| size < 0) {
        return false;
    }
    let Some(state) = file_state(path) else {
        return false;
    };
    let Some(receipt) = read_receipt(path) else {
        return false;
    };
    let normalized_hash = expected_hash.to_ascii_lowercase();
    let expected_size = normalized_size(expected_size);
    receipt.schema_version == RECEIPT_SCHEMA_VERSION
        && receipt.algorithm == algorithm
        && receipt.expected_hash == normalized_hash
        && receipt.expected_size == expected_size
        && receipt.file_size == state.file_size
        && receipt.modified_ns == state.modified_ns
        && receipt.file_identity == state.file_identity
}

/// Record a successful verification.  Callers should invoke this only after
/// their normal cryptographic and size checks have succeeded.
pub fn record_verified(
    path: &Path,
    algorithm: &str,
    expected_hash: &str,
    expected_size: Option<i64>,
) -> Result<(), String> {
    if expected_size.is_some_and(|size| size < 0) {
        return Err("cannot record a verification receipt with a negative size".into());
    }
    let state = file_state(path).ok_or_else(|| {
        format!(
            "cannot record verification for missing artifact {}",
            path.display()
        )
    })?;
    let expected_size = normalized_size(expected_size);
    if expected_size.is_some_and(|size| size != state.file_size) {
        return Err(format!(
            "artifact size changed before verification receipt was written: {}",
            path.display()
        ));
    }

    let receipt = ArtifactReceipt {
        schema_version: RECEIPT_SCHEMA_VERSION,
        algorithm: algorithm.to_string(),
        expected_hash: expected_hash.to_ascii_lowercase(),
        expected_size,
        file_size: state.file_size,
        modified_ns: state.modified_ns,
        file_identity: state.file_identity,
    };
    let bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|error| format!("failed to serialize artifact receipt: {error}"))?;
    let receipt_path = receipt_path(path);
    let parent = receipt_path
        .parent()
        .ok_or_else(|| format!("artifact receipt has no parent: {}", receipt_path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create artifact receipt directory: {error}"))?;
    let temp = parent.join(format!(
        ".{}.agora-receipt-{}-{}.tmp",
        receipt_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("artifact"),
        std::process::id(),
        RECEIPT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let write_result = (|| {
        let mut file = std::fs::File::create(&temp)
            .map_err(|error| format!("failed to create artifact receipt temp file: {error}"))?;
        std::io::Write::write_all(&mut file, &bytes)
            .map_err(|error| format!("failed to write artifact receipt: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("failed to sync artifact receipt: {error}"))?;
        Ok::<_, String>(())
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }

    // Windows does not replace an existing file with rename.  A receipt is
    // disposable metadata, so replacing it is safe; the artifact itself is
    // never removed by this operation.
    let _ = std::fs::remove_file(&receipt_path);
    if let Err(error) = std::fs::rename(&temp, &receipt_path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("failed to commit artifact receipt: {error}"));
    }
    Ok(())
}

fn normalized_size(size: Option<i64>) -> Option<u64> {
    size.and_then(|value| u64::try_from(value).ok())
}

fn receipt_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact");
    path.with_file_name(format!("{file_name}.agora-verified.json"))
}

fn read_receipt(path: &Path) -> Option<ArtifactReceipt> {
    let bytes = std::fs::read(receipt_path(path)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn file_state(path: &Path) -> Option<FileState> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified_ns = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(FileState {
        file_size: metadata.len(),
        modified_ns,
        file_identity: platform_file_identity(&metadata),
    })
}

#[cfg(unix)]
fn platform_file_identity(metadata: &std::fs::Metadata) -> Option<String> {
    Some(format!("unix:{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn platform_file_identity(metadata: &std::fs::Metadata) -> Option<String> {
    Some(format!(
        "windows:creation-time:{}",
        metadata.creation_time()
    ))
}

#[cfg(not(any(unix, windows)))]
fn platform_file_identity(_metadata: &std::fs::Metadata) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_accepts_unchanged_verified_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("client.jar");
        std::fs::write(&path, b"verified").unwrap();
        record_verified(&path, "sha1", "abc", Some(8)).unwrap();
        assert!(is_verified(&path, "sha1", "ABC", Some(8)));
    }

    #[test]
    fn receipt_rejects_metadata_change() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("client.jar");
        std::fs::write(&path, b"verified").unwrap();
        record_verified(&path, "sha1", "abc", Some(8)).unwrap();
        std::fs::write(&path, b"changed").unwrap();
        assert!(!is_verified(&path, "sha1", "abc", Some(8)));
    }

    #[test]
    fn receipt_rejects_replaced_file_with_same_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("client.jar");
        std::fs::write(&path, b"verified").unwrap();
        record_verified(&path, "sha1", "abc", Some(8)).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"verified").unwrap();
        assert!(!is_verified(&path, "sha1", "abc", Some(8)));
    }
}
