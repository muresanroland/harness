//! The Shell: the full-terminal screen that `harness` alone opens (ADR 0004).
//! `Screen` is the plain state the tests drive; `open` wraps it in the
//! terminal and the one draw, poll and tick loop (ADR 0003). The Shell owns
//! the Orchestrator: the scheduler runs on a thread of this process, its
//! Events come over a channel into RECENT and the log, and the TICKETS rows
//! are a snapshot of its State. A Wake or a blocked session becomes a
//! Question, answered through the Orchestrator's command queue (or, for a
//! nudge, straight to the session through Tools); the Orchestrator knows no
//! Shell type.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as Input, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::orchestrator::herdr::agent_name;
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::{result_name, stage_named, Config, Event, Orchestrator};
use crate::orchestrator::state::{
    acquire_lock, load_state, Lock, State, TicketState, STATUS_PARKED, STATUS_RUNNING,
};
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
/// The canned nudges from docs/design/judgment-prototype, both offered until
/// the Judgment (Ticket 12) picks one; {result_file} is the Stage's.
const NUDGE_RESULT: &str = "The Orchestrator is waiting for your result file {result_file} and cannot read anything else. Write it now, the first line exactly 'STATUS: done' (or 'STATUS: failed' and why), then stop.";
const NUDGE_PROCEED: &str = "Nobody is watching this pane and no one will answer. The Ticket is the spec: decide yourself, note the decision in the result file, carry on to the end, then write {result_file} with 'STATUS: done' as its first line.";

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
/// until the scheduler has returned and every Ticket thread has left.
struct Run {
    o: Arc<Orchestrator>,
    /// None once joined: the run is stopping, its Ticket threads leaving.
    scheduler: Option<JoinHandle<Result<(), String>>>,
    /// An Epic run, which a finished Epic clears from the state file.
    epic: bool,
    /// The scheduler returned an error: the state file is read back.
    failed: bool,
    _lock: Lock,
}

/// What a yes/no confirmation does on yes.
pub(crate) enum Pending {
    /// Discard the saved run and start this Epic or Ticket.
    Start { id: String, max: usize, epic: bool },
    /// Stop the run and exit.
    Exit,
}

/// What a Question offers; the kind decides its options and what an answer does.
pub(crate) enum Kind {
    /// A Wake: nudge with a canned prompt, retry, park, open the pane, a
    /// prompt of your own. The pane tail is read when the Question is raised.
    Wake { tail: String, prompts: [String; 2] },
    /// A blocked session: open the pane, park, "I answered it".
    Blocked,
    /// A yes/no confirmation; it jumps the queue.
    Confirm(Pending),
    /// The /continue checklist: one row per saved Ticket, reset toggled by Space.
    Continue { rows: Vec<(String, bool)> },
}

/// A form above the input line: what the Shell puts to the user when the
/// Orchestrator cannot act alone. One shows at a time, oldest first; a
/// Ticket's Question holds that Ticket alone, and none is ever saved.
pub(crate) struct Question {
    pub(crate) ticket: Option<String>,
    /// The event line it came from, or the confirmation's wording.
    pub(crate) text: String,
    pub(crate) kind: Kind,
    /// The option the cursor is on.
    pub(crate) cursor: usize,
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
    /// The Questions waiting, oldest first; the first shows unless hidden.
    pub(crate) questions: Vec<Question>,
    /// Esc hid the form; /questions or Esc on an empty input line brings it back.
    pub(crate) hidden: bool,
    /// The input line is a prompt of the user's own for the front Question.
    pub(crate) composing: bool,
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
            questions: Vec::new(),
            hidden: false,
            composing: false,
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

    /// Takes the Events and snapshots the live run's State. Once the
    /// scheduler thread has returned the run is stopping until every Ticket
    /// thread has left (each sees stop at its next sleep, and still saves
    /// state after its current Tools call); then the run is over, the lock
    /// goes, a stopped run says so, and a done Epic clears the saved run.
    pub(crate) fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            self.push(event);
        }
        let Some(run) = &mut self.run else {
            return;
        };
        self.state = run.o.state.lock().unwrap().clone();
        if run.scheduler.as_ref().is_some_and(JoinHandle::is_finished) {
            let outcome = run.scheduler.take().unwrap().join();
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    run.failed = true;
                    self.notice(&err, NOTICE_WINDOW);
                }
                Err(_) => {
                    run.o.stop();
                    self.notice("the scheduler thread died", NOTICE_WINDOW);
                }
            }
        }
        let over = self
            .run
            .as_ref()
            .is_some_and(|r| r.scheduler.is_none() && r.o.active.lock().unwrap().is_empty());
        if !over {
            return;
        }
        let run = self.run.take().unwrap();
        self.running = false;
        self.questions.retain(|q| q.ticket.is_none()); // never saved: derived again on resume
        self.composing = false;
        if run.o.stopping() {
            self.say("stopped, panes left running, /continue resumes");
        } else if run.failed {
            self.state = load_state(&self.launch.repo).unwrap_or_default();
        } else if run.epic {
            self.state = State::default(); // Epic done: nothing to resume
            if let Err(err) = self.state.save(&self.launch.repo) {
                self.notice(&format!("state not saved: {err}"), NOTICE_WINDOW);
            }
        }
        self.reload_epics();
    }

    /// The scheduler has returned and Ticket threads are still leaving.
    pub(crate) fn stopping(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.scheduler.is_none())
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

    /// An Event from the Orchestrator; only panel lines show. A Wake or a
    /// blocked session raises the Ticket's Question (replacing any it had),
    /// and a session that carries on by itself closes it.
    pub(crate) fn push(&mut self, event: Event) {
        if !event.panel {
            return;
        }
        self.events.push(event.clone());
        if self.events.len() > KEPT_EVENTS {
            self.events.drain(..self.events.len() - KEPT_EVENTS);
        }
        let Some(id) = &event.ticket else {
            return;
        };
        let kind = if event.text.starts_with("stuck in ") {
            Some(self.wake(id))
        } else if event.text.starts_with("waiting at a prompt") {
            Some(Kind::Blocked)
        } else if event.text == "carrying on" {
            None
        } else {
            return;
        };
        self.questions.retain(|q| q.ticket.as_deref() != Some(id));
        if let Some(kind) = kind {
            if self.questions.is_empty() {
                self.hidden = false;
            }
            self.questions.push(Question {
                ticket: Some(id.clone()),
                text: event.text.clone(),
                kind,
                cursor: 0,
            });
            // "stuck in fix 1", without the reason; a prompt line whole
            let short = match event.text.split_once(": ") {
                Some((stuck, _)) if stuck.starts_with("stuck in ") => stuck,
                _ => event.text.as_str(),
            };
            self.tell(Some(id), &format!("asking you: {short}"));
        }
    }

    /// A Wake's Question: the pane tail and the two canned nudges over the
    /// Stage's result file.
    fn wake(&self, id: &str) -> Kind {
        let ts = self.saved(id);
        let file = stage_named(&ts.stage).map_or(String::new(), |st| {
            self.launch
                .repo
                .join(".harness")
                .join("runs")
                .join(id)
                .join(result_name(st, ts.round))
                .display()
                .to_string()
        });
        let agent = agent_name(id, &ts.stage);
        let read = [
            "herdr",
            "agent",
            "read",
            &agent,
            "--source",
            "recent-unwrapped",
            "--lines",
            "120",
        ];
        let tail = self
            .launch
            .tools
            .run(&self.launch.repo, &read)
            .unwrap_or_default();
        let prompts = [NUDGE_RESULT, NUDGE_PROCEED].map(|p| p.replace("{result_file}", &file));
        Kind::Wake { tail, prompts }
    }

    /// One Ticket's state: the live run's, else the saved one.
    fn saved(&self, id: &str) -> TicketState {
        match &self.run {
            Some(run) => run.o.ticket(id),
            None => self.state.tickets.get(id).cloned().unwrap_or_default(),
        }
    }

    /// The pane of a Ticket's current Stage.
    fn pane_of(&self, id: &str) -> String {
        let ts = self.saved(id);
        ts.panes.get(&ts.stage).cloned().unwrap_or_default()
    }

    /// A line of the Shell's own, on RECENT and in the log as the
    /// Orchestrator's are; None is a run-level line.
    fn tell(&mut self, ticket: Option<&str>, text: &str) {
        let time = chrono::Local::now();
        let dir = self.launch.repo.join(".harness");
        let log = fs::create_dir_all(&dir).and_then(|()| {
            File::options()
                .create(true)
                .append(true)
                .open(dir.join("orchestrator.log"))
        });
        if let Ok(mut log) = log {
            let id = ticket.map_or(String::new(), |id| format!("{id} "));
            let _ = writeln!(log, "{} {id}{text}", time.format("%Y-%m-%d %H:%M:%S"));
        }
        self.push(Event {
            time,
            ticket: ticket.map(str::to_string),
            text: text.to_string(),
            panel: true,
        });
    }

    fn say(&mut self, text: &str) {
        self.tell(None, text);
    }

    /// A refused command: said, and a notice above the input line.
    fn refuse(&mut self, text: &str) {
        self.say(text);
        self.notice(text, NOTICE_WINDOW);
    }

    /// Whether a run is live or stopping, which refuses a start; said so.
    fn busy(&mut self) -> bool {
        match &self.run {
            None => false,
            Some(run) if run.scheduler.is_none() => {
                self.refuse("refused: a run is stopping");
                true
            }
            Some(_) => {
                self.refuse("refused: a run is live, /stop-work first");
                true
            }
        }
    }

    /// Whether a Ticket of the live run waits on the user: it has a Question
    /// waiting, or its last panel line is a trust dialog.
    // ponytail: the trust wait is read off the last Event rather than kept as state.
    pub(crate) fn blocked(&self, id: &str) -> bool {
        self.questions
            .iter()
            .any(|q| q.ticket.as_deref() == Some(id))
            || self
                .events
                .iter()
                .rev()
                .find(|e| e.ticket.as_deref() == Some(id))
                .is_some_and(|e| e.text.starts_with("waiting: "))
    }

    /// Whether the front Question shows above the input line.
    pub(crate) fn showing(&self) -> bool {
        !self.questions.is_empty() && !self.hidden
    }

    /// The front Question's options, numbered on the form in this order.
    pub(crate) fn options(&self) -> Vec<String> {
        let Some(q) = self.questions.first() else {
            return Vec::new();
        };
        match &q.kind {
            Kind::Wake { prompts, .. } => vec![
                format!("nudge: {}", prompts[0]),
                format!("nudge: {}", prompts[1]),
                "retry with a fresh session".to_string(),
                "park".to_string(),
                "open the pane".to_string(),
                "a prompt of your own".to_string(),
            ],
            Kind::Blocked => ["open the pane", "park", "I answered it"]
                .map(str::to_string)
                .to_vec(),
            Kind::Confirm(_) => ["yes", "no"].map(str::to_string).to_vec(),
            Kind::Continue { rows } => rows
                .iter()
                .map(|(id, reset)| {
                    let ts = self.state.tickets.get(id).cloned().unwrap_or_default();
                    let stage = if ts.round > 0 {
                        format!("{} {}", ts.stage, ts.round)
                    } else {
                        ts.stage.clone()
                    };
                    let parked = if ts.status == STATUS_PARKED {
                        format!("  parked: {}", ts.reason)
                    } else {
                        String::new()
                    };
                    let how = if *reset {
                        "reset to Implement"
                    } else {
                        "resume"
                    };
                    format!("{}  {stage}{parked}  → {how}", self.name(id))
                })
                .collect(),
        }
    }

    /// A Ticket as RECENT names it: its suffix and title, or its id alone.
    pub(crate) fn name(&self, id: &str) -> String {
        match self.title(id) {
            Some(title) => format!("{} {title}", suffix(id)),
            None => id.to_string(),
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
                self.quit();
            } else {
                self.ctrl_c = Some(Instant::now());
                self.notice("press Ctrl-C again to exit", CTRL_C_WINDOW);
            }
            return;
        }
        // With the form up and the input line empty the keys are its:
        // arrows or a number pick, Enter answers, Esc hides or cancels,
        // Space toggles a /continue row, y and n answer a confirmation; a
        // slash starts a command.
        if self.showing() && self.input.is_empty() && !self.composing {
            let n = self.options().len();
            let confirm = matches!(self.questions[0].kind, Kind::Confirm(_));
            let q = &mut self.questions[0];
            match key.code {
                KeyCode::Up => q.cursor = q.cursor.saturating_sub(1),
                KeyCode::Down => q.cursor = (q.cursor + 1).min(n - 1),
                KeyCode::Char(c @ '0'..='9') => {
                    if let Some(i) = (c as usize).checked_sub('1' as usize).filter(|i| *i < n) {
                        q.cursor = i;
                    }
                }
                KeyCode::Char(' ') => {
                    if let Kind::Continue { rows } = &mut q.kind {
                        rows[q.cursor].1 ^= true;
                    }
                }
                KeyCode::Char('y') if confirm => self.answer(0),
                KeyCode::Char('n') if confirm => self.answer(1),
                KeyCode::Enter => {
                    let cursor = q.cursor;
                    self.answer(cursor);
                }
                // Esc hides a Ticket's Question; it cancels a confirmation
                // or the /continue checklist.
                KeyCode::Esc if q.ticket.is_some() => self.hidden = true,
                KeyCode::Esc => {
                    self.questions.remove(0);
                    self.notice("cancelled", NOTICE_WINDOW);
                }
                KeyCode::Char(c) if !held => self.input.push(c),
                _ => {}
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
            KeyCode::Esc if self.composing => {
                self.composing = false;
                self.input.clear();
            }
            KeyCode::Esc if self.input.is_empty() => self.hidden = false,
            KeyCode::Esc => self.input.clear(),
            KeyCode::Tab => self.complete(),
            KeyCode::Enter if self.composing => {
                let prompt = std::mem::take(&mut self.input);
                if !prompt.trim().is_empty() {
                    self.composing = false;
                    self.nudge("your prompt", prompt.trim(), "nudged with your prompt");
                }
            }
            KeyCode::Enter => {
                let line = std::mem::take(&mut self.input);
                self.command(line.trim());
            }
            _ => {}
        }
    }

    /// The user picked option `choice` of the front Question.
    fn answer(&mut self, choice: usize) {
        let ticket = self.questions[0].ticket.clone().unwrap_or_default();
        match (&self.questions[0].kind, choice) {
            (Kind::Wake { prompts, .. }, 0 | 1) => {
                let prompt = prompts[choice].clone();
                let outcome = if choice == 0 {
                    "nudged: write the result file"
                } else {
                    "nudged: carry on, the Ticket is the spec"
                };
                self.nudge("nudge", &prompt, outcome);
            }
            (Kind::Wake { .. }, 2) => {
                self.answered("retry");
                self.send(&format!("retry-{ticket}"));
            }
            (Kind::Wake { .. }, 3) | (Kind::Blocked, 1) => {
                self.answered("park");
                self.send(&format!("park-{ticket}"));
            }
            (Kind::Wake { .. }, 4) | (Kind::Blocked, 0) => {
                // the Question stays
                let pane = self.pane_of(&ticket);
                let focus = self
                    .launch
                    .tools
                    .run(&self.launch.repo, &["herdr", "pane", "focus", &pane]);
                if let Err(err) = focus {
                    self.notice(&err.to_string(), NOTICE_WINDOW);
                }
            }
            (Kind::Wake { .. }, 5) => self.composing = true,
            (Kind::Blocked, 2) => self.answered("I answered it"),
            (Kind::Confirm(_), 0) => {
                let Kind::Confirm(pending) = self.questions.remove(0).kind else {
                    unreachable!()
                };
                match pending {
                    Pending::Start { id, max, epic } => self.start(&id, max, epic, true),
                    Pending::Exit => self.quit(),
                }
            }
            (Kind::Confirm(_), _) => {
                self.questions.remove(0);
                self.notice("cancelled", NOTICE_WINDOW);
            }
            (Kind::Continue { rows }, _) => {
                let rows = rows.clone();
                self.questions.remove(0);
                self.resume(&rows);
            }
            _ => {}
        }
        self.hidden = false;
    }

    /// The front Question is answered: line one names the user's answer.
    fn answered(&mut self, word: &str) {
        let q = self.questions.remove(0);
        self.tell(q.ticket.as_deref(), &format!("you answered: {word}"));
    }

    /// A command to the live run's Orchestrator.
    fn send(&mut self, command: &str) {
        if let Some(run) = &self.run {
            run.o.command(command);
        }
    }

    /// Prompts the front Question's session with `prompt` and re-arms its
    /// hold; the two lines follow, or the failure with the Question kept.
    fn nudge(&mut self, word: &str, prompt: &str, outcome: &str) {
        let ticket = self.questions[0].ticket.clone().unwrap_or_default();
        let pane = self.pane_of(&ticket);
        let sent = self.launch.tools.run(
            &self.launch.repo,
            &["herdr", "agent", "prompt", &pane, prompt],
        );
        if let Err(err) = sent {
            return self.notice(&format!("nudge failed: {err}"), NOTICE_WINDOW);
        }
        self.answered(word);
        self.send(&format!("nudge-{ticket}"));
        self.tell(Some(&ticket), outcome);
    }

    /// A yes/no confirmation, shown at once ahead of any waiting Question.
    fn confirm(&mut self, text: &str, pending: Pending) {
        self.questions.insert(
            0,
            Question {
                ticket: None,
                text: text.to_string(),
                kind: Kind::Confirm(pending),
                cursor: 0,
            },
        );
        self.hidden = false;
    }

    /// The /continue checklist answered: a reset Ticket loses its panes and
    /// its run directory and starts Implement over; the rest resume as saved.
    fn resume(&mut self, rows: &[(String, bool)]) {
        for (id, _) in rows.iter().filter(|(_, reset)| *reset) {
            let ts = self.state.tickets.get(id).cloned().unwrap_or_default();
            for pane in ts.panes.values() {
                let _ = self
                    .launch
                    .tools
                    .run(&self.launch.repo, &["herdr", "pane", "close", pane]);
            }
            let _ = fs::remove_dir_all(self.launch.repo.join(".harness").join("runs").join(id));
            if let Some(ts) = self.state.tickets.get_mut(id) {
                *ts = TicketState {
                    status: STATUS_RUNNING.to_string(),
                    stage: "implement".to_string(),
                    tab: ts.tab.clone(),
                    ..Default::default()
                };
            }
        }
        if rows.iter().any(|(_, reset)| *reset) {
            if let Err(err) = self.state.save(&self.launch.repo) {
                return self.notice(&format!("state not saved: {err}"), NOTICE_WINDOW);
            }
        }
        let epic = self.state.epic.clone();
        if !epic.is_empty() {
            self.launch(DEFAULT_MAX, true, move |o| o.run(&epic));
        } else {
            self.launch(DEFAULT_MAX, false, move |o| {
                o.run_tickets(&o.resumable());
                Ok(())
            });
        }
    }

    /// One input line: a slash command, or y/n typed at a confirmation.
    pub(crate) fn command(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        if matches!(self.questions.first(), Some(q) if matches!(q.kind, Kind::Confirm(_))) {
            match line {
                "y" | "yes" => return self.answer(0),
                "n" | "no" => return self.answer(1),
                _ => {}
            }
        }
        let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
        let query = rest.trim();
        match name {
            "/start-epic" | "/start-ticket" => {
                if self.busy() {
                    return;
                }
                let epic = name == "/start-epic";
                let (query, max) = match parse_args(query) {
                    Ok(parsed) => parsed,
                    Err(err) => return self.notice(&err, NOTICE_WINDOW),
                };
                self.reload_epics();
                if let Some(id) = self.resolve(&query, epic) {
                    self.start(&id, max, epic, false);
                }
            }
            "/continue" => {
                if self.busy() {
                    return;
                }
                let mut rows: Vec<(String, bool)> = self
                    .state
                    .tickets
                    .iter()
                    .filter(|(_, ts)| ts.status == STATUS_RUNNING || ts.status == STATUS_PARKED)
                    .map(|(id, _)| (id.clone(), false))
                    .collect();
                rows.sort_by_key(|(id, _)| suffix(id).parse::<usize>().unwrap_or(usize::MAX));
                let running = self
                    .state
                    .tickets
                    .values()
                    .any(|ts| ts.status == STATUS_RUNNING);
                if self.state.epic.is_empty() && !running {
                    self.refuse("refused: no saved Ticket to continue");
                } else if rows.is_empty() {
                    self.resume(&rows);
                } else {
                    self.questions.insert(
                        0,
                        Question {
                            ticket: None,
                            text: "continue the saved run: each Ticket resumes at its Stage, or is reset to Implement".to_string(),
                            kind: Kind::Continue { rows },
                            cursor: 0,
                        },
                    );
                    self.hidden = false;
                }
            }
            "/questions" => match self.questions.is_empty() {
                true => self.notice("no questions waiting", NOTICE_WINDOW),
                false => self.hidden = false,
            },
            "/stop-work" => self.stop_work(),
            "/retry" | "/park" | "/address" => match &self.run {
                _ if query.is_empty() => {
                    self.notice(&format!("usage: {name} <ticket>"), NOTICE_WINDOW)
                }
                None => self.refuse("refused: no run is live, /start-epic or /continue starts one"),
                Some(_)
                    if name != "/address"
                        && self
                            .questions
                            .iter()
                            .any(|q| q.ticket.as_deref() == Some(query)) =>
                {
                    self.refuse(&format!(
                        "refused: Ticket {} has a Question waiting",
                        suffix(query)
                    ));
                }
                Some(run) if name == "/address" && !run.epic => {
                    // ponytail: a single-Ticket run has no scheduler to consume it
                    run.o.report(query, "address refused: not an Epic run");
                }
                Some(run) => run.o.command(&format!("{}-{query}", &name[1..])),
            },
            "/exit" => match &self.run {
                None => self.quit(),
                Some(_) => self.confirm("stop the run and exit?", Pending::Exit),
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

    /// /start-epic and /start-ticket: over a different saved Epic (the
    /// Ticket's parent on the tree) it asks before discarding the saved run.
    fn start(&mut self, id: &str, max: usize, epic: bool, discard: bool) {
        let saved = self.state.epic.clone();
        let mine = if epic {
            id.to_string()
        } else {
            self.epics
                .iter()
                .find(|e| e.tickets.iter().any(|t| t.id == id))
                .map(|e| e.id.clone())
                .unwrap_or_default()
        };
        if !saved.is_empty() && saved != mine {
            if !discard {
                let pending = Pending::Start {
                    id: id.to_string(),
                    max,
                    epic,
                };
                return self.confirm(&format!("discard the saved run on {saved}?"), pending);
            }
            self.state = State::default();
            if let Err(err) = self.state.save(&self.launch.repo) {
                return self.notice(&format!("state not saved: {err}"), NOTICE_WINDOW);
            }
        }
        let id = id.to_string();
        if epic {
            self.launch(max, true, move |o| o.run(&id));
        } else {
            self.launch(max, false, move |o| {
                o.run_single(&id);
                Ok(())
            });
        }
    }

    /// Takes the lock and runs `work` over a fresh Orchestrator on the
    /// scheduler thread; it returns when the run ends and poll sees it. The
    /// callers have checked that no run is live.
    fn launch(
        &mut self,
        max: usize,
        epic: bool,
        work: impl FnOnce(&Arc<Orchestrator>) -> Result<(), String> + Send + 'static,
    ) {
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
            scheduler: Some(scheduler),
            epic,
            failed: false,
            _lock: lock,
        });
        self.running = true;
    }

    /// /stop-work: scheduling ends, live panes stay, the state file holds
    /// every Ticket as saved, and the lock goes once every thread has left.
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
