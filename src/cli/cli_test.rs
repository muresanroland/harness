use super::run;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;

fn run_str(args: &[&str], repo: &TempDir) -> (i32, String) {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let mut out = Vec::new();
    let code = run(
        &args,
        &mut out,
        Some(&mut &b""[..]),
        repo.path(),
        Fake::quiet(),
        &|_| String::new(),
    );
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn no_args_prints_usage() {
    let (code, out) = run_str(&[], &TempDir::new());
    assert_ne!(code, 0, "exit code = 0, want non-zero");
    for sub in [
        "start", "init", "status", "stop", "retry", "park", "address",
    ] {
        assert!(out.contains(sub), "usage does not mention {sub:?}:\n{out}");
    }
}

#[test]
fn unknown_command_prints_usage() {
    let (code, out) = run_str(&["bogus"], &TempDir::new());
    assert_ne!(code, 0, "exit code = 0, want non-zero");
    assert!(out.contains("usage:"), "no usage in output:\n{out}");
}
