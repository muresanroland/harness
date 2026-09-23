use super::judgment::fake::Fake;
use super::judgment::Action;
use super::judgment_test::{choose, criteria};
use super::stage::{Answer, Ask, Config, Orchestrator};
use super::state::{load_state, STATUS_PARKED, STATUS_PR_OPEN};
use super::world::{new_world, spawn_ticket, succeed, working, BdTicket, Prompt, World};
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

/// A retry re-arms the nudge, so the second Wake is a Question again (the
/// Judgment's, were TypeSafe up); once the fresh session's nudge is spent
/// too and a timeout leaves no wait, only park is left, and the Ticket
/// parks by rule.
#[test]
fn second_failure_after_retry_and_nudge_parks_the_ticket() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.timeout = Some(Duration::from_millis(5));
    w.session(working);
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 stuck in implement: timed out after 5ms");
    o.command("retry-hx-1");
    let wake = w.await_nth("stuck in implement: timed out after 5ms", 2);
    assert!(wake.ask.is_some(), "the second Wake asks nothing");
    o.answer(
        "hx-1",
        &o.ticket("hx-1").panes["implement"],
        Answer::Act(Action::NudgeWriteResult),
    );
    run.wait();

    let ts = o.ticket("hx-1");
    assert!(
        ts.status == STATUS_PARKED && ts.reason.contains("after a retry"),
        "state = {ts:?}, want parked after the retry failed"
    );
    w.await_line("hx-1 parked: implement timed out after 5ms again after a retry");
    let wakes = w.lines().iter().filter(|l| l.contains("stuck in")).count();
    assert_eq!(
        wakes, 2,
        "stuck lines, want 2: the third timeout parks instead"
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
    // Sent while the Ticket was running normally; a park then is /park on a
    // running Ticket, which parks it at once.
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
    let typesafe = Fake::new(|body| Ok(choose(body, "park", 0.9)));
    let mut o = o;
    o.cfg.typesafe = typesafe.clone();

    o.run_ticket("hx-1");

    let ts = o.ticket("hx-1");
    assert_eq!(
        ts.status, STATUS_PARKED,
        "state after resume = {ts:?}, want parked"
    );
    let asked = typesafe.requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        criteria(&asked[0]),
        ["nudge_proceed", "nudge_write_result", "park", "wait"],
        "a restart granted another retry"
    );
    assert!(w.lines().iter().all(|l| !l.contains("retrying")));
}

/// A nudge is sent by the hold to the Wake's session and re-arms it: the
/// Ticket Wakes again when the nudged session goes idle without a result,
/// and a result it then writes is accepted. A nudge for a session the Ticket
/// has left is dropped with a log line, never sent to the next Stage's pane.
#[test]
fn a_nudge_is_sent_to_its_session_and_re_arms_the_hold() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let file = o.run_dir("hx-1").join("implement.md");
    let done = file.clone();
    w.session(move |p| match p.stage.as_str() {
        "review" => working(p),
        _ if p.text == "finish up" => {
            write_file(&done, "STATUS: done\n");
            (String::new(), "idle".to_string())
        }
        _ => (String::new(), "idle".to_string()), // Implement and every nudge
    });
    let o = Arc::new(o);
    let run = spawn_ticket(o.clone(), "hx-1");

    let wake = w.await_event("stuck in implement: went idle without a result (pane 1-1)");
    let Some(Ask::Wake {
        pane,
        tail,
        file: asked,
        actions,
        judged,
    }) = wake.ask
    else {
        panic!("the Wake asks nothing: {wake:?}");
    };
    assert!(judged.is_none(), "a Judgment while TypeSafe is down");
    assert_eq!(asked, file);
    assert_eq!(actions, Action::ALL, "without a Judgment both nudges show");
    let nudge = |a: Action, file: &std::path::Path| a.nudge(file).unwrap().0;
    let offered = [
        nudge(Action::NudgeWriteResult, &file),
        nudge(Action::NudgeProceed, &file),
    ];
    assert_eq!(
        w.called("herdr agent read"),
        ["herdr agent read h-hx-1-implement --source recent-unwrapped --lines 120"],
        "the Ticket thread reads the tail when it Wakes"
    );
    assert_eq!(
        tail,
        "Ran the tests: 12 passed.\n> Should I also update the docs?\n"
    );
    assert!(offered[0].contains(&file.display().to_string()));

    o.answer("hx-1", &pane, Answer::Act(Action::NudgeWriteResult));
    w.await_line("hx-1 nudged: write the result file");
    assert_eq!(
        w.called(&format!("herdr agent prompt {pane} "))
            .iter()
            .filter(|c| c.ends_with(&offered[0]))
            .count(),
        1,
        "the canned nudge was not sent to the Wake's pane"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while w
        .lines()
        .iter()
        .filter(|l| l.contains("stuck in implement"))
        .count()
        < 2
    {
        assert!(
            deadline > std::time::Instant::now(),
            "no second Wake after the nudge"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        1,
        "a nudge started a fresh session"
    );

    // A prompt of the user's own; this time the session writes its result.
    o.answer("hx-1", &pane, Answer::Prompt("finish up".to_string()));
    w.await_line("hx-1 nudged with your prompt");
    w.await_line("hx-1 implemented");
    w.await_line("hx-1 review 1 started: codex (pane 1-2)");

    // Implement's session is gone: a nudge for it is not sent to Review's.
    let review = o.ticket("hx-1").panes["review"].clone();
    o.answer("hx-1", &pane, Answer::Act(Action::NudgeProceed));
    w.await_event("dropped your nudge: that session has moved on");
    let review_file = o.run_dir("hx-1").join("review-1.md");
    assert!(
        w.called(&format!("herdr agent prompt {review} "))
            .iter()
            .all(|c| !c.ends_with(&offered[1])
                && !c.ends_with(&nudge(Action::NudgeProceed, &review_file))),
        "a stale nudge reached the next Stage's pane"
    );
    assert!(o.answers.lock().unwrap().is_empty());
    drop(run);
}

/// A new process over the saved state, its Events to the same panel.
fn restarted(w: &Arc<World>, o: &Orchestrator) -> Arc<Orchestrator> {
    let mut cfg = Config::for_tests(w.clone(), &w.repo, &w.home);
    cfg.events = o.cfg.events.clone();
    Arc::new(Orchestrator::with_state(cfg, load_state(&w.repo).unwrap()))
}

/// Starts hx-1 with a session that keeps working, and stops the run once
/// Implement is prompted: its pane stays alive.
fn stopped_in_implement() -> (Arc<World>, Orchestrator, String) {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(working);
    let o = Arc::new(o);
    drop({
        let run = spawn_ticket(o.clone(), "hx-1");
        w.await_event("implement prompted");
        run // dropped: stopped, the pane keeps working
    });
    let pane = o.ticket("hx-1").panes["implement"].clone();
    assert_eq!(w.lock().agents[&pane], "working");
    let o = Arc::try_unwrap(o)
        .ok()
        .expect("the stopped run still holds the Orchestrator");
    (w, o, pane)
}

/// /continue re-runs the completion check on the live pane: a session still
/// working is watched, and one idle without a result Wakes again instead of
/// being restarted.
#[test]
fn a_resumed_stage_watches_its_live_session_and_wakes_without_a_result() {
    let (w, o, pane) = stopped_in_implement();
    let resumed = restarted(&w, &o);
    let mut run = spawn_ticket(resumed.clone(), "hx-1");
    assert!(
        !run.finished_within(Duration::from_millis(20)),
        "the resumed Ticket did not watch its live session"
    );
    assert_eq!(
        w.called("herdr agent start").len(),
        1,
        "resume started a fresh session over a live one"
    );
    w.lock().agents.insert(pane.clone(), "idle".to_string());
    w.await_line("hx-1 stuck in implement: went idle without a result (pane 1-1)");
    w.session(succeed);
    resumed.command("retry-hx-1");
    run.wait();
    assert_eq!(resumed.ticket("hx-1").status, STATUS_PR_OPEN);
    assert_eq!(w.called("herdr agent start h-hx-1-implement").len(), 2);
}

/// Resume takes a live pane for the Stage's only when herdr names the
/// Stage's own agent there; and a valid result already in the run
/// directory is accepted, however dead the pane.
#[test]
fn a_resumed_stage_adopts_only_its_own_agent_and_accepts_a_result_already_there() {
    let (w, o, pane) = stopped_in_implement();
    {
        let mut world = w.lock(); // another agent in that pane now
        world.names.remove("h-hx-1-implement");
        world.names.insert("someone-else".to_string(), pane.clone());
    }
    w.session(succeed);
    let resumed = restarted(&w, &o);
    let mut run = spawn_ticket(resumed.clone(), "hx-1");
    run.wait();
    assert_eq!(resumed.ticket("hx-1").status, STATUS_PR_OPEN);
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        2,
        "another agent's pane was taken for the Stage's session"
    );

    let (w, o, pane) = stopped_in_implement();
    w.lock().agents.remove(&pane); // the session died
    write_file(&o.run_dir("hx-1").join("implement.md"), "STATUS: done\n");
    w.session(succeed);
    let resumed = restarted(&w, &o);
    let mut run = spawn_ticket(resumed.clone(), "hx-1");
    run.wait();
    assert_eq!(resumed.ticket("hx-1").status, STATUS_PR_OPEN);
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        1,
        "a result already there was not accepted"
    );
}

/// A park at a prompt takes the Ticket out at its Stage; a session that
/// moves on by itself says so.
#[test]
fn a_blocked_session_is_parked_by_you_or_carries_on() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| (String::new(), "blocked".to_string()));
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 waiting at a prompt in implement (pane 1-1)");
    o.command("park-hx-1");
    run.wait();
    let ts = o.ticket("hx-1");
    assert_eq!(
        (ts.status.as_str(), ts.reason.as_str()),
        (STATUS_PARKED, "by you at implement")
    );
    w.await_line("hx-1 parked: by you at implement");

    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    fail_implement_once(&w, |_| {
        ("STATUS: done\n".to_string(), "blocked".to_string())
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 waiting at a prompt in implement (pane 1-1)");
    for status in w.lock().agents.values_mut() {
        *status = "idle".to_string();
    }
    w.await_line("hx-1 carrying on");
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);
}
