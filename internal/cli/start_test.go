package cli

import (
	"bytes"
	"fmt"
	"os"
	"strings"
	"testing"

	"harness/internal/orchestrator"
	"harness/internal/runner/runnertest"
)

func TestStartArgumentsParseInAnyOrder(t *testing.T) {
	for _, args := range [][]string{
		{"hx", "--max", "2", "--foreground"},
		{"--max", "2", "hx", "--foreground"},
		{"--foreground", "--max", "2", "hx"},
		{"--foreground", "hx", "--max=2"},
	} {
		got, err := parseStart(args, &strings.Builder{})
		if err != nil || got.epic != "hx" || got.max != 2 || !got.foreground {
			t.Errorf("parseStart(%q) = %+v, %v", args, got, err)
		}
	}
	if got, err := parseStart([]string{"--ticket", "hx-1"}, &strings.Builder{}); err != nil || got.ticket != "hx-1" || got.max != 3 {
		t.Errorf("--ticket: %+v, %v", got, err)
	}
	for _, bad := range [][]string{{}, {"hx", "--ticket", "hx-1"}, {"hx", "extra"}, {"hx", "--max", "0"}} {
		if _, err := parseStart(bad, &strings.Builder{}); err == nil {
			t.Errorf("parseStart(%q) accepted", bad)
		}
	}
	// The detached child must see --foreground whatever the user's argument order.
	if got := fmt.Sprint(foregroundArgs([]string{"start", "--max", "2", "hx"})); got != "[start --foreground --max 2 hx]" {
		t.Errorf("foregroundArgs = %s", got)
	}
}

func TestSecondStartInTheSameRepoRefuses(t *testing.T) {
	repo := preparedRepo(t)
	release, err := orchestrator.AcquireLock(repo) // a live Orchestrator: this process
	if err != nil {
		t.Fatal(err)
	}
	defer release()
	var out bytes.Buffer
	code := Run([]string{"start", "hx", "--foreground"}, &out, repo, (&runnertest.Fake{Handle: okTools}).Run, herdrEnv)
	if code == 0 || !strings.Contains(out.String(), "already running") {
		t.Errorf("second start: exit %d, output %q", code, out.String())
	}
}

func TestControlCommandsReachOnlyARunningOrchestrator(t *testing.T) {
	repo := t.TempDir()
	var out bytes.Buffer
	if code := Run([]string{"stop"}, &out, repo, nil, herdrEnv); code == 0 || !strings.Contains(out.String(), "no Orchestrator is running") {
		t.Errorf("stop with nothing running: exit %d, %q", code, out.String())
	}

	release, _ := orchestrator.AcquireLock(repo)
	defer release()
	for args, file := range map[string]string{"stop": "stop", "retry hx-1": "retry-hx-1", "park hx-1": "park-hx-1", "address hx-1": "address-hx-1"} {
		if code := Run(strings.Fields(args), &out, repo, nil, herdrEnv); code != 0 {
			t.Errorf("%s: exit %d", args, code)
		}
		if _, err := os.Stat(orchestrator.ControlFile(repo, file)); err != nil {
			t.Errorf("%s left no control file %s", args, file)
		}
	}
	if code := Run([]string{"retry"}, &out, repo, nil, herdrEnv); code == 0 {
		t.Errorf("retry without a Ticket accepted")
	}
}
