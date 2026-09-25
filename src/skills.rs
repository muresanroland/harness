//! The skills the Harness ships: one Stage skill per Stage, plus create-pr. 'harness init' copies them into a Target repo. A new skill
//! is one more line here.

use std::fs;
use std::io;
use std::path::Path;

/// (name, body of skills/<name>/SKILL.md), sorted by name.
pub(crate) const SKILLS: &[(&str, &str)] = &[
    ("create-pr", include_str!("../skills/create-pr/SKILL.md")),
    (
        "stage-address",
        include_str!("../skills/stage-address/SKILL.md"),
    ),
    ("stage-fix", include_str!("../skills/stage-fix/SKILL.md")),
    (
        "stage-implement",
        include_str!("../skills/stage-implement/SKILL.md"),
    ),
    (
        "stage-moderate",
        include_str!("../skills/stage-moderate/SKILL.md"),
    ),
    (
        "stage-review",
        include_str!("../skills/stage-review/SKILL.md"),
    ),
];

/// A Stage skill's installed copy, the one its Stage runs, wherever init put
/// it: a copy committed in the repo first, then the checkout's, then the
/// user's (no home, none). Only an absent copy falls through: one that
/// cannot be read is said.
pub(crate) fn stage_skill(repo: &Path, home: &Path, name: &str) -> Option<Result<String, String>> {
    let mut dirs = vec![repo.join(".agents/skills"), repo.join(".harness/skills")];
    if !home.as_os_str().is_empty() {
        dirs.push(home.join(".agents/skills"));
    }
    dirs.iter().find_map(|dir| {
        let path = dir.join(name).join("SKILL.md");
        match fs::read_to_string(&path) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            read => Some(read.map_err(|err| format!("cannot read {}: {err}", path.display()))),
        }
    })
}

// Nothing outside the tests calls update and remove until /config (Ticket 21).
#[allow(dead_code)]
pub(crate) mod manifest;

#[cfg(test)]
mod manifest_test;
