package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
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
	out, err := o.run(o.repo, "bd", args...)
	if err != nil {
		return nil, err
	}
	var issues []bdIssue
	if err := json.Unmarshal([]byte(out), &issues); err != nil {
		return nil, fmt.Errorf("bd %s: unreadable reply: %w", args[0], err)
	}
	tickets := issues[:0]
	for _, issue := range issues {
		if issue.IssueType != "epic" {
			tickets = append(tickets, issue)
		}
	}
	return tickets, nil
}

// Run drives an Epic: it starts ready Tickets, at most o.max at once, resumes
// the ones a killed run left behind, polls PRs for merges, obeys control
// commands, and returns when every child Ticket is closed or on 'stop'.
func (o *Orchestrator) Run(ctx context.Context, epic string) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	o.mu.Lock()
	o.state.Epic = epic
	o.mu.Unlock()

	var active sync.Map // ticket -> running in this process
	launch := func(ticket string, work func(context.Context, string)) {
		active.Store(ticket, true)
		o.sessions.Add(1)
		go func() {
			defer o.sessions.Done()
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
		if o.consume("stop") {
			o.report("stopped; live panes left alone, 'harness start %s' resumes", epic)
			return nil
		}
		for _, command := range o.commands() {
			kind, ticket, _ := strings.Cut(command, "-")
			_, busy := active.Load(ticket)
			ts := o.ticket(ticket)
			switch {
			case busy || ticket == "":
				// a Ticket's own hold consumes its retry and park
			case kind == "retry" && ts.Status == statusParked && o.consume(command):
				o.update(ticket, func(ts *TicketState) { ts.Status = statusRunning })
			case kind == "address" && o.consume(command):
				launch(ticket, o.address)
			case ts.Status == "" && o.consume(command):
				o.report("%s %s refused: not a Ticket of this run", ticket, kind)
			}
		}
		if time.Since(lastPoll) >= o.pollPRs {
			o.pollMerges()
			lastPoll = time.Now()
		}

		children, err := o.bdIssues("list", "--parent", epic, "--all", "--json")
		if err != nil {
			o.log.Printf("bd list: %v", err)
		} else if len(children) == 0 {
			return fmt.Errorf("%s has no Tickets: is it the id of a beads Epic in this repo?", epic)
		} else if allClosed(children) {
			o.report("epic %s done: every Ticket is closed", epic)
			return nil
		}

		for _, ticket := range o.resumable() {
			if _, busy := active.Load(ticket); !busy && inPipeline() < o.max {
				launch(ticket, o.runTicket)
			}
		}
		ready, err := o.bdIssues("ready", "--parent", epic, "--json")
		if err != nil {
			o.log.Printf("bd ready: %v", err)
		}
		for _, issue := range ready {
			if o.ticket(issue.ID).Status == "" && inPipeline() < o.max {
				o.update(issue.ID, func(*TicketState) {})
				launch(issue.ID, o.runTicket)
			}
		}
		if !o.sleep(ctx) {
			return ctx.Err()
		}
	}
}

// RunTicket runs a single Ticket's Pipeline, without scheduling or merge polling.
func (o *Orchestrator) RunTicket(ctx context.Context, ticket string) {
	o.runTicket(ctx, ticket)
}

func allClosed(children []bdIssue) bool {
	for _, child := range children {
		if child.Status != "closed" {
			return false
		}
	}
	return true
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
	sort.Strings(tickets)
	return tickets
}

// commands lists the control files waiting in .harness/control.
// ponytail: a polled directory; a socket if latency ever matters.
func (o *Orchestrator) commands() []string {
	entries, _ := os.ReadDir(filepath.Dir(o.controlPath("x")))
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
		out, err := o.run(o.repo, "gh", "pr", "view", ts.PR, "--json", "state,mergeable")
		var pr ghPR
		if err == nil {
			err = json.Unmarshal([]byte(out), &pr)
		}
		if err != nil {
			o.log.Printf("%s: gh pr view: %v", ticket, err)
			continue
		}
		switch {
		case pr.State == "MERGED":
			for _, cleanup := range [][]string{
				{"bd", "close", ticket, "--reason", "PR merged: " + ts.PR},
				{"bd", "worktree", "remove", o.worktree(ticket)},
				{"git", "branch", "-D", ticket},
			} {
				if _, err := o.run(o.repo, cleanup[0], cleanup[1:]...); err != nil {
					o.log.Printf("%s: %v", ticket, err)
				}
			}
			o.update(ticket, func(ts *TicketState) { ts.Status = statusMerged })
			o.report("%s merged: Ticket closed, worktree and branch removed", ticket)
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
	feedback, err := o.run(o.repo, "gh", "pr", "view", ts.PR, "--json", "mergeable,reviews,comments")
	if err != nil {
		o.report("%s address failed: %v", ticket, err)
		return
	}
	os.Remove(filepath.Join(o.runDir(ticket), resultName(stageAddress, 0))) // every address run is a new one
	_, err = o.runStage(ctx, ticket, stageAddress, 0, addressInputs(ts.PR, feedback), nil)
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
	json.Unmarshal([]byte(ghJSON), &view)
	conflicts := "no"
	if view.Mergeable == "CONFLICTING" {
		conflicts = "yes"
	}
	return [][2]string{{"PR", pr}, {"Conflicts with main", conflicts}, {"Review comments (gh JSON)", strings.TrimSpace(ghJSON)}}
}
