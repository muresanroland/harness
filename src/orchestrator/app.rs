//! The App table, compiled in, and .harness/config.json: the App, model and
//! effort each Stage starts on.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::plan::quoted;
use super::stage::{Stage, DEBATE};
use super::trust::{claude_records, codex_records};
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
    /// Its models' family, which /config labels each with.
    pub(crate) family: &'static str,
    /// The models /config offers besides default, run in the given dir.
    pub(crate) models: fn(&dyn Tools, &Path) -> Result<Vec<Model>, String>,
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
        family: "Anthropic",
        models: |_, _| {
            let efforts = ["low", "medium", "high", "xhigh", "max"].map(String::from);
            Ok(["fable", "opus", "sonnet", "haiku"]
                .map(|m| (m.to_string(), efforts.to_vec()))
                .to_vec())
        },
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
        family: "OpenAI",
        models: codex_models,
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
    let strings = |v: &Value, name: &str| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x[name].as_str().map(String::from))
            .collect()
    };
    Ok(doc["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"] == "list")
        .filter_map(|m| {
            let slug = m["slug"].as_str()?.to_string();
            Some((slug, strings(&m["supported_reasoning_levels"], "effort")))
        })
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
/// from its row. The audit runs on side A's.
pub(crate) fn debate_inputs(
    repo: &Path,
    run_dir: &str,
) -> Result<Vec<(&'static str, String)>, String> {
    Ok(vec![
        ("Side A command", row(repo, "side_a")?.side_command(run_dir)),
        ("Side B command", row(repo, "side_b")?.side_command(run_dir)),
    ])
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

/// The row under key: a Stage's, or a Debate side's (side_a, side_b).
fn row(repo: &Path, key: &str) -> Result<Row, String> {
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
/// App, the fallback's none, else default); not a string refuses.
pub(crate) fn field(doc: &Value, key: &str, name: &str) -> Result<String, String> {
    let default = match name {
        "app" if matches!(key, "review" | "side_b") => "codex",
        "app" => "claude",
        "model" if key == IF_LIMITED => "none",
        _ => "default",
    };
    match &doc[key][name] {
        Value::Null => Ok(default.to_string()),
        Value::String(v) if v.is_empty() => Ok(default.to_string()),
        Value::String(v) => Ok(v.clone()),
        _ => Err(format!("{key} {name} is not a string")),
    }
}

/// Off claude only the Review, its fallback and the Debate's sides run,
/// until codex has the two-step Plan, the network the Moderator's side
/// commands and TypeSafe calls need, and a Git write path: its sandbox keeps
/// Git metadata read-only.
pub(crate) fn runs_on(key: &str, app: &App) -> Result<(), String> {
    match app.name == "claude" || matches!(key, "review" | IF_LIMITED | "side_a" | "side_b") {
        true => Ok(()),
        false => Err(format!("{key} runs on claude only")),
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
    // A split: a plan model other than Implement's, not default.
    let plan_model = match key {
        "implement" => Some(field("plan_model")?),
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
        effort: field("effort")?,
        plan_model,
    })
}

/// The one-line prompt that tries model on app, before /config saves it:
/// the App's headless read-only command, in dir.
pub(crate) fn probe(app: &App, dir: &Path, model: &str) -> Vec<String> {
    let mut argv = fill(app.side, &dir.display().to_string());
    argv.extend(fill(app.model, model));
    argv.push("Reply with ok".to_string());
    argv
}
