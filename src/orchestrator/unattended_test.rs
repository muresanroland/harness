//! What the first live run exposed: a Stage nobody is watching has to be
//! stoppable, has to survive a launching pane that is not an agent, and must
//! not start a session into a trust dialog it cannot answer.

use super::state::{load_state, STATUS_PR_OPEN, STATUS_RUNNING};
use super::world::{new_world, spawn_single, spawn_ticket, working, BdTicket};
use super::write_file;
use crate::tempdir::TempDir;
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
        let mut wait_line = "hx-1 implement prompted";
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
                wait_line = "WAKE hx-1 implement blocked";
            }
            "wake" => {
                w.session(|_| (String::new(), "idle".to_string()));
                wait_line = "WAKE hx-1 implement went idle";
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
            w.await_line(wait_line);
        }
        w.control("stop");
        drop(release_start);
        assert!(
            run.finished_within(Duration::from_secs(1)),
            "{phase}: stop did not end the run while waiting"
        );
        assert!(o.stopping(), "{phase}: run ended without consuming stop");
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
fn control_files_left_by_an_earlier_run_do_not_command_this_one() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.control("stop"); // the stop that ended the run before this one

    assert!(!o.run_single("hx-1"), "the Ticket parked");
    let got = o.ticket("hx-1");
    assert_eq!(
        got.status, STATUS_PR_OPEN,
        "a stale stop ended the run before it began: {got:?}"
    );
}

#[test]
fn events_go_to_the_log_alone_when_the_launching_pane_hosts_no_agent() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().main_no_agent = true; // started from a shell pane, not a Claude session

    o.run_ticket("hx-1");

    let got = o.ticket("hx-1");
    assert_eq!(
        got.status, STATUS_PR_OPEN,
        "the Pipeline did not finish: {got:?}"
    );
    // One line is tried, finds nobody, and the rest go to the log: the run
    // must not spend every tick retrying a pane that will never take them.
    let got = w.called("herdr agent prompt main");
    assert_eq!(
        got.len(),
        1,
        "tried to reach a pane with no agent {} times: {got:?}",
        got.len()
    );
}

#[test]
fn stage_waits_until_its_agent_trusts_the_directory() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let home = TempDir::new(); // neither agent has been run anywhere yet
    o.cfg.home = home.path().to_path_buf();
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("does not trust");
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
    w.await_line("hx-1 implement started");
}
