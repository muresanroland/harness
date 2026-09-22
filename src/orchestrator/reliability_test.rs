use super::scheduler_test::{run_epic, with_deps};
use super::world::{new_world, spawn_ticket, succeed, BdTicket};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[test]
fn merged_ticket_is_retried_until_bd_closes_it() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1"), with_deps("hx-2", &["hx-1"])]);
    w.lock().merged = true;
    w.fail_once("bd close hx-1", "dolt: database is locked");
    let o = Arc::new(o);

    run_epic(&o); // finishes only if hx-1 does get closed and hx-2 unblocks

    assert_eq!(
        w.called("bd close hx-1").len(),
        2,
        "bd close hx-1, want a retry after the failure"
    );
    let merged = w
        .main_lines()
        .iter()
        .filter(|l| l.contains("hx-1 merged, Ticket closed"))
        .count();
    assert_eq!(
        merged, 1,
        "hx-1 reported merged {merged} times, want once, after it was really closed"
    );
}

#[test]
fn merge_cleanup_forces_past_bd_safety_checks_and_says_what_it_could_not_remove() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    w.fail_once("bd worktree remove", "HEAD not contained in upstream"); // a squash merge
    w.fail_once("git branch -D hx-1", "branch is checked out");
    let o = Arc::new(o);

    run_epic(&o);

    let got = w.called("bd worktree remove");
    assert!(
        got.len() == 1 && got[0].contains("--force"),
        "worktree removal = {got:?}, want --force: the PR is merged"
    );
    w.await_line("hx-1 merged, Ticket closed");
    let said = w.await_event("could not remove the worktree, remove it by hand: ");
    assert!(
        !said.panel && said.ticket.as_deref() == Some("hx-1"),
        "the failed clean-up is housekeeping, for the log alone: {said:?}"
    );
}

#[test]
fn lines_main_could_not_receive_are_delivered_later_in_order() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().main_blocked = true; // the launching pane sits at its own permission prompt
    w.session(|_| (String::new(), "idle".to_string()));
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    // Lines stay in order, so the first line is what gets retried; a third
    // attempt means the Ticket is already holding on its queued Wake.
    while w.called("herdr agent prompt main").len() < 3 {
        thread::sleep(Duration::from_millis(1));
    }
    let got = w.main_lines();
    assert!(got.is_empty(), "a blocked launching pane received {got:?}");
    w.lock().main_blocked = false;
    w.await_line("hx-1 stuck in implement");
    w.control("park-hx-1");
    run.wait();

    let lines = w.main_lines();
    let started = lines.iter().position(|l| l.contains("implement started"));
    let wake = lines.iter().position(|l| l.contains("stuck in"));
    assert!(
        matches!((started, wake), (Some(s), Some(k)) if s <= k)
            && lines[0] == "hx-1 branch hx-1 created",
        "lines arrived out of order or were lost: {lines:?}"
    );
}

#[test]
fn verdict_that_drops_findings_is_not_a_clean_verdict() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|p| match p.stage.as_str() {
        "review" => (
            "STATUS: done\n- (high) a.go:1 — nil map write\n- (low) b.go:2 — dead branch\n"
                .to_string(),
            "idle".to_string(),
        ),
        "verdict" => (
            "STATUS: done\n* fix: a.go:1 (the Moderator ignored the format)\n".to_string(),
            "idle".to_string(),
        ),
        _ => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 review 1 found 2 findings");
    w.await_line("hx-1 stuck in debate 1: Verdict settles 0 of the Review's 2 Findings (pane 1-3)");
    w.control("park-hx-1");
    run.wait();
    assert!(
        w.called("herdr agent start h-hx-1-fix").is_empty(),
        "a PR must not open on a Verdict that lost Findings"
    );
}

#[test]
fn fix_session_is_given_only_the_fix_items() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let fix_text = Arc::new(Mutex::new(String::new()));
    let seen = fix_text.clone();
    w.session(move |p| {
        if p.stage == "verdict" && p.round == 1 {
            return (
                "STATUS: done\n- [fix] (high) a.go:1 — nil map write | reason: agreed | settled: consensus\n- [skip] (low) b.go:2 — naming | reason: style | settled: consensus\n".to_string(),
                "idle".to_string(),
            );
        }
        if p.stage == "fix" && p.round == 1 {
            *seen.lock().unwrap() = p.text.clone();
        }
        succeed(p)
    });
    o.run_ticket("hx-1");

    w.await_line("hx-1 debate 1 settled: 1 to fix, 1 skipped");
    let fix_text = fix_text.lock().unwrap();
    assert!(
        fix_text.contains("a.go:1 — nil map write"),
        "Fix prompt lacks the fix item:\n{fix_text}"
    );
    assert!(
        !fix_text.contains("b.go:2") && !fix_text.contains("verdict-1.md"),
        "a Fix session that does not open the PR must see only fix items:\n{fix_text}"
    );
}
