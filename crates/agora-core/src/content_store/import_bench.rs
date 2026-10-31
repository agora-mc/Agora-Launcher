//! Import speed benchmark (perf/import). Reads real MO2 mod folders, read only, and times the
//! content store's add path on them. Ignored by default:
//!
//! `cargo test -p agora-core --lib import_bench -- --ignored --nocapture`
//!
//! Every timed run writes into a fresh root under `D:\Agora-bench\perfimport-data`, so nothing is
//! deduplicated against an earlier run. The root is removed at the start of each run.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;

const MODS_ROOT: &str = r"D:\Skyrim-MO2-Salvage\MO2\mods";
const DATA_ROOT: &str = r"D:\Agora-bench\perfimport-data";

/// The fixed subset: 50,424 files and about 5.9 GB. Two mods of many small files (OStim
/// animations), Nemesis (8,641 files), and two big ones (Darkend, Nature of the Wild Lands).
const SUBSET: &[&str] = &[
    "101 - Billyy's Animations for OStim 3.2.2",
    "100 - Anub's Animations for OStim 2.3.2",
    "090 - Nemesis Unlimited Behavior Engine 0.84",
    "293 - Darkend 1.4",
    "063 - Nature of the Wild Lands 3.14",
];

type ModFiles = Vec<(String, Vec<(RelPath, PathBuf)>)>;

fn folder_source(name: &str) -> ContentSource {
    ContentSource::Folder {
        path: name.to_string(),
        added_at_unix_ms: now_unix_ms(),
    }
}

/// Remove a benchmark root. Objects are unprotected first, as the store itself does.
fn reset_root(root: &Path) {
    let objects = root.join("content").join("objects");
    if objects.exists() {
        for shard in std::fs::read_dir(&objects).unwrap().flatten() {
            if let Ok(entries) = std::fs::read_dir(shard.path()) {
                for obj in entries.flatten() {
                    let _ = protect::unprotect(&obj.path());
                }
            }
        }
    }
    if root.exists() {
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn rate(bytes: u64, d: Duration) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / d.as_secs_f64()
}

/// Per-phase totals for a sequential pass over the subset.
#[derive(Default)]
struct Phases {
    copy_hash: Duration,
    dir_and_exists: Duration,
    trust: Duration,
    rename: Duration,
    protect_new: Duration,
    protection_check: Duration,
    discard: Duration,
    files: usize,
    bytes: u64,
}

impl Phases {
    fn print(&self, label: &str) {
        let rows = [
            ("open + copy-hash", self.copy_hash),
            ("parent dir + exists", self.dir_and_exists),
            ("object_is_trustworthy", self.trust),
            ("rename into objects", self.rename),
            ("protect (new object)", self.protect_new),
            ("protection check (present)", self.protection_check),
            ("discard staged copy", self.discard),
        ];
        println!(
            "  [{label}] {} files, {:.2} GB, one thread",
            self.files,
            self.bytes as f64 / 1e9
        );
        for (name, d) in rows {
            if d > Duration::ZERO {
                println!(
                    "    {name:<28} {:>10.1} ms total  {:>8.1} us/file",
                    ms(d),
                    d.as_secs_f64() * 1e6 / self.files as f64
                );
            }
        }
    }
}

/// The phases of `finish_adding` for each staged file, one file at a time, timed. A fresh root
/// stores every file as new (`protect_new` is the protection used for new objects); a root that
/// already holds them takes the present path.
fn phase_pass(
    ctx: &Ctx,
    mods: &ModFiles,
    buf: &mut [u8],
    protect_new: fn(&Path) -> std::io::Result<()>,
) -> Phases {
    let mut p = Phases::default();
    let staging = ctx.paths.content_staging_dir().join("bench");
    std::fs::create_dir_all(&staging).unwrap();
    for (_, files) in mods {
        for (idx, (_, abs)) in files.iter().enumerate() {
            let staged = staging.join(format!("obj_{idx}"));
            let t = Instant::now();
            let (size, sha) = copy_and_hash(abs, &staged, buf).unwrap();
            p.copy_hash += t.elapsed();
            p.files += 1;
            p.bytes += size;

            let obj = ctx.paths.content_object_path(&sha);
            let t = Instant::now();
            if let Some(parent) = obj.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let exists = obj.exists();
            p.dir_and_exists += t.elapsed();

            if exists {
                let t = Instant::now();
                let trusted = object_is_trustworthy(&obj, size, &sha).unwrap();
                p.trust += t.elapsed();
                if trusted {
                    let t = Instant::now();
                    std::fs::remove_file(&staged).unwrap();
                    p.discard += t.elapsed();
                    let t = Instant::now();
                    if protect::protection(&obj).unwrap() != Protection::Protected {
                        protect::protect(&obj).unwrap();
                    }
                    p.protection_check += t.elapsed();
                } else {
                    protect::unprotect(&obj).unwrap();
                    std::fs::remove_file(&obj).unwrap();
                    let t = Instant::now();
                    std::fs::rename(&staged, &obj).unwrap();
                    p.rename += t.elapsed();
                    let t = Instant::now();
                    protect_new(&obj).unwrap();
                    p.protect_new += t.elapsed();
                }
            } else {
                let t = Instant::now();
                std::fs::rename(&staged, &obj).unwrap();
                p.rename += t.elapsed();
                let t = Instant::now();
                protect_new(&obj).unwrap();
                p.protect_new += t.elapsed();
            }
        }
    }
    let _ = std::fs::remove_dir_all(&staging);
    p
}

/// The copy-and-hash phase alone, over the whole subset, through a buffer of `len` bytes.
fn copy_only_pass(ctx: &Ctx, mods: &ModFiles, len: usize) -> Duration {
    let staging = ctx.paths.content_staging_dir().join("bench-copy");
    std::fs::create_dir_all(&staging).unwrap();
    let mut buf = vec![0u8; len];
    let t = Instant::now();
    for (_, files) in mods {
        for (idx, (_, abs)) in files.iter().enumerate() {
            let staged = staging.join(format!("obj_{idx}"));
            copy_and_hash(abs, &staged, &mut buf).unwrap();
            std::fs::remove_file(&staged).unwrap();
        }
    }
    let d = t.elapsed();
    let _ = std::fs::remove_dir_all(&staging);
    d
}

#[test]
#[ignore = "times a real MO2 import: reads D:\\Skyrim-MO2-Salvage, writes D:\\Agora-bench\\perfimport-data"]
fn import_benchmark() {
    let mods_root = Path::new(MODS_ROOT);
    let data_root = Path::new(DATA_ROOT);

    // The subset, walked once. The walk is not part of the import.
    let walk_start = Instant::now();
    let mut mods: ModFiles = Vec::new();
    for name in SUBSET {
        let files = collect_folder_files(&mods_root.join(name)).expect("walk a subset mod");
        mods.push((name.to_string(), files));
    }
    let walk = walk_start.elapsed();
    let total_files: usize = mods.iter().map(|(_, f)| f.len()).sum();
    let total_bytes: u64 = mods
        .iter()
        .flat_map(|(_, f)| f.iter())
        .map(|(_, p)| std::fs::metadata(p).unwrap().len())
        .sum();
    assert!(total_files >= 20_000, "subset has {total_files} files");
    assert!(
        total_bytes >= 5 * 1024 * 1024 * 1024,
        "subset is {total_bytes} bytes"
    );
    println!(
        "subset: {} mods, {} files, {:.2} GB (walk {:.0} ms, not counted)",
        mods.len(),
        total_files,
        total_bytes as f64 / 1e9,
        ms(walk)
    );
    for (name, files) in &mods {
        println!("  {name}: {} files", files.len());
    }

    // Warm-up: the whole subset once, so the source files are in the disk cache. Not reported.
    let warm = data_root.join("warm");
    reset_root(&warm);
    {
        let ctx = Ctx::for_testing(warm.clone());
        for (name, files) in &mods {
            add_collected(&ctx, name, files.clone(), folder_source(name)).unwrap();
        }
    }
    reset_root(&warm);

    // End to end, the way the importer calls it: one add per mod, into a fresh root.
    let e2e = data_root.join("e2e");
    reset_root(&e2e);
    let ctx = Ctx::for_testing(e2e.clone());
    let t = Instant::now();
    let (mut new_count, mut present_count) = (0usize, 0usize);
    for (name, files) in &mods {
        let outcome = add_collected(&ctx, name, files.clone(), folder_source(name)).unwrap();
        new_count += outcome.objects_new();
        present_count += outcome.objects_present();
    }
    let e2e_new = t.elapsed();
    println!(
        "end to end, new objects: {:>8.0} ms  {:>6.1} MB/s  {:>7.0} files/s  (new {new_count}, present {present_count})",
        ms(e2e_new),
        rate(total_bytes, e2e_new),
        total_files as f64 / e2e_new.as_secs_f64()
    );

    // The same subset again into the same root: every object is present.
    let t = Instant::now();
    let (mut new_again, mut present_again) = (0usize, 0usize);
    for (name, files) in &mods {
        let outcome = add_collected(&ctx, name, files.clone(), folder_source(name)).unwrap();
        new_again += outcome.objects_new();
        present_again += outcome.objects_present();
    }
    let e2e_present = t.elapsed();
    println!(
        "end to end, all present: {:>7.0} ms  {:>6.1} MB/s  {:>7.0} files/s  (new {new_again}, present {present_again})",
        ms(e2e_present),
        rate(total_bytes, e2e_present),
        total_files as f64 / e2e_present.as_secs_f64()
    );
    reset_root(&e2e);

    // Per-phase breakdown, one file at a time on one thread, in a fresh root, then the present path.
    let phases = data_root.join("phases");
    reset_root(&phases);
    let ctx = Ctx::for_testing(phases.clone());
    let mut buf = vec![0u8; 128 * 1024];
    let new_phases = phase_pass(&ctx, &mods, &mut buf, protect::protect_by_handle);
    new_phases.print("new objects, protect by handle");
    let present_phases = phase_pass(&ctx, &mods, &mut buf, protect::protect);
    present_phases.print("all present");
    reset_root(&phases);

    // Read buffer: copy-and-hash alone, 128 KiB against 1 MiB, each on a warm cache.
    let ctx = Ctx::for_testing(data_root.join("buffers"));
    let d128 = copy_only_pass(&ctx, &mods, 128 * 1024);
    let d1m = copy_only_pass(&ctx, &mods, 1024 * 1024);
    let d128_again = copy_only_pass(&ctx, &mods, 128 * 1024);
    println!(
        "copy-and-hash only: 128 KiB {:.0} ms ({:.1} MB/s), 1 MiB {:.0} ms ({:.1} MB/s), 128 KiB again {:.0} ms",
        ms(d128),
        rate(total_bytes, d128),
        ms(d1m),
        rate(total_bytes, d1m),
        ms(d128_again)
    );
    reset_root(&data_root.join("buffers"));
}
