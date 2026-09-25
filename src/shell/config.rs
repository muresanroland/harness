//! /config: the App, model and effort of each row of .harness/config.json,
//! picked in a modal docked beside the live Shell (layout C of
//! harness-0sx.9, drawn in draw/config.rs) and saved at once. A Stage reads
//! its row as it starts, so the Stages that start after a change use it and
//! running ones keep theirs. A named model is probed with a one-line prompt
//! first, off the screen thread; it saves only if the App answers.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crossterm::event::KeyCode;
use ratatui::style::Color;
use serde_json::{json, Value};

use super::logo::{GREEN, MUTED, RED};
use super::{Screen, NOTICE_WINDOW};
use crate::orchestrator::app::{self, app, App, Check, Model, Row, APPS, IF_LIMITED};
use crate::skills::manifest::Manifest;
use crate::tools::RunError;

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

/// The Apps page's place on the left, after the Pipeline's sections.
pub(crate) const APPS_PAGE: usize = SECTIONS.len();

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
        }
    }
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
    /// A model or an effort, as the pick's field says.
    Value(String),
    /// 'type an id…'
    Typed,
}

/// A line of a pick list; one that picks nothing is a heading.
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) detail: String,
    pub(crate) current: bool,
    /// The rule a model picked would break: '✗ side A's family'.
    pub(crate) mark: Option<&'static str>,
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

/// /config, open.
pub(crate) struct Settings {
    pub(crate) doc: Value,
    /// Each App's models, as APPS orders them, read when /config opened.
    models: Vec<Result<Vec<Model>, String>>,
    /// Each App, as APPS orders them, on PATH: the first line its
    /// --version prints; None when not on PATH.
    pub(crate) installed: Vec<Option<String>>,
    /// The skills installed but the Shipped ones.
    pub(crate) skills: usize,
    /// The section on the left (APPS_PAGE past them), and whether the
    /// cursor is on its page.
    pub(crate) section: usize,
    pub(crate) open: bool,
    /// The setting under the cursor on the page, of Settings::items.
    pub(crate) setting: usize,
    pub(crate) pick: Option<Pick>,
    /// A model id being typed, for the model list it came from.
    pub(crate) typing: Option<(Pick, String)>,
    pub(crate) probe: Option<Probe>,
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

/// The note on an App not on PATH: where to get it.
fn not_on_path(app: &App) -> String {
    format!("{} is not on PATH: get it at {}", app.name, app.home)
}

/// The rows of a section.
fn rows_of(section: usize) -> impl Iterator<Item = usize> {
    (0..ROWS.len()).filter(move |&r| ROWS[r].section == section)
}

/// The section a check shows on: its last row's.
pub(crate) fn section_of(check: &Check) -> usize {
    let key = check.rows[check.rows.len() - 1];
    ROWS.iter().find(|r| r.key == key).unwrap().section
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
    pub(crate) fn value(&self, row: usize, field: Field) -> String {
        value(&self.doc, row, field)
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

    /// The rules on the rows as they are, those of a section when given.
    pub(crate) fn checks(&self, section: Option<usize>) -> Vec<Check> {
        let mut checks = app::checks(&self.doc);
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
        if self.section == 0 {
            let plan = self.split().map(|_| (0, Field::Plan));
            let head = [(0, Field::App), (0, Field::Same)].into_iter().chain(plan);
            return head
                .chain([(0, Field::Model), (0, Field::Effort)])
                .collect();
        }
        rows_of(self.section)
            .flat_map(|r| [Field::App, Field::Model, Field::Effort].map(|f| (r, f)))
            .collect()
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
                    let detail = match self.is_installed(a) {
                        true => a.family,
                        false => "not installed",
                    };
                    entry(a.name, detail.into(), Some(Picked::App(a)));
                }
            }
            (_, None) | (Field::Same, _) => {}
            (Field::Plan, Some(app)) => {
                for (id, _) in self.listed(app) {
                    entry(id, app.family.into(), Some(Picked::Value(id.clone())));
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
                    let detail = format!("{}'s own · {}", app.name, app.family);
                    entry("default", detail, value("default"));
                }
                if let Err(err) = self.catalog(app) {
                    entry(err, String::new(), None);
                }
                for (id, _) in self.listed(app) {
                    entry(id, app.family.into(), value(id));
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

    /// The rule model, picked from pick's list, would break.
    fn mark_of(&self, pick: &Pick, model: &str) -> Option<&'static str> {
        let key = ROWS[pick.row].key;
        let mut fields = vec![(pick.field, model.to_string())];
        if let Some(app) = pick.app {
            fields.insert(0, (Field::App, app.name.to_string()));
        }
        let mut doc = self.doc.clone();
        put(&mut doc, key, &fields);
        let broken = broken_by(&self.doc, &doc)?;
        let mark = match (broken.rows, key) {
            ([_, "review"], "implement") => "✗ the Review's model",
            ([_, _], "implement") => "✗ the fallback's model",
            (["implement", _], _) => "✗ Implement's model",
            (_, "side_a") => "✗ side B's family",
            _ => "✗ side A's family",
        };
        Some(mark)
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
    /// /config: reads config.json, each App's models and what the summaries
    /// count. An unreadable config.json, or one not an object, is a notice:
    /// nothing may save over it.
    // ponytail: the lists and `which` run on the screen thread (codex's
    // bundled catalog takes ~10 ms); a thread when an App's listing is slow.
    pub(super) fn open_config(&mut self) {
        let repo = &self.cfg.repo;
        let doc = match app::read_object(repo) {
            Ok((_, doc)) => doc,
            Err(err) => return self.notice(&err, NOTICE_WINDOW),
        };
        let tools = &*self.cfg.tools;
        let installed = APPS
            .iter()
            .map(|a| {
                tools.run(repo, &["which", a.name]).ok()?;
                let version = tools.run(repo, &[a.name, "--version"]).unwrap_or_default();
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
        let skills =
            Manifest::load(repo).map_or(0, |m| m.skills.values().filter(|s| !s.shipped).count());
        self.settings = Some(Settings {
            doc,
            models: APPS.iter().map(|a| (a.models)(tools, repo)).collect(),
            installed,
            skills,
            section: 0,
            open: false,
            setting: 0,
            pick: None,
            typing: None,
            probe: None,
            note: None,
            saved: None,
        });
    }

    /// A key while /config is open. A probe takes none but Esc, which drops
    /// it; typing an id takes the line; a pick list filters, moves and
    /// picks; the left list and a page move and open, Esc going back and
    /// then closing.
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
        if let Some((_, text)) = &mut st.typing {
            match code {
                KeyCode::Esc => st.typing = None,
                KeyCode::Backspace => _ = text.pop(),
                KeyCode::Char(c) if !held => text.push(c),
                KeyCode::Enter => {
                    let (pick, text) = st.typing.take().unwrap();
                    match text.trim() {
                        "" => {
                            st.note = Some(("Nothing typed: nothing changed.".to_string(), MUTED))
                        }
                        id => self.pick_model(&pick, id),
                    }
                }
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
                KeyCode::Down => st.section = (st.section + 1).min(APPS_PAGE),
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
            Picked::Typed => st.typing = Some((pick, String::new())),
            Picked::Value(value) if matches!(pick.field, Field::Model | Field::Plan) => {
                self.pick_model(&pick, &value)
            }
            Picked::Value(value) => self.change(pick.row, vec![(Field::Effort, value)]),
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
    /// that changes nothing is dropped; a named model is probed first,
    /// anything else saves at once.
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
            Ok(_) => self.save(probe.row, &probe.fields),
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
