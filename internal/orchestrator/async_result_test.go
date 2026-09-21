package orchestrator

import (
	"context"
	"strings"
	"testing"
	"time"
)

// A valid file is only half of completion: the agent must finish too.
func TestAResultWrittenByAWorkingAgentDoesNotAdvanceTheStage(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	prompted := make(chan prompt, 1)
	polled := make(chan struct{}, 1)
	w.session = func(p prompt) (string, string) {
		if p.stage == "implement" {
			prompted <- p
			return "STATUS: done\n", "working"
		}
		return succeed(p)
	}
	handle := w.Fake.Handle
	w.Fake.Handle = func(dir string, argv []string) (string, error) {
		if strings.HasPrefix(strings.Join(argv, " "), "herdr agent wait") {
			select {
			case polled <- struct{}{}:
			default:
			}
		}
		return handle(dir, argv)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	finished := make(chan struct{})
	go func() { o.RunTicket(ctx, "hx-1"); close(finished) }()
	t.Cleanup(func() { cancel(); <-finished })
	var p prompt
	select {
	case p = <-prompted:
	case <-ctx.Done():
		t.Fatal("Implement was never prompted")
	}
	select {
	case <-polled:
	case <-finished:
		t.Fatal("a working agent's result completed the pipeline")
	case <-ctx.Done():
		t.Fatal("orchestrator never waited for the working agent")
	}
	if got := o.ticket("hx-1"); got.Stage != "implement" || got.Status != statusRunning {
		t.Fatalf("advanced while Implement was still working: %+v", got)
	}
	if len(w.Called("herdr agent start h-hx-1-review")) != 0 {
		t.Fatal("Review started before Implement finished")
	}
	w.mu.Lock()
	w.agents[p.pane] = "idle"
	w.mu.Unlock()
	select {
	case <-finished:
	case <-ctx.Done():
		t.Fatal("did not accept the result after the agent finished")
	}
	if got := o.ticket("hx-1"); got.Status != statusPROpen {
		t.Errorf("pipeline did not finish: %+v", got)
	}
}
