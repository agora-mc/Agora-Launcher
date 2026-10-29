use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedJar {
    pub sha256: String,
    pub filename: String,
    pub size_bytes: u64,
    pub cached_at: String,
}

/// Content-addressed mod cache.
pub struct ModCache {
    cache_dir: PathBuf,
}

impl ModCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        ModCache { cache_dir }
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Store a JAR in the cache. Copies the file, verifies SHA-256, returns the cache path.
    /// If already cached (same hash), returns existing path without re-copying.
    pub fn store_jar(&self, jar_path: &Path, expected_sha256: &str) -> Result<PathBuf, String> {
        let data = fs::read(jar_path).map_err(|e| format!("failed to read jar: {}", e))?;
        let actual_sha256 = agora_core::download::sha256_hex(&data);
        if actual_sha256 != expected_sha256 {
            return Err(format!(
                "SHA-256 mismatch: expected {} got {}",
                expected_sha256, actual_sha256
            ));
        }

        let cache_path = self.cache_path_for(&actual_sha256);
        if cache_path.exists() {
            return Ok(cache_path);
        }

        let parent = cache_path
            .parent()
            .ok_or_else(|| "Invalid cache path".to_string())?;
        fs::create_dir_all(parent).map_err(|e| format!("failed to create cache dir: {}", e))?;
        fs::copy(jar_path, &cache_path).map_err(|e| format!("failed to copy to cache: {}", e))?;

        let metadata = CachedJar {
            sha256: actual_sha256.clone(),
            filename: jar_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string(),
            size_bytes: data.len() as u64,
            cached_at: chrono::Utc::now().to_rfc3339(),
        };
        // Metadata belongs to the content-addressed hash directory, alongside
        // `file.jar`. `list_cached` reads this canonical filename; the old
        // `file.json` location made newly cached jars invisible to cache
        // management and size accounting.
        let meta_path = cache_path
            .parent()
            .ok_or_else(|| "Invalid cache path".to_string())?
            .join("metadata.json");
        fs::write(&meta_path, serde_json::to_string_pretty(&metadata).unwrap())
            .map_err(|e| format!("failed to write metadata: {}", e))?;

        Ok(cache_path)
    }

    /// Hardlink (or copy) a cached JAR into a target directory with the given filename.
    /// Returns the created path. Falls back to copy if hardlink fails (cross-device, FAT).
    pub fn resolve_and_link(
        &self,
        sha256: &str,
        target_dir: &Path,
        filename: &str,
    ) -> Result<PathBuf, String> {
        let cache_path = self.cache_path_for(sha256);
        if !cache_path.exists() {
            return Err(format!("cached jar not found for hash {}", sha256));
        }

        let target_path = target_dir.join(filename);
        fs::create_dir_all(target_dir)
            .map_err(|e| format!("failed to create target dir: {}", e))?;

        if fs::hard_link(&cache_path, &target_path).is_err() {
            fs::copy(&cache_path, &target_path)
                .map_err(|e| format!("failed to copy jar: {}", e))?;
        }

        Ok(target_path)
    }

    /// Check if a JAR is in the cache.
    pub fn has_jar(&self, sha256: &str) -> bool {
        self.cache_path_for(sha256).exists()
    }

    /// List all cached JARs.
    pub fn list_cached(&self) -> Result<Vec<CachedJar>, String> {
        let mut jars = Vec::new();
        if !self.cache_dir.exists() {
            return Ok(jars);
        }

        let entries = fs::read_dir(&self.cache_dir)
            .map_err(|e| format!("failed to read cache dir: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("failed to read entry: {}", e))?;
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }

            let hash_entries = fs::read_dir(entry.path())
                .map_err(|e| format!("failed to read hash dir: {}", e))?;

            for hash_entry in hash_entries {
                let hash_entry =
                    hash_entry.map_err(|e| format!("failed to read hash entry: {}", e))?;
                if !hash_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }

                let metadata_path = hash_entry.path().join("metadata.json");
                // Keep existing player caches discoverable after correcting
                // the old writer, which placed metadata beside file.jar as
                // file.json.
                let meta_path = if metadata_path.exists() {
                    metadata_path
                } else {
                    hash_entry.path().join("file.json")
                };
                if meta_path.exists() {
                    let content = fs::read_to_string(&meta_path)
                        .map_err(|e| format!("failed to read metadata: {}", e))?;
                    let jar: CachedJar = serde_json::from_str(&content)
                        .map_err(|e| format!("failed to parse metadata: {}", e))?;
                    jars.push(jar);
                }
            }
        }

        Ok(jars)
    }

    /// Total cache size in bytes.
    pub fn total_size(&self) -> Result<u64, String> {
        let jars = self.list_cached()?;
        Ok(jars.iter().map(|j| j.size_bytes).sum())
    }

    /// Store verified bytes under their own SHA-256 and return that hash.
    ///
    /// The write is atomic: the bytes go to a uniquely named temporary file in
    /// the entry directory and are renamed into place, so a concurrent reader
    /// sees either no entry or a complete one, never a half-written jar.
    pub fn store_bytes(&self, bytes: &[u8], filename: &str) -> Result<String, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let sha256 = agora_core::download::sha256_hex(bytes);
        // Already present and intact (another install stored it): leave it
        // alone rather than racing a reader with a replace.
        if self.read_verified(&sha256).is_ok() {
            return Ok(sha256);
        }
        let cache_path = self.cache_path_for(&sha256);
        let dir = cache_path
            .parent()
            .ok_or_else(|| "Invalid cache path".to_string())?;
        if !plain_path_under(&self.cache_dir, dir) {
            return Err("cache path passes through a link".into());
        }
        fs::create_dir_all(dir).map_err(|e| format!("failed to create cache dir: {e}"))?;
        if !plain_path_under(&self.cache_dir, dir) {
            return Err("cache path passes through a link".into());
        }

        let tmp = dir.join(format!(
            "file.jar.{}.{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let write = || -> Result<(), String> {
            use std::io::Write;
            let mut file =
                fs::File::create(&tmp).map_err(|e| format!("failed to create cache file: {e}"))?;
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|e| format!("failed to write cache file: {e}"))?;
            drop(file);
            // A same-hash entry may already exist (another install won the
            // race, or it is corrupt). Replace it: the content is the one we
            // just hashed.
            if cache_path.exists() {
                let _ = fs::remove_file(&cache_path);
            }
            fs::rename(&tmp, &cache_path).map_err(|e| format!("failed to finalize cache file: {e}"))
        };
        if let Err(error) = write() {
            let _ = fs::remove_file(&tmp);
            // Another writer may have finished the same entry meanwhile.
            if self.read_verified(&sha256).is_ok() {
                return Ok(sha256);
            }
            return Err(error);
        }

        let metadata = CachedJar {
            sha256: sha256.clone(),
            filename: filename.to_string(),
            size_bytes: bytes.len() as u64,
            cached_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&metadata) {
            let _ = fs::write(dir.join("metadata.json"), json);
        }
        Ok(sha256)
    }

    /// Read an entry and confirm its bytes still hash to the key. Any error,
    /// including an absent entry, means "not usable".
    pub fn read_verified(&self, sha256: &str) -> Result<Vec<u8>, String> {
        if !is_sha256_hex(sha256) {
            return Err("not a SHA-256 key".into());
        }
        if !plain_path_under(&self.cache_dir, &self.cache_path_for(sha256)) {
            return Err("cache path passes through a link".into());
        }
        let bytes =
            fs::read(self.cache_path_for(sha256)).map_err(|e| format!("cache read failed: {e}"))?;
        if agora_core::download::sha256_hex(&bytes) != sha256 {
            return Err("cache entry does not match its hash".into());
        }
        Ok(bytes)
    }

    /// Record a use of an entry, so eviction treats it as recently needed.
    pub fn mark_used(&self, sha256: &str) {
        if let Some(dir) = self.entry_dir(sha256) {
            if !plain_path_under(&self.cache_dir, &dir) {
                return;
            }
            let _ = fs::write(dir.join("used"), b"");
        }
    }

    /// Remove one entry. Only a well-formed `<xx>/<64 hex>` directory under the
    /// cache root is ever deleted, and never through a symlink or junction.
    /// Hard links made from an entry (see `resolve_and_link`) are separate
    /// directory entries for the same data, so removing the entry cannot break
    /// a file already linked into an instance.
    pub fn remove_entry(&self, sha256: &str) -> Result<(), String> {
        let dir = self
            .entry_dir(sha256)
            .ok_or_else(|| "not a SHA-256 key".to_string())?;
        // Every component between the cache root and the entry must be a real
        // directory; a link at the prefix folder would aim the delete elsewhere.
        if !plain_path_under(&self.cache_dir, &dir) {
            return Err("refusing to remove through a link".into());
        }
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_dir() && !is_link_like(&meta) => {
                fs::remove_dir_all(&dir).map_err(|e| format!("failed to remove entry: {e}"))
            }
            Ok(_) => Err("refusing to remove a non-directory cache entry".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("failed to inspect entry: {e}")),
        }
    }

    /// Every well-formed entry found on disk, discovered from the directory
    /// layout (not metadata files, which may be missing or damaged). Anything
    /// that does not look like an entry is ignored and never touched.
    pub fn entries(&self) -> Vec<CacheEntryInfo> {
        let mut out = Vec::new();
        let Ok(prefixes) = fs::read_dir(&self.cache_dir) else {
            return out;
        };
        for prefix in prefixes.flatten() {
            let Some(prefix_name) = prefix.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if prefix_name.len() != 2
                || !is_lower_hex(&prefix_name)
                || !plain_path_under(&self.cache_dir, &prefix.path())
            {
                continue;
            }
            let Ok(hashes) = fs::read_dir(prefix.path()) else {
                continue;
            };
            for hash in hashes.flatten() {
                let Some(name) = hash.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if !is_sha256_hex(&name)
                    || !name.starts_with(&prefix_name)
                    || !plain_path_under(&self.cache_dir, &hash.path())
                {
                    continue;
                }
                let Ok(jar_meta) = fs::metadata(hash.path().join("file.jar")) else {
                    continue;
                };
                let modified = |p: PathBuf| fs::metadata(p).and_then(|m| m.modified()).ok();
                let last_used = [jar_meta.modified().ok(), modified(hash.path().join("used"))]
                    .into_iter()
                    .flatten()
                    .max();
                out.push(CacheEntryInfo {
                    sha256: name,
                    size_bytes: jar_meta.len(),
                    last_used,
                });
            }
        }
        out
    }

    /// Bound the cache. See [`EvictionPolicy`]. `is_protected` names entries an
    /// in-flight install holds; they are never removed. Anything that cannot be
    /// inspected or removed is left alone.
    pub fn evict(
        &self,
        policy: &EvictionPolicy,
        now: std::time::SystemTime,
        is_protected: &dyn Fn(&str) -> bool,
    ) -> EvictionReport {
        let mut report = EvictionReport::default();
        let mut entries = self.entries();
        // Oldest first.
        entries.sort_by_key(|e| e.last_used);
        let age = |e: &CacheEntryInfo| match e.last_used {
            Some(t) => match now.duration_since(t) {
                Ok(age) => age,
                // A last use in the future means the clock moved; the stamp is
                // untrustworthy. Count it as just past the grace period: the
                // size bound applies, but it is not aged out wholesale.
                Err(_) => policy.grace,
            },
            None => policy.grace,
        };
        self.sweep_leftovers(policy, now, &mut report, is_protected);
        let mut total: u64 = entries.iter().map(|e| e.size_bytes).sum();
        for entry in &entries {
            let evictable = !is_protected(&entry.sha256) && age(entry) >= policy.grace;
            if !evictable {
                report.kept_in_use += 1;
                continue;
            }
            let too_old = age(entry) > policy.max_age;
            let too_big = total > policy.max_bytes;
            if !(too_old || too_big) {
                continue;
            }
            match self.remove_entry(&entry.sha256) {
                Ok(()) => {
                    total = total.saturating_sub(entry.size_bytes);
                    report.removed += 1;
                    report.bytes_freed += entry.size_bytes;
                }
                Err(_) => report.failed += 1,
            }
        }
        report
    }

    /// Clean what a crash leaves behind: temporary files and entry folders
    /// with no jar, once older than the grace period. Only well-formed entry
    /// folders reached without crossing a link are looked at.
    fn sweep_leftovers(
        &self,
        policy: &EvictionPolicy,
        now: std::time::SystemTime,
        report: &mut EvictionReport,
        is_protected: &dyn Fn(&str) -> bool,
    ) {
        let old_enough = |p: &Path| {
            fs::symlink_metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .is_some_and(|t| now.duration_since(t).map_or(true, |a| a >= policy.grace))
        };
        let Ok(prefixes) = fs::read_dir(&self.cache_dir) else {
            return;
        };
        for prefix in prefixes.flatten() {
            let Some(pname) = prefix.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if pname.len() != 2
                || !is_lower_hex(&pname)
                || !plain_path_under(&self.cache_dir, &prefix.path())
            {
                continue;
            }
            let Ok(hashes) = fs::read_dir(prefix.path()) else {
                continue;
            };
            for hash in hashes.flatten() {
                let Some(name) = hash.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let dir = hash.path();
                if !is_sha256_hex(&name)
                    || !name.starts_with(&pname)
                    || is_protected(&name)
                    || !plain_path_under(&self.cache_dir, &dir)
                {
                    continue;
                }
                let Ok(files) = fs::read_dir(&dir) else {
                    continue;
                };
                let (mut has_jar, mut other) = (false, false);
                for file in files.flatten() {
                    let fname = file.file_name().to_string_lossy().into_owned();
                    let path = file.path();
                    let is_file = fs::symlink_metadata(&path)
                        .map(|m| m.file_type().is_file())
                        .unwrap_or(false);
                    if fname == "file.jar" {
                        has_jar = true;
                    } else if fname.starts_with("file.jar.") && fname.ends_with(".tmp") && is_file {
                        if old_enough(&path) && fs::remove_file(&path).is_ok() {
                            report.tmp_removed += 1;
                        } else {
                            other = true;
                        }
                    } else if fname != "metadata.json" && fname != "used" {
                        other = true;
                    }
                }
                // A folder with no jar and nothing unexpected in it is not an
                // entry. `remove_dir` only succeeds when it is empty.
                if !has_jar && !other && old_enough(&dir) {
                    for leftover in ["metadata.json", "used"] {
                        let _ = fs::remove_file(dir.join(leftover));
                    }
                    if fs::remove_dir(&dir).is_ok() {
                        report.empty_dirs_removed += 1;
                    }
                }
            }
        }
    }

    fn entry_dir(&self, sha256: &str) -> Option<PathBuf> {
        is_sha256_hex(sha256).then(|| self.cache_dir.join(&sha256[..2]).join(sha256))
    }

    fn cache_path_for(&self, sha256: &str) -> PathBuf {
        let prefix = &sha256[..2.min(sha256.len())];
        self.cache_dir.join(prefix).join(sha256).join("file.jar")
    }
}

/// How a cache is kept bounded.
///
/// Mod jars are re-downloadable, so the cache is a convenience and eviction is
/// always safe for correctness; the policy only has to avoid hurting a running
/// install. Entries go oldest-use first when the total exceeds `max_bytes`, and
/// any entry unused for longer than `max_age` goes regardless. An entry used
/// within `grace` is never removed (this protects installs in other processes,
/// which cannot be seen from here), nor is one the caller names as protected.
/// The bound is therefore soft while entries are in use.
#[derive(Debug, Clone)]
pub struct EvictionPolicy {
    pub max_bytes: u64,
    pub max_age: std::time::Duration,
    pub grace: std::time::Duration,
}

impl Default for EvictionPolicy {
    fn default() -> Self {
        EvictionPolicy {
            max_bytes: 512 * 1024 * 1024,
            max_age: std::time::Duration::from_secs(30 * 24 * 3600),
            grace: std::time::Duration::from_secs(3600),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheEntryInfo {
    pub sha256: String,
    pub size_bytes: u64,
    pub last_used: Option<std::time::SystemTime>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EvictionReport {
    pub removed: usize,
    pub bytes_freed: u64,
    pub kept_in_use: usize,
    pub failed: usize,
    /// Crash-left temporary files removed.
    pub tmp_removed: usize,
    /// Entry folders with no jar removed.
    pub empty_dirs_removed: usize,
}

fn is_lower_hex(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A SHA-256 cache key: exactly 64 lowercase hex characters.
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && is_lower_hex(s)
}

/// True only if every existing component of `target` below `root` is a real
/// file or directory: no symlink, junction or other reparse point. Any doubt
/// (cannot inspect, not under root) is false. `root` itself may be a link,
/// since that is the user's own choice of location.
pub(crate) fn plain_path_under(root: &Path, target: &Path) -> bool {
    let Ok(relative) = target.strip_prefix(root) else {
        return false;
    };
    let mut current = root.to_path_buf();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(part) => current.push(part),
            _ => return false,
        }
        match fs::symlink_metadata(&current) {
            Ok(meta) if is_link_like(&meta) => return false,
            Ok(_) => {}
            // Not there yet: nothing further down can be a link.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        }
    }
    true
}

pub(crate) fn is_link_like(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT: junctions and symlinks.
        meta.file_attributes() & 0x400 != 0 || meta.file_type().is_symlink()
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn store_jar_and_has_jar() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("cache"));

        let jar_dir = tmp.path().join("input");
        fs::create_dir_all(&jar_dir).unwrap();
        let jar_path = jar_dir.join("test.jar");
        fs::write(&jar_path, b"hello world").unwrap();

        let expected = agora_core::download::sha256_hex(b"hello world");
        let stored = cache.store_jar(&jar_path, &expected).unwrap();
        assert!(stored.exists());
        assert!(cache.has_jar(&expected));
    }

    #[test]
    fn store_jar_deduplicates() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("cache"));

        let jar_dir = tmp.path().join("input");
        fs::create_dir_all(&jar_dir).unwrap();
        let jar_path = jar_dir.join("test.jar");
        fs::write(&jar_path, b"same content").unwrap();

        let expected = agora_core::download::sha256_hex(b"same content");
        let first = cache.store_jar(&jar_path, &expected).unwrap();
        let second = cache.store_jar(&jar_path, &expected).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn store_jar_rejects_hash_mismatch() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("cache"));

        let jar_dir = tmp.path().join("input");
        fs::create_dir_all(&jar_dir).unwrap();
        let jar_path = jar_dir.join("test.jar");
        fs::write(&jar_path, b"content").unwrap();

        let result = cache.store_jar(&jar_path, "deadbeef");
        assert!(result.is_err());
    }

    #[test]
    fn resolve_and_link_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("cache"));

        let jar_dir = tmp.path().join("input");
        fs::create_dir_all(&jar_dir).unwrap();
        let jar_path = jar_dir.join("test.jar");
        fs::write(&jar_path, b"mod data").unwrap();

        let expected = agora_core::download::sha256_hex(b"mod data");
        cache.store_jar(&jar_path, &expected).unwrap();

        let target_dir = tmp.path().join("mods");
        let linked = cache
            .resolve_and_link(&expected, &target_dir, "mymod.jar")
            .unwrap();
        assert!(linked.exists());
        assert_eq!(fs::read(&linked).unwrap(), b"mod data");
    }

    #[test]
    fn list_cached_returns_stored_jars() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("cache"));

        let jar_dir = tmp.path().join("input");
        fs::create_dir_all(&jar_dir).unwrap();
        let jar_path = jar_dir.join("test.jar");
        fs::write(&jar_path, b"list test").unwrap();

        let expected = agora_core::download::sha256_hex(b"list test");
        cache.store_jar(&jar_path, &expected).unwrap();

        let jars = cache.list_cached().unwrap();
        assert_eq!(jars.len(), 1);
        assert_eq!(jars[0].sha256, expected);
    }

    #[test]
    fn total_size_matches_content_length() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("cache"));

        let jar_dir = tmp.path().join("input");
        fs::create_dir_all(&jar_dir).unwrap();
        let jar_path = jar_dir.join("test.jar");
        fs::write(&jar_path, b"size check").unwrap();

        let expected = agora_core::download::sha256_hex(b"size check");
        cache.store_jar(&jar_path, &expected).unwrap();

        assert_eq!(cache.total_size().unwrap(), 10);
    }
}
