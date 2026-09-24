//! The thin 'harness init': installs the shipped skills and every job's
//! default where the user says, offers bd init, the docs/agents setup and
//! herdr's integrations, keeps TypeSafe on or off and its key, and
//! preflights the Target repo.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::iter;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::orchestrator::app::{self, APPS};
use crate::skills::manifest::{self, Installed, Location, Manifest, Place, JOBS, NONE};
use crate::skills::SKILLS;
use crate::tools::Tools;

/// The record of every skill file init wrote, path to the text it wrote:
/// under refresh, a file that still matches is unedited and is rewritten.
const RECORD: &str = ".harness/installed-skills.json";
const KEY_FILE: &str = ".harness/typesafe-key";

#[derive(Clone, Copy)]
enum Mode {
    Fresh,
    Refresh,
    Overwrite,
}

/// Asks where the skills go (Location), the current place the default, and
/// moves the ones the Harness installed when the answer changes, but those
/// the repo has committed (`tools` asks git). Then writes
/// the Harness's skills there. When the shipped skills are already installed
/// the gate asks first: cancel (false: nothing more touched), refresh only
/// the files unedited since install (by the record), or overwrite
/// everything; force is overwrite unasked, where they are. A Shipped skill
/// the repo already has in .agents/skills, committed say, is written there,
/// whatever the Location. A Target repo that already has a create-pr skill
/// of its own is asked what to do with the shipped one, since the Fix Stage
/// runs whichever /create-pr the repo ends up with; later inits keep that
/// answer from the record. `input` answers the questions, in raw mode when
/// `tty`.
pub(crate) fn install_skills(
    repo: &Path,
    home: &Path,
    tools: &dyn Tools,
    force: bool,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<bool> {
    let mut record: BTreeMap<String, String> = fs::read_to_string(repo.join(RECORD))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let mut manifest = Manifest::load(repo).map_err(io::Error::other)?;
    // What an older init recorded only in its record is the Harness's too.
    for name in record.keys().filter_map(|key| {
        key.strip_prefix(".agents/skills/")?
            .strip_suffix("/SKILL.md")
    }) {
        manifest.skills.entry(name.to_string()).or_insert(shipped());
    }
    let current = manifest
        .location
        .unwrap_or(if installed(&repo.join(".agents/skills")) {
            Location::Repo // installed before init asked, or committed
        } else {
            Location::Checkout
        });
    manifest.location = Some(current);
    let location = if force {
        current
    } else {
        ask_location(out, &mut *input, tty, current)?
    };
    // Moving from user level needs HOME too, to find the skills it moves.
    if [current, location].contains(&Location::User) && home.as_os_str().is_empty() {
        return Err(io::Error::other(
            "no HOME, so no user level to put the skills in or move them from",
        ));
    }
    if location != current {
        for stayed in manifest.relocate(repo, home, tools, location) {
            write!(out, "init: {stayed}\r\n")?;
        }
        manifest.save(repo).map_err(io::Error::other)?;
    }
    let place = manifest.place(repo, home);
    let mode = if force {
        Mode::Overwrite
    } else if installed(&repo.join(".agents/skills")) || installed(&place.root.join(place.files)) {
        write!(
            out,
            "init: the shipped skills are already installed here.\r\n"
        )?;
        let options = [
            "cancel, leave them as they are",
            "refresh only the skills not edited since install",
            "overwrite everything with the shipped skills",
        ];
        match raw(tty, || choose(out, &mut *input, &options, 0))? {
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
        .find(|name| record.contains_key(&record_key(name)));
    let pr = match (recorded, mode) {
        (Some(name), _) => name,
        (None, _) if !has_skill(repo, &place, "create-pr") => "create-pr",
        (None, Mode::Fresh) => ask_about_create_pr(out, input, tty)?,
        (None, _) => "", // the repo's own, kept on the first init
    };
    // (shipped name, installed name, body); the repo's own create-pr is left out.
    let skills: Vec<(&str, &str, &str)> = SKILLS
        .iter()
        .filter(|&&(skill, _)| skill != "create-pr" || !pr.is_empty())
        .map(|&(skill, body)| (skill, if skill == "create-pr" { pr } else { skill }, body))
        .collect();
    let committed = Location::Repo.place(repo, home);
    for &(skill, name, body) in &skills {
        let rel = record_key(name);
        let at = if fs::symlink_metadata(committed.skill(name)).is_ok() {
            &committed
        } else {
            &place
        };
        let dest = at.skill(name).join("SKILL.md");
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
                .is_some_and(|wrote| fs::read_to_string(&dest).is_ok_and(|now| now == *wrote)),
            Mode::Fresh => skill == "create-pr" || existing.is_none(),
        };
        if write {
            let body = if name != skill {
                // Installed beside the repo's own, so it needs its own name in the text.
                body.replace("create-pr", name)
            } else {
                body.to_string()
            };
            fs::create_dir_all(dest.parent().unwrap())?;
            fs::write(&dest, &body)?;
            record.insert(rel, body);
            manifest.skills.insert(name.to_string(), shipped());
        }
        if let Some(link) = at.link(name).filter(|link| {
            name == pr
                && fs::symlink_metadata(link).is_ok_and(|meta| !meta.file_type().is_symlink())
        }) {
            writeln!(
                out,
                "init: {} is its own copy, not a link: remove it to use the shipped skill",
                link.display()
            )?;
        }
        manifest::link(at, name)?;
    }
    fs::create_dir_all(repo.join(".harness"))?;
    fs::write(
        repo.join(RECORD),
        serde_json::to_string_pretty(&record)? + "\n",
    )?;
    manifest.save(repo).map_err(io::Error::other)?;
    ignore_run_dir(repo)?;
    Ok(true)
}

/// A Shipped skill's manifest entry.
fn shipped() -> Installed {
    Installed {
        shipped: true,
        ..Installed::default()
    }
}

/// The record's key for a skill init wrote: its path in the repo's
/// .agents/skills, as the record has always named it, wherever it is now.
fn record_key(name: &str) -> String {
    format!(".agents/skills/{name}/SKILL.md")
}

/// Whether a shipped skill is already installed in dir. create-pr does not
/// count: a repo's own is not an install, and it has its own question.
// ponytail: a repo that deleted every Stage skill but kept a shipped create-pr
// reads as fresh; the record would tell, but nobody has done that.
fn installed(dir: &Path) -> bool {
    SKILLS
        .iter()
        .any(|&(name, _)| name != "create-pr" && dir.join(name).join("SKILL.md").exists())
}

/// Asks where the skills go, the current Location first selected: a silent
/// stdin, Ctrl-C or q keep it.
fn ask_location(
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
    current: Location,
) -> io::Result<Location> {
    write!(out, "init: where should the skills go?\r\n")?;
    let options = [
        "this checkout, uncommitted: .harness/skills, linked into each Ticket's worktree",
        "the repo, committed: .agents/skills, linked from .claude/skills; you commit them",
        "user level: ~/.agents/skills, linked from ~/.claude/skills",
    ];
    let locations = [Location::Checkout, Location::Repo, Location::User];
    let default = locations.iter().position(|l| *l == current).unwrap();
    Ok(locations[raw(tty, || choose(out, input, &options, default))?])
}

/// The rest of init once the skills are in: bd, the docs/agents setup,
/// TypeSafe, every job's default, and herdr's integrations.
pub(crate) fn set_up(
    repo: &Path,
    home: &Path,
    tools: &dyn Tools,
    env_key: &str,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    bd_init(repo, tools, out, input, tty)?;
    write_agent_docs(repo, out, input, tty)?;
    let typesafe = ask_typesafe(repo, env_key, out, input, tty)?;
    install_defaults(repo, home, tools, typesafe, out)?;
    install_integrations(repo, tools, out, input, tty)
}

/// Offers herdr's integration, which reports each session's id, for every
/// App on PATH whose integration herdr says is not installed or outdated:
/// what each install writes, then one question. Declined, or nobody
/// answering, those Stages cannot be resumed by id.
fn install_integrations(
    repo: &Path,
    tools: &dyn Tools,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    // herdr 0.9.1 prints it as text only: "codex: outdated (v7) (<path>)".
    let status = tools
        .run(repo, &["herdr", "integration", "status"])
        .unwrap_or_default();
    let stale: Vec<(&str, &str)> = APPS
        .iter()
        .filter_map(|app| {
            let state = status
                .lines()
                .find_map(|line| line.strip_prefix(app.name)?.strip_prefix(": "))?;
            if !state.starts_with("not installed") && !state.starts_with("outdated") {
                return None;
            }
            // The path, the last parenthesis: a version one may come first.
            let writes = state.find(" (/").map_or("", |at| {
                let path = state[at + 2..].trim_end();
                path.strip_suffix(')').unwrap_or(path)
            });
            tools.run(repo, &["which", app.name]).ok()?;
            Some((app.name, writes))
        })
        .collect();
    if stale.is_empty() {
        return Ok(());
    }
    write!(
        out,
        "init: herdr's integration tells herdr each session's id, so /continue can resume a Stage:\r\n"
    )?;
    for (name, writes) in &stale {
        match *writes {
            "" => write!(out, "  {name}\r\n")?,
            _ => write!(
                out,
                "  {name}: writes {writes}, and registers it as a hook in {name}'s settings\r\n"
            )?,
        }
    }
    if yes(out, input, tty, "Install herdr's integration for these?")? == Some(true) {
        for (name, _) in &stale {
            match tools.run(repo, &["herdr", "integration", "install", name]) {
                Ok(_) => write!(out, "init: installed herdr's {name} integration\r\n")?,
                Err(err) => write!(
                    out,
                    "init: herdr's {name} integration not installed: {err}\r\n"
                )?,
            }
        }
    } else {
        let names: Vec<&str> = stale.iter().map(|(name, _)| *name).collect();
        write!(
            out,
            "init: skipped: Stages on {} cannot be resumed by id, so /continue starts them fresh\r\n",
            names.join(", ")
        )?;
    }
    Ok(())
}

/// With no bd workspace, offers to run bd init. Nobody answering skips it,
/// and the preflight says it is missing.
fn bd_init(
    repo: &Path,
    tools: &dyn Tools,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    if repo.join(".beads").exists()
        || yes(out, input, tty, "No bd workspace here. Run bd init now?")? != Some(true)
    {
        return Ok(());
    }
    match tools.run(repo, &["bd", "init", "--non-interactive"]) {
        Ok(_) => write!(out, "init: bd init done\r\n"),
        Err(err) => write!(out, "init: bd init failed: {err}\r\n"),
    }
}

/// The beads docs/agents setup init writes, compiled in: this repo's own.
const AGENT_DOCS: [(&str, &str); 3] = [
    (
        "docs/agents/issue-tracker.md",
        include_str!("../docs/agents/issue-tracker.md"),
    ),
    (
        "docs/agents/triage-labels.md",
        include_str!("../docs/agents/triage-labels.md"),
    ),
    (
        "docs/agents/domain.md",
        include_str!("../docs/agents/domain.md"),
    ),
];

/// The Agent skills block, modelled on this repo's AGENTS.md.
const AGENT_SKILLS: &str = "## Agent skills

### Issue tracker

Issues live in beads (`bd`), not GitHub Issues. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default triage roles, each a bd label of the same name. See `docs/agents/triage-labels.md`.

### Domain docs

`CONTEXT.md` and `docs/adr/` at the root. See `docs/agents/domain.md`.
";

/// Writes what the repo lacks of the beads docs/agents setup, asked first;
/// a non-interactive init writes it too. The Agent skills block goes into
/// CLAUDE.md, else AGENTS.md, unless either has it already.
fn write_agent_docs(
    repo: &Path,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    let docs: Vec<_> = AGENT_DOCS
        .iter()
        .filter(|(path, _)| !repo.join(path).exists())
        .collect();
    let has_block = ["CLAUDE.md", "AGENTS.md"].iter().any(|file| {
        fs::read_to_string(repo.join(file)).is_ok_and(|text| text.contains("## Agent skills"))
    });
    if docs.is_empty() && has_block {
        return Ok(());
    }
    let question = "Write the beads docs/agents setup (issue tracker, triage labels, domain, Agent skills block)?";
    if yes(out, input, tty, question)? == Some(false) {
        return Ok(());
    }
    for (path, text) in docs {
        let at = repo.join(path);
        fs::create_dir_all(at.parent().unwrap())?;
        fs::write(&at, text)?;
        write!(out, "init: wrote {path}\r\n")?;
    }
    if !has_block {
        let file = if repo.join("CLAUDE.md").exists() {
            "CLAUDE.md"
        } else {
            "AGENTS.md"
        };
        let mut text = match fs::read_to_string(repo.join(file)) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err),
        };
        if !text.is_empty() {
            text += if text.ends_with('\n') { "\n" } else { "\n\n" };
        }
        fs::write(repo.join(file), text + AGENT_SKILLS)?;
        write!(out, "init: wrote the Agent skills block into {file}\r\n")?;
    }
    Ok(())
}

/// The typesafe-ai skill init installs when TypeSafe is on: (name, source).
pub(crate) const TYPESAFE_SKILL: (&str, &str) =
    ("typesafe-ai", "typesafe-ai/skills/skills/typesafe-ai");

/// Installs every job's default the manifest lacks, and the typesafe-ai
/// skill when TypeSafe is on, where init put the skills, each pinned by its
/// commit. At user level a skill you already have there is yours and stays,
/// linked for Claude should it lack the link. A failure is said and init
/// goes on: the preflight names the job.
pub(crate) fn install_defaults(
    repo: &Path,
    home: &Path,
    tools: &dyn Tools,
    typesafe: bool,
    out: &mut dyn Write,
) -> io::Result<()> {
    let manifest = Manifest::load(repo).map_err(io::Error::other)?;
    let place = manifest.place(repo, home);
    let defaults = JOBS.iter().map(|(job, suggestions)| {
        (
            suggestions[0],
            format!("the {} default", job.replace('-', " ")),
        )
    });
    let typesafe = typesafe.then(|| (TYPESAFE_SKILL, "for TypeSafe".to_string()));
    for ((name, source), what) in defaults.chain(typesafe) {
        if source.is_empty() || manifest.skills.contains_key(name) {
            continue;
        }
        if manifest.location == Some(Location::User)
            && iter::once(place.skill(name))
                .chain(place.link(name))
                .any(|path| fs::symlink_metadata(path).is_ok())
        {
            writeln!(out, "init: keeping your {name} at user level")?;
            if let Err(err) = manifest::link(&place, name) {
                writeln!(out, "init: your {name} not linked for Claude: {err}")?;
            }
            continue;
        }
        match manifest::add(repo, home, tools, source, Some(name)) {
            Ok(_) => writeln!(out, "init: installed {name}, {what}")?,
            Err(err) => writeln!(out, "init: {name} not installed: {err}")?,
        }
    }
    Ok(())
}

/// TypeSafe's opt-in, kept on or off in config.json. TYPESAFE_API_KEY
/// (`env_key`) set is on, unasked. Otherwise yes takes the key stored, or
/// asks for one; no, or an empty key, is off. Nobody answering (a
/// non-interactive init) leaves it on only where it is on with a key kept.
/// The key stays in its own file.
pub(crate) fn ask_typesafe(
    repo: &Path,
    env_key: &str,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<bool> {
    let question = "Use TypeSafe to judge plans, Wakes and disputed Findings?";
    let kept = repo.join(KEY_FILE).exists();
    let on = if !env_key.trim().is_empty() {
        true
    } else {
        match yes(out, input, tty, question)? {
            Some(true) => kept || ask_typesafe_key(repo, out, input, tty)?,
            Some(false) => false,
            None => kept && app::typesafe(repo),
        }
    };
    app::set_typesafe(repo, on).map_err(io::Error::other)?;
    write!(out, "init: TypeSafe {}\r\n", if on { "on" } else { "off" })?;
    Ok(on)
}

/// Asks for the TypeSafe API key with echo off and keeps it, readable only
/// by the user: whether one was kept. An empty answer, Ctrl-C, Ctrl-D or a
/// silent stdin keeps none.
fn ask_typesafe_key(
    repo: &Path,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<bool> {
    write!(
        out,
        "init: TypeSafe API key, kept in {KEY_FILE} (enter to skip): "
    )?;
    out.flush()?;
    let key = match raw(tty, || read_line(out, input, false))? {
        Line::Text(key) => key,
        Line::Cancel | Line::End => String::new(),
    };
    write!(out, "\r\n")?;
    if key.is_empty() {
        return Ok(false);
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
    Ok(true)
}

/// Puts a [Y/n] question: enter or y is yes, Ctrl-C, Ctrl-D or any other
/// answer no. None when the input ends unanswered: a non-interactive init.
/// A line, not a key, so the enter after a y never answers the next one.
fn yes(
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
    question: &str,
) -> io::Result<Option<bool>> {
    write!(out, "init: {question} [Y/n] ")?;
    out.flush()?;
    let answer = raw(tty, || read_line(out, input, true))?;
    write!(out, "\r\n")?;
    Ok(match answer {
        Line::Text(a) => Some(a.is_empty() || a.starts_with(['y', 'Y'])),
        Line::Cancel => Some(false),
        Line::End => None,
    })
}

/// What read_line read.
enum Line {
    Text(String),
    /// Ctrl-C or Ctrl-D.
    Cancel,
    /// The input ended with nothing typed: nobody is there to answer.
    End,
}

/// Reads a line, echoing it when `echo`.
fn read_line(out: &mut dyn Write, input: &mut dyn Read, echo: bool) -> io::Result<Line> {
    let text = |line: &[u8]| Line::Text(String::from_utf8_lossy(line).trim().to_string());
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while let Ok(1) = input.read(&mut byte) {
        match byte[0] {
            b'\r' | b'\n' => return Ok(text(&line)),
            3 | 4 => return Ok(Line::Cancel),
            0x7f | 0x08 => {
                if line.pop().is_some() && echo {
                    write!(out, "\x08 \x08")?;
                }
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
            b => {
                line.push(b);
                if echo {
                    out.write_all(&[b])?;
                }
            }
        }
        out.flush()?;
    }
    Ok(if line.is_empty() {
        Line::End
    } else {
        text(&line)
    })
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
    let options = [
        "keep this repo's, install nothing",
        "replace it with the shipped one",
        "install the shipped one beside it, as harness-create-pr",
    ];
    let choice = raw(tty, || choose(out, input, &options, 0))?;
    Ok(["", "create-pr", "harness-create-pr"][choice])
}

/// Draws a menu, the default selected, moves the selection on the arrow keys
/// (or j/k), and returns the index the user submits with enter. A digit
/// picks its option outright. A closed stdin leaves the selection; Ctrl-C
/// and q take the default.
fn choose(
    out: &mut dyn Write,
    input: &mut dyn Read,
    options: &[&str],
    default: usize,
) -> io::Result<usize> {
    let mut sel = default;
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
            3 | b'q' => return done(out, options, default),
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
    add_lines(&repo.join(".gitignore"), &[".harness/".to_string()])
}

/// Appends each line the file lacks, making the file and its folder if need be.
pub(crate) fn add_lines(path: &Path, lines: &[String]) -> io::Result<()> {
    let mut existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    let missing: Vec<&String> = lines
        .iter()
        .filter(|want| !existing.lines().any(|line| line.trim() == want.as_str()))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    for line in missing {
        existing.push_str(line);
        existing.push('\n');
    }
    fs::create_dir_all(path.parent().unwrap())?;
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
    let home = PathBuf::from(env("HOME"));
    let have: Vec<String> = manifest::list(repo, &home, tools)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    if !have.iter().any(|name| name == "create-pr") {
        missing
            .push("no create-pr skill: run 'harness init' to install the shipped one".to_string());
    }
    // Each job's pick, but none and one built into its App.
    match Manifest::load(repo) {
        Ok(manifest) => {
            for (job, suggestions) in JOBS {
                let pick = manifest.pick(job);
                let built_in = suggestions
                    .iter()
                    .any(|&(name, source)| name == pick && source.is_empty());
                if pick != NONE && !built_in && !have.iter().any(|name| name == pick) {
                    // init installs only the default
                    let fix = if pick == suggestions[0].0 {
                        "harness init installs it, or /config picks another"
                    } else {
                        "/config installs it, or picks another"
                    };
                    missing.push(format!(
                        "the {} skill {pick} is missing: {fix}",
                        job.replace('-', " ")
                    ));
                }
            }
        }
        Err(err) => missing.push(err),
    }
    // Each App a row picks, looked for once; a row that cannot be read is
    // the Orchestrator's to refuse.
    let mut on_path = BTreeMap::new();
    for (key, said) in app::ROWS {
        let Ok(row) = app::row(repo, key) else {
            continue;
        };
        let name = row.app.name;
        if !*on_path
            .entry(name)
            .or_insert_with(|| tools.run(repo, &["which", name]).is_ok())
        {
            missing.push(format!("{said} runs on {name}, which is not on PATH"));
        }
    }
    if env("HERDR_ENV") != "1" {
        missing.push("HERDR_ENV is not 1: run the Harness from a pane inside herdr".to_string());
    }
    missing
}

/// What the preflight warns of without failing: a personal skill that shadows
/// one the Harness installed, since Claude Code runs a personal skill over a
/// project one of the same name, and the superpowers plugin.
pub(crate) fn warnings(
    repo: &Path,
    tools: &dyn Tools,
    env: &dyn Fn(&str) -> String,
) -> Vec<String> {
    let home = PathBuf::from(env("HOME"));
    let mut warn = Vec::new();
    // A garbled manifest is the preflight's to fail on.
    let manifest = Manifest::load(repo).unwrap_or_default();
    if !home.as_os_str().is_empty() && manifest.location != Some(Location::User) {
        for name in manifest.skills.keys() {
            if home
                .join(".claude/skills")
                .join(name)
                .join("SKILL.md")
                .exists()
            {
                warn.push(format!("your personal ~/.claude/skills/{name} shadows the installed {name}: Claude Code runs a personal skill over a project one"));
            }
        }
    }
    if manifest::plugins(repo, tools)
        .iter()
        .any(|(name, _)| name == "superpowers")
    {
        warn.push(
            "the superpowers Claude Code plugin is enabled: its SessionStart hook can stall Stages"
                .to_string(),
        );
    }
    warn
}

/// Whether the skill is in the repo's .agents/skills or .claude/skills, or at
/// the place init puts skills.
fn has_skill(repo: &Path, place: &Place, name: &str) -> bool {
    [
        repo.join(".agents/skills").join(name),
        repo.join(".claude/skills").join(name),
        place.skill(name),
    ]
    .into_iter()
    .chain(place.link(name))
    .any(|dir| dir.join("SKILL.md").exists())
}

/// Prints each missing prerequisite and returns the exit code.
pub(crate) fn report_missing(out: &mut dyn Write, missing: &[String]) -> i32 {
    for m in missing {
        let _ = writeln!(out, "preflight: {m}");
    }
    i32::from(!missing.is_empty())
}

#[cfg(test)]
mod setup_test;
