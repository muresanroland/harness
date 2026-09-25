//! The Orchestrator and its Config, the Stage table and the Stage loop: one
//! Stage run to its completion rule, with the Wake hold when it cannot advance.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::app::{debate_inputs, stage_row, App, Row};
use super::herdr::{agent_name, split_target};
use super::judgment::{offered, Action, Judged, TypeSafe, FLOOR};
use super::limit::{Limit, LAST_LINES};
use super::result::{
    read_question, read_stage_result, stage_prompt, ResultRequirements, StageResult, ASKED,
};
use super::state::{load_state, Session, State, TicketState, STATUS_RUNNING};
use super::trust::trusts;
use crate::tools::{RunError, Tools};

/// One step of the Pipeline, carried out by a fresh agent session in its own
/// pane.
pub(crate) struct Stage {
    pub(crate) name: &'static str,
    pub(crate) skill: &'static str,
    pub(crate) timeout: Duration,
}

const fn stage(name: &'static str, skill: &'static str, minutes: u64) -> Stage {
    Stage {
        name,
        skill,
        timeout: Duration::from_secs(minutes * 60),
    }
}

// The App each Stage runs on comes from .harness/config.json (app.rs).
pub(crate) const IMPLEMENT: Stage = stage("implement", "stage-implement", 60);
pub(crate) const REVIEW: Stage = stage("review", "stage-review", 30);
pub(crate) const DEBATE: Stage = stage("debate", "stage-moderate", 30);
pub(crate) const FIX: Stage = stage("fix", "stage-fix", 60);
pub(crate) const ADDRESS: Stage = stage("address", "stage-address", 60);

/// How a Stage ends other than with an accepted result.
#[derive(Debug, PartialEq)]
pub(crate) enum StageError {
    /// The Ticket left the Pipeline; the reason is what the state file keeps.
    Parked(String),
    /// /stop-work arrived: a clean end, the Ticket resumes on /continue.
    Stopped,
}

/// Why a Ticket whose Stage asked while the user was Away is Parked.
pub(crate) const AWAY: &str = "asked you while away";

/// How long a just-prompted session may still look idle before an idle pane
/// with no result counts as a Stage that did not write one.
pub(super) const SETTLE_TICKS: u32 = 3;

/// How long a wait holds before the Ticket Wakes again.
const WAIT: Duration = Duration::from_secs(10 * 60);

/// How a Stage's session, or its hold, ends.
pub(super) enum Held {
    Retry,
    /// /park, or park answered: the Ticket leaves the Pipeline at its Stage.
    Park,
    /// The Stage asked while the user was Away: parked, its session left
    /// waiting in its pane.
    Away,
    Done(StageResult),
    Stopped,
    /// The Stage cannot advance, for this reason: a Wake.
    Woke(String),
    /// Its session stopped at this usage limit: never a Wake.
    Limited(Limit),
}

/// What a panel line asks of the user; the Shell puts it as a Question.
#[derive(Clone, Debug)]
pub(crate) enum Ask {
    /// A Wake: the session's pane, the tail of its output, the Stage's
    /// result file (the canned nudges' subject), the actions on offer (of
    /// the nudges, the one a Judgment below the floor picked) and that
    /// Judgment.
    Wake {
        pane: String,
        tail: String,
        file: PathBuf,
        actions: Vec<Action>,
        judged: Option<Judged>,
    },
    /// A session waiting at a prompt only the user can answer.
    Blocked { pane: String },
    /// An Implement session's plan to approve: its pane, the plan, the plan
    /// Judgment's score for yes if one was had, and the user's feedback
    /// that could not be sent, to send again.
    Plan {
        pane: String,
        plan: String,
        judged: Option<f64>,
        feedback: Option<String>,
    },
    /// A plan failure, the user's alone: the session's pane, and the
    /// feedback to send again, if any.
    PlanFailed {
        pane: String,
        feedback: Option<String>,
    },
    /// The Stage's own question (STATUS: question): its pane, the question
    /// and its options.
    StageQuestion {
        pane: String,
        question: String,
        options: Vec<String>,
    },
}

/// The user's answer to a Question, for the session (pane) it was about.
#[derive(Debug)]
pub(crate) enum Answer {
    Act(Action),
    /// A prompt of the user's own: a nudge to a Wake's session, or feedback
    /// on a plan.
    Prompt(String),
    /// A plan approved.
    Approve,
}

impl Answer {
    fn word(&self) -> &'static str {
        match self {
            Answer::Act(action) => action.word(),
            Answer::Prompt(_) => "prompt",
            Answer::Approve => "approve",
        }
    }
}

/// One moment of the run, said once in plain language: the same words on the
/// Shell's RECENT panel and in the log (docs/design/events.md).
#[derive(Clone, Debug)]
pub(crate) struct Event {
    pub(crate) time: chrono::DateTime<chrono::Local>,
    /// None for a run-level line.
    pub(crate) ticket: Option<String>,
    pub(crate) text: String,
    /// Shown on the panel; false keeps housekeeping in the log alone, and
    /// with an ask it is a Question with no line of its own.
    pub(crate) panel: bool,
    /// The line asks the user something: a Wake, a blocked session, a plan,
    /// a plan failure or a Stage's own question.
    pub(crate) ask: Option<Ask>,
}

/// What the Shell knows when it starts a run: kept whole by the Shell and
/// cloned for each run, which sets exe, max, log and events.
#[derive(Clone)]
pub(crate) struct Config {
    /// The seam to every external tool.
    pub(crate) tools: Arc<dyn Tools>,
    /// The Target repo's root.
    pub(crate) repo: PathBuf,
    /// HERDR_WORKSPACE_ID.
    pub(crate) workspace: String,
    /// TYPESAFE_API_KEY, handed to the Debate pane and the Judgment while
    /// TypeSafe is on (typesafe_key).
    pub(crate) api_key: String,
    /// The harness binary, which Implement's plan hook runs: resolved once
    /// when the Shell opens, since after a self-update a fresh lookup can
    /// name the old, deleted image.
    pub(crate) exe: PathBuf,
    /// The seam to TypeSafe, which the Wake Judgment asks.
    pub(crate) typesafe: Arc<dyn TypeSafe>,
    /// Where the agents record which directories they trust, and the
    /// user-level skills are.
    pub(crate) home: PathBuf,
    /// How often holds, commands and bd are polled.
    pub(crate) tick: Duration,
    /// How often gh is asked about open PRs.
    pub(crate) poll_prs: Duration,
    /// Tickets in the Pipeline at once.
    pub(crate) max: usize,
    /// The user is Away: a Stage's question parks its Ticket. The Shell's
    /// /away flips it, shared with every run; off when the Shell opens.
    pub(crate) away: Arc<AtomicBool>,
    /// Where every event line goes.
    pub(crate) log: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Every Event, for the Shell's RECENT panel; a dropped receiver is tolerated.
    pub(crate) events: Sender<Event>,
    /// The wall clock usage-limit resets are read against; the tests set it.
    pub(crate) clock: Arc<dyn Fn() -> chrono::DateTime<chrono::Local> + Send + Sync>,
    /// Every Stage's deadline in the tests; None is the Stage table's.
    #[cfg(test)]
    pub(crate) timeout: Option<Duration>,
    /// How long a wait holds in the tests; None is WAIT.
    #[cfg(test)]
    pub(crate) wait: Option<Duration>,
}

/// Owns Ticket state, pane placement and Stage transitions. It composes no
/// text: what it cannot advance by rule becomes a Wake, which a Judgment or
/// the user answers.
pub(crate) struct Orchestrator {
    pub(crate) cfg: Config,
    /// Never held across a sleep or a Tools call.
    pub(crate) state: Mutex<State>,
    /// /stop-work arrived: every sleep checks it (ADR 0003).
    pub(crate) stop: AtomicBool,
    /// A long usage limit ended the run: each Ticket closes its tab as it
    /// leaves.
    pub(crate) closed: AtomicBool,
    /// The Shell's commands waiting to be consumed: retry-<ticket>,
    /// park-<ticket>, address-<ticket>, continue-<ticket>. Stop is the flag
    /// above.
    pub(crate) commands: Mutex<Vec<String>>,
    /// Answers to Questions, each for one session: (ticket, pane, answer).
    pub(crate) answers: Mutex<Vec<(String, String, Answer)>>,
    /// The Tickets running on a thread of this process.
    pub(crate) active: Mutex<BTreeSet<String>>,
    /// Each Ticket's live session's deadline, which a wait keeps.
    deadlines: Mutex<BTreeMap<String, Instant>>,
    /// The plan last judged for each Ticket's Implement session: a plan.md
    /// with other text is a newer one. In memory, so a restarted run judges
    /// the plan on screen again.
    pub(super) plans: Mutex<BTreeMap<String, String>>,
    /// The Ticket threads, which the binary never joins; the tests do, so a
    /// failure on one fails the test.
    #[cfg(test)]
    pub(crate) threads: Mutex<Vec<thread::JoinHandle<()>>>,
}

impl Orchestrator {
    /// Loads the Target repo's state file, so a restarted Orchestrator
    /// resumes; a config.json no Pipeline Stage can start on refuses the
    /// run. Address runs on demand, so its row Wakes it alone (attempt).
    pub(crate) fn new(cfg: Config) -> io::Result<Arc<Self>> {
        for st in [&IMPLEMENT, &REVIEW, &DEBATE, &FIX] {
            stage_row(&cfg.repo, st).map_err(io::Error::other)?;
        }
        debate_inputs(&cfg.repo, "").map_err(io::Error::other)?;
        let state = load_state(&cfg.repo)?;
        Ok(Arc::new(Self::with_state(cfg, state)))
    }

    /// The fake world's constructor: a Config and a State, no file read.
    pub(crate) fn with_state(cfg: Config, state: State) -> Self {
        Orchestrator {
            cfg,
            state: Mutex::new(state),
            stop: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            commands: Mutex::new(Vec::new()),
            answers: Mutex::new(Vec::new()),
            active: Mutex::new(BTreeSet::new()),
            deadlines: Mutex::new(BTreeMap::new()),
            plans: Mutex::new(BTreeMap::new()),
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

    /// A full Stage deadline from now for the Ticket's session.
    pub(super) fn new_deadline(&self, ticket: &str, st: &Stage) -> Instant {
        let deadline = Instant::now() + self.timeout(st);
        let mut deadlines = self.deadlines.lock().unwrap();
        deadlines.insert(ticket.to_string(), deadline);
        deadline
    }

    /// The Ticket's live session's deadline, which an approved or sent-back
    /// plan starts over; a full one from now if it has none.
    fn deadline(&self, ticket: &str, st: &Stage) -> Instant {
        let deadline = self.deadlines.lock().unwrap().get(ticket).copied();
        deadline.unwrap_or_else(|| self.new_deadline(ticket, st))
    }

    /// How long a wait holds.
    fn wait_length(&self) -> Duration {
        #[cfg(test)]
        if let Some(wait) = self.cfg.wait {
            return wait;
        }
        WAIT
    }

    fn timed_out(&self, st: &Stage) -> String {
        format!("timed out after {}", short_duration(self.timeout(st)))
    }

    pub(crate) fn run_dir(&self, ticket: &str) -> PathBuf {
        run_dir(&self.cfg.repo, ticket)
    }

    pub(crate) fn worktree(&self, ticket: &str) -> PathBuf {
        self.cfg
            .repo
            .join(".harness")
            .join("worktrees")
            .join(ticket)
    }

    /// The one way an event is said: a log line 'YYYY-MM-DD HH:MM:SS <bd id>
    /// <event>' in local time (no id for a run-level line, ticket ""), and
    /// the Event to the Shell. A `detail` (a PR's url) goes on the log line
    /// alone, in parentheses.
    pub(crate) fn emit(&self, ticket: &str, text: &str, panel: bool, detail: &str) {
        self.event(ticket, text, panel, detail, None);
    }

    /// A panel line that asks the user: the Shell puts it as a Question.
    pub(super) fn asks(&self, ticket: &str, text: &str, ask: Ask) {
        self.event(ticket, text, true, "", Some(ask));
    }

    /// A Question with no line of its own, after the lines that led to it:
    /// the Shell's "asking you" is its line.
    pub(super) fn ask_only(&self, ticket: &str, text: &str, ask: Ask) {
        let _ = self.cfg.events.send(Event {
            time: chrono::Local::now(),
            ticket: Some(ticket.to_string()),
            text: text.to_string(),
            panel: false,
            ask: Some(ask),
        });
    }

    fn event(&self, ticket: &str, text: &str, panel: bool, detail: &str, ask: Option<Ask>) {
        let time = chrono::Local::now();
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(" ({detail})")
        };
        // one write: the Shell appends its own lines to the same file
        let line = log_line(time, ticket, &format!("{text}{detail}"));
        let _ = self.cfg.log.lock().unwrap().write_all(line.as_bytes());
        let _ = self.cfg.events.send(Event {
            time,
            ticket: (!ticket.is_empty()).then(|| ticket.to_string()),
            text: text.to_string(),
            panel,
            ask,
        });
    }

    /// An event that shows on the panel.
    pub(crate) fn report(&self, ticket: &str, text: &str) {
        self.emit(ticket, text, true, "");
    }

    /// Housekeeping: in the log only.
    pub(crate) fn log(&self, ticket: &str, text: &str) {
        self.emit(ticket, text, false, "");
    }

    /// Whether the run ended on /stop-work.
    pub(crate) fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// /stop-work: scheduling ends at the next sleep, live panes stay, and
    /// the state file already holds every Ticket as saved. The Shell says
    /// "stopped" once every Ticket thread has left.
    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// A command from the Shell: retry-<ticket>, park-<ticket>,
    /// address-<ticket> or continue-<ticket>, consumed by the Ticket's own
    /// waits or the scheduler.
    pub(crate) fn command(&self, name: &str) {
        self.commands.lock().unwrap().push(name.to_string());
    }

    /// The commands waiting to be consumed.
    pub(crate) fn commands(&self) -> Vec<String> {
        self.commands.lock().unwrap().clone()
    }

    /// Whether a command was waiting, and removes it.
    pub(crate) fn consume(&self, name: &str) -> bool {
        let mut commands = self.commands.lock().unwrap();
        match commands.iter().position(|c| c == name) {
            Some(i) => {
                commands.remove(i);
                true
            }
            None => false,
        }
    }

    /// The user's answer to a Question about the session in `pane`.
    pub(crate) fn answer(&self, ticket: &str, pane: &str, answer: Answer) {
        let entry = (ticket.to_string(), pane.to_string(), answer);
        self.answers.lock().unwrap().push(entry);
    }

    /// Takes the Ticket's answer for the session in `pane`. Every other
    /// answer queued for the Ticket (all of them, for None) missed its
    /// session, which has moved on, and is dropped with a log line.
    pub(crate) fn take_answer(&self, ticket: &str, pane: Option<&str>) -> Option<Answer> {
        let mine: Vec<(String, String, Answer)> = {
            let mut all = self.answers.lock().unwrap();
            let (mine, rest) = std::mem::take(&mut *all)
                .into_iter()
                .partition(|(t, _, _)| t == ticket);
            *all = rest;
            mine
        };
        let mut found = None;
        for (_, p, answer) in mine {
            if found.is_none() && pane == Some(p.as_str()) {
                found = Some(answer);
            } else {
                self.dropped(ticket, &answer);
            }
        }
        found
    }

    pub(super) fn dropped(&self, ticket: &str, answer: &Answer) {
        self.log(
            ticket,
            &format!("dropped your {}: that session has moved on", answer.word()),
        );
    }

    /// Whether /park, or park answered for the session in `pane`, has
    /// arrived; any other answer for it is dropped.
    fn park_arrived(&self, ticket: &str, pane: &str) -> bool {
        let answered = match self.take_answer(ticket, Some(pane)) {
            Some(Answer::Act(Action::Park)) => true,
            Some(other) => {
                self.dropped(ticket, &other);
                false
            }
            None => false,
        };
        self.consume(&format!("park-{ticket}")) || answered
    }

    /// Changes one Ticket's state and writes the state file.
    pub(crate) fn update(&self, ticket: &str, change: impl FnOnce(&mut TicketState)) {
        self.change_state(|state| {
            change(
                state
                    .tickets
                    .entry(ticket.to_string())
                    .or_insert_with(|| TicketState {
                        status: STATUS_RUNNING.to_string(),
                        ..Default::default()
                    }),
            )
        });
    }

    /// Changes the state and writes the state file.
    pub(super) fn change_state(&self, change: impl FnOnce(&mut State)) {
        let mut state = self.state.lock().unwrap();
        change(&mut state);
        let saved = state.save(&self.cfg.repo);
        drop(state);
        if let Err(err) = saved {
            self.report("", &format!("state not saved: {err}"));
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

/// A log line: 'YYYY-MM-DD HH:MM:SS <bd id> <text>', no id for ticket "".
pub(crate) fn log_line(time: chrono::DateTime<chrono::Local>, ticket: &str, text: &str) -> String {
    let id = if ticket.is_empty() {
        String::new()
    } else {
        format!("{ticket} ")
    };
    format!("{} {id}{text}\n", time.format("%Y-%m-%d %H:%M:%S"))
}

/// A Ticket's Run directory under the Target repo.
pub(crate) fn run_dir(repo: &Path, ticket: &str) -> PathBuf {
    repo.join(".harness").join("runs").join(ticket)
}

/// How a Stage is named in an event: "implement", "review 1", "fix 2".
pub(crate) fn stage_label(st: &Stage, round: usize) -> String {
    if round == 0 {
        st.name.to_string()
    } else {
        format!("{} {round}", st.name)
    }
}

/// "1 round", "3 findings".
pub(crate) fn plural(n: usize, word: &str) -> String {
    let s = if n == 1 { "" } else { "s" };
    format!("{n} {word}{s}")
}

/// A PR named by its number, as gh shows it: "PR #12".
pub(crate) fn pr_ref(url: &str) -> String {
    let number = url.trim_end_matches('/').rsplit('/').next().unwrap_or(url);
    format!("PR #{number}")
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

impl Orchestrator {
    /// Runs one Stage to its completion rule and returns its accepted result.
    /// A Stage with an already accepted result is not rerun. When it cannot
    /// advance by rule the Shell is woken and the Ticket holds for a retry,
    /// a park, or a late done result.
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
        let saved = self.ticket(ticket);
        let resumed = saved.stage == st.name && saved.round == round;
        self.update(ticket, |ts| {
            ts.limited.clear(); // a hold sets it again
            if !resumed {
                ts.stage = st.name.to_string();
                ts.round = round;
                ts.retried = false; // a resumed Stage keeps its spent retry
                ts.sessions.remove(st.name); // an earlier Round's is never resumed
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
        let label = stage_label(st, round);

        // A resumed Stage whose session from the stopped run is still alive
        // in its pane: /continue watches it rather than starting a fresh one.
        // The agent is asked for by its name, so a pane that now holds
        // another agent is not taken for the Stage's.
        let name = agent_name(ticket, st.name);
        let mut live = saved.panes.get(st.name).cloned().filter(|pane| {
            resumed
                && self
                    .herdr(&["agent", "get", &name])
                    .is_ok_and(|reply| reply.result.agent.pane_id == *pane)
        });
        // Its pane gone, it is resumed by its saved session id instead, and
        // watched as a live one; failing that it starts fresh.
        if let Some(session) = saved
            .sessions
            .get(st.name)
            .filter(|s| resumed && live.is_none() && !s.id.is_empty())
        {
            match self.wait_limit(ticket, &label, &session.app) {
                Some(Held::Park) => return Err(StageError::Parked(format!("by you at {label}"))),
                Some(_) => return Err(StageError::Stopped),
                None => {}
            }
            match self.resume(ticket, st, &label, session, &file) {
                Ok(pane) => live = Some(pane),
                Err(err) => self.log(
                    ticket,
                    &format!("{label} not resumed: {err}, starting it fresh"),
                ),
            }
        }
        let mut retry = false;
        loop {
            let mut held = match live.take() {
                Some(pane) => self.hold(ticket, st, &label, &pane, &file, want, true, None),
                None => self.attempt(ticket, st, &label, retry, &file, &all, want),
            };
            loop {
                if self.stopping() {
                    return Err(StageError::Stopped);
                }
                let (reason, at_limit) = match held {
                    Held::Done(result) => return Ok(result),
                    Held::Stopped => return Err(StageError::Stopped),
                    Held::Park => return Err(StageError::Parked(format!("by you at {label}"))),
                    Held::Away => return Err(StageError::Parked(AWAY.to_string())),
                    // never a Wake: put to the user, then watched again
                    Held::Woke(reason) if reason == ASKED => {
                        let pane = self.ticket(ticket).panes.get(st.name).cloned();
                        let pane = pane.unwrap_or_default(); // the pane it asked in
                        held = self
                            .question(ticket, st, &label, &pane, &file)
                            .unwrap_or_else(|| {
                                self.hold(ticket, st, &label, &pane, &file, want, true, None)
                            });
                        continue;
                    }
                    Held::Retry => {
                        self.update(ticket, |ts| ts.retried = true);
                        retry = true;
                        break;
                    }
                    Held::Limited(limit) => (String::new(), Some(limit)),
                    Held::Woke(reason) => (reason, None),
                };
                let ts = self.ticket(ticket);
                let pane = ts.panes.get(st.name).cloned().unwrap_or_default();
                let tail = self.tail(&name, 120);
                // A usage limit is never a Wake: the Ticket holds until the reset.
                if let Some(limit) = at_limit.or_else(|| self.limit_shown(&ts, st, &tail)) {
                    held = self.limited(ticket, st, &label, &pane, &file, want, limit);
                    continue;
                }
                let alive = self.agent_status(&pane).is_some();
                let actions = offered(&ts, &reason, alive);
                if actions == [Action::Park] {
                    return Err(StageError::Parked(format!(
                        "{label} {reason} again after a retry"
                    )));
                }
                // a command or answer sent before this Wake is not an answer to it
                self.consume(&format!("retry-{ticket}"));
                self.consume(&format!("park-{ticket}"));
                self.take_answer(ticket, None);
                let judged = self.judge(ticket, &ts, &reason, &file, &tail, &actions);
                if self.stopping() {
                    return Err(StageError::Stopped); // a late Judgment is not acted on
                }
                let stuck = format!("stuck in {label}: {reason} {}", self.locate(&pane));
                // At or above the floor the Judgment answers, and the hold
                // acts on it as on the user's answer.
                let act = match judged {
                    Some(judged) if judged.confidence >= FLOOR => {
                        self.report(ticket, &stuck);
                        self.report(ticket, &format!("judged: {}", judged.said()));
                        Some(Answer::Act(judged.choice))
                    }
                    judged => {
                        // of the nudges, the one the Judgment scored higher
                        let picked = judged
                            .as_ref()
                            .and_then(|j| j.scores.iter().map(|(a, _)| *a).find(|a| a.is_nudge()));
                        let actions = actions
                            .into_iter()
                            .filter(|a| !a.is_nudge() || picked.is_none_or(|p| p == *a))
                            .collect();
                        let said = judged.as_ref().map(Judged::said);
                        let ask = Ask::Wake {
                            pane: pane.clone(),
                            tail,
                            file: file.clone(),
                            actions,
                            judged,
                        };
                        self.asks(ticket, &stuck, ask);
                        if let Some(said) = said {
                            // log only: a panel line would close the Question
                            self.log(ticket, &format!("judged: {said}"));
                        }
                        None
                    }
                };
                held = self.hold(ticket, st, &label, &pane, &file, want, false, act);
                if matches!(held, Held::Park) {
                    return Err(StageError::Parked(format!("{label} {reason}")));
                }
            }
        }
    }

    /// Runs the Stage once in a fresh session: its accepted result, the
    /// reason it cannot complete, or /park. `retry` says so on the panel,
    /// once the fresh pane can be named.
    #[allow(clippy::too_many_arguments)]
    fn attempt(
        &self,
        ticket: &str,
        st: &Stage,
        label: &str,
        retry: bool,
        file: &Path,
        inputs: &[(&str, &str)],
        want: ResultRequirements,
    ) -> Held {
        let row = match stage_row(&self.cfg.repo, st) {
            Ok(row) => row,
            Err(err) => return Held::Woke(err),
        };
        // No Stage starts on a Limited App.
        if let Some(held) = self.wait_limit(ticket, label, row.app.name) {
            return held;
        }
        let run_dir = self.run_dir(ticket).display().to_string();
        // The Moderator is given each Debate side's command, read now too.
        let sides = match st.name == DEBATE.name {
            true => match debate_inputs(&self.cfg.repo, &run_dir) {
                Ok(sides) => sides,
                Err(err) => return Held::Woke(err),
            },
            false => Vec::new(),
        };
        let mut inputs = inputs.to_vec();
        inputs.extend(sides.iter().map(|(name, value)| (*name, value.as_str())));
        // Wherever init put it: a copy committed in the repo first, then the
        // checkout's, then the user's.
        let (repo, home) = (&self.cfg.repo, &self.cfg.home);
        let mut dirs = vec![repo.join(".agents/skills"), repo.join(".harness/skills")];
        if !home.as_os_str().is_empty() {
            dirs.push(home.join(".agents/skills"));
        }
        // Only an absent copy falls through: an unreadable one is said.
        let found = dirs.iter().find_map(|dir| {
            let path = dir.join(st.skill).join("SKILL.md");
            match fs::read_to_string(&path) {
                Err(err) if err.kind() == io::ErrorKind::NotFound => None,
                read => Some(read.map_err(|err| format!("cannot read {}: {err}", path.display()))),
            }
        });
        let skill = match found {
            Some(Ok(skill)) => skill,
            Some(Err(err)) => return Held::Woke(err),
            None => return Held::Woke("has no Stage skill (run 'harness init')".to_string()),
        };
        if let Err(err) = fs::create_dir_all(file.parent().unwrap()) {
            return Held::Woke(err.to_string());
        }
        let session = Session {
            app: row.app.name.to_string(),
            ..Default::default()
        };
        let pane = match self.fresh_pane(ticket, st, session) {
            Ok(pane) => pane,
            Err(err) => return Held::Woke(format!("got no pane: {err}")),
        };
        let at = self.locate(&pane);
        if retry {
            self.report(
                ticket,
                &format!("retrying {label} with a fresh session {at}"),
            );
        }
        let Ok(waited) = self.await_trust(ticket, st, row.app, &at) else {
            return Held::Stopped;
        };
        // The previous session can still write while its pane is closing.
        // Clear its result only after fresh_pane has replaced it, before the
        // new writer.
        let _ = fs::remove_file(file);
        let deadline = self.new_deadline(ticket, st);
        // The user accepts trust in that very pane, and trust flips while
        // their own session still holds it: after a trust wait the pane may
        // stay busy until they exit, as long as the Stage's deadline allows.
        let patience = if waited {
            deadline
        } else {
            Instant::now() + 6 * self.cfg.tick
        };

        let agent_args = match self.stage_args(ticket, st, &row) {
            Ok(args) => args,
            Err(reason) => {
                let failed = self.plan_failed(ticket, st, label, &pane, &at, reason.clone(), None);
                return failed.unwrap_or(Held::Woke(reason));
            }
        };
        let start_err = self
            .start_agent(ticket, st, row.app, &pane, &agent_args, patience)
            .err();
        if let Some(err) = &start_err {
            if !err.to_string().contains("agent_not_ready") {
                return Held::Woke(format!("session did not start: {err}"));
            }
        }
        self.report(ticket, &format!("{label} started: {} {at}", row.said()));
        if start_err.is_some() {
            // blocked at startup: nothing can be prompted yet
            if let Some(held) = self.wait_unblocked(ticket, st, label, &pane) {
                return held;
            }
        }

        // The prompt is sent on its own rather than with --wait, so that a
        // prompt which never lands is a reason of its own. Sent together, a
        // session still sitting at a dialog reads as idle the moment the call
        // returns, and an idle pane with no result file is indistinguishable
        // from a Stage that finished and forgot to write one.
        let prompt = stage_prompt(&skill, &inputs);
        if let Err(err) = self.herdr(&["agent", "prompt", &pane, &prompt]) {
            return Held::Woke(format!("never took the Stage skill: {err}"));
        }
        self.log(
            ticket,
            &format!(
                "{label} prompted, waiting for {}",
                file.file_name().unwrap_or_default().to_string_lossy()
            ),
        );
        // A session that has just been prompted still reads idle until it
        // takes the prompt up, which looks exactly like a Stage that finished
        // without writing a result. Give it a few ticks before believing that.
        let settled = Instant::now() + SETTLE_TICKS * self.cfg.tick;
        loop {
            if self.stopping() {
                return Held::Stopped;
            }
            if self.park_arrived(ticket, &pane) {
                return Held::Park;
            }
            match self.watch(ticket, st, &pane).as_deref() {
                None => return Held::Woke("session died".to_string()),
                Some("blocked") => {
                    if let Some(held) = self.wait_unblocked(ticket, st, label, &pane) {
                        return held;
                    }
                }
                Some("idle" | "done") => {
                    let (result, reason) = read_stage_result(file, want);
                    if reason.is_empty() {
                        return Held::Done(result);
                    }
                    if Instant::now() > settled {
                        return Held::Woke(reason);
                    }
                }
                Some(_) if Instant::now() > self.deadline(ticket, st) => {
                    return Held::Woke(self.timed_out(st));
                }
                Some(_) => {}
            }
            if !self.sleep() {
                return Held::Stopped;
            }
            // A tick at a time, never to the Stage's deadline: a wait that
            // blocks for an hour is an hour in which /stop-work does
            // nothing.
            let timeout = wait_for(self.deadline(ticket, st), self.cfg.tick);
            let _ = self.herdr(&["agent", "wait", &pane, "--timeout", &timeout]);
        }
    }

    /// The args a Stage's session starts with, the row's model and effort
    /// last. Only Implement's can fail: it has no plan hook.
    fn stage_args(&self, ticket: &str, st: &Stage, row: &Row) -> Result<Vec<String>, String> {
        let run_dir = self.run_dir(ticket).display().to_string();
        let mut args = if st.name == IMPLEMENT.name {
            // Implement plans first (harness-7bj.9), on claude alone
            // (stage_row): its own settings hold the hook that copies each
            // plan into the run directory, and on a split opusplan's remap.
            let settings = self
                .plan_settings(ticket, row)
                .map_err(|err| format!("has no plan hook: {err}"))?;
            [
                "--permission-mode",
                "plan",
                "--settings",
                &settings,
                "--add-dir",
                &run_dir,
            ]
            .map(String::from)
            .to_vec()
        } else if st.name == REVIEW.name {
            (row.app.run_dir_args)(&self.worktree(ticket).display().to_string())
        } else {
            // Debate, Fix and Address run on claude alone (stage_row).
            ["--permission-mode", "auto", "--add-dir", &run_dir]
                .map(String::from)
                .to_vec()
        };
        args.extend(row.flags());
        Ok(args)
    }

    /// Resumes a stopped run's Stage whose pane is gone: its saved session
    /// in a fresh pane, by id with the Stage's own args (Implement plans
    /// again, so its plan is judged again), told to continue, unless its
    /// result `file` holds a question it waits on. Only while the Stage's
    /// App is unchanged; an error starts it fresh.
    fn resume(
        &self,
        ticket: &str,
        st: &Stage,
        label: &str,
        session: &Session,
        file: &Path,
    ) -> Result<String, String> {
        let row = stage_row(&self.cfg.repo, st)?;
        if row.app.name != session.app {
            return Err(format!("its App is now {}", row.app.name));
        }
        let pane = self
            .fresh_pane(ticket, st, session.clone())
            .map_err(|err| format!("got no pane: {err}"))?;
        let mut args = row.resume(&session.id);
        args.extend(self.stage_args(ticket, st, &row)?);
        let patience = Instant::now() + 6 * self.cfg.tick;
        // A session that asked waits for its answer, not "continue": its
        // question is put to the user again.
        let asked = read_question(file).is_some();
        self.start_agent(ticket, st, row.app, &pane, &args, patience)
            .and_then(|()| match asked {
                true => Ok(()),
                false => self
                    .herdr(&["agent", "prompt", &pane, "continue"])
                    .map(drop),
            })
            .map_err(|err| err.to_string())?;
        let at = self.locate(&pane);
        self.report(ticket, &format!("{label} resumed: {} {at}", row.said()));
        Ok(pane)
    }

    /// Holds a Stage until its agent trusts the directory its pane started
    /// in. Only the user can accept a trust dialog, so the Orchestrator names
    /// the pane, already in that directory, and waits instead of prompting
    /// into one.
    /// Ok(true) when it had to wait.
    fn await_trust(&self, ticket: &str, st: &Stage, app: &App, at: &str) -> Result<bool, String> {
        if self.cfg.home.as_os_str().is_empty() {
            return Ok(false); // no home, no trust stores to read: let the Stage try
        }
        let dir = self.stage_cwd(ticket, st);
        let trusted = || trusts(app, &self.cfg.home, &dir, &self.cfg.repo);
        if trusted() {
            return Ok(false);
        }
        self.report(
            ticket,
            &format!(
                "waiting: {} does not trust {} yet, open it there once and accept {at}",
                app.name,
                dir.display()
            ),
        );
        while !trusted() {
            if !self.sleep() {
                return Err("stopped".to_string());
            }
        }
        self.report(
            ticket,
            &format!("{} trusts {} now, carrying on", app.name, dir.display()),
        );
        Ok(true)
    }

    /// The directory a Stage's pane starts in: the Ticket's worktree, or the
    /// run directory for the Review, whose sandbox may write only where it
    /// starts.
    fn stage_cwd(&self, ticket: &str, st: &Stage) -> PathBuf {
        if st.name == REVIEW.name {
            self.run_dir(ticket)
        } else {
            self.worktree(ticket)
        }
    }

    /// Starts the Stage's session on `app` in `pane` with `args`, giving a
    /// pane that has just been created the moment it needs to get a shell:
    /// until it has one herdr refuses with agent_pane_busy, which is not the
    /// pane being unusable. `give_up` bounds that patience; stop ends it.
    fn start_agent(
        &self,
        ticket: &str,
        st: &Stage,
        app: &App,
        pane: &str,
        args: &[String],
        give_up: Instant,
    ) -> Result<(), RunError> {
        let name = agent_name(ticket, st.name);
        let mut argv = vec![
            "agent", "start", &name, "--kind", app.name, "--pane", pane, "--",
        ];
        argv.extend(args.iter().map(String::as_str));
        loop {
            let err = match self.herdr(&argv) {
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

    /// A blocked session: Implement at its plan dialog with a plan newer
    /// than the last judged is a plan ready (plan.rs); one at a usage limit
    /// (Claude's options menu) is Limited; anything else is the ordinary
    /// blocked Question.
    fn wait_unblocked(&self, ticket: &str, st: &Stage, label: &str, pane: &str) -> Option<Held> {
        self.take_answer(ticket, None); // an answer sent before this prompt is not for it
        if st.name == IMPLEMENT.name {
            if let Some(plan) = self.plan_ready(ticket, pane) {
                return self.plan(ticket, st, label, pane, plan);
            }
        }
        let tail = self.tail(pane, LAST_LINES);
        if let Some(limit) = self.limit_shown(&self.ticket(ticket), st, &tail) {
            return Some(Held::Limited(limit));
        }
        self.blocked(ticket, st, label, pane, &self.locate(pane))
    }

    /// Asks the user about a blocked session, which only they may answer,
    /// and waits for the session to move on (None, said as "carrying on"),
    /// for park, or for a reason it cannot.
    pub(super) fn blocked(
        &self,
        ticket: &str,
        st: &Stage,
        label: &str,
        pane: &str,
        at: &str,
    ) -> Option<Held> {
        self.asks(
            ticket,
            &format!("waiting at a prompt in {label} {at}"),
            Ask::Blocked {
                pane: pane.to_string(),
            },
        );
        loop {
            if self.park_arrived(ticket, pane) {
                return Some(Held::Park);
            }
            match self.agent_status(pane).as_deref() {
                None => return Some(Held::Woke("session died".to_string())),
                Some(status) if status != "blocked" => {
                    self.report(ticket, "carrying on");
                    return None;
                }
                Some(_) if Instant::now() > self.deadline(ticket, st) => {
                    return Some(Held::Woke(self.timed_out(st)))
                }
                Some(_) => {}
            }
            if !self.sleep() {
                return Some(Held::Stopped);
            }
        }
    }

    /// A Stage's own question, its result file reading STATUS: question:
    /// never a Wake, never judged, and no deadline runs while it waits.
    /// Away, the Ticket parks with a bd comment asking for a manual resume,
    /// its session left waiting in its pane; turning Away on while the
    /// Question waits does the same. Otherwise, and always for Address,
    /// whose Ticket has its PR open and no Parked to go to, it is a
    /// Question, whose answer goes into the pane as a prompt. None once the
    /// answer is sent, or the session moves on in the pane.
    fn question(
        &self,
        ticket: &str,
        st: &Stage,
        label: &str,
        pane: &str,
        file: &Path,
    ) -> Option<Held> {
        let mut raised = false;
        loop {
            let asked = read_question(file);
            match self.agent_status(pane).as_deref() {
                None => return Some(Held::Woke("session died".to_string())),
                Some("idle" | "done") if asked.is_some() => {}
                Some(_) => {
                    // answered in the pane: no longer open, as a sent answer;
                    // a new one written since the read stays for the watch
                    if asked.is_some() && read_question(file) == asked {
                        let _ = fs::remove_file(file);
                    }
                    if raised {
                        self.report(ticket, "carrying on"); // answered in the pane
                    }
                    return None;
                }
            }
            let (question, options) = asked.unwrap();
            if self.cfg.away.load(Ordering::SeqCst) && st.name != ADDRESS.name {
                let comment = format!(
                    "{label} asked a question while you were away and needs a manual resume: \
                     /continue @{ticket} in the Harness Shell puts it to you, its session \
                     still waiting in its pane.\n\n{question}\n{}",
                    options
                        .iter()
                        .map(|o| format!("- {o}\n"))
                        .collect::<String>()
                );
                let argv = ["bd", "comments", "add", ticket, comment.trim_end()];
                if let Err(err) = self.cfg.tools.run(&self.cfg.repo, &argv) {
                    self.log(ticket, &format!("no bd comment: {err}"));
                }
                return Some(Held::Away);
            }
            if !raised {
                self.take_answer(ticket, None); // an answer sent before it is not for it
                let at = self.locate(pane);
                let ask = Ask::StageQuestion {
                    pane: pane.to_string(),
                    question,
                    options,
                };
                self.asks(ticket, &format!("question in {label} {at}"), ask);
                raised = true;
            }
            match self.take_answer(ticket, Some(pane)) {
                Some(Answer::Act(Action::Park)) => return Some(Held::Park),
                Some(Answer::Prompt(text)) => {
                    // An answered question is no longer open: a session that
                    // goes idle without rewriting it has no result. Removed
                    // first, so nothing written after the prompt is taken.
                    let kept = fs::read(file);
                    let _ = fs::remove_file(file);
                    if let Err(err) = self.herdr(&["agent", "prompt", pane, &text]) {
                        // never taken, still open: put back unless written anew,
                        // so the Wake's hold asks it again
                        if let Ok(kept) = kept {
                            let _ = fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .open(file)
                                .and_then(|mut f| f.write_all(&kept));
                        }
                        return Some(Held::Woke(format!("never took your answer: {err}")));
                    }
                    self.report(ticket, "sent your answer");
                    self.settle(pane, &["idle", "done"]);
                    return None;
                }
                Some(other) => self.dropped(ticket, &other),
                None => {}
            }
            if self.consume(&format!("park-{ticket}")) {
                return Some(Held::Park);
            }
            if !self.sleep() {
                return Some(Held::Stopped);
            }
        }
    }

    /// Keeps a woken Ticket waiting, leaving every other Ticket running,
    /// until a retry or park arrives or a done result appears. The
    /// Judgment's answer `act`, taken ahead of anything sent meanwhile, and
    /// the user's are acted on here: a nudge is sent to the session in
    /// `pane` and spends the session's one, a wait spends one of its three.
    /// `armed`, a nudge or a wait arms the completion check on the live
    /// session as well: the Ticket Wakes again when the session goes idle
    /// without a result (once the wait is over, whatever its state), dies
    /// or runs out of time, and a prompt it stops at is a blocked session
    /// as in any Stage.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn hold(
        &self,
        ticket: &str,
        st: &Stage,
        label: &str,
        pane: &str,
        file: &Path,
        want: ResultRequirements,
        armed: bool,
        mut act: Option<Answer>,
    ) -> Held {
        // (settled, waiting): idle before `settled` is not believed, and a
        // wait Wakes at `settled` whatever the state.
        let settle = SETTLE_TICKS * self.cfg.tick;
        let mut armed = armed.then(|| {
            self.new_deadline(ticket, st);
            (Instant::now() + settle, false)
        });
        loop {
            let answer = match act.take() {
                Some(act) => Some(act),
                None if self.consume(&format!("retry-{ticket}")) => return Held::Retry,
                None if self.consume(&format!("park-{ticket}")) => return Held::Park,
                None => self.take_answer(ticket, Some(pane)),
            };
            let nudge = match answer {
                Some(Answer::Act(Action::Retry)) => return Held::Retry,
                Some(Answer::Act(Action::Park)) => return Held::Park,
                Some(Answer::Act(Action::Wait)) => {
                    self.update(ticket, |ts| ts.waits += 1);
                    self.report(
                        ticket,
                        &format!("waiting: still working {}", self.locate(pane)),
                    );
                    // the session's own deadline stands
                    armed = Some((Instant::now() + self.wait_length(), true));
                    None
                }
                Some(Answer::Act(nudge)) => nudge
                    .nudge(file)
                    .map(|(prompt, said)| (prompt, format!("nudged: {said}"))),
                Some(Answer::Prompt(text)) => Some((text, "nudged with your prompt".to_string())),
                Some(approve @ Answer::Approve) => {
                    self.dropped(ticket, &approve); // its plan is gone
                    None
                }
                None => None,
            };
            if let Some((prompt, said)) = nudge {
                // spent even if never taken: the Judgment does not nudge again
                self.update(ticket, |ts| ts.nudged = true);
                if let Err(err) = self.herdr(&["agent", "prompt", pane, &prompt]) {
                    return Held::Woke(format!("never took the nudge: {err}"));
                }
                self.report(ticket, &said);
                self.new_deadline(ticket, st);
                armed = Some((Instant::now() + settle, false));
            }
            // The status before the result, as in attempt: a result written
            // between the two reads must not look like idle without one.
            let status = self.watch(ticket, st, pane);
            let (result, reason) = read_stage_result(file, want);
            if reason.is_empty() && matches!(status.as_deref(), None | Some("idle" | "done")) {
                return Held::Done(result);
            }
            if let Some((settled, waiting)) = armed {
                let now = Instant::now();
                match status.as_deref() {
                    None => return Held::Woke("session died".to_string()),
                    Some("blocked") => {
                        if let Some(held) = self.wait_unblocked(ticket, st, label, pane) {
                            return held;
                        }
                    }
                    Some("idle" | "done") if now > settled => return Held::Woke(reason),
                    Some(_) if now > self.deadline(ticket, st) => {
                        return Held::Woke(self.timed_out(st))
                    }
                    Some(_) if waiting && now > settled && !reason.is_empty() => {
                        return Held::Woke(reason)
                    }
                    Some(_) => {}
                }
            } else if reason == ASKED && matches!(status.as_deref(), Some("idle" | "done")) {
                // a session taken up again in its pane after a Wake asks too
                return Held::Woke(reason);
            }
            if !self.sleep() {
                return Held::Stopped;
            }
        }
    }

    /// The herdr state of the agent in the Stage's pane, as agent_status,
    /// saving the session id herdr reports there as the Stage's: the id
    /// /continue resumes it by once the pane is gone. Saved only while herdr
    /// still names the Stage's own agent in that pane, so another agent's
    /// id is never resumed as the Stage's.
    pub(super) fn watch(&self, ticket: &str, st: &Stage, pane: &str) -> Option<String> {
        let agent = self.herdr(&["agent", "get", pane]).ok()?.result.agent;
        let id = agent.agent_session.map(|s| s.value).unwrap_or_default();
        if !id.is_empty()
            && self
                .ticket(ticket)
                .sessions
                .get(st.name)
                .is_some_and(|s| s.id != id)
            && self
                .herdr(&["agent", "get", &agent_name(ticket, st.name)])
                .is_ok_and(|reply| reply.result.agent.pane_id == pane)
        {
            self.update(ticket, |ts| {
                if let Some(session) = ts.sessions.get_mut(st.name) {
                    session.id = id;
                }
            });
        }
        Some(agent.status)
    }

    /// One tick of every polling loop. False once /stop-work has arrived.
    pub(crate) fn sleep(&self) -> bool {
        if self.stopping() {
            return false;
        }
        thread::sleep(self.cfg.tick);
        // Checked after the sleep: a stop that arrived during it must not buy
        // one more loop, which in a hold could start a fresh session.
        !self.stopping()
    }

    /// Gives the Stage an empty shell pane in the Ticket tab, replacing the
    /// pane of an earlier session of the same Stage (a previous Round, a
    /// retry, a resumed run) so no session starts in an old pane, and records
    /// the session it will run beside it.
    fn fresh_pane(&self, ticket: &str, st: &Stage, session: Session) -> Result<String, RunError> {
        let ts = self.ticket(ticket);
        if let Some(old) = ts.panes.get(st.name) {
            let at = self.locate(old);
            if self.herdr(&["pane", "close", old]).is_ok() {
                // already gone is fine
                self.log(ticket, &format!("dropped a leftover pane {at}"));
            }
        }
        let cwd = self.stage_cwd(ticket, st).display().to_string();
        let key = self.typesafe_key();
        let env = format!("TYPESAFE_API_KEY={key}");
        let mut placement = vec!["--cwd", cwd.as_str(), "--no-focus"];
        if st.name == "debate" && !key.is_empty() {
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
            ts.sessions.insert(st.name.to_string(), session);
            // a fresh session has its nudge and its waits, and no feedback
            ts.nudged = false;
            ts.waits = 0;
            ts.feedback.clear();
        });
        Ok(pane)
    }
}

/// A deadline as the Stage table and the tests set them: "1h", "30m", "5ms".
// ponytail: the largest whole unit; a 90-minute Stage would read "90m".
fn short_duration(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0 => format!("{}ms", d.as_millis()),
        s if s % 3600 == 0 => format!("{}h", s / 3600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
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
            workspace: "w1".to_string(),
            api_key: "sk-test".to_string(),
            exe: PathBuf::from("/opt/the harness/harness"),
            typesafe: super::judgment::fake::Fake::down(),
            home: home.to_path_buf(),
            tick: Duration::from_millis(1),
            poll_prs: Duration::from_millis(1),
            max: 3,
            away: Arc::new(AtomicBool::new(false)),
            log: Arc::new(Mutex::new(Box::new(io::sink()))),
            events: std::sync::mpsc::channel().0,
            clock: Arc::new(chrono::Local::now),
            timeout: None,
            wait: None,
        }
    }
}
