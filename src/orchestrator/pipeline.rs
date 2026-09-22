//! The Pipeline: Implement, then Rounds of Review, Debate and Fix until a
//! Verdict has no fix items or the cap, then the pull request.

use std::fs;

use super::result::ResultRequirements;
use super::stage::{
    plural, pr_ref, result_name, Orchestrator, StageError, DEBATE, FIX, IMPLEMENT, REVIEW,
};
use super::state::{STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};

const MAX_ROUNDS: usize = 3;

/// What a run keeps for whoever reads it later: the Stages' result files,
/// diffs and debate transcripts, all flat text.
const EVIDENCE: [&str; 5] = ["md", "txt", "patch", "json", "sh"];

impl Orchestrator {
    /// Moves one Ticket through the Pipeline: Implement, then Rounds of
    /// Review, Debate and Fix until a Verdict has no fix items or the cap is
    /// reached, ending with an open pull request. Finished Stages are skipped
    /// by their result files, so calling it again resumes where a stopped run
    /// stopped.
    pub(crate) fn run_ticket(&self, ticket: &str) {
        match self.pipeline(ticket) {
            Ok(()) | Err(StageError::Stopped) => {}
            Err(StageError::Parked(reason)) => {
                self.update(ticket, |ts| {
                    ts.status = STATUS_PARKED.to_string();
                    ts.reason = reason.clone();
                });
                self.report(ticket, &format!("parked: {reason}"));
            }
        }
    }

    fn pipeline(&self, ticket: &str) -> Result<(), StageError> {
        self.update(ticket, |ts| {
            ts.status = STATUS_RUNNING.to_string();
            ts.reason.clear();
        });
        self.prepare_worktree(ticket)?;

        self.run_stage(ticket, &IMPLEMENT, 0, &[], ResultRequirements::default())?;
        self.report(ticket, "implemented");

        let mut verdicts = Vec::new();
        for round in 1..=MAX_ROUNDS {
            let review =
                self.run_stage(ticket, &REVIEW, round, &[], ResultRequirements::default())?;
            self.report(
                ticket,
                &format!(
                    "review {round} found {}",
                    plural(review.findings, "finding")
                ),
            );
            let review_file = self.run_dir(ticket).join(result_name(&REVIEW, round));
            let verdict = self.run_stage(
                ticket,
                &DEBATE,
                round,
                &[("Review file", &review_file.display().to_string())],
                ResultRequirements {
                    review_findings: review.findings,
                    ..Default::default()
                },
            )?;
            let fixes = verdict.fixes;
            self.report(
                ticket,
                &format!(
                    "debate {round} settled: {} to fix, {} skipped",
                    fixes.len(),
                    verdict.skips
                ),
            );
            verdicts.push(
                self.run_dir(ticket)
                    .join(result_name(&DEBATE, round))
                    .display()
                    .to_string(),
            );

            // The Fix session always runs, even with nothing to fix, because
            // the last one opens the pull request. It is given only the fix
            // items; the last one also gets the Verdict files, for the PR
            // description.
            let last = fixes.is_empty() || round == MAX_ROUNDS;
            let items = if fixes.is_empty() {
                "none".to_string()
            } else {
                format!("\n  {}", fixes.join("\n  "))
            };
            let history = verdicts.join(", ");
            let mut inputs = vec![("Open PR", "no"), ("Fix items", items.as_str())];
            if last {
                inputs[0].1 = "yes";
                inputs.push(("Verdict history", history.as_str()));
            }
            let fix = self.run_stage(
                ticket,
                &FIX,
                round,
                &inputs,
                ResultRequirements {
                    require_pr: last,
                    ..Default::default()
                },
            )?;
            self.report(ticket, &format!("fix {round} done"));
            if !last {
                continue;
            }

            let tab = self.ticket(ticket).tab;
            self.update(ticket, |ts| {
                ts.status = STATUS_PR_OPEN.to_string();
                ts.pr = fix.pr.clone();
                ts.tab.clear();
                ts.panes.clear();
            });
            if !tab.is_empty() {
                let _ = self.herdr(&["tab", "close", &tab]);
            }
            self.emit(
                ticket,
                &format!(
                    "{} opened after {}",
                    pr_ref(&fix.pr),
                    plural(round, "round")
                ),
                true,
                &fix.pr, // the log line adds the url
            );
            self.wait_dependents(ticket, &fix.pr);
            self.prune_run_dir(ticket);
            return Ok(());
        }
        Ok(())
    }

    /// Drops a Ticket's build scratch once its pull request is open and its
    /// Pipeline is over. The run directory is the Codex sandbox's only
    /// writable root, so a Stage that has to compile puts its build cache
    /// there: a Go cache runs to some 100MB per Ticket, and nothing reads it
    /// again. Keeping only the evidence survives the next Stage inventing a
    /// fifth name for its cache. Best effort: scratch that cannot be removed
    /// is only disk.
    fn prune_run_dir(&self, ticket: &str) {
        let dir = self.run_dir(ticket);
        let Ok(entries) = fs::read_dir(&dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let evidence = path
                .extension()
                .is_some_and(|ext| EVIDENCE.contains(&ext.to_string_lossy().as_ref()));
            if path.is_dir() || !evidence {
                let removed = if path.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                if let Err(err) = removed {
                    self.log(ticket, &format!("scratch left in the run directory: {err}"));
                }
            }
        }
    }

    /// Creates the Ticket's worktree and branch once, brings the new branch
    /// up to the remote's default branch so a dependent Ticket builds on what
    /// was just merged (ADR 0002), and marks the Ticket in progress.
    fn prepare_worktree(&self, ticket: &str) -> Result<(), StageError> {
        let worktree = self.worktree(ticket);
        if worktree.exists() {
            return Ok(());
        }
        let tools = &self.cfg.tools;
        let repo = &self.cfg.repo;
        let path = worktree.display().to_string();
        tools
            .run(
                repo,
                &["bd", "worktree", "create", &path, "--branch", ticket],
            )
            .map_err(|err| StageError::Parked(format!("worktree not created: {err}")))?;
        if let Err(err) = tools.run(&worktree, &["git", "pull", "--ff-only", "origin", "HEAD"]) {
            // Building on a stale main is what ADR 0002 exists to prevent;
            // leave nothing behind so a retry prepares the worktree again.
            let _ = tools.run(repo, &["bd", "worktree", "remove", &path]);
            let _ = tools.run(repo, &["git", "branch", "-D", ticket]);
            return Err(StageError::Parked(format!(
                "new branch not brought up to origin's default branch: {err}"
            )));
        }
        if let Err(err) = tools.run(repo, &["bd", "update", ticket, "--status", "in_progress"]) {
            self.log(ticket, &format!("not marked in_progress: {err}"));
        }
        self.report(ticket, &format!("branch {ticket} created"));
        Ok(())
    }
}
