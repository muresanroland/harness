package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"syscall"
)

const (
	statusRunning = "running"
	statusParked  = "parked"
	statusPROpen  = "pr-open"
	statusMerged  = "merged"
)

// TicketState is what the Orchestrator knows about one Ticket.
type TicketState struct {
	Status   string            `json:"status"`
	Stage    string            `json:"stage"`
	Round    int               `json:"round"`
	Tab      string            `json:"tab,omitempty"`
	Panes    map[string]string `json:"panes,omitempty"` // Stage name -> pane id
	PR       string            `json:"pr,omitempty"`
	Reason   string            `json:"reason,omitempty"` // why it is Parked
	Retried  bool              `json:"retried,omitempty"`
	Conflict bool              `json:"conflict_reported,omitempty"`
}

type State struct {
	Epic    string                  `json:"epic,omitempty"`
	Tickets map[string]*TicketState `json:"tickets"`
}

func statePath(repo string) string { return filepath.Join(repo, ".harness", "state.json") }

func loadState(repo string) (*State, error) {
	state := &State{Tickets: map[string]*TicketState{}}
	raw, err := os.ReadFile(statePath(repo))
	if errors.Is(err, fs.ErrNotExist) {
		return state, nil
	}
	if err != nil {
		return nil, err
	}
	if err := json.Unmarshal(raw, state); err != nil {
		return nil, fmt.Errorf("%s: %w", statePath(repo), err)
	}
	if state.Tickets == nil {
		state.Tickets = map[string]*TicketState{}
	}
	return state, nil
}

// save writes the state file atomically: a reader sees the old or the new
// state, never half of one.
func (s *State) save(repo string) error {
	raw, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(statePath(repo)), 0o755); err != nil {
		return err
	}
	tmp := statePath(repo) + ".tmp"
	if err := os.WriteFile(tmp, raw, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, statePath(repo))
}

func printStatus(out io.Writer, repo string) int {
	state, err := loadState(repo)
	if err != nil {
		fmt.Fprintln(out, "status:", err)
		return 1
	}
	if pid := lockHolder(repo); pid != 0 {
		fmt.Fprintf(out, "Orchestrator running (pid %d)", pid)
	} else {
		fmt.Fprint(out, "Orchestrator not running")
	}
	if state.Epic != "" {
		fmt.Fprintf(out, ", Epic %s", state.Epic)
	}
	fmt.Fprintln(out)
	ids := make([]string, 0, len(state.Tickets))
	for id := range state.Tickets {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	for _, id := range ids {
		ts := state.Tickets[id]
		line := fmt.Sprintf("%-20s %-8s %s", id, ts.Status, ts.Stage)
		if ts.Round > 0 {
			line += fmt.Sprintf(" round %d", ts.Round)
		}
		for _, extra := range []string{ts.PR, ts.Reason} {
			if extra != "" {
				line += "  " + extra
			}
		}
		fmt.Fprintln(out, line)
	}
	return 0
}

func lockPath(repo string) string { return filepath.Join(repo, ".harness", "lock") }

// lockHolder returns the pid of the live Orchestrator holding this Target
// repo's lock, or 0.
func lockHolder(repo string) int {
	raw, err := os.ReadFile(lockPath(repo))
	if err != nil {
		return 0
	}
	pid, err := strconv.Atoi(strings.TrimSpace(string(raw)))
	if err != nil || pid <= 0 {
		return 0
	}
	if proc, err := os.FindProcess(pid); err != nil || proc.Signal(syscall.Signal(0)) != nil {
		return 0 // stale: that Orchestrator was killed
	}
	return pid
}

// acquireLock enforces one run per Target repo.
func acquireLock(repo string) (release func(), err error) {
	if pid := lockHolder(repo); pid != 0 {
		return nil, fmt.Errorf("an Orchestrator is already running in this repo (pid %d); 'harness stop' ends it", pid)
	}
	if err := os.MkdirAll(filepath.Dir(lockPath(repo)), 0o755); err != nil {
		return nil, err
	}
	// ponytail: check-then-write, two starts in the same millisecond could both win; O_EXCL plus stale takeover if that ever happens.
	if err := os.WriteFile(lockPath(repo), []byte(strconv.Itoa(os.Getpid())), 0o644); err != nil {
		return nil, err
	}
	return func() { os.Remove(lockPath(repo)) }, nil
}
