use super::result::stage_prompt;
use super::scheduler::address_inputs;
use super::stage::{Config, Orchestrator};
use super::state::{
    acquire_lock, load_state, lock_holder, print_status, STATUS_MERGED, STATUS_PARKED,
    STATUS_PR_OPEN, STATUS_RUNNING,
};
use super::world::{new_world, spawn_epic, succeed, BdTicket, Prompt};
use std::fs;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Runs the Orchestrator until it returns, as 'harness start <epic>' does.
pub(super) fn run_epic(o: &Arc<Orchestrator>) {
    let mut run = spawn_epic(o.clone(), "hx");
    assert!(
        run.finished_within(Duration::from_secs(10)),
        "the Orchestrator never finished the Epic"
    );
    run.wait();
    o.wait_in_flight();
}

pub(super) fn with_deps(id: &str, deps: &[&str]) -> BdTicket {
    BdTicket {
        deps: deps.iter().map(|d| d.to_string()).collect(),
        ..BdTicket::new(id)
    }
}

#[test]
fn scheduler_runs_every_ready_ticket_but_never_more_than_max_at_once() {
    for max in [3, 2] {
        let tickets = (1..=5).map(|i| BdTicket::new(&format!("hx-{i}"))).collect();
        let (w, mut o) = new_world(tickets);
        o.cfg.max = max;
        w.lock().merged = true;
        w.session(|p| {
            thread::sleep(Duration::from_millis(2)); // long enough for Tickets to overlap
            succeed(p)
        });
        let o = Arc::new(o);

        run_epic(&o);

        assert_eq!(w.called("bd worktree create").len(), 5, "Tickets run");
        let peak = w.lock().peak;
        assert_eq!(
            peak, max,
            "most Tickets in the Pipeline at once, want exactly --max {max}"
        );
        w.await_line("epic hx done");
    }
}

/// A Ticket thread that dies gives its --max slot back, so the Ticket is
/// resumed on the next tick instead of holding the Pipeline forever.
#[test]
fn a_ticket_thread_that_panics_frees_its_slot() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    let died = AtomicBool::new(false);
    w.session(move |p| {
        if p.stage == "implement" && !died.swap(true, Ordering::SeqCst) {
            panic!("the fake world failed inside the Ticket thread");
        }
        succeed(p)
    });
    let o = Arc::new(o);

    let mut run = spawn_epic(o.clone(), "hx");
    assert!(
        run.finished_within(Duration::from_secs(10)),
        "the Epic never finished: the dead Ticket's slot was not given back"
    );
    run.wait();
    let joined = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| o.wait_in_flight()));
    assert!(joined.is_err(), "the Ticket thread's panic was lost");

    assert!(
        o.active.lock().unwrap().is_empty(),
        "a dead Ticket still holds a slot"
    );
    assert_eq!(
        o.ticket("hx-1").status,
        STATUS_MERGED,
        "the Ticket was not resumed"
    );
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        2,
        "want a fresh Implement session after the thread died"
    );
}

#[test]
fn blocked_ticket_starts_only_after_its_dependency_is_merged_and_closed() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1"), with_deps("hx-2", &["hx-1"])]);
    w.lock().merged = true;
    let o = Arc::new(o);

    run_epic(&o);

    let order = w.calls().join("\n");
    let closed = order.find("bd close hx-1");
    let started = order.find(&format!(
        "bd worktree create {}",
        o.worktree("hx-2").display()
    ));
    assert!(
        matches!((closed, started), (Some(c), Some(s)) if s >= c),
        "hx-2 must start after hx-1 is closed (close at {closed:?}, start at {started:?})"
    );
    for want in [
        format!("bd worktree remove {}", o.worktree("hx-1").display()),
        "git branch -D hx-1".to_string(),
    ] {
        assert_eq!(w.called(&want).len(), 1, "merge cleanup missing {want:?}");
    }
    assert_eq!(o.ticket("hx-1").status, STATUS_MERGED, "hx-1 status");
}

#[test]
fn one_run_per_target_repo_but_a_stale_lock_does_not_block_a_restart() {
    let repo = crate::tempdir::TempDir::new();
    let release = acquire_lock(repo.path()).unwrap(); // a live Orchestrator: this process
    let err = acquire_lock(repo.path())
        .map(|_| ())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("already running"),
        "second lock = {err}, want a refusal"
    );
    assert_eq!(
        lock_holder(repo.path()),
        std::process::id(),
        "LockHolder, want this process"
    );

    drop(release);
    fs::write(repo.path().join(".harness/lock"), "999999").unwrap(); // a killed Orchestrator's stale lock
    assert!(
        acquire_lock(repo.path()).is_ok(),
        "a stale lock must not block a restart"
    );
}

#[test]
fn closed_pr_parks_the_ticket_and_conflict_is_reported_exactly_once() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    {
        let mut inner = w.lock();
        inner.prs.insert(
            "https://example.test/pr/hx-1".to_string(),
            r#"{"state":"CLOSED","mergeable":"UNKNOWN"}"#.to_string(),
        );
        inner.prs.insert(
            "https://example.test/pr/hx-2".to_string(),
            r#"{"state":"OPEN","mergeable":"CONFLICTING"}"#.to_string(),
        );
    }
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_line("hx-1 parked: PR closed without merging");
    w.await_line("hx-2 pr conflicts with main");
    thread::sleep(Duration::from_millis(30)); // many more polls
    w.control("stop");
    run.wait();
    o.wait_in_flight();

    let conflicts = w
        .main_lines()
        .iter()
        .filter(|l| l.contains("conflicts with main"))
        .count();
    assert_eq!(
        conflicts, 1,
        "conflict reported {conflicts} times, want once"
    );
    assert_eq!(
        o.ticket("hx-1").status,
        STATUS_PARKED,
        "hx-1 status, want parked"
    );
    assert!(
        w.called("bd close").is_empty(),
        "no Ticket may be closed without a merge"
    );
}

#[test]
fn killed_run_resumes_at_the_right_stage_without_redoing_finished_ones() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    let o = Arc::new(o);
    // The first Orchestrator is stopped while Review round 1 is mid-flight.
    let kill = o.clone();
    w.session(move |p| {
        if p.stage == "review" {
            kill.stop.store(true, Ordering::SeqCst);
            return (String::new(), "working".to_string());
        }
        succeed(p)
    });
    let _ = o.run("hx");
    o.wait_in_flight();
    let ts = o.ticket("hx-1");
    assert!(
        ts.status == STATUS_RUNNING && ts.stage == "review" && ts.round == 1,
        "state when killed = {ts:?}"
    );

    let mut status = Vec::new();
    print_status(&mut status, &w.repo);
    let status = String::from_utf8(status).unwrap();
    assert!(
        status.contains("hx-1") && status.contains("review round 1"),
        "status output:\n{status}"
    );

    // A new process: fresh Orchestrator, state loaded from the file.
    let old_pane = o.ticket("hx-1").panes["review"].clone();
    let old = w.lock().agents.get(&old_pane).cloned();
    assert_eq!(
        old.as_deref(),
        Some("working"),
        "restart fixture must leave Review alive"
    );
    let seen = w.clone();
    w.session(move |p| {
        if p.stage == "review" {
            let alive = seen.lock().agents.contains_key(&old_pane);
            assert!(
                !alive,
                "resumed Review was prompted while its old session was still alive"
            );
        }
        succeed(p)
    });
    let state = load_state(&w.repo).unwrap();
    let resumed = Arc::new(Orchestrator::with_state(
        Config::for_tests(w.clone(), &w.repo, &w.home),
        state,
    ));
    let before = w.called("herdr agent start").len();
    run_epic(&resumed);

    let stages: Vec<String> = w.called("herdr agent start")[before..]
        .iter()
        .map(|call| call.split_whitespace().nth(3).unwrap().to_string())
        .collect();
    assert_eq!(
        stages,
        ["h-hx-1-review", "h-hx-1-debate", "h-hx-1-fix"],
        "sessions after resume; Implement was done and Review starts again from its beginning"
    );
    assert_eq!(
        w.called("bd worktree create").len(),
        1,
        "worktree created more than once"
    );
}

#[test]
fn stop_exits_with_state_saved_and_leaves_panes_alone() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| (String::new(), "idle".to_string())); // hx-1 holds on a Wake
    let o = Arc::new(o);
    // Hold the scheduler in an external call while the Ticket consumes stop
    // and exits. It must not resume that now-inactive Ticket when the call
    // returns, even though its persisted state is deliberately still running.
    let armed = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let (arm, world, run) = (armed.clone(), w.clone(), o.clone());
    w.hook(move |_, argv| {
        if argv.join(" ").starts_with("bd list")
            && !stopped.load(Ordering::SeqCst)
            && arm.load(Ordering::SeqCst)
        {
            stopped.store(true, Ordering::SeqCst);
            world.control("stop");
            run.wait_in_flight();
        }
        None
    });
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("WAKE hx-1");

    armed.store(true, Ordering::SeqCst);
    assert!(
        run.finished_within(Duration::from_secs(5)),
        "Run after stop never returned"
    );
    run.wait();
    o.wait_in_flight();

    assert!(
        w.called("herdr pane close").len() + w.called("herdr tab close").len() == 0,
        "stop must leave live panes alone; calls:\n{}",
        w.calls().join("\n")
    );
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        1,
        "scheduler restarted a stopped Ticket"
    );
    let saved = load_state(&w.repo).unwrap();
    let ts = saved.tickets.get("hx-1");
    assert!(
        ts.is_some_and(|ts| ts.status == STATUS_RUNNING && ts.stage == "implement"),
        "saved state = {ts:?}"
    );
}

#[test]
fn retry_unparks_a_parked_ticket() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    w.session(|_| (String::new(), "idle".to_string()));
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_line("WAKE hx-1");
    w.control("park-hx-1");
    w.await_line("hx-1 parked");
    w.session(succeed);
    w.control("retry-hx-1");
    run.wait();
    o.wait_in_flight();
    assert_eq!(
        o.ticket("hx-1").status,
        STATUS_MERGED,
        "want the Ticket to run to a merge after the retry"
    );
}

#[test]
fn address_prompt_carries_the_pr_feedback_and_conflict_state() {
    let gh = r#"{"mergeable":"CONFLICTING","reviews":[{"body":"rename this"}],"comments":[]}"#;
    let inputs = address_inputs("https://example.test/pr/9", gh);
    let inputs: Vec<(&str, &str)> = inputs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let text = stage_prompt("---\nname: stage-address\n---\nAddress the PR.", &inputs);
    for want in [
        "Address the PR.",
        "- PR: https://example.test/pr/9",
        "- Conflicts with main: yes",
        "rename this",
    ] {
        assert!(
            text.contains(want),
            "address prompt lacks {want:?}:\n{text}"
        );
    }
    let clean = address_inputs("u", r#"{"mergeable":"MERGEABLE"}"#);
    assert_eq!(clean[1].1, "no", "mergeable PR reported as conflicting");
}

#[test]
fn address_command_starts_a_fresh_session_in_the_kept_worktree() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().prs.insert(
        "https://example.test/pr/hx-1".to_string(),
        r#"{"state":"OPEN","mergeable":"CONFLICTING","reviews":[{"body":"rename this"}]}"#
            .to_string(),
    );
    let address_prompt: Arc<Mutex<Option<Prompt>>> = Default::default();
    let seen = address_prompt.clone();
    w.session(move |p| {
        if p.stage == "address" {
            *seen.lock().unwrap() = Some(p.clone());
        }
        succeed(p)
    });
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 pr open");

    w.control("address-hx-1");
    w.await_line("hx-1 address done");
    w.control("stop");
    run.wait();
    o.wait_in_flight();

    let text = address_prompt
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.text.clone())
        .unwrap_or_default();
    assert!(
        text.contains("rename this") && text.contains("Conflicts with main: yes"),
        "address prompt:\n{text}"
    );
    let tabs = w.called("herdr tab create");
    assert!(
        tabs.len() == 2 && tabs[1].contains(&format!("--cwd {}", o.worktree("hx-1").display())),
        "address must reopen a Ticket tab in the kept worktree: {tabs:?}"
    );
    assert_eq!(
        o.ticket("hx-1").status,
        STATUS_PR_OPEN,
        "status after address, want it still pr-open"
    );
}

#[test]
fn epic_without_tickets_is_an_error_not_a_done_epic() {
    let (w, o) = new_world(vec![]); // a mistyped Epic id: bd lists no children
    let o = Arc::new(o);
    let err = o.run("hx-typo").unwrap_err();
    assert!(
        err.contains("no Tickets"),
        "Run = {err}, want an error naming the empty Epic"
    );
    for line in w.main_lines() {
        assert!(
            !line.contains("done"),
            "reported {line:?} for an Epic with no Tickets"
        );
    }
}
