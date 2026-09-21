// Package orchestrator is the deterministic process that owns Ticket state,
// pane placement and Stage transitions (ADR 0001).
package orchestrator

import (
	"context"
	"errors"
	"fmt"
	"log"
	"maps"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"

	"harness/internal/runner"
)

// Stage is one step of the Pipeline, carried out by a fresh agent session in
// its own pane.
type Stage struct {
	Name    string
	Skill   string
	Kind    string // herdr agent kind: claude or codex
	Timeout time.Duration
}

var (
	stageImplement = Stage{"implement", "stage-implement", "claude", 60 * time.Minute}
	stageReview    = Stage{"review", "stage-review", "codex", 30 * time.Minute}
	stageDebate    = Stage{"debate", "stage-moderate", "claude", 30 * time.Minute}
	stageFix       = Stage{"fix", "stage-fix", "claude", 60 * time.Minute}
	stageAddress   = Stage{"address", "stage-address", "claude", 60 * time.Minute}
)

// Orchestrator owns Ticket state, pane placement and Stage transitions. It
// makes no judgment calls: what it cannot advance by rule becomes a Wake.
type Orchestrator struct {
	Config

	mu       sync.Mutex // guards state
	state    *State
	reportMu sync.Mutex     // one line at a time into the Main session
	unsent   []string       // lines the Main session has not taken yet, oldest first
	inFlight sync.WaitGroup // Ticket goroutines; Run does not wait for them, because 'stop' must not wait out a Stage
}

// Config is what 'harness start' knows at launch.
type Config struct {
	Exec      runner.Runner // the seam to every external tool
	Repo      string        // the Target repo's root
	MainPane  string        // HERDR_PANE_ID of the Main session
	Workspace string        // HERDR_WORKSPACE_ID
	APIKey    string        // TYPESAFE_API_KEY, handed to the Debate pane
	Tick      time.Duration // how often holds, control files and bd are polled
	PollPRs   time.Duration // how often gh is asked about open PRs
	Max       int           // Tickets in the Pipeline at once
	Log       *log.Logger
}

// New loads the Target repo's state file, so a restarted Orchestrator resumes.
func New(cfg Config) (*Orchestrator, error) {
	state, err := loadState(cfg.Repo)
	if err != nil {
		return nil, err
	}
	return &Orchestrator{Config: cfg, state: state}, nil
}

var errParked = errors.New("parked")

func (o *Orchestrator) runDir(ticket string) string {
	return filepath.Join(o.Repo, ".harness", "runs", ticket)
}

func (o *Orchestrator) worktree(ticket string) string {
	return filepath.Join(o.Repo, ".harness", "worktrees", ticket)
}

func resultName(st Stage, round int) string {
	switch st.Name {
	case "debate":
		return fmt.Sprintf("verdict-%d.md", round)
	case "review", "fix":
		return fmt.Sprintf("%s-%d.md", st.Name, round)
	}
	return st.Name + ".md"
}

// report sends one event line into the Main session's pane. herdr refuses a
// prompt while the Main session is blocked on a prompt of its own, so a line
// that cannot be delivered is kept and sent, in order, once it can be.
func (o *Orchestrator) report(format string, args ...any) {
	line := "[harness] " + fmt.Sprintf(format, args...)
	o.Log.Print(line)
	o.reportMu.Lock()
	o.unsent = append(o.unsent, line)
	o.reportMu.Unlock()
	o.flush()
}

func (o *Orchestrator) flush() {
	o.reportMu.Lock()
	defer o.reportMu.Unlock()
	for len(o.unsent) > 0 {
		if _, err := o.Exec(o.Repo, "herdr", "agent", "prompt", o.MainPane, o.unsent[0]); err != nil {
			o.Log.Printf("the Main session did not take %q, will retry: %v", o.unsent[0], err)
			return
		}
		o.unsent = o.unsent[1:]
	}
}

// update changes one Ticket's state and writes the state file.
func (o *Orchestrator) update(ticket string, change func(*TicketState)) {
	o.mu.Lock()
	defer o.mu.Unlock()
	ts := o.state.Tickets[ticket]
	if ts == nil {
		ts = &TicketState{Status: statusRunning}
		o.state.Tickets[ticket] = ts
	}
	change(ts)
	if err := o.state.save(o.Repo); err != nil {
		o.Log.Printf("state not saved: %v", err)
	}
}

func (o *Orchestrator) ticket(ticket string) TicketState {
	o.mu.Lock()
	defer o.mu.Unlock()
	ts := o.state.Tickets[ticket]
	if ts == nil {
		return TicketState{}
	}
	snapshot := *ts
	snapshot.Panes = maps.Clone(ts.Panes)
	return snapshot
}

// runStage runs one Stage to its completion rule and returns its accepted
// result. A Stage with an already accepted result is not rerun. When it cannot
// advance by rule the Main session is woken and the Ticket holds for a retry,
// a park, or a late done result; errParked means the Ticket left the Pipeline.
func (o *Orchestrator) runStage(ctx context.Context, ticket string, st Stage, round int, inputs [][2]string, want resultRequirements) (stageResult, error) {
	file := filepath.Join(o.runDir(ticket), resultName(st, round))
	if result, reason := readStageResult(file, want); reason == "" {
		return result, nil
	}
	o.update(ticket, func(ts *TicketState) {
		if ts.Stage != st.Name || ts.Round != round { // a resumed Stage keeps its spent retry
			ts.Stage, ts.Round, ts.Retried = st.Name, round, false
		}
	})
	inputs = append([][2]string{{"Ticket", ticket}, {"Round", strconv.Itoa(round)}, {"Worktree", o.worktree(ticket)}, {"Run directory", o.runDir(ticket)}, {"Result file", file}}, inputs...)

	for {
		result, reason := o.attempt(ctx, ticket, st, file, inputs, want)
		if ctx.Err() != nil {
			return stageResult{}, ctx.Err()
		}
		if reason == "" {
			return result, nil
		}
		ts := o.ticket(ticket)
		if ts.Retried {
			return stageResult{}, fmt.Errorf("%w: %s %s again after a retry", errParked, st.Name, reason)
		}
		pane := ts.Panes[st.Name]
		o.consume("retry-" + ticket) // a command sent before this Wake is not an answer to it
		o.consume("park-" + ticket)
		o.report("WAKE %s %s %s at %s", ticket, st.Name, reason, o.locate(pane))
		result, command := o.hold(ctx, ticket, pane, file, want)
		switch command {
		case "retry":
			o.update(ticket, func(ts *TicketState) { ts.Retried = true })
			o.report("%s %s retrying with a fresh session", ticket, st.Name)
		case "park":
			return stageResult{}, fmt.Errorf("%w: %s %s", errParked, st.Name, reason)
		case "done":
			return result, nil
		default:
			return stageResult{}, ctx.Err()
		}
	}
}

// attempt runs the Stage once in a fresh session and returns its accepted
// result, or the reason it cannot complete.
func (o *Orchestrator) attempt(ctx context.Context, ticket string, st Stage, file string, inputs [][2]string, want resultRequirements) (stageResult, string) {
	skill, err := os.ReadFile(filepath.Join(o.Repo, ".agents", "skills", st.Skill, "SKILL.md"))
	if err != nil {
		return stageResult{}, "has no Stage skill (run 'harness init'): " + err.Error()
	}
	os.Remove(file)
	if err := os.MkdirAll(filepath.Dir(file), 0o755); err != nil {
		return stageResult{}, err.Error()
	}
	pane, err := o.freshPane(ticket, st)
	if err != nil {
		return stageResult{}, "got no pane: " + err.Error()
	}
	deadline := time.Now().Add(st.Timeout)

	agentArgs := []string{"--permission-mode", "auto", "--add-dir", o.runDir(ticket)}
	if st.Kind == "codex" {
		// The pane's cwd is the run directory, so the sandbox lets Codex write
		// its result file there and nothing in the worktree.
		agentArgs = []string{"--sandbox", "workspace-write"}
	}
	start := append([]string{"agent", "start", agentName(ticket, st.Name), "--kind", st.Kind, "--pane", pane, "--"}, agentArgs...)
	_, startErr := o.herdr(start...)
	if startErr != nil && !strings.Contains(startErr.Error(), "agent_not_ready") {
		return stageResult{}, "session did not start: " + startErr.Error()
	}
	o.report("%s %s started -> %s", ticket, st.Name, o.locate(pane))
	if startErr != nil { // blocked at startup: nothing can be prompted yet
		if reason := o.waitUnblocked(ctx, ticket, st, pane, deadline); reason != "" {
			return stageResult{}, reason
		}
	}

	_, waitErr := o.herdr("agent", "prompt", pane, stagePrompt(string(skill), inputs), "--wait", "--timeout", remaining(deadline))
	for {
		if ctx.Err() != nil {
			return stageResult{}, "stopped"
		}
		status, alive := o.agentStatus(pane)
		switch {
		case !alive:
			return stageResult{}, "pane died"
		case status == "blocked":
			if reason := o.waitUnblocked(ctx, ticket, st, pane, deadline); reason != "" {
				return stageResult{}, reason
			}
		case status == "idle" || status == "done":
			return readStageResult(file, want)
		case time.Now().After(deadline) || (waitErr != nil && strings.Contains(waitErr.Error(), "timeout")):
			return stageResult{}, fmt.Sprintf("timed out after %s", st.Timeout)
		}
		if waitErr != nil && !o.sleep(ctx) { // herdr refused to wait: do not spin
			return stageResult{}, "stopped"
		}
		_, waitErr = o.herdr("agent", "wait", pane, "--timeout", remaining(deadline))
	}
}

func remaining(deadline time.Time) string {
	return strconv.FormatInt(max(time.Until(deadline).Milliseconds(), 1), 10)
}

// waitUnblocked wakes the Main session about a blocked session, which only the
// user may answer, and waits for the session to move on.
func (o *Orchestrator) waitUnblocked(ctx context.Context, ticket string, st Stage, pane string, deadline time.Time) string {
	o.report("WAKE %s %s blocked at %s", ticket, st.Name, o.locate(pane))
	for {
		status, alive := o.agentStatus(pane)
		switch {
		case !alive:
			return "pane died"
		case status != "blocked":
			return ""
		case time.Now().After(deadline):
			return fmt.Sprintf("timed out after %s", st.Timeout)
		}
		if !o.sleep(ctx) {
			return "stopped"
		}
	}
}

// hold keeps a woken Ticket waiting, leaving every other Ticket running, until
// a control command arrives or the Main session's nudge produces a done result.
func (o *Orchestrator) hold(ctx context.Context, ticket, pane, file string, want resultRequirements) (stageResult, string) {
	for {
		if o.consume("retry-" + ticket) {
			return stageResult{}, "retry"
		}
		if o.consume("park-" + ticket) {
			return stageResult{}, "park"
		}
		if result, reason := readStageResult(file, want); reason == "" {
			if status, alive := o.agentStatus(pane); !alive || status == "idle" || status == "done" {
				return result, "done"
			}
		}
		if !o.sleep(ctx) {
			return stageResult{}, "stopped"
		}
	}
}

// sleep is one tick of every polling loop, and so also the moment undelivered
// lines are tried again.
func (o *Orchestrator) sleep(ctx context.Context) bool {
	o.flush()
	select {
	case <-ctx.Done():
		return false
	case <-time.After(o.Tick):
		return true
	}
}

// freshPane gives the Stage an empty shell pane in the Ticket tab, replacing
// the pane of an earlier session of the same Stage (a previous Round, a retry,
// a resumed run) so every session starts fresh.
func (o *Orchestrator) freshPane(ticket string, st Stage) (string, error) {
	ts := o.ticket(ticket)
	if old := ts.Panes[st.Name]; old != "" {
		o.herdr("pane", "close", old) // already gone is fine
	}
	cwd := o.worktree(ticket)
	if st.Kind == "codex" {
		cwd = o.runDir(ticket)
	}
	placement := []string{"--cwd", cwd, "--no-focus"}
	if st.Name == "debate" && o.APIKey != "" {
		placement = append(placement, "--env", "TYPESAFE_API_KEY="+o.APIKey)
	}

	var inTab []string
	if ts.Tab != "" {
		if panes, err := o.herdr("pane", "list", "--workspace", o.Workspace); err == nil {
			for _, p := range panes.Result.Panes {
				if p.TabID == ts.Tab {
					inTab = append(inTab, p.PaneID)
				}
			}
		}
	}
	tab, pane := ts.Tab, ""
	if len(inTab) == 0 {
		reply, err := o.herdr(append([]string{"tab", "create", "--workspace", o.Workspace, "--label", ticket}, placement...)...)
		if err != nil {
			return "", err
		}
		tab, pane = reply.Result.Tab.TabID, reply.Result.RootPane.PaneID
	} else {
		direction := "right"
		if len(inTab)%2 == 0 {
			direction = "down"
		}
		reply, err := o.herdr(append([]string{"pane", "split", inTab[len(inTab)-1], "--direction", direction}, placement...)...)
		if err != nil {
			return "", err
		}
		pane = reply.Result.Pane.PaneID
	}
	o.update(ticket, func(ts *TicketState) {
		if ts.Panes == nil {
			ts.Panes = map[string]string{}
		}
		ts.Tab, ts.Panes[st.Name] = tab, pane
	})
	return pane, nil
}

// ControlFile is where a control command (stop, retry-<ticket>, park-<ticket>,
// address-<ticket>) is left for the running Orchestrator.
// ponytail: a polled directory; a socket if latency ever matters.
func ControlFile(repo, name string) string {
	return filepath.Join(repo, ".harness", "control", name)
}

// consume reports whether a control command was waiting, and removes it.
func (o *Orchestrator) consume(name string) bool {
	return os.Remove(ControlFile(o.Repo, name)) == nil
}
