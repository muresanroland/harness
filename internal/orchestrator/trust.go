package orchestrator

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
)

// An agent started in a directory it does not trust yet opens a trust dialog
// instead of working. Claude's dialog registers as a blocked pane, Codex's
// does not: a Codex pane at its trust screen reads as idle, so the Stage looks
// finished the moment it starts and its result file is simply missing. Neither
// dialog can be answered by the Orchestrator, so it reads what the agents
// themselves record and waits rather than prompting into a dialog.
//
// A Ticket's worktree is a fresh directory every time, so this is the normal
// case, not an edge one.

// trusts reports whether kind already trusts dir. Both agents record trust
// against the project root they resolved, which for a worktree or a run
// directory is an ancestor, so dir's ancestors up to repo answer for it. The
// nearest recorded directory decides: a subdirectory recorded as untrusted is
// untrusted however its repo is recorded.
func trusts(kind, home, dir, repo string) bool {
	recorded := claudeRecords
	if kind == "codex" {
		recorded = codexRecords
	}
	for _, candidate := range append([]string{dir}, ancestors(dir, repo)...) {
		if trusted, known := recorded(home, candidate); known {
			return trusted
		}
	}
	return false
}

// ancestors are dir's parents up to and including repo, nearest first.
func ancestors(dir, repo string) []string {
	var out []string
	for d := filepath.Dir(dir); strings.HasPrefix(d, repo) && d != string(filepath.Separator); d = filepath.Dir(d) {
		out = append(out, d)
		if d == repo {
			break
		}
	}
	return out
}

// claudeRecords reads ~/.claude.json, where an accepted trust dialog is
// recorded per project directory.
func claudeRecords(home, dir string) (trusted, known bool) {
	raw, err := os.ReadFile(filepath.Join(home, ".claude.json"))
	if err != nil {
		return false, false
	}
	var doc struct {
		Projects map[string]struct {
			Accepted bool `json:"hasTrustDialogAccepted"`
		} `json:"projects"`
	}
	if json.Unmarshal(raw, &doc) != nil {
		return false, false
	}
	entry, known := doc.Projects[dir]
	return entry.Accepted, known
}

// codexRecords reads ~/.codex/config.toml, where a trusted project is a
// [projects."<dir>"] table with trust_level = "trusted".
// ponytail: a line scan, not a TOML parser; that one table shape is all the
// Orchestrator reads, and a parser would be a dependency (stdlib has none).
func codexRecords(home, dir string) (trusted, known bool) {
	raw, err := os.ReadFile(filepath.Join(home, ".codex", "config.toml"))
	if err != nil {
		return false, false
	}
	want := `[projects."` + dir + `"]`
	inTable := false
	for _, line := range strings.Split(string(raw), "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "[") {
			inTable = line == want
			continue
		}
		if inTable && strings.HasPrefix(line, "trust_level") {
			_, value, _ := strings.Cut(line, "=")
			return strings.Trim(strings.TrimSpace(value), `"`) == "trusted", true
		}
	}
	return false, false
}
