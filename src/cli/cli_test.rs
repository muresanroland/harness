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

/// The hidden mode Claude Code runs as the PreToolUse hook on ExitPlanMode:
/// the plan on stdin lands in the file named, and it exits 0 with nothing on
/// stdout, no decision, so the dialog shows as usual. Input it cannot read
/// fails without 2, the exit code that would block the tool.
#[test]
fn the_plan_hook_writes_the_plan_and_decides_nothing() {
    let dir = TempDir::new();
    let path = dir.path().join("plan.md");
    let args = ["__plan-hook".to_string(), path.display().to_string()];
    let call = serde_json::json!({
        "session_id": "s1", "hook_event_name": "PreToolUse", "tool_name": "ExitPlanMode",
        "tool_input": { "plan": "# Plan\n\n- change x\n", "planFilePath": "/h/.claude/plans/p.md" },
        "permission_mode": "plan",
    })
    .to_string();
    let mut out = Vec::new();
    let quiet = |_: &str| String::new();
    let code = super::run(
        &args,
        &mut out,
        Some(&mut call.as_bytes()),
        dir.path(),
        Fake::quiet(),
        &quiet,
    );
    assert_eq!(code, 0);
    assert!(
        out.is_empty(),
        "stdout: {:?}",
        String::from_utf8_lossy(&out)
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# Plan\n\n- change x\n"
    );

    std::fs::remove_file(&path).unwrap();
    let code = super::run(
        &args,
        &mut out,
        Some(&mut &b"not json"[..]),
        dir.path(),
        Fake::quiet(),
        &quiet,
    );
    assert!(code != 0 && code != 2, "exit code {code}");
    assert!(out.is_empty() && !path.exists());

    let (_, usage) = run_with(&["bogus"], dir.path(), Fake::quiet(), &quiet);
    assert!(
        !usage.contains("plan-hook"),
        "the hidden mode is in the usage"
    );
}
