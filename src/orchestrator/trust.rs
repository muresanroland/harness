//! An App started in a directory it does not trust yet opens a trust dialog
//! instead of working. herdr reads either App's dialog, Claude's or Codex's,
//! as a blocked pane. Neither dialog can be answered by the Orchestrator, so
//! it reads what the Apps themselves record and waits rather than prompting
//! into a dialog.
//!
//! A Ticket's worktree is a fresh directory every time, so this is the normal
//! case, not an edge one.

use std::fs;
use std::path::{Path, PathBuf};

/// Whether kind already trusts dir. Both agents record trust against the
/// project root they resolved, which for a worktree or a run directory is an
/// ancestor, so dir's ancestors up to repo answer for it. The nearest recorded
/// directory decides: a subdirectory recorded as untrusted is untrusted
/// however its repo is recorded.
pub(crate) fn trusts(kind: &str, home: &Path, dir: &Path, repo: &Path) -> bool {
    let recorded = if kind == "codex" {
        codex_records
    } else {
        claude_records
    };
    std::iter::once(dir.to_path_buf())
        .chain(ancestors(dir, repo))
        .find_map(|candidate| recorded(home, &candidate))
        .unwrap_or(false)
}

/// dir's parents up to and including repo, nearest first.
pub(crate) fn ancestors(dir: &Path, repo: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut next = dir.parent();
    while let Some(d) = next {
        if !d.starts_with(repo) || d == Path::new("/") {
            break;
        }
        out.push(d.to_path_buf());
        if d == repo {
            break;
        }
        next = d.parent();
    }
    out
}

/// Reads ~/.claude.json, where an accepted trust dialog is recorded per
/// project directory: Some(trusted) when dir is recorded, None when not.
fn claude_records(home: &Path, dir: &Path) -> Option<bool> {
    let raw = fs::read(home.join(".claude.json")).ok()?;
    let doc: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let entry = doc.get("projects")?.get(dir.to_str()?)?;
    match entry.get("hasTrustDialogAccepted") {
        None => Some(false),
        Some(value) => value.as_bool(), // not a bool: unknown
    }
}

/// Reads ~/.codex/config.toml, where a trusted project is a [projects."<dir>"]
/// table with trust_level = "trusted".
// ponytail: a line scan, not a TOML parser; that one table shape is all the
// Orchestrator reads, and a parser would be a dependency.
fn codex_records(home: &Path, dir: &Path) -> Option<bool> {
    let raw = fs::read_to_string(home.join(".codex").join("config.toml")).ok()?;
    let want = format!("[projects.\"{}\"]", dir.display());
    let mut in_table = false;
    for line in raw.lines().map(str::trim) {
        if line.starts_with('[') {
            in_table = line == want;
            continue;
        }
        if in_table && line.starts_with("trust_level") {
            let value = line.split_once('=').map_or("", |(_, value)| value);
            return Some(value.trim().trim_matches('"') == "trusted");
        }
    }
    None
}
