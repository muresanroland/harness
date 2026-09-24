//! Tools is the one seam to every external tool (herdr, bd, gh, git, claude).

use std::fmt;
use std::path::Path;
use std::process::Command;

/// A failed command: `Display` is "<command>: <status>: <stderr>".
#[derive(Debug, Clone, PartialEq)]
pub struct RunError {
    /// The command line, space-joined.
    pub command: String,
    /// "exit status N", or why it could not run.
    pub status: String,
    /// Trimmed stderr.
    pub stderr: String,
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.command, self.status, self.stderr)
    }
}

impl std::error::Error for RunError {}

/// The space-joined command line of a failure, with the TypeSafe key redacted:
/// the Debate pane takes it as --env, and a failed spawn lands in the log.
fn command_line(argv: &[&str]) -> String {
    argv.iter()
        .map(|arg| match arg.strip_prefix("TYPESAFE_API_KEY=") {
            Some(_) => "TYPESAFE_API_KEY=***",
            None => arg,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runs argv in dir and returns stdout; a failure carries stderr.
pub trait Tools: Send + Sync {
    fn run(&self, dir: &Path, argv: &[&str]) -> Result<String, RunError>;
}

/// The real Tools.
pub struct Exec;

impl Tools for Exec {
    fn run(&self, dir: &Path, argv: &[&str]) -> Result<String, RunError> {
        let [name, args @ ..] = argv else {
            return Err(RunError {
                command: String::new(),
                status: "no command".to_string(),
                stderr: String::new(),
            });
        };
        let command = command_line(argv);
        let output = Command::new(name)
            .args(args)
            .current_dir(dir)
            .output()
            .map_err(|err| RunError {
                command: command.clone(),
                status: err.to_string(),
                stderr: String::new(),
            })?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.status.success() {
            let status = match output.status.code() {
                Some(code) => format!("exit status {code}"),
                None => output.status.to_string(),
            };
            return Err(RunError {
                command,
                status,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(stdout)
    }
}

/// The test double for the Tools seam.
#[cfg(test)]
pub(crate) mod fake {
    use super::{RunError, Tools};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// Answers a call with stdout, or with the stderr of a failure.
    pub(crate) type Handle = dyn Fn(&Path, &[&str]) -> Result<String, String> + Send + Sync;

    /// Records every call and answers it from `handle`; no handle returns ""
    /// with no error.
    pub(crate) struct Fake {
        calls: Mutex<Vec<String>>,
        handle: Option<Box<Handle>>,
    }

    impl Fake {
        pub(crate) fn new(
            handle: impl Fn(&Path, &[&str]) -> Result<String, String> + Send + Sync + 'static,
        ) -> Arc<Self> {
            Arc::new(Fake {
                calls: Mutex::new(Vec::new()),
                handle: Some(Box::new(handle)),
            })
        }

        pub(crate) fn quiet() -> Arc<Self> {
            Arc::new(Fake {
                calls: Mutex::new(Vec::new()),
                handle: None,
            })
        }

        /// Every call so far, in order, as space-joined command lines.
        pub(crate) fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Tools for Fake {
        fn run(&self, dir: &Path, argv: &[&str]) -> Result<String, RunError> {
            self.calls.lock().unwrap().push(argv.join(" "));
            match &self.handle {
                None => Ok(String::new()),
                Some(handle) => handle(dir, argv).map_err(|stderr| RunError {
                    command: super::command_line(argv),
                    status: "exit status 1".to_string(),
                    stderr,
                }),
            }
        }
    }
}

#[cfg(test)]
mod tools_test;
