package orchestrator

import (
	"path/filepath"
	"testing"
)

// The fixtures are the real shapes: ~/.claude.json carries far more than the
// projects map, and ~/.codex/config.toml carries tables that are not projects.
func trustHome(t *testing.T, repo string) string {
	t.Helper()
	home := t.TempDir()
	writeFile(t, filepath.Join(home, ".claude.json"), `{
	  "numStartups": 12,
	  "projects": {
	    "`+repo+`": {"hasTrustDialogAccepted": true, "history": []},
	    "`+filepath.Join(repo, "untrusted")+`": {"hasTrustDialogAccepted": false}
	  }
	}`)
	writeFile(t, filepath.Join(home, ".codex", "config.toml"), `model = "gpt-6-astra"

[projects."`+repo+`"]
trust_level = "trusted"

[projects."`+filepath.Join(repo, "untrusted")+`"]
trust_level = "untrusted"

[features]
hooks = true
`)
	return home
}

func TestTrustIsReadFromWhatTheAgentsThemselvesRecord(t *testing.T) {
	repo := t.TempDir()
	home := trustHome(t, repo)
	worktree := filepath.Join(repo, ".harness", "worktrees", "hx-1")

	for _, kind := range []string{"claude", "codex"} {
		if !trusts(kind, home, repo, repo) {
			t.Errorf("%s: the repo is trusted in the fixture but did not read as trusted", kind)
		}
		// A worktree and a run directory are new directories every run; the
		// agents resolve them to the project root they sit under.
		if !trusts(kind, home, worktree, repo) {
			t.Errorf("%s: a directory under a trusted repo did not read as trusted", kind)
		}
		if trusts(kind, home, filepath.Join(repo, "untrusted"), repo) {
			t.Errorf("%s: an explicitly untrusted directory read as trusted", kind)
		}
		if trusts(kind, home, t.TempDir(), t.TempDir()) {
			t.Errorf("%s: an unknown directory read as trusted", kind)
		}
	}
	if trusts("claude", t.TempDir(), repo, repo) {
		t.Error("claude: a home with no .claude.json read as trusted")
	}
	if trusts("codex", t.TempDir(), repo, repo) {
		t.Error("codex: a home with no config.toml read as trusted")
	}
}
