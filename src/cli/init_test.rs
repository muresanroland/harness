use super::run;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use crate::tools::Tools;
use std::fs;
use std::path::Path;
use std::sync::Arc;

const SKILL_NAMES: [&str; 6] = [
    "start-work",
    "stage-implement",
    "stage-review",
    "stage-moderate",
    "stage-fix",
    "stage-address",
];

/// A Target repo that passes every preflight check.
pub(super) fn prepared_repo() -> TempDir {
    let repo = TempDir::new();
    for dir in [".beads", ".agents/skills/create-pr"] {
        fs::create_dir_all(repo.path().join(dir)).unwrap();
    }
    fs::write(repo.path().join(".agents/skills/create-pr/SKILL.md"), "pr").unwrap();
    repo
}

pub(super) fn ok_tools() -> Arc<dyn Tools> {
    Fake::new(|_, argv| {
        Ok(if argv.join(" ") == "git remote" {
            "origin\n".to_string()
        } else {
            String::new()
        })
    })
}

pub(super) fn herdr_env(key: &str) -> String {
    match key {
        "HERDR_ENV" => "1",
        "HERDR_PANE_ID" => "w1:p1",
        "HERDR_WORKSPACE_ID" => "w1",
        _ => "",
    }
    .to_string()
}

pub(super) fn run_with(
    args: &[&str],
    repo: &Path,
    tools: Arc<dyn Tools>,
    env: &dyn Fn(&str) -> String,
) -> (i32, String) {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let mut out = Vec::new();
    let code = run(&args, &mut out, Some(&mut &b""[..]), repo, tools, env);
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn init_installs_skills_with_working_symlinks() {
    let repo = prepared_repo();
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "init exit {code}:\n{out}");
    for name in SKILL_NAMES {
        let link = repo.path().join(".claude/skills").join(name);
        let via_link = fs::read_to_string(link.join("SKILL.md"))
            .unwrap_or_else(|err| panic!("{name}: symlink does not resolve: {err}"));
        assert!(
            via_link.contains(&format!("name: {name}")),
            "{name}: SKILL.md has no matching name in frontmatter"
        );
        let target = fs::read_link(&link).unwrap();
        assert!(
            target.is_relative(),
            "{name}: symlink target {target:?} is not relative"
        );
    }
    let ignore = fs::read_to_string(repo.path().join(".gitignore")).unwrap_or_default();
    assert!(
        ignore.contains(".harness/"),
        ".gitignore lacks .harness/: {ignore:?}"
    );
}

#[test]
fn init_keeps_edited_skill_unless_forced() {
    let repo = prepared_repo();
    run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    let skill = repo.path().join(".agents/skills/stage-fix/SKILL.md");
    let shipped = fs::read_to_string(&skill).unwrap();
    fs::write(&skill, "edited in the Target repo").unwrap();
    let ignore_before = fs::read_to_string(repo.path().join(".gitignore")).unwrap();

    let (code, _) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "second init exit {code}");
    assert_eq!(
        fs::read_to_string(&skill).unwrap(),
        "edited in the Target repo",
        "rerun overwrote an edited skill"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".gitignore")).unwrap(),
        ignore_before,
        "rerun changed .gitignore"
    );

    run_with(&["init", "--force"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(
        fs::read_to_string(&skill).unwrap(),
        shipped,
        "--force did not restore the shipped skill"
    );
}

#[test]
fn preflight_names_each_missing_prerequisite() {
    let repo = TempDir::new(); // no bd workspace, no create-pr skill
    let tools: Arc<dyn Tools> = Fake::new(|_, argv| {
        if argv.join(" ") == "gh auth status" {
            return Err("not logged in".to_string());
        }
        Ok(String::new()) // git remote prints nothing
    });
    let no_env = |_: &str| String::new();

    for args in [vec!["init"], vec!["start", "some-epic"]] {
        let command = args[0];
        let start_repo = TempDir::new(); // start installs nothing, so it still misses create-pr
        let mut want = vec![
            "bd workspace",
            "gh is not authenticated",
            "git remote",
            "HERDR_ENV",
        ];
        let repo = if command == "start" {
            want.push("create-pr");
            &start_repo
        } else {
            &repo
        };
        let (code, out) = run_with(&args, repo.path(), tools.clone(), &no_env);
        assert_ne!(code, 0, "{command}: exit 0 with nothing prepared");
        for want in want {
            assert!(
                out.contains(want),
                "{command}: output lacks {want:?}:\n{out}"
            );
        }
    }
}

#[test]
fn init_installs_create_pr_and_keeps_the_repos_own() {
    let fresh = prepared_repo();
    fs::remove_dir_all(fresh.path().join(".agents/skills/create-pr")).unwrap();
    let (code, _) = run_with(&["init"], fresh.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "init exit {code}");
    let got = fs::read_to_string(fresh.path().join(".claude/skills/create-pr/SKILL.md"));
    assert!(
        got.as_ref()
            .is_ok_and(|got| got.contains("name: create-pr")),
        "shipped create-pr not installed: {got:?}"
    );

    // Nothing on stdin to answer with, so the repo's own create-pr stands.
    let own = prepared_repo();
    let (_, out) = run_with(&["init"], own.path(), ok_tools(), &herdr_env);
    let got = fs::read_to_string(own.path().join(".agents/skills/create-pr/SKILL.md")).unwrap();
    assert_eq!(
        got, "pr",
        "init replaced the repo's own create-pr unasked: {got:?}"
    );
    assert!(
        out.contains("already has a create-pr skill"),
        "init did not ask:\n{out}"
    );
}
