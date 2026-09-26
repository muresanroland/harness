use super::init_test::run_with;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;

// No arguments opens the Shell (harness-kqe.9), which no test can run
// without a terminal; the usage is checked on an unknown command instead.
// Before init it never gets there: it says to run orqa init.
#[test]
fn no_command_before_init_says_to_run_init() {
    let (code, out) = run_with(&[], TempDir::new().path(), Fake::quiet(), &|_| {
        String::new()
    });
    assert_eq!(code, 1, "exit code = {code}, want 1:\n{out}");
    assert!(out.contains("hasn't been run"), "no init notice:\n{out}");
    assert!(out.contains("orqa init"), "no orqa init:\n{out}");
}

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

/// The hidden mode Claude Code runs as a split's PostModelSwitch hook: the
/// model switched to, from its input on stdin, is a line of the Ticket's
/// in the log, after the lines already there. Input it cannot read fails
/// without 2.
#[test]
fn the_switch_hook_logs_the_model_it_switched_to() {
    let dir = TempDir::new();
    let log = dir.path().join("orchestrator.log");
    std::fs::write(&log, "2026-09-24 10:00:00 hx-1 plan approved\n").unwrap();
    let args = [
        "__switch-hook".to_string(),
        log.display().to_string(),
        "hx-1".to_string(),
    ];
    let call = serde_json::json!({
        "session_id": "s1", "hook_event_name": "PostModelSwitch",
        "from_model": "claude-fable-5-1", "to_model": "claude-opus-5-5",
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
    let logged = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = logged.lines().collect();
    assert_eq!(lines.len(), 2, "{logged}");
    assert_eq!(lines[0], "2026-09-24 10:00:00 hx-1 plan approved");
    assert!(
        lines[1].ends_with(" hx-1 implement switched to claude-opus-5-5"),
        "{logged}"
    );

    let code = super::run(
        &args,
        &mut out,
        Some(&mut &b"not json"[..]),
        dir.path(),
        Fake::quiet(),
        &quiet,
    );
    assert!(code != 0 && code != 2, "exit code {code}");
    assert_eq!(std::fs::read_to_string(&log).unwrap(), logged);

    let (_, usage) = run_with(&["bogus"], dir.path(), Fake::quiet(), &quiet);
    assert!(
        !usage.contains("switch-hook"),
        "the hidden mode is in the usage"
    );
}
