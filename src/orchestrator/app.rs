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
    /// Its models' family; None for an App that runs several, where only
    /// a named model's name tells.
    pub(crate) family: Option<&'static str>,
    /// Where to get it, for /config's window on an App not installed.
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
}

impl App {
    /// Whether it loads a skill manifest::list found: the name list gives
    /// it, and the folder it is in.
    pub(crate) fn loads(&self, name: &str, dir: &Path) -> bool {
        (self.plugins && name.contains(':'))
            || dir.ends_with(self.skill_dir)
            || dir.ends_with(".harness/skills")
    }
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
        family: Some("Anthropic"),
        home: "https://claude.com/product/claude-code",
        models: |_, _| {
            let efforts = ["low", "medium", "high", "xhigh", "max"].map(String::from);
            Ok(["fable", "opus", "sonnet", "haiku"]
                .map(|m| (m.to_string(), efforts.to_vec()))
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
        family: Some("OpenAI"),
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
    let sides = debate((a.app, &a.model), (b.app, &b.model));
    if sides.holds != Some(true) {
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
/// anthropic/claude-opus-5-5 and claude-opus-5-5[1m] are claude-opus-5-5.
pub(crate) fn canonical(model: &str) -> String {
    let m = model.rsplit('/').next().unwrap_or(model).to_lowercase();
    let m = m.split('[').next().unwrap_or_default().replace('.', "-");
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

/// What a model's name tells of its family, on an App of no one family.
const FAMILIES: [(&str, &str); 8] = [
    ("claude", "Anthropic"),
    ("fable", "Anthropic"),
    ("opus", "Anthropic"),
    ("sonnet", "Anthropic"),
    ("haiku", "Anthropic"),
    ("gpt", "OpenAI"),
    ("gemini", "Google"),
    ("grok", "xAI"),
];

/// The family of model on app: the App's own, else what the model's name
/// tells; None when neither does, as for a default there.
pub(crate) fn family(app: &App, model: &str) -> Option<&'static str> {
    let m = model.to_lowercase();
    app.family.or_else(|| {
        FAMILIES
            .iter()
            .find(|(name, _)| m.contains(name))
            .map(|(_, family)| *family)
    })
}

/// A model as the rules compare it: its one name, or the App's default,
/// which cannot be told on an App of no one family.
fn model_id(app: &App, model: &str) -> Option<String> {
    match model {
        "default" => app.family.map(|_| format!("{}'s default", app.name)),
        _ => Some(canonical(model)),
    }
}

/// A rule config.json's rows keep: the rows it reads, whether it holds
/// (None: a family or model that cannot be told, which counts as broken),
/// and what it says.
pub(crate) struct Check {
    pub(crate) rows: &'static [&'static str],
    pub(crate) holds: Option<bool>,
    pub(crate) text: String,
}

/// The Debate's rule: its sides from two families, each told.
fn debate(a: (&App, &str), b: (&App, &str)) -> Check {
    let (holds, text) = match (family(a.0, a.1), family(b.0, b.1)) {
        (Some(x), Some(y)) if x != y => (
            Some(true),
            format!("The sides come from two families: {x} and {y}"),
        ),
        (Some(x), Some(_)) => (
            Some(false),
            format!("Both sides would be {x}: the Debate needs two families"),
        ),
        (x, _) => {
            let (side, (app, model)) = if x.is_none() { ("A", a) } else { ("B", b) };
            let text = format!(
                "side {side}'s family can't be told ({} {model}): pick a model that names it",
                app.name
            );
            (None, text)
        }
    };
    Check {
        rows: &["side_a", "side_b"],
        holds,
        text,
    }
}

/// The rules over a table of rows: each row's App and model by its key,
/// "plan" giving Implement's plan model. A split plans and implements in
/// one family; the Review and its fallback never run on Implement's model;
/// the Debate's sides come from two families.
pub(crate) fn rules<'a>(row: impl Fn(&str) -> Option<(&'a App, String)>) -> Vec<Check> {
    let mut out = Vec::new();
    let imp = row("implement");
    if let Some((app, model)) = &imp {
        let plan = row("plan").filter(|(_, plan)| plan != "default" && plan != model);
        if let Some((_, plan)) = plan {
            let (holds, text) = match (family(app, &plan), family(app, model)) {
                (Some(p), Some(i)) if p == i => (
                    Some(true),
                    format!("Plans on {plan} and implements on {model}, both {p}"),
                ),
                (Some(_), Some(_)) => (
                    Some(false),
                    format!("The plan ({plan}) and the implementation ({model}) must be one family"),
                ),
                _ => (
                    None,
                    format!(
                        "The plan's family or the implementation's can't be told ({} {plan}, {model}): pick models that name it",
                        app.name
                    ),
                ),
            };
            out.push(Check {
                rows: &["implement"],
                holds,
                text,
            });
        }
    }
    let reviews: [(&'static [&str], &str, &str); 2] = [
        (&["implement", "review"], "The Review", "The Review's"),
        (
            &["implement", IF_LIMITED],
            "The Review if limited",
            "The fallback's",
        ),
    ];
    for (rows, who, whose) in reviews {
        let (Some((ia, im)), Some((app, model))) = (&imp, row(rows[1])) else {
            continue;
        };
        if model == "none" {
            continue;
        }
        let (holds, text) = match (model_id(ia, im), model_id(app, &model)) {
            (Some(i), Some(r)) if i == r => (
                Some(false),
                format!(
                    "{who} would run on Implement's model, {i}: it must not review its own work"
                ),
            ),
            (Some(i), Some(r)) => (
                Some(true),
                format!("{who} runs on {r}, not Implement's {i}"),
            ),
            (None, _) => (
                None,
                format!(
                    "Implement's model can't be told ({} default): pick a named model",
                    ia.name
                ),
            ),
            (_, None) => (
                None,
                format!(
                    "{whose} model can't be told ({} default): pick a named model",
                    app.name
                ),
            ),
        };
        out.push(Check { rows, holds, text });
    }
    if let (Some(a), Some(b)) = (row("side_a"), row("side_b")) {
        out.push(debate((a.0, &a.1), (b.0, &b.1)));
    }
    out
}

/// The rules over config.json; a row that cannot be read is left out, as
/// reading it refuses on its own.
pub(crate) fn checks(doc: &Value) -> Vec<Check> {
    rules(|key| {
        let (key, name) = match key {
            "plan" => ("implement", "plan_model"),
            key => (key, "model"),
        };
        Some((
            app(&field(doc, key, "app").ok()?)?,
            field(doc, key, name).ok()?,
        ))
    })
}

/// config.json keeps every rule: the first broken one refuses, as a run
/// starts on a config.json edited by hand.
pub(crate) fn check(repo: &Path) -> Result<(), String> {
    let (_, doc) = read(repo)?;
    match checks(&doc).into_iter().find(|c| c.holds != Some(true)) {
        Some(broken) => Err(broken.text),
        None => Ok(()),
    }
}

/// The one-line prompt that tries model on app, before /config saves it:
/// the App's headless read-only command, in dir.
pub(crate) fn probe(app: &App, dir: &Path, model: &str) -> Vec<String> {
    let mut argv = fill(app.side, &dir.display().to_string());
    argv.extend(fill(app.model, model));
    argv.push("Reply with ok".to_string());
    argv
}
