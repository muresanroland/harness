package cli

import (
	"bytes"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"harness/internal/runner/runnertest"
)

var skillNames = []string{"start-work", "stage-implement", "stage-review", "stage-moderate", "stage-fix", "stage-address"}

// preparedRepo is a Target repo that passes every preflight check.
func preparedRepo(t *testing.T) string {
	t.Helper()
	repo := t.TempDir()
	for _, dir := range []string{".beads", ".agents/skills/create-pr"} {
		if err := os.MkdirAll(filepath.Join(repo, dir), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(repo, ".agents/skills/create-pr/SKILL.md"), []byte("pr"), 0o644); err != nil {
		t.Fatal(err)
	}
	return repo
}

func okTools(dir string, argv []string) (string, error) {
	if strings.Join(argv, " ") == "git remote" {
		return "origin\n", nil
	}
	return "", nil
}

func herdrEnv(key string) string {
	return map[string]string{"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p1", "HERDR_WORKSPACE_ID": "w1"}[key]
}

func TestInitInstallsSkillsWithWorkingSymlinks(t *testing.T) {
	repo := preparedRepo(t)
	var out bytes.Buffer
	if code := Run([]string{"init"}, &out, repo, (&runnertest.Fake{Handle: okTools}).Run, herdrEnv); code != 0 {
		t.Fatalf("init exit %d:\n%s", code, out.String())
	}
	for _, name := range skillNames {
		viaLink, err := os.ReadFile(filepath.Join(repo, ".claude/skills", name, "SKILL.md"))
		if err != nil {
			t.Errorf("%s: symlink does not resolve: %v", name, err)
			continue
		}
		if !strings.Contains(string(viaLink), "name: "+name) {
			t.Errorf("%s: SKILL.md has no matching name in frontmatter", name)
		}
		target, _ := os.Readlink(filepath.Join(repo, ".claude/skills", name))
		if filepath.IsAbs(target) {
			t.Errorf("%s: symlink target %q is not relative", name, target)
		}
	}
	ignore, _ := os.ReadFile(filepath.Join(repo, ".gitignore"))
	if !strings.Contains(string(ignore), ".harness/") {
		t.Errorf(".gitignore lacks .harness/: %q", ignore)
	}
}

func TestInitKeepsEditedSkillUnlessForced(t *testing.T) {
	repo := preparedRepo(t)
	run := (&runnertest.Fake{Handle: okTools}).Run
	Run([]string{"init"}, &bytes.Buffer{}, repo, run, herdrEnv)
	skill := filepath.Join(repo, ".agents/skills/stage-fix/SKILL.md")
	shipped, _ := os.ReadFile(skill)
	os.WriteFile(skill, []byte("edited in the Target repo"), 0o644)
	ignoreBefore, _ := os.ReadFile(filepath.Join(repo, ".gitignore"))

	if code := Run([]string{"init"}, &bytes.Buffer{}, repo, run, herdrEnv); code != 0 {
		t.Fatalf("second init exit %d", code)
	}
	if got, _ := os.ReadFile(skill); string(got) != "edited in the Target repo" {
		t.Errorf("rerun overwrote an edited skill")
	}
	if ignoreAfter, _ := os.ReadFile(filepath.Join(repo, ".gitignore")); !bytes.Equal(ignoreBefore, ignoreAfter) {
		t.Errorf("rerun changed .gitignore")
	}

	Run([]string{"init", "--force"}, &bytes.Buffer{}, repo, run, herdrEnv)
	if got, _ := os.ReadFile(skill); !bytes.Equal(got, shipped) {
		t.Errorf("--force did not restore the shipped skill")
	}
}

func TestPreflightNamesEachMissingPrerequisite(t *testing.T) {
	repo := t.TempDir() // no bd workspace, no create-pr skill
	run := (&runnertest.Fake{Handle: func(dir string, argv []string) (string, error) {
		if strings.Join(argv, " ") == "gh auth status" {
			return "", errors.New("not logged in")
		}
		return "", nil // git remote prints nothing
	}}).Run
	noEnv := func(string) string { return "" }

	for _, args := range [][]string{{"init"}, {"start", "some-epic"}} {
		command := args[0]
		var out bytes.Buffer
		if code := Run(args, &out, repo, run, noEnv); code == 0 {
			t.Errorf("%s: exit 0 with nothing prepared", command)
		}
		for _, want := range []string{"bd workspace", "gh is not authenticated", "git remote", "create-pr", "HERDR_ENV"} {
			if !strings.Contains(out.String(), want) {
				t.Errorf("%s: output lacks %q:\n%s", command, want, out.String())
			}
		}
	}
}
