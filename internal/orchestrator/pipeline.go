package orchestrator

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const maxRounds = 3

// runTicket moves one Ticket through the Pipeline: Implement, then Rounds of
// Review, Debate and Fix until a Verdict has no fix items or the cap is
// reached, ending with an open pull request. Finished Stages are skipped by
// their result files, so calling it again resumes where a killed run stopped.
func (o *Orchestrator) runTicket(ctx context.Context, ticket string) {
	err := o.pipeline(ctx, ticket)
	switch {
	case errors.Is(err, errParked):
		reason := strings.TrimPrefix(err.Error(), errParked.Error()+": ")
		o.update(ticket, func(ts *TicketState) { ts.Status, ts.Reason = statusParked, reason })
		o.report("%s parked: %s", ticket, reason)
	case err != nil && ctx.Err() == nil:
		o.update(ticket, func(ts *TicketState) { ts.Status, ts.Reason = statusParked, err.Error() })
		o.report("%s parked: %v", ticket, err)
	}
}

func (o *Orchestrator) pipeline(ctx context.Context, ticket string) error {
	o.update(ticket, func(ts *TicketState) { ts.Status, ts.Reason = statusRunning, "" })
	if err := o.prepareWorktree(ticket); err != nil {
		return err
	}

	if _, err := o.runStage(ctx, ticket, stageImplement, 0, nil, resultRequirements{}); err != nil {
		return err
	}
	o.report("%s implement done", ticket)

	var verdicts []string
	for round := 1; round <= maxRounds; round++ {
		review, err := o.runStage(ctx, ticket, stageReview, round, nil, resultRequirements{})
		if err != nil {
			return err
		}
		o.report("%s review %d done: %d findings", ticket, round, review.Findings)
		reviewFile := filepath.Join(o.runDir(ticket), resultName(stageReview, round))
		verdict, err := o.runStage(ctx, ticket, stageDebate, round, [][2]string{{"Review file", reviewFile}}, resultRequirements{ReviewFindings: review.Findings})
		if err != nil {
			return err
		}
		fixes := verdict.Fixes
		o.report("%s debate %d done: %d to fix, %d skipped", ticket, round, len(fixes), verdict.Skips)
		verdicts = append(verdicts, filepath.Join(o.runDir(ticket), resultName(stageDebate, round)))

		// The Fix session always runs, even with nothing to fix, because the
		// last one opens the pull request. It is given only the fix items; the
		// last one also gets the Verdict files, for the PR description.
		last := len(fixes) == 0 || round == maxRounds
		items := "none"
		if len(fixes) > 0 {
			items = "\n  " + strings.Join(fixes, "\n  ")
		}
		inputs := [][2]string{{"Open PR", "no"}, {"Fix items", items}}
		if last {
			inputs[0][1] = "yes"
			inputs = append(inputs, [2]string{"Verdict history", strings.Join(verdicts, ", ")})
		}
		fix, err := o.runStage(ctx, ticket, stageFix, round, inputs, resultRequirements{RequirePR: last})
		if err != nil {
			return err
		}
		o.report("%s fix %d done", ticket, round)
		if !last {
			continue
		}

		tab := o.ticket(ticket).Tab
		o.update(ticket, func(ts *TicketState) {
			ts.Status, ts.PR, ts.Tab, ts.Panes = statusPROpen, fix.PR, "", nil
		})
		if tab != "" {
			o.herdr("tab", "close", tab)
		}
		o.report("%s pr open after %d round(s): %s", ticket, round, fix.PR)
		o.pruneRunDir(ticket)
		return nil
	}
	return nil
}

// evidence is what a run keeps for whoever reads it later: the Stages' result
// files, diffs and debate transcripts, all flat text.
var evidence = map[string]bool{".md": true, ".txt": true, ".patch": true, ".json": true, ".sh": true}

// pruneRunDir drops a Ticket's build scratch once its pull request is open and
// its Pipeline is over. The run directory is the Codex sandbox's only writable
// root, so a Stage that has to compile puts its build cache there: a Go cache
// runs to some 100MB per Ticket, and nothing reads it again. Keeping only the
// evidence survives the next Stage inventing a fifth name for its cache. Best
// effort: scratch that cannot be removed is only disk.
func (o *Orchestrator) pruneRunDir(ticket string) {
	dir := o.runDir(ticket)
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	for _, e := range entries {
		if e.IsDir() || !evidence[filepath.Ext(e.Name())] {
			if err := os.RemoveAll(filepath.Join(dir, e.Name())); err != nil {
				o.Log.Printf("%s: scratch left in the run directory: %v", ticket, err)
			}
		}
	}
}

// prepareWorktree creates the Ticket's worktree and branch once, brings the
// new branch up to the remote's default branch so a dependent Ticket builds on
// what was just merged (ADR 0002), and marks the Ticket in progress.
func (o *Orchestrator) prepareWorktree(ticket string) error {
	if _, err := os.Stat(o.worktree(ticket)); err == nil {
		return nil
	}
	if _, err := o.Exec(o.Repo, "bd", "worktree", "create", o.worktree(ticket), "--branch", ticket); err != nil {
		return fmt.Errorf("worktree not created: %w", err)
	}
	if _, err := o.Exec(o.worktree(ticket), "git", "pull", "--ff-only", "origin", "HEAD"); err != nil {
		// Building on a stale main is what ADR 0002 exists to prevent; leave
		// nothing behind so a retry prepares the worktree again.
		o.Exec(o.Repo, "bd", "worktree", "remove", o.worktree(ticket))
		o.Exec(o.Repo, "git", "branch", "-D", ticket)
		return fmt.Errorf("new branch not brought up to origin's default branch: %w", err)
	}
	if _, err := o.Exec(o.Repo, "bd", "update", ticket, "--status", "in_progress"); err != nil {
		o.Log.Printf("%s: not marked in_progress: %v", ticket, err)
	}
	o.report("%s worktree ready on branch %s, Ticket in_progress", ticket, ticket)
	return nil
}
