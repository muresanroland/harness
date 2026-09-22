//! The Orchestrator and its Config, the Stage table and the Stage loop: one
//! Stage run to its completion rule, with the Wake hold when it cannot advance.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::herdr::{agent_name, split_target};
use super::result::{read_stage_result, stage_prompt, ResultRequirements, StageResult};
use super::state::{load_state, State, TicketState, STATUS_RUNNING};
use super::trust::trusts;
use crate::tools::{RunError, Tools};

/// One step of the Pipeline, carried out by a fresh agent session in its own
/// pane.
pub(crate) struct Stage {
    pub(crate) name: &'static str,
    pub(crate) skill: &'static str,
    /// herdr agent kind: claude or codex.
    pub(crate) kind: &'static str,
    pub(crate) timeout: Duration,
}

const fn stage(name: &'static str, skill: &'static str, kind: &'static str, minutes: u64) -> Stage {
    Stage {
        name,
        skill,
        kind,
        timeout: Duration::from_secs(minutes * 60),
    }
}

pub(crate) const IMPLEMENT: Stage = stage("implement", "stage-implement", "claude", 60);
pub(crate) const REVIEW: Stage = stage("review", "stage-review", "codex", 30);
pub(crate) const DEBATE: Stage = stage("debate", "stage-moderate", "claude", 30);
pub(crate) const FIX: Stage = stage("fix", "stage-fix", "claude", 60);
pub(crate) const ADDRESS: Stage = stage("address", "stage-address", "claude", 60);

/// How a Stage ends other than with an accepted result.
#[derive(Debug, PartialEq)]
pub(crate) enum StageError {
    /// The Ticket left the Pipeline; the reason is what the state file keeps.
    Parked(String),
    /// 'harness stop' arrived: a clean end, the Ticket resumes on restart.
    Stopped,
}

/// How long a just-prompted session may still look idle before an idle pane
/// with no result counts as a Stage that did not write one.
const SETTLE_TICKS: u32 = 3;

/// What 'harness start' knows at launch.
pub(crate) struct Config {
    /// The seam to every external tool.
    pub(crate) tools: Arc<dyn Tools>,
    /// The Target repo's root.
    pub(crate) repo: PathBuf,
    /// HERDR_PANE_ID of the launching pane, which every event line is sent to.
    pub(crate) main_pane: String,
    /// HERDR_WORKSPACE_ID.
    pub(crate) workspace: String,
    /// TYPESAFE_API_KEY, handed to the Debate pane.
    pub(crate) api_key: String,
    /// Where the agents record which directories they trust.
    pub(crate) home: PathBuf,
    /// How often holds, control files and bd are polled.
    pub(crate) tick: Duration,
    /// How often gh is asked about open PRs.
    pub(crate) poll_prs: Duration,
    /// Tickets in the Pipeline at once.
    pub(crate) max: usize,
    /// Where every event line goes.
    pub(crate) log: Mutex<Box<dyn Write + Send>>,
    /// Every Stage's deadline in the tests, where Go's set
    /// stageImplement.Timeout; None is the Stage table's.
    #[cfg(test)]
    pub(crate) timeout: Option<Duration>,
}

/// Owns Ticket state, pane placement and Stage transitions. It makes no
/// judgment calls: what it cannot advance by rule becomes a Wake.
pub(crate) struct Orchestrator {
    pub(crate) cfg: Config,
    /// Never held across a sleep or a Tools call.
    pub(crate) state: Mutex<State>,
    /// 'harness stop' arrived: every sleep checks it (ADR 0003).
    pub(crate) stop: AtomicBool,
    /// One line at a time into the launching pane.
    pub(crate) unsent: Mutex<Unsent>,
    /// The Tickets running on a thread of this process.
    pub(crate) active: Mutex<BTreeSet<String>>,
    /// The Ticket threads, which the binary never joins; the tests do, so a
    /// failure on one fails the test as Go's t.Errorf did.
    #[cfg(test)]
    pub(crate) threads: Mutex<Vec<thread::JoinHandle<()>>>,
}

#[derive(Default)]
pub(crate) struct Unsent {
    /// Lines the launching pane has not taken yet, oldest first.
    pub(crate) lines: Vec<String>,
    /// The launching pane hosts no agent: events go to the log alone.
    no_main: bool,
}

impl Orchestrator {
    /// Loads the Target repo's state file, so a restarted Orchestrator resumes.
    pub(crate) fn new(mut cfg: Config) -> io::Result<Arc<Self>> {
        let state = load_state(&cfg.repo)?;
        if cfg.home.as_os_str().is_empty() {
            cfg.home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
        }
        Ok(Arc::new(Self::with_state(cfg, state)))
    }

    /// The fake world's constructor: a Config and a State, no file read.
    pub(crate) fn with_state(cfg: Config, state: State) -> Self {
        Orchestrator {
            cfg,
            state: Mutex::new(state),
            stop: AtomicBool::new(false),
            unsent: Mutex::new(Unsent::default()),
            active: Mutex::new(BTreeSet::new()),
            #[cfg(test)]
            threads: Mutex::new(Vec::new()),
        }
    }

    /// How long a Stage's session may run.
    fn timeout(&self, st: &Stage) -> Duration {
        #[cfg(test)]
        if let Some(timeout) = self.cfg.timeout {
            return timeout;
        }
        st.timeout
    }

    fn timed_out(&self, st: &Stage) -> String {
        format!("timed out after {}", go_duration(self.timeout(st)))
    }

    pub(crate) fn run_dir(&self, ticket: &str) -> PathBuf {
        self.cfg.repo.join(".harness").join("runs").join(ticket)
    }

    pub(crate) fn worktree(&self, ticket: &str) -> PathBuf {
        self.cfg
            .repo
            .join(".harness")
            .join("worktrees")
            .join(ticket)
    }

    // ponytail: no timestamp yet; the log line format is harness-kqe.8's
    // (events), which brings chrono for the local time.
    pub(crate) fn log(&self, line: &str) {
        let mut log = self.cfg.log.lock().unwrap();
        let _ = writeln!(log, "{line}");
    }

    /// Sends one event line into the launching pane. herdr refuses a prompt
    /// while the agent there is blocked on a prompt of its own, so a line
    /// that cannot be delivered is kept and sent, in order, once it can be.
    pub(crate) fn report(&self, event: &str) {
        let line = format!("[harness] {event}");
        self.log(&line);
        self.unsent.lock().unwrap().lines.push(line);
        self.flush();
    }

    pub(crate) fn flush(&self) {
        let mut unsent = self.unsent.lock().unwrap();
        if self.cfg.main_pane.is_empty() || unsent.no_main {
            unsent.lines.clear(); // every line is in the log already
            return;
        }
        while let Some(line) = unsent.lines.first() {
            let argv = ["herdr", "agent", "prompt", &self.cfg.main_pane, line];
            match self.cfg.tools.run(&self.cfg.repo, &argv) {
                Ok(_) => {
                    unsent.lines.remove(0);
                }
                Err(err) if err.to_string().contains("agent_not_found") => {
                    // Launched from a shell pane rather than a Claude session:
                    // there is nobody to tell, and retrying every line forever
                    // buries the log in the failure.
                    self.log("the launching pane hosts no agent; events are in this log only");
                    unsent.no_main = true;
                    unsent.lines.clear();
                    return;
                }
                Err(err) => {
                    self.log(&format!(
                        "the launching pane did not take {line:?}, will retry: {err}"
                    ));
                    return;
                }
            }
        }
    }

    /// Whether the run ended on 'harness stop'.
    pub(crate) fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Changes one Ticket's state and writes the state file.
    pub(crate) fn update(&self, ticket: &str, change: impl FnOnce(&mut TicketState)) {
        let mut state = self.state.lock().unwrap();
        let ts = state
            .tickets
            .entry(ticket.to_string())
            .or_insert_with(|| TicketState {
                status: STATUS_RUNNING.to_string(),
                ..Default::default()
            });
        change(ts);
        if let Err(err) = state.save(&self.cfg.repo) {
            self.log(&format!("state not saved: {err}"));
        }
    }

    /// A snapshot of one Ticket's state; the default for an unknown Ticket.
    pub(crate) fn ticket(&self, ticket: &str) -> TicketState {
        self.state
            .lock()
            .unwrap()
            .tickets
            .get(ticket)
            .cloned()
            .unwrap_or_default()
    }
}

/// The name of a Stage's result file in the run directory.
pub(crate) fn result_name(st: &Stage, round: usize) -> String {
    match st.name {
        "debate" => format!("verdict-{round}.md"),
        "review" | "fix" => format!("{}-{round}.md", st.name),
        _ => format!("{}.md", st.name),
    }
}

/// How long to ask herdr to wait: one tick, never past the deadline.
fn wait_for(deadline: Instant, tick: Duration) -> String {
    let left = deadline.saturating_duration_since(Instant::now()).min(tick);
    left.as_millis().max(1).to_string()
}

/// Where a control command (stop, retry-<ticket>, park-<ticket>,
/// address-<ticket>) is left for the running Orchestrator.
// ponytail: a polled directory; a socket if latency ever matters.
pub(crate) fn control_file(repo: &Path, name: &str) -> PathBuf {
    repo.join(".harness").join("control").join(name)
}

impl Orchestrator {
    /// Runs one Stage to its completion rule and returns its accepted result.
    /// A Stage with an already accepted result is not rerun. When it cannot
    /// advance by rule the launching pane is woken and the Ticket holds for a
    /// retry, a park, or a late done result.
    pub(crate) fn run_stage(
        &self,
        ticket: &str,
        st: &Stage,
        round: usize,
        inputs: &[(&str, &str)],
        want: ResultRequirements,
    ) -> Result<StageResult, StageError> {
        let file = self.run_dir(ticket).join(result_name(st, round));
        let (result, reason) = read_stage_result(&file, want);
        if reason.is_empty() {
            return Ok(result);
        }
        self.update(ticket, |ts| {
            if ts.stage != st.name || ts.round != round {
                // a resumed Stage keeps its spent retry
                ts.stage = st.name.to_string();
                ts.round = round;
                ts.retried = false;
            }
        });
        let (round_s, worktree, run_dir, file_s) = (
            round.to_string(),
            self.worktree(ticket).display().to_string(),
            self.run_dir(ticket).display().to_string(),
            file.display().to_string(),
        );
        let mut all = vec![
            ("Ticket", ticket),
            ("Round", &round_s),
            ("Worktree", &worktree),
            ("Run directory", &run_dir),
            ("Result file", &file_s),
        ];
        all.extend_from_slice(inputs);

        loop {
            let (result, reason) = self.attempt(ticket, st, &file, &all, want);
            if self.stopping() {
                return Err(StageError::Stopped);
            }
            if reason.is_empty() {
                return Ok(result);
            }
            let ts = self.ticket(ticket);
            if ts.retried {
                return Err(StageError::Parked(format!(
                    "{} {reason} again after a retry",
                    st.name
                )));
            }
            let pane = ts.panes.get(st.name).cloned().unwrap_or_default();
            self.consume(&format!("retry-{ticket}")); // a command sent before this Wake is not an answer to it
            self.consume(&format!("park-{ticket}"));
            self.report(&format!(
                "WAKE {ticket} {} {reason} at {}",
                st.name,
                self.locate(&pane)
            ));
            let (result, command) = self.hold(ticket, &pane, &file, want);
            match command {
                "retry" => {
                    self.update(ticket, |ts| ts.retried = true);
                    self.report(&format!(
                        "{ticket} {} retrying with a fresh session",
                        st.name
                    ));
                }
                "park" => return Err(StageError::Parked(format!("{} {reason}", st.name))),
                "done" => return Ok(result),
                _ => return Err(StageError::Stopped),
            }
        }
    }

    /// Runs the Stage once in a fresh session and returns its accepted
    /// result, or the reason it cannot complete.
    fn attempt(
        &self,
        ticket: &str,
        st: &Stage,
        file: &Path,
        inputs: &[(&str, &str)],
        want: ResultRequirements,
    ) -> (StageResult, String) {
        let none = StageResult::default();
        let skill_path = self
            .cfg
            .repo
            .join(".agents")
            .join("skills")
            .join(st.skill)
            .join("SKILL.md");
        let skill = match fs::read_to_string(&skill_path) {
            Ok(skill) => skill,
            Err(err) => {
                return (
                    none,
                    format!("has no Stage skill (run 'harness init'): {err}"),
                )
            }
        };
        if let Err(err) = fs::create_dir_all(file.parent().unwrap()) {
            return (none, err.to_string());
        }
        let reason = self.await_trust(ticket, st);
        if !reason.is_empty() {
            return (none, reason);
        }
        let pane = match self.fresh_pane(ticket, st) {
            Ok(pane) => pane,
            Err(err) => return (none, format!("got no pane: {err}")),
        };
        // The previous session can still write while its pane is closing.
        // Clear its result only after fresh_pane has replaced it, before the
        // new writer.
        let _ = fs::remove_file(file);
        let deadline = Instant::now() + self.timeout(st);

        let run_dir = self.run_dir(ticket).display().to_string();
        let agent_args: &[&str] = if st.kind == "codex" {
            // The pane's cwd is the run directory, so the sandbox lets Codex
            // write its result file there and nothing in the worktree.
            &["--sandbox", "workspace-write"]
        } else {
            &["--permission-mode", "auto", "--add-dir", &run_dir]
        };
        let name = agent_name(ticket, st.name);
        let mut start = vec![
            "agent", "start", &name, "--kind", st.kind, "--pane", &pane, "--",
        ];
        start.extend_from_slice(agent_args);
        let start_err = self.start_agent(&start).err();
        if let Some(err) = &start_err {
            if !err.to_string().contains("agent_not_ready") {
                return (none, format!("session did not start: {err}"));
            }
        }
        self.report(&format!(
            "{ticket} {} started: {} in {} -> {}",
            st.name,
            st.kind,
            self.stage_cwd(ticket, st).display(),
            self.locate(&pane)
        ));
        if start_err.is_some() {
            // blocked at startup: nothing can be prompted yet
            let reason = self.wait_unblocked(ticket, st, &pane, deadline);
            if !reason.is_empty() {
                return (none, reason);
            }
        }

        // The prompt is sent on its own rather than with --wait, so that a
        // prompt which never lands is a reason of its own. Sent together, a
        // session still sitting at a dialog reads as idle the moment the call
        // returns, and an idle pane with no result file is indistinguishable
        // from a Stage that finished and forgot to write one.
        let prompt = stage_prompt(&skill, inputs);
        if let Err(err) = self.herdr(&["agent", "prompt", &pane, &prompt]) {
            return (none, format!("did not take the prompt: {err}"));
        }
        self.report(&format!(
            "{ticket} {} prompted, waiting for {}",
            st.name,
            file.file_name().unwrap_or_default().to_string_lossy()
        ));
        // A session that has just been prompted still reads idle until it
        // takes the prompt up, which looks exactly like a Stage that finished
        // without writing a result. Give it a few ticks before believing that.
        let settled = Instant::now() + SETTLE_TICKS * self.cfg.tick;
        loop {
            if self.stopping() {
                return (none, "stopped".to_string());
            }
            match self.agent_status(&pane).as_deref() {
                None => return (none, "pane died".to_string()),
                Some("blocked") => {
                    let reason = self.wait_unblocked(ticket, st, &pane, deadline);
                    if !reason.is_empty() {
                        return (none, reason);
                    }
                }
                Some("idle" | "done") => {
                    let (result, reason) = read_stage_result(file, want);
                    if reason.is_empty() || Instant::now() > settled {
                        return (result, reason);
                    }
                }
                Some(_) if Instant::now() > deadline => {
                    return (none, self.timed_out(st));
                }
                Some(_) => {}
            }
            if !self.sleep() {
                return (none, "stopped".to_string());
            }
            // A tick at a time, never to the Stage's deadline: a wait that
            // blocks for an hour is an hour in which 'harness stop' does
            // nothing.
            let timeout = wait_for(deadline, self.cfg.tick);
            let _ = self.herdr(&["agent", "wait", &pane, "--timeout", &timeout]);
        }
    }

    /// Holds a Stage until its agent trusts the directory its pane will start
    /// in. Only the user can accept a trust dialog, so the Orchestrator names
    /// the directory and waits instead of prompting into one.
    fn await_trust(&self, ticket: &str, st: &Stage) -> String {
        if self.cfg.home.as_os_str().is_empty() {
            return String::new(); // no home, no trust stores to read: let the Stage try
        }
        let dir = self.stage_cwd(ticket, st);
        let trusted = || trusts(st.kind, &self.cfg.home, &dir, &self.cfg.repo);
        if trusted() {
            return String::new();
        }
        self.report(&format!(
            "{ticket} {} waiting: {} does not trust {} yet. Open {} there once and accept it; the Stage carries on by itself",
            st.name, st.kind, dir.display(), st.kind
        ));
        while !trusted() {
            if !self.sleep() {
                return "stopped".to_string();
            }
        }
        self.report(&format!(
            "{ticket} {}: {} trusts {} now",
            st.name,
            st.kind,
            dir.display()
        ));
        String::new()
    }

    /// The directory a Stage's pane starts in: the Ticket's worktree, or the
    /// run directory for Codex, whose sandbox may write only where it starts.
    fn stage_cwd(&self, ticket: &str, st: &Stage) -> PathBuf {
        if st.kind == "codex" {
            self.run_dir(ticket)
        } else {
            self.worktree(ticket)
        }
    }

    /// Starts the Stage's session, giving a pane that has just been created
    /// the moment it needs to get a shell: until it has one herdr refuses with
    /// agent_pane_busy, which is not the pane being unusable.
    fn start_agent(&self, argv: &[&str]) -> Result<(), RunError> {
        let give_up = Instant::now() + 6 * self.cfg.tick;
        loop {
            let err = match self.herdr(argv) {
                Ok(_) => return Ok(()),
                Err(err) => err,
            };
            if !err.to_string().contains("agent_pane_busy") || Instant::now() > give_up {
                return Err(err);
            }
            if !self.sleep() {
                return Err(err);
            }
        }
    }

    /// Wakes the launching pane about a blocked session, which only the user
    /// may answer, and waits for the session to move on.
    fn wait_unblocked(&self, ticket: &str, st: &Stage, pane: &str, deadline: Instant) -> String {
        self.report(&format!(
            "WAKE {ticket} {} blocked at {}",
            st.name,
            self.locate(pane)
        ));
        loop {
            match self.agent_status(pane).as_deref() {
                None => return "pane died".to_string(),
                Some(status) if status != "blocked" => return String::new(),
                Some(_) if Instant::now() > deadline => return self.timed_out(st),
                Some(_) => {}
            }
            if !self.sleep() {
                return "stopped".to_string();
            }
        }
    }

    /// Keeps a woken Ticket waiting, leaving every other Ticket running, until
    /// a control command arrives or the user's nudge produces a done result.
    fn hold(
        &self,
        ticket: &str,
        pane: &str,
        file: &Path,
        want: ResultRequirements,
    ) -> (StageResult, &'static str) {
        loop {
            if self.consume(&format!("retry-{ticket}")) {
                return (StageResult::default(), "retry");
            }
            if self.consume(&format!("park-{ticket}")) {
                return (StageResult::default(), "park");
            }
            let (result, reason) = read_stage_result(file, want);
            if reason.is_empty()
                && matches!(
                    self.agent_status(pane).as_deref(),
                    None | Some("idle" | "done")
                )
            {
                return (result, "done");
            }
            if !self.sleep() {
                return (StageResult::default(), "stopped");
            }
        }
    }

    /// One tick of every polling loop, and so also the moment undelivered
    /// lines are tried again. False once 'harness stop' has arrived.
    pub(crate) fn sleep(&self) -> bool {
        self.flush();
        if self.consume("stop") {
            self.stop.store(true, Ordering::SeqCst);
            self.report("stopped; live panes left alone, 'harness start' resumes the run");
            return false;
        }
        thread::sleep(self.cfg.tick);
        // Checked after the sleep: a stop that arrived during it must not buy
        // one more loop, which in a hold could start a fresh session.
        !self.stopping()
    }

    /// Gives the Stage an empty shell pane in the Ticket tab, replacing the
    /// pane of an earlier session of the same Stage (a previous Round, a
    /// retry, a resumed run) so every session starts fresh.
    fn fresh_pane(&self, ticket: &str, st: &Stage) -> Result<String, RunError> {
        let ts = self.ticket(ticket);
        if let Some(old) = ts.panes.get(st.name) {
            let _ = self.herdr(&["pane", "close", old]); // already gone is fine
        }
        let cwd = self.stage_cwd(ticket, st).display().to_string();
        let env = format!("TYPESAFE_API_KEY={}", self.cfg.api_key);
        let mut placement = vec!["--cwd", cwd.as_str(), "--no-focus"];
        if st.name == "debate" && !self.cfg.api_key.is_empty() {
            placement.extend(["--env", env.as_str()]);
        }

        let mut in_tab = Vec::new();
        if !ts.tab.is_empty() {
            if let Ok(panes) = self.herdr(&["pane", "list", "--workspace", &self.cfg.workspace]) {
                in_tab.extend(
                    panes
                        .result
                        .panes
                        .into_iter()
                        .filter(|p| p.tab_id == ts.tab)
                        .map(|p| p.pane_id),
                );
            }
        }
        let (tab, pane) = match in_tab.last() {
            None => {
                let mut argv = vec![
                    "tab",
                    "create",
                    "--workspace",
                    &self.cfg.workspace,
                    "--label",
                    ticket,
                ];
                argv.extend_from_slice(&placement);
                let reply = self.herdr(&argv)?;
                (reply.result.tab.tab_id, reply.result.root_pane.pane_id)
            }
            Some(last) => {
                let (mut target, mut direction) = (last.clone(), "right".to_string());
                if let Ok(layout) = self.herdr(&["pane", "layout", "--pane", &in_tab[0]]) {
                    let (p, d) = split_target(&layout.result.layout.panes);
                    if !p.is_empty() {
                        (target, direction) = (p, d);
                    }
                }
                let mut argv = vec!["pane", "split", &target, "--direction", &direction];
                argv.extend_from_slice(&placement);
                let reply = self.herdr(&argv)?;
                (ts.tab.clone(), reply.result.pane.pane_id)
            }
        };
        self.update(ticket, |ts| {
            ts.tab = tab;
            ts.panes.insert(st.name.to_string(), pane.clone());
        });
        Ok(pane)
    }

    /// The control files waiting for the Orchestrator.
    pub(crate) fn commands(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.cfg.repo.join(".harness").join("control")) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Drops control files left behind by an earlier run: a 'stop' sent to a
    /// process that has since died must not stop this one before it starts.
    pub(crate) fn drain_commands(&self) {
        for name in self.commands() {
            if self.consume(&name) {
                self.log(&format!("dropped {name:?}, left over from an earlier run"));
            }
        }
    }

    /// Whether a control command was waiting, and removes it.
    pub(crate) fn consume(&self, name: &str) -> bool {
        fs::remove_file(control_file(&self.cfg.repo, name)).is_ok()
    }
}

/// Go's Duration string, for the deadlines the Stage table and the tests use:
/// "1h0m0s", "30m0s", "5ms".
// ponytail: whole units only; Go also prints "1.5s" and "2m30.5s".
fn go_duration(d: Duration) -> String {
    if d < Duration::from_secs(1) {
        return format!("{}ms", d.as_millis());
    }
    let secs = d.as_secs();
    match (secs / 3600, secs % 3600 / 60, secs % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m{s}s"),
        (h, m, s) => format!("{h}h{m}m{s}s"),
    }
}

impl Config {
    /// A Config for the fake world: every duration a millisecond, three
    /// Tickets at once, events to nowhere.
    #[cfg(test)]
    pub(crate) fn for_tests(
        tools: Arc<dyn Tools>,
        repo: &std::path::Path,
        home: &std::path::Path,
    ) -> Self {
        Config {
            tools,
            repo: repo.to_path_buf(),
            main_pane: "main".to_string(),
            workspace: "w1".to_string(),
            api_key: "sk-test".to_string(),
            home: home.to_path_buf(),
            tick: Duration::from_millis(1),
            poll_prs: Duration::from_millis(1),
            max: 3,
            log: Mutex::new(Box::new(io::sink())),
            timeout: None,
        }
    }
}
