//! The Shell: the full-terminal screen that `harness` alone opens (ADR 0004).
//! `Screen` is the plain state the tests drive; `open` wraps it in the
//! terminal and the one draw, poll and tick loop (ADR 0003).

use std::io;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as Input, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::Event;
use crate::orchestrator::state::{load_state, State};
use crate::tools::Tools;

mod draw;
mod logo;

/// The hop and the banner step every 50 ms while a run is live; at rest the
/// screen redraws every 250 ms.
const TICK: Duration = Duration::from_millis(50);
const IDLE_TICK: Duration = Duration::from_millis(250);
/// How long 'press Ctrl-C again to exit' stands.
const CTRL_C_WINDOW: Duration = Duration::from_secs(2);
const NOTICE_WINDOW: Duration = Duration::from_secs(5);

/// An open Epic and its child Tickets, one row each on the idle screen.
pub(crate) struct Epic {
    pub(crate) id: String,
    pub(crate) title: String,
    /// state.json holds a run of this Epic; /continue resumes it.
    pub(crate) resumable: bool,
    pub(crate) tickets: Vec<BdIssue>,
}

/// What the screen shows, with no terminal in it.
pub(crate) struct Screen {
    pub(crate) folder: String,
    pub(crate) version: String,
    /// COLORTERM says 24-bit; otherwise every color is folded to the 256 cube.
    pub(crate) truecolor: bool,
    pub(crate) logo: logo::Logo,
    pub(crate) epics: Vec<Epic>,
    /// The saved run: the Overall bar and the resumable mark come from it.
    pub(crate) state: State,
    /// The panel's lines, oldest first.
    pub(crate) events: Vec<Event>,
    pub(crate) input: String,
    /// The first TICKETS row shown, for a tree taller than its box.
    pub(crate) scroll: usize,
    /// One line above the input, and when it goes.
    pub(crate) notice: Option<(String, Instant)>,
    ctrl_c: Option<Instant>,
    pub(crate) ticks: u64,
    /// A run is live: the logo hops. Never in this Ticket (harness-kqe.10 sets it).
    pub(crate) running: bool,
    pub(crate) quit: bool,
}

impl Screen {
    pub(crate) fn new(folder: String, truecolor: bool, epics: Vec<Epic>, state: State) -> Self {
        Screen {
            folder,
            version: crate::version::version(),
            truecolor,
            logo: logo::Logo::embedded(),
            epics,
            state,
            events: Vec::new(),
            input: String::new(),
            scroll: 0,
            notice: None,
            ctrl_c: None,
            ticks: 0,
            running: false,
            quit: false,
        }
    }

    /// The idle screen for a Target repo: the open Epics from bd and the saved run.
    pub(crate) fn open(repo: &Path, tools: &dyn Tools, env: &dyn Fn(&str) -> String) -> Self {
        let home = env("HOME");
        let folder = match repo.strip_prefix(&home) {
            Ok(rest) if !home.is_empty() => Path::new("~").join(rest).display().to_string(),
            _ => repo.display().to_string(),
        };
        let colorterm = env("COLORTERM");
        let truecolor = colorterm == "truecolor" || colorterm == "24bit";
        let state = load_state(repo).unwrap_or_default();
        let mut notice = None;
        let epics = match load_epics(repo, tools, &state.epic) {
            Ok(epics) => epics,
            Err(err) => {
                notice = Some((
                    format!("bd list failed: {err}"),
                    Instant::now() + NOTICE_WINDOW,
                ));
                Vec::new()
            }
        };
        let mut screen = Screen::new(folder, truecolor, epics, state);
        screen.notice = notice;
        screen
    }

    pub(crate) fn tick(&mut self) {
        self.ticks += 1;
        if self.ctrl_c.is_some_and(|at| at.elapsed() >= CTRL_C_WINDOW) {
            self.ctrl_c = None;
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() >= *until)
        {
            self.notice = None;
        }
    }

    /// An Event from the Orchestrator; only panel lines show.
    pub(crate) fn push(&mut self, event: Event) {
        if event.panel {
            self.events.push(event);
        }
    }

    pub(crate) fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let held = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.ctrl_c.is_some_and(|at| at.elapsed() < CTRL_C_WINDOW) {
                self.quit = true;
            } else {
                self.ctrl_c = Some(Instant::now());
                self.notice("press Ctrl-C again to exit", CTRL_C_WINDOW);
            }
            return;
        }
        let rows = self.rows();
        let scroll = |by: isize| (self.scroll as isize + by).clamp(0, rows as isize - 1) as usize;
        match key.code {
            KeyCode::Char(_) if held => {}
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Down if self.input.is_empty() => self.scroll = scroll(1),
            KeyCode::Up if self.input.is_empty() => self.scroll = scroll(-1),
            KeyCode::PageDown if self.input.is_empty() => self.scroll = scroll(10),
            KeyCode::PageUp if self.input.is_empty() => self.scroll = scroll(-10),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Esc => self.input.clear(),
            KeyCode::Enter => {
                let line = std::mem::take(&mut self.input);
                self.command(line.trim());
            }
            _ => {}
        }
    }

    fn command(&mut self, line: &str) {
        match line {
            "" => {}
            "/exit" => self.quit = true,
            other => self.notice(&format!("unknown command: {other}"), NOTICE_WINDOW),
        }
    }

    fn notice(&mut self, text: &str, span: Duration) {
        self.notice = Some((text.to_string(), Instant::now() + span));
    }

    /// The rows of the TICKETS tree: one per Epic and one per Ticket.
    pub(crate) fn rows(&self) -> usize {
        self.epics.iter().map(|e| e.tickets.len() + 1).sum()
    }

    /// The title of a Ticket on the idle tree, for the RECENT Ticket column.
    pub(crate) fn title(&self, id: &str) -> Option<&str> {
        self.epics
            .iter()
            .flat_map(|e| &e.tickets)
            .find(|t| t.id == id)
            .map(|t| t.title.as_str())
    }
}

/// The child suffix of a bd id: harness-kqe.9 is 9.
pub(crate) fn suffix(id: &str) -> &str {
    id.rsplit_once('.').map_or(id, |(_, s)| s)
}

/// Every open Epic expanded into its Tickets, from one bd list call; the Epic
/// of the saved run is marked resumable.
fn load_epics(repo: &Path, tools: &dyn Tools, saved: &str) -> Result<Vec<Epic>, String> {
    let out = tools
        .run(repo, &["bd", "list", "--json", "--brief", "--all"])
        .map_err(|err| err.to_string())?;
    let issues: Option<Vec<BdIssue>> =
        serde_json::from_str(&out).map_err(|err| format!("unreadable reply: {err}"))?;
    let mut issues = issues.unwrap_or_default();
    let mut epics: Vec<Epic> = issues
        .iter()
        .filter(|i| i.issue_type == "epic" && i.status != "closed")
        .map(|i| Epic {
            id: i.id.clone(),
            title: i.title.clone(),
            resumable: i.id == saved,
            tickets: Vec::new(),
        })
        .collect();
    issues.sort_by_key(|i| suffix(&i.id).parse::<usize>().unwrap_or(usize::MAX));
    for issue in issues {
        if issue.issue_type == "epic" {
            continue;
        }
        if let Some(epic) = epics.iter_mut().find(|e| e.id == issue.parent) {
            epic.tickets.push(issue);
        }
    }
    Ok(epics)
}

/// Opens the Shell over the Target repo and returns when the user exits.
pub(crate) fn open(
    repo: &Path,
    tools: Arc<dyn Tools>,
    env: &dyn Fn(&str) -> String,
) -> io::Result<()> {
    let mut screen = Screen::open(repo, &*tools, env);
    // ponytail: the sender is dropped here; harness-kqe.10 hands it to the
    // Orchestrator's Config.
    let (_events, receiver) = mpsc::channel::<Event>();
    let mut terminal = ratatui::try_init()?;
    let result = run(&mut terminal, &mut screen, &receiver);
    ratatui::restore();
    result
}

/// The screen thread: take the Events, draw, poll for a key until the next
/// tick, tick. A redraw follows every key, Event and tick; at rest the tick
/// is 250 ms, in a live run 50 ms for the hop.
fn run(
    terminal: &mut DefaultTerminal,
    screen: &mut Screen,
    events: &Receiver<Event>,
) -> io::Result<()> {
    let mut last = Instant::now();
    while !screen.quit {
        while let Ok(event) = events.try_recv() {
            screen.push(event);
        }
        terminal.draw(|f| draw::draw(f, screen))?;
        let tick = if screen.running { TICK } else { IDLE_TICK };
        if !event::poll(tick.saturating_sub(last.elapsed()))? {
            screen.tick();
            last = Instant::now();
            continue;
        }
        if let Input::Key(key) = event::read()? {
            screen.key(key);
        }
    }
    Ok(())
}

#[cfg(test)]
mod shell_test;
