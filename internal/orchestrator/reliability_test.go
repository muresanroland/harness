package orchestrator

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"
)

func TestMergedTicketIsRetriedUntilBdClosesIt(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"}, &bdTicket{ID: "hx-2", deps: []string{"hx-1"}})
	w.merged = true
	w.failOnce("bd close hx-1", errors.New("dolt: database is locked"))

	runEpic(t, o) // finishes only if hx-1 does get closed and hx-2 unblocks

	if got := len(w.Called("bd close hx-1")); got != 2 {
		t.Errorf("bd close hx-1 called %d times, want a retry after the failure", got)
	}
	merged := 0
	for _, line := range w.mainLines() {
		if strings.Contains(line, "hx-1 merged") {
			merged++
		}
	}
	if merged != 1 {
		t.Errorf("hx-1 reported merged %d times, want once, after it was really closed", merged)
	}
}

func TestMergeCleanupForcesPastBdSafetyChecksAndSaysWhatItCouldNotRemove(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.merged = true
	w.failOnce("bd worktree remove", errors.New("HEAD not contained in upstream")) // a squash merge
	w.failOnce("git branch -D hx-1", errors.New("branch is checked out"))

	runEpic(t, o)

	if got := w.Called("bd worktree remove"); len(got) != 1 || !strings.Contains(got[0], "--force") {
		t.Errorf("worktree removal = %q, want --force: the PR is merged", got)
	}
	line := w.awaitLine("hx-1 merged")
	if !strings.Contains(line, "could not") {
		t.Errorf("merge line claims a clean-up that failed: %q", line)
	}
}

func TestLinesMainCouldNotReceiveAreDeliveredLaterInOrder(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.mu.Lock()
	w.mainBlocked = true // the Main session sits at its own permission prompt
	w.mu.Unlock()
	w.session = func(prompt) (string, string) { return "", "idle" }
	finished := make(chan struct{})
	go func() { o.runTicket(context.Background(), "hx-1"); close(finished) }()

	// Lines stay in order, so the refused "started" line is what gets retried;
	// a third attempt means the Ticket is already holding on its queued WAKE.
	for len(w.Called("herdr agent prompt main [harness] hx-1 implement started")) < 3 {
		time.Sleep(time.Millisecond)
	}
	if got := w.mainLines(); len(got) != 0 {
		t.Fatalf("a blocked Main session received %q", got)
	}
	w.mu.Lock()
	w.mainBlocked = false
	w.mu.Unlock()
	w.awaitLine("WAKE hx-1 implement")
	w.control("park-hx-1")
	<-finished

	lines := w.mainLines()
	if len(lines) < 3 || !strings.Contains(lines[0], "started") || !strings.Contains(lines[1], "WAKE") {
		t.Errorf("lines arrived out of order or were lost: %q", lines)
	}
}

func TestVerdictThatDropsFindingsIsNotACleanVerdict(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.session = func(p prompt) (string, string) {
		switch p.stage {
		case "review":
			return "STATUS: done\n- (high) a.go:1 — nil map write\n- (low) b.go:2 — dead branch\n", "idle"
		case "verdict":
			return "STATUS: done\n* fix: a.go:1 (the Moderator ignored the format)\n", "idle"
		}
		return succeed(p)
	}
	finished := make(chan struct{})
	go func() { o.runTicket(context.Background(), "hx-1"); close(finished) }()

	w.awaitLine("WAKE hx-1 debate Verdict settles 0 of the Review's 2 Findings")
	w.control("park-hx-1")
	<-finished
	if len(w.Called("herdr agent start h-hx-1-fix")) != 0 {
		t.Errorf("a PR must not open on a Verdict that lost Findings")
	}
}

func TestFixSessionIsGivenOnlyTheFixItems(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	var fixText string
	w.session = func(p prompt) (string, string) {
		switch {
		case p.stage == "verdict" && p.round == 1:
			return "STATUS: done\n- [fix] (high) a.go:1 — nil map write | reason: agreed | settled: consensus\n- [skip] (low) b.go:2 — naming | reason: style | settled: consensus\n", "idle"
		case p.stage == "fix" && p.round == 1:
			fixText = p.text
		}
		return succeed(p)
	}
	o.runTicket(context.Background(), "hx-1")

	if !strings.Contains(fixText, "a.go:1 — nil map write") {
		t.Errorf("Fix prompt lacks the fix item:\n%s", fixText)
	}
	if strings.Contains(fixText, "b.go:2") || strings.Contains(fixText, "verdict-1.md") {
		t.Errorf("a Fix session that does not open the PR must see only fix items:\n%s", fixText)
	}
}
