use super::world::{new_world, spawn_ticket, succeed, BdTicket};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[test]
fn lines_main_could_not_receive_are_delivered_later_in_order() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().main_blocked = true; // the launching pane sits at its own permission prompt
    w.session(|_| (String::new(), "idle".to_string()));
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    // Lines stay in order, so the first line is what gets retried; a third
    // attempt means the Ticket is already holding on its queued WAKE.
    while w.called("herdr agent prompt main").len() < 3 {
        thread::sleep(Duration::from_millis(1));
    }
    let got = w.main_lines();
    assert!(got.is_empty(), "a blocked launching pane received {got:?}");
    w.lock().main_blocked = false;
    w.await_line("WAKE hx-1 implement");
    w.control("park-hx-1");
    run.wait();

    let lines = w.main_lines();
    let started = lines.iter().position(|l| l.contains("implement started"));
    let wake = lines.iter().position(|l| l.contains("WAKE"));
    assert!(
        matches!((started, wake), (Some(s), Some(k)) if s <= k)
            && lines[0].contains("worktree ready"),
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

    w.await_line("WAKE hx-1 debate Verdict settles 0 of the Review's 2 Findings");
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
