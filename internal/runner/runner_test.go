package runner_test

import (
	"errors"
	"fmt"
	"strings"
	"testing"

	"harness/internal/runner"
	"harness/internal/runner/runnertest"
)

func TestRunnerSeamThroughFake(t *testing.T) {
	f := &runnertest.Fake{Handle: func(dir string, argv []string) (string, error) {
		if strings.Join(argv, " ") == "git remote" {
			return "origin\n", nil
		}
		return "", errors.New("boom")
	}}
	var run runner.Runner = f.Run
	out, err := run("/repo", "git", "remote")
	if err != nil || out != "origin\n" {
		t.Errorf("git remote = %q, %v", out, err)
	}
	if _, err := run("/repo", "gh", "auth", "status"); err == nil {
		t.Errorf("want error from gh")
	}
	if got := fmt.Sprint(f.Calls()); got != "[git remote gh auth status]" {
		t.Errorf("calls = %s", got)
	}
}

func TestExecReturnsStdoutAndStderrInError(t *testing.T) {
	out, err := runner.Exec(t.TempDir(), "sh", "-c", "echo hi; echo oops >&2; exit 3")
	if out != "hi\n" {
		t.Errorf("stdout = %q", out)
	}
	if err == nil || !strings.Contains(err.Error(), "oops") {
		t.Errorf("err = %v, want it to carry stderr", err)
	}
}
