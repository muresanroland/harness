use super::run;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use crate::tools::Tools;
use std::fs;
use std::io::Read;
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

#[test]
fn init_asks_for_the_typesafe_key_and_preflight_warns_without_one() {
    let repo = prepared_repo();
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "a missing key failed preflight:\n{out}");
    assert!(
        out.contains("preflight: no TypeSafe key: every Wake will be a Question"),
        "no warning without a key:\n{out}"
    );
    assert!(
        out.contains("TypeSafe API key"),
        "init did not ask for the key:\n{out}"
    );

    let with_key = |key: &str| {
        let key = key.to_string();
        move |name: &str| {
            if name == "TYPESAFE_API_KEY" {
                key.clone()
            } else {
                herdr_env(name)
            }
        }
    };
    let (_, out) = run_with(&["init"], repo.path(), ok_tools(), &with_key("sk-env"));
    assert!(
        !out.contains("TypeSafe API key"),
        "asked with the variable set:\n{out}"
    );
    assert!(
        !out.contains("no TypeSafe key"),
        "warned with the variable set:\n{out}"
    );

    // Typed on init's stdin after the gate's answer (its own keystroke, as a
    // terminal delivers it), the key is kept in the repo.
    let args = ["init".to_string()];
    let mut out = Vec::new();
    let mut keys = b"1".chain(&b"sk-typed\n"[..]);
    let code = run(
        &args,
        &mut out,
        Some(&mut keys),
        repo.path(),
        ok_tools(),
        &herdr_env,
    );
    let out = String::from_utf8(out).unwrap();
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read_to_string(repo.path().join(".harness/typesafe-key"))
            .unwrap()
            .trim(),
        "sk-typed"
    );
    assert!(
        !out.contains("no TypeSafe key"),
        "warned with the key kept:\n{out}"
    );
}
