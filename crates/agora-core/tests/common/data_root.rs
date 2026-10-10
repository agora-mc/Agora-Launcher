//! A temporary data root for tests that store content.
//!
//! Every object the content store adds is protected by a deny ACE (`content_store::protect`). A
//! `TempDir` removes its tree with plain deletes, and a protected object can refuse one: its drop
//! error is ignored, so the object stays behind. `DataRoot` unprotects the objects before its
//! `TempDir` is removed, so nothing is left behind. Production protection is unchanged.

use std::path::Path;

use agora_core::content_store::protect;
use tempfile::TempDir;

/// A temporary folder that removes its content objects, protected or not, when it drops.
pub struct DataRoot {
    dir: TempDir,
}

impl DataRoot {
    /// A fresh folder in the system temp directory.
    pub fn new() -> Self {
        Self {
            dir: TempDir::new().unwrap(),
        }
    }

    /// A fresh folder with `prefix`, inside `parent`.
    pub fn in_dir(parent: &Path, prefix: &str) -> Self {
        Self {
            dir: tempfile::Builder::new()
                .prefix(prefix)
                .tempdir_in(parent)
                .unwrap(),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

impl Drop for DataRoot {
    fn drop(&mut self) {
        unprotect_objects(self.dir.path());
    }
}

/// Unprotect every object under `root`, at any depth: each `content/objects` folder in the tree
/// (a store can sit below the root, as the benchmark's `app_data` does). Errors are ignored; a
/// failed unprotect shows up as a failed removal in the `TempDir` drop.
pub fn unprotect_objects(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        // Links are not followed: a link back into the tree would make this walk loop.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        let is_objects = path.file_name().is_some_and(|n| n == "objects")
            && path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "content");
        if is_objects {
            unprotect_shards(&path);
        } else {
            unprotect_objects(&path);
        }
    }
}

fn unprotect_shards(objects: &Path) {
    let Ok(shards) = std::fs::read_dir(objects) else {
        return;
    };
    for shard in shards.flatten() {
        let Ok(objects) = std::fs::read_dir(shard.path()) else {
            continue;
        };
        for object in objects.flatten() {
            let _ = protect::unprotect(&object.path());
        }
    }
}
