//! The App table, compiled in, and .harness/config.json: the App, model and
//! effort each Stage starts on.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::Value;

use super::stage::{Stage, DEBATE, IMPLEMENT};
use super::trust::{claude_records, codex_records};

/// An agent CLI a Stage can run on: one row of the App table. In the arg
/// forms, "{}" is the value put in.
pub(crate) struct App {
    /// Its name in config.json, and herdr's agent kind.
    pub(crate) name: &'static str,
    #[allow(dead_code)] // preflight's check that the App is installed (harness-7nq.20)
    pub(crate) binary: &'static str,
    #[allow(dead_code)] // /config's not-installed window (harness-7nq.9)
    pub(crate) homepage: &'static str,
    /// The unattended args of a Stage in the Ticket's worktree; "{}" is the
    /// Run directory.
    pub(crate) worktree_args: &'static [&'static str],
    /// The unattended args of the Review, in the Run directory; "{}" is the
    /// worktree.
    pub(crate) run_dir_args: &'static [&'static str],
    /// Added for Fix and Address, which push and open pull requests.
    pub(crate) network: &'static [&'static str],
    pub(crate) model: &'static [&'static str],
    pub(crate) effort: &'static [&'static str],
    /// How a Stage skill names a Delegate skill.
    #[allow(dead_code)] // the Stage skills' {{job}} lines (harness-7nq.19)
    pub(crate) mention: &'static str,
    /// Where the App records the directories it trusts: Some(trusted) when
    /// dir is recorded.
    pub(crate) trust: fn(&Path, &Path) -> Option<bool>,
}

pub(crate) static APPS: [App; 2] = [
    App {
        name: "claude",
        binary: "claude",
        homepage: "https://claude.com/product/claude-code",
        worktree_args: &["--permission-mode", "auto", "--add-dir", "{}"],
        run_dir_args: &["--permission-mode", "auto", "--add-dir", "{}"],
        network: &[],
        model: &["--model", "{}"],
        effort: &["--effort", "{}"],
        mention: "the {} skill",
        trust: claude_records,
    },
    App {
        name: "codex",
        binary: "codex",
        homepage: "https://developers.openai.com/codex",
        worktree_args: &[
            "--sandbox",
            "workspace-write",
            "-a",
            "never",
            "--add-dir",
            "{}",
        ],
        // The sandbox writes only where the pane starts: the result file
        // there, nothing in the worktree.
        run_dir_args: &["--sandbox", "workspace-write"],
        network: &["-c", "sandbox_workspace_write.network_access=true"],
        model: &["-m", "{}"],
        effort: &["-c", "model_reasoning_effort={}"],
        mention: "${}",
        trust: codex_records,
    },
];

/// The App config.json names.
pub(crate) fn app(name: &str) -> Option<&'static App> {
    APPS.iter().find(|a| a.name == name)
}

/// The rows of .harness/config.json and the App each starts on.
const ROWS: [(&str, &str); 8] = [
    ("implement", "claude"),
    ("review", "codex"),
    ("review_if_limited", "none"), // read by Limited on the Review (harness-7nq.15)
    ("moderator", "claude"),
    // the Debate's sides, read by harness-7nq.11; side A also runs the
    // ponytail audit
    ("side_a", "claude"),
    ("side_b", "codex"),
    ("fix", "claude"),
    ("address", "claude"),
];

/// One Stage's row: its App, model and effort; "default" passes no flag.
pub(crate) struct Row {
    pub(crate) app: &'static App,
    pub(crate) model: String,
    pub(crate) effort: String,
}

impl Row {
    /// The model and effort args.
    pub(crate) fn flags(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (form, value) in [
            (self.app.model, &self.model),
            (self.app.effort, &self.effort),
        ] {
            if value != "default" {
                out.extend(fill(form, value));
            }
        }
        out
    }

    /// As the started line names it: "claude", "claude opus/high".
    pub(crate) fn said(&self) -> String {
        match (self.model.as_str(), self.effort.as_str()) {
            ("default", "default") => self.app.name.to_string(),
            (model, "default") => format!("{} {model}", self.app.name),
            (model, effort) => format!("{} {model}/{effort}", self.app.name),
        }
    }
}

/// An arg form with value put in for "{}".
pub(crate) fn fill(form: &[&str], value: &str) -> Vec<String> {
    form.iter().map(|arg| arg.replace("{}", value)).collect()
}

/// The Stage's row, read from .harness/config.json as the Stage starts, so a
/// change reaches the Stages that start after it. A missing file, row or
/// field is the default.
pub(crate) fn stage_row(repo: &Path, st: &Stage) -> Result<Row, String> {
    let path = repo.join(".harness").join("config.json");
    let doc: Value = match fs::read(&path) {
        Ok(raw) => {
            serde_json::from_slice(&raw).map_err(|err| format!("{}: {err}", path.display()))?
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Value::Null,
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    let key = if st.name == DEBATE.name {
        "moderator"
    } else {
        st.name
    };
    let (_, default) = ROWS
        .iter()
        .find(|(k, _)| *k == key)
        .expect("every Stage has a row");
    let field = |name: &str, default: &str| {
        let value = doc[key][name].as_str().filter(|v| !v.is_empty());
        value.unwrap_or(default).to_string()
    };
    let name = field("app", default);
    let app =
        app(&name).ok_or_else(|| format!("{}: no App named {name:?} for {key}", path.display()))?;
    if st.name == IMPLEMENT.name && app.name != "claude" {
        return Err("Implement off claude needs the two-step Plan".to_string());
    }
    Ok(Row {
        app,
        model: field("model", "default"),
        effort: field("effort", "default"),
    })
}
