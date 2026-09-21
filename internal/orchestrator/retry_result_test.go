package orchestrator

import (
	"context"
	"strings"
	"testing"
	"time"
)

// An old session can finish while its pane is being closed. That result
// belongs to the old attempt, even if the replacement session goes idle.
func TestRetryDiscardsAResultWrittenWhileClosingTheOldPane(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	var old prompt
	attempts := 0
	w.session = func(p prompt) (string, string) {
		if p.stage != "implement" {
			return succeed(p)
		}
		attempts++
		if attempts == 1 {
			old = p
			return "STATUS: failed\n", "idle"
		}
		return "", "idle" // the replacement never produces its own result
	}
	handle := w.Fake.Handle
	w.Fake.Handle = func(dir string, argv []string) (string, error) {
		if strings.Join(argv, " ") == "herdr pane close "+old.pane {
			writeFile(t, old.file, "STATUS: done\n")
		}
		return handle(dir, argv)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	finished := make(chan struct{})
	go func() { o.RunTicket(ctx, "hx-1"); close(finished) }()
	t.Cleanup(func() { cancel(); <-finished })
	w.awaitLine("WAKE hx-1 implement reported STATUS: failed")
	w.control("retry-hx-1")
	select {
	case <-finished:
	case <-ctx.Done():
		t.Fatal("retry did not finish")
	}
	if ts := o.ticket("hx-1"); ts.Status != statusParked || !strings.Contains(ts.Reason, "without a done result") {
		t.Errorf("a result from the old attempt completed its replacement: %+v", ts)
	}
	if attempts != 2 {
		t.Errorf("Implement attempts = %d, want 2", attempts)
	}
	if len(w.Called("herdr agent start h-hx-1-review")) != 0 {
		t.Error("Review started even though the replacement wrote no result")
	}
}
