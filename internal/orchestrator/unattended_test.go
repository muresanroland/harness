package orchestrator

import (
	"context"
	"path/filepath"
	"testing"
	"time"
)

// What the first live run exposed: a Stage nobody is watching has to be
// stoppable, has to survive a launching pane that is not an agent, and must
// not start a session into a trust dialog it cannot answer.

// working is a session that starts and never finishes.
func working(prompt) (string, string) { return "", "working" }

func TestStopEndsASingleTicketRunToo(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.session = working
	ended := make(chan bool, 1)
	go func() { ended <- o.RunTicket(context.Background(), "hx-1") }()

	w.awaitLine("hx-1 implement prompted")
	w.control("stop")
	select {
	case <-ended:
	case <-time.After(5 * time.Second):
		t.Fatal("'harness stop' left a --ticket run going; only 'harness start <epic>' could be stopped")
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
