//! The App table, compiled in, and .harness/config.json: the App, model and
//! effort each Stage starts on.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::plan::quoted;
use super::stage::{Stage, DEBATE};
use super::trust::{claude_records, codex_records, copilot_records, cursor_records, pi_records};
use crate::tools::Tools;

/// The Review's fallback row, for when the Review's App is Limited; its
/// model starts as none, no fallback.
pub(crate) const IF_LIMITED: &str = "review_if_limited";

/// A model id and its effort levels, as an App lists them.
pub(crate) type Model = (String, Vec<String>);

/// An agent CLI a Stage can run on: one row of the App table. In the arg
/// forms, "{}" is the value put in.
pub(crate) struct App {
    /// Its name in config.json, and herdr's agent kind.
    pub(crate) name: &'static str,
    /// The binary herdr starts for that kind, found on PATH.
    pub(crate) bin: &'static str,
    /// The unattended args of the Review, in the Run directory, given the
    /// worktree.
    pub(crate) run_dir_args: fn(&str) -> Vec<String>,
    /// The unattended args of a Stage in the worktree, given the Run
    /// directory.
    pub(crate) worktree_args: fn(&str) -> Vec<String>,
    pub(crate) model: &'static [&'static str],
    pub(crate) effort: &'static [&'static str],
    pub(crate) resume: &'static [&'static str],
    /// The headless read-only command a Debate side and the audit run: the
    /// model and effort args go before a closing -p, which the brief
    /// follows, and after anything else. "{}" is the Run directory, where
    /// the diff is.
    pub(crate) side: &'static [&'static str],
    /// Where the App records the directories it trusts: Some(trusted) when
    /// dir is recorded.
    pub(crate) trust: fn(&Path, &Path) -> Option<bool>,
    /// Its models' family; "" for an App that runs several, told from each
    /// model's name (family_of).
    pub(crate) family: &'static str,
    /// Where to get it, for /config on an App not installed.
    pub(crate) home: &'static str,
    /// The models /config offers besides default, run in the given dir.
    pub(crate) models: fn(&dyn Tools, &Path) -> Result<Vec<Model>, String>,
    /// What its pane shows at a usage limit: regexes, with the reset in
    /// the group "reset" and which limit in "what" when the App says.
    pub(crate) limits: &'static [&'static str],
    /// What a Stage skill's "Use the {} skill" puts before a job's pick.
    pub(crate) mention: &'static str,
    /// The skills built into it: a pick of one needs nothing installed here,
    /// and is not installed on another App.
    pub(crate) built_in: &'static [&'static str],
    /// Its own skills folder, at the repo and at home; it loads
    /// .harness/skills too, where the checkout's skills are linked from.
    pub(crate) skill_dir: &'static str,
    /// Whether it loads its enabled plugins' skills, named plugin:skill.
    pub(crate) plugins: bool,
    /// Its row is from its docs, never run here: /config says so.
    pub(crate) experimental: bool,
}

/// pi's --thinking levels, the same on every model.
const PI_THINKING: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

impl App {
    /// Whether it loads a skill manifest::list found: the name list gives
    /// it, and the folder it is in.
    pub(crate) fn loads(&self, name: &str, dir: &Path) -> bool {
        (self.plugins && name.contains(':'))
            || dir.ends_with(self.skill_dir)
            || dir.ends_with(".harness/skills")
    }
}

pub(crate) static APPS: [App; 6] = [
    App {
        name: "claude",
        bin: "claude",
        // --add-dir lets it read the worktree but also write there: the Edit
        // deny rule stops the file tools and, merged into the strict Bash
        // sandbox, every command.
        run_dir_args: |worktree| {
            let settings = json!({
                "permissions": { "deny": [format!("Edit(/{worktree}/**)")] },
                "sandbox": {
                    "enabled": true,
                    "failIfUnavailable": true,
                    "allowUnsandboxedCommands": false,
                },
            });
            [
                "--permission-mode",
                "auto",
                "--add-dir",
                worktree,
                "--settings",
                &settings.to_string(),
            ]
            .map(String::from)
            .to_vec()
        },
        worktree_args: |run_dir| {
            ["--permission-mode", "auto", "--add-dir", run_dir]
                .map(String::from)
                .to_vec()
        },
        model: &["--model", "{}"],
        effort: &["--effort", "{}"],
        resume: &["--resume", "{}"],
        // Only the read-only tools, named, so a tool added later is out too:
        // a shell or other code-running tool runs unsandboxed here and can
        // write an ignored file or a path outside the worktree that the
        // Moderator's git guard cannot put back. The tool list takes every
        // arg up to the next flag: before -p it cannot take the brief as a
        // tool. It starts in the worktree: the Run directory, a sibling that
        // holds the diff, is granted on its own, as the Moderator's grant is
        // not passed on.
        side: &[
            "claude",
            "--tools",
            "Read,Grep,Glob,Skill",
            "--add-dir",
            "{}",
            "-p",
        ],
        trust: claude_records,
        family: "Anthropic",
        home: "https://claude.com/product/claude-code",
        models: |_, _| {
            let efforts = ["low", "medium", "high", "xhigh", "max"].map(String::from);
            Ok(ALIASES
                .map(|(alias, _)| (alias.to_string(), efforts.to_vec()))
                .to_vec())
        },
        limits: &[
            r"You['’]ve hit your (?P<what>.*?limit) · resets (?P<reset>.+)",
            r"Usage limit reached · continuing automatically at (?P<reset>.+?)(?: · |$)",
        ],
        // In words, a plugin's skill plugin-qualified as the pick names it.
        mention: "",
        built_in: &[],
        skill_dir: ".claude/skills",
        plugins: true,
        experimental: false,
    },
    App {
        name: "codex",
        bin: "codex",
        // The sandbox writes only where the pane starts: the result file
        // there, nothing in the worktree.
        run_dir_args: |_| ["--sandbox", "workspace-write"].map(String::from).to_vec(),
        // The sandbox writes the worktree, where the pane starts, and the
        // Run directory, for the plan and the result.
        // ponytail: Git metadata stays read-only, so Implement's commit asks
        // you to let it out of the sandbox (a blocked session); a Git write
        // path that keeps .git's hooks and config out of reach when that
        // asks too often.
        worktree_args: |run_dir| {
            ["--sandbox", "workspace-write", "--add-dir", run_dir]
                .map(String::from)
                .to_vec()
        },
        model: &["-m", "{}"],
        effort: &["-c", "model_reasoning_effort={}"],
        resume: &["resume", "{}"],
        side: &["codex", "exec", "--sandbox", "read-only"],
        trust: codex_records,
        family: "OpenAI",
        home: "https://developers.openai.com/codex",
        models: codex_models,
        // U+2019 in You’ve; "Try again later." gives no reset
        limits: &[
            r"You['’]ve hit your (?P<what>usage limit)\..*?[Tt]ry again (?:at (?P<reset>.+?)|later)\.",
        ],
        // $name, which a skill with implicit invocation off (review-agent)
        // needs.
        mention: "$",
        built_in: &["review-agent"],
        // Never Claude's .claude/skills or its plugins.
        skill_dir: ".agents/skills",
        plugins: false,
        experimental: false,
    },
    // The four below are from docs/research/agent-clis.md (research/agent-clis
    // branch): none was run here. Each names its skills in words, loads
    // .agents/skills and has none built in.
    App {
        name: "pi",
        bin: "pi",
        // No permission prompts and no sandbox, by design: the Review's
        // guard is all that keeps the worktree as it was.
        run_dir_args: |_| Vec::new(),
        worktree_args: |_| Vec::new(),
        model: &["--model", "{}"],
        effort: &["--thinking", "{}"],
        resume: &["--session", "{}"],
        side: &["pi", "--tools", "read,grep,find,ls", "-p"],
        trust: pi_records,
        family: "",
        home: "https://pi.dev",
        models: pi_models,
        limits: &[
            r"You have hit your (?P<what>.*?usage limit)(?: \([^)]*\))?\. Try again (?P<reset>in ~?\d+ min)",
        ],
        mention: "",
        built_in: &[],
        skill_dir: ".agents/skills",
        plugins: false,
        experimental: true,
    },
    App {
        name: "opencode",
        bin: "opencode",
        // --auto answers every ask, a directory outside the start one's too.
        // No sandbox.
        run_dir_args: |_| vec!["--auto".to_string()],
        worktree_args: |_| vec!["--auto".to_string()],
        model: &["-m", "{}"],
        // Its TUI takes an effort only through config.
        effort: &[],
        resume: &["--session", "{}"],
        // A quoted VAR=value is no assignment to the shell: env sets it. The
        // diff is in the Run directory, outside the worktree it starts in.
        side: &[
            "env",
            r#"OPENCODE_PERMISSION={"edit":"deny","bash":"deny","external_directory":"allow"}"#,
            "opencode",
            "run",
        ],
        trust: |_, _| Some(true), // no trust dialog
        family: "",
        home: "https://opencode.ai",
        models: opencode_models,
        // It waits out a retry-after inside its turn, looking working, with
        // no bound of its own: the check before a timeout Wake reads it, and
        // on a Debate side the Moderator's timeout. Seconds alone are no
        // limit.
        limits: &[
            r"(?P<what>[\w-]+ usage limit) reached\. It will reset (?P<reset>in [^.]+)\.",
            r"\[retrying (?P<reset>in [^\]]+?) attempt #\d+\]",
        ],
        mention: "",
        built_in: &[],
        skill_dir: ".agents/skills",
        plugins: false,
        experimental: true,
    },
    App {
        name: "copilot",
        bin: "copilot",
        run_dir_args: |worktree| {
            ["--allow-all", "--add-dir", worktree]
                .map(String::from)
                .to_vec()
        },
        worktree_args: |run_dir| {
            ["--allow-all", "--add-dir", run_dir]
                .map(String::from)
                .to_vec()
        },
        model: &["--model", "{}"],
        effort: &["--effort", "{}"],
        resume: &["--resume={}"],
        side: &[
            "copilot",
            "--deny-tool=write",
            "--deny-tool=shell",
            "--add-dir",
            "{}",
            "-p",
        ],
        trust: copilot_records,
        family: "",
        home: "https://github.com/features/copilot/cli",
        // No listing but in a live session: default and 'type an id…'.
        models: |_, _| Ok(Vec::new()),
        // Its monthly credits reset at 00:00 UTC on the 1st.
        limits: &[
            r"(?:You['’]ve (?:hit|reached) (?:your|the) (?P<what>(?:\w+ )?rate limit).*?)?Please wait for your limit to reset (?P<reset>in \d+ minutes?|on .+?) or switch",
            r"You['’]ve run out of your included (?P<what>AI credits) (?P<reset>for the month)",
        ],
        mention: "",
        built_in: &[],
        skill_dir: ".agents/skills",
        plugins: false,
        experimental: true,
    },
    App {
        name: "cursor",
        // herdr starts and resumes cursor-agent, not agent.
        bin: "cursor-agent",
        run_dir_args: |worktree| {
            ["--force", "--add-dir", worktree]
                .map(String::from)
                .to_vec()
        },
        worktree_args: |run_dir| ["--force", "--add-dir", run_dir].map(String::from).to_vec(),
        model: &["--model", "{}"],
        // No effort flag: a typed slug[effort=high] model carries one.
        effort: &[],
        resume: &["--resume", "{}"],
        // Headless in an untrusted folder it exits 1, and no trust wait
        // covers a side: --trust.
        side: &[
            "cursor-agent",
            "--mode",
            "ask",
            "--trust",
            "--add-dir",
            "{}",
            "-p",
        ],
        trust: cursor_records,
        family: "",
        home: "https://cursor.com/cli",
        models: cursor_models,
        // The server's text, seen only second hand; no reset in it.
        limits: &[r"You['’]re out of usage"],
        mention: "",
        built_in: &[],
        skill_dir: ".agents/skills",
        plugins: false,
        experimental: true,
    },
];

/// codex's catalog: the models it lists for picking, each with its reasoning
/// levels; --bundled skips the refresh.
fn codex_models(tools: &dyn Tools, dir: &Path) -> Result<Vec<Model>, String> {
    let out = tools
        .run(dir, &["codex", "debug", "models", "--bundled"])
        .map_err(|err| err.to_string())?;
    let doc: Value =
        serde_json::from_str(&out).map_err(|err| format!("codex debug models: {err}"))?;
    Ok(doc["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"] == "list")
        .filter_map(|m| {
            let slug = m["slug"].as_str()?.to_string();
            let efforts = m["supported_reasoning_levels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|level| level["effort"].as_str().map(String::from))
                .collect();
            Some((slug, efforts))
        })
        .collect())
}

// ponytail: the three listings below read shapes no one here has seen
// printed; a shape they miss lists nothing, and 'type an id…' still works.

/// pi's models with working auth: a table under a "provider model …"
/// header, each picked as provider/model, with pi's thinking levels.
fn pi_models(tools: &dyn Tools, dir: &Path) -> Result<Vec<Model>, String> {
    let out = tools
        .run(dir, &["pi", "--list-models"])
        .map_err(|err| err.to_string())?;
    Ok(out
        .lines()
        .skip_while(|line| !line.trim_start().starts_with("provider"))
        .skip(1)
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let id = format!("{}/{}", cols.next()?, cols.next()?);
            Some((id, PI_THINKING.map(String::from).to_vec()))
        })
        .collect())
}

/// opencode's models, one provider/model a line; no effort in a pane.
fn opencode_models(tools: &dyn Tools, dir: &Path) -> Result<Vec<Model>, String> {
    let out = tools
        .run(dir, &["opencode", "models"])
        .map_err(|err| err.to_string())?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|id| id.contains('/') && !id.contains(' '))
        .map(|id| (id.to_string(), Vec::new()))
        .collect())
}

/// cursor's models, "<id> - <name>" a line; its efforts are not listed.
fn cursor_models(tools: &dyn Tools, dir: &Path) -> Result<Vec<Model>, String> {
    let out = tools
        .run(dir, &["cursor-agent", "models"])
        .map_err(|err| err.to_string())?;
    Ok(out
        .lines()
        .filter_map(|line| Some(line.split_once(" - ")?.0.trim()))
        .filter(|id| !id.is_empty() && !id.contains(' '))
        .map(|id| (id.to_string(), Vec::new()))
        .collect())
}

/// The App config.json names.
pub(crate) fn app(name: &str) -> Option<&'static App> {
    APPS.iter().find(|a| a.name == name)
}

/// One Stage's row: its App, model and effort; "default" passes no flag.
pub(crate) struct Row {
    pub(crate) app: &'static App,
    pub(crate) model: String,
    pub(crate) effort: String,
    /// Implement's plan model on a split, one other than its model: the
    /// session runs opusplan with the halves remapped (plan_settings).
    pub(crate) plan_model: Option<String>,
}

impl Row {
    /// The model and effort args of a pane and a Debate side.
    pub(crate) fn flags(&self) -> Vec<String> {
        let model = match self.plan_model {
            Some(_) => "opusplan",
            None => &self.model,
        };
        let mut out = Vec::new();
        for (form, value) in [(self.app.model, model), (self.app.effort, &self.effort)] {
            if value != "default" {
                out.extend(fill(form, value));
            }
        }
        out
    }

    /// The args that resume session `id`, ahead of the Stage's own.
    pub(crate) fn resume(&self, id: &str) -> Vec<String> {
        fill(self.app.resume, id)
    }

    /// The headless read-only command, as one shell line: every arg single
    /// quoted, since a model id like claude-opus-5-5[1m] is a glob to the
    /// shell.
    pub(crate) fn side_command(&self, run_dir: &str) -> String {
        side_argv(self.app.side, run_dir, self.flags())
            .into_iter()
            .map(|arg| quoted(&arg))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// As the started line names it: "claude", "claude opus/high", on a
    /// split "claude claude-fable-5-1→claude-opus-5-5/high".
    pub(crate) fn said(&self) -> String {
        let model = match &self.plan_model {
            Some(plan) => format!("{plan}→{}", self.model),
            None => self.model.clone(),
        };
        match (model.as_str(), self.effort.as_str()) {
            ("default", "default") => self.app.name.to_string(),
            (model, "default") => format!("{} {model}", self.app.name),
            (model, effort) => format!("{} {model}/{effort}", self.app.name),
        }
    }
}

/// An arg form with value put in for "{}".
fn fill(form: &[&str], value: &str) -> Vec<String> {
    form.iter().map(|arg| arg.replace("{}", value)).collect()
}

/// A side command with the Run directory put in and `args` before its
/// closing -p, so the brief put last follows it: copilot's -p takes the
/// brief as its value.
fn side_argv(form: &[&str], run_dir: &str, args: Vec<String>) -> Vec<String> {
    let mut argv = fill(form, run_dir);
    let at = argv.len() - usize::from(argv.last().is_some_and(|arg| arg == "-p"));
    argv.splice(at..at, args);
    argv
}

/// Inputs as (name, value).
type Inputs = Vec<(&'static str, String)>;

/// The Moderator's Inputs, read as the Debate starts: each side's command
/// from its row, the two on Apps of different families, and "limited until
/// <t>" for a side whose App `limited` says is, the audit running on side
/// A's, and TypeSafe when off. With them side A's App, which runs the
/// audit's line.
pub(crate) fn debate_inputs(
    repo: &Path,
    run_dir: &str,
    limited: impl Fn(&str) -> Option<String>,
) -> Result<(Inputs, &'static App), String> {
    let (a, b) = (row(repo, "side_a")?, row(repo, "side_b")?);
    let sides = debate(family_of(a.app, &a.model), family_of(b.app, &b.model));
    if !sides.holds {
        return Err(sides.text);
    }
    let mut inputs = vec![
        ("Side A command", a.side_command(run_dir)),
        ("Side B command", b.side_command(run_dir)),
    ];
    for (side, row) in [("Side A", &a), ("Side B", &b)] {
        if let Some(when) = limited(row.app.name) {
            inputs.push((side, format!("limited until {when}")));
        }
    }
    if !typesafe(repo) {
        inputs.push(("TypeSafe", "off".to_string()));
    }
    Ok((inputs, a.app))
}

/// The Review's fallback row; None while its model is none, as it is while
/// config.json has no review_if_limited.
pub(crate) fn fallback_row(repo: &Path) -> Result<Option<Row>, String> {
    row(repo, IF_LIMITED).map(|row| Some(row).filter(|row| row.model != "none"))
}

/// The Stage's row, read from .harness/config.json as the Stage starts, so a
/// change reaches the Stages that start after it. A missing file, row or
/// field, or an empty one, is the default; a field not a string refuses.
pub(crate) fn stage_row(repo: &Path, st: &Stage) -> Result<Row, String> {
    let key = if st.name == DEBATE.name {
        "moderator"
    } else {
        st.name
    };
    row(repo, key)
}

/// Whether TypeSafe is on: config.json's "typesafe", read at each use so a
/// change reaches the next Judgment and Debate. Unset, or a config.json that
/// cannot be read, is on: the key alone decides, as before init asked.
pub(crate) fn typesafe(repo: &Path) -> bool {
    read(repo).map_or(true, |(_, doc)| doc["typesafe"] != false)
}

/// Keeps TypeSafe on or off in config.json, the rows as they were; a
/// config.json that is not an object is refused, not overwritten.
pub(crate) fn set_typesafe(repo: &Path, on: bool) -> Result<(), String> {
    let (path, mut doc) = read_object(repo)?;
    doc["typesafe"] = Value::Bool(on);
    write(&path, &doc)
}

/// A Judgment's confidence floor in config.json: its key, and the value
/// that stands for it missing or empty.
pub(crate) struct Floor {
    pub(crate) key: &'static str,
    pub(crate) default: f64,
}

/// A floor as config.json has it, read at each use as "typesafe" is, so a
/// change reaches the next Judgment. A config.json that cannot be read, or
/// a value that is not a number from 0 to 1, refuses: the Judgment is then
/// not acted on.
pub(crate) fn floor(repo: &Path, floor: &Floor) -> Result<f64, String> {
    floor_in(&read(repo)?.1, floor)
}

/// A floor in doc: missing or empty is its default; a null is refused.
pub(crate) fn floor_in(doc: &Value, floor: &Floor) -> Result<f64, String> {
    match doc.get(floor.key) {
        None => Ok(floor.default),
        Some(Value::String(s)) if s.is_empty() => Ok(floor.default),
        Some(value) => value
            .as_f64()
            .filter(|f| (0.0..=1.0).contains(f))
            .ok_or(format!("{} is not a number from 0 to 1", floor.key)),
    }
}

/// Keeps a floor in config.json, or with None takes it out, back to its
/// default; the rest as it was. A config.json that is not an object is
/// refused, as set_typesafe refuses it.
pub(crate) fn set_floor(repo: &Path, floor: &Floor, value: Option<f64>) -> Result<(), String> {
    let (path, mut doc) = read_object(repo)?;
    let top = doc.as_object_mut().expect("read_object gives an object");
    match value {
        Some(value) => top.insert(floor.key.to_string(), json!(value)),
        None => top.remove(floor.key),
    };
    write(&path, &doc)
}

/// The key of every row of config.json.
pub(crate) const ROWS: [&str; 8] = [
    "implement",
    "review",
    IF_LIMITED,
    "moderator",
    "side_a",
    "side_b",
    "fix",
    "address",
];

/// The row under key: a Stage's, or a Debate side's (side_a, side_b).
pub(crate) fn row(repo: &Path, key: &str) -> Result<Row, String> {
    let (path, doc) = read(repo)?;
    row_in(&doc, key, &path)
}

/// .harness/config.json and its path; a missing file is Null.
pub(crate) fn read(repo: &Path) -> Result<(PathBuf, Value), String> {
    let path = repo.join(".harness").join("config.json");
    let doc = match fs::read(&path) {
        Ok(raw) => {
            serde_json::from_slice(&raw).map_err(|err| format!("{}: {err}", path.display()))?
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Value::Null,
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    Ok((path, doc))
}

/// config.json to edit: missing (or null) is empty; any other non-object
/// refuses, so a save never writes over it.
pub(crate) fn read_object(repo: &Path) -> Result<(PathBuf, Value), String> {
    match read(repo)? {
        (path, Value::Null) => Ok((path, json!({}))),
        (path, doc) if doc.is_object() => Ok((path, doc)),
        (path, _) => Err(format!("{}: not a JSON object", path.display())),
    }
}

/// Writes config.json whole through a temp file, so a Stage reading it as
/// it starts never sees half of it.
pub(crate) fn write(path: &Path, doc: &Value) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(doc).unwrap() + "\n";
    fs::create_dir_all(path.parent().unwrap())
        .and_then(|()| fs::write(&tmp, text))
        .and_then(|()| fs::rename(&tmp, path))
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// A field of the row under key: missing or empty, its default (the row's
/// App, the fallback's none while config.json has no fallback row, else
/// default); not a string refuses.
pub(crate) fn field(doc: &Value, key: &str, name: &str) -> Result<String, String> {
    let default = match name {
        "app" if matches!(key, "review" | "side_b") => "codex",
        "app" => "claude",
        "model" if key == IF_LIMITED && doc[key].is_null() => "none",
        _ => "default",
    };
    match &doc[key][name] {
        Value::Null => Ok(default.to_string()),
        Value::String(v) if v.is_empty() => Ok(default.to_string()),
        Value::String(v) => Ok(v.clone()),
        _ => Err(format!("{key} {name} is not a string")),
    }
}

/// On codex only Implement (the two-step Plan, plan.rs), the Review, its
/// fallback and the Debate's sides run, until it has the network the
/// Moderator's side commands and TypeSafe calls need, and a Git write path
/// for Fix and Address: its sandbox keeps Git metadata read-only. Every
/// other App runs every Stage.
pub(crate) fn runs_on(key: &str, app: &App) -> Result<(), String> {
    match app.name != "codex"
        || matches!(
            key,
            "implement" | "review" | IF_LIMITED | "side_a" | "side_b"
        ) {
        true => Ok(()),
        false => Err(format!("{key} does not run on codex")),
    }
}

/// The row under key in doc, the config.json at path.
pub(crate) fn row_in(doc: &Value, key: &str, path: &Path) -> Result<Row, String> {
    let field =
        |name: &str| field(doc, key, name).map_err(|err| format!("{}: {err}", path.display()));
    let name = field("app")?;
    let app =
        app(&name).ok_or_else(|| format!("{}: no App named {name:?} for {key}", path.display()))?;
    runs_on(key, app)?;
    let model = field("model")?;
    // none, no model, is the fallback's alone: elsewhere it would run as one.
    if model == "none" && key != IF_LIMITED {
        return Err(format!(
            "{}: {key} model none: only {IF_LIMITED} takes none",
            path.display()
        ));
    }
    // A split: a plan model other than Implement's, not default.
    let plan_model = match key {
        "implement" => Some(field("plan_model")?),
        _ => None,
    }
    .filter(|plan| *plan != model && plan != "default");
    if plan_model.is_some() && app.name != "claude" {
        return Err(format!(
            "{}: implement plan_model splits from model: the split runs on claude only",
            path.display()
        ));
    }
    // Full ids: opusplan's remap env takes no alias, and an alias for one
    // half may resolve through the other's remap to the same model.
    let full = |m: &str| m.starts_with("claude-");
    if plan_model
        .as_deref()
        .is_some_and(|plan| !full(plan) || !full(&model))
    {
        return Err(format!(
            "{}: implement plan_model splits from model: \
             the split needs a full claude- model id for each half",
            path.display()
        ));
    }
    Ok(Row {
        app,
        model,
        effort: field("effort")?,
        plan_model,
    })
}

/// claude's aliases and the full ids they name.
// ponytail: pinned to today's models; add a row as Claude ships one.
const ALIASES: [(&str, &str); 4] = [
    ("fable", "claude-fable-5-1"),
    ("opus", "claude-opus-5-5"),
    ("sonnet", "claude-sonnet-5"),
    ("haiku", "claude-haiku-4-5"),
];

/// The full id a claude alias names; any other model as it is.
pub(crate) fn full_id(model: &str) -> String {
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == model)
        .map_or(model, |(_, id)| id)
        .to_string()
}

/// One name per model whichever App runs it: opus, opus-5.5,
/// anthropic/claude-opus-5-5, claude-opus-5-5[1m] and pi's
/// claude-opus-5-5:high are claude-opus-5-5.
pub(crate) fn canonical(model: &str) -> String {
    let lower = model.rsplit('/').next().unwrap_or(model).to_lowercase();
    let m = lower.split('[').next().unwrap_or_default();
    // pi's model:level; any other tag names a model of its own.
    let m = match m.rsplit_once(':') {
        Some((head, level)) if PI_THINKING.contains(&level) => head,
        _ => m,
    };
    let m = m.replace('.', "-");
    let m = match m.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => &m,
    };
    match ALIASES
        .iter()
        .find(|(alias, _)| m.starts_with(&format!("{alias}-")))
    {
        Some(_) => format!("claude-{m}"),
        None => full_id(m),
    }
}

/// The family each model name starts with, as its one name gives it.
// ponytail: the families a Debate side is likely to name; any other reads
// as unknown, which the rules refuse, until it is added here.
const FAMILIES: [(&str, &str); 4] = [
    ("claude-", "Anthropic"),
    ("gpt-", "OpenAI"),
    ("gemini-", "Google"),
    ("grok-", "xAI"),
];

/// The family of model on app: the App's own, else told from the model's
/// name; None when it cannot be told, as of such an App's default.
pub(crate) fn family_of(app: &App, model: &str) -> Option<&'static str> {
    if !app.family.is_empty() {
        return Some(app.family);
    }
    let m = canonical(model);
    FAMILIES
        .iter()
        .find(|(prefix, _)| m.starts_with(prefix))
        .map(|(_, family)| *family)
}

/// A model as the rules compare it: its one name, or the App's default;
/// None for the default of an App that runs several, which could be any.
// ponytail: an App's default counts as a model of its own, though
// claude's may be opus; resolve it with the probe when that bites.
fn model_id(app: &App, model: &str) -> Option<String> {
    match model {
        "default" if app.family.is_empty() => None,
        "default" => Some(format!("{}'s default", app.name)),
        _ => Some(canonical(model)),
    }
}

/// A rule config.json's rows keep: the rows it reads, whether it holds,
/// and what it says. /config flags a bad floor as one, its key for the row.
pub(crate) struct Check {
    pub(crate) rows: &'static [&'static str],
    pub(crate) holds: bool,
    pub(crate) text: String,
}

/// The Debate's rule: its sides from two families, each told.
fn debate(a: Option<&str>, b: Option<&str>) -> Check {
    let (holds, text) = match (a, b) {
        (Some(x), Some(y)) if x != y => (
            true,
            format!("The sides come from two families: {x} and {y}"),
        ),
        (Some(x), Some(_)) => (
            false,
            format!("Both sides would be {x}: the Debate needs two families"),
        ),
        _ => {
            let side = if a.is_none() { "Side A" } else { "Side B" };
            let text = format!("{side}'s family cannot be told from its model: name one");
            (false, text)
        }
    };
    Check {
        rows: &["side_a", "side_b"],
        holds,
        text,
    }
}

/// The rules over config.json: the Review and its fallback never run on
/// Implement's model; the Debate's sides come from two families. A row that
/// cannot be read is left out, as reading it refuses on its own.
pub(crate) fn checks(doc: &Value) -> Vec<Check> {
    let row = |key: &str| {
        Some((
            app(&field(doc, key, "app").ok()?)?,
            field(doc, key, "model").ok()?,
        ))
    };
    let mut out = Vec::new();
    let imp = row("implement");
    let reviews: [(&'static [&str], &str); 2] = [
        (&["implement", "review"], "The Review"),
        (&["implement", IF_LIMITED], "The Review if limited"),
    ];
    for (rows, who) in reviews {
        let (Some((ia, im)), Some((app, model))) = (&imp, row(rows[1])) else {
            continue;
        };
        if model == "none" {
            continue;
        }
        let (holds, text) = match (model_id(ia, im), model_id(app, &model)) {
            (Some(i), Some(r)) if i != r => {
                (true, format!("{who} runs on {r}, not Implement's {i}"))
            }
            (Some(i), Some(_)) => (
                false,
                format!(
                    "{who} would run on Implement's model, {i}: it must not review its own work"
                ),
            ),
            (i, _) => {
                let on = if i.is_none() { ia.name } else { app.name };
                (
                    false,
                    format!("{who} cannot be checked: {on}'s default model is unknown, name one"),
                )
            }
        };
        out.push(Check { rows, holds, text });
    }
    if let (Some(a), Some(b)) = (row("side_a"), row("side_b")) {
        out.push(debate(family_of(a.0, &a.1), family_of(b.0, &b.1)));
    }
    out
}

/// config.json keeps every rule: the first broken one refuses, as a run
/// starts on a config.json edited by hand.
pub(crate) fn check(repo: &Path) -> Result<(), String> {
    let (_, doc) = read(repo)?;
    match checks(&doc).into_iter().find(|c| !c.holds) {
        Some(broken) => Err(broken.text),
        None => Ok(()),
    }
}

/// The one-line prompt that tries model on app, before /config saves it:
/// the App's headless read-only command, in dir.
pub(crate) fn probe(app: &App, dir: &Path, model: &str) -> Vec<String> {
    let mut argv = side_argv(app.side, &dir.display().to_string(), fill(app.model, model));
    argv.push("Reply with ok".to_string());
    argv
}
