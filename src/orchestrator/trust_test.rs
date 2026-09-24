use super::app::{app, APPS};
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

    for app in &APPS {
        assert!(
            trusts(app, home, repo, repo),
            "{}: the repo is trusted in the fixture but did not read as trusted",
            app.name
        );
        // A worktree and a run directory are new directories every run; the
        // agents resolve them to the project root they sit under.
        assert!(
            trusts(app, home, &worktree, repo),
            "{}: a directory under a trusted repo did not read as trusted",
            app.name
        );
        assert!(
            !trusts(app, home, &repo.join("untrusted"), repo),
            "{}: an explicitly untrusted directory read as trusted",
            app.name
        );
        let (a, b) = (TempDir::new(), TempDir::new());
        assert!(
            !trusts(app, home, a.path(), b.path()),
            "{}: an unknown directory read as trusted",
            app.name
        );
    }
    // A record that is not a bool is no record: the ancestors answer instead.
    let odd = TempDir::new();
    write_file(
        &odd.path().join(".claude.json"),
        &format!(
            r#"{{"projects": {{"{0}": {{"hasTrustDialogAccepted": true}}, "{0}/untrusted": {{"hasTrustDialogAccepted": "no"}}}}}}"#,
            repo.display()
        ),
    );
    assert!(
        trusts(
            app("claude").unwrap(),
            odd.path(),
            &repo.join("untrusted"),
            repo
        ),
        "claude: a non-bool record hid the trusted repo above it"
    );
    let empty = TempDir::new();
    assert!(
        !trusts(app("claude").unwrap(), empty.path(), repo, repo),
        "claude: a home with no .claude.json read as trusted"
    );
    assert!(
        !trusts(app("codex").unwrap(), empty.path(), repo, repo),
        "codex: a home with no config.toml read as trusted"
    );
}
