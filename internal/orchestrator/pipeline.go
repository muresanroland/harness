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

	if _, err := o.runStage(ctx, ticket, stageImplement, 0, nil, nil); err != nil {
		return err
	}
	o.report("%s implement done", ticket)

	var verdicts []string
	for round := 1; round <= maxRounds; round++ {
		review, err := o.runStage(ctx, ticket, stageReview, round, nil, nil)
		if err != nil {
			return err
		}
		o.report("%s review %d done: %d findings", ticket, round, findingCount(review))

		// A Verdict settles every Finding exactly once. One that lists fewer
		// than the Review raised has lost some, or ignored the format, and must
		// not read as "nothing to fix". (It may list more: the Moderator adds
		// the over-engineering audit's Findings.)
		raised := findingCount(review)
		settlesAll := func(verdict string) string {
			if fix, skip := verdictCounts(verdict); fix+skip < raised {
				return fmt.Sprintf("Verdict settles %d of the Review's %d Findings", fix+skip, raised)
			}
			return ""
		}
		reviewFile := filepath.Join(o.runDir(ticket), resultName(stageReview, round))
		verdict, err := o.runStage(ctx, ticket, stageDebate, round, [][2]string{{"Review file", reviewFile}}, settlesAll)
		if err != nil {
			return err
		}
		fixes, skips := verdictCounts(verdict)
		o.report("%s debate %d done: %d to fix, %d skipped", ticket, round, fixes, skips)
		verdicts = append(verdicts, filepath.Join(o.runDir(ticket), resultName(stageDebate, round)))

		// The Fix session always runs, even with nothing to fix, because the
		// last one opens the pull request. It is given only the fix items; the
		// last one also gets the Verdict files, for the PR description.
		last := fixes == 0 || round == maxRounds
		items := "none"
		if fixes > 0 {
			items = "\n  " + strings.Join(fixItems(verdict), "\n  ")
		}
		inputs := [][2]string{{"Open PR", "no"}, {"Fix items", items}}
		var valid func(string) string
		if last {
			inputs[0][1] = "yes"
			inputs = append(inputs, [2]string{"Verdict history", strings.Join(verdicts, ", ")})
			valid = func(body string) string {
				if prURL(body) == "" {
					return "wrote a done result without a 'PR:' line"
				}
				return ""
			}
		}
		fix, err := o.runStage(ctx, ticket, stageFix, round, inputs, valid)
		if err != nil {
			return err
		}
		o.report("%s fix %d done", ticket, round)
		if !last {
			continue
		}

		tab := o.ticket(ticket).Tab
		o.update(ticket, func(ts *TicketState) {
			ts.Status, ts.PR, ts.Tab, ts.Panes = statusPROpen, prURL(fix), "", nil
		})
		if tab != "" {
			o.herdr("tab", "close", tab)
		}
		o.report("%s pr open after %d round(s): %s", ticket, round, prURL(fix))
		return nil
	}
	return nil
}

// prepareWorktree creates the Ticket's worktree and branch once, brings the
// new branch up to the remote's default branch so a dependent Ticket builds on
// what was just merged (ADR 0002), and marks the Ticket in progress.
func (o *Orchestrator) prepareWorktree(ticket string) error {
	if _, err := os.Stat(o.worktree(ticket)); err == nil {
		return nil
	}
	if _, err := o.run(o.repo, "bd", "worktree", "create", o.worktree(ticket), "--branch", ticket); err != nil {
		return fmt.Errorf("worktree not created: %w", err)
	}
	if _, err := o.run(o.worktree(ticket), "git", "pull", "--ff-only", "origin", "HEAD"); err != nil {
		// Building on a stale main is what ADR 0002 exists to prevent; leave
		// nothing behind so a retry prepares the worktree again.
		o.run(o.repo, "bd", "worktree", "remove", o.worktree(ticket))
		o.run(o.repo, "git", "branch", "-D", ticket)
		return fmt.Errorf("new branch not brought up to origin's default branch: %w", err)
	}
	if _, err := o.run(o.repo, "bd", "update", ticket, "--status", "in_progress"); err != nil {
		o.log.Printf("%s: not marked in_progress: %v", ticket, err)
	}
	return nil
}
