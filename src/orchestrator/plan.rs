//! Plans (harness-7bj.9, research/plan-mode): Implement starts in plan mode
//! with a hook that copies every plan it presents into the run directory as
//! plan.md, where a blocked Implement finds it. The plan dialog is answered
//! by keys in the pane: enter approves, and feedback closes the dialog on its
//! empty third option and goes in as a prompt.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use serde_json::json;

use super::stage::{Held, Orchestrator, SETTLE_TICKS};
use crate::tools::RunError;

/// Where the hook copies the plan the session presents.
const PLAN: &str = "plan.md";
/// The last plan answered: a plan.md beside it is a newer one.
const JUDGED: &str = "plan-judged.md";

impl Orchestrator {
    /// Writes the Implement session's settings file and returns its path:
    /// one PreToolUse hook on ExitPlanMode, this binary's hidden mode, which
    /// copies the plan into the run directory and decides nothing, so the
    /// dialog shows as usual. An earlier session's plan goes.
    pub(crate) fn plan_settings(&self, ticket: &str) -> Result<PathBuf, String> {
        let dir = self.run_dir(ticket);
        let _ = fs::remove_file(dir.join(PLAN));
        let exe = crate::update::exe_path()?;
        let command = format!("{} __plan-hook {}", quoted(&exe), quoted(&dir.join(PLAN)));
        let settings = json!({ "hooks": { "PreToolUse": [{
            "matcher": "ExitPlanMode",
            "hooks": [{ "type": "command", "command": command }],
        }] } });
        let path = dir.join("settings.json");
        fs::write(&path, settings.to_string()).map_err(|err| err.to_string())?;
        Ok(path)
    }

    /// The plan the hook copied in since the last one was answered.
    pub(crate) fn fresh_plan(&self, ticket: &str) -> Option<String> {
        fs::read_to_string(self.run_dir(ticket).join(PLAN)).ok()
    }

    /// The fresh plan has its answer: it is the last judged now.
    pub(crate) fn plan_answered(&self, ticket: &str) {
        let dir = self.run_dir(ticket);
        let _ = fs::rename(dir.join(PLAN), dir.join(JUDGED));
    }

    /// Approves the plan in the pane: enter, on the dialog's first option,
    /// leaves plan mode for auto mode, as every other Stage launches.
    pub(crate) fn approve_plan(&self, ticket: &str, pane: &str) -> Option<Held> {
        self.plan_answered(ticket);
        if let Err(err) = self.keys(pane, "enter") {
            return refused(err);
        }
        self.report(ticket, "plan approved");
        self.settle(pane, &["blocked"]);
        None
    }

    /// Sends the plan back with the user's feedback. The dialog's third
    /// option, left empty, closes it and keeps plan mode: down, down, enter,
    /// one key a call with the pane re-read between, since keys sent in one
    /// call were seen to approve instead; esc and 3 approved outright and
    /// are never sent. Once the session is idle and still in plan mode the
    /// feedback is its prompt, and the revised plan is judged with it.
    pub(crate) fn send_back(&self, ticket: &str, pane: &str, feedback: &str) -> Option<Held> {
        self.plan_answered(ticket);
        self.update(ticket, |ts| ts.feedback = feedback.to_string());
        for (n, key) in ["down", "down", "enter"].into_iter().enumerate() {
            if n > 0 {
                // Not stop's sleep: a dialog left with its cursor on the
                // third option would take the next approval as feedback.
                thread::sleep(self.cfg.tick);
                self.visible(pane);
            }
            if let Err(err) = self.keys(pane, key) {
                return refused(err);
            }
        }
        let mut ticks = 0;
        while !(matches!(self.agent_status(pane).as_deref(), Some("idle" | "done"))
            && self.visible(pane).contains("plan mode on"))
        {
            ticks += 1;
            if ticks > SETTLE_TICKS {
                return Some(Held::Woke(
                    "left plan mode before your feedback".to_string(),
                ));
            }
            if !self.sleep() {
                return Some(Held::Stopped);
            }
        }
        if let Err(err) = self.herdr(&["agent", "prompt", pane, feedback]) {
            return refused(err);
        }
        self.report(ticket, "plan sent back with your feedback");
        self.settle(pane, &["idle", "done"]);
        None
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
    fn settle(&self, pane: &str, was: &[&str]) {
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

fn refused(err: RunError) -> Option<Held> {
    Some(Held::Woke(format!(
        "never took the answer to its plan: {err}"
    )))
}

/// A path as one shell word: the hook command runs under a shell.
fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}
