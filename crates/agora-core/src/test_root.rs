//! A folder that a unit test creates under the system temp directory, removed when the test ends.
//!
//! Tests that only use `std::env::temp_dir().join(..)` leave their folders behind unless they
//! remove them by hand, and a test that fails part way never gets to. Holding one of these keeps the
//! folder for the test's life, and its drop removes the whole tree.

use std::ops::Deref;
use std::path::{Path, PathBuf};

/// A folder removed (with everything in it) when this value drops.
#[derive(Debug)]
pub(crate) struct RemoveOnDrop(PathBuf);

impl RemoveOnDrop {
    /// Take ownership of `path`. The folder is not created here.
    pub(crate) fn new(path: PathBuf) -> Self {
        Self(path)
    }
}

impl Deref for RemoveOnDrop {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for RemoveOnDrop {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
