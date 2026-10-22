//! Reuse of a download that was already fetched and verified.
//!
//! A curated install reads the chosen jar at review time (to learn the mods the
//! jar itself requires) and would otherwise download it again at staging. This
//! cache keeps the verified bytes between the two.
//!
//! Fail-closed rules:
//! - Only bytes that matched every published hash are ever stored.
//! - A hit is re-verified against the published hashes on every read. An entry
//!   that is unreadable, damaged, or no longer matches is discarded and the
//!   file is downloaded again. The cache is never trusted.
//! - A cache problem (cannot write, cannot evict) never fails an install; it
//!   only costs a download.
//! - A fresh download that does not match its published hash is still returned
//!   to the caller unchanged, so the caller's own verification rejects it.
//!
//! Storage is [`ModCache`] (keyed by SHA-256) under `blobs/`, plus `index/`,
//! which maps a published SHA-1/SHA-512/MD5 value to the SHA-256 key for
//! sources that publish nothing stronger. The directory is owned by this
//! module; nothing else reads or writes it.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::install_pipeline::{verify_bytes, HashAlgorithm, HashSpec};
use crate::mod_cache::{is_sha256_hex, EvictionPolicy, ModCache};

/// Entries an in-flight read currently holds, by blob path. Process-wide so two
/// `DownloadCache` handles on the same directory protect each other.
static PINS: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());

struct Pin(String);

impl Pin {
    fn new(key: String) -> Self {
        if let Ok(mut pins) = PINS.lock() {
            *pins.entry(key.clone()).or_insert(0) += 1;
        }
        Pin(key)
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        if let Ok(mut pins) = PINS.lock() {
            if let Some(count) = pins.get_mut(&self.0) {
                *count -= 1;
                if *count == 0 {
                    pins.remove(&self.0);
                }
            }
        }
    }
}

fn is_pinned(key: &str) -> bool {
    // If the registry cannot be read, assume everything is held.
    PINS.lock().map(|p| p.contains_key(key)).unwrap_or(true)
}

pub struct DownloadCache {
    root: PathBuf,
    blobs: ModCache,
    policy: EvictionPolicy,
}

impl DownloadCache {
    pub fn new(root: PathBuf) -> Self {
        Self::with_policy(root, EvictionPolicy::default())
    }

    pub fn with_policy(root: PathBuf, policy: EvictionPolicy) -> Self {
        let blobs = ModCache::new(root.join("blobs"));
        DownloadCache {
            root,
            blobs,
            policy,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the file's bytes, from the cache when a verified copy exists,
    /// otherwise from `fetch`. `filename` is informational metadata only.
    pub async fn get_or_fetch<F, Fut>(
        &self,
        hashes: &HashSpec,
        filename: &str,
        fetch: F,
    ) -> Result<Vec<u8>, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<u8>, String>>,
    {
        // Nothing published to check against: nothing safe to reuse or store.
        if hashes.values.is_empty() {
            return fetch().await;
        }
        if let Some(bytes) = self.lookup(hashes) {
            return Ok(bytes);
        }
        let bytes = fetch().await?;
        if verify_bytes(&bytes, hashes).is_ok() {
            self.remember(&bytes, hashes, filename);
        }
        Ok(bytes)
    }

    fn lookup(&self, hashes: &HashSpec) -> Option<Vec<u8>> {
        let mut keys: Vec<String> = Vec::new();
        for published in &hashes.values {
            let value = published.value.trim().to_ascii_lowercase();
            if !is_plain_hex(&value) {
                continue;
            }
            let key = if published.algorithm == HashAlgorithm::Sha256 {
                Some(value)
            } else {
                self.read_index(&published.algorithm, &value)
            };
            if let Some(key) = key.filter(|k| is_sha256_hex(k)) {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        for key in keys {
            let _pin = Pin::new(self.pin_key(&key));
            let bytes = match self.blobs.read_verified(&key) {
                Ok(bytes) => bytes,
                Err(_) => {
                    // Absent entries are a plain miss. Present-but-unusable ones
                    // (damaged, truncated, unreadable) are discarded so the
                    // fresh download replaces them.
                    if self.blobs.has_jar(&key) {
                        let _ = self.blobs.remove_entry(&key);
                    }
                    continue;
                }
            };
            if verify_bytes(&bytes, hashes).is_err() {
                // The content is self-consistent but is not what this source
                // publishes: a stale or wrong index. Forget the index, keep
                // the blob (other sources may legitimately own it).
                self.forget_index(hashes);
                continue;
            }
            self.blobs.mark_used(&key);
            return Some(bytes);
        }
        None
    }

    fn remember(&self, bytes: &[u8], hashes: &HashSpec, filename: &str) {
        let Ok(key) = self.blobs.store_bytes(bytes, filename) else {
            return;
        };
        let _pin = Pin::new(self.pin_key(&key));
        for published in &hashes.values {
            if published.algorithm == HashAlgorithm::Sha256 {
                continue;
            }
            let value = published.value.trim().to_ascii_lowercase();
            if is_plain_hex(&value) {
                self.write_index(&published.algorithm, &value, &key);
            }
        }
        self.blobs.mark_used(&key);
        self.blobs
            .evict(&self.policy, std::time::SystemTime::now(), &|k| {
                is_pinned(&self.pin_key(k))
            });
    }

    fn pin_key(&self, sha256: &str) -> String {
        format!("{}|{sha256}", self.root.display())
    }

    fn index_path(&self, algorithm: &HashAlgorithm, value: &str) -> PathBuf {
        let algo = match algorithm {
            HashAlgorithm::Sha256 => "sha256",
            HashAlgorithm::Sha512 => "sha512",
            HashAlgorithm::Sha1 => "sha1",
            HashAlgorithm::Md5 => "md5",
        };
        self.root.join("index").join(algo).join(value)
    }

    fn read_index(&self, algorithm: &HashAlgorithm, value: &str) -> Option<String> {
        std::fs::read_to_string(self.index_path(algorithm, value))
            .ok()
            .map(|s| s.trim().to_string())
    }

    fn write_index(&self, algorithm: &HashAlgorithm, value: &str, key: &str) {
        let path = self.index_path(algorithm, value);
        let Some(parent) = path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        if std::fs::write(&tmp, key).is_ok() {
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            if std::fs::rename(&tmp, &path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    fn forget_index(&self, hashes: &HashSpec) {
        for published in &hashes.values {
            let value = published.value.trim().to_ascii_lowercase();
            if published.algorithm != HashAlgorithm::Sha256 && is_plain_hex(&value) {
                let _ = std::fs::remove_file(self.index_path(&published.algorithm, &value));
            }
        }
    }
}

/// Published values become file names, so only plain hex is accepted.
fn is_plain_hex(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install_pipeline::HashedValue;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;

    fn spec(algorithm: HashAlgorithm, value: String) -> HashSpec {
        HashSpec {
            values: vec![HashedValue { algorithm, value }],
        }
    }

    fn sha256_spec(bytes: &[u8]) -> HashSpec {
        spec(HashAlgorithm::Sha256, crate::download::sha256_hex(bytes))
    }

    fn sha1_spec(bytes: &[u8]) -> HashSpec {
        spec(HashAlgorithm::Sha1, crate::download::sha1_hex(bytes))
    }

    /// A fake source that counts how often it is fetched.
    struct Source {
        bytes: Vec<u8>,
        fetches: AtomicUsize,
    }

    impl Source {
        fn new(bytes: &[u8]) -> Self {
            Source {
                bytes: bytes.to_vec(),
                fetches: AtomicUsize::new(0),
            }
        }
        async fn fetch(&self) -> Result<Vec<u8>, String> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            Ok(self.bytes.clone())
        }
        fn count(&self) -> usize {
            self.fetches.load(Ordering::SeqCst)
        }
    }

    fn jar_path(cache: &DownloadCache, bytes: &[u8]) -> PathBuf {
        let sha = crate::download::sha256_hex(bytes);
        cache
            .root()
            .join("blobs")
            .join(&sha[..2])
            .join(&sha)
            .join("file.jar")
    }

    #[tokio::test]
    async fn the_root_jar_is_fetched_once_across_review_and_staging() {
        let tmp = TempDir::new().unwrap();
        let cache = DownloadCache::new(tmp.path().join("dc"));
        let source = Source::new(b"root jar");
        let hashes = sha256_spec(b"root jar");

        // Review time, then staging.
        let review = cache
            .get_or_fetch(&hashes, "m.jar", || source.fetch())
            .await
            .unwrap();
        let staging = cache
            .get_or_fetch(&hashes, "m.jar", || source.fetch())
            .await
            .unwrap();

        assert_eq!(review, b"root jar");
        assert_eq!(staging, b"root jar");
        assert_eq!(source.count(), 1);
    }

    #[tokio::test]
    async fn a_source_that_publishes_only_sha1_is_reused_too() {
        let tmp = TempDir::new().unwrap();
        let cache = DownloadCache::new(tmp.path().join("dc"));
        let source = Source::new(b"modrinth jar");
        let hashes = sha1_spec(b"modrinth jar");

        for _ in 0..2 {
            let got = cache
                .get_or_fetch(&hashes, "m.jar", || source.fetch())
                .await
                .unwrap();
            assert_eq!(got, b"modrinth jar");
        }
        assert_eq!(source.count(), 1);
    }

    #[tokio::test]
    async fn a_corrupt_cache_entry_is_discarded_and_downloaded_again() {
        let tmp = TempDir::new().unwrap();
        let cache = DownloadCache::new(tmp.path().join("dc"));
        let source = Source::new(b"good bytes");
        let hashes = sha256_spec(b"good bytes");
        cache
            .get_or_fetch(&hashes, "m.jar", || source.fetch())
            .await
            .unwrap();

        // Same length, different content: a size check alone would pass it.
        std::fs::write(jar_path(&cache, b"good bytes"), b"evil bytes").unwrap();

        let got = cache
            .get_or_fetch(&hashes, "m.jar", || source.fetch())
            .await
            .unwrap();
        assert_eq!(got, b"good bytes", "corrupt bytes must never be returned");
        assert_eq!(source.count(), 2);
        // The replacement is the good copy again, so the third read hits.
        assert_eq!(
            std::fs::read(jar_path(&cache, b"good bytes")).unwrap(),
            b"good bytes"
        );
        cache
            .get_or_fetch(&hashes, "m.jar", || source.fetch())
            .await
            .unwrap();
        assert_eq!(source.count(), 2);
    }

    #[tokio::test]
    async fn an_unreadable_cache_entry_is_downloaded_again() {
        let tmp = TempDir::new().unwrap();
        let cache = DownloadCache::new(tmp.path().join("dc"));
        let source = Source::new(b"jar");
        let hashes = sha256_spec(b"jar");
        cache
            .get_or_fetch(&hashes, "m.jar", || source.fetch())
            .await
            .unwrap();
        // A directory where the file should be cannot be read as one.
        let path = jar_path(&cache, b"jar");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        let got = cache
            .get_or_fetch(&hashes, "m.jar", || source.fetch())
            .await
            .unwrap();
        assert_eq!(got, b"jar");
        assert_eq!(source.count(), 2);
    }

    #[tokio::test]
    async fn a_stale_sha1_index_does_not_serve_the_wrong_jar() {
        let tmp = TempDir::new().unwrap();
        let cache = DownloadCache::new(tmp.path().join("dc"));
        let other = Source::new(b"some other jar");
        cache
            .get_or_fetch(&sha256_spec(b"some other jar"), "o.jar", || other.fetch())
            .await
            .unwrap();
        // Point the sha1 of "wanted" at the other jar's blob.
        let hashes = sha1_spec(b"wanted");
        let other_key = crate::download::sha256_hex(b"some other jar");
        cache.write_index(
            &HashAlgorithm::Sha1,
            &crate::download::sha1_hex(b"wanted"),
            &other_key,
        );

        let wanted = Source::new(b"wanted");
        let got = cache
            .get_or_fetch(&hashes, "w.jar", || wanted.fetch())
            .await
            .unwrap();
        assert_eq!(got, b"wanted");
        assert_eq!(wanted.count(), 1);
    }

    #[tokio::test]
    async fn a_fresh_download_with_the_wrong_hash_is_not_stored_and_still_fails_verification() {
        let tmp = TempDir::new().unwrap();
        let cache = DownloadCache::new(tmp.path().join("dc"));
        let hashes = sha256_spec(b"what was published");
        let source = Source::new(b"what the server sent");

        let got = cache
            .get_or_fetch(&hashes, "m.jar", || source.fetch())
            .await
            .unwrap();
        // The caller's own check is the hard error; the cache just never
        // launders the bytes into a trusted entry.
        assert!(verify_bytes(&got, &hashes).is_err());
        assert!(cache.blobs.entries().is_empty());
        let again = cache
            .get_or_fetch(&hashes, "m.jar", || source.fetch())
            .await
            .unwrap();
        assert!(verify_bytes(&again, &hashes).is_err());
        assert_eq!(source.count(), 2);
    }

    #[tokio::test]
    async fn nothing_published_means_nothing_is_cached() {
        let tmp = TempDir::new().unwrap();
        let cache = DownloadCache::new(tmp.path().join("dc"));
        let source = Source::new(b"unchecked");
        let none = HashSpec { values: vec![] };
        for _ in 0..2 {
            cache
                .get_or_fetch(&none, "m.jar", || source.fetch())
                .await
                .unwrap();
        }
        assert_eq!(source.count(), 2);
        assert!(cache.blobs.entries().is_empty());
    }

    #[tokio::test]
    async fn an_absent_or_empty_cache_folder_is_a_plain_miss() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("never-created");
        let cache = DownloadCache::new(root.clone());
        let source = Source::new(b"x");
        let got = cache
            .get_or_fetch(&sha256_spec(b"x"), "x.jar", || source.fetch())
            .await
            .unwrap();
        assert_eq!(got, b"x");
        assert!(root.join("blobs").is_dir());

        // An existing but empty folder behaves the same.
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let cache = DownloadCache::new(empty);
        assert!(cache.blobs.entries().is_empty());
        let source = Source::new(b"y");
        cache
            .get_or_fetch(&sha256_spec(b"y"), "y.jar", || source.fetch())
            .await
            .unwrap();
        assert_eq!(source.count(), 1);
    }

    #[tokio::test]
    async fn a_cache_that_cannot_be_written_never_fails_the_install() {
        let tmp = TempDir::new().unwrap();
        // The cache root is a file, so no directory can be created under it.
        let root = tmp.path().join("blocked");
        std::fs::write(&root, b"not a directory").unwrap();
        let cache = DownloadCache::new(root);
        let source = Source::new(b"jar");
        let got = cache
            .get_or_fetch(&sha256_spec(b"jar"), "m.jar", || source.fetch())
            .await
            .unwrap();
        assert_eq!(got, b"jar");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_installs_share_one_entry_and_it_stays_intact() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("dc");
        let source = std::sync::Arc::new(Source::new(b"shared jar"));
        let hashes = std::sync::Arc::new(sha256_spec(b"shared jar"));

        // Prime once, as the first install's review would.
        DownloadCache::new(root.clone())
            .get_or_fetch(&hashes, "m.jar", || source.fetch())
            .await
            .unwrap();

        // Two instances stage the same jar concurrently through separate handles.
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (root, source, hashes) = (root.clone(), source.clone(), hashes.clone());
            tasks.push(tokio::spawn(async move {
                DownloadCache::new(root)
                    .get_or_fetch(&hashes, "m.jar", || source.fetch())
                    .await
                    .unwrap()
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap(), b"shared jar");
        }
        assert_eq!(source.count(), 1);
        assert_eq!(DownloadCache::new(root).blobs.entries().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_cold_installs_never_serve_a_partial_entry() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("dc");
        let payload = vec![7u8; 2 * 1024 * 1024];
        let source = std::sync::Arc::new(Source::new(&payload));
        let hashes = std::sync::Arc::new(sha256_spec(&payload));
        let mut tasks = Vec::new();
        for _ in 0..6 {
            let (root, source, hashes) = (root.clone(), source.clone(), hashes.clone());
            tasks.push(tokio::spawn(async move {
                let cache = DownloadCache::new(root);
                cache
                    .get_or_fetch(&hashes, "m.jar", || source.fetch())
                    .await
                    .unwrap()
            }));
        }
        for task in tasks {
            assert!(verify_bytes(&task.await.unwrap(), &hashes).is_ok());
        }
        // Whatever the race did, nothing partial is left behind: one more
        // read yields the exact bytes and leaves exactly one intact entry.
        let cache = DownloadCache::new(root);
        let got = cache
            .get_or_fetch(&hashes, "m.jar", || source.fetch())
            .await
            .unwrap();
        assert!(verify_bytes(&got, &hashes).is_ok());
        assert_eq!(cache.blobs.entries().len(), 1);
    }

    // --- eviction -------------------------------------------------------

    fn put(cache: &ModCache, bytes: &[u8]) -> String {
        cache.store_bytes(bytes, "m.jar").unwrap()
    }

    fn policy(max_bytes: u64) -> EvictionPolicy {
        EvictionPolicy {
            max_bytes,
            max_age: Duration::from_secs(1_000_000),
            grace: Duration::from_secs(3600),
        }
    }

    fn later(secs: u64) -> SystemTime {
        SystemTime::now() + Duration::from_secs(secs)
    }

    #[test]
    fn eviction_brings_the_cache_under_its_bound_oldest_first() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("c"));
        let a = put(&cache, &[1u8; 100]);
        std::thread::sleep(Duration::from_millis(30));
        let b = put(&cache, &[2u8; 100]);
        std::thread::sleep(Duration::from_millis(30));
        let c = put(&cache, &[3u8; 100]);

        // Two hours on, everything is past the grace period.
        let report = cache.evict(&policy(150), later(7200), &|_| false);

        assert_eq!(report.removed, 2);
        let left: Vec<String> = cache.entries().into_iter().map(|e| e.sha256).collect();
        assert_eq!(left, vec![c], "the newest entry survives");
        assert!(!left.contains(&a) && !left.contains(&b));
        let total: u64 = cache.entries().iter().map(|e| e.size_bytes).sum();
        assert!(total <= 150);
    }

    #[test]
    fn eviction_never_removes_an_entry_an_install_holds() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("c"));
        let held = put(&cache, &[1u8; 100]);
        let free = put(&cache, &[2u8; 100]);

        let report = cache.evict(&policy(0), later(7200), &|k| k == held);

        assert_eq!(report.removed, 1);
        assert_eq!(report.kept_in_use, 1);
        let left: Vec<String> = cache.entries().into_iter().map(|e| e.sha256).collect();
        assert_eq!(left, vec![held]);
        assert!(!left.contains(&free));
    }

    #[test]
    fn eviction_spares_anything_used_within_the_grace_period() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("c"));
        put(&cache, &[1u8; 100]);
        // Only a minute later: another process may be mid-install.
        let report = cache.evict(&policy(0), later(60), &|_| false);
        assert_eq!(report.removed, 0);
        assert_eq!(cache.entries().len(), 1);
    }

    #[test]
    fn eviction_removes_entries_past_their_maximum_age_even_under_the_size_bound() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("c"));
        put(&cache, &[1u8; 10]);
        let p = EvictionPolicy {
            max_bytes: u64::MAX,
            max_age: Duration::from_secs(10_000),
            grace: Duration::from_secs(60),
        };
        assert_eq!(cache.evict(&p, later(5_000), &|_| false).removed, 0);
        assert_eq!(cache.evict(&p, later(20_000), &|_| false).removed, 1);
    }

    #[test]
    fn eviction_only_touches_well_formed_entries() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("c");
        let cache = ModCache::new(root.clone());
        put(&cache, &[1u8; 10]);
        // Things that merely live nearby must survive any eviction.
        std::fs::create_dir_all(root.join("zz").join("not-a-hash")).unwrap();
        std::fs::write(root.join("zz").join("not-a-hash").join("file.jar"), b"x").unwrap();
        std::fs::write(root.join("stray.txt"), b"keep").unwrap();
        let sha = crate::download::sha256_hex(b"other");
        std::fs::create_dir_all(root.join("00").join(&sha)).unwrap(); // wrong prefix
        std::fs::write(root.join("00").join(&sha).join("file.jar"), b"other").unwrap();

        cache.evict(&policy(0), later(7200), &|_| false);

        assert!(root.join("zz").join("not-a-hash").join("file.jar").exists());
        assert!(root.join("stray.txt").exists());
        assert!(root.join("00").join(&sha).join("file.jar").exists());
    }

    #[test]
    fn remove_entry_rejects_anything_but_a_sha256_key() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("c");
        let cache = ModCache::new(root.clone());
        let victim = tmp.path().join("victim");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("keep.txt"), b"keep").unwrap();

        for bad in ["..", "../victim", "", "ab", "../../victim"] {
            assert!(cache.remove_entry(bad).is_err(), "{bad:?}");
        }
        assert!(victim.join("keep.txt").exists());
    }

    #[test]
    fn eviction_on_an_absent_cache_folder_is_a_noop() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("missing"));
        assert_eq!(
            cache.evict(&policy(0), later(7200), &|_| false),
            EvictionReport::default()
        );
    }

    #[test]
    fn eviction_does_not_break_a_jar_already_linked_into_an_instance() {
        let tmp = TempDir::new().unwrap();
        let cache = ModCache::new(tmp.path().join("c"));
        let sha = put(&cache, b"linked jar");
        let mods = tmp.path().join("mods");
        let linked = cache.resolve_and_link(&sha, &mods, "m.jar").unwrap();

        cache.evict(&policy(0), later(7200), &|_| false);

        assert!(cache.entries().is_empty());
        assert_eq!(std::fs::read(linked).unwrap(), b"linked jar");
    }

    use crate::mod_cache::EvictionReport;

    #[tokio::test]
    async fn a_store_evicts_old_entries_but_keeps_the_one_it_just_wrote() {
        let tmp = TempDir::new().unwrap();
        let p = EvictionPolicy {
            max_bytes: 0,
            max_age: Duration::from_secs(1_000_000),
            grace: Duration::from_secs(0),
        };
        let cache = DownloadCache::with_policy(tmp.path().join("dc"), p);
        let old = Source::new(b"old jar");
        cache
            .get_or_fetch(&sha256_spec(b"old jar"), "o.jar", || old.fetch())
            .await
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let new = Source::new(b"new jar");
        cache
            .get_or_fetch(&sha256_spec(b"new jar"), "n.jar", || new.fetch())
            .await
            .unwrap();

        // Zero grace and a zero bound: only the pinned just-written entry
        // survives, and the older one is gone.
        let left: Vec<String> = cache
            .blobs
            .entries()
            .into_iter()
            .map(|e| e.sha256)
            .collect();
        assert_eq!(left, vec![crate::download::sha256_hex(b"new jar")]);
    }
}
