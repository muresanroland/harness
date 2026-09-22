//! The scheduler: starts ready Tickets, at most max at once, each on a thread
//! of its own that is never joined (ADR 0003), resumes the ones a killed run
//! left behind, polls PRs for merges (ADR 0002) and obeys the control commands.

use serde::Deserialize;
use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use super::result::ResultRequirements;
use super::stage::{pr_ref, result_name, Orchestrator, StageError, ADDRESS};
use super::state::{TicketState, STATUS_MERGED, STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BdIssue {
    id: String,
    status: String,
    issue_type: String,
    dependencies: Vec<BdDependency>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BdDependency {
    depends_on_id: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GhPr {
    state: String,
    mergeable: String,
}

impl Orchestrator {
    fn bd_issues(&self, args: &[&str]) -> Result<Vec<BdIssue>, String> {
        let mut argv = vec!["bd"];
        argv.extend_from_slice(args);
        let out = self
            .cfg
            .tools
            .run(&self.cfg.repo, &argv)
            .map_err(|err| err.to_string())?;
        let issues: Option<Vec<BdIssue>> = serde_json::from_str(&out)
            .map_err(|err| format!("bd {}: unreadable reply: {err}", args[0]))?;
        Ok(issues
            .unwrap_or_default()
            .into_iter()
            .filter(|issue| issue.issue_type != "epic")
            .collect())
    }

    /// Drives an Epic: it starts ready Tickets, at most max at once, resumes
    /// the ones a killed run left behind, polls PRs for merges, obeys control
    /// commands, and returns when every child Ticket is closed or on 'stop'.
    pub(crate) fn run(self: &Arc<Self>, epic: &str) -> Result<(), String> {
        self.drain_commands();
        self.state.lock().unwrap().epic = epic.to_string();

        let launch = |ticket: &str, work: fn(&Orchestrator, &str)| {
            // A Ticket can consume stop while the scheduler is in a bd call.
            // Its saved state stays running for resume, but this run is over.
            if self.stopping() {
                return;
            }
            self.active.lock().unwrap().insert(ticket.to_string());
            let slot = Slot {
                o: Arc::clone(self),
                ticket: ticket.to_string(),
            };
            let handle = thread::spawn(move || work(&slot.o, &slot.ticket));
            #[cfg(test)]
            self.threads.lock().unwrap().push(handle);
            #[cfg(not(test))]
            let _ = handle; // never joined: 'stop' must not wait out a Stage
        };
        let busy = |ticket: &str| self.active.lock().unwrap().contains(ticket);
        let in_pipeline = || self.active.lock().unwrap().len();

        let mut last_poll: Option<Instant> = None;
        loop {
            for command in self.commands() {
                let (kind, ticket) = command.split_once('-').unwrap_or((&command, ""));
                let ts = self.ticket(ticket);
                if busy(ticket) || ticket.is_empty() {
                    // a Ticket's own hold consumes its retry and park, and
                    // sleep owns stop, in every mode
                } else if kind == "retry" && ts.status == STATUS_PARKED && self.consume(&command) {
                    self.update(ticket, |ts| ts.status = STATUS_RUNNING.to_string());
                } else if kind == "address" && self.consume(&command) {
                    launch(ticket, Orchestrator::address);
                } else if ts.status.is_empty() && self.consume(&command) {
                    self.report(ticket, "refused: not a Ticket of this run");
                } else if self.consume(&command) {
                    self.report(ticket, "ignored: not waiting on a Wake");
                }
            }
            if last_poll.is_none_or(|at| at.elapsed() >= self.cfg.poll_prs) {
                self.poll_merges();
                last_poll = Some(Instant::now());
            }

            match self.bd_issues(&["list", "--parent", epic, "--all", "--json"]) {
                Err(err) => self.report("", &format!("bd list failed: {err}")),
                Ok(children) if children.is_empty() => {
                    return Err(format!(
                        "{epic} has no Tickets: is it the id of a beads Epic in this repo?"
                    ))
                }
                Ok(children) if all_closed(&children) => {
                    self.report("", "Epic done, every Ticket closed");
                    return Ok(());
                }
                Ok(_) => {}
            }

            for ticket in self.resumable() {
                if !busy(&ticket) && in_pipeline() < self.cfg.max {
                    launch(&ticket, Orchestrator::run_ticket);
                }
            }
            match self.bd_issues(&["ready", "--parent", epic, "--json"]) {
                Err(err) => self.report("", &format!("bd ready failed: {err}")),
                Ok(ready) => {
                    for issue in ready {
                        if self.ticket(&issue.id).status.is_empty() && in_pipeline() < self.cfg.max
                        {
                            self.update(&issue.id, |_| {});
                            launch(&issue.id, Orchestrator::run_ticket);
                        }
                    }
                }
            }
            if !self.sleep() {
                return Ok(()); // 'harness stop' is a clean end, not a failure
            }
        }
    }

    /// Runs a single Ticket's Pipeline, without scheduling or merge polling,
    /// and reports whether the Ticket ended Parked.
    pub(crate) fn run_single(&self, ticket: &str) -> bool {
        self.drain_commands();
        self.run_ticket(ticket);
        self.ticket(ticket).status == STATUS_PARKED
    }

    /// Tells each open Ticket that depends on `ticket` that it now waits on
    /// the PR's merge (ADR 0002). A single-Ticket run has no Epic and nothing
    /// waiting.
    pub(crate) fn wait_dependents(&self, ticket: &str, pr: &str) {
        let epic = self.state.lock().unwrap().epic.clone();
        if epic.is_empty() {
            return;
        }
        let children = match self.bd_issues(&["list", "--parent", &epic, "--all", "--json"]) {
            Ok(children) => children,
            Err(err) => return self.report("", &format!("bd list failed: {err}")),
        };
        let suffix = ticket.rsplit('.').next().unwrap_or(ticket);
        for child in children {
            let blocked = child
                .dependencies
                .iter()
                .any(|d| d.depends_on_id == ticket && d.kind == "blocks");
            if blocked && child.status != "closed" {
                self.report(
                    &child.id,
                    &format!("waiting for {} to merge (Ticket {suffix})", pr_ref(pr)),
                );
            }
        }
    }

    /// The Tickets the state file says are in the Pipeline.
    fn resumable(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .tickets
            .iter()
            .filter(|(_, ts)| ts.status == STATUS_RUNNING)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Asks gh about every open PR. A merge is what closes a Ticket and so
    /// unblocks its dependents (ADR 0002).
    fn poll_merges(&self) {
        let open: Vec<(String, TicketState)> = self
            .state
            .lock()
            .unwrap()
            .tickets
            .iter()
            .filter(|(_, ts)| ts.status == STATUS_PR_OPEN)
            .map(|(id, ts)| (id.clone(), ts.clone()))
            .collect();
        let (tools, repo) = (&self.cfg.tools, &self.cfg.repo);

        for (ticket, ts) in open {
            let view = tools
                .run(
                    repo,
                    &["gh", "pr", "view", &ts.pr, "--json", "state,mergeable"],
                )
                .map_err(|err| err.to_string())
                .and_then(|out| serde_json::from_str::<GhPr>(&out).map_err(|err| err.to_string()));
            let pr = match view {
                Ok(pr) => pr,
                Err(err) => {
                    self.log(&ticket, &format!("gh pr view failed: {err}"));
                    continue;
                }
            };
            if pr.state == "MERGED" {
                // Closing the Ticket is what unblocks its dependents: until bd
                // has done it the Ticket stays pr-open and the next poll tries
                // again.
                let reason = format!("PR merged: {}", ts.pr);
                if let Err(err) = tools.run(repo, &["bd", "close", &ticket, "--reason", &reason]) {
                    self.log(
                        &ticket,
                        &format!("merged but not closed, will retry: {err}"),
                    );
                    continue;
                }
                // The work is on main now, so bd's cleanliness and containment
                // checks (which a squash merge fails) no longer protect anything.
                let worktree = self.worktree(&ticket).display().to_string();
                if let Err(err) =
                    tools.run(repo, &["bd", "worktree", "remove", &worktree, "--force"])
                {
                    self.log(
                        &ticket,
                        &format!("could not remove the worktree, remove it by hand: {err}"),
                    );
                } else if let Err(err) = tools.run(repo, &["git", "branch", "-D", &ticket]) {
                    self.log(
                        &ticket,
                        &format!("worktree removed, could not delete the branch: {err}"),
                    );
                }
                self.update(&ticket, |ts| ts.status = STATUS_MERGED.to_string());
                self.report(&ticket, "merged, Ticket closed");
            } else if pr.state == "CLOSED" {
                self.update(&ticket, |ts| {
                    ts.status = STATUS_PARKED.to_string();
                    ts.reason = "PR closed without merging".to_string();
                });
                self.report(
                    &ticket,
                    &format!("parked: {} closed without merging", pr_ref(&ts.pr)),
                );
            } else if pr.mergeable == "CONFLICTING" && !ts.conflict {
                self.update(&ticket, |ts| ts.conflict = true);
                self.report(
                    &ticket,
                    &format!(
                        "{} conflicts with main, /address resolves it",
                        pr_ref(&ts.pr)
                    ),
                );
            } else if pr.mergeable == "MERGEABLE" && ts.conflict {
                self.update(&ticket, |ts| ts.conflict = false);
            }
        }
    }

    /// Runs the address Stage for a Ticket with an open PR, on the user's
    /// command only: a fresh session in the kept worktree, fed the PR's
    /// review comments and whether it conflicts with main.
    fn address(&self, ticket: &str) {
        let ts = self.ticket(ticket);
        if ts.status != STATUS_PR_OPEN {
            self.report(ticket, "address refused: no open PR");
            return;
        }
        let feedback = match self.cfg.tools.run(
            &self.cfg.repo,
            &[
                "gh",
                "pr",
                "view",
                &ts.pr,
                "--json",
                "mergeable,reviews,comments",
            ],
        ) {
            Ok(feedback) => feedback,
            Err(err) => {
                self.report(ticket, &format!("address failed: {err}"));
                return;
            }
        };
        // every address run is a new one
        let _ = fs::remove_file(self.run_dir(ticket).join(result_name(&ADDRESS, 0)));
        let inputs = address_inputs(&ts.pr, &feedback);
        let inputs: Vec<(&str, &str)> = inputs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let result = self.run_stage(ticket, &ADDRESS, 0, &inputs, ResultRequirements::default());
        if self.stopping() {
            return;
        }
        match result {
            Ok(_) => {
                let tab = self.ticket(ticket).tab;
                if !tab.is_empty() {
                    let _ = self.herdr(&["tab", "close", &tab]);
                    self.update(ticket, |ts| {
                        ts.tab.clear();
                        ts.panes.clear();
                    });
                }
                self.report(ticket, &format!("addressed {}", pr_ref(&ts.pr)));
            }
            Err(StageError::Parked(reason)) => {
                self.report(ticket, &format!("address gave up: {reason}"));
            }
            Err(StageError::Stopped) => {}
        }
    }
}

/// A Ticket's place in the Pipeline, given back when its thread ends, also
/// by a panic: Go's `defer active.Delete`.
struct Slot {
    o: Arc<Orchestrator>,
    ticket: String,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.o.active.lock().unwrap().remove(&self.ticket);
    }
}

fn all_closed(children: &[BdIssue]) -> bool {
    children.iter().all(|child| child.status == "closed")
}

/// The address Stage's inputs, from gh's view of the PR.
pub(crate) fn address_inputs(pr: &str, gh_json: &str) -> Vec<(String, String)> {
    let mut conflicts = "unknown, check with gh";
    if let Ok(view) = serde_json::from_str::<GhPr>(gh_json) {
        if !view.mergeable.is_empty() && view.mergeable != "UNKNOWN" {
            conflicts = if view.mergeable == "CONFLICTING" {
                "yes"
            } else {
                "no"
            };
        }
    }
    [
        ("PR", pr),
        ("Conflicts with main", conflicts),
        ("Review comments (gh JSON)", gh_json.trim()),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .to_vec()
}
