//! The Shell: the full-terminal screen that `harness` alone opens (ADR 0004).
//! `Screen` is the plain state the tests drive; `open` wraps it in the
//! terminal and the one draw, poll and tick loop (ADR 0003). The Shell owns
//! the Orchestrator: the scheduler runs on a thread of this process, its
//! Events come over a channel into RECENT and the log, and the TICKETS rows
//! are a snapshot of its State.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as Input, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::{Config, Event, Orchestrator};
use crate::orchestrator::state::{acquire_lock, load_state, Lock, State};
use crate::setup;
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
/// Tickets in the Pipeline at once, without --max.
const DEFAULT_MAX: usize = 3;
/// RECENT keeps this many Events; older ones are in the log.
const KEPT_EVENTS: usize = 1000;

/// An open Epic and its child Tickets, one row each on the TICKETS tree.
pub(crate) struct Epic {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) tickets: Vec<BdIssue>,
}

/// What a run needs besides its Epic: the Target repo, the Tools, the herdr
/// workspace, the TypeSafe key and the clocks.
pub(crate) struct Launch {
    pub(crate) tools: Arc<dyn Tools>,
    pub(crate) repo: PathBuf,
    pub(crate) workspace: String,
    pub(crate) api_key: String,
    pub(crate) home: PathBuf,
    pub(crate) tick: Duration,
    pub(crate) poll_prs: Duration,
}

/// The live run: the Orchestrator, its scheduler thread and the lock, held
/// exactly as long as the run is.
struct Run {
    o: Arc<Orchestrator>,
    scheduler: JoinHandle<Result<(), String>>,
    _lock: Lock,
}

/// A y/n question above the input line, and what yes does.
enum Pending {
    /// Discard the saved run and start this Epic.
    Start { epic: String, max: usize },
    /// Stop the run and exit.
    Exit,
}

/// What the screen shows, with no terminal in it.
pub(crate) struct Screen {
    pub(crate) folder: String,
    pub(crate) version: String,
    /// COLORTERM says 24-bit; otherwise every color is folded to the 256 cube.
    pub(crate) truecolor: bool,
    pub(crate) logo: logo::Logo,
    pub(crate) epics: Vec<Epic>,
    /// The run's State: a snapshot of the live Orchestrator's, or the saved
    /// one; the Overall bar, the TICKETS rows and the resumable mark come
    /// from it.
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
    /// A run is live: the logo hops, the status row spins.
    pub(crate) running: bool,
    pub(crate) quit: bool,
    launch: Launch,
    /// What the preflight found missing at open; a run is refused while
    /// anything is.
    missing: Vec<String>,
    /// The Orchestrator's Events: the Shell holds the receiver, every run's
    /// Config gets the sender.
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    run: Option<Run>,
    pending: Option<Pending>,
}

impl Screen {
    pub(crate) fn new(
        launch: Launch,
        folder: String,
        truecolor: bool,
        epics: Vec<Epic>,
        state: State,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
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
            launch,
            missing: Vec::new(),
            sender,
            receiver,
            run: None,
            pending: None,
        }
    }

    /// The idle screen for a Target repo: the open Epics from bd, the saved
    /// run and the preflight.
    pub(crate) fn open(repo: &Path, tools: Arc<dyn Tools>, env: &dyn Fn(&str) -> String) -> Self {
        let home = env("HOME");
        let folder = match repo.strip_prefix(&home) {
            Ok(rest) if !home.is_empty() => Path::new("~").join(rest).display().to_string(),
            _ => repo.display().to_string(),
        };
        let colorterm = env("COLORTERM");
        let truecolor = colorterm == "truecolor" || colorterm == "24bit";
        let state = load_state(repo).unwrap_or_default();
        let missing = setup::preflight(repo, &*tools, env);
        let launch = Launch {
            tools,
            repo: repo.to_path_buf(),
            workspace: env("HERDR_WORKSPACE_ID"),
            api_key: setup::typesafe_key(repo, env).unwrap_or_default(),
            home: PathBuf::new(), // the Orchestrator reads HOME
            tick: Duration::from_secs(5),
            poll_prs: Duration::from_secs(30),
        };
        let mut screen = Screen::new(launch, folder, truecolor, Vec::new(), state);
        screen.missing = missing;
        screen.reload_epics();
        screen
    }

    /// The bd cache again: on open, on every /start-epic, after a Ticket
    /// closes and when a run ends.
    fn reload_epics(&mut self) {
        match load_epics(&self.launch.repo, &*self.launch.tools) {
            Ok(epics) => self.epics = epics,
            Err(err) => self.notice(&format!("bd list failed: {err}"), NOTICE_WINDOW),
        }
    }

    /// Takes the Events, snapshots the live run's State and notices when the
    /// scheduler thread has returned: the run is over, the lock goes, and a
    /// done Epic clears the saved run.
    pub(crate) fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            self.push(event);
        }
        let Some(run) = &self.run else {
            return;
        };
        self.state = run.o.state.lock().unwrap().clone();
        if !run.scheduler.is_finished() {
            return;
        }
        let run = self.run.take().unwrap();
        self.running = false;
        match run.scheduler.join() {
            Ok(Ok(())) if !run.o.stopping() && !self.state.epic.is_empty() => {
                self.state = State::default(); // Epic done: nothing to resume
                if let Err(err) = self.state.save(&self.launch.repo) {
                    self.notice(&format!("state not saved: {err}"), NOTICE_WINDOW);
                }
            }
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                self.state = load_state(&self.launch.repo).unwrap_or_default();
                self.notice(&err, NOTICE_WINDOW);
            }
            Err(_) => self.notice("the scheduler thread died", NOTICE_WINDOW),
        }
        self.reload_epics();
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

    /// An Event from the Orchestrator; only panel lines show. A closed
    /// Ticket refreshes the tree.
    pub(crate) fn push(&mut self, event: Event) {
        if !event.panel {
            return;
        }
        let closed = event.text == "merged, Ticket closed";
        self.events.push(event);
        if self.events.len() > KEPT_EVENTS {
            self.events.drain(..self.events.len() - KEPT_EVENTS);
        }
        if closed {
            self.reload_epics();
        }
    }

    /// Whether a Ticket of the live run waits on the user: its last panel
    /// line is a Wake, a prompt or a trust dialog.
    // ponytail: read off the last Event rather than kept as state.
    pub(crate) fn blocked(&self, id: &str) -> bool {
        self.events
            .iter()
            .rev()
            .find(|e| e.ticket.as_deref() == Some(id))
            .is_some_and(|e| {
                e.text.starts_with("stuck in ")
                    || e.text.starts_with("waiting at a prompt")
                    || e.text.starts_with("waiting: ")
            })
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
                self.quit();
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
            KeyCode::Tab => self.complete(),
            KeyCode::Enter => {
                let line = std::mem::take(&mut self.input);
                self.command(line.trim());
            }
            _ => {}
        }
    }

    /// One input line: a slash command, or the answer to a pending y/n.
    pub(crate) fn command(&mut self, line: &str) {
        if let Some(pending) = self.pending.take() {
            if matches!(line, "y" | "yes") {
                match pending {
                    Pending::Start { epic, max } => self.start_epic(&epic, max, true),
                    Pending::Exit => self.quit(),
                }
            }
            return;
        }
        let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
        let (query, max) = match parse_args(rest) {
            Ok(parsed) => parsed,
            Err(err) => return self.notice(&err, NOTICE_WINDOW),
        };
        match name {
            "" => {}
            "/start-epic" => {
                self.reload_epics();
                if let Some(id) = self.resolve(&query, true) {
                    self.start_epic(&id, max, false);
                }
            }
            "/start-ticket" => {
                if let Some(id) = self.resolve(&query, false) {
                    self.launch(max, move |o| {
                        o.run_single(&id);
                        Ok(())
                    });
                }
            }
            "/continue" => {
                let epic = self.state.epic.clone();
                if epic.is_empty() {
                    self.notice("no saved run to continue", NOTICE_WINDOW);
                } else {
                    self.launch(max, move |o| o.run(&epic));
                }
            }
            "/stop-work" => self.stop_work(),
            "/retry" | "/park" | "/address" => match &self.run {
                _ if query.is_empty() => {
                    self.notice(&format!("usage: {name} <ticket>"), NOTICE_WINDOW)
                }
                None => self.notice(
                    "refused: no run is live, /start-epic or /continue starts one",
                    NOTICE_WINDOW,
                ),
                Some(run) => run.o.command(&format!("{}-{query}", &name[1..])),
            },
            "/exit" => match &self.run {
                None => self.quit(),
                Some(_) => {
                    self.pending = Some(Pending::Exit);
                    self.notice("stop the run and exit? (y/n)", NOTICE_WINDOW);
                }
            },
            _ => self.notice(&format!("unknown command: {line}"), NOTICE_WINDOW),
        }
    }

    /// Tab on '/start-epic <q>' or '/start-ticket <q>' fills in the one id
    /// that matches, or names the matches.
    fn complete(&mut self) {
        let Some((name, query)) = self.input.split_once(' ') else {
            return;
        };
        let (name, query) = (name.to_string(), query.trim().to_string());
        let epics = match name.as_str() {
            "/start-epic" => true,
            "/start-ticket" => false,
            _ => return,
        };
        if let Some(id) = self.resolve(&query, epics) {
            self.input = format!("{name} {id} ");
        }
    }

    /// The one open Epic (or Ticket) an argument names: its id exactly, or
    /// the only one whose id or title contains it. Anything else is a notice.
    fn resolve(&mut self, query: &str, epics: bool) -> Option<String> {
        let candidates: Vec<(&str, &str)> = if epics {
            self.epics
                .iter()
                .map(|e| (e.id.as_str(), e.title.as_str()))
                .collect()
        } else {
            self.epics
                .iter()
                .flat_map(|e| &e.tickets)
                .map(|t| (t.id.as_str(), t.title.as_str()))
                .collect()
        };
        if let Some((id, _)) = candidates.iter().find(|(id, _)| *id == query) {
            return Some(id.to_string());
        }
        let q = query.to_lowercase();
        let found: Vec<String> = candidates
            .iter()
            .filter(|(id, title)| {
                id.to_lowercase().contains(&q) || title.to_lowercase().contains(&q)
            })
            .map(|(id, title)| format!("{id} {title}"))
            .collect();
        let what = if epics { "open Epic" } else { "Ticket" };
        match found.as_slice() {
            [one] => Some(one.split(' ').next().unwrap().to_string()),
            [] => {
                self.notice(&format!("no {what} matches {query:?}"), NOTICE_WINDOW);
                None
            }
            many => {
                self.notice(&format!("matches: {}", many.join("  ·  ")), NOTICE_WINDOW);
                None
            }
        }
    }

    /// /start-epic: over a different saved Epic it asks before discarding.
    fn start_epic(&mut self, epic: &str, max: usize, discard: bool) {
        if self.run.is_some() {
            return self.notice("refused: a run is live, /stop-work first", NOTICE_WINDOW);
        }
        let saved = self.state.epic.clone();
        if !saved.is_empty() && saved != epic {
            if !discard {
                self.pending = Some(Pending::Start {
                    epic: epic.to_string(),
                    max,
                });
                return self.notice(
                    &format!("discard the saved run on {saved}? (y/n)"),
                    NOTICE_WINDOW,
                );
            }
            self.state = State::default();
            if let Err(err) = self.state.save(&self.launch.repo) {
                return self.notice(&format!("state not saved: {err}"), NOTICE_WINDOW);
            }
        }
        let epic = epic.to_string();
        self.launch(max, move |o| o.run(&epic));
    }

    /// Takes the lock and runs `work` over a fresh Orchestrator on the
    /// scheduler thread; it returns when the run ends and poll sees it.
    fn launch(
        &mut self,
        max: usize,
        work: impl FnOnce(&Arc<Orchestrator>) -> Result<(), String> + Send + 'static,
    ) {
        if self.run.is_some() {
            return self.notice("refused: a run is live, /stop-work first", NOTICE_WINDOW);
        }
        if let Some(missing) = self.missing.first() {
            return self.notice(&format!("refused: {missing}"), NOTICE_WINDOW);
        }
        let repo = &self.launch.repo;
        let lock = match setup::ignore_run_dir(repo).and_then(|()| acquire_lock(repo)) {
            Ok(lock) => lock,
            Err(err) => return self.notice(&err.to_string(), NOTICE_WINDOW),
        };
        let log: Box<dyn io::Write + Send> = match File::options()
            .create(true)
            .append(true)
            .open(repo.join(".harness").join("orchestrator.log"))
        {
            Ok(file) => Box::new(file),
            Err(_) => Box::new(io::sink()),
        };
        let o = match Orchestrator::new(Config {
            tools: self.launch.tools.clone(),
            repo: repo.clone(),
            workspace: self.launch.workspace.clone(),
            api_key: self.launch.api_key.clone(),
            home: self.launch.home.clone(),
            tick: self.launch.tick,
            poll_prs: self.launch.poll_prs,
            max,
            log: Mutex::new(log),
            events: self.sender.clone(),
            #[cfg(test)]
            timeout: None,
        }) {
            Ok(o) => o,
            Err(err) => return self.notice(&err.to_string(), NOTICE_WINDOW),
        };
        let scheduler = {
            let o = o.clone();
            thread::spawn(move || work(&o))
        };
        self.run = Some(Run {
            o,
            scheduler,
            _lock: lock,
        });
        self.running = true;
    }

    /// /stop-work: scheduling ends, live panes stay, the state file holds
    /// every Ticket as saved, and the lock goes when the scheduler returns.
    fn stop_work(&mut self) {
        match &self.run {
            Some(run) => run.o.stop(),
            None => self.notice("nothing is running", NOTICE_WINDOW),
        }
    }

    /// Ends the Shell; a live run is stopped first and its panes stay.
    fn quit(&mut self) {
        if let Some(run) = &self.run {
            run.o.stop();
        }
        self.quit = true;
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

/// '<query words> [--max N]' in any order: the words joined, and N or the default.
fn parse_args(rest: &str) -> Result<(String, usize), String> {
    let mut words = Vec::new();
    let mut max = DEFAULT_MAX;
    let mut args = rest.split_whitespace();
    while let Some(arg) = args.next() {
        let value = match arg.strip_prefix("--max") {
            Some("") => args.next(),
            Some(eq) => eq.strip_prefix('='),
            None => {
                words.push(arg);
                continue;
            }
        };
        max = value
            .and_then(|v| v.parse().ok())
            .filter(|n| *n >= 1)
            .ok_or("--max wants a number of at least 1")?;
    }
    Ok((words.join(" "), max))
}

/// Every open Epic expanded into its Tickets, from one bd list call.
fn load_epics(repo: &Path, tools: &dyn Tools) -> Result<Vec<Epic>, String> {
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
    let mut screen = Screen::open(repo, tools, env);
    let mut terminal = ratatui::try_init()?;
    let result = run(&mut terminal, &mut screen);
    ratatui::restore();
    result
}

/// The screen thread: take the Events and the State, draw, poll for a key
/// until the next tick, tick. A redraw follows every key, Event and tick; at
/// rest the tick is 250 ms, in a live run 50 ms for the hop. It never waits
/// on the Orchestrator: the State mutex is held for a clone, nothing longer.
fn run(terminal: &mut DefaultTerminal, screen: &mut Screen) -> io::Result<()> {
    let mut last = Instant::now();
    while !screen.quit {
        screen.poll();
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

impl Launch {
    /// A Launch over the fake world: every duration a millisecond.
    #[cfg(test)]
    pub(crate) fn for_tests(tools: Arc<dyn Tools>, repo: &Path, home: &Path) -> Self {
        Launch {
            tools,
            repo: repo.to_path_buf(),
            workspace: "w1".to_string(),
            api_key: "sk-test".to_string(),
            home: home.to_path_buf(),
            tick: Duration::from_millis(1),
            poll_prs: Duration::from_millis(1),
        }
    }
}

#[cfg(test)]
mod shell_test;
