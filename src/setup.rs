//! The thin 'harness init': installs the shipped skills into a Target repo
//! and preflights it.

use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::fs::symlink;
use std::path::Path;

use crate::skills::SKILLS;
use crate::tools::Tools;

/// Writes the Harness's skills to <repo>/.agents/skills and links them from
/// <repo>/.claude/skills. An existing skill file is the Target repo's own and
/// is left alone unless force. A Target repo that already has a create-pr
/// skill of its own is asked what to do with the shipped one, since the Fix
/// Stage runs whichever /create-pr the repo ends up with. `input` answers
/// that question; None is the terminal's stdin, put in raw mode for the menu.
pub(crate) fn install_skills(
    repo: &Path,
    force: bool,
    out: &mut dyn Write,
    input: Option<&mut dyn Read>,
) -> io::Result<()> {
    // The name the shipped create-pr is installed under; "" keeps the repo's own.
    let mut pr = "create-pr";
    if !force && has_skill(repo, "create-pr") {
        pr = ask_about_create_pr(out, input)?;
    }
    for &(skill, body) in SKILLS {
        let (mut name, mut overwrite, mut renamed) = (skill, force, false);
        if skill == "create-pr" {
            if pr.is_empty() {
                continue;
            }
            (name, overwrite, renamed) = (pr, true, pr != skill);
        }
        let dest = repo.join(".agents/skills").join(name).join("SKILL.md");
        if fs::symlink_metadata(&dest).is_ok() && !overwrite {
            continue;
        }
        let body = if renamed {
            // Installed beside the repo's own, so it needs its own name in the text.
            body.replace("create-pr", name)
        } else {
            body.to_string()
        };
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::write(&dest, body)?;
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
    ignore_run_dir(repo)
}

/// Asks what to do with the shipped create-pr when the Target repo already has
/// one, and returns the name to install it under: "" keeps the repo's own and
/// installs nothing. A stdin that is not the terminal, or is closed, keeps the
/// repo's own, so a non-interactive init never overwrites it.
fn ask_about_create_pr(
    out: &mut dyn Write,
    input: Option<&mut dyn Read>,
) -> io::Result<&'static str> {
    write!(
        out,
        "init: this repo already has a create-pr skill, and the Harness ships its own.\r\n"
    )?;
    let mut stdin = io::stdin();
    let (input, tty): (&mut dyn Read, bool) = match input {
        Some(scripted) => (scripted, false),
        None => (&mut stdin, io::stdin().is_terminal()),
    };
    let raw = tty && crossterm::terminal::enable_raw_mode().is_ok();
    let choice = choose(
        out,
        input,
        &[
            "keep this repo's, install nothing",
            "replace it with the shipped one",
            "install the shipped one beside it, as harness-create-pr",
        ],
    );
    if raw {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    Ok(["", "create-pr", "harness-create-pr"][choice?])
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
