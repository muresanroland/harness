package main

import (
	"embed"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
)

//go:embed skills
var shippedSkills embed.FS

// installSkills writes the Harness's skills to <repo>/.agents/skills and links
// them from <repo>/.claude/skills. An existing skill file is the Target repo's
// own and is left alone unless force.
func installSkills(repo string, force bool) error {
	err := fs.WalkDir(shippedSkills, "skills", func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		dest := filepath.Join(repo, ".agents", path)
		if _, err := os.Lstat(dest); err == nil && !force {
			return nil
		}
		body, err := shippedSkills.ReadFile(path)
		if err != nil {
			return err
		}
		if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
			return err
		}
		return os.WriteFile(dest, body, 0o644)
	})
	if err != nil {
		return err
	}
	names, err := shippedSkills.ReadDir("skills")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Join(repo, ".claude/skills"), 0o755); err != nil {
		return err
	}
	for _, name := range names {
		link := filepath.Join(repo, ".claude/skills", name.Name())
		if _, err := os.Lstat(link); err == nil {
			continue
		}
		if err := os.Symlink(filepath.Join("../../.agents/skills", name.Name()), link); err != nil {
			return err
		}
	}
	return ignoreRunDir(repo)
}

// ignoreRunDir adds .harness/ to the Target repo's .gitignore once.
func ignoreRunDir(repo string) error {
	path := filepath.Join(repo, ".gitignore")
	existing, err := os.ReadFile(path)
	if err != nil && !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	for _, line := range strings.Split(string(existing), "\n") {
		if strings.TrimSpace(line) == ".harness/" {
			return nil
		}
	}
	if len(existing) > 0 && !strings.HasSuffix(string(existing), "\n") {
		existing = append(existing, '\n')
	}
	return os.WriteFile(path, append(existing, ".harness/\n"...), 0o644)
}

// preflight returns one specific message per missing prerequisite.
func preflight(repo string, run Runner, env func(string) string) []string {
	var missing []string
	if _, err := os.Stat(filepath.Join(repo, ".beads")); err != nil {
		missing = append(missing, "no bd workspace here: run 'bd init'")
	}
	if _, err := run(repo, "gh", "auth", "status"); err != nil {
		missing = append(missing, "gh is not authenticated: run 'gh auth login'")
	}
	if remotes, err := run(repo, "git", "remote"); err != nil || strings.TrimSpace(remotes) == "" {
		missing = append(missing, "no git remote: add one with 'git remote add origin <url>'")
	}
	if !hasSkill(repo, "create-pr") {
		missing = append(missing, "no create-pr skill in .agents/skills or .claude/skills: the Fix Stage needs the repo's own /create-pr")
	}
	if env("HERDR_ENV") != "1" {
		missing = append(missing, "HERDR_ENV is not 1: run the Harness from a pane inside herdr")
	}
	return missing
}

func hasSkill(repo, name string) bool {
	for _, root := range []string{".agents/skills", ".claude/skills"} {
		if _, err := os.Stat(filepath.Join(repo, root, name, "SKILL.md")); err == nil {
			return true
		}
	}
	return false
}

func reportMissing(out io.Writer, missing []string) int {
	for _, m := range missing {
		fmt.Fprintln(out, "preflight:", m)
	}
	if len(missing) > 0 {
		return 1
	}
	return 0
}
