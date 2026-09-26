//! An App started in a directory it does not trust yet opens a trust dialog
//! instead of working. herdr reads claude's and codex's dialogs as a blocked
//! pane; how it reads pi's, copilot's and cursor's is unverified, and
//! opencode has none. No dialog can be answered by the Orchestrator, so it
//! reads what the Apps themselves record and waits rather than prompting
//! into a dialog.
//!
//! A Ticket's worktree is a fresh directory every time, so this is the normal
//! case, not an edge one.

use std::fs;
use std::path::{Path, PathBuf};

use super::app::App;

/// Whether the App already trusts dir. claude and codex record trust against
/// the project root they resolved, which for a worktree or a run directory
/// is an ancestor, so dir's ancestors up to repo answer for it. The nearest
/// recorded directory decides: a subdirectory recorded as untrusted is
/// untrusted however its repo is recorded.
pub(crate) fn trusts(app: &App, home: &Path, dir: &Path, repo: &Path) -> bool {
    std::iter::once(dir.to_path_buf())
        .chain(ancestors(dir, repo))
        .find_map(|candidate| (app.trust)(home, &candidate))
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
pub(super) fn claude_records(home: &Path, dir: &Path) -> Option<bool> {
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
pub(super) fn codex_records(home: &Path, dir: &Path) -> Option<bool> {
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

// The three below are from the research/agent-clis notes, never run here.
// Each App lets a trusted ancestor cover dir, above the repo too.

/// Reads ~/.pi/agent/trust.json, a directory to its answer: the nearest one
/// recorded decides. pi asks only in a folder with protected resources, a
/// .agents/skills or a .pi holding anything, there or in a parent below
/// home: with none it asks nothing, records nothing and is trusted.
/// Otherwise nothing recorded is untrusted: dir's ancestors were read here.
pub(super) fn pi_records(home: &Path, dir: &Path) -> Option<bool> {
    let protected = dir.ancestors().take_while(|d| *d != home).any(|d| {
        d.join(".agents/skills").exists()
            || fs::read_dir(d.join(".pi")).is_ok_and(|mut entries| entries.next().is_some())
    });
    if !protected {
        return Some(true);
    }
    let recorded = || {
        let raw = fs::read(home.join(".pi/agent/trust.json")).ok()?;
        let doc: serde_json::Value = serde_json::from_slice(&raw).ok()?;
        dir.ancestors()
            .find_map(|d| doc.get(d.to_str()?)?.as_bool())
    };
    Some(recorded().unwrap_or(false))
}

/// Reads trustedFolders in ~/.copilot/config.json; it records no untrusted
/// ones.
pub(super) fn copilot_records(home: &Path, dir: &Path) -> Option<bool> {
    let raw = fs::read(home.join(".copilot/config.json")).ok()?;
    let doc: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let folders = doc.get("trustedFolders")?.as_array()?;
    let listed = |d: &Path| folders.iter().any(|f| f.as_str().map(Path::new) == Some(d));
    dir.ancestors().any(listed).then_some(true)
}

/// Reads cursor's marker, ~/.cursor/projects/<slug>/.workspace-trusted; it
/// records no untrusted ones.
pub(super) fn cursor_records(home: &Path, dir: &Path) -> Option<bool> {
    let projects = home.join(".cursor/projects");
    dir.ancestors()
        .any(|d| {
            projects
                .join(cursor_slug(d))
                .join(".workspace-trusted")
                .exists()
        })
        .then_some(true)
}

/// The name cursor keeps a directory's project under.
// ponytail: the research names <slug> without its rule; this is claude's
// without the leading dash, unverified.
pub(super) fn cursor_slug(dir: &Path) -> String {
    claude_slug(dir).trim_start_matches('-').to_string()
}

/// The folder claude keeps a directory's transcripts in, under
/// ~/.claude/projects: every character but a letter or digit a dash, and
/// past 200 its first 200 and a hash of the path. claude counts and hashes
/// a path's UTF-16 units, as JavaScript does.
pub(crate) fn claude_slug(dir: &Path) -> String {
    let path = dir.to_string_lossy();
    let slug: String = path
        .encode_utf16()
        .map(|u| match char::from_u32(u.into()) {
            Some(c) if c.is_ascii_alphanumeric() => c,
            _ => '-',
        })
        .collect();
    if slug.len() <= 200 {
        return slug;
    }
    let hash = path.encode_utf16().fold(0i32, |h, u| {
        h.wrapping_shl(5).wrapping_sub(h).wrapping_add(u.into())
    });
    let (mut n, mut base36) = (i64::from(hash).unsigned_abs(), Vec::new());
    loop {
        base36.push(char::from_digit((n % 36) as u32, 36).unwrap());
        n /= 36;
        if n == 0 {
            break;
        }
    }
    format!(
        "{}-{}",
        &slug[..200],
        base36.iter().rev().collect::<String>()
    )
}
