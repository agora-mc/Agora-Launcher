//! Integration tests for the content store (MASTER_SPEC §26.6).

use std::fs::File;
use std::io::Write;
use std::path::Path;
use tempfile::TempDir;

use agora_core::content_store::{
    add_archive, add_folder, protect, remove_item, verify_all, verify_item, AddOutcome,
    ContentError, ProblemKind, Protection, VerifyDepth,
};
use agora_core::ctx::Ctx;

fn test_ctx() -> (TempDir, Ctx) {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = Ctx::for_testing(tmp.path().to_path_buf());
    (tmp, ctx)
}

fn create_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap();
}

fn create_zip_with_symlink(path: &Path) {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        zip.start_file("symlink_entry", zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(b"target").unwrap();
        zip.finish().unwrap();
    }

    let total = buf.len();
    let eocd_pos = total - 22;
    let cd_offset =
        u32::from_le_bytes(buf[eocd_pos + 16..eocd_pos + 20].try_into().unwrap()) as usize;
    let pos = cd_offset;
    buf[pos + 5] = 3; // OS = Unix
    let mode: u32 = 0o120777; // S_IFLNK
    let ext = mode << 16;
    buf[pos + 38..pos + 42].copy_from_slice(&ext.to_le_bytes());

    std::fs::write(path, buf).unwrap();
}

fn create_zip_with_oversized_entry(path: &Path) {
    // Creates a zip where local header says uncompressed_size = 5,
    // but the payload written is 10 bytes.
    let file = File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("overflow.txt", options).unwrap();
    zip.write_all(b"0123456789").unwrap();
    zip.finish().unwrap();

    // Patch the zip uncompressed size: change 10 (0x0A, 0x00, 0x00, 0x00) to 5 (0x05, 0x00, 0x00, 0x00)
    let mut bytes = std::fs::read(path).unwrap();
    let five = 5u32.to_le_bytes();

    // Patch local file header (uncompressed size at offset 22)
    if bytes.len() > 26 && &bytes[0..4] == b"PK\x03\x04" {
        bytes[22..26].copy_from_slice(&five);
    }
    // Patch central directory (search for PK\x01\x02)
    for i in 0..bytes.len().saturating_sub(28) {
        if &bytes[i..i + 4] == b"PK\x01\x02" {
            bytes[i + 24..i + 28].copy_from_slice(&five);
        }
    }
    std::fs::write(path, bytes).unwrap();
}

fn assert_store_empty(ctx: &Ctx) {
    let staging = ctx.paths.content_staging_dir();
    if staging.exists() {
        let count = std::fs::read_dir(&staging).unwrap().count();
        assert_eq!(count, 0, "staging directory should be empty");
    }
    let objects = ctx.paths.content_objects_dir();
    if objects.exists() {
        let count = std::fs::read_dir(&objects).unwrap().count();
        assert_eq!(count, 0, "objects directory should be empty");
    }
    let items = ctx.paths.content_items_dir();
    if items.exists() {
        let count = std::fs::read_dir(&items).unwrap().count();
        assert_eq!(count, 0, "items directory should be empty");
    }
}

// ---------------------------------------------------------------------------
// 1. Zip and Folder with same files are one item; adding twice is Existing
// ---------------------------------------------------------------------------

#[test]
fn test_zip_and_folder_same_item_and_sources() {
    let (_tmp, ctx) = test_ctx();
    let work_dir = tempfile::tempdir().unwrap();

    let zip_path = work_dir.path().join("mod.zip");
    create_zip(
        &zip_path,
        &[("file1.txt", b"hello"), ("sub/file2.txt", b"world")],
    );

    let folder_path = work_dir.path().join("mod_folder");
    std::fs::create_dir_all(folder_path.join("sub")).unwrap();
    std::fs::write(folder_path.join("file1.txt"), b"hello").unwrap();
    std::fs::write(folder_path.join("sub/file2.txt"), b"world").unwrap();

    // 1. Add zip
    let outcome1 = add_archive(&ctx, &zip_path, Some("TestMod")).unwrap();
    assert!(matches!(outcome1, AddOutcome::Added { .. }));
    let item1 = outcome1.item();
    assert_eq!(item1.files.len(), 2);
    assert_eq!(item1.sources.len(), 1);
    assert_eq!(outcome1.objects_new(), 2);

    // 2. Add folder with identical files
    let outcome2 = add_folder(&ctx, &folder_path, Some("TestMod")).unwrap();
    assert!(matches!(outcome2, AddOutcome::Existing { .. }));
    let item2 = outcome2.item();
    assert_eq!(item1.item_id, item2.item_id);
    assert_eq!(item2.sources.len(), 2);
    assert_eq!(outcome2.objects_new(), 0);
    assert_eq!(outcome2.objects_present(), 2);

    // 3. Adding again records no duplicate source
    let outcome3 = add_folder(&ctx, &folder_path, Some("TestMod")).unwrap();
    assert_eq!(outcome3.item().sources.len(), 2);
}

// ---------------------------------------------------------------------------
// 2. Objects are shared and removal retains objects other items need
// ---------------------------------------------------------------------------

#[test]
fn test_shared_objects_and_removal() {
    let (_tmp, ctx) = test_ctx();
    let work_dir = tempfile::tempdir().unwrap();

    let zip1 = work_dir.path().join("mod1.zip");
    create_zip(
        &zip1,
        &[
            ("shared.txt", b"shared-data"),
            ("unique1.txt", b"mod1-only"),
        ],
    );

    let zip2 = work_dir.path().join("mod2.zip");
    create_zip(
        &zip2,
        &[
            ("shared.txt", b"shared-data"),
            ("unique2.txt", b"mod2-only"),
        ],
    );

    let out1 = add_archive(&ctx, &zip1, Some("Mod1")).unwrap();
    assert_eq!(out1.objects_new(), 2);

    let out2 = add_archive(&ctx, &zip2, Some("Mod2")).unwrap();
    // shared.txt was already present, so only 1 new object
    assert_eq!(out2.objects_new(), 1);

    let shared_hash = &out1.item().files[0].sha256;
    let unique1_hash = &out1.item().files[1].sha256;
    let unique2_hash = &out2.item().files[1].sha256;

    let shared_path = ctx.paths.content_object_path(shared_hash);
    let u1_path = ctx.paths.content_object_path(unique1_hash);
    let u2_path = ctx.paths.content_object_path(unique2_hash);

    assert!(shared_path.exists());
    assert!(u1_path.exists());
    assert!(u2_path.exists());

    // Remove Item 1
    remove_item(&ctx, &out1.item().item_id).unwrap();

    // Item 1 manifest removed, unique1 removed, shared KEPT, unique2 KEPT
    assert!(!ctx.paths.content_item_path(&out1.item().item_id).exists());
    assert!(!u1_path.exists());
    assert!(shared_path.exists());
    assert!(u2_path.exists());

    // Item 2 verifies clean
    let ver = verify_item(&ctx, &out2.item().item_id, VerifyDepth::Full).unwrap();
    assert!(ver.problems.is_empty());
}

// ---------------------------------------------------------------------------
// 3. Refusal tests (Decision 5 & 6) - Each leaves nothing behind
// ---------------------------------------------------------------------------

#[test]
fn test_refusal_empty_component() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("bad.zip");
    create_zip(&zip_path, &[("a//b.txt", b"content")]);

    let res = add_archive(&ctx, &zip_path, None);
    assert!(matches!(res, Err(ContentError::InvalidPath { .. })));
    assert_store_empty(&ctx);
}

#[test]
fn test_refusal_dot_component() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("bad.zip");
    create_zip(&zip_path, &[("a/./b.txt", b"content")]);

    let res = add_archive(&ctx, &zip_path, None);
    assert!(matches!(res, Err(ContentError::InvalidPath { .. })));
    assert_store_empty(&ctx);
}

#[test]
fn test_refusal_trailing_slash_file() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("bad.zip");
    // Manually create zip with a file entry that has a trailing slash but is not marked as directory
    let file = File::create(&zip_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    // Explicit start_file without is_dir
    zip.start_file("a/b/", options).unwrap();
    zip.write_all(b"not a dir").unwrap();
    zip.finish().unwrap();

    let res = add_archive(&ctx, &zip_path, None);
    // Either ignored as directory or rejected as empty component / empty item
    assert!(res.is_err());
    assert_store_empty(&ctx);
}

#[test]
fn test_refusal_dot_ending_component() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("bad.zip");
    create_zip(&zip_path, &[("folder./file.txt", b"content")]);

    let res = add_archive(&ctx, &zip_path, None);
    assert!(matches!(res, Err(ContentError::InvalidPath { .. })));
    assert_store_empty(&ctx);
}

#[test]
fn test_refusal_space_ending_component() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("bad.zip");
    create_zip(&zip_path, &[("folder /file.txt", b"content")]);

    let res = add_archive(&ctx, &zip_path, None);
    assert!(matches!(res, Err(ContentError::InvalidPath { .. })));
    assert_store_empty(&ctx);
}

#[test]
fn test_refusal_windows_device_name() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("bad.zip");
    create_zip(&zip_path, &[("sub/con.txt", b"content")]);

    let res = add_archive(&ctx, &zip_path, None);
    assert!(matches!(res, Err(ContentError::InvalidPath { .. })));
    assert_store_empty(&ctx);
}

#[test]
fn test_refusal_case_collision() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("bad.zip");
    create_zip(
        &zip_path,
        &[("file.txt", b"content1"), ("FILE.TXT", b"content2")],
    );

    let res = add_archive(&ctx, &zip_path, None);
    assert!(matches!(res, Err(ContentError::InvalidPath { .. })));
    assert_store_empty(&ctx);
}

#[test]
fn test_refusal_prefix_folder_collision() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("bad.zip");
    create_zip(&zip_path, &[("a", b"file a"), ("a/b", b"file a/b")]);

    let res = add_archive(&ctx, &zip_path, None);
    assert!(matches!(res, Err(ContentError::InvalidPath { .. })));
    assert_store_empty(&ctx);
}

#[test]
fn test_refusal_symlink_entry() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("symlink.zip");
    create_zip_with_symlink(&zip_path);

    let res = add_archive(&ctx, &zip_path, None);
    assert!(matches!(res, Err(ContentError::InvalidPath { .. })));
    assert_store_empty(&ctx);
}

#[test]
fn test_refusal_empty_archive() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("empty.zip");
    create_zip(&zip_path, &[]);

    let res = add_archive(&ctx, &zip_path, None);
    assert!(matches!(res, Err(ContentError::EmptyItem)));
    assert_store_empty(&ctx);
}

#[test]
fn test_refusal_entry_exceeds_declared_size() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("overflow.zip");
    create_zip_with_oversized_entry(&zip_path);

    let res = add_archive(&ctx, &zip_path, None);
    assert!(matches!(
        res,
        Err(ContentError::EntryExceedsDeclaredSize { .. })
    ));
    assert_store_empty(&ctx);
}

#[test]
fn test_refusal_corrupt_zip() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();
    let zip_path = work.path().join("corrupt.zip");
    std::fs::write(&zip_path, b"PK\x03\x04truncated_garbage").unwrap();

    let res = add_archive(&ctx, &zip_path, None);
    assert!(matches!(res, Err(ContentError::CorruptArchive(_))));
    assert_store_empty(&ctx);
}

// ---------------------------------------------------------------------------
// 4. Protection on Windows
// ---------------------------------------------------------------------------

#[test]
#[cfg(windows)]
fn test_protection_on_windows() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("object_test.bin");
    std::fs::write(&file_path, b"immutable-content").unwrap();

    // Before protect: unprotected
    assert_eq!(
        protect::protection(&file_path).unwrap(),
        Protection::Unprotected
    );

    // Protect
    protect::protect(&file_path).unwrap();
    assert_eq!(
        protect::protection(&file_path).unwrap(),
        Protection::Protected
    );

    // Writing fails with PermissionDenied
    let write_res = std::fs::OpenOptions::new().write(true).open(&file_path);
    assert!(write_res.is_err());
    assert_eq!(
        write_res.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );

    // Reading works
    let read_bytes = std::fs::read(&file_path).unwrap();
    assert_eq!(read_bytes, b"immutable-content");

    // Creating hard link works (FILE_WRITE_ATTRIBUTES is not denied)
    let link_path = dir.path().join("object_test_link.bin");
    std::fs::hard_link(&file_path, &link_path).unwrap();
    assert!(link_path.exists());

    // Unprotect
    protect::unprotect(&file_path).unwrap();
    assert_eq!(
        protect::protection(&file_path).unwrap(),
        Protection::Unprotected
    );

    // Writing now works
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&file_path)
        .unwrap();
    file.write_all(b"updated").unwrap();
}

// ---------------------------------------------------------------------------
// 5. Verify finds each problem kind
// ---------------------------------------------------------------------------

#[test]
fn test_verify_problem_kinds() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();

    let zip_path = work.path().join("mod.zip");
    create_zip(
        &zip_path,
        &[("file1.txt", b"content1"), ("file2.txt", b"content2")],
    );

    let out = add_archive(&ctx, &zip_path, Some("VerifyTest")).unwrap();
    let item_id = out.item().item_id.clone();

    // Verify clean initially
    let ver_clean = verify_item(&ctx, &item_id, VerifyDepth::Quick).unwrap();
    assert!(ver_clean.problems.is_empty());

    // 1. Delete an object -> Missing
    let obj1_hash = &out.item().files[0].sha256;
    let obj1_path = ctx.paths.content_object_path(obj1_hash);
    protect::unprotect(&obj1_path).unwrap();
    std::fs::remove_file(&obj1_path).unwrap();

    let ver_missing = verify_item(&ctx, &item_id, VerifyDepth::Quick).unwrap();
    assert_eq!(ver_missing.problems.len(), 1);
    assert!(matches!(ver_missing.problems[0].kind, ProblemKind::Missing));

    // Restore obj1
    std::fs::write(&obj1_path, b"content1").unwrap();
    protect::protect(&obj1_path).unwrap();

    // 2. Unprotect and rewrite with different bytes of same length
    let obj2_hash = &out.item().files[1].sha256;
    let obj2_path = ctx.paths.content_object_path(obj2_hash);
    protect::unprotect(&obj2_path).unwrap();
    std::fs::write(&obj2_path, b"contentX").unwrap(); // same 8 bytes

    // Quick depth: size matches, but Unprotected
    let ver_quick = verify_item(&ctx, &item_id, VerifyDepth::Quick).unwrap();
    assert_eq!(ver_quick.problems.len(), 1);
    assert!(matches!(
        ver_quick.problems[0].kind,
        ProblemKind::Unprotected
    ));

    // Full depth: Unprotected AND HashMismatch
    let ver_full = verify_item(&ctx, &item_id, VerifyDepth::Full).unwrap();
    assert!(ver_full
        .problems
        .iter()
        .any(|p| matches!(p.kind, ProblemKind::Unprotected)));
    assert!(ver_full
        .problems
        .iter()
        .any(|p| matches!(p.kind, ProblemKind::HashMismatch { .. })));

    // 3. Manifest replaced with garbage -> CorruptManifest (reported, not skipped)
    let manifest_path = ctx.paths.content_item_path(&item_id);
    std::fs::write(&manifest_path, b"{ not valid json }").unwrap();

    let ver_report = verify_all(&ctx, VerifyDepth::Quick).unwrap();
    assert!(ver_report
        .problems
        .iter()
        .any(|p| matches!(p.kind, ProblemKind::CorruptManifest { .. })));
}

// ---------------------------------------------------------------------------
// 6. Removal refuses when another manifest is unreadable
// ---------------------------------------------------------------------------

#[test]
fn test_removal_refuses_when_another_manifest_unreadable() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();

    let zip1 = work.path().join("mod1.zip");
    create_zip(&zip1, &[("file1.txt", b"mod1-data")]);
    let out1 = add_archive(&ctx, &zip1, Some("Mod1")).unwrap();

    let zip2 = work.path().join("mod2.zip");
    create_zip(&zip2, &[("file2.txt", b"mod2-data")]);
    let out2 = add_archive(&ctx, &zip2, Some("Mod2")).unwrap();

    // Corrupt Mod2 manifest
    let m2_path = ctx.paths.content_item_path(&out2.item().item_id);
    std::fs::write(&m2_path, b"corrupt manifest").unwrap();

    // Attempt to remove Mod1
    let res = remove_item(&ctx, &out1.item().item_id);
    assert!(matches!(res, Err(ContentError::UnreadableManifest { .. })));

    // Mod1 manifest and object still exist! Deletes nothing.
    assert!(ctx.paths.content_item_path(&out1.item().item_id).exists());
    let obj1_path = ctx.paths.content_object_path(&out1.item().files[0].sha256);
    assert!(obj1_path.exists());
}

// ---------------------------------------------------------------------------
// 7. Re-adding an item whose object was deleted restores it (restored: 1)
// ---------------------------------------------------------------------------

#[test]
fn test_readd_restores_deleted_object() {
    let (_tmp, ctx) = test_ctx();
    let work = tempfile::tempdir().unwrap();

    let zip_path = work.path().join("mod.zip");
    create_zip(
        &zip_path,
        &[("file1.txt", b"content1"), ("file2.txt", b"content2")],
    );

    let out1 = add_archive(&ctx, &zip_path, Some("RestoreTest")).unwrap();
    assert_eq!(out1.objects_new(), 2);

    // Delete object 1
    let obj1_path = ctx.paths.content_object_path(&out1.item().files[0].sha256);
    protect::unprotect(&obj1_path).unwrap();
    std::fs::remove_file(&obj1_path).unwrap();
    assert!(!obj1_path.exists());

    // Re-add
    let out2 = add_archive(&ctx, &zip_path, Some("RestoreTest")).unwrap();
    assert!(matches!(out2, AddOutcome::Existing { .. }));
    assert_eq!(out2.objects_restored(), 1);
    assert_eq!(out2.objects_present(), 1);
    assert!(obj1_path.exists());
    assert_eq!(
        protect::protection(&obj1_path).unwrap(),
        Protection::Protected
    );
}
