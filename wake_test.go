package main

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"
)

// failImplementOnce makes the first Implement session end as the trigger
// dictates; every later session succeeds.
func failImplementOnce(w *world, fail func(p prompt) (string, string)) {
	failed := false
	w.session = func(p prompt) (string, string) {
		if p.stage == "implement" && !failed {
			failed = true
			return fail(p)
		}
		return succeed(p)
	}
}

func TestEachWakeTriggerSendsAWakeLineAndRetryRestartsTheStage(t *testing.T) {
	triggers := []struct {
		name, reason string
		arrange      func(w *world)
	}{
		{"failed", "reported STATUS: failed", func(w *world) {
			failImplementOnce(w, func(prompt) (string, string) { return "STATUS: failed\ntests red\n", "idle" })
		}},
		{"idle without result", "went idle without a done result", func(w *world) {
			failImplementOnce(w, func(prompt) (string, string) { return "", "idle" })
		}},
		{"timeout", "timed out", func(w *world) {
			failImplementOnce(w, func(prompt) (string, string) {
				w.mu.Lock()
				w.waitErr = errors.New(`{"error":{"code":"timeout"}}`)
				w.mu.Unlock()
				return "", "working"
			})
		}},
		{"pane died", "pane died", func(w *world) {
			failImplementOnce(w, func(p prompt) (string, string) {
				w.mu.Lock()
				delete(w.agents, p.pane)
				w.mu.Unlock()
				return "", "idle"
			})
		}},
	}
	for _, trigger := range triggers {
		t.Run(trigger.name, func(t *testing.T) {
			w, o := newWorld(t, &bdTicket{ID: "hx-1"})
			trigger.arrange(w)
			finished := make(chan struct{})
			go func() { o.runTicket(context.Background(), "hx-1"); close(finished) }()

			wake := w.awaitLine("WAKE hx-1 implement " + trigger.reason)
			if !strings.HasSuffix(wake, " at 1-1") {
				t.Errorf("WAKE line does not end with the location: %q", wake)
			}
			select {
			case <-finished:
				t.Fatal("the Ticket did not wait after its WAKE")
			case <-time.After(20 * time.Millisecond):
			}

			w.mu.Lock()
			w.waitErr = nil
			w.mu.Unlock()
			w.control("retry-hx-1")
			<-finished
			if got := o.ticket("hx-1").Status; got != statusPROpen {
				t.Errorf("status after retry = %q, want %q", got, statusPROpen)
			}
			if starts := w.called("herdr agent start h-hx-1-implement"); len(starts) != 2 {
				t.Errorf("implement sessions = %d, want a second, fresh one", len(starts))
			}
		})
	}
}

func TestBlockedSessionWakesMainThenContinuesWhenTheUserAnswers(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	failImplementOnce(w, func(prompt) (string, string) { return "STATUS: done\n", "blocked" })
	finished := make(chan struct{})
	go func() { o.runTicket(context.Background(), "hx-1"); close(finished) }()

	w.awaitLine("WAKE hx-1 implement blocked at 1-1")
	w.mu.Lock()
	for pane := range w.agents {
		w.agents[pane] = "idle" // the user answered the permission prompt
	}
	w.mu.Unlock()
	<-finished
	if got := o.ticket("hx-1").Status; got != statusPROpen {
		t.Errorf("status = %q, want %q", got, statusPROpen)
	}
	if starts := w.called("herdr agent start h-hx-1-implement"); len(starts) != 1 {
		t.Errorf("a blocked session must not be restarted; sessions = %d", len(starts))
	}
}

func TestSecondFailureAfterRetryParksTheTicket(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.session = func(prompt) (string, string) { return "STATUS: failed\n", "idle" }
	finished := make(chan struct{})
	go func() { o.runTicket(context.Background(), "hx-1"); close(finished) }()

	w.awaitLine("WAKE hx-1 implement")
	w.control("retry-hx-1")
	<-finished

	if ts := o.ticket("hx-1"); ts.Status != statusParked || !strings.Contains(ts.Reason, "after a retry") {
		t.Errorf("state = %+v, want parked after the retry failed", ts)
	}
	w.awaitLine("hx-1 parked:")
	wakes := 0
	for _, line := range w.mainLines() {
		if strings.Contains(line, "WAKE") {
			wakes++
		}
	}
	if wakes != 1 {
		t.Errorf("WAKE lines = %d, want 1: the second failure parks instead", wakes)
	}
}

func TestParkCommandParksAWokenTicket(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	w.session = func(prompt) (string, string) { return "", "idle" }
	finished := make(chan struct{})
	go func() { o.runTicket(context.Background(), "hx-1"); close(finished) }()

	w.awaitLine("WAKE hx-1 implement")
	w.control("park-hx-1")
	<-finished
	if got := o.ticket("hx-1").Status; got != statusParked {
		t.Errorf("status = %q, want parked", got)
	}
}

func TestNudgedSessionThatThenWritesDoneAdvancesWithoutARetry(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	var file string
	failImplementOnce(w, func(p prompt) (string, string) { file = p.file; return "", "idle" })
	finished := make(chan struct{})
	go func() { o.runTicket(context.Background(), "hx-1"); close(finished) }()

	w.awaitLine("WAKE hx-1 implement went idle")
	writeFile(t, file, "STATUS: done\n") // the Main session's follow-up prompt worked
	<-finished
	if starts := w.called("herdr agent start h-hx-1-implement"); len(starts) != 1 {
		t.Errorf("implement sessions = %d, want 1", len(starts))
	}
	if got := o.ticket("hx-1").Status; got != statusPROpen {
		t.Errorf("status = %q", got)
	}
}
