package orchestrator

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"strings"
	"testing"
	"time"
)

// runEpic runs the Orchestrator until it returns, as 'harness start <epic>' does.
func runEpic(t *testing.T, o *Orchestrator) {
	t.Helper()
	finished := make(chan error, 1)
	go func() { finished <- o.Run(context.Background(), "hx") }()
	select {
	case err := <-finished:
		if err != nil {
			t.Fatalf("Run: %v", err)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("the Orchestrator never finished the Epic")
	}
	o.inFlight.Wait()
}

func TestSchedulerRunsEveryReadyTicketButNeverMoreThanMaxAtOnce(t *testing.T) {
	var tickets []*bdTicket
	for i := 1; i <= 5; i++ {
		tickets = append(tickets, &bdTicket{ID: fmt.Sprintf("hx-%d", i)})
	}
	w, o := newWorld(t, tickets...)
	w.merged = true
	w.session = func(p prompt) (string, string) {
		time.Sleep(2 * time.Millisecond) // long enough for Tickets to overlap
		return succeed(p)
	}

	runEpic(t, o)

	if got := len(w.Called("bd worktree create")); got != 5 {
		t.Errorf("Tickets run = %d, want 5", got)
	}
	if w.peak != 3 {
		t.Errorf("most Tickets in the Pipeline at once = %d, want exactly 3", w.peak)
	}
	w.awaitLine("epic hx done")
}

func TestBlockedTicketStartsOnlyAfterItsDependencyIsMergedAndClosed(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"}, &bdTicket{ID: "hx-2", deps: []string{"hx-1"}})
	w.merged = true

	runEpic(t, o)

	order := strings.Join(w.Calls(), "\n")
	closed := strings.Index(order, "bd close hx-1")
	started := strings.Index(order, "bd worktree create "+o.worktree("hx-2"))
	if closed < 0 || started < 0 || started < closed {
		t.Errorf("hx-2 must start after hx-1 is closed (close at %d, start at %d)", closed, started)
	}
	for _, want := range []string{"bd worktree remove " + o.worktree("hx-1"), "git branch -D hx-1"} {
		if len(w.Called(want)) != 1 {
			t.Errorf("merge cleanup missing %q", want)
		}
	}
	if got := o.ticket("hx-1").Status; got != statusMerged {
		t.Errorf("hx-1 status = %q", got)
	}
}

func TestOneRunPerTargetRepoButAStaleLockDoesNotBlockARestart(t *testing.T) {
	repo := t.TempDir()
	release, err := AcquireLock(repo) // a live Orchestrator: this process
	if err != nil {
		t.Fatal(err)
	}
	if _, err := AcquireLock(repo); err == nil || !strings.Contains(err.Error(), "already running") {
		t.Errorf("second lock = %v, want a refusal", err)
	}
	if LockHolder(repo) != os.Getpid() {
		t.Errorf("LockHolder = %d, want this process", LockHolder(repo))
	}

	release()
	os.WriteFile(lockPath(repo), []byte("999999"), 0o644) // a killed Orchestrator's stale lock
	if _, err := AcquireLock(repo); err != nil {
		t.Errorf("a stale lock must not block a restart: %v", err)
	}
}

func TestClosedPRParksTheTicketAndConflictIsReportedExactlyOnce(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"}, &bdTicket{ID: "hx-2"})
	w.prs["https://example.test/pr/hx-1"] = `{"state":"CLOSED","mergeable":"UNKNOWN"}`
	w.prs["https://example.test/pr/hx-2"] = `{"state":"OPEN","mergeable":"CONFLICTING"}`
	finished := make(chan error, 1)
	go func() { finished <- o.Run(context.Background(), "hx") }()

	w.awaitLine("hx-1 parked: PR closed without merging")
	w.awaitLine("hx-2 pr conflicts with main")
	time.Sleep(30 * time.Millisecond) // many more polls
	w.control("stop")
	<-finished
	o.inFlight.Wait()

	conflicts := 0
	for _, line := range w.mainLines() {
		if strings.Contains(line, "conflicts with main") {
			conflicts++
		}
	}
	if conflicts != 1 {
		t.Errorf("conflict reported %d times, want once", conflicts)
	}
	if got := o.ticket("hx-1").Status; got != statusParked {
		t.Errorf("hx-1 status = %q, want parked", got)
	}
	if len(w.Called("bd close")) != 0 {
		t.Errorf("no Ticket may be closed without a merge")
	}
}

func TestKilledRunResumesAtTheRightStageWithoutRedoingFinishedOnes(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.merged = true
	// The first Orchestrator is stopped while Review round 1 is mid-flight.
	ctx, kill := context.WithCancel(context.Background())
	w.session = func(p prompt) (string, string) {
		if p.stage == "review" {
			kill()
			return "", "working"
		}
		return succeed(p)
	}
	o.Run(ctx, "hx")
	o.inFlight.Wait()
	if ts := o.ticket("hx-1"); ts.Status != statusRunning || ts.Stage != "review" || ts.Round != 1 {
		t.Fatalf("state when killed = %+v", ts)
	}

	var status bytes.Buffer
	PrintStatus(&status, w.repo)
	if !strings.Contains(status.String(), "hx-1") || !strings.Contains(status.String(), "review round 1") {
		t.Errorf("status output:\n%s", status.String())
	}

	// A new process: fresh Orchestrator, state loaded from the file.
	w.session = succeed
	state, err := loadState(w.repo)
	if err != nil {
		t.Fatal(err)
	}
	resumed := &Orchestrator{Config: o.Config, state: state}
	before := len(w.Called("herdr agent start"))
	runEpic(t, resumed)

	var stages []string
	for _, call := range w.Called("herdr agent start")[before:] {
		stages = append(stages, strings.Fields(call)[3])
	}
	if got := fmt.Sprint(stages); got != "[h-hx-1-review h-hx-1-debate h-hx-1-fix]" {
		t.Errorf("sessions after resume = %s; Implement was done and Review starts again from its beginning", got)
	}
	if got := len(w.Called("bd worktree create")); got != 1 {
		t.Errorf("worktree created %d times", got)
	}
}

func TestStopExitsWithStateSavedAndLeavesPanesAlone(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.session = func(prompt) (string, string) { return "", "idle" } // hx-1 holds on a Wake
	finished := make(chan error, 1)
	go func() { finished <- o.Run(context.Background(), "hx") }()
	w.awaitLine("WAKE hx-1")

	w.control("stop")
	if err := <-finished; err != nil {
		t.Fatalf("Run after stop: %v", err)
	}
	o.inFlight.Wait()

	if len(w.Called("herdr pane close"))+len(w.Called("herdr tab close")) != 0 {
		t.Errorf("stop must leave live panes alone")
	}
	saved, _ := loadState(w.repo)
	if ts := saved.Tickets["hx-1"]; ts == nil || ts.Status != statusRunning || ts.Stage != "implement" {
		t.Errorf("saved state = %+v", ts)
	}
}

func TestRetryUnparksAParkedTicket(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.merged = true
	w.session = func(prompt) (string, string) { return "", "idle" }
	finished := make(chan error, 1)
	go func() { finished <- o.Run(context.Background(), "hx") }()

	w.awaitLine("WAKE hx-1")
	w.control("park-hx-1")
	w.awaitLine("hx-1 parked")
	w.mu.Lock()
	w.session = succeed
	w.mu.Unlock()
	w.control("retry-hx-1")
	<-finished
	o.inFlight.Wait()
	if got := o.ticket("hx-1").Status; got != statusMerged {
		t.Errorf("status = %q, want the Ticket to run to a merge after the retry", got)
	}
}

func TestAddressPromptCarriesThePRFeedbackAndConflictState(t *testing.T) {
	gh := `{"mergeable":"CONFLICTING","reviews":[{"body":"rename this"}],"comments":[]}`
	text := stagePrompt("---\nname: stage-address\n---\nAddress the PR.", addressInputs("https://example.test/pr/9", gh))
	for _, want := range []string{"Address the PR.", "- PR: https://example.test/pr/9", "- Conflicts with main: yes", "rename this"} {
		if !strings.Contains(text, want) {
			t.Errorf("address prompt lacks %q:\n%s", want, text)
		}
	}
	if clean := addressInputs("u", `{"mergeable":"MERGEABLE"}`); clean[1][1] != "no" {
		t.Errorf("mergeable PR reported as conflicting")
	}
}

func TestAddressCommandStartsAFreshSessionInTheKeptWorktree(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.prs["https://example.test/pr/hx-1"] = `{"state":"OPEN","mergeable":"CONFLICTING","reviews":[{"body":"rename this"}]}`
	var addressPrompt prompt
	w.session = func(p prompt) (string, string) {
		if p.stage == "address" {
			addressPrompt = p
		}
		return succeed(p)
	}
	finished := make(chan error, 1)
	go func() { finished <- o.Run(context.Background(), "hx") }()
	w.awaitLine("hx-1 pr open")

	w.control("address-hx-1")
	w.awaitLine("hx-1 address done")
	w.control("stop")
	<-finished
	o.inFlight.Wait()

	if !strings.Contains(addressPrompt.text, "rename this") || !strings.Contains(addressPrompt.text, "Conflicts with main: yes") {
		t.Errorf("address prompt:\n%s", addressPrompt.text)
	}
	tabs := w.Called("herdr tab create")
	if len(tabs) != 2 || !strings.Contains(tabs[1], "--cwd "+o.worktree("hx-1")) {
		t.Errorf("address must reopen a Ticket tab in the kept worktree: %q", tabs)
	}
	if got := o.ticket("hx-1").Status; got != statusPROpen {
		t.Errorf("status after address = %q, want it still %q", got, statusPROpen)
	}
}

func TestEpicWithoutTicketsIsAnErrorNotADoneEpic(t *testing.T) {
	w, o := newWorld(t) // a mistyped Epic id: bd lists no children
	if err := o.Run(context.Background(), "hx-typo"); err == nil || !strings.Contains(err.Error(), "no Tickets") {
		t.Errorf("Run = %v, want an error naming the empty Epic", err)
	}
	for _, line := range w.mainLines() {
		if strings.Contains(line, "done") {
			t.Errorf("reported %q for an Epic with no Tickets", line)
		}
	}
}
