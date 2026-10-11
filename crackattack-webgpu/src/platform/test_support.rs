//! Shared helpers for the tests of this module (compiled in test builds only).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Unique directory path in the system temp directory (pid + counter), removed on drop
/// — also when the test fails. The directory is **not** created: the test decides.
pub(crate) struct TestDir(PathBuf);

impl TestDir {
    pub(crate) fn new(tag: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("ca_test_{tag}_{}_{n}", std::process::id())))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
