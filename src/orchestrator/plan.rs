//! Plans (harness-7bj.9, research/plan-mode): Implement starts in plan mode
//! with a hook that copies every plan it presents into the run directory as
//! plan.md. Blocked at the plan dialog with a plan newer than the last judged
//! is a plan ready. The dialog is answered by keys, and only on what the pane
//! shows: enter on a Yes approves; feedback moves the cursor down, a key a
//! call, to "Tell Claude what to change", enters it empty, and goes in as a
//! prompt. esc and 3 are never sent: in the research they approved.
//!
//! A split (harness-7nq.8, research/stage-models) plans on one model and
//! implements on another: opusplan, its halves remapped in the settings,
//! which show the dialog's clear-context option; approval moves there, so
//! the implementing model starts from the plan alone.
//!
//! Off claude (harness-7nq.12) the Plan takes two steps: the session writes
//! plan.md and a Stage result STATUS: plan, and waits. Feedback goes into
//! its pane as a prompt; approval prompts "implement the approved plan",
//! only while the worktree is as the session found it: a session that
//! changed it before approval is a plan failure.

use std::fs;
use std::thread;

use serde_json::{json, Value};

use super::app::Row;
use super::judgment::{Action, PlanJudged};
use super::result::{read_stage_result, ResultRequirements, PLANNED};
use super::stage::{result_name, Answer, Ask, Held, Orchestrator, Stage, IMPLEMENT, SETTLE_TICKS};
use crate::tools::RunError;

/// Where the hook copies the plan the session presents.
const PLAN: &str = "plan.md";
/// The Implement session's settings file, in the run directory.
const SETTINGS: &str = "settings.json";
/// The plan dialog's option that takes feedback; the ones that approve
/// begin with Yes.
const FEEDBACK: &str = "Tell Claude what to change";
/// The approving option a split's settings show, which clears the context.
const CLEAR: &str = "Yes, clear context";
/// A two-step Plan's approval, the prompt that ends the planning.
const APPROVED: &str = "implement the approved plan";
/// The worktree's HEAD and tree as a two-step Plan's session started.
const BEFORE: &str = "before-implement.json";

/// How an approval went.
enum Approval {
    Sent,
    /// The plan changed after it was judged: this one is judged instead.
    Changed(String),
    /// The pane no longer shows the plan dialog on a Yes: no Enter.
    Gone,
    Failed(String),
}

/// How feedback went.
enum SentBack {
    Sent,
    /// The plan changed after it was judged: no key was sent, and this one
    /// is judged instead.
    Changed(String),
    /// No Enter was sent: the dialog was not there, or its cursor never
    /// reached the feedback option.
    NotSent(&'static str),
    Failed(String),
}

/// How moving the dialog's cursor went.
enum Moved {
    Reached,
    /// The cursor never reached the option.
    Stalled,
    /// The dialog left the screen.
    Gone,
    Failed(RunError),
}

/// The plan dialog on a pane's visible screen: its options top to bottom
/// and the one the cursor (❯) is on. It is the block of lines, between blank
/// lines, around the feedback option.
// ponytail: read off Claude Code's screen by its ❯, "N. " and the feedback
// label; a redesigned dialog reads as none, and no key is sent.
struct Dialog {
    options: Vec<String>,
    cursor: Option<usize>,
}

impl Dialog {
    fn on(&self, label: &str) -> bool {
        self.cursor
            .is_some_and(|i| self.options[i].starts_with(label))
    }
}

fn plan_dialog(screen: &str) -> Option<Dialog> {
    let lines: Vec<&str> = screen.lines().collect();
    let at = lines.iter().rposition(|line| line.contains(FEEDBACK))?;
    let blank = |line: &&str| line.trim().is_empty();
    let from = lines[..at].iter().rposition(blank).map_or(0, |i| i + 1);
    let to = lines[at..]
        .iter()
        .position(blank)
        .map_or(lines.len(), |i| at + i);
    let mut dialog = Dialog {
        options: Vec::new(),
        cursor: None,
    };
    for line in &lines[from..to] {
        let line = line.trim_start();
        let (marked, line) = match line.strip_prefix('❯') {
            Some(rest) => (true, rest.trim_start()),
            None => (false, line),
        };
        let Some((n, label)) = line.split_once(". ") else {
            continue;
        };
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if marked {
            dialog.cursor = Some(dialog.options.len());
        }
        dialog.options.push(label.trim().to_string());
    }
    Some(dialog)
}

impl Orchestrator {
    /// Writes the Implement session's settings file and returns its path:
    /// one PreToolUse hook on ExitPlanMode, this binary's hidden mode, which
    /// copies the plan into the run directory and decides nothing, so the
    /// dialog shows as usual. An earlier session's plan goes. On a split the
    /// env remaps opusplan's halves, plan and implement model, the dialog
    /// shows its clear-context option, and a PostModelSwitch hook logs the
    /// switch.
    pub(super) fn plan_settings(&self, ticket: &str, row: &Row) -> Result<String, String> {
        let dir = self.run_dir(ticket);
        let _ = fs::remove_file(dir.join(PLAN));
        self.plans.lock().unwrap().remove(ticket);
        if self.cfg.exe.as_os_str().is_empty() {
            return Err("no path to the orqa binary".to_string());
        }
        let command = format!(
            "{} __plan-hook {}",
            quoted(self.cfg.exe.display()),
            quoted(dir.join(PLAN).display())
        );
        let mut settings = json!({ "hooks": { "PreToolUse": [{
            "matcher": "ExitPlanMode",
            "hooks": [{ "type": "command", "command": command }],
        }] } });
        if let Some(plan_model) = &row.plan_model {
            let log = self.cfg.repo.join(".orqadence").join("orchestrator.log");
            let command = format!(
                "{} __switch-hook {} {}",
                quoted(self.cfg.exe.display()),
                quoted(log.display()),
                quoted(ticket)
            );
            settings["env"] = json!({
                "ANTHROPIC_DEFAULT_OPUS_MODEL": plan_model,
                "ANTHROPIC_DEFAULT_SONNET_MODEL": row.model,
            });
            settings["showClearContextOnPlanAccept"] = json!(true);
            settings["hooks"]["PostModelSwitch"] =
                json!([{ "hooks": [{ "type": "command", "command": command }] }]);
        }
        let path = dir.join(SETTINGS);
        fs::write(&path, settings.to_string()).map_err(|err| err.to_string())?;
        Ok(path.display().to_string())
    }

    /// Readies a fresh session's two-step Plan: an earlier session's plan
    /// goes, and the worktree as it starts is kept, which approval compares
    /// with: in the run directory, so a resumed session is compared with
    /// the tree it started from.
    pub(super) fn start_written_plan(&self, ticket: &str) -> Result<(), String> {
        let dir = self.run_dir(ticket);
        let _ = fs::remove_file(dir.join(PLAN));
        self.plans.lock().unwrap().remove(ticket);
        fs::write(dir.join(BEFORE), self.snapshot(ticket))
            .map_err(|err| format!("worktree snapshot not saved: {err}"))
    }

    /// A two-step Plan ready: the session wrote STATUS: plan, and its plan
    /// is plan.md.
    pub(super) fn written_plan(
        &self,
        ticket: &str,
        st: &Stage,
        label: &str,
        pane: &str,
    ) -> Option<Held> {
        match fs::read_to_string(self.run_dir(ticket).join(PLAN)) {
            Ok(plan) => self.plan(ticket, st, label, pane, plan),
            Err(_) => Some(Held::Woke(format!("wrote STATUS: plan and no {PLAN}"))),
        }
    }

    /// Whether the Ticket's Implement session plans in two steps: its App
    /// is not claude.
    pub(super) fn writes_plan(&self, ticket: &str) -> bool {
        let ts = self.ticket(ticket);
        ts.sessions
            .get(IMPLEMENT.name)
            .is_some_and(|session| session.app != "claude")
    }

    /// Whether the worktree is not as the two-step Plan's session found it:
    /// HEAD moved or the tree changed. No snapshot, no check.
    fn worktree_changed(&self, ticket: &str) -> bool {
        fs::read_to_string(self.run_dir(ticket).join(BEFORE))
            .is_ok_and(|before| before != self.snapshot(ticket))
    }

    /// The plan of an Implement session blocked at its plan dialog, when
    /// the hook copied in one newer than the last judged.
    pub(super) fn plan_ready(&self, ticket: &str, pane: &str) -> Option<String> {
        let plan = fs::read_to_string(self.run_dir(ticket).join(PLAN)).ok()?;
        let judged = self.plans.lock().unwrap().get(ticket) == Some(&plan);
        (!judged && plan_dialog(&self.visible(pane)).is_some()).then_some(plan)
    }

    /// A plan ready: the plan Judgment approves it when its Nouls clear the
    /// floor, otherwise the user answers its Question, until the session
    /// moves on (None) or the Stage ends. No deadline runs while the
    /// Question waits.
    pub(super) fn plan(
        &self,
        ticket: &str,
        st: &Stage,
        label: &str,
        pane: &str,
        mut plan: String,
    ) -> Option<Held> {
        let at = self.locate(pane);
        let ready = format!("plan ready in {label} {at}");
        'judge: loop {
            self.plans
                .lock()
                .unwrap()
                .insert(ticket.to_string(), plan.clone());
            let judged = self.judge_plan(ticket, &plan);
            if self.stopping() {
                return Some(Held::Stopped); // a late Judgment is not acted on
            }
            self.report(ticket, &ready);
            if let Some(judged) = judged {
                self.report(ticket, &format!("judged: {}", judged.said()));
            }
            // Clearing the floor the Judgment approves, as the user would.
            let mut approved = judged.filter(PlanJudged::approves).map(|_| Answer::Approve);
            let mut kept = None;
            loop {
                let answer = match approved.take() {
                    Some(answer) => answer,
                    None => {
                        let ask = Ask::Plan {
                            pane: pane.to_string(),
                            plan: plan.clone(),
                            judged,
                            feedback: kept.clone(),
                        };
                        self.ask_only(ticket, &ready, ask);
                        let answered = self.plan_answer(ticket, pane);
                        self.new_deadline(ticket, st); // none ran while it waited
                        match answered {
                            Ok(answer) => answer,
                            Err(end) => return end,
                        }
                    }
                };
                if let Answer::Prompt(feedback) = answer {
                    match self.send_back(ticket, st, pane, &feedback) {
                        SentBack::Sent => return None,
                        SentBack::Changed(newer) => {
                            self.dropped(ticket, &Answer::Prompt(feedback));
                            plan = newer;
                            continue 'judge;
                        }
                        SentBack::NotSent(why) => {
                            self.report(ticket, &format!("feedback not sent: {why} {at}"));
                            kept = Some(feedback);
                        }
                        SentBack::Failed(reason) => {
                            return self.plan_failed(
                                ticket,
                                st,
                                label,
                                pane,
                                &at,
                                reason,
                                Some(feedback),
                            )
                        }
                    }
                    continue;
                }
                match self.approve(ticket, st, pane, &plan) {
                    Approval::Sent => return None,
                    Approval::Changed(newer) => {
                        plan = newer;
                        continue 'judge;
                    }
                    Approval::Gone => return self.blocked(ticket, st, label, pane, &at),
                    Approval::Failed(reason) => {
                        return self.plan_failed(ticket, st, label, pane, &at, reason, None)
                    }
                }
            }
        }
    }

    /// Waits for the plan Question's approve or feedback; or ends the wait
    /// with park, stop, the session dying, or its moving on in the pane
    /// (None, "carrying on"). A Question has no timeout.
    fn plan_answer(&self, ticket: &str, pane: &str) -> Result<Answer, Option<Held>> {
        // at its dialog, or idle at its written plan
        let waiting: &[&str] = match self.writes_plan(ticket) {
            true => &["idle", "done"],
            false => &["blocked"],
        };
        loop {
            match self.take_answer(ticket, Some(pane)) {
                Some(Answer::Act(Action::Park)) => return Err(Some(Held::Park)),
                Some(answer @ (Answer::Approve | Answer::Prompt(_))) => return Ok(answer),
                Some(other) => self.dropped(ticket, &other),
                None => {}
            }
            if self.consume(&format!("park-{ticket}")) {
                return Err(Some(Held::Park));
            }
            match self.agent_status(pane).as_deref() {
                None => return Err(Some(Held::Woke("session died".to_string()))),
                Some(status) if waiting.contains(&status) => {}
                Some(_) => {
                    self.report(ticket, "carrying on"); // answered in the pane
                    return Err(None);
                }
            }
            if !self.sleep() {
                return Err(Some(Held::Stopped));
            }
        }
    }

    /// Approves the plan, only while the pane is blocked at the plan dialog
    /// with its cursor on a Yes and plan.md holds the plan judged: enter
    /// there leaves plan mode for auto mode, as every other Stage launches,
    /// and the Stage's deadline starts over. The screen is read first: the
    /// hook writes plan.md before its dialog shows. A split's session, by
    /// the settings it started with, is approved on the clear-context
    /// option, the cursor moved there first.
    fn approve(&self, ticket: &str, st: &Stage, pane: &str, judged: &str) -> Approval {
        if self.writes_plan(ticket) {
            return self.approve_written(ticket, st, pane, judged);
        }
        let blocked = self.agent_status(pane).as_deref() == Some("blocked");
        let dialog = blocked.then(|| plan_dialog(&self.visible(pane))).flatten();
        let plan = fs::read_to_string(self.run_dir(ticket).join(PLAN)).ok();
        match (dialog, plan) {
            (Some(dialog), Some(plan)) if plan == judged && dialog.on("Yes") => {
                if self.clears_context(ticket) {
                    match self.cursor_to(pane, dialog, CLEAR) {
                        Moved::Reached => {}
                        Moved::Stalled => {
                            return Approval::Failed(format!("the cursor never reached {CLEAR}"))
                        }
                        Moved::Gone => return Approval::Gone,
                        Moved::Failed(err) => return Approval::Failed(unanswered(err)),
                    }
                }
                if let Err(err) = self.keys(pane, "enter") {
                    return Approval::Failed(unanswered(err));
                }
                self.report(ticket, "plan approved");
                self.new_deadline(ticket, st);
                self.settle(pane, &["blocked"]);
                Approval::Sent
            }
            (Some(_), Some(plan)) if plan != judged => Approval::Changed(plan),
            _ => Approval::Gone,
        }
    }

    /// Approves a two-step Plan, only while plan.md holds the plan judged
    /// and the worktree is as the session found it: its result, the plan,
    /// goes, and "implement the approved plan" is its prompt.
    fn approve_written(&self, ticket: &str, st: &Stage, pane: &str, judged: &str) -> Approval {
        let dir = self.run_dir(ticket);
        match fs::read_to_string(dir.join(PLAN)) {
            Ok(plan) if plan != judged => return Approval::Changed(plan),
            Ok(_) => {}
            Err(err) => return Approval::Failed(format!("{PLAN}: {err}")),
        }
        if self.worktree_changed(ticket) {
            return Approval::Failed(
                "changed the worktree before its plan was approved".to_string(),
            );
        }
        let _ = fs::remove_file(dir.join(result_name(st, 0))); // no longer open
        if let Err(err) = self.herdr(&["agent", "prompt", pane, APPROVED]) {
            return Approval::Failed(unanswered(err));
        }
        self.report(ticket, "plan approved");
        self.new_deadline(ticket, st);
        self.settle(pane, &["idle", "done"]);
        Approval::Sent
    }

    /// Whether the session's settings show the clear-context option: a split.
    fn clears_context(&self, ticket: &str) -> bool {
        let settings = fs::read_to_string(self.run_dir(ticket).join(SETTINGS)).unwrap_or_default();
        serde_json::from_str::<Value>(&settings)
            .is_ok_and(|settings| settings["showClearContextOnPlanAccept"] == true)
    }

    /// Moves the dialog's cursor to the option that begins with label, up
    /// when it shows above the cursor and down otherwise, a key a call, the
    /// pane re-read after each (keys sent together were seen to land where
    /// the screen did not show): at most one key per option and none after
    /// a key that did not move it.
    fn cursor_to(&self, pane: &str, mut dialog: Dialog, label: &str) -> Moved {
        let mut moves = 0;
        while !dialog.on(label) {
            if moves == dialog.options.len() {
                return Moved::Stalled;
            }
            let target = dialog.options.iter().position(|o| o.starts_with(label));
            let up = matches!((target, dialog.cursor), (Some(t), Some(c)) if t < c);
            if let Err(err) = self.keys(pane, if up { "up" } else { "down" }) {
                return Moved::Failed(err);
            }
            moves += 1;
            // Not stop's sleep: begun, the keys run to their end.
            thread::sleep(self.cfg.tick);
            match plan_dialog(&self.visible(pane)) {
                Some(now) if now.cursor == dialog.cursor => return Moved::Stalled,
                Some(now) => dialog = now,
                None => return Moved::Gone,
            }
        }
        Moved::Reached
    }

    /// Sends the plan back with the user's feedback, acting only on what
    /// the pane shows: with the dialog of the plan judged there, the cursor
    /// moved to the feedback option; then enter, which leaves it empty and
    /// keeps plan mode. Once the session is idle in plan mode the feedback
    /// is its prompt, and the Stage's deadline starts over.
    fn send_back(&self, ticket: &str, st: &Stage, pane: &str, feedback: &str) -> SentBack {
        if self.writes_plan(ticket) {
            return self.send_back_written(ticket, st, pane, feedback);
        }
        let screen = self.visible(pane);
        match plan_dialog(&screen) {
            Some(dialog) => {
                // read after the screen, as in approve
                let plan = fs::read_to_string(self.run_dir(ticket).join(PLAN)).unwrap_or_default();
                if self.plans.lock().unwrap().get(ticket) != Some(&plan) {
                    return SentBack::Changed(plan);
                }
                match self.cursor_to(pane, dialog, FEEDBACK) {
                    Moved::Reached => {}
                    Moved::Stalled => {
                        return SentBack::NotSent(
                            "the cursor never reached Tell Claude what to change",
                        )
                    }
                    Moved::Gone => return SentBack::NotSent("the plan dialog is not on screen"),
                    Moved::Failed(err) => return SentBack::Failed(unanswered(err)),
                }
                if let Err(err) = self.keys(pane, "enter") {
                    return SentBack::Failed(unanswered(err));
                }
                let mut ticks = 0;
                while !self.idle_in_plan_mode(pane, &self.visible(pane)) {
                    ticks += 1;
                    if ticks > SETTLE_TICKS {
                        return SentBack::Failed("left plan mode before your feedback".to_string());
                    }
                    thread::sleep(self.cfg.tick); // as the keys: the prompt follows them
                }
            }
            // closed already, by keys whose prompt never followed
            None if self.idle_in_plan_mode(pane, &screen) => {}
            None => return SentBack::NotSent("the plan dialog is not on screen"),
        }
        self.feedback_prompt(ticket, st, pane, feedback)
    }

    /// Sends a two-step Plan back with the user's feedback, only while
    /// plan.md holds the plan judged: its result, the plan, goes, and the
    /// feedback is its prompt as it waits.
    fn send_back_written(&self, ticket: &str, st: &Stage, pane: &str, feedback: &str) -> SentBack {
        let dir = self.run_dir(ticket);
        let plan = fs::read_to_string(dir.join(PLAN)).unwrap_or_default();
        if self.plans.lock().unwrap().get(ticket) != Some(&plan) {
            return SentBack::Changed(plan);
        }
        let _ = fs::remove_file(dir.join(result_name(st, 0))); // no longer open
        self.feedback_prompt(ticket, st, pane, feedback)
    }

    /// The feedback as the session's prompt, kept for the plan's next
    /// Judgment; the Stage's deadline starts over.
    fn feedback_prompt(&self, ticket: &str, st: &Stage, pane: &str, feedback: &str) -> SentBack {
        if let Err(err) = self.herdr(&["agent", "prompt", pane, feedback]) {
            return SentBack::Failed(unanswered(err));
        }
        self.update(ticket, |ts| ts.feedback = feedback.to_string());
        self.report(ticket, "plan sent back with your feedback");
        self.new_deadline(ticket, st);
        self.settle(pane, &["idle", "done"]);
        SentBack::Sent
    }

    /// A plan failure: a Question for the user, never the Wake Judgment,
    /// whose nudges mean nothing at a plan dialog. It offers open the pane,
    /// park, retry, and resend the feedback when there is some, and waits
    /// for an answer, or for the session to move on: blocked at a newer
    /// plan, or its result written (None, "carrying on", the Stage's
    /// deadline started over).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_failed(
        &self,
        ticket: &str,
        st: &Stage,
        label: &str,
        pane: &str,
        at: &str,
        mut reason: String,
        feedback: Option<String>,
    ) -> Option<Held> {
        let file = self.run_dir(ticket).join(result_name(st, 0));
        'ask: loop {
            let ask = Ask::PlanFailed {
                pane: pane.to_string(),
                feedback: feedback.clone(),
            };
            self.asks(ticket, &format!("stuck in {label}: {reason} {at}"), ask);
            loop {
                match self.take_answer(ticket, Some(pane)) {
                    Some(Answer::Act(Action::Park)) => return Some(Held::Park),
                    Some(Answer::Act(Action::Retry)) => return Some(Held::Retry),
                    Some(Answer::Prompt(text)) if feedback.is_some() => {
                        reason = match self.send_back(ticket, st, pane, &text) {
                            SentBack::Sent => return None,
                            SentBack::Changed(_) => {
                                // the loop below finds the newer plan ready
                                self.dropped(ticket, &Answer::Prompt(text));
                                continue;
                            }
                            SentBack::NotSent(why) => format!("feedback not sent: {why}"),
                            SentBack::Failed(reason) => reason,
                        };
                        continue 'ask;
                    }
                    Some(other) => self.dropped(ticket, &other),
                    None => {}
                }
                if self.consume(&format!("park-{ticket}")) {
                    return Some(Held::Park);
                }
                if self.consume(&format!("retry-{ticket}")) {
                    return Some(Held::Retry);
                }
                let moved = match self.agent_status(pane).as_deref() {
                    Some("blocked") => self.plan_ready(ticket, pane).is_some(),
                    _ => match read_stage_result(&file, ResultRequirements::default())
                        .1
                        .as_str()
                    {
                        "" => true,
                        // a two-step Plan's newer plan
                        PLANNED => {
                            let plan = fs::read_to_string(self.run_dir(ticket).join(PLAN)).ok();
                            self.plans.lock().unwrap().get(ticket) != plan.as_ref()
                        }
                        _ => false,
                    },
                };
                if moved {
                    self.report(ticket, "carrying on");
                    self.new_deadline(ticket, st);
                    return None;
                }
                if !self.sleep() {
                    return Some(Held::Stopped);
                }
            }
        }
    }

    fn idle_in_plan_mode(&self, pane: &str, screen: &str) -> bool {
        matches!(self.agent_status(pane).as_deref(), Some("idle" | "done"))
            && screen.contains("plan mode on")
    }

    /// One key to the session's pane.
    fn keys(&self, pane: &str, key: &str) -> Result<String, RunError> {
        let argv = ["herdr", "agent", "send-keys", pane, key];
        self.cfg.tools.run(&self.cfg.repo, &argv)
    }

    /// What the pane shows now; the dialog's history is not kept while it
    /// is blocked, the visible screen always is.
    fn visible(&self, pane: &str) -> String {
        let argv = ["herdr", "agent", "read", pane, "--source", "visible"];
        self.cfg
            .tools
            .run(&self.cfg.repo, &argv)
            .unwrap_or_default()
    }

    /// Gives the session the settle ticks to leave `was`: herdr's status is
    /// a moment behind keys and prompts, and the Stage loop would take the
    /// stale one for another prompt, or for a Stage idle without a result.
    pub(super) fn settle(&self, pane: &str, was: &[&str]) {
        for _ in 0..SETTLE_TICKS {
            let still = self
                .agent_status(pane)
                .is_some_and(|status| was.contains(&status.as_str()));
            if !still || !self.sleep() {
                return;
            }
        }
    }
}

fn unanswered(err: RunError) -> String {
    format!("never took the answer to its plan: {err}")
}

/// One shell word, single quoted: the hook commands and a Debate side's
/// command run under a shell.
pub(super) fn quoted(word: impl std::fmt::Display) -> String {
    format!("'{}'", word.to_string().replace('\'', r"'\''"))
}
