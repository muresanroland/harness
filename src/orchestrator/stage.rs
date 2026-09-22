//! The Orchestrator and its Config; the Stage table and the Stage loop follow
//! with harness-kqe.3.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::state::{load_state, State, TicketState, STATUS_RUNNING};
use crate::tools::Tools;

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
}

/// Owns Ticket state, pane placement and Stage transitions. It makes no
/// judgment calls: what it cannot advance by rule becomes a Wake.
pub(crate) struct Orchestrator {
    pub(crate) cfg: Config,
    state: Mutex<State>,
    /// 'harness stop' arrived: every sleep checks it (ADR 0003).
    pub(crate) stop: AtomicBool,
    /// One line at a time into the launching pane.
    pub(crate) unsent: Mutex<Unsent>,
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
        }
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
pub(crate) fn result_name(stage: &str, round: usize) -> String {
    match stage {
        "debate" => format!("verdict-{round}.md"),
        "review" | "fix" => format!("{stage}-{round}.md"),
        _ => format!("{stage}.md"),
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
        }
    }
}
