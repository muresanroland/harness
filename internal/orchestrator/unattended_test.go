package orchestrator

import (
	"context"
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// What the first live run exposed: a Stage nobody is watching has to be
// stoppable, has to survive a launching pane that is not an agent, and must
// not start a session into a trust dialog it cannot answer.

// working is a session that starts and never finishes.
func working(prompt) (string, string) { return "", "working" }

// Each wait must observe stop, including waits before an agent is prompted.
func TestStopEndsASingleTicketRunInEveryWaitState(t *testing.T) {
	for _, phase := range []string{"trust", "shell startup", "working", "blocked", "wake"} {
		t.Run(phase, func(t *testing.T) {
			w, o := newWorld(t, &bdTicket{ID: "hx-1"})
			ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			w.session = working
			entered := make(chan struct{}, 1)
			releaseStart := make(chan struct{})
			waitLine := "hx-1 implement prompted"
			switch phase {
			case "trust":
				o.Home = t.TempDir()
				waitLine = "does not trust"
			case "shell startup":
				// Keep the startup deadline beyond the test's context, and do
				// not return busy until stop exists: it must reach startup's
				// sleep, not a later Wake hold after startup times out.
				o.Tick = time.Second
				handle := w.Fake.Handle
				w.Fake.Handle = func(dir string, argv []string) (string, error) {
					if strings.HasPrefix(strings.Join(argv, " "), "herdr agent start") {
						select {
						case entered <- struct{}{}:
						default:
						}
						select {
						case <-releaseStart:
						case <-ctx.Done():
						}
						return "", errors.New(`{"error":{"code":"agent_pane_busy"}}`)
					}
					return handle(dir, argv)
				}
			case "blocked":
				w.session = func(prompt) (string, string) { return "", "blocked" }
				waitLine = "WAKE hx-1 implement blocked"
			case "wake":
				w.session = func(prompt) (string, string) { return "", "idle" }
				waitLine = "WAKE hx-1 implement went idle"
			}
			finished := make(chan struct{})
			go func() { o.RunTicket(ctx, "hx-1"); close(finished) }()
			t.Cleanup(func() { cancel(); <-finished })
			if phase == "shell startup" {
				select {
				case <-entered:
				case <-ctx.Done():
					t.Fatal("never attempted to start an agent")
				}
			} else {
				w.awaitLine(waitLine)
			}
			w.control("stop")
			close(releaseStart)
			select {
			case <-finished:
			case <-time.After(time.Second):
				t.Fatal("stop did not end the run while waiting")
			}
			if !o.stopping() {
				t.Fatal("run ended without consuming stop")
			}
			saved, err := loadState(w.repo)
			if err != nil {
				t.Fatal(err)
			}
			if ts := saved.Tickets["hx-1"]; ts == nil || ts.Status != statusRunning || ts.Stage != "implement" {
				t.Errorf("stop did not preserve resumable state: %+v", ts)
			}
			if len(w.Called("herdr pane close"))+len(w.Called("herdr tab close")) != 0 {
				t.Error("stop closed a live pane")
			}
			if len(w.Called("herdr agent start h-hx-1-review")) != 0 {
				t.Error("a stopped stage advanced to Review")
			}
		})
	}
}

func TestControlFilesLeftByAnEarlierRunDoNotCommandThisOne(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.control("stop") // the stop that ended the run before this one

	if parked := o.RunTicket(context.Background(), "hx-1"); parked {
		t.Error("the Ticket parked")
	}
	if got := o.ticket("hx-1"); got.Status != statusPROpen {
		t.Errorf("a stale stop ended the run before it began: %+v", got)
	}
}

func TestEventsGoToTheLogAloneWhenTheLaunchingPaneHostsNoAgent(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.mu.Lock()
	w.mainNoAgent = true // started from a shell pane, not a Claude session
	w.mu.Unlock()

	o.runTicket(context.Background(), "hx-1")

	if got := o.ticket("hx-1"); got.Status != statusPROpen {
		t.Errorf("the Pipeline did not finish: %+v", got)
	}
	// One line is tried, finds nobody, and the rest go to the log: the run
	// must not spend every tick retrying a pane that will never take them.
	if got := w.Called("herdr agent prompt main"); len(got) != 1 {
		t.Errorf("tried to reach a pane with no agent %d times: %q", len(got), got)
	}
}

func TestStageWaitsUntilItsAgentTrustsTheDirectory(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	home := t.TempDir() // neither agent has been run anywhere yet
	o.Home = home
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go o.runTicket(ctx, "hx-1")

	w.awaitLine("does not trust")
	if got := w.Called("herdr agent start"); len(got) != 0 {
		t.Errorf("a session was started into a trust dialog: %q", got)
	}

	// The user opens Claude there once and accepts it.
	writeFile(t, filepath.Join(home, ".claude.json"),
		`{"projects": {"`+o.worktree("hx-1")+`": {"hasTrustDialogAccepted": true}}}`)
	w.awaitLine("hx-1 implement started")
}
