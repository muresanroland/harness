//! The Harness: drives a beads Epic through the Pipeline. The Go tree beside
//! this crate is the spec until the cutover removes it; `pub(crate)` stands in
//! for Go's `internal/`.

pub mod cli;
pub(crate) mod orchestrator;
pub(crate) mod setup;
pub(crate) mod sha256;
pub(crate) mod shell;
pub(crate) mod skills;
pub mod tools;
pub(crate) mod update;
pub mod version;

/// A scratch directory that is removed on Drop, the port of Go's t.TempDir().
#[cfg(test)]
pub(crate) mod tempdir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        /// A directory that did not exist before: one a killed or leaking
        /// test process left under a pid now reused must not be reused with
        /// it, stale run files and all.
        pub(crate) fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "harness-test-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => return TempDir(path),
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(err) => panic!("{}: {err}", path.display()),
                }
            }
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
