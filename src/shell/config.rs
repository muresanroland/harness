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
use crate::orchestrator::app::{self, app, App, Model, Row, APPS, IF_LIMITED};
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

const fn config_row(
    key: &'static str,
    name: &'static str,
    lead: &'static str,
    section: usize,
    note: &'static str,
) -> ConfigRow {
    ConfigRow {
        key,
        name,
        lead,
        section,
        note,
    }
}

pub(crate) const ROWS: [ConfigRow; 8] = [
    config_row(
        "implement",
        "Implement",
        "",
        0,
        "Plans, implements test-first, reviews itself and commits in the Ticket's worktree.",
    ),
    config_row("review", "Review", "", 1, "Reviews the diff into Findings."),
    config_row(
        IF_LIMITED,
        "Review if limited",
        "if limited:",
        1,
        "Runs the Review when the Review's App is Limited and you answer to review with it; none leaves wait or open the PR unreviewed.",
    ),
    config_row(
        "moderator",
        "Moderator",
        "Moderator",
        2,
        "stage-moderate's pane: runs the Debate and settles each Finding.",
    ),
    config_row(
        "side_a",
        "Debate side A",
        "side A",
        2,
        "Argues each Finding, headless; also runs the ponytail audit.",
    ),
    config_row(
        "side_b",
        "Debate side B",
        "side B",
        2,
        "Argues each Finding, headless, on a model from another family than side A.",
    ),
    config_row("fix", "Fix", "", 3, "Fixes the Findings to fix, then opens the PR."),
    config_row(
        "address",
        "Address",
        "",
        4,
        "Resolves a PR's merge conflicts or review comments.",
    ),
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
        "Reviews the diff into Findings, respecting earlier Verdicts. It never runs on the model that implemented.",
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

/// A setting of a row.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Field {
    App,
    Model,
    Effort,
}

impl Field {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Field::App => "app",
            Field::Model => "model",
            Field::Effort => "effort",
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
    doc: Value,
    /// Each App's models, as APPS orders them, read when /config opened.
    models: Vec<Result<Vec<Model>, String>>,
    /// The Apps on PATH, and the skills installed but the Shipped ones.
    pub(crate) installed: usize,
    pub(crate) skills: usize,
    /// The section on the left, and whether the cursor is on its page.
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
    app::field(doc, ROWS[row].key, field.name()).unwrap_or_else(|err| err)
}

/// A row in doc as RECENT's started line names it: "codex gpt-6-sol/high",
/// or "none" for a fallback that is not set.
fn said(doc: &Value, row: usize) -> String {
    let model = value(doc, row, Field::Model);
    match app(&value(doc, row, Field::App)) {
        _ if model == "none" => model,
        None => value(doc, row, Field::App),
        Some(app) => Row {
            app,
            model,
            effort: value(doc, row, Field::Effort),
            plan_model: None,
        }
        .said(),
    }
}

/// The rows of a section.
fn rows_of(section: usize) -> impl Iterator<Item = usize> {
    (0..ROWS.len()).filter(move |&r| ROWS[r].section == section)
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

    /// A section's line on the left: its row's App and model; the Debate's
    /// Apps.
    pub(crate) fn summary(&self, section: usize) -> String {
        if section == 2 {
            return distinct(rows_of(section).map(|r| self.value(r, Field::App))).join("+");
        }
        let row = rows_of(section).next().unwrap();
        match self.value(row, Field::Model).as_str() {
            "default" => self.value(row, Field::App),
            model => format!("{} {model}", self.value(row, Field::App)),
        }
    }

    /// A section's settings, row by row.
    pub(crate) fn items(section: usize) -> Vec<(usize, Field)> {
        rows_of(section)
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
            if shown {
                out.push(Entry {
                    name: name.to_string(),
                    detail,
                    current: picks.is_some() && name == current,
                    picks,
                });
            }
        };
        // A row on an App no longer in the table still lists the Apps.
        let app = match (pick.field, self.pick_app(pick)) {
            (Field::App, _) => {
                for a in &APPS {
                    entry(a.name, a.family.to_string(), Some(Picked::App(a)));
                }
                return out;
            }
            (_, None) => return out,
            (_, Some(app)) => app,
        };
        match pick.field {
            Field::App => {}
            Field::Model => {
                let value = |m: &str| Some(Picked::Value(m.to_string()));
                if ROWS[pick.row].key == IF_LIMITED {
                    let detail = "no fallback".to_string();
                    entry("none", detail, value("none"));
                }
                let detail = format!("{}'s own · {}", app.name, app.family);
                entry("default", detail, value("default"));
                if let Err(err) = self.catalog(app) {
                    entry(err, String::new(), None);
                }
                for (id, _) in self.listed(app) {
                    entry(id, app.family.to_string(), value(id));
                }
                let detail = "probed before it saves".to_string();
                entry("type an id…", detail, Some(Picked::Typed));
            }
            Field::Effort => {
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

    pub(crate) fn choices(&self, pick: &Pick) -> Vec<Picked> {
        self.entries(pick)
            .into_iter()
            .filter_map(|e| e.picks)
            .collect()
    }

    /// The foot's note on a setting.
    pub(crate) fn note_of(row: usize, field: Field) -> String {
        let tail = match field {
            Field::App => "Changing the App leads into its model list; the pair saves together.",
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
    let (path, read) = app::read(repo)?;
    let mut doc = read.clone();
    if !doc.is_object() {
        doc = json!({});
    }
    if !doc[key].is_object() {
        doc[key] = json!({});
    }
    for (field, value) in fields {
        doc[key][field.name()] = json!(value);
    }
    let row = app::row_in(&doc, key, &path)?;
    Ok((path, read, doc, row))
}

impl Screen {
    /// /config: reads config.json, each App's models and what the summaries
    /// count. An unreadable config.json is a notice: nothing may save over it.
    // ponytail: the lists and `which` run on the screen thread (codex's
    // bundled catalog takes ~10 ms); a thread when an App's listing is slow.
    pub(super) fn open_config(&mut self) {
        let repo = &self.cfg.repo;
        let doc = match app::read(repo) {
            Ok((_, doc)) => doc,
            Err(err) => return self.notice(&err, NOTICE_WINDOW),
        };
        let tools = &*self.cfg.tools;
        let installed = APPS
            .iter()
            .filter(|a| tools.run(repo, &["which", a.name]).is_ok())
            .count();
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
                KeyCode::Down => st.section = (st.section + 1).min(SECTIONS.len() - 1),
                KeyCode::Right | KeyCode::Enter => {
                    st.open = true;
                    st.setting = 0;
                }
                KeyCode::Esc => self.settings = None,
                _ => {}
            }
            return;
        }
        let items = Settings::items(st.section);
        match code {
            KeyCode::Up => st.setting = st.setting.saturating_sub(1),
            KeyCode::Down => st.setting = (st.setting + 1).min(items.len() - 1),
            KeyCode::Left | KeyCode::Esc => st.open = false,
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

    /// A pick list's choice: a new App leads into its model list, one a
    /// row cannot run on refused; a model or an effort changes the row.
    fn choose(&mut self, pick: Pick, picked: Picked) {
        let st = self.settings.as_mut().unwrap();
        match picked {
            Picked::App(a) if st.app(pick.row).is_some_and(|now| now.name == a.name) => {}
            Picked::App(a) => match app::runs_on(ROWS[pick.row].key, a) {
                Ok(()) => st.open_pick(pick.row, Field::Model, Some(a)),
                Err(err) => st.note = Some((format!("Refused: {err}. Nothing changed."), RED)),
            },
            Picked::Typed => st.typing = Some((pick, String::new())),
            Picked::Value(value) if pick.field == Field::Model => self.pick_model(&pick, &value),
            Picked::Value(value) => self.change(pick.row, vec![(Field::Effort, value)]),
        }
    }

    /// A model picked or typed: with the new App it came after, the pair;
    /// the effort back to default when the model does not list it.
    fn pick_model(&mut self, pick: &Pick, model: &str) {
        let st = self.settings.as_ref().unwrap();
        let mut fields = vec![(Field::Model, model.to_string())];
        if let Some(app) = st.pick_app(pick) {
            if pick.app.is_some() {
                fields.insert(0, (Field::App, app.name.to_string()));
            }
            let effort = st.value(pick.row, Field::Effort);
            if effort != "default" && !st.efforts(app, model).contains(&effort) {
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
        let (now, app, model) = match staged(&repo, key, &fields) {
            Ok((_, now, _, staged)) => (now, staged.app, staged.model),
            Err(err) => {
                st.note = Some((format!("Refused: {err}. Nothing changed."), RED));
                return;
            }
        };
        let same =
            |(field, v): &(Field, String)| app::field(&now, key, field.name()).as_ref() == Ok(v);
        if fields.iter().all(same) {
            return;
        }
        let picked = fields.iter().any(|(field, _)| *field == Field::Model);
        if !picked || model == "default" || model == "none" {
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
