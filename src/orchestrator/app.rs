//! The App table, compiled in, and .harness/config.json: the App, model and
//! effort each Stage starts on.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::{json, Value};

use super::plan::quoted;
use super::stage::{Stage, DEBATE};
use super::trust::{claude_records, codex_records};

/// An agent CLI a Stage can run on: one row of the App table. In the arg
/// forms, "{}" is the value put in.
pub(crate) struct App {
    /// Its name in config.json, and herdr's agent kind.
    pub(crate) name: &'static str,
    /// The unattended args of the Review, in the Run directory, given the
    /// worktree.
    pub(crate) run_dir_args: fn(&str) -> Vec<String>,
    pub(crate) model: &'static [&'static str],
    pub(crate) effort: &'static [&'static str],
    pub(crate) resume: &'static [&'static str],
    /// The headless read-only command a Debate side and the audit run,
    /// before the model and effort args; the brief follows. "{}" is the Run
    /// directory, where the diff is.
    pub(crate) side: &'static [&'static str],
    /// Where the App records the directories it trusts: Some(trusted) when
    /// dir is recorded.
    pub(crate) trust: fn(&Path, &Path) -> Option<bool>,
    /// What its pane shows at a usage limit: regexes, with the reset in
    /// the group "reset" and which limit in "what" when the App says.
    pub(crate) limits: &'static [&'static str],
}

pub(crate) static APPS: [App; 2] = [
    App {
        name: "claude",
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
        limits: &[
            r"You['’]ve hit your (?P<what>.*?limit) · resets (?P<reset>.+)",
            r"Usage limit reached · continuing automatically at (?P<reset>.+?)(?: · |$)",
        ],
    },
    App {
        name: "codex",
        // The sandbox writes only where the pane starts: the result file
        // there, nothing in the worktree.
        run_dir_args: |_| ["--sandbox", "workspace-write"].map(String::from).to_vec(),
        model: &["-m", "{}"],
        effort: &["-c", "model_reasoning_effort={}"],
        resume: &["resume", "{}"],
        side: &["codex", "exec", "--sandbox", "read-only"],
        trust: codex_records,
        // U+2019 in You’ve; "Try again later." gives no reset
        limits: &[
            r"You['’]ve hit your (?P<what>usage limit)\..*?[Tt]ry again (?:at (?P<reset>.+?)|later)\.",
        ],
    },
];

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
    /// The model and effort args.
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
        fill(self.app.side, run_dir)
            .into_iter()
            .chain(self.flags())
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

/// The Moderator's Inputs, read as the Debate starts: each side's command
/// from its row, the audit running on side A's, and TypeSafe when off.
pub(crate) fn debate_inputs(
    repo: &Path,
    run_dir: &str,
) -> Result<Vec<(&'static str, String)>, String> {
    let mut inputs = vec![
        ("Side A command", row(repo, "side_a")?.side_command(run_dir)),
        ("Side B command", row(repo, "side_b")?.side_command(run_dir)),
    ];
    if !typesafe(repo) {
        inputs.push(("TypeSafe", "off".to_string()));
    }
    Ok(inputs)
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
    config(repo).map_or(true, |doc| doc["typesafe"] != false)
}

/// Keeps TypeSafe on or off in config.json, the rows as they were; a
/// config.json that is not an object is refused, not overwritten.
pub(crate) fn set_typesafe(repo: &Path, on: bool) -> Result<(), String> {
    let path = repo.join(".harness").join("config.json");
    let mut doc = config(repo)?;
    if doc.is_null() {
        doc = json!({});
    }
    let Some(fields) = doc.as_object_mut() else {
        return Err(format!("{}: not a JSON object", path.display()));
    };
    fields.insert("typesafe".to_string(), Value::Bool(on));
    fs::create_dir_all(path.parent().unwrap())
        .and_then(|()| fs::write(&path, format!("{doc:#}\n")))
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// config.json, Null when there is none.
fn config(repo: &Path) -> Result<Value, String> {
    let path = repo.join(".harness").join("config.json");
    match fs::read(&path) {
        Ok(raw) => serde_json::from_slice(&raw).map_err(|err| format!("{}: {err}", path.display())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Value::Null),
        Err(err) => Err(format!("{}: {err}", path.display())),
    }
}

/// The key of every row of config.json.
pub(crate) const ROWS: [&str; 7] = [
    "implement",
    "review",
    "moderator",
    "side_a",
    "side_b",
    "fix",
    "address",
];

/// The row under key: a Stage's, or a Debate side's (side_a, side_b).
pub(crate) fn row(repo: &Path, key: &str) -> Result<Row, String> {
    let path = repo.join(".harness").join("config.json");
    let doc = config(repo)?;
    let default = if matches!(key, "review" | "side_b") {
        "codex"
    } else {
        "claude"
    };
    let field = |name: &str, default: &str| match &doc[key][name] {
        Value::Null => Ok(default.to_string()),
        Value::String(v) if v.is_empty() => Ok(default.to_string()),
        Value::String(v) => Ok(v.clone()),
        _ => Err(format!("{}: {key} {name} is not a string", path.display())),
    };
    let name = field("app", default)?;
    let app =
        app(&name).ok_or_else(|| format!("{}: no App named {name:?} for {key}", path.display()))?;
    // Off claude only the Review and the Debate's sides run, until codex has
    // the two-step Plan, the network the Moderator's side commands and
    // TypeSafe calls need, and a Git write path: its sandbox keeps Git
    // metadata read-only.
    if app.name != "claude" && !matches!(key, "review" | "side_a" | "side_b") {
        return Err(format!("{key} runs on claude only"));
    }
    let model = field("model", "default")?;
    // A split: a plan model other than Implement's, not default.
    let plan_model = match key {
        "implement" => Some(field("plan_model", "default")?),
        _ => None,
    }
    .filter(|plan| *plan != model && plan != "default");
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
        effort: field("effort", "default")?,
        plan_model,
    })
}
