package setup

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// repoWithOwnPR is a Target repo that already has a create-pr skill of its own.
func repoWithOwnPR(t *testing.T) string {
	t.Helper()
	repo := t.TempDir()
	dir := filepath.Join(repo, ".agents/skills/create-pr")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "SKILL.md"), []byte("the repo's own"), 0o644); err != nil {
		t.Fatal(err)
	}
	return repo
}

func install(t *testing.T, repo, answer string) string {
	t.Helper()
	var out bytes.Buffer
	if err := InstallSkills(repo, false, &out, strings.NewReader(answer)); err != nil {
		t.Fatal(err)
	}
	return out.String()
}

func read(t *testing.T, repo, path string) string {
	t.Helper()
	body, err := os.ReadFile(filepath.Join(repo, path))
	if err != nil {
		t.Fatalf("%s: %v", path, err)
	}
	return string(body)
}

func TestInstallSkillsAsksBeforeTouchingTheReposOwnCreatePR(t *testing.T) {
	for _, c := range []struct {
		answer, own, beside string
	}{
		{"\r", "the repo's own", ""},                                          // enter on the first option keeps it
		{"", "the repo's own", ""},                                            // so does a closed stdin
		{"nonsense\r", "the repo's own", ""},                                  // so do keys that mean nothing here
		{"\x1b[B\x1b[A\r", "the repo's own", ""},                              // down, then back up
		{"\x1b[B\r", "name: create-pr", ""},                                   // down one: replace it
		{"\x1b[B\x1b[B\x1b[B\r", "the repo's own", "name: harness-create-pr"}, // down past the end
		{"1", "the repo's own", ""},                                           // a digit picks its option outright
		{"2", "name: create-pr", ""},
		{"3", "the repo's own", "name: harness-create-pr"},
	} {
		repo := repoWithOwnPR(t)
		out := install(t, repo, c.answer)
		if !strings.Contains(out, "already has a create-pr skill") {
			t.Errorf("%q: init did not ask:\n%s", c.answer, out)
		}
		if got := read(t, repo, ".agents/skills/create-pr/SKILL.md"); !strings.Contains(got, c.own) {
			t.Errorf("%q: create-pr is %q, want %q", c.answer, got, c.own)
		}
		beside := filepath.Join(repo, ".claude/skills/harness-create-pr/SKILL.md")
		body, err := os.ReadFile(beside)
		if c.beside == "" {
			if err == nil {
				t.Errorf("%q: installed harness-create-pr anyway", c.answer)
			}
			continue
		}
		if err != nil || !strings.Contains(string(body), c.beside) {
			t.Errorf("%q: harness-create-pr not installed through its link: %v %q", c.answer, err, body)
		}
		if strings.Contains(string(body), "name: create-pr") {
			t.Errorf("%q: harness-create-pr still calls itself create-pr", c.answer)
		}
	}
}

func TestInstallSkillsDoesNotAskWhenTheRepoHasNoCreatePR(t *testing.T) {
	repo := t.TempDir()
	if out := install(t, repo, ""); strings.Contains(out, "already has") {
		t.Errorf("init asked about a create-pr the repo does not have:\n%s", out)
	}
	if got := read(t, repo, ".claude/skills/create-pr/SKILL.md"); !strings.Contains(got, "name: create-pr") {
		t.Errorf("shipped create-pr not installed: %q", got)
	}
}

func TestInstallSkillsForceSkipsTheQuestion(t *testing.T) {
	repo := repoWithOwnPR(t)
	var out bytes.Buffer
	if err := InstallSkills(repo, true, &out, strings.NewReader("1\n")); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(out.String(), "already has") {
		t.Errorf("--force still asked:\n%s", out.String())
	}
	if got := read(t, repo, ".agents/skills/create-pr/SKILL.md"); !strings.Contains(got, "name: create-pr") {
		t.Errorf("--force did not install the shipped create-pr: %q", got)
	}
}
