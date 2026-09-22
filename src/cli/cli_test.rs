use super::init_test::run_with;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;

#[test]
fn no_args_prints_usage() {
    let (code, out) = run_with(&[], TempDir::new().path(), Fake::quiet(), &|_| {
        String::new()
    });
    assert_ne!(code, 0, "exit code = 0, want non-zero");
    for sub in [
        "start", "init", "status", "stop", "retry", "park", "address",
    ] {
        assert!(out.contains(sub), "usage does not mention {sub:?}:\n{out}");
    }
    assert!(
        !out.contains("--detach"),
        "usage still offers --detach (ADR 0003):\n{out}"
    );
}

#[test]
fn unknown_command_prints_usage() {
    let (code, out) = run_with(&["bogus"], TempDir::new().path(), Fake::quiet(), &|_| {
        String::new()
    });
    assert_ne!(code, 0, "exit code = 0, want non-zero");
    assert!(out.contains("usage:"), "no usage in output:\n{out}");
}
