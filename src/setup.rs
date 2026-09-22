//! The thin 'harness init': installs the shipped skills into a Target repo,
//! keeps the TypeSafe key, and preflights it.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::{symlink, OpenOptionsExt};
use std::path::Path;

use crate::sha256::sha256;
use crate::skills::SKILLS;
use crate::tools::Tools;

/// The record of every skill file init wrote, path to sha256 of its content:
/// under refresh, a file that still matches is unedited and is rewritten.
const RECORD: &str = ".harness/installed-skills.json";
const KEY_FILE: &str = ".harness/typesafe-key";

#[derive(Clone, Copy)]
enum Mode {
    Fresh,
    Refresh,
    Overwrite,
}

/// Writes the Harness's skills to <repo>/.agents/skills and links them from
/// <repo>/.claude/skills. When the shipped skills are already installed the
/// gate asks first: cancel (false: nothing touched), refresh only the files
/// unedited since install (by the record), or overwrite everything; force is
/// overwrite unasked. A Target repo that already has a create-pr skill of its
/// own is asked what to do with the shipped one, since the Fix Stage runs
/// whichever /create-pr the repo ends up with; later inits keep that answer
/// from the record. `input` answers the questions, in raw mode when `tty`.
pub(crate) fn install_skills(
    repo: &Path,
    force: bool,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<bool> {
    let mut record: BTreeMap<String, String> = fs::read_to_string(repo.join(RECORD))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let mode = if force {
        Mode::Overwrite
    } else if installed(repo) {
        write!(
            out,
            "init: the shipped skills are already installed here.\r\n"
        )?;
        match menu(
            out,
            &mut *input,
            tty,
            &[
                "cancel, leave them as they are",
                "refresh only the skills not edited since install",
                "overwrite everything with the shipped skills",
            ],
        )? {
            0 => return Ok(false),
            1 => Mode::Refresh,
            _ => Mode::Overwrite,
        }
    } else {
        Mode::Fresh
    };
    // The name the shipped create-pr is installed under; "" keeps the repo's own.
    let recorded = ["create-pr", "harness-create-pr"]
        .into_iter()
        .find(|name| record.contains_key(&skill_path(name)));
    let pr = match (recorded, mode) {
        (Some(name), _) => name,
        (None, _) if !has_skill(repo, "create-pr") => "create-pr",
        (None, Mode::Fresh) => ask_about_create_pr(out, input, tty)?,
        (None, _) => "", // the repo's own, kept on the first init
    };
    for &(skill, body) in SKILLS {
        let name = match skill {
            "create-pr" if pr.is_empty() => continue,
            "create-pr" => pr,
            _ => skill,
        };
        let rel = skill_path(name);
        let dest = repo.join(&rel);
        let existing = fs::symlink_metadata(&dest).ok();
        // A link is the repo's own arrangement: never written through.
        if existing
            .as_ref()
            .is_some_and(|meta| meta.file_type().is_symlink())
        {
            continue;
        }
        let write = match mode {
            Mode::Overwrite => true,
            Mode::Refresh => record
                .get(&rel)
                .is_some_and(|hash| fs::read(&dest).is_ok_and(|now| sha256(&now) == *hash)),
            Mode::Fresh => skill == "create-pr" || existing.is_none(),
        };
        if !write {
            continue;
        }
        let body = if name != skill {
            // Installed beside the repo's own, so it needs its own name in the text.
            body.replace("create-pr", name)
        } else {
            body.to_string()
        };
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::write(&dest, &body)?;
        record.insert(rel, sha256(body.as_bytes()));
    }
    fs::create_dir_all(repo.join(".claude/skills"))?;
    for &(skill, _) in SKILLS {
        let mut name = skill;
        if skill == "create-pr" {
            if pr.is_empty() {
                continue;
            }
            name = pr;
        }
        let link = repo.join(".claude/skills").join(name);
        if let Ok(meta) = fs::symlink_metadata(&link) {
            if name == pr && !meta.file_type().is_symlink() {
                writeln!(out, "init: .claude/skills/{name} is this repo's own copy, not a link: remove it to use the shipped skill")?;
            }
            continue;
        }
        symlink(Path::new("../../.agents/skills").join(name), link)?;
    }
    fs::create_dir_all(repo.join(".harness"))?;
    fs::write(
        repo.join(RECORD),
        serde_json::to_string_pretty(&record)? + "\n",
    )?;
    ignore_run_dir(repo)?;
    Ok(true)
}

fn skill_path(name: &str) -> String {
    format!(".agents/skills/{name}/SKILL.md")
}

/// Whether a shipped skill is already installed. create-pr does not count: a
/// repo's own is not an install, and it has its own question.
// ponytail: a repo that deleted every Stage skill but kept a shipped create-pr
// reads as fresh; the record would tell, but nobody has done that.
fn installed(repo: &Path) -> bool {
    SKILLS
        .iter()
        .any(|&(name, _)| name != "create-pr" && repo.join(skill_path(name)).exists())
}

/// Asks for the TypeSafe API key with echo off and keeps it, readable only by
/// the user, when TYPESAFE_API_KEY (`env_key`) is unset and no key is stored.
/// An empty answer, Ctrl-C, Ctrl-D or a silent stdin skips the question.
pub(crate) fn ask_typesafe_key(
    repo: &Path,
    env_key: &str,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    if !env_key.trim().is_empty() || repo.join(KEY_FILE).exists() {
        return Ok(());
    }
    write!(
        out,
        "init: TypeSafe API key, kept in {KEY_FILE} (enter to skip): "
    )?;
    out.flush()?;
    let key = raw(tty, || {
        let mut key = Vec::new();
        let mut byte = [0u8; 1];
        while let Ok(1) = input.read(&mut byte) {
            match byte[0] {
                b'\r' | b'\n' => break,
                3 | 4 => {
                    // Ctrl-C or Ctrl-D: skip
                    key.clear();
                    break;
                }
                0x7f | 0x08 => {
                    key.pop();
                }
                0x1b => {
                    // An escape sequence, an arrow key say: skipped whole.
                    if let Ok(1) = input.read(&mut byte) {
                        if byte[0] == b'[' {
                            while let Ok(1) = input.read(&mut byte) {
                                if (0x40..=0x7e).contains(&byte[0]) {
                                    break;
                                }
                            }
                        }
                    }
                }
                b if b < 0x20 => {}
                b => key.push(b),
            }
        }
        Ok(String::from_utf8_lossy(&key).trim().to_string())
    })?;
    write!(out, "\r\n")?;
    if key.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(repo.join(".harness"))?;
    File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(repo.join(KEY_FILE))?
        .write_all(format!("{key}\n").as_bytes())?;
    write!(out, "init: TypeSafe key kept in {KEY_FILE}\r\n")?;
    Ok(())
}

/// The TypeSafe key for a Judgment: TYPESAFE_API_KEY when set, else the key
/// init kept in the Target repo, else None.
pub(crate) fn typesafe_key(repo: &Path, env: &dyn Fn(&str) -> String) -> Option<String> {
    let from_env = env("TYPESAFE_API_KEY");
    let key = if from_env.trim().is_empty() {
        fs::read_to_string(repo.join(KEY_FILE)).unwrap_or_default()
    } else {
        from_env
    };
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_string())
}

/// Runs `f` with the terminal in raw mode when stdin is one, so keys arrive
/// one at a time and nothing typed is echoed.
fn raw<T>(tty: bool, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let raw = tty && crossterm::terminal::enable_raw_mode().is_ok();
    let result = f();
    if raw {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    result
}

/// Puts a `choose` menu to the answering stdin.
fn menu(
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
    options: &[&str],
) -> io::Result<usize> {
    raw(tty, || choose(out, input, options))
}

/// Asks what to do with the shipped create-pr when the Target repo already has
/// one, and returns the name to install it under: "" keeps the repo's own and
/// installs nothing. A silent stdin keeps the repo's own, so a
/// non-interactive init never overwrites it.
fn ask_about_create_pr(
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<&'static str> {
    write!(
        out,
        "init: this repo already has a create-pr skill, and the Harness ships its own.\r\n"
    )?;
    let choice = menu(
        out,
        input,
        tty,
        &[
            "keep this repo's, install nothing",
            "replace it with the shipped one",
            "install the shipped one beside it, as harness-create-pr",
        ],
    )?;
    Ok(["", "create-pr", "harness-create-pr"][choice])
}

/// Draws a menu, moves the selection on the arrow keys (or j/k), and returns
/// the index the user submits with enter. A digit picks its option outright.
/// Anything else, including a closed stdin, leaves the first option.
fn choose(out: &mut dyn Write, input: &mut dyn Read, options: &[&str]) -> io::Result<usize> {
    let mut sel: usize = 0;
    let draw = |out: &mut dyn Write, sel: usize| -> io::Result<()> {
        for (i, option) in options.iter().enumerate() {
            // Reverse video, so the selected line reads at a glance.
            let marker = if i == sel { "\x1b[7m>" } else { "  " };
            write!(out, "{marker} {option}\x1b[0m\x1b[K\r\n")?;
        }
        write!(out, "  ↑/↓ to move, enter to choose\x1b[K\r")?;
        out.flush()
    };
    draw(out, sel)?;
    let mut key = [0u8; 3];
    loop {
        let n = match input.read(&mut key) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let arrow = |code: u8| n >= 3 && key[0] == 0x1b && key[1] == b'[' && key[2] == code;
        match key[0] {
            _ if arrow(b'A') => sel = sel.saturating_sub(1),
            b'k' => sel = sel.saturating_sub(1),
            _ if arrow(b'B') => sel += 1,
            b'j' => sel += 1,
            b'\r' | b'\n' => return done(out, options, sel),
            digit if digit > b'0' && usize::from(digit - b'0') <= options.len() => {
                return done(out, options, usize::from(digit - b'0') - 1)
            }
            // Ctrl-C in raw mode: take the safe option.
            3 | b'q' => return done(out, options, 0),
            _ => {}
        }
        sel = sel.min(options.len() - 1);
        write!(out, "\x1b[{}A", options.len())?; // back over the menu and redraw it
        draw(out, sel)?;
    }
    done(out, options, sel)
}

fn done(out: &mut dyn Write, options: &[&str], sel: usize) -> io::Result<usize> {
    write!(out, "\x1b[K\r\ninit: {}\r\n", options[sel])?;
    out.flush()?;
    Ok(sel)
}

/// Adds .harness/ to the Target repo's .gitignore once.
pub(crate) fn ignore_run_dir(repo: &Path) -> io::Result<()> {
    let path = repo.join(".gitignore");
    let mut existing = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    if existing.lines().any(|line| line.trim() == ".harness/") {
        return Ok(());
    }
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(".harness/\n");
    fs::write(path, existing)
}

/// Returns one specific message per missing prerequisite.
pub(crate) fn preflight(
    repo: &Path,
    tools: &dyn Tools,
    env: &dyn Fn(&str) -> String,
) -> Vec<String> {
    let mut missing = Vec::new();
    if !repo.join(".beads").exists() {
        missing.push("no bd workspace here: run 'bd init'".to_string());
    }
    if tools.run(repo, &["gh", "auth", "status"]).is_err() {
        missing.push("gh is not authenticated: run 'gh auth login'".to_string());
    }
    if !tools
        .run(repo, &["git", "remote"])
        .is_ok_and(|remotes| !remotes.trim().is_empty())
    {
        missing.push("no git remote: add one with 'git remote add origin <url>'".to_string());
    }
    if !has_skill(repo, "create-pr") {
        missing.push("no create-pr skill in .agents/skills or .claude/skills: run 'harness init' to install the shipped one".to_string());
    }
    if env("HERDR_ENV") != "1" {
        missing.push("HERDR_ENV is not 1: run the Harness from a pane inside herdr".to_string());
    }
    missing
}

fn has_skill(repo: &Path, name: &str) -> bool {
    [".agents/skills", ".claude/skills"]
        .iter()
        .any(|root| repo.join(root).join(name).join("SKILL.md").exists())
}

/// Prints each missing prerequisite and returns the exit code.
pub(crate) fn report_missing(out: &mut dyn Write, missing: &[String]) -> i32 {
    for m in missing {
        let _ = writeln!(out, "preflight: {m}");
    }
    if missing.is_empty() {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod setup_test;
