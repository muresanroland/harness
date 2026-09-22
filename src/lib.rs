//! The Harness: drives a beads Epic through the Pipeline. The Go tree beside
//! this crate is the spec until the cutover removes it; `pub(crate)` stands in
//! for Go's `internal/`.

pub mod cli;
pub(crate) mod orchestrator;
pub(crate) mod setup;
pub(crate) mod sha256;
pub(crate) mod skills;
pub mod tools;
pub mod version;

/// A scratch directory that is removed on Drop, the port of Go's t.TempDir().
#[cfg(test)]
pub(crate) mod tempdir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "harness-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
