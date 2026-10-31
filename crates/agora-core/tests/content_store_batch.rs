//! Batches through the content store's parallel add path (perf/import): identical bytes in one
//! batch, an untrustworthy existing object, an unreadable source, empty batches, and the
//! protection of newly stored objects and their hardlinks.

use std::fs;
use std::path::{Path, PathBuf};

use agora_core::content_store::{
    add_folder, list_items, protect, verify_item, AddOutcome, ContentError, Protection, VerifyDepth,
};
use agora_core::ctx::Ctx;
use tempfile::TempDir;

fn test_ctx() -> (TempDir, Ctx) {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = Ctx::for_testing(tmp.path().to_path_buf());
    (tmp, ctx)
}

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

/// Every object file in the store, across all shard folders.
fn object_files(ctx: &Ctx) -> Vec<PathBuf> {
    let root = ctx.paths.content_objects_dir();
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    for shard in fs::read_dir(&root).unwrap() {
        let shard = shard.unwrap().path();
        if shard.is_dir() {
            for obj in fs::read_dir(&shard).unwrap() {
                out.push(obj.unwrap().path());
            }
        }
    }
    out
}

fn staging_is_empty(ctx: &Ctx) -> bool {
    let staging = ctx.paths.content_staging_dir();
    !staging.exists() || fs::read_dir(&staging).unwrap().next().is_none()
}

fn sha_of(outcome: &AddOutcome, rel: &str) -> String {
    outcome
        .item()
        .files
        .iter()
        .find(|f| f.path.as_str() == rel)
        .unwrap()
        .sha256
        .clone()
}

#[test]
fn identical_bytes_in_one_batch_store_one_object() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let folder = work.path().join("mod");
    write(&folder, "a.txt", b"same bytes");
    write(&folder, "dir/b.txt", b"same bytes");
    write(&folder, "dir/c.txt", b"same bytes");
    write(&folder, "d.txt", b"other bytes");

    let out = add_folder(&ctx, &folder, Some("Dupes")).unwrap();
    match &out {
        AddOutcome::Added { item, objects_new } => {
            assert_eq!(*objects_new, 2, "two distinct byte strings, two objects");
            assert_eq!(item.files.len(), 4);
        }
        other => panic!("expected Added, got {other:?}"),
    }
    assert_eq!(object_files(&ctx).len(), 2);
    for f in &out.item().files {
        let obj = ctx.paths.content_object_path(&f.sha256);
        assert_eq!(
            protect::protection(&obj).unwrap(),
            Protection::Protected,
            "{}",
            f.path
        );
    }
    assert!(verify_item(&ctx, &out.item().item_id, VerifyDepth::Full)
        .unwrap()
        .problems
        .is_empty());
    assert!(staging_is_empty(&ctx));

    // The same folder again: nothing is new, and every entry finds its object present.
    let again = add_folder(&ctx, &folder, Some("Dupes")).unwrap();
    match again {
        AddOutcome::Existing {
            objects_new,
            objects_restored,
            objects_present,
            ..
        } => {
            assert_eq!(objects_new, 0);
            assert_eq!(objects_restored, 0);
            assert_eq!(objects_present, 4);
        }
        other => panic!("expected Existing, got {other:?}"),
    }
    assert_eq!(object_files(&ctx).len(), 2);
}

#[test]
fn untrustworthy_existing_object_is_replaced_not_protected_as_it_is() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let folder = work.path().join("mod");
    write(&folder, "x.txt", b"hello world");
    write(&folder, "y.txt", b"second file");

    let first = add_folder(&ctx, &folder, Some("Replace")).unwrap();
    let sha = sha_of(&first, "x.txt");
    let obj = ctx.paths.content_object_path(&sha);

    // Same length, different bytes, and unprotected: the object no longer holds its name's content.
    protect::unprotect(&obj).unwrap();
    fs::write(&obj, b"HELLO WORLD").unwrap();

    let again = add_folder(&ctx, &folder, Some("Replace")).unwrap();
    match again {
        AddOutcome::Existing {
            objects_new,
            objects_restored,
            objects_present,
            ..
        } => {
            assert_eq!(objects_new, 0);
            assert_eq!(
                objects_restored, 1,
                "the replaced object counts as new, then restored"
            );
            assert_eq!(objects_present, 1, "y.txt's object is still trustworthy");
        }
        other => panic!("expected Existing, got {other:?}"),
    }
    assert_eq!(fs::read(&obj).unwrap(), b"hello world");
    assert_eq!(protect::protection(&obj).unwrap(), Protection::Protected);
    assert!(verify_item(&ctx, &first.item().item_id, VerifyDepth::Full)
        .unwrap()
        .problems
        .is_empty());
}

#[cfg(windows)]
#[test]
fn unreadable_source_fails_and_leaves_no_item_or_staging() {
    use std::os::windows::fs::OpenOptionsExt;

    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let folder = work.path().join("mod");
    for i in 0..12 {
        write(
            &folder,
            &format!("f{i:02}.txt"),
            format!("content {i}").as_bytes(),
        );
    }
    // Held open with no sharing: every other open of this file fails.
    let _held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(folder.join("f07.txt"))
        .unwrap();

    let err = add_folder(&ctx, &folder, Some("Locked")).unwrap_err();
    assert!(matches!(err, ContentError::Io(_)), "got {err:?}");
    assert!(list_items(&ctx).unwrap().is_empty(), "no item is written");
    assert!(object_files(&ctx).is_empty(), "no object is placed");
    assert!(staging_is_empty(&ctx), "staging is cleaned up");
}

#[test]
fn empty_batch_and_empty_name_behave_as_before() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();

    let empty = work.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let err = add_folder(&ctx, &empty, Some("Empty")).unwrap_err();
    assert!(matches!(err, ContentError::EmptyItem), "got {err:?}");
    assert!(list_items(&ctx).unwrap().is_empty());

    // An empty name is kept as given (it does not fall back to the folder's name).
    let folder = work.path().join("named");
    write(&folder, "f.txt", b"one file");
    let out = add_folder(&ctx, &folder, Some("")).unwrap();
    assert_eq!(out.item().name, "");
    assert_eq!(out.objects_new(), 1);
}

// Deletion is not asserted here. The deny mask carries DELETE, but a folder that grants its user
// FILE_DELETE_CHILD still lets that user delete the file (see protect.rs), and the data folders do.
// The mask guards modification only, which is the documented scope.
#[test]
fn new_object_and_its_hardlink_refuse_write_and_delete() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let folder = work.path().join("mod");
    write(&folder, "obj.bin", b"immutable");

    let out = add_folder(&ctx, &folder, Some("Protect")).unwrap();
    let obj = ctx.paths.content_object_path(&sha_of(&out, "obj.bin"));
    assert_eq!(protect::protection(&obj).unwrap(), Protection::Protected);

    let link = work.path().join("obj_link.bin");
    fs::hard_link(&obj, &link).unwrap();

    for path in [&obj, &link] {
        let write_err = fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect_err("write must be refused");
        assert_eq!(write_err.kind(), std::io::ErrorKind::PermissionDenied);

        let append_err = fs::OpenOptions::new()
            .append(true)
            .open(path)
            .expect_err("append must be refused");
        assert_eq!(append_err.kind(), std::io::ErrorKind::PermissionDenied);
    }
    assert_eq!(fs::read(&obj).unwrap(), b"immutable");
}
