package orchestrator

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"
)

type bdIssue struct {
	ID        string `json:"id"`
	Status    string `json:"status"`
	IssueType string `json:"issue_type"`
}

func (o *Orchestrator) bdIssues(args ...string) ([]bdIssue, error) {
	out, err := o.Exec(o.Repo, "bd", args...)
	if err != nil {
		return nil, err
	}
	var issues []bdIssue
	if err := json.Unmarshal([]byte(out), &issues); err != nil {
		return nil, fmt.Errorf("bd %s: unreadable reply: %w", args[0], err)
	}
	return slices.DeleteFunc(issues, func(issue bdIssue) bool { return issue.IssueType == "epic" }), nil
}

// Run drives an Epic: it starts ready Tickets, at most o.Max at once, resumes
// the ones a killed run left behind, polls PRs for merges, obeys control
// commands, and returns when every child Ticket is closed or on 'stop'.
func (o *Orchestrator) Run(ctx context.Context, epic string) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	o.cancel = cancel
	o.drainCommands()
	o.mu.Lock()
	o.state.Epic = epic
	o.mu.Unlock()

	var active sync.Map // ticket -> running in this process
	launch := func(ticket string, work func(context.Context, string)) {
		// A Ticket can consume stop while the scheduler is in a bd call.
		// Its saved state stays running for resume, but this run is over.
		if ctx.Err() != nil {
			return
		}
		active.Store(ticket, true)
		o.inFlight.Add(1)
		go func() {
			defer o.inFlight.Done()
			defer active.Delete(ticket)
			work(ctx, ticket)
		}()
	}
	inPipeline := func() (n int) {
		active.Range(func(_, _ any) bool { n++; return true })
		return n
	}

	var lastPoll time.Time
	for {
		for _, command := range o.commands() {
			kind, ticket, _ := strings.Cut(command, "-")
			_, busy := active.Load(ticket)
			ts := o.ticket(ticket)
			switch {
			case busy || ticket == "":
				// a Ticket's own hold consumes its retry and park
			case kind == "retry" && ts.Status == statusParked && o.consume(command):
				o.update(ticket, func(ts *TicketState) { ts.Status = statusRunning })
			case command == "stop": // sleep owns it, in every mode
			case kind == "address" && o.consume(command):
				launch(ticket, o.address)
			case ts.Status == "" && o.consume(command):
				o.report("%s %s refused: not a Ticket of this run", ticket, kind)
			case o.consume(command):
				o.report("%s %s ignored: the Ticket is %s and not waiting on a Wake", ticket, kind, ts.Status)
			}
		}
		if time.Since(lastPoll) >= o.PollPRs {
			o.pollMerges()
			lastPoll = time.Now()
		}

		children, err := o.bdIssues("list", "--parent", epic, "--all", "--json")
		if err != nil {
			o.Log.Printf("bd list: %v", err)
		} else if len(children) == 0 {
			return fmt.Errorf("%s has no Tickets: is it the id of a beads Epic in this repo?", epic)
		} else if allClosed(children) {
			o.report("epic %s done: every Ticket is closed", epic)
			return nil
		}

		for _, ticket := range o.resumable() {
			if _, busy := active.Load(ticket); !busy && inPipeline() < o.Max {
				launch(ticket, o.runTicket)
			}
		}
		ready, err := o.bdIssues("ready", "--parent", epic, "--json")
		if err != nil {
			o.Log.Printf("bd ready: %v", err)
		}
		for _, issue := range ready {
			if o.ticket(issue.ID).Status == "" && inPipeline() < o.Max {
				o.update(issue.ID, func(*TicketState) {})
				launch(issue.ID, o.runTicket)
			}
		}
		if !o.sleep(ctx) {
			if o.stopping() {
				return nil // 'harness stop' is a clean end, not a failure
			}
			return ctx.Err()
		}
	}
}

// RunTicket runs a single Ticket's Pipeline, without scheduling or merge
// polling, and reports whether the Ticket ended Parked.
func (o *Orchestrator) RunTicket(ctx context.Context, ticket string) (parked bool) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	o.cancel = cancel
	o.drainCommands()
	o.runTicket(ctx, ticket)
	return o.ticket(ticket).Status == statusParked
}

func allClosed(children []bdIssue) bool {
	return !slices.ContainsFunc(children, func(child bdIssue) bool { return child.Status != "closed" })
}

// resumable lists Tickets the state file says are in the Pipeline.
func (o *Orchestrator) resumable() []string {
	o.mu.Lock()
	defer o.mu.Unlock()
	var tickets []string
	for id, ts := range o.state.Tickets {
		if ts.Status == statusRunning {
			tickets = append(tickets, id)
		}
	}
	slices.Sort(tickets)
	return tickets
}

// commands lists the control files waiting for the Orchestrator.
func (o *Orchestrator) commands() []string {
	entries, _ := os.ReadDir(filepath.Dir(ControlFile(o.Repo, "x")))
	var names []string
	for _, entry := range entries {
		names = append(names, entry.Name())
	}
	return names
}

type ghPR struct {
	State     string `json:"state"`
	Mergeable string `json:"mergeable"`
}

// pollMerges asks gh about every open PR. A merge is what closes a Ticket and
// so unblocks its dependents (ADR 0002).
func (o *Orchestrator) pollMerges() {
	o.mu.Lock()
	open := map[string]TicketState{}
	for id, ts := range o.state.Tickets {
		if ts.Status == statusPROpen {
			open[id] = *ts
		}
	}
	o.mu.Unlock()

	for ticket, ts := range open {
		out, err := o.Exec(o.Repo, "gh", "pr", "view", ts.PR, "--json", "state,mergeable")
		var pr ghPR
		if err == nil {
			err = json.Unmarshal([]byte(out), &pr)
		}
		if err != nil {
			o.Log.Printf("%s: gh pr view: %v", ticket, err)
			continue
		}
		switch {
		case pr.State == "MERGED":
			// Closing the Ticket is what unblocks its dependents: until bd has
			// done it the Ticket stays pr-open and the next poll tries again.
			if _, err := o.Exec(o.Repo, "bd", "close", ticket, "--reason", "PR merged: "+ts.PR); err != nil {
				o.Log.Printf("%s: merged but not closed, will retry: %v", ticket, err)
				continue
			}
			// The work is on main now, so bd's cleanliness and containment
			// checks (which a squash merge fails) no longer protect anything.
			cleanup := "worktree and branch removed"
			if _, err := o.Exec(o.Repo, "bd", "worktree", "remove", o.worktree(ticket), "--force"); err != nil {
				o.Log.Printf("%s: %v", ticket, err)
				cleanup = "could not remove the worktree, remove it by hand"
			} else if _, err := o.Exec(o.Repo, "git", "branch", "-D", ticket); err != nil {
				o.Log.Printf("%s: %v", ticket, err)
				cleanup = "worktree removed, could not delete the branch"
			}
			o.update(ticket, func(ts *TicketState) { ts.Status = statusMerged })
			o.report("%s merged: Ticket closed, %s", ticket, cleanup)
		case pr.State == "CLOSED":
			o.update(ticket, func(ts *TicketState) { ts.Status, ts.Reason = statusParked, "PR closed without merging" })
			o.report("%s parked: PR closed without merging: %s", ticket, ts.PR)
		case pr.Mergeable == "CONFLICTING" && !ts.Conflict:
			o.update(ticket, func(ts *TicketState) { ts.Conflict = true })
			o.report("%s pr conflicts with main: %s ('harness address %s' resolves it)", ticket, ts.PR, ticket)
		case pr.Mergeable == "MERGEABLE" && ts.Conflict:
			o.update(ticket, func(ts *TicketState) { ts.Conflict = false })
		}
	}
}

// address runs the address Stage for a Ticket with an open PR, on the user's
// command only: a fresh session in the kept worktree, fed the PR's review
// comments and whether it conflicts with main.
func (o *Orchestrator) address(ctx context.Context, ticket string) {
	ts := o.ticket(ticket)
	if ts.Status != statusPROpen {
		o.report("%s address refused: the Ticket has no open PR", ticket)
		return
	}
	feedback, err := o.Exec(o.Repo, "gh", "pr", "view", ts.PR, "--json", "mergeable,reviews,comments")
	if err != nil {
		o.report("%s address failed: %v", ticket, err)
		return
	}
	os.Remove(filepath.Join(o.runDir(ticket), resultName(stageAddress, 0))) // every address run is a new one
	_, err = o.runStage(ctx, ticket, stageAddress, 0, addressInputs(ts.PR, feedback), resultRequirements{})
	if ctx.Err() != nil {
		return
	}
	if tab := o.ticket(ticket).Tab; tab != "" && err == nil {
		o.herdr("tab", "close", tab)
		o.update(ticket, func(ts *TicketState) { ts.Tab, ts.Panes = "", nil })
	}
	if err != nil {
		o.report("%s address gave up: %v", ticket, err)
		return
	}
	o.report("%s address done: %s", ticket, ts.PR)
}

// addressInputs are the address Stage's inputs, from gh's view of the PR.
func addressInputs(pr, ghJSON string) [][2]string {
	var view ghPR
	conflicts := "unknown, check with gh"
	if json.Unmarshal([]byte(ghJSON), &view) == nil && view.Mergeable != "" && view.Mergeable != "UNKNOWN" {
		conflicts = "no"
		if view.Mergeable == "CONFLICTING" {
			conflicts = "yes"
		}
	}
	return [][2]string{{"PR", pr}, {"Conflicts with main", conflicts}, {"Review comments (gh JSON)", strings.TrimSpace(ghJSON)}}
}
