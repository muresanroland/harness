use super::init_test::run_with;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;

// No arguments opens the Shell (harness-kqe.9), which no test can run
// without a terminal; the usage is checked on an unknown command instead.
#[test]
fn unknown_command_prints_usage() {
    let (code, out) = run_with(&["bogus"], TempDir::new().path(), Fake::quiet(), &|_| {
        String::new()
    });
    assert_ne!(code, 0, "exit code = 0, want non-zero");
    assert!(out.contains("usage:"), "no usage in output:\n{out}");
    for sub in ["Shell", "init", "--version"] {
        assert!(out.contains(sub), "usage does not mention {sub:?}:\n{out}");
    }
    // Every run is driven from the Shell (ADR 0004).
    for gone in ["start", "status", "stop", "retry", "park", "address"] {
        assert!(
            !out.contains(&format!("  {gone}")),
            "usage still offers the {gone} command:\n{out}"
        );
    }
}

#[test]
fn version_flag_prints_version() {
    let (code, out) = run_with(
        &["--version"],
        TempDir::new().path(),
        Fake::quiet(),
        &|_| String::new(),
    );
    assert_eq!(code, 0, "exit code = {code}, want 0:\n{out}");
    assert_eq!(out, format!("{}\n", crate::version::version()));
}
