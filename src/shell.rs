//! The Shell: the full-terminal screen that `harness` alone opens (ADR 0004).
//! `Screen` is the plain state the tests drive; `open` wraps it in the
//! terminal and the one draw, poll and tick loop (ADR 0003). The Shell owns
//! the Orchestrator: the scheduler runs on a thread of this process, its
//! Events come over a channel into RECENT and the log, and the TICKETS rows
//! are a snapshot of its State. An Event that asks (a Wake, a blocked
//! session) becomes a Question, whose answer goes back to the Orchestrator
//! for that session; the Orchestrator knows no Shell type.

use std::cell::{Cell, OnceCell, RefCell};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
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
    acquire_lock, load_state, Lock, Review, State, TicketState, STATUS_MERGED, STATUS_PARKED,
    STATUS_PR_OPEN, STATUS_RUNNING,
};
use crate::setup;
use crate::tools::Tools;
use crate::update::{self, Checked, Ready, Releases};
use summary::Summary;

mod config;
mod draw;
mod logo;
mod summary;

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
/// Every command the Shell takes: its name, arguments and what it does. The
/// / list shows it, and the README's table.
const COMMANDS: [(&str, &str, &str); 12] = [
    (
        "/start-epic",
        "<epic> [--max N]",
        "run every Ticket of an open Epic",
    ),
    ("/start-ticket", "<ticket>", "run one Ticket"),
    (
        "/continue",
        "[<ticket>]",
        "resume the saved run, or unpark one Ticket",
    ),
    ("/stop-work", "", "stop the run, the panes stay"),
    (
        "/retry",
        "<ticket>",
        "the Ticket's Stage again, in a fresh session",
    ),
    ("/park", "<ticket>", "take a Ticket out to wait for you"),
    (
        "/address",
        "<ticket>",
        "resolve a PR's conflicts or review comments",
    ),
    ("/questions", "", "show the hidden Questions"),
    (
        "/config",
        "",
        "the App, model and effort each Stage runs on",
    ),
    (
        "/away",
        "",
        "a Stage's question parks its Ticket, again to turn off",
    ),
    (
        "/summary",
        "[<epic>]",
        "the Epic's PRs, Rounds and Findings",
    ),
    ("/exit", "", "leave the Harness"),
];

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
    /// The Epic summary has opened by itself, which it does once a run.
    summarized: bool,
    _lock: Lock,
}

/// What a yes/no confirmation does on yes.
pub(crate) enum Pending {
    /// Discard the saved run and start this Epic or Ticket.
    Start { id: String, max: usize, epic: bool },
    /// Stop the run and exit.
    Exit,
    /// Close the done Epic in bd: its completed State, which poll has
    /// cleared from the Shell, for the summary in the reason.
    Close(State),
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
    /// The first row of a plan shown, which the modal's reading keys move;
    /// the draw, which knows the width, keeps it inside the plan.
    pub(crate) scroll: Cell<usize>,
    /// When a plan's modal first drew it, for its count of the lines since.
    pub(crate) opened: OnceCell<chrono::DateTime<chrono::Local>>,
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
    /// The open list's cursor row, back to the top on every key that types.
    pub(crate) pick: usize,
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
    /// /config, while it is open: it takes every key but Ctrl-C.
    pub(crate) settings: Option<config::Settings>,
    /// The Ticket /continue @ticket unparked: its next Question goes ahead
    /// of every other Ticket's.
    first: Option<String>,
    /// The docked plan's page, or the summary's, at the last draw, which
    /// PageUp, PageDown and Space move by.
    pub(crate) page: Cell<usize>,
    /// The rows the docked plan's headings, or the summary's Tickets, start
    /// on at the last draw, for Tab and Shift-Tab.
    pub(crate) heads: RefCell<Vec<usize>>,
    /// The updater thread's checks, applied between commands in poll().
    update_sender: Sender<Checked>,
    update_receiver: Receiver<Checked>,
    /// A release downloaded while a run holds the lock: installed when it ends.
    pub(crate) update: Option<Ready>,
    /// When a pending update may try the lock again, from tick().
    pub(crate) retry: Instant,
    /// An idle-Shell update installed: open() re-execs after the terminal is back.
    pub(crate) reexec: bool,
    /// The Epic summary, over the whole terminal while open.
    pub(crate) summary: Option<Summary>,
    /// The Epic of the last run to finish, whose done Epic cleared the
    /// State: what /summary alone shows next. In memory only.
    last_epic: String,
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
            pick: 0,
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
            settings: None,
            first: None,
            page: Cell::new(0),
            heads: RefCell::new(Vec::new()),
            update_sender,
            update_receiver,
            update: None,
            retry: Instant::now(),
            reexec: false,
            summary: None,
            last_epic: String::new(),
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
            away: Arc::default(), // off each time the Shell opens
            log: Arc::new(Mutex::new(Box::new(io::sink()))),
            events: mpsc::channel().0,
            clock: Arc::new(chrono::Local::now),
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
    /// closes, when a run ends and before the Epic summary opens by
    /// itself. Whether bd answered.
    fn reload_epics(&mut self) -> bool {
        match load_epics(&self.cfg.repo, &*self.cfg.tools) {
            Ok(epics) => {
                self.epics = epics;
                true
            }
            Err(err) => {
                self.notice(&format!("bd list failed: {err}"), NOTICE_WINDOW);
                false
            }
        }
    }

    /// Takes the Events and snapshots the live run's State, after the
    /// scheduler's end so a finished run's snapshot is its last; in an Epic
    /// run the summary opens by itself once every Ticket has its PR. Once
    /// the scheduler thread has returned the run is stopping until every
    /// Ticket thread has left (each sees stop at its next sleep, and still
    /// saves state after its current Tools call); then the run is over, the
    /// lock goes, a stopped run says so, and a done Epic clears the saved
    /// run and asks whether to close the Epic.
    pub(crate) fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            self.push(event);
        }
        while let Ok(checked) = self.update_receiver.try_recv() {
            self.updated(checked);
        }
        self.probed();
        self.finished();
        let Some(run) = &mut self.run else {
            return;
        };
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
        let run = self.run.as_ref().unwrap();
        self.state = run.o.state.lock().unwrap().clone();
        // the cache again once it says done: a Ticket added since has no PR
        if run.epic
            && !run.summarized
            && self.all_prs_open()
            && self.reload_epics()
            && self.all_prs_open()
        {
            self.run.as_mut().unwrap().summarized = true;
            let epic = self.state.epic.clone();
            self.summarize(&epic);
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
        self.first = None;
        if run.o.stopping() {
            // a long usage limit has said it closed the panes
            if !run.o.closed() {
                self.say("stopped, panes left running, /continue resumes");
            }
        } else if run.failed {
            self.state = load_state(&self.cfg.repo).unwrap_or_default();
        } else if run.epic {
            let done = std::mem::take(&mut self.state); // Epic done: nothing to resume
            let epic = done.epic.clone();
            if let Err(err) = self.state.save(&self.cfg.repo) {
                self.notice(&format!("state not saved: {err}"), NOTICE_WINDOW);
            }
            let text = match self.epics.iter().find(|e| e.id == epic) {
                Some(e) => format!("close Epic {epic} {}?", e.title),
                None => format!("close Epic {epic}?"),
            };
            self.confirm(&text, Pending::Close(done));
            self.last_epic = epic;
        }
        self.reload_epics();
        drop(run); // the lock goes
        self.install(false); // the last act of /stop-work
    }

    /// Whether every Ticket of the run's Epic on the bd tree has its PR open
    /// or merged or is Parked, one at least with its PR: the Epic summary's
    /// EPIC DONE, from the live State.
    fn all_prs_open(&self) -> bool {
        let Some(epic) = self.epics.iter().find(|e| e.id == self.state.epic) else {
            return false;
        };
        let status = |t: &BdIssue| match self.state.tickets.get(&t.id) {
            _ if t.status == "closed" => STATUS_MERGED,
            Some(ts) => ts.status.as_str(),
            None => "",
        };
        let out = [STATUS_PR_OPEN, STATUS_MERGED, STATUS_PARKED];
        epic.tickets.iter().all(|t| out.contains(&status(t)))
            && epic.tickets.iter().any(|t| status(t) != STATUS_PARKED)
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
            Ask::Blocked { .. }
            | Ask::Plan { .. }
            | Ask::Limited { .. }
            | Ask::StageQuestion { .. } => text.as_str(),
        };
        let asking = format!("asking you: {short}");
        // /continue @ticket's goes after a confirmation, and the Question
        // a prompt is being typed for, only
        let at = match self.first.as_deref() == Some(id.as_str()) {
            true => {
                self.first = None;
                self.hidden = false;
                let confirms = self.questions.iter().take_while(|q| q.ticket.is_none());
                confirms.count().max(usize::from(self.composing))
            }
            false => self.questions.len(),
        };
        self.questions.insert(
            at,
            Question {
                ticket: Some(id.clone()),
                text,
                about: About::Asked(ask),
                cursor: 0,
                scroll: Cell::new(0),
                opened: OnceCell::new(),
            },
        );
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

    /// Whether the front Question is a plan, which docks in the modal.
    pub(crate) fn modal(&self) -> bool {
        self.showing() && matches!(self.questions[0].about, About::Asked(Ask::Plan { .. }))
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
            About::Asked(Ask::Limited { fallback, .. }) => ["wait for the reset".to_string()]
                .into_iter()
                .chain(fallback.iter().map(|f| format!("review with {f}")))
                .chain(["open the PR unreviewed".to_string()])
                .collect(),
            About::Asked(Ask::StageQuestion { options, .. }) => options
                .iter()
                .cloned()
                .chain(["an answer of your own", "open the pane", "park"].map(str::to_string))
                .collect(),
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

    /// The list open above the input line, each row what it fills in, its
    /// middle column and its text; empty when none is. '/' with no space yet
    /// lists the commands containing it, else those it is a subsequence of.
    /// '@<query>' ending the line lists the open Epics and Tickets whose id
    /// contains it, then whose title does, then whose id it is a subsequence
    /// of; after a command, only what it takes. None opens on a prompt of
    /// your own.
    pub(crate) fn list(&self) -> Vec<(&str, &str, &str)> {
        if self.composing {
            return Vec::new();
        }
        let input = self.input.to_lowercase();
        if input.starts_with('/') && !input.contains(' ') {
            let containing: Vec<_> = COMMANDS
                .into_iter()
                .filter(|c| c.0.contains(&input))
                .collect();
            if !containing.is_empty() {
                return containing;
            }
            return COMMANDS
                .into_iter()
                .filter(|c| subsequence(&input, c.0))
                .collect();
        }
        let Some((before, q)) = input
            .rsplit_once('@')
            .filter(|(before, q)| !q.contains(' ') && (before.is_empty() || before.ends_with(' ')))
        else {
            return Vec::new();
        };
        // What the command before it takes, by its args in COMMANDS.
        let takes = COMMANDS
            .iter()
            .find(|c| Some(c.0) == before.split(' ').next())
            .map_or("", |c| c.1);
        let (epics, tickets) = (!takes.contains("<ticket>"), !takes.contains("<epic>"));
        let mut found = Vec::new();
        for e in &self.epics {
            if epics {
                found.push((e.id.as_str(), "Epic", e.title.as_str()));
            }
            for t in e.tickets.iter().filter(|t| tickets && t.status != "closed") {
                found.push((t.id.as_str(), "Ticket", t.title.as_str()));
            }
        }
        let rank = |(id, _, title): &(&str, &str, &str)| {
            let (id, title) = (id.to_lowercase(), title.to_lowercase());
            [id.contains(q), title.contains(q), subsequence(q, &id)]
                .iter()
                .position(|hit| *hit)
        };
        found.retain(|row| rank(row).is_some());
        found.sort_by_key(rank);
        found
    }

    /// Tab or Enter on an open list: a command fills in as '<command> ', an
    /// Epic or Ticket id in place of '@<query>', a space after either.
    fn fill(&mut self, picked: &str) {
        let at = self.input.rfind('@').unwrap_or(0);
        self.input.truncate(at);
        self.input.push_str(picked);
        self.input.push(' ');
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
        // The Epic summary reads like the plan and takes nothing else; Esc
        // closes it.
        if let Some(summary) = &self.summary {
            match key.code {
                KeyCode::Esc => self.summary = None,
                code => self.scroll_rows(&summary.scroll, code),
            }
            return;
        }
        if self.settings.is_some() {
            return self.config_key(key.code, held);
        }
        // With a Question showing and the input line empty the keys are
        // its: arrows or a number pick, Enter answers, Esc hides or cancels,
        // Space toggles a /continue row, y and n answer a confirmation; a
        // plan reads like a pager, ↑↓ a line, PageUp, PageDown and Space a
        // page, Home and End, Tab and Shift-Tab heading to heading, and ←→
        // pick. A slash starts a command.
        if self.showing() && self.input.is_empty() && !self.composing {
            let n = self.options().len();
            let confirm = matches!(self.questions[0].about, About::Confirm(_));
            let plan = self.modal();
            let q = &mut self.questions[0];
            match key.code {
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Char(' ')
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Tab
                | KeyCode::BackTab
                    if plan =>
                {
                    self.scroll_rows(&self.questions[0].scroll, key.code)
                }
                KeyCode::Up | KeyCode::Left => q.cursor = q.cursor.saturating_sub(1),
                KeyCode::Down | KeyCode::Right => q.cursor = (q.cursor + 1).min(n - 1),
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
                KeyCode::Char(c) if !held => {
                    self.input.push(c);
                    self.pick = 0;
                }
                _ => {}
            }
            return;
        }
        // With a list open Up and Down move its cursor, Tab fills in its row
        // and so does Enter, but on a command typed whole that needs no
        // argument Enter runs it. With the input line empty, Up and Down (and
        // the wheel, which the terminal sends as them) scroll RECENT; PageUp
        // and PageDown TICKETS.
        let (open, picked, whole, pick) = {
            let list = self.list();
            // a reloaded bd cache may have shortened the list under the cursor
            let pick = self.pick.min(list.len().saturating_sub(1));
            let row = list.get(pick).copied();
            // an optional argument ([...]) may be left out
            let whole =
                row.is_some_and(|(name, args, _)| !args.starts_with('<') && name == self.input);
            (list.len(), row.map(|row| row.0.to_string()), whole, pick)
        };
        self.pick = pick;
        let scroll = |rows: &Cell<usize>, by: isize| rows.set(rows.get().saturating_add_signed(by));
        match key.code {
            KeyCode::Char(_) if held => {}
            KeyCode::Char(c) => {
                self.input.push(c);
                self.pick = 0;
            }
            KeyCode::Up if open > 0 => self.pick = self.pick.saturating_sub(1),
            KeyCode::Down if open > 0 => self.pick = (self.pick + 1).min(open - 1),
            KeyCode::Tab if picked.is_some() => self.fill(&picked.unwrap()),
            KeyCode::Enter if picked.is_some() && !whole => self.fill(&picked.unwrap()),
            KeyCode::PageDown | KeyCode::PageUp if self.composing => {
                if self.modal() {
                    self.scroll_rows(&self.questions[0].scroll, key.code);
                }
            }
            KeyCode::Down if self.input.is_empty() => scroll(&self.recent, -1),
            KeyCode::Up if self.input.is_empty() => scroll(&self.recent, 1),
            KeyCode::PageDown if self.input.is_empty() => scroll(&self.scroll, 10),
            KeyCode::PageUp if self.input.is_empty() => scroll(&self.scroll, -10),
            KeyCode::Backspace => {
                self.input.pop();
                self.pick = 0;
            }
            KeyCode::Esc if self.composing => {
                self.composing = false;
                self.input.clear();
            }
            KeyCode::Esc if self.input.is_empty() => self.hidden = false,
            KeyCode::Esc => self.input.clear(),
            KeyCode::Enter if self.composing => {
                let prompt = std::mem::take(&mut self.input);
                if !prompt.trim().is_empty() {
                    self.composing = false;
                    let word = match self.questions[0].about {
                        About::Asked(Ask::Plan { .. }) => "feedback",
                        About::Asked(Ask::StageQuestion { .. }) => "your answer",
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

    /// The reading keys of the plan modal and the Epic summary over its
    /// first row shown: a line, a page, either end, the next or previous
    /// heading (a Ticket, in the summary); any other key none. The draw
    /// keeps the row inside.
    fn scroll_rows(&self, scroll: &Cell<usize>, code: KeyCode) {
        let (row, page) = (scroll.get(), self.page.get());
        let heads = self.heads.borrow();
        scroll.set(match code {
            KeyCode::Up => row.saturating_sub(1),
            KeyCode::Down => row.saturating_add(1),
            KeyCode::PageUp => row.saturating_sub(page),
            KeyCode::Home => 0,
            KeyCode::End => usize::MAX,
            KeyCode::Tab => heads.iter().copied().find(|&h| h > row).unwrap_or(row),
            KeyCode::BackTab => heads.iter().copied().rfind(|&h| h < row).unwrap_or(0),
            KeyCode::PageDown | KeyCode::Char(' ') => row.saturating_add(page),
            _ => row,
        });
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
            // wait, review with the fallback, open the PR unreviewed: the
            // answer stands for every Review on the App until the reset
            (About::Asked(Ask::Limited { app, .. }), n) => {
                let (app, options) = (app.clone(), self.options());
                let answer = match n {
                    0 => Review::Wait,
                    n if n + 1 == options.len() => Review::Unreviewed,
                    _ => Review::Fallback,
                };
                let q = self.questions.remove(0);
                self.tell(
                    q.ticket.as_deref(),
                    &format!("you answered: {}", options[n]),
                );
                if let Some(run) = &self.run {
                    run.o.review(&app, answer);
                }
            }
            // the Stage's options, an answer of your own, open the pane, park
            (About::Asked(Ask::StageQuestion { options, pane, .. }), n) => {
                match (options.get(n).cloned(), n.saturating_sub(options.len())) {
                    (Some(option), _) => self.reply(&option.clone(), Answer::Prompt(option)),
                    (None, 0) => self.composing = true,
                    (None, 1) => self.open_pane(pane.clone()),
                    (None, _) => self.reply("park", Answer::Act(Action::Park)),
                }
            }
            (About::Confirm(_), 0) => {
                let About::Confirm(pending) = self.questions.remove(0).about else {
                    unreachable!()
                };
                match pending {
                    Pending::Start { id, max, epic } => self.start(&id, max, epic, true),
                    Pending::Exit => self.quit(),
                    Pending::Close(done) => self.close_epic(&done),
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

    /// Closes the done Epic in bd, its summary in the reason, after a comment
    /// that lists each Ticket with its PR, from the 'PR merged: <url>'
    /// poll_merges closed it with; a Ticket closed any other way has no PR.
    /// The summary is of the run's completed State, Parked Tickets and all.
    fn close_epic(&mut self, done: &State) {
        let epic = done.epic.as_str();
        let tickets = self
            .epics
            .iter()
            .find(|e| e.id == epic)
            .map(|e| &e.tickets[..]);
        let lines: Vec<String> = tickets
            .unwrap_or_default()
            .iter()
            .map(|t| {
                let reason = &t.close_reason;
                let pr = reason
                    .strip_prefix("PR merged: ")
                    .filter(|url| !url.is_empty())
                    .unwrap_or("no PR");
                format!("- {} {}: {pr}", t.id, t.title)
            })
            .collect();
        let comment = format!("Every Ticket merged:\n{}", lines.join("\n"));
        let (tools, repo) = (&self.cfg.tools, &self.cfg.repo);
        let reason = match bd_list(repo, &**tools)
            .and_then(|issues| Summary::build(repo, &issues, done, epic))
        {
            Ok(summary) => format!("every Ticket merged\n\n{}", draw::plain(&summary)),
            Err(_) => "every Ticket merged".to_string(), // no evidence: the reason alone
        };
        let closed = tools
            .run(repo, &["bd", "comments", "add", epic, &comment])
            .and_then(|_| tools.run(repo, &["bd", "close", epic, "--reason", &reason]));
        match closed {
            Ok(_) => _ = self.reload_epics(),
            Err(err) => self.notice(&err.to_string(), NOTICE_WINDOW),
        }
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
                | Ask::PlanFailed { pane, .. }
                | Ask::StageQuestion { pane, .. },
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
            opened: OnceCell::new(),
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
        let query = query.strip_prefix('@').unwrap_or(query); // '@<id>' typed, not picked
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
            "/continue" if !query.is_empty() => self.continue_ticket(query),
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
                rows.sort_by_key(|(id, _)| suffix_order(id));
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
                        opened: OnceCell::new(),
                    });
                }
            }
            // An Epic by its id, as the @ list fills it in; closed ones too.
            "/summary" => {
                let epic = if !query.is_empty() {
                    query.to_string()
                } else if !self.state.epic.is_empty() {
                    self.state.epic.clone()
                } else if !self.last_epic.is_empty() {
                    self.last_epic.clone()
                } else {
                    return self
                        .notice("no Epic run yet, /summary @<epic> shows one", NOTICE_WINDOW);
                };
                self.summarize(&epic);
            }
            "/questions" => match self.questions.is_empty() {
                true => self.notice("no questions waiting", NOTICE_WINDOW),
                false => self.hidden = false,
            },
            "/stop-work" => self.stop_work(),
            "/config" => self.open_config(),
            "/away" => {
                let away = !self.cfg.away.fetch_xor(true, Ordering::SeqCst);
                self.say(match away {
                    true => "away: on, a Stage's question parks its Ticket",
                    false => "away: off",
                });
            }
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

    /// /continue @ticket: unparks that one Ticket at its Stage, watching its
    /// live session, and puts its next Question first; Away goes off, the
    /// user being back. With no run live the saved run resumes; in a live
    /// Epic run it is the scheduler's command, as /retry is.
    fn continue_ticket(&mut self, id: &str) {
        let parked = self
            .state
            .tickets
            .get(id)
            .is_some_and(|ts| ts.status == STATUS_PARKED);
        match self.run.as_ref().map(|run| (run.epic, run.o.clone())) {
            _ if !parked => {
                return self.refuse(&format!("refused: Ticket {} is not parked", suffix(id)))
            }
            None => {}
            Some(_) if self.stopping() => return self.refuse("refused: a run is stopping"),
            // ponytail: a single-Ticket run has no scheduler to consume it
            Some((false, _)) => return self.tell(Some(id), "continue refused: not an Epic run"),
            Some((true, o)) => o.command(&format!("continue-{id}")),
        }
        self.first = Some(id.to_string());
        if self.cfg.away.swap(false, Ordering::SeqCst) {
            self.say("away: off");
        }
        if self.run.is_none() {
            self.resume(&[(id.to_string(), false)]);
        }
    }

    /// The one open Epic (or Ticket) an argument names, a leading @ stripped:
    /// its id exactly, or the only one whose id or title contains it.
    /// Anything else is a notice.
    fn resolve(&mut self, query: &str, epics: bool) -> Option<String> {
        let query = query.strip_prefix('@').unwrap_or(query); // after --max N
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

    /// Opens the Epic summary, built fresh from bd, the State and the Run
    /// directories; a failure, or an Epic with no evidence, is a notice.
    fn summarize(&mut self, epic: &str) {
        let repo = &self.cfg.repo;
        let built = bd_list(repo, &*self.cfg.tools)
            .and_then(|issues| Summary::build(repo, &issues, &self.state, epic));
        match built {
            Ok(summary) => self.summary = Some(summary),
            Err(err) => self.notice(&err, NOTICE_WINDOW),
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
            summarized: false,
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

/// Whether `q`'s characters appear in `text` in order.
fn subsequence(q: &str, text: &str) -> bool {
    let mut chars = text.chars();
    q.chars().all(|c| chars.any(|t| t == c))
}

/// The child suffix of a bd id: harness-kqe.9 is 9.
pub(crate) fn suffix(id: &str) -> &str {
    id.rsplit_once('.').map_or(id, |(_, s)| s)
}

/// Where an id sorts among its siblings: by its child suffix as a number,
/// one with none last.
fn suffix_order(id: &str) -> usize {
    suffix(id).parse().unwrap_or(usize::MAX)
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

/// Every issue, Epics and closed ones too, from one bd list call.
fn bd_list(repo: &Path, tools: &dyn Tools) -> Result<Vec<BdIssue>, String> {
    let out = tools
        .run(repo, &["bd", "list", "--json", "--brief", "--all"])
        .map_err(|err| err.to_string())?;
    let issues: Option<Vec<BdIssue>> =
        serde_json::from_str(&out).map_err(|err| format!("unreadable reply: {err}"))?;
    Ok(issues.unwrap_or_default())
}

/// Every open Epic expanded into its Tickets, from one bd list call.
fn load_epics(repo: &Path, tools: &dyn Tools) -> Result<Vec<Epic>, String> {
    let mut issues = bd_list(repo, tools)?;
    let mut epics: Vec<Epic> = issues
        .iter()
        .filter(|i| i.issue_type == "epic" && i.status != "closed")
        .map(|i| Epic {
            id: i.id.clone(),
            title: i.title.clone(),
            tickets: Vec::new(),
        })
        .collect();
    issues.sort_by_key(|i| suffix_order(&i.id));
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
mod config_test;
#[cfg(test)]
mod shell_test;
