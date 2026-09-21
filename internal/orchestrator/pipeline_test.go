package orchestrator

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func stagesRun(w *world) []string {
	var stages []string
	for _, call := range w.Called("herdr agent start") {
		name := strings.Fields(call)[3]
		stages = append(stages, name[strings.LastIndex(name, "-")+1:])
	}
	return stages
}

func TestImplementStageRunsInATicketTabAndReportsToMain(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-12"})
	w.tabs = []string{"w1:main-tab"}
	w.panes = []paneInfo{{PaneID: "main", TabID: "w1:main-tab"}}

	o.runTicket(context.Background(), "hx-12")

	if got := w.Called("bd worktree create"); len(got) != 1 || !strings.HasSuffix(got[0], ".harness/worktrees/hx-12 --branch hx-12") {
		t.Errorf("worktree calls = %q", got)
	}
	if got := w.Called("bd update hx-12"); len(got) != 1 || !strings.Contains(got[0], "in_progress") {
		t.Errorf("ticket not marked in_progress: %q", got)
	}
	tab := w.Called("herdr tab create")
	if len(tab) != 1 || !strings.Contains(tab[0], "--label hx-12") || !strings.Contains(tab[0], "--no-focus") {
		t.Errorf("tab create = %q", tab)
	}
	start := w.Called("herdr agent start")[0]
	for _, want := range []string{"--kind claude", "--permission-mode auto", "--add-dir " + o.runDir("hx-12")} {
		if !strings.Contains(start, want) {
			t.Errorf("agent start lacks %q: %s", want, start)
		}
	}
	// The Main session is told where the session is and that it finished.
	started := w.awaitLine("hx-12 implement started")
	if !strings.HasSuffix(started, "-> 2-1") {
		t.Errorf("started line does not locate the pane: %q", started)
	}
	if !strings.Contains(started, "claude in "+o.worktree("hx-12")) {
		t.Errorf("started line does not say what runs where: %q", started)
	}
	w.awaitLine("hx-12 implement done")
}

func TestCleanFirstVerdictOpensPRAfterOneRound(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})

	o.runTicket(context.Background(), "hx-1")

	if got := fmt.Sprint(stagesRun(w)); got != "[implement review debate fix]" {
		t.Errorf("stages = %s", got)
	}
	ts := o.ticket("hx-1")
	if ts.Status != statusPROpen || ts.PR != "https://example.test/pr/hx-1" {
		t.Errorf("state = %+v", ts)
	}
	w.awaitLine("hx-1 pr open after 1 round(s): https://example.test/pr/hx-1")
	if len(w.Called("herdr tab close")) != 1 {
		t.Errorf("Ticket tab not closed once the PR opened")
	}
}

func TestReviewRunsCodexInTheRunDirectoryAndDebateGetsTheAPIKey(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	o.runTicket(context.Background(), "hx-1")

	review := w.Called("herdr agent start")[1]
	if !strings.Contains(review, "--kind codex") || !strings.Contains(review, "--sandbox workspace-write") {
		t.Errorf("review session = %s", review)
	}
	splits := w.Called("herdr pane split")
	if len(splits) != 3 {
		t.Fatalf("pane splits = %q, want one per Stage after Implement", splits)
	}
	if !strings.Contains(splits[0], "--cwd "+o.runDir("hx-1")) {
		t.Errorf("Codex pane is not in the run directory: %s", splits[0])
	}
	if !strings.Contains(splits[1], "--env TYPESAFE_API_KEY=sk-test") || strings.Contains(splits[2], "TYPESAFE") {
		t.Errorf("TYPESAFE_API_KEY must reach the Debate pane only: %q", splits[1:])
	}
}

func TestTicketThatKeepsProducingFixItemsGetsPRAfterExactlyThreeRounds(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	var fixPrompts []prompt
	w.session = func(p prompt) (string, string) {
		switch p.stage {
		case "verdict":
			return "STATUS: done\n- [fix] (high) a.go:1 — still broken | reason: agreed | settled: consensus\n", "idle"
		case "fix":
			fixPrompts = append(fixPrompts, p)
		}
		return succeed(p)
	}

	o.runTicket(context.Background(), "hx-1")

	want := "[implement review debate fix review debate fix review debate fix]"
	if got := fmt.Sprint(stagesRun(w)); got != want {
		t.Errorf("stages = %s\nwant     %s", got, want)
	}
	if len(fixPrompts) != 3 || fixPrompts[0].openPR || fixPrompts[1].openPR || !fixPrompts[2].openPR {
		t.Fatalf("only the round 3 Fix session may open the PR: %+v", fixPrompts)
	}
	for round := 1; round <= 3; round++ {
		if file := fmt.Sprintf("verdict-%d.md", round); !strings.Contains(fixPrompts[2].text, file) {
			t.Errorf("last Fix prompt lacks %s for the PR's Verdict history", file)
		}
	}
	w.awaitLine("hx-1 pr open after 3 round(s)")
}

func TestOpenPRPrunesBuildScratchAndKeepsEvidence(t *testing.T) {
	w, o := newWorld(t, &bdTicket{ID: "hx-1"})
	// The Review Stage compiles the branch, and the run directory is the only
	// place its sandbox may write, so its build cache lands there.
	w.session = func(p prompt) (string, string) {
		if p.stage == "review" {
			dir := o.runDir(p.ticket)
			writeFile(t, filepath.Join(dir, ".review-cache", "ab", "obj-a"), "go object data")
			writeFile(t, filepath.Join(dir, "check-testharness"), "a compiled test binary")
			writeFile(t, filepath.Join(dir, "diff-1.patch"), "the diff it reviewed")
		}
		return succeed(p)
	}

	o.runTicket(context.Background(), "hx-1")

	w.awaitLine("hx-1 pr open after 1 round(s)")
	for _, gone := range []string{".review-cache", "check-testharness"} {
		if _, err := os.Stat(filepath.Join(o.runDir("hx-1"), gone)); !os.IsNotExist(err) {
			t.Errorf("%s still in the run directory after the PR opened: %v", gone, err)
		}
	}
	for _, kept := range []string{"implement.md", "review-1.md", "verdict-1.md", "fix-1.md", "diff-1.patch"} {
		if _, err := os.Stat(filepath.Join(o.runDir("hx-1"), kept)); err != nil {
			t.Errorf("evidence pruned: %v", err)
		}
	}
}
