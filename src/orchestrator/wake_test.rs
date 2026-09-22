use super::state::{STATUS_PARKED, STATUS_PR_OPEN};
use super::world::{new_world, spawn_ticket, succeed, BdTicket, Prompt, World};
use super::write_file;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Makes the first Implement session end as the trigger dictates; every
/// later session succeeds.
fn fail_implement_once(
    w: &World,
    fail: impl Fn(&Prompt) -> (String, String) + Send + Sync + 'static,
) {
    let failed = AtomicBool::new(false);
    w.session(move |p| {
        if p.stage == "implement" && !failed.swap(true, Ordering::SeqCst) {
            return fail(p);
        }
        succeed(p)
    });
}

#[test]
fn each_wake_trigger_sends_a_wake_line_and_retry_restarts_the_stage() {
    for (name, reason) in [
        ("failed", "session reported failure"),
        ("idle without result", "went idle without a result"),
        ("timeout", "timed out after 5ms"),
        ("prompt not taken", "never took the Stage skill"),
        ("pane died", "session died"),
    ] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        match name {
            "failed" => fail_implement_once(&w, |_| {
                (
                    "STATUS: failed\ntests red\n".to_string(),
                    "idle".to_string(),
                )
            }),
            "idle without result" => {
                fail_implement_once(&w, |_| (String::new(), "idle".to_string()))
            }
            "timeout" => {
                o.cfg.timeout = Some(Duration::from_millis(5));
                fail_implement_once(&w, |_| (String::new(), "working".to_string()));
            }
            "prompt not taken" => {
                // A session still at a trust dialog takes no prompt, and then
                // sits there looking idle: without this the Stage reads as
                // finished.
                w.fail_once(
                    "herdr agent prompt w1:p",
                    r#"{"error":{"code":"agent_blocked"}}"#,
                );
            }
            "pane died" => {
                let world = w.clone();
                fail_implement_once(&w, move |p| {
                    world.lock().agents.remove(&p.pane);
                    (String::new(), "idle".to_string())
                });
            }
            _ => unreachable!(),
        }
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");

        let wake = w.await_line(&format!("hx-1 stuck in implement: {reason}"));
        assert!(
            wake.ends_with(" (pane 1-1)"),
            "{name}: the stuck line does not end with the pane: {wake:?}"
        );
        assert!(
            !run.finished_within(Duration::from_millis(20)),
            "{name}: the Ticket did not wait after its Wake"
        );

        w.lock().wait_err = None;
        o.command("retry-hx-1");
        let dropped = w.await_event("dropped a leftover pane (pane 1-1)");
        assert!(
            !dropped.panel,
            "housekeeping showed on the panel: {dropped:?}"
        );
        w.await_line("hx-1 retrying implement with a fresh session (pane 1-1)");
        run.wait();
        assert_eq!(
            o.ticket("hx-1").status,
            STATUS_PR_OPEN,
            "{name}: status after retry"
        );
        let starts = w.called("herdr agent start h-hx-1-implement");
        assert_eq!(
            starts.len(),
            2,
            "{name}: implement sessions, want a second, fresh one"
        );
    }
}

#[test]
fn blocked_session_wakes_main_then_continues_when_the_user_answers() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    fail_implement_once(&w, |_| {
        ("STATUS: done\n".to_string(), "blocked".to_string())
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 waiting at a prompt in implement (pane 1-1)");
    for status in w.lock().agents.values_mut() {
        *status = "idle".to_string(); // the user answered the permission prompt
    }
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        1,
        "a blocked session must not be restarted"
    );
}

#[test]
fn second_failure_after_retry_parks_the_ticket() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| ("STATUS: failed\n".to_string(), "idle".to_string()));
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 stuck in implement");
    o.command("retry-hx-1");
    run.wait();

    let ts = o.ticket("hx-1");
    assert!(
        ts.status == STATUS_PARKED && ts.reason.contains("after a retry"),
        "state = {ts:?}, want parked after the retry failed"
    );
    w.await_line("hx-1 parked: implement session reported failure again after a retry");
    let wakes = w.lines().iter().filter(|l| l.contains("stuck in")).count();
    assert_eq!(
        wakes, 1,
        "stuck lines, want 1: the second failure parks instead"
    );
}

#[test]
fn park_command_parks_a_woken_ticket() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| (String::new(), "idle".to_string()));
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 stuck in implement");
    o.command("park-hx-1");
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PARKED, "want parked");
}

#[test]
fn nudged_session_that_then_writes_done_advances_without_a_retry() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let file = Arc::new(Mutex::new(String::new()));
    let seen = file.clone();
    fail_implement_once(&w, move |p| {
        *seen.lock().unwrap() = p.file.clone();
        (String::new(), "idle".to_string())
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 stuck in implement: went idle");
    let file = file.lock().unwrap().clone();
    write_file(file.as_ref(), "STATUS: done\n"); // the follow-up prompt worked
    run.wait();
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        1,
        "implement sessions, want 1"
    );
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);
}

#[test]
fn commands_sent_before_a_wake_do_not_answer_it() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    o.command("park-hx-1"); // sent while the Ticket was running normally
    o.command("retry-hx-1");
    w.session(|_| (String::new(), "idle".to_string()));
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 stuck in implement");
    assert!(
        !run.finished_within(Duration::from_millis(20)),
        "a stale command answered the Wake"
    );
    o.command("park-hx-1");
    run.wait();
}

#[test]
fn restart_does_not_grant_a_second_retry() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| ("STATUS: failed\n".to_string(), "idle".to_string()));
    // The state a killed Orchestrator left behind: Implement was already retried.
    o.update("hx-1", |ts| {
        ts.stage = "implement".to_string();
        ts.round = 0;
        ts.retried = true;
    });

    o.run_ticket("hx-1");

    let ts = o.ticket("hx-1");
    assert_eq!(
        ts.status, STATUS_PARKED,
        "state after resume = {ts:?}, want parked: a restart must not grant another retry"
    );
    for line in w.lines() {
        assert!(
            !line.contains("stuck in"),
            "unexpected {line:?}: the one retry was already spent"
        );
    }
}
