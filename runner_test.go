package main

import (
	"errors"
	"fmt"
	"strings"
	"sync"
	"testing"
)

// fake is the test double for the Runner seam. It records every call and
// answers from handle; an unhandled call returns "" with no error.
type fake struct {
	mu     sync.Mutex
	calls  []string
	handle func(dir, cmd string) (string, error)
	argv   func(dir string, argv []string) (string, error) // for calls whose arguments contain spaces
}

func (f *fake) run(dir, name string, args ...string) (string, error) {
	cmd := strings.Join(append([]string{name}, args...), " ")
	f.mu.Lock()
	f.calls = append(f.calls, cmd)
	f.mu.Unlock()
	if f.argv != nil {
		return f.argv(dir, append([]string{name}, args...))
	}
	if f.handle == nil {
		return "", nil
	}
	return f.handle(dir, cmd)
}

func (f *fake) called(prefix string) []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	var out []string
	for _, c := range f.calls {
		if strings.HasPrefix(c, prefix) {
			out = append(out, c)
		}
	}
	return out
}

func TestRunnerSeamThroughFake(t *testing.T) {
	f := &fake{handle: func(dir, cmd string) (string, error) {
		if cmd == "git remote" {
			return "origin\n", nil
		}
		return "", errors.New("boom")
	}}
	var run Runner = f.run
	out, err := run("/repo", "git", "remote")
	if err != nil || out != "origin\n" {
		t.Errorf("git remote = %q, %v", out, err)
	}
	if _, err := run("/repo", "gh", "auth", "status"); err == nil {
		t.Errorf("want error from gh")
	}
	if got := fmt.Sprint(f.calls); got != "[git remote gh auth status]" {
		t.Errorf("calls = %s", got)
	}
}

func TestExecRunnerReturnsStdoutAndStderrInError(t *testing.T) {
	out, err := execRunner(t.TempDir(), "sh", "-c", "echo hi; echo oops >&2; exit 3")
	if out != "hi\n" {
		t.Errorf("stdout = %q", out)
	}
	if err == nil || !strings.Contains(err.Error(), "oops") {
		t.Errorf("err = %v, want it to carry stderr", err)
	}
}
