package orchestrator

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"
)

func rects(sizes ...[3]int) []paneRect {
	var panes []paneRect
	for i, s := range sizes {
		var p paneRect
		p.PaneID, p.Rect.Width, p.Rect.Height = string(rune('a'+i)), s[1], s[2]
		panes = append(panes, p)
	}
	return panes
}

func TestANewStagePaneComesOutOfTheRoomiestPaneAlongItsLongerSide(t *testing.T) {
	// {_, width, height} in cells; a cell is twice as tall as it is wide, so
	// 200x100 is a square on screen and 100x100 is a tall sliver.
	for _, c := range []struct {
		name            string
		panes           []paneRect
		pane, direction string
	}{
		{"a wide pane is cut down the middle", rects([3]int{0, 400, 100}), "a", "right"},
		{"a tall pane is cut across", rects([3]int{0, 100, 100}), "a", "down"},
		{"a square pane is cut across", rects([3]int{0, 200, 100}), "a", "down"},
		{"the roomiest pane is the one cut", rects([3]int{0, 100, 50}, [3]int{0, 400, 100}, [3]int{0, 100, 100}), "b", "right"},
		{"no layout, no answer", nil, "", ""},
	} {
		t.Run(c.name, func(t *testing.T) {
			pane, direction := splitTarget(c.panes)
			if pane != c.pane || direction != c.direction {
				t.Errorf("splitTarget = %q %q, want %q %q", pane, direction, c.pane, c.direction)
			}
		})
	}
}

func TestAStagePaneIsSplitTheWayTheTabIsShaped(t *testing.T) {
	for _, c := range []struct {
		name, direction string
		rect            [2]int
	}{
		{"a tall window is cut across", "down", [2]int{100, 400}},
		{"a wide window is cut down the middle", "right", [2]int{400, 50}},
	} {
		t.Run(c.name, func(t *testing.T) {
			w, o := newWorld(t, &bdTicket{ID: "hx-1"})
			w.rect = c.rect
			o.runTicket(context.Background(), "hx-1")

			split := w.Called("herdr pane split")
			if len(split) == 0 {
				t.Fatal("no pane was split")
			}
			if !strings.Contains(split[0], "--direction "+c.direction) {
				t.Errorf("a %v pane was cut the other way: %q", c.rect, split[0])
			}
		})
	}
}

func TestAPaneThatHasNotGotItsShellYetIsWaitedFor(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	// herdr refuses until the pane it just created has a shell.
	w.failOnce("herdr agent start", errors.New(`{"error":{"code":"agent_pane_busy","message":"agent target w1:p2 is not an available shell"}}`))

	o.runTicket(context.Background(), "hx-1")

	if got := o.ticket("hx-1"); got.Status != statusPROpen {
		t.Errorf("a pane that was a moment from ready ended the Stage: %+v", got)
	}
}

func TestASessionStillPickingUpItsPromptIsNotAFinishedStage(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	o.Tick = 10 * time.Millisecond // a 30ms grace for the session to get going
	w.session = func(p prompt) (string, string) {
		// Reads idle at once, as a real session does, and writes its result
		// a moment later.
		go func() {
			time.Sleep(5 * time.Millisecond)
			writeFile(t, p.file, "STATUS: done\nPR: https://example.test/pr\n")
		}()
		return "", "idle"
	}

	o.runTicket(context.Background(), "hx-1")

	for _, line := range w.mainLines() {
		if strings.Contains(line, "WAKE") {
			t.Errorf("a session that was still starting was woken on: %q", line)
		}
	}
	if got := o.ticket("hx-1"); got.Status != statusPROpen {
		t.Errorf("the Pipeline did not finish: %+v", got)
	}
}
