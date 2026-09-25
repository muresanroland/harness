use super::app::{app, APPS};
use super::trust::{cursor_slug, trusts};
use super::write_file;
use crate::tempdir::TempDir;
use std::fs;
use std::path::Path;

/// Gives pi a protected resource in dir, so that it asks there.
fn pi_protects(dir: &Path) {
    fs::create_dir_all(dir.join(".agents/skills")).unwrap();
}

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
    write_file(
        &home.path().join(".pi/agent/trust.json"),
        &format!(r#"{{"{repo}": true, "{repo}/untrusted": false}}"#),
    );
    write_file(
        &home.path().join(".copilot/config.json"),
        &format!(r#"{{"banner": "never", "trustedFolders": ["{repo}"]}}"#),
    );
    let slug = cursor_slug(Path::new(&repo.to_string()));
    write_file(
        &home
            .path()
            .join(format!(".cursor/projects/{slug}/.workspace-trusted")),
        "",
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
    pi_protects(repo);

    // opencode has no trust dialog: it trusts every directory.
    for app in APPS.iter().filter(|a| a.name != "opencode") {
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
        // copilot and cursor record no untrusted directory.
        let recorded = !matches!(app.name, "copilot" | "cursor");
        assert_eq!(
            !trusts(app, home, &repo.join("untrusted"), repo),
            recorded,
            "{}: an explicitly untrusted directory",
            app.name
        );
        let (a, b) = (TempDir::new(), TempDir::new());
        pi_protects(a.path());
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

/// pi, copilot and cursor let a trusted ancestor cover its children, above
/// the repo too, as trusting the parent folder records it; opencode trusts
/// every directory.
#[test]
fn a_trusted_ancestor_covers_a_worktree_and_opencode_trusts_any() {
    let parent = TempDir::new();
    let repo = parent.path().join("repo");
    let worktree = repo.join(".harness/worktrees/hx-1");
    let home = trust_home(parent.path());
    pi_protects(&repo);
    for name in ["pi", "copilot", "cursor", "opencode"] {
        assert!(
            trusts(app(name).unwrap(), home.path(), &worktree, &repo),
            "{name}: a worktree under a trusted parent did not read as trusted"
        );
    }
    let (empty, a, b) = (TempDir::new(), TempDir::new(), TempDir::new());
    assert!(trusts(
        app("opencode").unwrap(),
        empty.path(),
        a.path(),
        b.path()
    ));
    for name in ["pi", "copilot", "cursor"] {
        assert!(
            !trusts(app(name).unwrap(), empty.path(), &worktree, &repo),
            "{name}: a home with nothing recorded read as trusted"
        );
    }
}

/// pi asks, and records, only where a .agents/skills or a .pi holding
/// anything is in dir or a parent below home; with none it trusts dir.
#[test]
fn pi_trusts_a_folder_with_nothing_to_protect() {
    let pi = app("pi").unwrap();
    let home = TempDir::new();
    let home = home.path();
    let repo = home.join("repo");
    let worktree = repo.join(".harness/worktrees/hx-1");
    fs::create_dir_all(&worktree).unwrap();
    // Skills at the User location are no project's.
    pi_protects(home);
    fs::create_dir_all(repo.join(".pi")).unwrap();
    assert!(
        trusts(pi, home, &worktree, &repo),
        "an empty .pi or ~/.agents/skills asked"
    );

    write_file(&repo.join(".pi/settings.json"), "{}");
    assert!(
        !trusts(pi, home, &worktree, &repo),
        "a .pi holding a file did not ask"
    );

    // Protected in the worktree alone: its parents, unprotected, do not
    // answer for it.
    fs::remove_dir_all(repo.join(".pi")).unwrap();
    pi_protects(&worktree);
    assert!(
        !trusts(pi, home, &worktree, &repo),
        "a worktree's own skills did not ask"
    );
}

/// cursor keeps a project under its path, every other character a dash.
#[test]
fn cursor_names_a_project_by_its_path() {
    assert_eq!(
        cursor_slug(Path::new("/Users/me/my.repo")),
        "Users-me-my-repo"
    );
}
