use super::fake::Fake;
use super::{Exec, Tools};
use crate::tempdir::TempDir;
use std::path::Path;

#[test]
fn tools_seam_through_fake() {
    let fake = Fake::new(|_, argv| match argv.join(" ").as_str() {
        "git remote" => Ok("origin\n".to_string()),
        _ => Err("boom".to_string()),
    });
    let tools: &dyn Tools = &*fake;
    assert_eq!(
        tools.run(Path::new("/repo"), &["git", "remote"]).as_deref(),
        Ok("origin\n")
    );
    assert!(
        tools
            .run(Path::new("/repo"), &["gh", "auth", "status"])
            .is_err(),
        "want error from gh"
    );
    assert_eq!(fake.calls(), ["git remote", "gh auth status"]);
}

#[test]
fn exec_returns_stdout_and_stderr_in_error() {
    let dir = TempDir::new();
    let err = Exec
        .run(dir.path(), &["sh", "-c", "echo hi; echo oops >&2; exit 3"])
        .unwrap_err();
    assert!(
        err.to_string().contains("oops"),
        "err = {err}, want it to carry stderr"
    );
    assert_eq!(
        err.to_string(),
        "sh -c echo hi; echo oops >&2; exit 3: exit status 3: oops"
    );
}

#[test]
fn a_failure_redacts_the_typesafe_key_from_its_command_line() {
    let argv = ["sh", "-c", "exit 1", "--env", "TYPESAFE_API_KEY=sk-secret"];
    let err = Exec.run(Path::new("/"), &argv).unwrap_err();
    assert_eq!(
        err.to_string(),
        "sh -c exit 1 --env TYPESAFE_API_KEY=***: exit status 1: "
    );

    let fake = Fake::new(|_, _| Err("boom".to_string()));
    let err = fake.run(Path::new("/"), &argv).unwrap_err();
    assert!(!err.to_string().contains("sk-secret"), "{err}");
    assert_eq!(
        fake.calls(),
        [argv.join(" ")],
        "the recorded call keeps the argv"
    );
}
