//! /config: the App, model and effort of each row of .harness/config.json,
//! picked in a modal docked beside the live Shell (layout C of
//! harness-0sx.9, drawn in draw/config.rs) and saved at once. A Stage reads
//! its row as it starts, so the Stages that start after a change use it and
//! running ones keep theirs. A named model is probed with a one-line prompt
//! first, off the screen thread; it saves only if the App answers. Also each
//! job's Delegate skill, the skills the Harness installed (harness-0sx.7),
//! cloned off the screen thread too, and TypeSafe on or off with its key
//! and the Judgments' floors.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use crossterm::event::KeyCode;
use ratatui::style::Color;
use serde_json::{json, Value};

use super::logo::{CYAN, GREEN, MUTED, ORANGE, RED};
use super::{Screen, NOTICE_WINDOW};
use crate::orchestrator::app::{self, app, App, Check, Floor, Model, Row, APPS, IF_LIMITED};
use crate::orchestrator::judgment::{PLAN_FLOOR, WAKE_FLOOR};
use crate::setup;
use crate::skills::manifest::{self, job_row, parse_source, Added, Manifest, JOBS, NONE};
use crate::tools::{RunError, Tools};

/// One row of config.json: its key, its name, the lead of its settings'
/// labels on a section's page ("" for the section's own row), the section
/// it sits in, and what it runs.
pub(crate) struct ConfigRow {
    pub(crate) key: &'static str,
    pub(crate) name: &'static str,
    pub(crate) lead: &'static str,
    pub(crate) section: usize,
    pub(crate) note: &'static str,
}

pub(crate) const ROWS: [ConfigRow; 8] = [
    ConfigRow {
        key: "implement",
        name: "Implement",
        lead: "",
        section: 0,
        note: "Plans, implements test-first, reviews itself and commits in the Ticket's worktree.",
    },
    ConfigRow {
        key: "review",
        name: "Review",
        lead: "",
        section: 1,
        note: "Reviews the diff into Findings.",
    },
    ConfigRow {
        key: IF_LIMITED,
        name: "Review if limited",
        lead: "if limited:",
        section: 1,
        note: "Runs the Review when the Review's App is Limited and you answer to review with it; none leaves wait or open the PR unreviewed.",
    },
    ConfigRow {
        key: "moderator",
        name: "Moderator",
        lead: "Moderator",
        section: 2,
        note: "stage-moderate's pane: runs the Debate and settles each Finding.",
    },
    ConfigRow {
        key: "side_a",
        name: "Debate side A",
        lead: "side A",
        section: 2,
        note: "Argues each Finding, headless; also runs the ponytail audit.",
    },
    ConfigRow {
        key: "side_b",
        name: "Debate side B",
        lead: "side B",
        section: 2,
        note: "Argues each Finding, headless, on a model from another family than side A.",
    },
    ConfigRow {
        key: "fix",
        name: "Fix",
        lead: "",
        section: 3,
        note: "Fixes the Findings to fix, then opens the PR.",
    },
    ConfigRow {
        key: "address",
        name: "Address",
        lead: "",
        section: 4,
        note: "Resolves a PR's merge conflicts or review comments.",
    },
];

/// The Pipeline's sections: title, short name on the left, description.
pub(crate) const SECTIONS: [(&str, &str, &str); 5] = [
    (
        "Plan + Implement",
        "Plan+Impl",
        "In the Ticket's worktree: plans in plan mode, then implements test-first, reviews itself and commits.",
    ),
    (
        "Review",
        "Review",
        "Reviews the diff into Findings, respecting earlier Verdicts.",
    ),
    (
        "Debate",
        "Debate",
        "The Moderator puts each disputed Finding to side A and side B, two models from different families; TypeSafe scores what they still dispute.",
    ),
    ("Fix", "Fix", "Fixes the Findings to fix, then opens the PR."),
    (
        "Address",
        "Address",
        "Resolves a PR's merge conflicts or its review comments.",
    ),
];

/// The Apps page's place on the left, after the Pipeline's sections, and
/// the Skills and TypeSafe pages' after it.
pub(crate) const APPS_PAGE: usize = SECTIONS.len();
pub(crate) const SKILLS_PAGE: usize = APPS_PAGE + 1;
pub(crate) const TYPESAFE_PAGE: usize = APPS_PAGE + 2;

/// The floors the TypeSafe page shows under the key, in its order.
pub(crate) const FLOORS: [&Floor; 2] = [&WAKE_FLOOR, &PLAN_FLOOR];

/// A floor as the pages name it: "wake floor".
pub(crate) fn floor_name(floor: &Floor) -> String {
    floor.key.replace('_', " ")
}

/// What turning TypeSafe off changes, asked first.
const TYPESAFE_OFF: &str = "Turn TypeSafe off? Every Wake and Plan becomes a Question; disputed Findings are skipped and listed in the PR.";

/// A setting of a row.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Field {
    App,
    Model,
    Effort,
    /// Implement's plan model, other than its model on a split.
    Plan,
    /// Plan + Implement's 'Same model for plan and implementation'.
    Same,
    /// A job's Delegate skill, of JOBS, kept in the Skill manifest.
    Job(usize),
}

impl Field {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Field::Plan => "plan model",
            Field::Same => "same model",
            field => field.key(),
        }
    }

    /// Its name in config.json.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Field::App => "app",
            Field::Model => "model",
            Field::Effort => "effort",
            Field::Plan => "plan_model",
            Field::Same => "same",
            Field::Job(j) => JOBS[j].0,
        }
    }
}

/// A job as a Stage's page names it.
pub(crate) fn job_name(j: usize) -> String {
    match JOBS[j].0 {
        "audit" => "over-engineering audit".to_string(),
        "test-first" => "test-first".to_string(),
        job => job.replace('-', " "),
    }
}

/// The row whose App runs a job's line.
pub(crate) fn job_row_of(j: usize) -> usize {
    let key = job_row(JOBS[j].0);
    ROWS.iter().position(|r| r.key == key).unwrap()
}

/// A job with its section: "Plan + Implement test-first".
pub(crate) fn job_said(j: usize) -> String {
    format!(
        "{} {}",
        SECTIONS[ROWS[job_row_of(j)].section].0,
        job_name(j)
    )
}

/// A clone URL as the pages show it: owner/repo on GitHub.
pub(crate) fn short(repo: &str) -> &str {
    repo.strip_prefix("https://github.com/").unwrap_or(repo)
}

/// A commit as the pages show it.
pub(crate) fn short_commit(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

/// Every skill you have, each with its folder as the pages show it: from the
/// repo's root, or ~ for home.
fn found(repo: &Path, home: &Path, tools: &dyn Tools) -> Vec<(String, PathBuf)> {
    manifest::list(repo, home, tools)
        .into_iter()
        .map(|(name, path)| {
            let dir = path.parent().unwrap_or(&path);
            let shown = match (dir.strip_prefix(repo), dir.strip_prefix(home)) {
                (Ok(rel), _) => rel.to_path_buf(),
                (_, Ok(rel)) if !home.as_os_str().is_empty() => Path::new("~").join(rel),
                _ => dir.to_path_buf(),
            };
            (name, shown)
        })
        .collect()
}

/// An open pick list, which replaces the section's page.
pub(crate) struct Pick {
    pub(crate) row: usize,
    pub(crate) field: Field,
    /// A model list for a new App, picked just before: the pair saves together.
    pub(crate) app: Option<&'static App>,
    pub(crate) cursor: usize,
    pub(crate) filter: String,
}

/// What a pick list's entry picks.
#[derive(Clone)]
pub(crate) enum Picked {
    App(&'static App),
    /// A model, an effort or a skill, as the pick's field says.
    Value(String),
    /// 'type an id…'
    Typed,
    /// A suggested skill not installed: its name and source, installed first.
    Install(&'static str, &'static str),
}

/// A line of a pick list; one that picks nothing is a heading.
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) detail: String,
    pub(crate) current: bool,
    /// The rule a model picked would break: '✗ side A's family'; how the
    /// job's App has a skill: 'installed', 'yours'.
    pub(crate) mark: Option<(&'static str, Color)>,
    /// Greyed: an App not installed.
    pub(crate) dim: bool,
    pub(crate) picks: Option<Picked>,
}

/// A named model being tried before its change saves.
pub(crate) struct Probe {
    result: Receiver<Result<String, RunError>>,
    row: usize,
    fields: Vec<(Field, String)>,
    pub(crate) app: &'static str,
    pub(crate) model: String,
}

/// What the foot's line is typing.
pub(crate) enum Typing {
    /// A model id, for the model list it came from.
    Model(Pick),
    /// A skill's source, on the Skills page.
    Source,
    /// The TypeSafe key, shown as dots.
    Key,
    /// A floor, on the TypeSafe page.
    Floor(&'static Floor),
}

/// The skills of a source that holds several: each name, whether it is
/// installed from there already (ticked for good), and whether it is ticked.
pub(crate) struct Listing {
    pub(crate) source: String,
    pub(crate) names: Vec<(String, bool, bool)>,
    pub(crate) cursor: usize,
}

/// What a yes to the foot's question does.
pub(crate) enum Confirm {
    Remove(String),
    TypeSafeOff,
}

/// A clone or an update going on its own thread, said in the foot.
pub(crate) struct Busy {
    pub(crate) text: String,
    done: Receiver<(Done, Vec<(String, PathBuf)>)>,
}

/// What a clone came back with.
enum Done {
    /// A pasted source, and what add made of it.
    Added(String, Result<Added, String>),
    /// The checklist's ticked skills, each with what add made of it.
    Ticked(Vec<(String, Result<Added, String>)>),
    /// A job's suggestion, installed for the job to pick.
    ForJob(usize, Result<Added, String>),
    /// One skill updated, or every one (None): those that failed, with why.
    Updated(Option<String>, Result<Vec<(String, String)>, String>),
}

/// /config, open.
pub(crate) struct Settings {
    pub(crate) doc: Value,
    /// Each App's models, as APPS orders them, read when /config opened.
    models: Vec<Result<Vec<Model>, String>>,
    /// Each App, as APPS orders them, on PATH: the first line its
    /// --version prints; None when not on PATH.
    pub(crate) installed: Vec<Option<String>>,
    /// The Skill manifest, read again after each change to it.
    pub(crate) manifest: Manifest,
    /// Every skill you have, as found() gives them.
    found: Vec<(String, PathBuf)>,
    /// The section on the left (APPS_PAGE past them), and whether the
    /// cursor is on its page.
    pub(crate) section: usize,
    pub(crate) open: bool,
    /// The setting under the cursor on the page, of Settings::items.
    pub(crate) setting: usize,
    pub(crate) pick: Option<Pick>,
    /// The foot's line being typed.
    pub(crate) typing: Option<(Typing, String)>,
    pub(crate) probe: Option<Probe>,
    /// Each App and model a probe answered while this /config is open: not
    /// probed again till it opens anew.
    probed: Vec<(&'static str, String)>,
    pub(crate) busy: Option<Busy>,
    /// A source's skills to tick, which replaces the Skills page.
    pub(crate) listing: Option<Listing>,
    /// The foot's yes/no question.
    pub(crate) confirm: Option<(String, Confirm)>,
    /// The foot's line in place of the row's note: saved, refused, failed.
    pub(crate) note: Option<(String, Color)>,
    /// When this /config last saved.
    pub(crate) saved: Option<String>,
}

/// A row's setting in doc, the default filled in; a field not a string
/// shows why.
fn value(doc: &Value, row: usize, field: Field) -> String {
    app::field(doc, ROWS[row].key, field.key()).unwrap_or_else(|err| err)
}

/// Implement's plan model in doc on a split: one other than its model, not
/// default.
fn split(doc: &Value) -> Option<String> {
    let plan = value(doc, 0, Field::Plan);
    (plan != "default" && plan != value(doc, 0, Field::Model)).then_some(plan)
}

/// A row in doc as RECENT's started line names it: "codex gpt-6-sol/high",
/// on a split "claude claude-fable-5-1→claude-opus-5-5", or "none" for a
/// fallback that is not set.
fn said(doc: &Value, row: usize) -> String {
    let model = value(doc, row, Field::Model);
    match app(&value(doc, row, Field::App)) {
        _ if model == "none" => model,
        None => value(doc, row, Field::App),
        Some(app) => Row {
            app,
            model,
            effort: value(doc, row, Field::Effort),
            plan_model: split(doc).filter(|_| row == 0),
        }
        .said(),
    }
}

/// The family of model on app as /config shows it.
pub(crate) fn family_label(app: &App, model: &str) -> &'static str {
    app::family_of(app, model).unwrap_or("family unknown")
}

/// The note on an App not on PATH: where to get it.
fn not_on_path(app: &App) -> String {
    format!("{} is not on PATH: get it at {}", app.bin, app.home)
}

/// The rows of a section.
fn rows_of(section: usize) -> impl Iterator<Item = usize> {
    (0..ROWS.len()).filter(move |&r| ROWS[r].section == section)
}

/// The section a check shows on: its last row's; a floor's, the TypeSafe
/// page.
pub(crate) fn section_of(check: &Check) -> usize {
    let key = check.rows[check.rows.len() - 1];
    ROWS.iter()
        .find(|r| r.key == key)
        .map_or(TYPESAFE_PAGE, |r| r.section)
}

/// Each once, in the order first met.
pub(crate) fn distinct(all: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for one in all {
        if !out.contains(&one) {
            out.push(one);
        }
    }
    out
}

impl Settings {
    /// A setting as it is: a job's the Skill manifest's pick.
    pub(crate) fn value(&self, row: usize, field: Field) -> String {
        match field {
            Field::Job(j) => self.manifest.pick(JOBS[j].0).to_string(),
            _ => value(&self.doc, row, field),
        }
    }

    /// Whether TypeSafe judges: not off in config.json, and with a key.
    pub(crate) fn typesafe(&self, key: &str) -> bool {
        !key.is_empty() && self.doc["typesafe"] != false
    }

    /// The skills installed but the Shipped ones.
    pub(crate) fn skills(&self) -> usize {
        self.manifest.skills.values().filter(|s| !s.shipped).count()
    }

    /// The jobs that pick a skill, each as job_said names it.
    pub(crate) fn jobs_using(&self, name: &str) -> Vec<String> {
        (0..JOBS.len())
            .filter(|&j| self.manifest.pick(JOBS[j].0) == name)
            .map(job_said)
            .collect()
    }

    /// The skills app loads, each name once with its folder, the Shipped ones
    /// left out.
    fn seen(&self, app: &App) -> Vec<(&str, &Path)> {
        let mut out: Vec<(&str, &Path)> = Vec::new();
        for (name, dir) in &self.found {
            let shipped = self.manifest.skills.get(name).is_some_and(|s| s.shipped);
            if app.loads(name, dir) && !shipped && !out.iter().any(|(n, _)| n == name) {
                out.push((name, dir));
            }
        }
        out
    }

    /// How app has a skill: built into it, installed by the Harness, or
    /// yours; with where from. None when it lacks it.
    pub(crate) fn have(&self, app: &App, name: &str) -> Option<(&'static str, Color, String)> {
        if app.built_in.contains(&name) {
            return Some(("built in", CYAN, format!("built into {}", app.name)));
        }
        let (_, dir) = self.seen(app).into_iter().find(|(n, _)| *n == name)?;
        Some(
            match (self.manifest.skills.get(name), name.split_once(':')) {
                (Some(skill), _) => ("installed", GREEN, short(&skill.repo).to_string()),
                (None, Some((plugin, _))) => ("yours", CYAN, format!("plugin {plugin}")),
                (None, None) => ("yours", CYAN, dir.display().to_string()),
            },
        )
    }

    /// The App a row runs on, None for a name no App has.
    pub(crate) fn app(&self, row: usize) -> Option<&'static App> {
        app(&self.value(row, Field::App))
    }

    /// Whether app was on PATH when /config opened.
    pub(crate) fn is_installed(&self, app: &App) -> bool {
        APPS.iter()
            .zip(&self.installed)
            .any(|(a, on)| on.is_some() && a.name == app.name)
    }

    /// The Apps line on the left: "1 of 2 installed".
    pub(crate) fn apps_summary(&self) -> String {
        let on = self.installed.iter().filter(|on| on.is_some()).count();
        format!("{on} of {} installed", APPS.len())
    }

    /// The foot's note on the Apps page's App i.
    pub(crate) fn app_note(&self, i: usize) -> String {
        match self.installed[i] {
            Some(_) => format!("{} is on PATH: a Stage can run on it.", APPS[i].name),
            None => not_on_path(&APPS[i]),
        }
    }

    /// Implement's plan model on a split.
    pub(crate) fn split(&self) -> Option<String> {
        split(&self.doc)
    }

    /// The rules on the rows as they are, and each floor that is not a
    /// number from 0 to 1; those of a section when given. A floor is not a
    /// rule a run refuses to start on: its Judgments ask instead.
    pub(crate) fn checks(&self, section: Option<usize>) -> Vec<Check> {
        let mut checks = app::checks(&self.doc);
        for floor in FLOORS {
            if let Err(err) = app::floor_in(&self.doc, floor) {
                checks.push(Check {
                    rows: std::slice::from_ref(&floor.key),
                    holds: false,
                    text: format!("{err}: its Judgments are not acted on"),
                });
            }
        }
        checks.retain(|c| section.is_none_or(|s| section_of(c) == s));
        checks
    }

    /// A section's line on the left: its row's App and model; the Debate's
    /// Apps.
    pub(crate) fn summary(&self, section: usize) -> String {
        if section == 2 {
            return distinct(rows_of(section).map(|r| self.value(r, Field::App))).join("+");
        }
        let row = rows_of(section).next().unwrap();
        if let Some(plan) = self.split().filter(|_| section == 0) {
            let model = self.value(row, Field::Model);
            return format!("{} {plan}→{model}", self.value(row, Field::App));
        }
        match self.value(row, Field::Model).as_str() {
            "default" => self.value(row, Field::App),
            model => format!("{} {model}", self.value(row, Field::App)),
        }
    }

    /// The open section's settings, row by row; Plan + Implement's with
    /// its toggle, and the plan model on a split.
    pub(crate) fn items(&self) -> Vec<(usize, Field)> {
        let mut items: Vec<(usize, Field)> = if self.section == 0 {
            let plan = self.split().map(|_| (0, Field::Plan));
            let head = [(0, Field::App), (0, Field::Same)].into_iter().chain(plan);
            head.chain([(0, Field::Model), (0, Field::Effort)])
                .collect()
        } else {
            rows_of(self.section)
                .flat_map(|r| [Field::App, Field::Model, Field::Effort].map(|f| (r, f)))
                .collect()
        };
        let jobs = (0..JOBS.len()).map(|j| (job_row_of(j), Field::Job(j)));
        items.extend(jobs.filter(|&(r, _)| ROWS[r].section == self.section));
        items
    }

    /// The App a pick's models and efforts are for.
    pub(crate) fn pick_app(&self, pick: &Pick) -> Option<&'static App> {
        pick.app.or_else(|| self.app(pick.row))
    }

    /// What app listed when /config opened.
    fn catalog(&self, app: &App) -> &Result<Vec<Model>, String> {
        &self.models[APPS.iter().position(|a| a.name == app.name).unwrap()]
    }

    fn listed(&self, app: &App) -> &[Model] {
        self.catalog(app).as_deref().unwrap_or_default()
    }

    /// The effort levels of model on app: its own when listed, else every
    /// listed model's.
    fn efforts(&self, app: &App, model: &str) -> Vec<String> {
        let listed = self.listed(app);
        match listed.iter().find(|(id, _)| id == model) {
            Some((_, efforts)) => efforts.clone(),
            None => distinct(listed.iter().flat_map(|(_, efforts)| efforts.clone())),
        }
    }

    /// A pick list's lines, filtered: a heading only unfiltered, 'type an
    /// id…' always.
    pub(crate) fn entries(&self, pick: &Pick) -> Vec<Entry> {
        if let Field::Job(j) = pick.field {
            return self.job_entries(pick, j);
        }
        let current = match pick.app {
            Some(_) => String::new(),
            None => self.value(pick.row, pick.field),
        };
        let filter = pick.filter.to_lowercase();
        let mut out = Vec::new();
        let mut entry = |name: &str, detail: String, picks: Option<Picked>| {
            let shown = match &picks {
                None => filter.is_empty(),
                Some(Picked::Typed) => true,
                Some(_) => name.to_lowercase().contains(&filter),
            };
            let model = matches!(pick.field, Field::Model | Field::Plan);
            let current = match &picks {
                None => false,
                Some(_) if model => app::canonical(name) == app::canonical(&current),
                Some(_) => name == current,
            };
            let mark = match &picks {
                Some(Picked::Value(m)) if model => self.mark_of(pick, m),
                _ => None,
            };
            let dim = matches!(&picks, Some(Picked::App(a)) if !self.is_installed(a));
            if shown {
                out.push(Entry {
                    name: name.to_string(),
                    detail,
                    current,
                    mark,
                    dim,
                    picks,
                });
            }
        };
        // A row on an App no longer in the table still lists the Apps.
        match (pick.field, self.pick_app(pick)) {
            (Field::App, _) => {
                for a in &APPS {
                    // Not installed, an App is greyed too.
                    let detail = match (self.is_installed(a), a.experimental) {
                        (_, true) => "experimental, unverified",
                        (false, false) => "not installed",
                        (true, false) => a.family,
                    };
                    entry(a.name, detail.into(), Some(Picked::App(a)));
                }
            }
            (_, None) | (Field::Same | Field::Job(_), _) => {}
            (Field::Plan, Some(app)) => {
                for (id, _) in self.listed(app) {
                    entry(
                        id,
                        family_label(app, id).into(),
                        Some(Picked::Value(id.clone())),
                    );
                }
                let detail = "probed before it saves".to_string();
                entry("type an id…", detail, Some(Picked::Typed));
            }
            (Field::Model, Some(app)) => {
                let value = |m: &str| Some(Picked::Value(m.to_string()));
                if ROWS[pick.row].key == IF_LIMITED {
                    let detail = "no fallback".to_string();
                    entry("none", detail, value("none"));
                }
                // A split needs a named model for each half.
                if pick.row != 0 || pick.app.is_some() || self.split().is_none() {
                    let detail = format!("{}'s own · {}", app.name, family_label(app, "default"));
                    entry("default", detail, value("default"));
                }
                if let Err(err) = self.catalog(app) {
                    entry(err, String::new(), None);
                }
                for (id, _) in self.listed(app) {
                    entry(id, family_label(app, id).into(), value(id));
                }
                let detail = "probed before it saves".to_string();
                entry("type an id…", detail, Some(Picked::Typed));
            }
            (Field::Effort, Some(app)) => {
                let model = self.value(pick.row, Field::Model);
                let levels =
                    std::iter::once("default".to_string()).chain(self.efforts(app, &model));
                for level in levels {
                    let detail = if level == "default" { "no flag" } else { "" };
                    entry(
                        &level,
                        detail.to_string(),
                        Some(Picked::Value(level.clone())),
                    );
                }
            }
        }
        out
    }

    /// A job's pick list: its suggestions as the job's App has them, one it
    /// cannot have left out; none; then every other skill that App loads or
    /// has built in. Filtered as entries are.
    fn job_entries(&self, pick: &Pick, j: usize) -> Vec<Entry> {
        let (job, suggestions) = JOBS[j];
        let current = self.manifest.pick(job);
        let filter = pick.filter.to_lowercase();
        let mut out = Vec::new();
        let mut entry = |name: &str, detail: String, mark, picks: Option<Picked>| {
            let shown = match &picks {
                None => filter.is_empty(),
                Some(_) => name.to_lowercase().contains(&filter),
            };
            if shown {
                out.push(Entry {
                    name: name.to_string(),
                    detail,
                    current: matches!(&picks, Some(Picked::Value(v)) if v == current),
                    mark,
                    dim: false,
                    picks,
                });
            }
        };
        let value = |name: &str| Some(Picked::Value(name.to_string()));
        let app = self.app(pick.row);
        let mut listed = vec![NONE.to_string()];
        if let Some(app) = app {
            entry("SUGGESTED", String::new(), None, None);
            let seen = self.seen(app);
            for &(name, source) in suggestions.iter() {
                if name == NONE || (source.is_empty() && !app.built_in.contains(&name)) {
                    continue;
                }
                // Yours through a plugin is picked as plugin:name.
                let named = seen
                    .iter()
                    .map(|(n, _)| *n)
                    .find(|n| n.rsplit(':').next() == Some(name))
                    .unwrap_or(name)
                    .to_string();
                // One you have is used as it is, from whatever source.
                match self.have(app, &named) {
                    Some((mark, color, detail)) => {
                        entry(&named, detail, Some((mark, color)), value(&named))
                    }
                    None => {
                        let detail = parse_source(source)
                            .map_or(String::new(), |s| short(&s.repo).to_string());
                        let mark = Some(("not installed", ORANGE));
                        entry(name, detail, mark, Some(Picked::Install(name, source)));
                    }
                }
                listed.push(named);
            }
        }
        let own = "the Stage skill's own instructions".to_string();
        entry(NONE, own, None, value(NONE));
        let Some(app) = app else {
            return out;
        };
        let names = app
            .built_in
            .iter()
            .copied()
            .chain(self.seen(app).into_iter().map(|(n, _)| n));
        let others: Vec<String> = distinct(names.map(String::from))
            .into_iter()
            .filter(|name| !listed.contains(name))
            .collect();
        if !others.is_empty() {
            let heading = format!("YOUR OTHER SKILLS {} CAN SEE", app.name.to_uppercase());
            entry(&heading, String::new(), None, None);
        }
        for name in others {
            if let Some((mark, color, detail)) = self.have(app, &name) {
                entry(&name, detail, Some((mark, color)), value(&name));
            }
        }
        out
    }

    /// The rule model, picked from pick's list, would break.
    fn mark_of(&self, pick: &Pick, model: &str) -> Option<(&'static str, Color)> {
        let key = ROWS[pick.row].key;
        let mut fields = vec![(pick.field, model.to_string())];
        if let Some(app) = pick.app {
            fields.insert(0, (Field::App, app.name.to_string()));
        }
        let mut doc = self.doc.clone();
        put(&mut doc, key, &fields);
        let broken = broken_by(&self.doc, &doc)?;
        let mark = match (broken.rows, key) {
            // Only the rules' unknown-family texts say "cannot".
            _ if broken.text.contains("cannot") => "? family unknown",
            ([_, "review"], "implement") => "✗ the Review's model",
            ([_, _], "implement") => "✗ the fallback's model",
            (["implement", _], _) => "✗ Implement's model",
            (_, "side_a") => "✗ side B's family",
            _ => "✗ side A's family",
        };
        Some((mark, RED))
    }

    pub(crate) fn choices(&self, pick: &Pick) -> Vec<Picked> {
        self.entries(pick)
            .into_iter()
            .filter_map(|e| e.picks)
            .collect()
    }

    /// The foot's note on a setting, or on its open pick list.
    pub(crate) fn note_of(&self, row: usize, field: Field) -> String {
        let tail = match field {
            Field::Job(j) => return format!(
                "The skill the Stage uses for {}: one you have is used as it is, one not installed is installed first; none leaves the Stage skill's own instructions.",
                job_name(j)
            ),
            Field::App => "Changing the App leads into its model list; the pair saves together.",
            Field::Same => match self.split() {
                Some(plan) => {
                    let model = self.value(row, Field::Model);
                    return format!("Off: plans on {plan}, implements on {model}. Enter or Space turns it on: one model plans and implements.");
                }
                None => return "On: one model plans and implements. Enter or Space turns it off to plan on another model of Implement's family.".to_string(),
            },
            Field::Plan if self.pick.is_some() && self.split().is_none() => {
                return "Pick the plan's model to split planning from implementing; Esc keeps one model for both.".to_string();
            }
            Field::Plan => {
                return "Plans in plan mode, on a model of Implement's family; Implement's own model plans on one model again.".to_string();
            }
            _ => "Default passes no flag.",
        };
        format!("{} {tail}", ROWS[row].note)
    }

    /// The Skills page's rows past the location: the installed skills.
    pub(crate) fn skill_names(&self) -> Vec<String> {
        self.manifest.skills.keys().cloned().collect()
    }

    /// The foot's note on the Skills page's row i: the location, or a skill.
    pub(crate) fn skills_note(&self, i: usize) -> String {
        let names = self.skill_names();
        let Some(name) = i.checked_sub(1).map(|i| &names[i]) else {
            return "Run harness init again to change where skills are installed.".to_string();
        };
        match self.manifest.skills[name].shipped {
            true => format!("{name} is a Shipped skill: harness init installs and updates it, and it cannot be removed."),
            false => format!("u updates {name} from its source, d removes it; a adds a skill from a source, U updates every one."),
        }
    }

    /// The foot's note on the TypeSafe page's row i.
    pub(crate) fn typesafe_note(&self, i: usize) -> String {
        let floor = "Enter types a number from 0 to 1, or nothing for the default, saved at once: the next Judgment reads it.";
        match i {
            0 => "Judges a Wake's next step, a Plan's approval and a Finding the Debate still disputes; Enter turns it on or off.".to_string(),
            1 => format!("Kept in {}, readable only by you; TYPESAFE_API_KEY in the environment wins over it.", setup::KEY_FILE),
            2 => format!("At or above it a Wake's Judgment acts; below it the Wake is a Question. {floor}"),
            _ => format!("A Plan whose covers and in scope reach it, and that asks nothing, is approved; otherwise it is a Question. {floor}"),
        }
    }

    /// A floor as the TypeSafe page shows it: its number, and whether that
    /// is the default; or config.json's value as written, when it is not a
    /// number from 0 to 1.
    pub(crate) fn floor(&self, floor: &Floor) -> Result<(f64, bool), String> {
        let set = &self.doc[floor.key];
        app::floor_in(&self.doc, floor)
            .map(|value| (value, set.is_null() || set == ""))
            .map_err(|_| set.to_string())
    }

    /// Opens the pick list for a row's setting, the cursor on its value.
    fn open_pick(&mut self, row: usize, field: Field, app: Option<&'static App>) {
        let mut pick = Pick {
            row,
            field,
            app,
            cursor: 0,
            filter: String::new(),
        };
        pick.cursor = self
            .entries(&pick)
            .iter()
            .filter(|e| e.picks.is_some())
            .position(|e| e.current)
            .unwrap_or(0);
        self.pick = Some(pick);
    }
}

/// What an App said as it refused a probe: the last line it printed,
/// stdout first (claude -p prints its error there, under a warning on
/// stderr), a JSON error line's message (codex exec's) pulled out.
fn refusal(err: &RunError) -> String {
    let last = |text: &str| {
        text.lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .map(String::from)
    };
    let line = last(&err.stdout)
        .or_else(|| last(&err.stderr))
        .unwrap_or_else(|| err.status.clone());
    let json = line
        .find('{')
        .and_then(|i| serde_json::from_str::<Value>(&line[i..]).ok());
    let message = json.and_then(|v| {
        v["error"]["message"]
            .as_str()
            .or(v["message"].as_str())
            .map(String::from)
    });
    message.unwrap_or(line).trim_end_matches('.').to_string()
}

/// config.json read afresh: as read, with a row's fields put in, and the
/// row as the Stage that starts on it will read it.
fn staged(
    repo: &Path,
    key: &str,
    fields: &[(Field, String)],
) -> Result<(PathBuf, Value, Value, Row), String> {
    let (path, read) = app::read_object(repo)?;
    let mut doc = read.clone();
    put(&mut doc, key, fields);
    let row = app::row_in(&doc, key, &path)?;
    if let Some(broken) = broken_by(&read, &doc) {
        return Err(broken.text);
    }
    Ok((path, read, doc, row))
}

/// The first rule the change from before to after breaks: one broken after
/// that held before, or was not checked, so a config.json broken by hand
/// mends one rule at a time.
fn broken_by(before: &Value, after: &Value) -> Option<Check> {
    let was = app::checks(before);
    let was_broken = |c: &Check| was.iter().any(|w| w.rows == c.rows && !w.holds);
    app::checks(after)
        .into_iter()
        .find(|c| !c.holds && !was_broken(c))
}

/// doc with a row's fields put in. On Implement a new App, or a plan model
/// that is default or Implement's own, plans on one model again, so an App
/// change never strands the plan; a split names both halves by full id, as
/// opusplan's remap needs. A field not a string is left for row_in to refuse.
pub(crate) fn put(doc: &mut Value, key: &str, fields: &[(Field, String)]) {
    if !doc[key].is_object() {
        doc[key] = json!({});
    }
    for (field, value) in fields {
        doc[key][field.key()] = json!(value);
    }
    if key != "implement" {
        return;
    }
    let (Ok(model), Ok(plan)) = (
        app::field(doc, key, Field::Model.key()),
        app::field(doc, key, Field::Plan.key()),
    ) else {
        return;
    };
    let row = doc[key].as_object_mut().unwrap();
    let new_app = fields.iter().any(|(field, _)| *field == Field::App);
    if new_app || plan == "default" || app::canonical(&plan) == app::canonical(&model) {
        row.remove(Field::Plan.key());
    } else {
        row.insert(Field::Model.key().into(), json!(app::full_id(&model)));
        row.insert(Field::Plan.key().into(), json!(app::full_id(&plan)));
    }
}

impl Screen {
    /// /config: reads config.json, each App's models, the Skill manifest
    /// and the skills you have. An unreadable config.json, or one not an
    /// object, is a notice: nothing may save over it. A garbled manifest
    /// shows no skills and says why; each change to it reads it afresh and
    /// refuses.
    // ponytail: the lists, `which` and claude plugin list run on the screen
    // thread (codex's bundled catalog takes ~10 ms); a thread when one is slow.
    pub(super) fn open_config(&mut self) {
        let repo = &self.cfg.repo;
        let doc = match app::read_object(repo) {
            Ok((_, doc)) => doc,
            Err(err) => return self.notice(&err, NOTICE_WINDOW),
        };
        let (manifest, note) = match Manifest::load(repo) {
            Ok(manifest) => (manifest, None),
            Err(err) => (Manifest::default(), Some((err, RED))),
        };
        let tools = &*self.cfg.tools;
        let installed = APPS
            .iter()
            .map(|a| {
                tools.run(repo, &["which", a.bin]).ok()?;
                let version = tools.run(repo, &[a.bin, "--version"]).unwrap_or_default();
                Some(
                    version
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                )
            })
            .collect();
        self.settings = Some(Settings {
            doc,
            models: APPS.iter().map(|a| (a.models)(tools, repo)).collect(),
            installed,
            manifest,
            found: found(repo, &self.cfg.home, tools),
            section: 0,
            open: false,
            setting: 0,
            pick: None,
            typing: None,
            probe: None,
            probed: Vec::new(),
            busy: None,
            listing: None,
            confirm: None,
            note,
            saved: None,
        });
    }

    /// A key while /config is open. A probe or a clone takes none but Esc,
    /// which stops waiting on it; a question takes y or n; typing takes the
    /// line; a checklist or a pick list moves and picks; the left list and a
    /// page move and open, Esc going back and then closing.
    pub(super) fn config_key(&mut self, code: KeyCode, held: bool) {
        let st = self.settings.as_mut().unwrap();
        st.note = None;
        if st.probe.is_some() {
            if code == KeyCode::Esc {
                st.probe = None;
                st.note = Some(("Probe dropped: nothing changed.".to_string(), MUTED));
            }
            return;
        }
        // ponytail: no key stops waiting on a clone, which saves the manifest
        // when done and would overwrite a change made meanwhile; a clone that
        // hangs holds /config until git gives up (Ctrl-C still leaves).
        if st.busy.is_some() {
            return;
        }
        if st.confirm.is_some() {
            match code {
                KeyCode::Char('y') => {
                    let (_, confirm) = st.confirm.take().unwrap();
                    match confirm {
                        Confirm::Remove(name) => self.remove_skill(&name),
                        Confirm::TypeSafeOff => self.typesafe_to(false),
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => st.confirm = None,
                _ => {}
            }
            return;
        }
        if let Some((_, text)) = &mut st.typing {
            match code {
                KeyCode::Esc => st.typing = None,
                KeyCode::Backspace => _ = text.pop(),
                KeyCode::Char(c) if !held => text.push(c),
                KeyCode::Enter => {
                    let (typing, text) = st.typing.take().unwrap();
                    match (typing, text.trim()) {
                        (Typing::Floor(floor), text) => self.keep_floor(floor, text.to_string()),
                        (_, "") => {
                            st.note = Some(("Nothing typed: nothing changed.".to_string(), MUTED))
                        }
                        (Typing::Model(pick), id) => self.pick_model(&pick, id),
                        (Typing::Source, source) => self.add_source(source.to_string()),
                        (Typing::Key, key) => self.keep_key(key.to_string()),
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some(listing) = &mut st.listing {
            let n = listing.names.len();
            match code {
                KeyCode::Up => listing.cursor = listing.cursor.saturating_sub(1),
                KeyCode::Down => listing.cursor = (listing.cursor + 1).min(n - 1),
                KeyCode::Char(' ') => {
                    let (_, installed, ticked) = &mut listing.names[listing.cursor];
                    *ticked = *installed || !*ticked;
                }
                KeyCode::Enter => self.install_ticked(),
                KeyCode::Esc => st.listing = None,
                _ => {}
            }
            return;
        }
        if let Some(pick) = &st.pick {
            let n = st.choices(pick).len();
            let pick = st.pick.as_mut().unwrap();
            match code {
                KeyCode::Up => pick.cursor = pick.cursor.saturating_sub(1),
                KeyCode::Down => pick.cursor = (pick.cursor + 1).min(n.saturating_sub(1)),
                KeyCode::Esc => st.pick = None,
                KeyCode::Backspace => {
                    pick.filter.pop();
                    pick.cursor = 0;
                }
                KeyCode::Char(c) if !held => {
                    pick.filter.push(c);
                    pick.cursor = 0;
                }
                KeyCode::Enter => {
                    let pick = st.pick.take().unwrap();
                    match st.choices(&pick).into_iter().nth(pick.cursor) {
                        Some(picked) => self.choose(pick, picked),
                        None => st.pick = Some(pick),
                    }
                }
                _ => {}
            }
            return;
        }
        if !st.open {
            match code {
                KeyCode::Up => st.section = st.section.saturating_sub(1),
                KeyCode::Down => st.section = (st.section + 1).min(TYPESAFE_PAGE),
                KeyCode::Right | KeyCode::Enter => {
                    st.open = true;
                    st.setting = 0;
                }
                KeyCode::Esc => self.settings = None,
                _ => {}
            }
            return;
        }
        if st.section == APPS_PAGE {
            match code {
                KeyCode::Up => st.setting = st.setting.saturating_sub(1),
                KeyCode::Down => st.setting = (st.setting + 1).min(APPS.len() - 1),
                KeyCode::Left | KeyCode::Esc => st.open = false,
                _ => {}
            }
            return;
        }
        if st.section == SKILLS_PAGE {
            let names = st.skill_names();
            let skill = st.setting.checked_sub(1).map(|i| names[i].clone());
            match code {
                KeyCode::Up => st.setting = st.setting.saturating_sub(1),
                KeyCode::Down => st.setting = (st.setting + 1).min(names.len()),
                KeyCode::Left | KeyCode::Esc => st.open = false,
                KeyCode::Char('a') => st.typing = Some((Typing::Source, String::new())),
                KeyCode::Char('U') => self.update_skills(None),
                KeyCode::Char('u') if skill.is_some() => self.update_skills(skill),
                KeyCode::Char('d') | KeyCode::Delete if skill.is_some() => {
                    self.ask_remove(skill.unwrap())
                }
                _ => {}
            }
            return;
        }
        if st.section == TYPESAFE_PAGE {
            let key = self.cfg.api_key.clone();
            match code {
                KeyCode::Up => st.setting = st.setting.saturating_sub(1),
                KeyCode::Down => st.setting = (st.setting + 1).min(1 + FLOORS.len()),
                KeyCode::Left | KeyCode::Esc => st.open = false,
                KeyCode::Enter if st.setting >= 2 => {
                    st.typing = Some((Typing::Floor(FLOORS[st.setting - 2]), String::new()))
                }
                KeyCode::Enter if key.is_empty() => st.typing = Some((Typing::Key, String::new())),
                KeyCode::Enter if st.setting == 1 => {}
                KeyCode::Enter if st.typesafe(&key) => {
                    st.confirm = Some((TYPESAFE_OFF.to_string(), Confirm::TypeSafeOff))
                }
                KeyCode::Enter => self.typesafe_to(true),
                _ => {}
            }
            return;
        }
        let items = st.items();
        match code {
            KeyCode::Up => st.setting = st.setting.saturating_sub(1),
            KeyCode::Down => st.setting = (st.setting + 1).min(items.len() - 1),
            KeyCode::Left | KeyCode::Esc => st.open = false,
            KeyCode::Enter | KeyCode::Char(' ') if items[st.setting].1 == Field::Same => {
                self.toggle()
            }
            KeyCode::Enter => {
                let (row, field) = items[st.setting];
                match st.app(row) {
                    Some(app) if field == Field::Effort && app.effort.is_empty() => {
                        let text = format!("{} has no effort flag.", app.name);
                        st.note = Some((text, MUTED));
                    }
                    _ => st.open_pick(row, field, None),
                }
            }
            _ => {}
        }
    }

    /// 'Same model for plan and implementation': on, a split turns it off
    /// through the plan's model list, refused off claude (opusplan) or with
    /// Implement's model at default; off, turning it on saves at once.
    fn toggle(&mut self) {
        let st = self.settings.as_mut().unwrap();
        if st.split().is_some() {
            return self.change(0, vec![(Field::Plan, "default".to_string())]);
        }
        let refused = match st.app(0) {
            Some(app) if app.name != "claude" => format!(
                "Refused: {} cannot plan on one model and implement on another: only claude splits, through opusplan. Nothing changed.",
                app.name
            ),
            _ if st.value(0, Field::Model) == "default" => {
                "Pick Implement's model first: the split needs a named model for each half."
                    .to_string()
            }
            _ => return st.open_pick(0, Field::Plan, None),
        };
        st.note = Some((refused, RED));
    }

    /// A pick list's choice: a new App leads into its model list, one not
    /// on PATH says where to get it, one a row cannot run on is refused; a
    /// model or an effort changes the row.
    fn choose(&mut self, pick: Pick, picked: Picked) {
        let st = self.settings.as_mut().unwrap();
        match picked {
            Picked::App(a) if !st.is_installed(a) => st.note = Some((not_on_path(a), MUTED)),
            Picked::App(a) if st.app(pick.row).is_some_and(|now| now.name == a.name) => {}
            Picked::App(a) => match app::runs_on(ROWS[pick.row].key, a) {
                Ok(()) => st.open_pick(pick.row, Field::Model, Some(a)),
                Err(err) => st.note = Some((format!("Refused: {err}. Nothing changed."), RED)),
            },
            Picked::Typed => st.typing = Some((Typing::Model(pick), String::new())),
            Picked::Install(name, source) => {
                if let Field::Job(j) = pick.field {
                    self.install_for(j, name, source)
                }
            }
            Picked::Value(value) => match pick.field {
                Field::Job(j) => {
                    self.set_pick(j, &value);
                }
                Field::Model | Field::Plan => self.pick_model(&pick, &value),
                _ => self.change(pick.row, vec![(Field::Effort, value)]),
            },
        }
    }

    /// A model picked or typed: with the new App it came after, the pair;
    /// the effort back to default on a new App, or when the model does not
    /// list it.
    fn pick_model(&mut self, pick: &Pick, model: &str) {
        let st = self.settings.as_ref().unwrap();
        let mut fields = vec![(pick.field, model.to_string())];
        if let Some(app) = st.pick_app(pick).filter(|_| pick.field == Field::Model) {
            if pick.app.is_some() {
                fields.insert(0, (Field::App, app.name.to_string()));
            }
            let effort = st.value(pick.row, Field::Effort);
            if effort != "default"
                && (pick.app.is_some() || !st.efforts(app, model).contains(&effort))
            {
                fields.push((Field::Effort, "default".to_string()));
            }
        }
        self.change(pick.row, fields);
    }

    /// A change to a row: refused if the Stage could not start on it; one
    /// that changes nothing is dropped; a named model is probed first, once
    /// per App while /config is open, anything else saves at once.
    // ponytail: no timeout on the probe: Esc drops one that hangs, its
    // process running on to its end (it has no stdin to wait on).
    fn change(&mut self, row: usize, fields: Vec<(Field, String)>) {
        let repo = self.cfg.repo.clone();
        let tools = self.cfg.tools.clone();
        let st = self.settings.as_mut().unwrap();
        let key = ROWS[row].key;
        let (now, doc, staged) = match staged(&repo, key, &fields) {
            Ok((_, now, doc, staged)) => (now, doc, staged),
            Err(err) => {
                st.note = Some((format!("Refused: {err}. Nothing changed."), RED));
                return;
            }
        };
        let same =
            |(field, v): &(Field, String)| app::field(&now, key, field.key()).as_ref() == Ok(v);
        if doc == now || fields.iter().all(same) {
            return;
        }
        // The model picked: a plan model, or the row's model.
        let has = |f: Field| fields.iter().any(|(field, _)| *field == f);
        let picked = if has(Field::Plan) {
            staged.plan_model
        } else {
            has(Field::Model).then_some(staged.model)
        };
        let Some(model) = picked.filter(|m| m != "default" && m != "none") else {
            return self.save(row, &fields);
        };
        let app = staged.app;
        if st.probed.contains(&(app.name, model.clone())) {
            return self.save(row, &fields);
        }
        let argv = app::probe(app, &repo, &model);
        let (tx, result) = mpsc::channel();
        thread::spawn(move || {
            let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
            let _ = tx.send(tools.run(&repo, &argv));
        });
        st.probe = Some(Probe {
            result,
            row,
            fields,
            app: app.name,
            model,
        });
    }

    /// The probe's answer, taken in poll(): the change saves, or the App's
    /// error shows and the old value stays.
    pub(super) fn probed(&mut self) {
        let Some(st) = &mut self.settings else {
            return;
        };
        let Some(Ok(result)) = st.probe.as_ref().map(|p| p.result.try_recv()) else {
            return;
        };
        let probe = st.probe.take().unwrap();
        match result {
            Ok(_) => {
                st.probed.push((probe.app, probe.model));
                self.save(probe.row, &probe.fields);
            }
            Err(err) => {
                let why = refusal(&err);
                let text = format!(
                    "{} refused {}: {why}. Nothing changed.",
                    probe.app, probe.model
                );
                st.note = Some((text, RED));
            }
        }
    }

    /// A change saved outside config.json's rows: the foot says it, and
    /// during a run RECENT and the log.
    fn done(&mut self, text: String) {
        let live = self.run.is_some();
        let st = self.settings.as_mut().unwrap();
        st.saved = Some(chrono::Local::now().format("%H:%M:%S").to_string());
        st.note = Some((text.clone(), GREEN));
        if live {
            self.say(&format!("config: {text}"));
        }
    }

    fn refused(&mut self, err: &str) {
        let st = self.settings.as_mut().unwrap();
        st.note = Some((format!("{err}. Nothing changed."), RED));
    }

    /// TypeSafe on or off in config.json, which /config reads again.
    fn typesafe_to(&mut self, on: bool) {
        let repo = &self.cfg.repo;
        match app::set_typesafe(repo, on).and_then(|()| app::read_object(repo)) {
            Ok((_, doc)) => {
                self.settings.as_mut().unwrap().doc = doc;
                self.done(format!("TypeSafe {}", if on { "on" } else { "off" }));
            }
            Err(err) => self.refused(&err),
        }
    }

    /// A floor typed: a number from 0 to 1 saves at once, nothing puts the
    /// default back, and the next Judgment reads it; anything else is
    /// refused, the text kept to mend.
    fn keep_floor(&mut self, floor: &'static Floor, text: String) {
        let st = self.settings.as_mut().unwrap();
        let value = match text.parse::<f64>() {
            _ if text.is_empty() => None,
            Ok(value) if (0.0..=1.0).contains(&value) => Some(value),
            _ => {
                let refused =
                    format!("Refused: {text} is not a number from 0 to 1. Nothing changed.");
                st.note = Some((refused, RED));
                st.typing = Some((Typing::Floor(floor), text));
                return;
            }
        };
        let repo = &self.cfg.repo;
        match app::set_floor(repo, floor, value).and_then(|()| app::read_object(repo)) {
            Ok((_, doc)) => {
                self.settings.as_mut().unwrap().doc = doc;
                let name = floor_name(floor);
                self.done(match value {
                    Some(value) => format!("{name} {value:.2}"),
                    None => format!("{name} {:.2}, its default", floor.default),
                });
            }
            Err(err) => self.refused(&err),
        }
    }

    /// A key typed while there was none: kept as init keeps it, and
    /// TypeSafe on.
    // ponytail: a live run keeps the Config it started with, so its
    // Judgments get the key from the next run on.
    fn keep_key(&mut self, key: String) {
        match setup::keep_key(&self.cfg.repo, &key) {
            Ok(()) => {
                self.cfg.api_key = key;
                self.typesafe_to(true);
            }
            Err(err) => self.refused(&format!("{}: {err}", setup::KEY_FILE)),
        }
    }

    /// Runs work on its own thread, then lists the skills you have there
    /// too; said in the foot until finished() takes its answer.
    fn off_thread(
        &mut self,
        text: String,
        work: impl FnOnce(&Path, &Path, &dyn Tools) -> Done + Send + 'static,
    ) {
        let (repo, home) = (self.cfg.repo.clone(), self.cfg.home.clone());
        let tools = self.cfg.tools.clone();
        let (tx, done) = mpsc::channel();
        thread::spawn(move || {
            let done = work(&repo, &home, &*tools);
            let _ = tx.send((done, found(&repo, &home, &*tools)));
        });
        self.settings.as_mut().unwrap().busy = Some(Busy { text, done });
    }

    /// A pasted source: a bare name refused at once, the text kept to mend;
    /// anything else cloned.
    fn add_source(&mut self, text: String) {
        let st = self.settings.as_mut().unwrap();
        let source = match parse_source(&text) {
            Ok(source) => source,
            Err(err) => {
                st.note = Some((err, RED));
                st.typing = Some((Typing::Source, text));
                return;
            }
        };
        let said = format!("cloning {}…", short(&source.repo));
        self.off_thread(said, move |repo, home, tools| {
            let added = manifest::add(repo, home, tools, &text, None);
            Done::Added(text, added)
        });
    }

    /// The checklist's ticked skills not installed yet, each installed.
    // ponytail: a clone per skill ticked; one clone for them all when a
    // pack's size makes that slow.
    fn install_ticked(&mut self) {
        let st = self.settings.as_mut().unwrap();
        let listing = st.listing.as_ref().unwrap();
        let names: Vec<String> = listing
            .names
            .iter()
            .filter(|(_, installed, ticked)| *ticked && !installed)
            .map(|(name, _, _)| name.clone())
            .collect();
        if names.is_empty() {
            st.note = Some(("Space ticks a skill to install.".to_string(), MUTED));
            return;
        }
        let source = st.listing.take().unwrap().source;
        let said = format!("cloning {source} for {}…", names.join(", "));
        self.off_thread(said, move |repo, home, tools| {
            let added = names.into_iter().map(|name| {
                let added = manifest::add(repo, home, tools, &source, Some(&name));
                (name, added)
            });
            Done::Ticked(added.collect())
        });
    }

    /// A job's suggestion not installed: installed, then picked.
    fn install_for(&mut self, j: usize, name: &'static str, source: &'static str) {
        let from = parse_source(source).map_or(source.to_string(), |s| short(&s.repo).into());
        self.off_thread(
            format!("cloning {from} for {name}…"),
            move |repo, home, tools| {
                Done::ForJob(j, manifest::add(repo, home, tools, source, Some(name)))
            },
        );
    }

    /// A job's pick, saved in the Skill manifest read afresh; the pick it
    /// has changes nothing. Whether it saved.
    fn set_pick(&mut self, j: usize, name: &str) -> bool {
        let repo = &self.cfg.repo;
        let mut manifest = match Manifest::load(repo) {
            Ok(manifest) => manifest,
            Err(err) => {
                self.refused(&err);
                return false;
            }
        };
        if manifest.pick(JOBS[j].0) == name {
            return false;
        }
        manifest
            .picks
            .insert(JOBS[j].0.to_string(), name.to_string());
        if let Err(err) = manifest.save(repo) {
            self.refused(&err);
            return false;
        }
        self.settings.as_mut().unwrap().manifest = manifest;
        self.done(format!("{} picks {name}", job_said(j)));
        true
    }

    /// u on a skill, or U: fetched again off the screen thread. A Shipped
    /// skill is init's to update.
    fn update_skills(&mut self, one: Option<String>) {
        let st = self.settings.as_mut().unwrap();
        let said = match &one {
            Some(name) if st.manifest.skills[name].shipped => {
                let text = format!("{name} is a Shipped skill: harness init updates it.");
                st.note = Some((text, RED));
                return;
            }
            Some(name) => format!(
                "fetching {} for {name}…",
                short(&st.manifest.skills[name].repo)
            ),
            None => "fetching every skill's source…".to_string(),
        };
        self.off_thread(said, move |repo, home, tools| {
            let failed = match &one {
                Some(name) => manifest::update(repo, home, tools, name).map(|()| Vec::new()),
                None => manifest::update_all(repo, home, tools),
            };
            Done::Updated(one, failed)
        });
    }

    /// d on a skill: asked first, naming the jobs it would leave on none. A
    /// Shipped skill is never removed.
    fn ask_remove(&mut self, name: String) {
        let st = self.settings.as_mut().unwrap();
        let skill = &st.manifest.skills[&name];
        if skill.shipped {
            let text = format!("{name} is a Shipped skill: it cannot be removed.");
            st.note = Some((text, RED));
            return;
        }
        let jobs = st.jobs_using(&name);
        let those = if jobs.len() == 1 {
            "that job"
        } else {
            "those jobs"
        };
        let text = match jobs.len() {
            0 => format!("Remove {name} ({})?", short(&skill.repo)),
            _ => format!(
                "{name} is the Delegate skill for {}. Remove it and set {those} to none?",
                jobs.join(", ")
            ),
        };
        st.confirm = Some((text, Confirm::Remove(name)));
    }

    /// A yes to removing a skill: no clone, so on the screen thread.
    fn remove_skill(&mut self, name: &str) {
        let jobs = self.settings.as_ref().unwrap().jobs_using(name);
        if let Err(err) = manifest::remove(&self.cfg.repo, &self.cfg.home, name) {
            return self.refused(&err);
        }
        let found = found(&self.cfg.repo, &self.cfg.home, &*self.cfg.tools);
        self.reload_skills(found);
        let is = if jobs.len() == 1 { "is" } else { "are" };
        let text = match jobs.len() {
            0 => format!("removed {name}"),
            _ => format!("removed {name}; {} {is} none", jobs.join(", ")),
        };
        self.done(text);
    }

    /// The Skill manifest read again after a change, with the skills you
    /// have found since.
    fn reload_skills(&mut self, found: Vec<(String, PathBuf)>) {
        let st = self.settings.as_mut().unwrap();
        if let Ok(manifest) = Manifest::load(&self.cfg.repo) {
            st.manifest = manifest;
        }
        st.found = found;
        if st.section == SKILLS_PAGE {
            st.setting = st.setting.min(st.manifest.skills.len());
        }
    }

    /// "installed tdd from mattpocock/skills @ abc1234".
    fn installed(&self, name: &str) -> String {
        let st = self.settings.as_ref().unwrap();
        match st.manifest.skills.get(name) {
            Some(skill) => format!(
                "installed {name} from {} @ {}",
                short(&skill.repo),
                short_commit(&skill.commit)
            ),
            None => format!("installed {name}"),
        }
    }

    /// A clone's answer, taken in poll(): the skills read again, and what it
    /// did said in the foot.
    pub(super) fn finished(&mut self) {
        let Some(st) = &mut self.settings else {
            return;
        };
        let (done, found) = match st.busy.as_ref().map(|b| b.done.try_recv()) {
            Some(Ok(got)) => got,
            Some(Err(TryRecvError::Disconnected)) => {
                st.busy = None;
                let text =
                    "The clone's thread died: /config shows what it installed when it opens again.";
                st.note = Some((text.to_string(), RED));
                return;
            }
            _ => return,
        };
        st.busy = None;
        let before = st.manifest.skills.clone();
        self.reload_skills(found);
        let st = self.settings.as_mut().unwrap();
        match done {
            Done::Added(_, Err(err)) | Done::ForJob(_, Err(err)) => self.refused(&err),
            Done::Added(_, Ok(Added::Installed(name))) => self.done(self.installed(&name)),
            Done::Added(source, Ok(Added::Choose(names))) => {
                let repo = parse_source(&source).map(|s| s.repo).unwrap_or_default();
                let names = names
                    .into_iter()
                    .map(|name| {
                        let here = st
                            .manifest
                            .skills
                            .get(&name)
                            .is_some_and(|s| s.repo == repo);
                        (name, here, here)
                    })
                    .collect();
                st.listing = Some(Listing {
                    source,
                    names,
                    cursor: 0,
                });
            }
            Done::ForJob(j, Ok(added)) => {
                let Added::Installed(name) = added else {
                    return;
                };
                let installed = self.installed(&name);
                if self.set_pick(j, &name) {
                    self.done(format!("{installed}; {} uses it", job_said(j)));
                } else {
                    self.done(installed);
                }
            }
            Done::Ticked(added) => {
                let mut said = Vec::new();
                let mut failed = false;
                for (name, added) in added {
                    match added {
                        Ok(_) => said.push(self.installed(&name)),
                        Err(err) => {
                            failed = true;
                            said.push(format!("{name} not installed: {err}"));
                        }
                    }
                }
                match failed {
                    false => self.done(said.join("; ")),
                    true => self.refused(&said.join("; ")),
                }
            }
            Done::Updated(_, Err(err)) => self.refused(&err),
            Done::Updated(one, Ok(failed)) => {
                let changed: Vec<String> = st
                    .manifest
                    .skills
                    .iter()
                    .filter_map(|(name, skill)| {
                        let old = &before.get(name)?.commit;
                        (*old != skill.commit).then(|| {
                            format!(
                                "{name} {} → {}",
                                short_commit(old),
                                short_commit(&skill.commit)
                            )
                        })
                    })
                    .collect();
                let head = match (changed.is_empty(), &one) {
                    (false, _) => format!("updated {}", changed.join(", ")),
                    (true, Some(name)) => format!("{name} is up to date"),
                    (true, None) => "every skill is up to date".to_string(),
                };
                let said = std::iter::once(head)
                    .chain(
                        failed
                            .iter()
                            .map(|(name, why)| format!("{name} not updated: {why}")),
                    )
                    .collect::<Vec<_>>()
                    .join("; ");
                match (failed.is_empty(), changed.is_empty()) {
                    (true, true) => st.note = Some((said, GREEN)),
                    (true, false) => self.done(said),
                    _ => self.refused(&said),
                }
            }
        }
    }

    /// Writes the change into config.json, read afresh; during a run RECENT
    /// and the log say what changed.
    fn save(&mut self, row: usize, fields: &[(Field, String)]) {
        let saved = staged(&self.cfg.repo, ROWS[row].key, fields)
            .and_then(|(path, old, doc, _)| app::write(&path, &doc).map(|()| (old, doc)));
        let live = self.run.is_some();
        let st = self.settings.as_mut().unwrap();
        let (old, doc) = match saved {
            Ok(docs) => docs,
            Err(err) => {
                st.note = Some((format!("{err}. Nothing changed."), RED));
                return;
            }
        };
        // as it was on disk, a hand edit since /config opened included
        let (old, new) = (said(&old, row), said(&doc, row));
        st.doc = doc;
        st.saved = Some(chrono::Local::now().format("%H:%M:%S").to_string());
        let name = ROWS[row].name;
        let text = match live {
            true => format!(
                "saved: {name} {new}. Stages that start from now use it; running ones keep theirs."
            ),
            false => format!("saved: {name} {new}, in .harness/config.json"),
        };
        st.note = Some((text, GREEN));
        if live {
            self.say(&format!("config: {name} {old} → {new}"));
        }
    }
}
