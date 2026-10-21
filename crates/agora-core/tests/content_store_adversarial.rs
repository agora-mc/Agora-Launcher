//! Adversarial tests for the content store: hostile archives, Windows-only path collisions,
//! interrupted and concurrent adds, removal next to unreadable state, and protection.

use std::io::Write;
use std::path::{Path, PathBuf};

use agora_core::content_store as cs;
use agora_core::ctx::CoreContext;

fn ctx(tmp: &Path) -> CoreContext {
    CoreContext::for_testing(tmp.join("data"))
}

fn zip_with(path: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::FileOptions::default();
    for (name, bytes) in entries {
        if name.ends_with('/') {
            zip.add_directory(name.trim_end_matches('/'), opts).unwrap();
        } else {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(bytes).unwrap();
        }
    }
    zip.finish().unwrap();
    path.to_path_buf()
}

fn content_root(ctx: &CoreContext) -> PathBuf {
    ctx.paths.content_root()
}

fn count_files(dir: &Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        n += if p.is_dir() { count_files(&p) } else { 1 };
    }
    n
}

fn assert_nothing_left(ctx: &CoreContext) {
    let root = content_root(ctx);
    assert_eq!(count_files(&root.join("objects")), 0, "objects left behind");
    assert_eq!(count_files(&root.join("items")), 0, "manifest left behind");
    assert_eq!(count_files(&root.join("staging")), 0, "staging left behind");
}

#[test]
fn probe_file_and_folder_with_same_name_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let z = zip_with(&tmp.path().join("m.zip"), &[("a", b"1"), ("a/b", b"2")]);
    assert!(cs::add_archive(&c, &z, None).is_err());
    assert_nothing_left(&c);
}

#[test]
fn probe_case_collision_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let z = zip_with(
        &tmp.path().join("m.zip"),
        &[("Data/x.esp", b"1"), ("data/X.ESP", b"2")],
    );
    assert!(cs::add_archive(&c, &z, None).is_err());
    assert_nothing_left(&c);
}

#[test]
fn probe_windows_hostile_names_are_refused() {
    for bad in [
        "con.txt",
        "Data/AUX",
        "lpt9.dll",
        "foo.",
        "foo ",
        "a//b",
        "./a",
        "a/./b",
        "x:y",
        "../escape",
        "/abs",
        "C:/abs",
        "a/..\\b",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let c = ctx(tmp.path());
        let z = zip_with(&tmp.path().join("m.zip"), &[("ok.txt", b"1"), (bad, b"2")]);
        let r = cs::add_archive(&c, &z, None);
        assert!(r.is_err(), "accepted hostile path {bad:?}");
        assert_nothing_left(&c);
    }
}

#[test]
fn probe_near_misses_are_accepted() {
    // Must not over-refuse: these are ordinary mod paths.
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let z = zip_with(
        &tmp.path().join("m.zip"),
        &[
            ("console.txt", b"1"),
            ("Data/conf.ini", b"2"),
            ("com10.txt", b"3"),
            ("a.b/c d.esp", b"4"),
            ("Data/..hidden", b"5"),
        ],
    );
    cs::add_archive(&c, &z, None).expect("ordinary paths refused");
}

#[test]
fn probe_directory_only_archive_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let z = zip_with(
        &tmp.path().join("m.zip"),
        &[("Data/", b""), ("Data/Textures/", b"")],
    );
    assert!(cs::add_archive(&c, &z, None).is_err());
    assert_nothing_left(&c);
}

#[test]
fn probe_entry_order_does_not_change_the_id() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let a = zip_with(&tmp.path().join("a.zip"), &[("x", b"1"), ("y/z", b"2")]);
    let b = zip_with(&tmp.path().join("b.zip"), &[("y/z", b"2"), ("x", b"1")]);
    let ia = cs::add_archive(&c, &a, None).unwrap();
    let ib = cs::add_archive(&c, &b, None).unwrap();
    assert_eq!(ia.item().item_id, ib.item().item_id);
}

#[test]
fn probe_late_refusal_leaves_no_objects() {
    // The bad entry comes last, after good entries were already extracted.
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let z = zip_with(
        &tmp.path().join("m.zip"),
        &[("a.txt", b"1"), ("b.txt", b"2"), ("nul", b"3")],
    );
    assert!(cs::add_archive(&c, &z, None).is_err());
    assert_nothing_left(&c);
}

#[test]
fn probe_parallel_adds_of_one_archive_make_one_item() {
    let tmp = tempfile::tempdir().unwrap();
    let z = zip_with(
        &tmp.path().join("m.zip"),
        &[("a.txt", b"hello"), ("b/c.txt", b"world")],
    );
    let data = tmp.path().to_path_buf();
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let z = z.clone();
            let data = data.clone();
            std::thread::spawn(move || {
                let c = ctx(&data);
                cs::add_archive(&c, &z, None).map(|o| o.item().item_id.clone())
            })
        })
        .collect();
    let ids: Vec<String> = handles
        .into_iter()
        .map(|h| h.join().unwrap().unwrap())
        .collect();
    assert!(ids.windows(2).all(|w| w[0] == w[1]));
    let c = ctx(&data);
    assert_eq!(count_files(&content_root(&c).join("items")), 1);
    assert_eq!(count_files(&content_root(&c).join("objects")), 2);
}

#[test]
fn probe_removal_with_unreadable_neighbour_deletes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let a = cs::add_archive(
        &c,
        &zip_with(
            &tmp.path().join("a.zip"),
            &[("shared", b"same"), ("a", b"a")],
        ),
        None,
    )
    .unwrap();
    let b = cs::add_archive(
        &c,
        &zip_with(
            &tmp.path().join("b.zip"),
            &[("shared", b"same"), ("b", b"b")],
        ),
        None,
    )
    .unwrap();
    let b_manifest = content_root(&c)
        .join("items")
        .join(format!("{}.json", b.item().item_id));
    std::fs::write(&b_manifest, b"{ not json").unwrap();
    let objects_before = count_files(&content_root(&c).join("objects"));
    assert!(
        cs::remove_item(&c, &a.item().item_id).is_err(),
        "removal proceeded past an unreadable manifest"
    );
    assert_eq!(
        count_files(&content_root(&c).join("objects")),
        objects_before
    );
    assert!(content_root(&c)
        .join("items")
        .join(format!("{}.json", a.item().item_id))
        .exists());
}

#[test]
fn probe_unknown_item_removal_is_an_error_not_a_noop_sweep() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    cs::add_archive(
        &c,
        &zip_with(&tmp.path().join("a.zip"), &[("a", b"a")]),
        None,
    )
    .unwrap();
    assert!(cs::remove_item(
        &c,
        "0000000000000000000000000000000000000000000000000000000000000000"
    )
    .is_err());
    assert_eq!(count_files(&content_root(&c).join("objects")), 1);
}

#[test]
fn probe_item_id_is_not_a_path() {
    // A hostile id must never become a path outside items/.
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    std::fs::create_dir_all(tmp.path().join("victim")).unwrap();
    std::fs::write(tmp.path().join("victim/keep.json"), b"{}").unwrap();
    for id in ["../../victim/keep", "..\\..\\victim\\keep", "", "abc/def"] {
        let _ = cs::remove_item(&c, id);
        let _ = cs::verify_item(&c, id, cs::VerifyDepth::Quick);
    }
    assert!(tmp.path().join("victim/keep.json").exists());
}

#[cfg(windows)]
#[test]
fn probe_protected_object_survives_in_place_write_but_links() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let out = cs::add_archive(
        &c,
        &zip_with(&tmp.path().join("a.zip"), &[("a.txt", b"original")]),
        None,
    )
    .unwrap();
    let item = cs::get_item(&c, &out.item().item_id).unwrap();
    let hash = &item.files[0].sha256;
    let object = content_root(&c).join("objects").join(&hash[..2]).join(hash);
    assert!(std::fs::OpenOptions::new()
        .write(true)
        .open(&object)
        .is_err());
    assert!(std::fs::OpenOptions::new()
        .append(true)
        .open(&object)
        .is_err());
    assert_eq!(std::fs::read(&object).unwrap(), b"original");
    let link = tmp.path().join("link.txt");
    std::fs::hard_link(&object, &link).expect("hardlink to a protected object must work");
    assert!(
        std::fs::OpenOptions::new().write(true).open(&link).is_err(),
        "write through a link must fail too"
    );
}

#[test]
fn probe_readding_replaces_a_corrupted_unprotected_object() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let z = zip_with(&tmp.path().join("a.zip"), &[("a.txt", b"original")]);
    let item = cs::add_archive(&c, &z, None).unwrap().item().clone();
    let object = c.paths.content_object_path(&item.files[0].sha256);
    cs::protect::unprotect(&object).unwrap();
    std::fs::write(&object, b"tampered").unwrap(); // same length, different bytes
    let again = cs::add_archive(&c, &z, None).unwrap();
    assert_eq!(
        again.objects_restored(),
        1,
        "a corrupted object must be replaced, not re-protected"
    );
    assert_eq!(std::fs::read(&object).unwrap(), b"original");
    let report = cs::verify_item(&c, &item.item_id, cs::VerifyDepth::Full).unwrap();
    assert!(report.problems.is_empty(), "{:?}", report.problems);
}

#[test]
fn probe_removal_sweeps_orphans_left_by_an_interrupted_add() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let keep = cs::add_archive(
        &c,
        &zip_with(&tmp.path().join("k.zip"), &[("k", b"keep")]),
        None,
    )
    .unwrap()
    .item()
    .clone();
    let gone = cs::add_archive(
        &c,
        &zip_with(&tmp.path().join("g.zip"), &[("g", b"gone")]),
        None,
    )
    .unwrap()
    .item()
    .clone();
    let orphan_hash = "ab".repeat(32);
    let orphan = c.paths.content_object_path(&orphan_hash);
    std::fs::create_dir_all(orphan.parent().unwrap()).unwrap();
    std::fs::write(&orphan, b"orphan").unwrap();
    cs::remove_item(&c, &gone.item_id).unwrap();
    assert!(!orphan.exists(), "orphan object not swept");
    assert!(c.paths.content_object_path(&keep.files[0].sha256).exists());
}

#[test]
fn probe_empty_or_non_hex_prefix_names_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let only = cs::add_archive(
        &c,
        &zip_with(&tmp.path().join("a.zip"), &[("a", b"a")]),
        None,
    )
    .unwrap()
    .item()
    .clone();
    for p in ["", " ", "*", "A", &only.item_id.to_uppercase()] {
        assert!(
            cs::remove_item(&c, p).is_err(),
            "prefix {p:?} removed an item"
        );
    }
    assert!(cs::get_item(&c, &only.item_id).is_ok());
}

#[test]
fn probe_a_corrupt_item_can_still_be_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(tmp.path());
    let item = cs::add_archive(
        &c,
        &zip_with(&tmp.path().join("a.zip"), &[("a", b"a")]),
        None,
    )
    .unwrap()
    .item()
    .clone();
    std::fs::write(c.paths.content_item_path(&item.item_id), b"garbage").unwrap();
    cs::remove_item(&c, &item.item_id).expect("a corrupt item must be removable");
    assert!(!c.paths.content_object_path(&item.files[0].sha256).exists());
}
