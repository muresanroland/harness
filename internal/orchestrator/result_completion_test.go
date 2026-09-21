package orchestrator

import (
	"context"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// Drive the Pipeline so these cases also verify that Review counts and the
// final-Fix requirement reach every completion path, and accepted content is
// carried back to the next Stage and Ticket state.
func TestResultAcceptanceAcrossCompletionPaths(t *testing.T) {
	const fixItem = "- [fix] (high) a.go:1 — fix me"
	const skipItem = "- [skip] (low) b.go:2 — leave me"
	cases := []struct {
		name, review, verdict, invalid, accepted, reason, pr string
		stage                                                Stage
	}{
		{
			name: "Verdict", stage: stageDebate,
			review:   "STATUS: done\n- (high) a.go:1 — fix me\n- (low) b.go:2 — leave me\n",
			invalid:  "STATUS: done\n" + fixItem + "\n",
			accepted: "STATUS: done\n" + fixItem + "\n" + skipItem + "\n",
			reason:   "Verdict settles 1 of the Review's 2 Findings",
			pr:       "https://example.test/pr/hx-1",
		},
		{
			name: "final Fix", stage: stageFix,
			review: "STATUS: done\n", verdict: "STATUS: done\n",
			invalid:  "STATUS: done\n",
			accepted: "STATUS: done\nPR: https://example.test/pr/accepted\n",
			reason:   "wrote a done result without a 'PR:' line",
			pr:       "https://example.test/pr/accepted",
		},
	}
	for _, c := range cases {
		for _, path := range []string{"live", "resume invalid", "resume accepted", "late"} {
			t.Run(c.name+"/"+path, func(t *testing.T) {
				w, o := newWorld(t, &bdTicket{ID: "hx-1"})
				dir := o.runDir("hx-1")
				writeFile(t, filepath.Join(dir, "implement.md"), "STATUS: done\n")
				writeFile(t, filepath.Join(dir, "review-1.md"), c.review)
				if c.verdict != "" {
					writeFile(t, filepath.Join(dir, "verdict-1.md"), c.verdict)
				}
				file := filepath.Join(dir, resultName(c.stage, 1))
				switch path {
				case "resume invalid":
					writeFile(t, file, c.invalid)
				case "resume accepted":
					writeFile(t, file, c.accepted)
				}
				attempts := 0
				var firstFix string
				w.session = func(p prompt) (string, string) {
					if p.stage == "fix" && p.round == 1 {
						firstFix = p.text
					}
					if p.file != file {
						return succeed(p)
					}
					attempts++
					if path == "live" && attempts == 1 {
						return c.invalid, "idle"
					}
					if path == "late" {
						return "", "idle"
					}
					return c.accepted, "idle"
				}

				ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
				finished := make(chan struct{})
				go func() { o.RunTicket(ctx, "hx-1"); close(finished) }()
				t.Cleanup(func() { cancel(); <-finished })
				wantAttempts := 1
				switch path {
				case "live":
					w.awaitLine("WAKE hx-1 " + c.stage.Name + " " + c.reason)
					w.control("retry-hx-1")
					wantAttempts = 2
				case "resume accepted":
					wantAttempts = 0
				case "late":
					w.awaitLine("WAKE hx-1 " + c.stage.Name + " went idle without a done result")
					writeFile(t, file, c.invalid)
					select {
					case <-finished:
						t.Fatal("an invalid late result advanced the Pipeline")
					case <-time.After(20 * time.Millisecond):
					}
					writeFile(t, file, c.accepted)
				}
				<-finished
				if ts := o.ticket("hx-1"); ts.Status != statusPROpen || ts.PR != c.pr {
					t.Errorf("Ticket state = %+v, want PR open at %s", ts, c.pr)
				}
				if attempts != wantAttempts {
					t.Errorf("sessions = %d, want %d", attempts, wantAttempts)
				}
				if c.stage == stageDebate && (!strings.Contains(firstFix, fixItem) || strings.Contains(firstFix, skipItem)) {
					t.Errorf("Fix did not receive only accepted fix items:\n%s", firstFix)
				}
				if len(w.Called("herdr agent start h-hx-1-implement")) != 0 {
					t.Error("reran completed Implement Stage")
				}
			})
		}
	}
}
