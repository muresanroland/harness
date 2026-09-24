//! The App table and .harness/config.json: each Stage's App, model and
//! effort, read when the Stage starts.

use super::app::app;
use super::world::{new_world, spawn_ticket, succeed, BdTicket, World};
use super::write_file;
use crate::tempdir::TempDir;
use std::sync::Arc;

fn config(w: &World, body: &str) {
    write_file(&w.repo.join(".harness/config.json"), body);
}

/// The args after herdr's `--` that the Stage's session started with.
fn argv(w: &World, stage: &str) -> String {
    let start = &w.called(&format!("herdr agent start h-hx-1-{stage}"))[0];
    start.split_once(" -- ").unwrap().1.to_string()
}

#[test]
fn a_rows_model_and_effort_add_the_flags_on_claude_and_on_codex() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    // Partial: the Review's app is left to its default, codex.
    config(
        &w,
        r#"{
  "implement": {"app": "claude", "model": "opus", "effort": "high"},
  "review": {"model": "gpt-6-sol", "effort": "low"},
  "fix": {"model": "sonnet"}
}"#,
    );
    o.run_ticket("hx-1");

    let run = o.run_dir("hx-1").display().to_string();
    assert!(
        argv(&w, "implement").ends_with(&format!("--add-dir {run} --model opus --effort high")),
        "{}",
        argv(&w, "implement")
    );
    assert_eq!(
        argv(&w, "review"),
        "--sandbox workspace-write -m gpt-6-sol -c model_reasoning_effort=low"
    );
    assert_eq!(
        argv(&w, "fix"),
        format!("--permission-mode auto --add-dir {run} --model sonnet")
    );
    assert!(w.called("herdr agent start h-hx-1-review")[0].contains("--kind codex"));
    w.await_line("hx-1 implement started: claude opus/high (pane 1-1)");
    w.await_line("hx-1 review 1 started: codex gpt-6-sol/low (pane 1-2)");
    w.await_line("hx-1 fix 1 started: claude sonnet (pane 1-4)");
}

/// A Review on claude starts in the Run directory with the worktree added
/// read-only, and waits on claude's trust, not on codex's, its default
/// App's.
#[test]
fn a_review_on_claude_runs_in_the_run_directory_and_trust_is_read_through_the_rows_app() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let home = TempDir::new(); // claude trusts the repo, codex nothing
    write_file(
        &home.path().join(".claude.json"),
        &format!(
            r#"{{"projects": {{"{}": {{"hasTrustDialogAccepted": true}}}}}}"#,
            w.repo.display()
        ),
    );
    o.cfg.home = home.path().to_path_buf();
    config(&w, r#"{"review": {"app": "claude"}}"#);
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");

    let worktree = o.worktree("hx-1").display().to_string();
    w.await_line("hx-1 review 1 started: claude (pane 1-2)");
    assert_eq!(
        argv(&w, "review"),
        format!(
            "--permission-mode auto --add-dir {worktree} --settings \
             {{\"permissions\":{{\"deny\":[\"Edit(/{worktree}/**)\"]}},\"sandbox\":\
             {{\"allowUnsandboxedCommands\":false,\"enabled\":true,\"failIfUnavailable\":true}}}}"
        )
    );
    let split = &w.called("herdr pane split")[0];
    assert!(
        split.contains(&format!("--cwd {} ", o.run_dir("hx-1").display())),
        "{split}"
    );
}

/// A worktree path with a quote or backslash still makes valid settings JSON.
#[test]
fn a_worktree_path_is_escaped_in_the_review_settings_on_claude() {
    let worktree = r#"/tmp/a"b\c"#;
    let args = (app("claude").unwrap().run_dir_args)(worktree);
    let settings: serde_json::Value = serde_json::from_str(args.last().unwrap()).unwrap();
    assert_eq!(
        settings["permissions"]["deny"][0],
        format!("Edit(/{worktree}/**)")
    );
}

/// Running Stages keep theirs; the Stages that start after a change use it.
#[test]
fn config_changed_between_two_stages_reaches_the_second() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let file = w.repo.join(".harness/config.json");
    w.session(move |p| {
        if p.stage == "implement" {
            write_file(&file, r#"{"review": {"model": "gpt-6-sol"}}"#);
        }
        succeed(p)
    });
    o.run_ticket("hx-1");

    assert!(!argv(&w, "implement").contains("gpt-6-sol"));
    assert_eq!(argv(&w, "review"), "--sandbox workspace-write -m gpt-6-sol");
}

/// A config.json no Stage can start on, read as a Stage starts, is a Wake
/// before its session starts: unreadable, naming the file, a field not a
/// string, a Stage other than the Review off claude, or a plan model split
/// from Implement's model where either is not a full claude- id.
#[test]
fn an_unreadable_config_or_a_stage_off_claude_wakes_the_stage_that_reads_it() {
    const SPLIT_NOT_FULL: &str = "{file}: implement plan_model splits from model: \
         the split needs a full claude- model id for each half";
    for (body, label, reason) in [
        ("{ not json", "implement", "{file}: "),
        (
            r#"{"implement": {"model": 5}}"#,
            "implement",
            "{file}: implement model is not a string",
        ),
        (
            r#"{"moderator": {"app": "codex"}}"#,
            "debate 1",
            "moderator runs on claude only",
        ),
        (
            r#"{"implement": {"plan_model": "claude-fable-5-1"}}"#,
            "implement",
            SPLIT_NOT_FULL,
        ),
        (
            r#"{"implement": {"model": "opus", "plan_model": "claude-fable-5-1"}}"#,
            "implement",
            SPLIT_NOT_FULL,
        ),
        (
            r#"{"implement": {"model": "claude-opus-5-5", "plan_model": "fable"}}"#,
            "implement",
            SPLIT_NOT_FULL,
        ),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        config(&w, body);
        let o = Arc::new(o);
        let _run = spawn_ticket(o.clone(), "hx-1");

        let file = w.repo.join(".harness/config.json");
        let reason = reason.replace("{file}", &file.display().to_string());
        w.await_line(&format!("hx-1 stuck in {label}: {reason}"));
        let stage = label.split(' ').next().unwrap();
        assert!(
            w.called(&format!("herdr agent start h-hx-1-{stage}"))
                .is_empty(),
            "{body}"
        );
    }
}
