use super::trust::trusts;
use super::write_file;
use crate::tempdir::TempDir;
use std::path::Path;

/// The fixtures are the real shapes: ~/.claude.json carries far more than the
/// projects map, and ~/.codex/config.toml carries tables that are not projects.
pub(crate) fn trust_home(repo: &Path) -> TempDir {
    let home = TempDir::new();
    let repo = repo.display();
    write_file(
        &home.path().join(".claude.json"),
        &format!(
            r#"{{
  "numStartups": 12,
  "projects": {{
    "{repo}": {{"hasTrustDialogAccepted": true, "history": []}},
    "{repo}/untrusted": {{"hasTrustDialogAccepted": false}}
  }}
}}"#
        ),
    );
    write_file(
        &home.path().join(".codex/config.toml"),
        &format!(
            r#"model = "gpt-6-astra"

[projects."{repo}"]
trust_level = "trusted"

[projects."{repo}/untrusted"]
trust_level = "untrusted"

[features]
hooks = true
"#
        ),
    );
    home
}

#[test]
fn trust_is_read_from_what_the_agents_themselves_record() {
    let repo = TempDir::new();
    let repo = repo.path();
    let home = trust_home(repo);
    let home = home.path();
    let worktree = repo.join(".harness/worktrees/hx-1");

    for kind in ["claude", "codex"] {
        assert!(
            trusts(kind, home, repo, repo),
            "{kind}: the repo is trusted in the fixture but did not read as trusted"
        );
        // A worktree and a run directory are new directories every run; the
        // agents resolve them to the project root they sit under.
        assert!(
            trusts(kind, home, &worktree, repo),
            "{kind}: a directory under a trusted repo did not read as trusted"
        );
        assert!(
            !trusts(kind, home, &repo.join("untrusted"), repo),
            "{kind}: an explicitly untrusted directory read as trusted"
        );
        let (a, b) = (TempDir::new(), TempDir::new());
        assert!(
            !trusts(kind, home, a.path(), b.path()),
            "{kind}: an unknown directory read as trusted"
        );
    }
    let empty = TempDir::new();
    assert!(
        !trusts("claude", empty.path(), repo, repo),
        "claude: a home with no .claude.json read as trusted"
    );
    assert!(
        !trusts("codex", empty.path(), repo, repo),
        "codex: a home with no config.toml read as trusted"
    );
}
