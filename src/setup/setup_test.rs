use super::install_skills;
use crate::tempdir::TempDir;
use std::fs;
use std::path::Path;

/// A Target repo that already has a create-pr skill of its own.
fn repo_with_own_pr() -> TempDir {
    let repo = TempDir::new();
    let dir = repo.path().join(".agents/skills/create-pr");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), "the repo's own").unwrap();
    repo
}

fn install(repo: &Path, answer: &str) -> String {
    let mut out = Vec::new();
    install_skills(repo, false, &mut out, Some(&mut answer.as_bytes())).unwrap();
    String::from_utf8(out).unwrap()
}

fn read(repo: &Path, path: &str) -> String {
    fs::read_to_string(repo.join(path)).unwrap_or_else(|err| panic!("{path}: {err}"))
}

#[test]
fn install_skills_asks_before_touching_the_repos_own_create_pr() {
    for (answer, own, beside) in [
        ("\r", "the repo's own", ""), // enter on the first option keeps it
        ("", "the repo's own", ""),   // so does a closed stdin
        ("nonsense\r", "the repo's own", ""), // so do keys that mean nothing here
        ("\x1b[B\x1b[A\r", "the repo's own", ""), // down, then back up
        ("\x1b[B\r", "name: create-pr", ""), // down one: replace it
        (
            "\x1b[B\x1b[B\x1b[B\r",
            "the repo's own",
            "name: harness-create-pr",
        ), // down past the end
        ("1", "the repo's own", ""),  // a digit picks its option outright
        ("2", "name: create-pr", ""),
        ("3", "the repo's own", "name: harness-create-pr"),
    ] {
        let repo = repo_with_own_pr();
        let out = install(repo.path(), answer);
        assert!(
            out.contains("already has a create-pr skill"),
            "{answer:?}: init did not ask:\n{out}"
        );
        let got = read(repo.path(), ".agents/skills/create-pr/SKILL.md");
        assert!(
            got.contains(own),
            "{answer:?}: create-pr is {got:?}, want {own:?}"
        );
        let body = fs::read_to_string(
            repo.path()
                .join(".claude/skills/harness-create-pr/SKILL.md"),
        );
        if beside.is_empty() {
            assert!(
                body.is_err(),
                "{answer:?}: installed harness-create-pr anyway"
            );
            continue;
        }
        let body = body.unwrap_or_else(|err| {
            panic!("{answer:?}: harness-create-pr not installed through its link: {err}")
        });
        assert!(
            body.contains(beside),
            "{answer:?}: harness-create-pr not installed through its link: {body:?}"
        );
        assert!(
            !body.contains("name: create-pr"),
            "{answer:?}: harness-create-pr still calls itself create-pr"
        );
    }
}

#[test]
fn install_skills_does_not_ask_when_the_repo_has_no_create_pr() {
    let repo = TempDir::new();
    let out = install(repo.path(), "");
    assert!(
        !out.contains("already has"),
        "init asked about a create-pr the repo does not have:\n{out}"
    );
    let got = read(repo.path(), ".claude/skills/create-pr/SKILL.md");
    assert!(
        got.contains("name: create-pr"),
        "shipped create-pr not installed: {got:?}"
    );
}

#[test]
fn install_skills_force_skips_the_question() {
    let repo = repo_with_own_pr();
    let mut out = Vec::new();
    install_skills(repo.path(), true, &mut out, Some(&mut "1\n".as_bytes())).unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(!out.contains("already has"), "--force still asked:\n{out}");
    let got = read(repo.path(), ".agents/skills/create-pr/SKILL.md");
    assert!(
        got.contains("name: create-pr"),
        "--force did not install the shipped create-pr: {got:?}"
    );
}
