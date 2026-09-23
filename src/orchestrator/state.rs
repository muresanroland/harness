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

/// What the Orchestrator knows about one Ticket. The JSON began as Go's (the
/// same names, order and omissions); nudged and waits came after the Go tree
/// was gone.
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
    /// The Stage's live session has had its one nudge.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) nudged: bool,
    /// The waits the Stage's live session has taken, of three.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(crate) waits: usize,
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
    #[serde(default, deserialize_with = "null_is_empty")]
    pub(crate) tickets: BTreeMap<String, TicketState>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Go's nil map: "tickets": null loads as no Tickets.
fn null_is_empty<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, TicketState>, D::Error> {
    Option::deserialize(d).map(Option::unwrap_or_default)
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

fn lock_path(repo: &Path) -> PathBuf {
    repo.join(".harness").join("lock")
}

/// The lock on a Target repo: an advisory flock the kernel releases when the
/// holder dies, with the pid inside for messages (ADR 0003). Dropping it
/// releases the lock; the file stays, since unlinking it would let two later
/// starts lock two different inodes. An unflocked file reads as stale.
#[derive(Debug)]
pub(crate) struct Lock {
    _flock: File,
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

/// Tests only: whether the lock can be taken within a second. A process
/// spawned on another test thread holds a copy of every open descriptor until
/// it execs, so a lock closed during a parallel test's spawn stays held that long.
#[cfg(test)]
pub(crate) fn lock_frees(repo: &Path) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while acquire_lock(repo).is_err() {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    true
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
                "a run is live in this repo (pid {pid}); /stop-work there ends it"
            )));
        }
        Err(TryLockError::Error(err)) => return Err(err),
    }
    file.set_len(0)?;
    write!(file, "{}", std::process::id())?;
    Ok(Lock { _flock: file })
}
