//! The App table and .harness/config.json: each Stage's App, model and
//! effort, read when the Stage starts.

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
  "moderator": {"app": "codex"},
  "fix": {"app": "codex", "model": "gpt-6-luna"}
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
    // A worktree Stage on codex; only Fix and Address get the network.
    assert_eq!(
        argv(&w, "debate"),
        format!("--sandbox workspace-write -a never --add-dir {run}")
    );
    assert_eq!(
        argv(&w, "fix"),
        format!("--sandbox workspace-write -a never --add-dir {run} -c sandbox_workspace_write.network_access=true -m gpt-6-luna")
    );
    assert!(w.called("herdr agent start h-hx-1-fix")[0].contains("--kind codex"));
    w.await_line("hx-1 implement started: claude opus/high (pane 1-1)");
    w.await_line("hx-1 review 1 started: codex gpt-6-sol/low (pane 1-2)");
    w.await_line("hx-1 fix 1 started: codex gpt-6-luna (pane 1-4)");
}

/// A Review on claude starts in the Run directory with the worktree added,
/// and each Stage waits on the trust of its own row's App.
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
    config(
        &w,
        r#"{"review": {"app": "claude"}, "fix": {"app": "codex"}}"#,
    );
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");

    let worktree = o.worktree("hx-1").display().to_string();
    w.await_line("hx-1 review 1 started: claude (pane 1-2)");
    assert_eq!(
        argv(&w, "review"),
        format!("--permission-mode auto --add-dir {worktree}")
    );
    let split = &w.called("herdr pane split")[0];
    assert!(
        split.contains(&format!("--cwd {} ", o.run_dir("hx-1").display())),
        "{split}"
    );
    w.await_line(&format!(
        "hx-1 waiting: codex does not trust {worktree} yet"
    ));
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
/// before any session starts: unreadable, naming the file, or Implement off
/// claude.
#[test]
fn an_unreadable_config_or_implement_off_claude_wakes_the_stage_that_reads_it() {
    let file = |w: &World| w.repo.join(".harness/config.json").display().to_string();
    for (body, reason) in [
        ("{ not json", None),
        (
            r#"{"implement": {"app": "codex"}}"#,
            Some("Implement off claude needs the two-step Plan"),
        ),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        config(&w, body);
        let o = Arc::new(o);
        let _run = spawn_ticket(o.clone(), "hx-1");

        let reason = reason.map_or_else(|| format!("{}: ", file(&w)), str::to_string);
        w.await_line(&format!("hx-1 stuck in implement: {reason}"));
        assert!(w.called("herdr agent start").is_empty(), "{body}");
    }
}
