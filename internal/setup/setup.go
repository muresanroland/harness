// Package setup is the thin 'harness init': it installs the shipped skills
// into a Target repo and preflights it.
package setup

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"harness/internal/runner"
	"harness/skills"
)

// InstallSkills writes the Harness's skills to <repo>/.agents/skills and links
// them from <repo>/.claude/skills. An existing skill file is the Target repo's
// own and is left alone unless force. A Target repo that already has a
// create-pr skill of its own is asked what to do with the shipped one, since
// the Fix Stage runs whichever /create-pr the repo ends up with.
func InstallSkills(repo string, force bool, out io.Writer, in io.Reader) error {
	pr := "create-pr" // the name the shipped create-pr is installed under; "" keeps the repo's own
	if !force && hasSkill(repo, "create-pr") {
		pr = askAboutCreatePR(out, in)
	}
	err := fs.WalkDir(skills.FS, ".", func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		name, rest, _ := strings.Cut(path, "/")
		overwrite, renamed := force, false
		if name == "create-pr" {
			if pr == "" {
				return nil
			}
			name, overwrite, renamed = pr, true, pr != name
		}
		dest := filepath.Join(repo, ".agents", "skills", name, rest)
		if _, err := os.Lstat(dest); err == nil && !overwrite {
			return nil
		}
		body, err := skills.FS.ReadFile(path)
		if err != nil {
			return err
		}
		if renamed { // installed beside the repo's own, so it needs its own name in the text
			body = bytes.ReplaceAll(body, []byte("create-pr"), []byte(name))
		}
		if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
			return err
		}
		return os.WriteFile(dest, body, 0o644)
	})
	if err != nil {
		return err
	}
	names, err := skills.FS.ReadDir(".")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Join(repo, ".claude/skills"), 0o755); err != nil {
		return err
	}
	for _, dir := range names {
		name := dir.Name()
		if name == "create-pr" {
			if pr == "" {
				continue
			}
			name = pr
		}
		link := filepath.Join(repo, ".claude/skills", name)
		if info, err := os.Lstat(link); err == nil {
			if name == pr && info.Mode()&os.ModeSymlink == 0 {
				fmt.Fprintf(out, "init: .claude/skills/%s is this repo's own copy, not a link: remove it to use the shipped skill\n", name)
			}
			continue
		}
		if err := os.Symlink(filepath.Join("../../.agents/skills", name), link); err != nil {
			return err
		}
	}
	return IgnoreRunDir(repo)
}

// askAboutCreatePR asks what to do with the shipped create-pr when the Target
// repo already has one, and returns the name to install it under: "" keeps the
// repo's own and installs nothing. A stdin that is not the terminal, or is
// closed, keeps the repo's own, so a non-interactive init never overwrites it.
func askAboutCreatePR(out io.Writer, in io.Reader) string {
	fmt.Fprint(out, "init: this repo already has a create-pr skill, and the Harness ships its own.\r\n")
	defer rawTTY(in)()
	choice := choose(out, in, []string{
		"keep this repo's, install nothing",
		"replace it with the shipped one",
		"install the shipped one beside it, as harness-create-pr",
	})
	return []string{"", "create-pr", "harness-create-pr"}[choice]
}

// choose draws a menu, moves the selection on the arrow keys (or j/k), and
// returns the index the user submits with enter. A digit picks its option
// outright. Anything else, including a closed stdin, leaves the first option.
func choose(out io.Writer, in io.Reader, options []string) int {
	sel := 0
	draw := func() {
		for i, option := range options {
			marker := "  "
			if i == sel {
				marker = "\x1b[7m>" // reverse video, so the selected line reads at a glance
			}
			fmt.Fprintf(out, "%s %s\x1b[0m\x1b[K\r\n", marker, option)
		}
		fmt.Fprint(out, "  ↑/↓ to move, enter to choose\x1b[K\r")
	}
	draw()
	var key [3]byte
	for {
		n, err := in.Read(key[:])
		if err != nil || n == 0 {
			break
		}
		switch {
		case n >= 3 && key[0] == 0x1b && key[1] == '[' && key[2] == 'A', key[0] == 'k':
			sel--
		case n >= 3 && key[0] == 0x1b && key[1] == '[' && key[2] == 'B', key[0] == 'j':
			sel++
		case key[0] == '\r', key[0] == '\n':
			return done(out, options, sel)
		case key[0] > '0' && int(key[0]-'0') <= len(options):
			return done(out, options, int(key[0]-'0')-1)
		case key[0] == 3, key[0] == 'q': // ctrl-C in raw mode: take the safe option
			return done(out, options, 0)
		}
		sel = max(0, min(sel, len(options)-1))
		fmt.Fprintf(out, "\x1b[%dA", len(options)) // back over the menu and redraw it
		draw()
	}
	return done(out, options, sel)
}

func done(out io.Writer, options []string, sel int) int {
	fmt.Fprintf(out, "\x1b[K\r\ninit: %s\r\n", options[sel])
	return sel
}

// rawTTY puts the terminal in raw mode so arrow keys arrive as bytes, and
// returns the func that restores it. When in is not the terminal it does
// nothing, and both are no-ops.
func rawTTY(in io.Reader) func() {
	tty, ok := in.(*os.File)
	if !ok {
		return func() {}
	}
	if info, err := tty.Stat(); err != nil || info.Mode()&os.ModeCharDevice == 0 {
		return func() {}
	}
	stty := func(args ...string) (string, error) {
		cmd := exec.Command("stty", args...)
		cmd.Stdin = tty
		saved, err := cmd.Output()
		return string(bytes.TrimSpace(saved)), err
	}
	saved, err := stty("-g")
	if err != nil {
		return func() {}
	}
	if _, err := stty("raw", "-echo"); err != nil {
		return func() {}
	}
	return func() { stty(saved) }
}

// IgnoreRunDir adds .harness/ to the Target repo's .gitignore once.
func IgnoreRunDir(repo string) error {
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

// Preflight returns one specific message per missing prerequisite.
func Preflight(repo string, run runner.Runner, env func(string) string) []string {
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
		missing = append(missing, "no create-pr skill in .agents/skills or .claude/skills: run 'harness init' to install the shipped one")
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

// ReportMissing prints each missing prerequisite and returns the exit code.
func ReportMissing(out io.Writer, missing []string) int {
	for _, m := range missing {
		fmt.Fprintln(out, "preflight:", m)
	}
	if len(missing) > 0 {
		return 1
	}
	return 0
}
