//! The Harness: drives a beads Epic through the Pipeline.

pub mod cli;
pub(crate) mod orchestrator;
pub(crate) mod setup;
pub(crate) mod shell;
pub(crate) mod skills;
pub mod tools;
pub(crate) mod update;
pub mod version;

/// A scratch directory that is removed on Drop.
pub(crate) mod tempdir {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        /// A directory that did not exist before: one a killed or leaking
        /// process left under a pid now reused must not be reused with it,
        /// stale files and all.
        pub(crate) fn create() -> io::Result<Self> {
            static N: AtomicU64 = AtomicU64::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "harness-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Ok(TempDir(path)),
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(err) => {
                        return Err(io::Error::new(
                            err.kind(),
                            format!("{}: {err}", path.display()),
                        ))
                    }
                }
            }
        }

        #[cfg(test)]
        pub(crate) fn new() -> Self {
            Self::create().unwrap_or_else(|err| panic!("{err}"))
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
