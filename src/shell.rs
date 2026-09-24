//! The Shell: the full-terminal screen that `harness` alone opens (ADR 0004).
//! `Screen` is the plain state the tests drive; `open` wraps it in the
//! terminal and the one draw, poll and tick loop (ADR 0003). The Shell owns
//! the Orchestrator: the scheduler runs on a thread of this process, its
//! Events come over a channel into RECENT and the log, and the TICKETS rows
//! are a snapshot of its State. An Event that asks (a Wake, a blocked
//! session) becomes a Question, whose answer goes back to the Orchestrator
//! for that session; the Orchestrator knows no Shell type.

use std::cell::Cell;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as Input, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::orchestrator::judgment::{self, Action};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::{log_line, Answer, Ask, Config, Event, Orchestrator};
use crate::orchestrator::state::{
    acquire_lock, load_state, Lock, State, TicketState, STATUS_PARKED, STATUS_RUNNING,
};
use crate::setup;
use crate::tools::Tools;
use crate::update::{self, Checked, Ready, Releases};

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
/// An update another process's run keeps from installing is tried this often.
const RETRY: Duration = Duration::from_secs(60);

/// An open Epic and its child Tickets, one row each on the TICKETS tree.
pub(crate) struct Epic {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) tickets: Vec<BdIssue>,
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

/// What a Question is about, which decides its options and what an answer does.
pub(crate) enum About {
    /// What the Orchestrator asked of a Ticket. A Wake offers the actions
    /// still unspent (a nudge with either canned prompt, or the one a
    /// Judgment picked; retry; park; wait), then open the pane and a prompt
    /// of your own; a blocked session offers open the pane, park, "I
    /// answered it"; a plan offers approve, feedback of your own, resend
    /// the feedback that was not sent, park, open the pane; a plan
    /// failure offers open the pane, park, retry, resend the feedback.
    Asked(Ask),
    /// A yes/no confirmation; it jumps the queue.
    Confirm(Pending),
    /// The /continue checklist: one row per saved Ticket, reset toggled by Space.
    Continue { rows: Vec<(String, bool)> },
}

/// What the Shell puts to the user above the input line when the
/// Orchestrator cannot act alone. One shows at a time, oldest first; a
/// Ticket's Question holds that Ticket alone, and none is ever saved.
pub(crate) struct Question {
    pub(crate) ticket: Option<String>,
    /// The event line it came from, or the confirmation's wording.
    pub(crate) text: String,
    pub(crate) about: About,
    /// The option the cursor is on.
    pub(crate) cursor: usize,
    /// The first row of a plan shown, which PageUp and PageDown move; the
    /// draw, which knows the width, keeps it inside the plan.
    pub(crate) scroll: Cell<usize>,
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
    /// The first TICKETS row shown, for a tree taller than its room; the
    /// draw, which knows the height, keeps it inside the tree.
    pub(crate) scroll: Cell<usize>,
    /// RECENT's rows scrolled up from the newest, 0 sticking to the bottom;
    /// the draw, which knows the height, keeps it inside the lines.
    pub(crate) recent: Cell<usize>,
    /// One line above the input, and when it goes.
    pub(crate) notice: Option<(String, Instant)>,
    ctrl_c: Option<Instant>,
    pub(crate) ticks: u64,
    /// A run is live: the logo hops, the status row spins.
    pub(crate) running: bool,
    pub(crate) quit: bool,
    /// Every run's Config, cloned for the run. Its exe is the running
    /// binary's real path, resolved once at open: an update renames over it,
    /// and every run's plan hook runs it. Tests point it at a scratch file.
    pub(crate) cfg: Config,
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
    /// Esc hid the Question; /questions or Esc on an empty input line brings it back.
    pub(crate) hidden: bool,
    /// The input line is a prompt of the user's own for the front Question.
    pub(crate) composing: bool,
    /// The updater thread's checks, applied between commands in poll().
    update_sender: Sender<Checked>,
    update_receiver: Receiver<Checked>,
    /// A release downloaded while a run holds the lock: installed when it ends.
    pub(crate) update: Option<Ready>,
    /// When a pending update may try the lock again, from tick().
    pub(crate) retry: Instant,
    /// An idle-Shell update installed: open() re-execs after the terminal is back.
    pub(crate) reexec: bool,
}

impl Screen {
    pub(crate) fn new(
        cfg: Config,
        folder: String,
        truecolor: bool,
        epics: Vec<Epic>,
        state: State,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let (update_sender, update_receiver) = mpsc::channel();
        Screen {
            folder,
            version: crate::version::version(),
            truecolor,
            logo: logo::Logo::embedded(),
            epics,
            state,
            events: Vec::new(),
            input: String::new(),
            scroll: Cell::new(0),
            recent: Cell::new(0),
            notice: None,
            ctrl_c: None,
            ticks: 0,
            running: false,
            quit: false,
            cfg,
            missing: Vec::new(),
            sender,
            receiver,
            run: None,
            questions: Vec::new(),
            hidden: false,
            composing: false,
            update_sender,
            update_receiver,
            update: None,
            retry: Instant::now(),
            reexec: false,
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
        let cfg = Config {
            tools,
            repo: repo.to_path_buf(),
            workspace: env("HERDR_WORKSPACE_ID"),
            api_key: setup::typesafe_key(repo, env).unwrap_or_default(),
            exe: PathBuf::new(),
            typesafe: Arc::new(judgment::Api),
            home: PathBuf::from(&home),
            tick: Duration::from_secs(5),
            poll_prs: Duration::from_secs(30),
            max: DEFAULT_MAX,
            log: Arc::new(Mutex::new(Box::new(io::sink()))),
            events: mpsc::channel().0,
            #[cfg(test)]
            timeout: None,
            #[cfg(test)]
            wait: None,
        };
        let mut screen = Screen::new(cfg, folder, truecolor, Vec::new(), state);
        screen.missing = missing;
        screen.reload_epics();
        match update::exe_path() {
            Ok(exe) => {
                screen.cfg.exe = exe;
                screen.check_updates(Arc::new(update::GitHub), update::EVERY);
            }
            Err(err) => screen.say(&format!("update check failed: {err}")),
        }
        screen
    }

    /// The updater thread: one check now, then one every `every`, each handed
    /// to poll(); it never renames, and stops once it has handed over a
    /// release, one replace per process. A dev build's check asks nothing.
    pub(crate) fn check_updates(&self, releases: Arc<dyn Releases>, every: Duration) {
        let (version, exe, tx) = (
            self.version.clone(),
            self.cfg.exe.clone(),
            self.update_sender.clone(),
        );
        thread::spawn(move || loop {
            let checked = update::check(&*releases, &version, &exe);
            let done = matches!(checked, Ok(Some(_)));
            if tx.send(checked).is_err() || done {
                return;
            }
            thread::sleep(every);
        });
    }

    /// A check's outcome: a failure is one line; a release installs at once
    /// on an idle Shell, or waits for the run to release the lock.
    fn updated(&mut self, checked: Checked) {
        match checked {
            Err(err) => self.say(&format!("update check failed: {err}")),
            Ok(None) => {}
            Ok(Some(ready)) => {
                self.update = Some(ready);
                match &self.run {
                    Some(_) => self.say(&format!(
                        "{} downloaded, installs when the run stops",
                        self.update.as_ref().unwrap().tag
                    )),
                    None => self.install(true),
                }
            }
        }
    }

    /// The pending rename over the exe, under the repo lock, which no run of
    /// this Shell may hold: another process's run keeps it pending, tried
    /// again from tick() a minute on. On an idle Shell a done rename says
    /// 'updating to vX' and quits for open() to re-exec; at the end of a run
    /// it says nothing. A refused rename is one line, the temp file gone.
    fn install(&mut self, reexec: bool) {
        let Some(ready) = self.update.take() else {
            return;
        };
        let tag = ready.tag.clone();
        let installed = match acquire_lock(&self.cfg.repo) {
            Ok(_lock) => ready.install(),
            Err(_) => {
                self.update = Some(ready);
                self.retry = Instant::now() + RETRY;
                return;
            }
        };
        match installed {
            Ok(()) if reexec => {
                self.say(&format!("updating to {tag}"));
                self.quit = true;
                self.reexec = true;
            }
            Ok(()) => {}
            Err(err) => self.say(&format!("update check failed: {err}")),
        }
    }

    /// The header's version: with a release waiting on the run, where it goes.
    pub(crate) fn shown_version(&self) -> String {
        match &self.update {
            Some(ready) => format!("{} → {} at stop", self.version, ready.tag),
            None => self.version.clone(),
        }
    }

    /// The Shell's last act, after the terminal is back: the run goes, its
    /// lock with it, and then the release that waited on it installs (/exit
    /// and Ctrl-C twice, with no re-exec). Another process's lock drops it.
    pub(crate) fn close(&mut self) {
        drop(self.run.take());
        self.install(false);
        if let Some(ready) = self.update.take() {
            ready.discard();
        }
    }

    /// The bd cache again: on open, on every /start-epic, after a Ticket
    /// closes and when a run ends.
    fn reload_epics(&mut self) {
        match load_epics(&self.cfg.repo, &*self.cfg.tools) {
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
        while let Ok(checked) = self.update_receiver.try_recv() {
            self.updated(checked);
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
            self.state = load_state(&self.cfg.repo).unwrap_or_default();
        } else if run.epic {
            self.state = State::default(); // Epic done: nothing to resume
            if let Err(err) = self.state.save(&self.cfg.repo) {
                self.notice(&format!("state not saved: {err}"), NOTICE_WINDOW);
            }
        }
        self.reload_epics();
        drop(run); // the lock goes
        self.install(false); // the last act of /stop-work
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
        if self.run.is_none() && self.update.is_some() && Instant::now() >= self.retry {
            self.install(true);
        }
    }

    /// An Event from the Orchestrator; only panel lines show. Any line of
    /// a Ticket closes the Question it had: the Ticket has moved on, and an
    /// answer sent to it meanwhile is dropped. A line that asks, or an ask
    /// with no line of its own, raises the Ticket's Question anew.
    pub(crate) fn push(&mut self, event: Event) {
        if !event.panel && event.ask.is_none() {
            return;
        }
        let (ticket, ask, text) = (event.ticket.clone(), event.ask.clone(), event.text.clone());
        if event.panel {
            self.show(event);
        }
        let Some(id) = ticket else {
            return;
        };
        if let Some(i) = self
            .questions
            .iter()
            .position(|q| q.ticket.as_deref() == Some(&id))
        {
            self.questions.remove(i);
            if i == 0 && self.composing {
                self.composing = false; // the prompt was for that Question
                self.input.clear();
            }
            if let Some(run) = &self.run {
                run.o.take_answer(&id, None);
            }
        }
        let Some(ask) = ask else {
            return;
        };
        if self.questions.is_empty() {
            self.hidden = false;
        }
        // "stuck in fix 1", without the reason; a prompt line whole
        let short = match ask {
            Ask::Wake { .. } | Ask::PlanFailed { .. } => {
                text.split_once(": ").map_or(text.as_str(), |(s, _)| s)
            }
            Ask::Blocked { .. } | Ask::Plan { .. } => text.as_str(),
        };
        let asking = format!("asking you: {short}");
        self.questions.push(Question {
            ticket: Some(id.clone()),
            text,
            about: About::Asked(ask),
            cursor: 0,
            scroll: Cell::new(0),
        });
        self.tell(Some(&id), &asking);
    }

    /// A line on RECENT.
    fn show(&mut self, event: Event) {
        self.events.push(event);
        // scrolled up, RECENT stays on the lines it shows
        if self.recent.get() > 0 {
            self.recent.set(self.recent.get() + 1);
        }
        if self.events.len() > KEPT_EVENTS {
            self.events.drain(..self.events.len() - KEPT_EVENTS);
        }
    }

    /// A line of the Shell's own, on RECENT and in the log as the
    /// Orchestrator's are; None is a run-level line.
    fn tell(&mut self, ticket: Option<&str>, text: &str) {
        let time = chrono::Local::now();
        let dir = self.cfg.repo.join(".harness");
        let log = fs::create_dir_all(&dir).and_then(|()| {
            File::options()
                .create(true)
                .append(true)
                .open(dir.join("orchestrator.log"))
        });
        if let Ok(mut log) = log {
            // one write: Ticket threads append to the same file
            let _ = log.write_all(log_line(time, ticket.unwrap_or(""), text).as_bytes());
        }
        self.show(Event {
            time,
            ticket: ticket.map(str::to_string),
            text: text.to_string(),
            panel: true,
            ask: None,
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
                .is_some_and(|e| {
                    e.text.starts_with("waiting: ") && e.text.contains(" does not trust ")
                })
    }

    /// Whether the front Question shows above the input line.
    pub(crate) fn showing(&self) -> bool {
        !self.questions.is_empty() && !self.hidden
    }

    /// The front Question's options, numbered in this order.
    pub(crate) fn options(&self) -> Vec<String> {
        let Some(q) = self.questions.first() else {
            return Vec::new();
        };
        match &q.about {
            About::Asked(Ask::Wake { actions, file, .. }) => actions
                .iter()
                .map(|action| action.option(file))
                .chain(["open the pane", "a prompt of your own"].map(str::to_string))
                .collect(),
            About::Asked(Ask::Blocked { .. }) => ["open the pane", "park", "I answered it"]
                .map(str::to_string)
                .to_vec(),
            About::Asked(Ask::Plan { feedback, .. }) => ["approve", "feedback of your own"]
                .map(str::to_string)
                .into_iter()
                .chain(
                    feedback
                        .iter()
                        .map(|f| format!("resend your feedback: {f}")),
                )
                .chain(["park", "open the pane"].map(str::to_string))
                .collect(),
            About::Asked(Ask::PlanFailed { feedback, .. }) => {
                ["open the pane", "park", "retry with a fresh session"]
                    .map(str::to_string)
                    .into_iter()
                    .chain(
                        feedback
                            .iter()
                            .map(|f| format!("resend your feedback: {f}")),
                    )
                    .collect()
            }
            About::Confirm(_) => ["yes", "no"].map(str::to_string).to_vec(),
            About::Continue { rows } => rows
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
        // With a Question showing and the input line empty the keys are
        // its: arrows or a number pick, Enter answers, Esc hides or cancels,
        // Space toggles a /continue row, y and n answer a confirmation,
        // PageUp and PageDown scroll a plan; a slash starts a command.
        if self.showing() && self.input.is_empty() && !self.composing {
            let n = self.options().len();
            let confirm = matches!(self.questions[0].about, About::Confirm(_));
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
                    if let About::Continue { rows } = &mut q.about {
                        rows[q.cursor].1 ^= true;
                    }
                }
                KeyCode::PageDown | KeyCode::PageUp => scroll_plan(q, key.code),
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
        // With the input line empty, Up and Down (and the wheel, which the
        // terminal sends as them) scroll RECENT; PageUp and PageDown TICKETS.
        let scroll = |by: isize| self.scroll.set(self.scroll.get().saturating_add_signed(by));
        match key.code {
            KeyCode::Char(_) if held => {}
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::PageDown | KeyCode::PageUp if self.composing => {
                if let Some(q) = self.questions.first() {
                    scroll_plan(q, key.code);
                }
            }
            KeyCode::Down if self.input.is_empty() => {
                self.recent.set(self.recent.get().saturating_sub(1))
            }
            KeyCode::Up if self.input.is_empty() => self.recent.set(self.recent.get() + 1),
            KeyCode::PageDown if self.input.is_empty() => scroll(10),
            KeyCode::PageUp if self.input.is_empty() => scroll(-10),
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
                    let word = match self.questions[0].about {
                        About::Asked(Ask::Plan { .. }) => "feedback",
                        _ => "your prompt",
                    };
                    self.reply(word, Answer::Prompt(prompt.trim().to_string()));
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
        match (&self.questions[0].about, choice) {
            // a Wake's actions, then open the pane and a prompt of your own
            (About::Asked(Ask::Wake { actions, pane, .. }), _) => match actions.get(choice) {
                Some(&action) => self.reply(action.word(), Answer::Act(action)),
                None if choice == actions.len() => self.open_pane(pane.clone()),
                None => self.composing = true,
            },
            (About::Asked(Ask::Blocked { pane }), 0) => self.open_pane(pane.clone()),
            (About::Asked(Ask::Blocked { .. }), 1) => self.reply("park", Answer::Act(Action::Park)),
            (About::Asked(Ask::Blocked { .. }), 2) => {
                let q = self.questions.remove(0);
                self.tell(q.ticket.as_deref(), "you answered: I answered it");
            }
            // approve, feedback of your own, resend, park, open the pane
            (About::Asked(Ask::Plan { feedback, pane, .. }), n) => {
                let resend = usize::from(feedback.is_some());
                match n {
                    0 => self.reply("approve", Answer::Approve),
                    1 => self.composing = true,
                    2 if resend == 1 => {
                        let feedback = feedback.clone().unwrap_or_default();
                        self.reply("feedback", Answer::Prompt(feedback));
                    }
                    n if n == 2 + resend => self.reply("park", Answer::Act(Action::Park)),
                    _ => self.open_pane(pane.clone()),
                }
            }
            (About::Asked(Ask::PlanFailed { pane, .. }), 0) => self.open_pane(pane.clone()),
            (About::Asked(Ask::PlanFailed { .. }), 1) => {
                self.reply("park", Answer::Act(Action::Park))
            }
            (About::Asked(Ask::PlanFailed { .. }), 2) => {
                self.reply("retry", Answer::Act(Action::Retry))
            }
            (
                About::Asked(Ask::PlanFailed {
                    feedback: Some(f), ..
                }),
                _,
            ) => {
                let feedback = f.clone();
                self.reply("feedback", Answer::Prompt(feedback));
            }
            (About::Confirm(_), 0) => {
                let About::Confirm(pending) = self.questions.remove(0).about else {
                    unreachable!()
                };
                match pending {
                    Pending::Start { id, max, epic } => self.start(&id, max, epic, true),
                    Pending::Exit => self.quit(),
                }
            }
            (About::Confirm(_), _) => {
                self.questions.remove(0);
                self.notice("cancelled", NOTICE_WINDOW);
            }
            (About::Continue { rows }, _) => {
                let rows = rows.clone();
                self.questions.remove(0);
                self.resume(&rows);
            }
            _ => {}
        }
        self.hidden = false;
    }

    /// Focuses the pane a Question is about; the Question stays.
    fn open_pane(&mut self, pane: String) {
        let focus = self
            .cfg
            .tools
            .run(&self.cfg.repo, &["herdr", "agent", "focus", &pane]);
        if let Err(err) = focus {
            self.notice(&err.to_string(), NOTICE_WINDOW);
        }
    }

    /// The front Question answered: line one names the answer, which goes
    /// to the Orchestrator for the session the Question was about; line two
    /// is the Orchestrator's, once it acts.
    fn reply(&mut self, word: &str, answer: Answer) {
        let q = self.questions.remove(0);
        let id = q.ticket.unwrap_or_default();
        self.tell(Some(&id), &format!("you answered: {word}"));
        if let (
            Some(run),
            About::Asked(
                Ask::Wake { pane, .. }
                | Ask::Blocked { pane }
                | Ask::Plan { pane, .. }
                | Ask::PlanFailed { pane, .. },
            ),
        ) = (&self.run, &q.about)
        {
            run.o.answer(&id, pane, answer);
        }
    }

    /// A yes/no confirmation, its cursor on no: Enter alone never discards
    /// or exits.
    fn confirm(&mut self, text: &str, pending: Pending) {
        self.ask_first(Question {
            ticket: None,
            text: text.to_string(),
            about: About::Confirm(pending),
            cursor: 1,
            scroll: Cell::new(0),
        });
    }

    /// A confirmation or the /continue checklist: shown at once, ahead of
    /// every Ticket's Question, in place of one still waiting.
    fn ask_first(&mut self, question: Question) {
        self.questions.retain(|q| q.ticket.is_some());
        self.questions.insert(0, question);
        self.hidden = false;
    }

    /// The /continue checklist answered, once the lock is held: a reset
    /// Ticket starts Implement over, a Parked one set to resume is unparked
    /// at its Stage, and the rest resume as saved.
    fn resume(&mut self, rows: &[(String, bool)]) {
        let Some(prepared) = self.prepare(DEFAULT_MAX) else {
            return;
        };
        let o = prepared.1.clone();
        for (id, reset) in rows {
            if *reset {
                if let Err(err) = reset_ticket(&o, id) {
                    self.notice(&format!("{id} not reset: {err}"), NOTICE_WINDOW);
                }
            } else if o.ticket(id).status == STATUS_PARKED {
                o.update(id, |ts| ts.status = STATUS_RUNNING.to_string());
            }
        }
        let epic = o.state.lock().unwrap().epic.clone();
        if !epic.is_empty() {
            self.spawn(prepared, true, move |o| o.run(&epic));
        } else {
            self.spawn(prepared, false, move |o| {
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
        if matches!(self.questions.first(), Some(q) if matches!(q.about, About::Confirm(_))) {
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
                if rows.is_empty() && self.state.epic.is_empty() {
                    self.refuse("refused: no saved Ticket to continue");
                } else if rows.is_empty() {
                    self.resume(&rows);
                } else {
                    self.ask_first(Question {
                        ticket: None,
                        text: "continue the saved run: each Ticket resumes at its Stage, or is reset to Implement".to_string(),
                        about: About::Continue { rows },
                        cursor: 0,
                        scroll: Cell::new(0),
                    });
                }
            }
            "/questions" => match self.questions.is_empty() {
                true => self.notice("no questions waiting", NOTICE_WINDOW),
                false => self.hidden = false,
            },
            "/stop-work" => self.stop_work(),
            "/retry" | "/park" | "/address" => {
                let waiting = self
                    .questions
                    .iter()
                    .any(|q| q.ticket.as_deref() == Some(query));
                match self.run.as_ref().map(|run| (run.epic, run.o.clone())) {
                    _ if query.is_empty() => {
                        self.notice(&format!("usage: {name} <ticket>"), NOTICE_WINDOW)
                    }
                    None => {
                        self.refuse("refused: no run is live, /start-epic or /continue starts one")
                    }
                    Some(_) if name != "/address" && waiting => self.refuse(&format!(
                        "refused: Ticket {} has a Question waiting",
                        suffix(query)
                    )),
                    // ponytail: a single-Ticket run has no scheduler to consume it
                    Some((false, _)) if name == "/address" => {
                        self.tell(Some(query), "address refused: not an Epic run")
                    }
                    Some((_, o)) => o.command(&format!("{}-{query}", &name[1..])),
                }
            }
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
    /// Ticket's parent on the tree) it asks before discarding the saved run,
    /// which goes only once the lock is held.
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
        let other = !saved.is_empty() && saved != mine;
        if other && !discard {
            let pending = Pending::Start {
                id: id.to_string(),
                max,
                epic,
            };
            return self.confirm(&format!("discard the saved run on {saved}?"), pending);
        }
        let Some(prepared) = self.prepare(max) else {
            return;
        };
        if other {
            if let Err(err) = State::default().save(&self.cfg.repo) {
                return self.notice(&format!("state not saved: {err}"), NOTICE_WINDOW);
            }
            *prepared.1.state.lock().unwrap() = State::default();
        }
        let id = id.to_string();
        if epic {
            self.spawn(prepared, true, move |o| o.run(&id));
        } else {
            self.spawn(prepared, false, move |o| {
                o.run_ticket(&id);
                Ok(())
            });
        }
    }

    /// Takes the lock and makes the run's Orchestrator over the state
    /// file; None, said in a notice, when a run cannot start. The callers
    /// have checked that no run is live.
    fn prepare(&mut self, max: usize) -> Option<(Lock, Arc<Orchestrator>)> {
        if let Some(missing) = self.missing.first() {
            self.notice(&format!("refused: {missing}"), NOTICE_WINDOW);
            return None;
        }
        let repo = &self.cfg.repo;
        let lock = match setup::ignore_run_dir(repo).and_then(|()| acquire_lock(repo)) {
            Ok(lock) => lock,
            Err(err) => {
                self.notice(&err.to_string(), NOTICE_WINDOW);
                return None;
            }
        };
        let log: Box<dyn io::Write + Send> = match File::options()
            .create(true)
            .append(true)
            .open(repo.join(".harness").join("orchestrator.log"))
        {
            Ok(file) => Box::new(file),
            Err(_) => Box::new(io::sink()),
        };
        match Orchestrator::new(Config {
            max,
            log: Arc::new(Mutex::new(log)),
            events: self.sender.clone(),
            ..self.cfg.clone()
        }) {
            Ok(o) => Some((lock, o)),
            Err(err) => {
                self.notice(&err.to_string(), NOTICE_WINDOW);
                None
            }
        }
    }

    /// Runs `work` over the prepared Orchestrator on the scheduler thread;
    /// it returns when the run ends and poll sees it.
    fn spawn(
        &mut self,
        (lock, o): (Lock, Arc<Orchestrator>),
        epic: bool,
        work: impl FnOnce(&Arc<Orchestrator>) -> Result<(), String> + Send + 'static,
    ) {
        self.state = o.state.lock().unwrap().clone();
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

    /// The title of a Ticket on the idle tree, for the RECENT Ticket column.
    pub(crate) fn title(&self, id: &str) -> Option<&str> {
        self.epics
            .iter()
            .flat_map(|e| &e.tickets)
            .find(|t| t.id == id)
            .map(|t| t.title.as_str())
    }
}

/// PageDown and PageUp move a plan Question ten rows.
fn scroll_plan(q: &Question, code: KeyCode) {
    if matches!(q.about, About::Asked(Ask::Plan { .. })) {
        let row = q.scroll.get();
        q.scroll.set(match code {
            KeyCode::PageDown => row + 10,
            _ => row.saturating_sub(10),
        });
    }
}

/// A /continue reset: the Ticket's panes close, its run directory moves
/// aside as <id>.reset-<n> (the evidence stays, and no result in it is
/// accepted again), and it starts over at Implement. A directory that
/// cannot move refuses the reset.
fn reset_ticket(o: &Orchestrator, id: &str) -> io::Result<()> {
    for pane in o.ticket(id).panes.values() {
        let _ = o.herdr(&["pane", "close", pane]);
    }
    let dir = o.run_dir(id);
    if dir.exists() {
        let aside = (1..)
            .map(|n| dir.with_file_name(format!("{id}.reset-{n}")))
            .find(|path| !path.exists())
            .unwrap();
        fs::rename(&dir, aside)?;
    }
    o.update(id, |ts| {
        *ts = TicketState {
            status: STATUS_RUNNING.to_string(),
            stage: "implement".to_string(),
            ..Default::default()
        }
    });
    Ok(())
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
    screen.close();
    if screen.reexec {
        // An idle-Shell update: the same argv comes back under the new version.
        eprintln!("harness: {}", update::reexec(&screen.cfg.exe));
    }
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
        if screen.quit {
            break; // an idle update installed: no key may start a run the re-exec ends
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
