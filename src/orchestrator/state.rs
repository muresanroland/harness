//! The state file and the lock: what the Orchestrator knows about every
//! Ticket, saved atomically, and one run per Target repo.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(crate) const STATUS_RUNNING: &str = "running";
pub(crate) const STATUS_PARKED: &str = "parked";
pub(crate) const STATUS_PR_OPEN: &str = "pr-open";
pub(crate) const STATUS_MERGED: &str = "merged";

/// What the Orchestrator knows about one Ticket. The JSON is Go's: the same
/// names, order and omissions, so either binary picks up the other's run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct TicketState {
    pub(crate) status: String,
    pub(crate) stage: String,
    pub(crate) round: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) tab: String,
    /// Stage name -> pane id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) panes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) pr: String,
    /// Why it is Parked.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) reason: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) retried: bool,
    #[serde(
        default,
        rename = "conflict_reported",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub(crate) conflict: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct State {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) epic: String,
    #[serde(default)]
    pub(crate) tickets: BTreeMap<String, TicketState>,
}

fn state_path(repo: &Path) -> PathBuf {
    repo.join(".harness").join("state.json")
}

pub(crate) fn load_state(repo: &Path) -> io::Result<State> {
    let path = state_path(repo);
    let raw = match fs::read(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(State::default()),
        Err(err) => return Err(err),
    };
    serde_json::from_slice(&raw)
        .map_err(|err| io::Error::other(format!("{}: {err}", path.display())))
}

impl State {
    /// Writes the state file atomically: a reader sees the old or the new
    /// state, never half of one.
    pub(crate) fn save(&self, repo: &Path) -> io::Result<()> {
        let path = state_path(repo);
        let raw = serde_json::to_vec_pretty(self)?;
        fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, raw)?;
        fs::rename(tmp, path)
    }
}

/// 'harness status'.
pub(crate) fn print_status(out: &mut dyn Write, repo: &Path) -> i32 {
    let state = match load_state(repo) {
        Ok(state) => state,
        Err(err) => {
            let _ = writeln!(out, "status: {err}");
            return 1;
        }
    };
    let _ = match lock_holder(repo) {
        0 => write!(out, "Orchestrator not running"),
        pid => write!(out, "Orchestrator running (pid {pid})"),
    };
    if !state.epic.is_empty() {
        let _ = write!(out, ", Epic {}", state.epic);
    }
    let _ = writeln!(out);
    for (id, ts) in &state.tickets {
        let mut line = format!("{id:<20} {:<8} {}", ts.status, ts.stage);
        if ts.round > 0 {
            line += &format!(" round {}", ts.round);
        }
        for extra in [&ts.pr, &ts.reason] {
            if !extra.is_empty() {
                line += "  ";
                line += extra;
            }
        }
        let _ = writeln!(out, "{line}");
    }
    0
}

fn lock_path(repo: &Path) -> PathBuf {
    repo.join(".harness").join("lock")
}

/// The lock on a Target repo: an advisory flock the kernel releases when the
/// holder dies, with the pid inside for messages (ADR 0003). Dropping it
/// releases the lock and removes the file.
#[derive(Debug)]
pub(crate) struct Lock {
    _file: File,
    path: PathBuf,
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// The pid of the live Orchestrator holding this Target repo's lock, or 0.
pub(crate) fn lock_holder(repo: &Path) -> u32 {
    let Ok(file) = File::open(lock_path(repo)) else {
        return 0;
    };
    match file.try_lock() {
        Err(TryLockError::WouldBlock) => fs::read_to_string(lock_path(repo))
            .ok()
            .and_then(|raw| raw.trim().parse().ok())
            .unwrap_or(0),
        _ => 0, // ours now, so nobody's: stale, that Orchestrator was killed
    }
}

/// Enforces one run per Target repo.
pub(crate) fn acquire_lock(repo: &Path) -> io::Result<Lock> {
    let path = lock_path(repo);
    fs::create_dir_all(path.parent().unwrap())?;
    let mut file = File::options().create(true).append(true).open(&path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            // ponytail: the holder writes its pid right after locking, so a
            // start in that same instant names pid 0; it still refuses.
            let pid = lock_holder(repo);
            return Err(io::Error::other(format!(
                "an Orchestrator is already running in this repo (pid {pid}); 'harness stop' ends it"
            )));
        }
        Err(TryLockError::Error(err)) => return Err(err),
    }
    file.set_len(0)?;
    write!(file, "{}", std::process::id())?;
    Ok(Lock { _file: file, path })
}
