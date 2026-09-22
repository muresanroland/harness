//! What the first live run exposed: a Stage nobody is watching has to be
//! stoppable, and must not start a session into a trust dialog it cannot
//! answer.

use super::state::{load_state, STATUS_RUNNING};
use super::world::{new_world, spawn_single, spawn_ticket, working, BdTicket};
use super::write_file;
use crate::tempdir::TempDir;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Each wait must observe stop, including waits before an agent is prompted.
#[test]
fn stop_ends_a_single_ticket_run_in_every_wait_state() {
    for phase in ["trust", "shell startup", "working", "blocked", "wake"] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        w.session(working);
        let (entered_tx, entered) = sync_channel::<()>(1);
        let (release_start, release) = channel::<()>();
        let release = Mutex::new(release);
        let mut wait_line = "implement prompted, waiting for implement.md";
        let home = TempDir::new();
        match phase {
            "trust" => {
                o.cfg.home = home.path().to_path_buf();
                wait_line = "does not trust";
            }
            "shell startup" => {
                // Keep the startup deadline beyond the test's patience, and
                // do not return busy until stop exists: it must reach
                // startup's sleep, not a later Wake hold after startup times
                // out.
                o.cfg.tick = Duration::from_secs(1);
                w.hook(move |_, argv| {
                    if argv.join(" ").starts_with("herdr agent start") {
                        let _ = entered_tx.try_send(());
                        let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(5));
                        return Some(Err(r#"{"error":{"code":"agent_pane_busy"}}"#.to_string()));
                    }
                    None
                });
            }
            "blocked" => {
                w.session(|_| (String::new(), "blocked".to_string()));
                wait_line = "waiting at a prompt in implement (pane 1-1)";
            }
            "wake" => {
                w.session(|_| (String::new(), "idle".to_string()));
                wait_line = "stuck in implement: went idle without a result (pane 1-1)";
            }
            _ => {}
        }
        let o = Arc::new(o);
        let run = spawn_single(o.clone(), "hx-1");
        if phase == "shell startup" {
            entered
                .recv_timeout(Duration::from_secs(5))
                .expect("never attempted to start an agent");
        } else {
            w.await_event(wait_line);
        }
        o.stop();
        drop(release_start);
        assert!(
            run.finished_within(Duration::from_secs(1)),
            "{phase}: stop did not end the run while waiting"
        );
        let saved = load_state(&w.repo).unwrap();
        let ts = saved.tickets.get("hx-1");
        assert!(
            ts.is_some_and(|ts| ts.status == STATUS_RUNNING && ts.stage == "implement"),
            "{phase}: stop did not preserve resumable state: {ts:?}"
        );
        assert!(
            w.called("herdr pane close").len() + w.called("herdr tab close").len() == 0,
            "{phase}: stop closed a live pane"
        );
        assert!(
            w.called("herdr agent start h-hx-1-review").is_empty(),
            "{phase}: a stopped stage advanced to Review"
        );
    }
}

#[test]
fn stage_waits_until_its_agent_trusts_the_directory() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let home = TempDir::new(); // neither agent has been run anywhere yet
    o.cfg.home = home.path().to_path_buf();
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");

    w.await_line(&format!(
        "hx-1 waiting: claude does not trust {} yet, open it there once and accept (pane 1-1)",
        o.worktree("hx-1").display()
    ));
    let got = w.called("herdr agent start");
    assert!(
        got.is_empty(),
        "a session was started into a trust dialog: {got:?}"
    );

    // The user opens Claude there once and accepts it.
    write_file(
        &home.path().join(".claude.json"),
        &format!(
            r#"{{"projects": {{"{}": {{"hasTrustDialogAccepted": true}}}}}}"#,
            o.worktree("hx-1").display()
        ),
    );
    w.await_line(&format!(
        "hx-1 claude trusts {} now, carrying on",
        o.worktree("hx-1").display()
    ));
    w.await_line("hx-1 implement started: claude (pane 1-1)");
}

/// The user accepts trust in the Stage's own pane, and trust is granted while
/// their session still holds it: the Stage waits for the pane past the usual
/// startup patience instead of waking on a session that did not start.
#[test]
fn trust_accepted_in_a_busy_pane_waits_for_the_pane_to_free() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let home = TempDir::new();
    o.cfg.home = home.path().to_path_buf();
    let busy = AtomicUsize::new(0);
    w.hook(move |_, argv| {
        // the user's own claude holds the pane for well over six ticks
        if argv.join(" ").starts_with("herdr agent start")
            && busy.fetch_add(1, Ordering::SeqCst) < 20
        {
            return Some(Err(r#"{"error":{"code":"agent_pane_busy"}}"#.to_string()));
        }
        None
    });
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("does not trust");
    write_file(
        &home.path().join(".claude.json"),
        &format!(
            r#"{{"projects": {{"{}": {{"hasTrustDialogAccepted": true}}}}}}"#,
            o.worktree("hx-1").display()
        ),
    );
    // A session that did not start is a Wake, which comes instead of this line.
    w.await_line("hx-1 implement started: claude (pane 1-1)");
    assert!(
        w.called("herdr agent start h-hx-1-implement").len() > 20,
        "the busy pane was not waited for: {:?}",
        w.called("herdr agent start")
    );
    assert!(
        !w.lines().iter().any(|l| l.contains("stuck in")),
        "a busy pane after a trust wait raised a Wake: {:?}",
        w.lines()
    );
}

/// A stop that arrives while a held Ticket sleeps ends the hold: a retry
/// sent in that same moment must not start a fresh session (ADR 0003).
#[test]
fn stop_during_a_hold_does_not_start_a_fresh_session() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.tick = Duration::from_millis(100); // the hold sleeps long enough to be caught in it
    w.session(|_| (String::new(), "idle".to_string()));
    let o = Arc::new(o);
    let run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 stuck in implement: went idle");
    o.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    o.command("retry-hx-1");
    assert!(
        run.finished_within(Duration::from_secs(1)),
        "stop did not end the hold"
    );
    let starts = w.called("herdr agent start");
    assert_eq!(
        starts.len(),
        1,
        "a stopped hold started a fresh session: {starts:?}"
    );
    let ts = o.ticket("hx-1");
    assert!(
        ts.status == STATUS_RUNNING && ts.stage == "implement",
        "stop did not preserve resumable state: {ts:?}"
    );
}
