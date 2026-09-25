use super::app::app;
use super::judgment::fake::Fake as TypeSafeFake;
use super::limit::{find, until, Limit};
use super::stage::{Ask, Orchestrator};
use super::state::{load_state, Review};
use super::world::{
    new_world, restarted, set_clock, spawn_ticket, succeed, wait_until, BdTicket, World,
};
use super::write_file;
use crate::skills::SKILLS;
use crate::tools::Tools;
use chrono::{DateTime, Local, TimeZone};
use std::fs;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Mutex};
use std::thread;

/// Friday 25 Sep 2026, 2pm, local time.
fn now() -> DateTime<Local> {
    at(25, 14, 0)
}

/// A local time in September 2026.
fn at(day: u32, hour: u32, min: u32) -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 9, day, hour, min, 0).unwrap()
}

/// The limit `tail` shows for the App named `app_name`, at now().
fn found(app_name: &str, tail: &str) -> Option<Limit> {
    find(app(app_name).unwrap(), tail, now())
}

#[test]
fn each_apps_limit_text_is_a_limit_until_its_reset() {
    // (App, pane tail, reset, long, what)
    let cases = [
        (
            "claude",
            "⎿  You've hit your session limit · resets 3:45pm (Europe/Bucharest)\n",
            at(25, 15, 45),
            false,
            "session limit",
        ),
        (
            "claude",
            "Usage limit reached · continuing automatically at 3:45pm · esc to cancel",
            at(25, 15, 45),
            false,
            "usage limit",
        ),
        // A time without a date is its next local occurrence.
        (
            "claude",
            "You've hit your limit · resets 1pm (America/Toronto)",
            at(26, 13, 0),
            false,
            "limit",
        ),
        (
            "claude",
            "You've hit your weekly limit · resets Mon 12:00am",
            at(28, 0, 0),
            true,
            "weekly limit",
        ),
        (
            "claude",
            "You've hit your Opus limit · resets Sep 27, 3pm (Europe/Bucharest)",
            at(27, 15, 0),
            true,
            "Opus limit",
        ),
        (
            "codex",
            "■ You’ve hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at 3:05 PM.",
            at(25, 15, 5),
            false,
            "usage limit",
        ),
        (
            "codex",
            "■ You’ve hit your usage limit. Try again at Sep 26th, 2026 1:05 AM.",
            at(26, 1, 5),
            false,
            "usage limit",
        ),
        // No reset at all: an hour from now, then look again.
        (
            "codex",
            "■ You’ve hit your usage limit. Try again later.",
            at(25, 15, 0),
            false,
            "usage limit",
        ),
        // The experimental Apps, their text as the research quotes it; a
        // reset "in" a while is from now.
        (
            "pi",
            "Error: You have hit your ChatGPT usage limit (plus plan). Try again in ~45 min.",
            at(25, 14, 45),
            false,
            "ChatGPT usage limit",
        ),
        (
            "opencode",
            "5-hour usage limit reached. It will reset in 3 hours 12 minutes. Upgrade to continue.",
            at(25, 17, 12),
            false,
            "5-hour usage limit",
        ),
        // It waits inside its turn, looking working.
        (
            "opencode",
            "Rate limit exceeded [retrying in 3h 12m attempt #2]",
            at(25, 17, 12),
            false,
            "usage limit",
        ),
        (
            "copilot",
            "You've hit your session rate limit. Please wait for your limit to reset in 12 minutes or switch to auto model to continue.",
            at(25, 14, 12),
            false,
            "session rate limit",
        ),
        (
            "copilot",
            "You've reached your weekly rate limit. Please wait for your limit to reset on September 28, 2026 at 3:00 PM or switch to auto model to continue.",
            at(28, 15, 0),
            true,
            "weekly rate limit",
        ),
        (
            "copilot",
            "You've hit the rate limit for this model. Please wait for your limit to reset in 5 minutes or switch to auto model to continue.",
            at(25, 14, 5),
            false,
            "rate limit",
        ),
        // The second sentence on a line of its own.
        (
            "copilot",
            "Please wait for your limit to reset in 5 minutes or switch to auto model to continue.",
            at(25, 14, 5),
            false,
            "usage limit",
        ),
        (
            "copilot",
            "You've run out of your included AI credits for the month. Upgrade or wait for next month.",
            at(25, 15, 0),
            false,
            "AI credits",
        ),
        (
            "cursor",
            "Increase limits for faster responses — You're out of usage. Switch to Auto, or ask your admin to increase your limit.",
            at(25, 15, 0),
            false,
            "usage limit",
        ),
    ];
    for (name, tail, reset, long, what) in cases {
        let limit = found(name, tail).unwrap_or_else(|| panic!("{name}: no limit in {tail:?}"));
        assert_eq!(
            (limit.app, limit.reset, limit.long, limit.what.as_str()),
            (name, reset, long, what),
            "{tail:?}"
        );
    }
}

#[test]
fn claudes_options_menu_is_a_long_limit_whatever_its_reset() {
    let tail = "You've hit your Opus limit · resets 3:45pm\n\n What do you want to do?\n\n ❯ 1. Stop and wait for limit to reset\n   2. Upgrade your plan\n";
    let limit = found("claude", tail).unwrap();
    assert!(limit.long && limit.reset == at(25, 15, 45), "{limit:?}");
}

#[test]
fn a_menu_older_than_the_limit_line_is_not_its_menu() {
    let tail =
        "Agent asked: What do you want to do?\n\nYou've hit your session limit · resets 3:45pm\n";
    let limit = found("claude", tail).unwrap();
    assert!(!limit.long && limit.reset == at(25, 15, 45), "{limit:?}");
}

#[test]
fn a_past_reset_an_old_line_or_another_apps_text_is_no_limit() {
    let newer = "Ran the tests: 12 passed.\n".repeat(20);
    for (name, tail) in [
        (
            "codex",
            "■ You’ve hit your usage limit. Try again at Sep 24th, 2026 3:05 PM.".to_string(),
        ),
        (
            "claude",
            "You've hit your session limit · resets Sep 24, 3pm".to_string(),
        ),
        (
            "claude",
            format!("You've hit your session limit · resets 3:45pm\n{newer}"),
        ),
        (
            "claude",
            "You've used 85% of your session limit · resets 3:45pm".to_string(),
        ),
        (
            "claude",
            "■ You’ve hit your usage limit. Try again at 3:05 PM.".to_string(),
        ),
        (
            "codex",
            "You've hit your session limit · resets 3:45pm".to_string(),
        ),
        (
            "pi",
            format!("You have hit your ChatGPT usage limit (plus plan). Try again in ~45 min.\n{newer}"),
        ),
        (
            "opencode",
            format!("5-hour usage limit reached. It will reset in 3 hours 12 minutes.\n{newer}"),
        ),
        // A retry in seconds is no limit's.
        (
            "opencode",
            "Server overloaded [retrying in 45s attempt #1]".to_string(),
        ),
        (
            "copilot",
            format!("You've hit your session rate limit. Please wait for your limit to reset in 12 minutes or switch to auto model to continue.\n{newer}"),
        ),
        (
            "copilot",
            "You've reached your weekly rate limit. Please wait for your limit to reset on September 24, 2026 at 3:00 PM or switch to auto model to continue.".to_string(),
        ),
        (
            "cursor",
            format!("You're out of usage. Switch to Auto.\n{newer}"),
        ),
        (
            "cursor",
            "You have hit your ChatGPT usage limit (plus plan). Try again in ~45 min.".to_string(),
        ),
    ] {
        assert!(found(name, &tail).is_none(), "{name}: {tail:?}");
    }
    let just_in = format!(
        "You've hit your session limit · resets 3:45pm\n{}",
        "ok\n".repeat(19)
    );
    assert!(
        found("claude", &just_in).is_some(),
        "the 20th line from the end is read"
    );
}

#[test]
fn a_yearless_date_across_the_new_year_is_the_nearest_one() {
    let claude = app("claude").unwrap();
    let eve = Local.with_ymd_and_hms(2026, 12, 31, 14, 0, 0).unwrap();
    let limit = find(
        claude,
        "You've hit your weekly limit · resets Jan 2, 3am",
        eve,
    )
    .unwrap();
    assert_eq!(
        limit.reset,
        Local.with_ymd_and_hms(2027, 1, 2, 3, 0, 0).unwrap()
    );
    let new_year = Local.with_ymd_and_hms(2027, 1, 1, 14, 0, 0).unwrap();
    let old = "You've hit your session limit · resets Dec 31, 3pm";
    assert!(
        find(claude, old, new_year).is_none(),
        "last year's line is old"
    );
}

#[test]
fn a_reset_reads_as_its_time_today_or_its_day_otherwise() {
    assert_eq!(until(at(25, 15, 45), now()), "3:45pm");
    assert_eq!(until(at(25, 15, 0), now()), "3:00pm");
    assert_eq!(until(at(28, 0, 0), now()), "Mon 12:00am");
}

const CLAUDE: &str = "⎿  You've hit your session limit · resets 3:45pm (Europe/Bucharest)";
const CODEX: &str = "■ You’ve hit your usage limit. Try again at 3:05 PM.";

/// Sets the Orchestrator's wall clock at now(); the test moves it.
fn clock(o: &mut Orchestrator) -> Arc<Mutex<DateTime<Local>>> {
    set_clock(&mut o.cfg, now())
}

/// Ticket `ticket`'s `stage` session stops at a usage limit: no result, its
/// pane settled in `status` with `tail` as its last lines. A session told
/// to continue afterwards (resumed by id too) writes that result and goes
/// idle; every other session succeeds.
pub(crate) fn hits(
    w: &Arc<World>,
    ticket: &'static str,
    stage: &'static str,
    status: &str,
    tail: &str,
) {
    let (world, status, tail) = (w.clone(), status.to_string(), tail.to_string());
    let file = Mutex::new(String::new());
    w.session(move |p| {
        if p.ticket == ticket && p.stage == stage {
            world.lock().tails.insert(p.pane.clone(), tail.clone());
            *file.lock().unwrap() = p.file.clone();
            return (String::new(), status.clone());
        }
        if p.text == "continue" {
            write_file(
                std::path::Path::new(&*file.lock().unwrap()),
                "STATUS: done\n",
            );
            return (String::new(), "idle".to_string());
        }
        succeed(p)
    });
}

#[test]
fn claude_or_codex_limit_text_holds_the_ticket_with_no_wake_and_no_judgment() {
    // (Stage, the hit line, its App, the pane tail, the reset)
    let cases = [
        (
            "implement",
            "hx-1 claude session limit until 3:45pm: implement holds (pane 1-1)",
            "claude",
            CLAUDE,
            at(25, 15, 45),
        ),
        (
            "review",
            "hx-1 codex usage limit until 3:05pm: review 1 holds (pane 1-2)",
            "codex",
            CODEX,
            at(25, 15, 5),
        ),
    ];
    for (stage, line, app, tail, reset) in cases {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        let typesafe = TypeSafeFake::down();
        o.cfg.typesafe = typesafe.clone();
        clock(&mut o);
        // yesterday's limit at the same time of day, another session's
        o.state
            .lock()
            .unwrap()
            .limits
            .insert(app.to_string(), reset - chrono::Duration::days(1));
        hits(&w, "hx-1", stage, "idle", tail);
        let o = Arc::new(o);
        let run = spawn_ticket(o.clone(), "hx-1");

        w.await_line(line);
        assert!(
            !run.finished_within(std::time::Duration::from_millis(30)),
            "{app}: the Ticket did not hold"
        );
        let stuck: Vec<String> = w
            .lines()
            .into_iter()
            .filter(|l| l.contains("stuck"))
            .collect();
        assert!(stuck.is_empty(), "{app}: a limit Woke: {stuck:?}");
        assert!(
            typesafe.requests().is_empty(),
            "{app}: a Judgment was asked"
        );
        let ts = o.ticket("hx-1");
        assert!(
            ts.limited == app && !ts.nudged && ts.waits == 0 && !ts.retried,
            "{app}: {ts:?}"
        );
        assert_eq!(o.state.lock().unwrap().limits[app], reset);
    }
}

#[test]
fn a_limited_app_holds_every_tickets_next_stage_on_it_while_the_other_app_runs() {
    let tickets = ["hx-1", "hx-2", "hx-3"].map(BdTicket::new).to_vec();
    let (w, mut o) = new_world(tickets);
    let clock = clock(&mut o);
    hits(&w, "hx-1", "implement", "idle", CLAUDE);
    let o = Arc::new(o);
    let _one = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 claude session limit until 3:45pm");
    // hx-3 is past Implement: its Review runs on codex
    write_file(&o.run_dir("hx-3").join("implement.md"), "STATUS: done\n");
    let _two = spawn_ticket(o.clone(), "hx-2");
    let _three = spawn_ticket(o.clone(), "hx-3");

    w.await_line("hx-3 review 1 started: codex");
    wait_until("hx-2 and hx-3 held on claude", || {
        o.ticket("hx-2").limited == "claude" && o.ticket("hx-3").limited == "claude"
    });
    thread::sleep(std::time::Duration::from_millis(20));
    for agent in ["h-hx-2-implement", "h-hx-3-debate"] {
        let started = w.called(&format!("herdr agent start {agent} "));
        assert!(
            started.is_empty(),
            "{agent} started while claude was limited"
        );
    }

    *clock.lock().unwrap() = at(25, 15, 47);
    w.await_line("hx-2 implement started: claude");
    w.await_line("hx-3 debate 1 started: claude");
    w.await_line("hx-1 claude session limit over: implement carries on (pane 1-1)");
}

#[test]
fn the_deadline_holds_off_and_at_the_reset_plus_two_minutes_an_idle_pane_gets_continue() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.timeout = Some(std::time::Duration::from_millis(5));
    let clock = clock(&mut o);
    hits(&w, "hx-1", "implement", "idle", CLAUDE);
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 claude session limit until 3:45pm: implement holds (pane 1-1)");
    let pane = o.ticket("hx-1").panes["implement"].clone();
    let go_on = format!("herdr agent prompt {pane} continue");

    *clock.lock().unwrap() = at(25, 15, 46);
    thread::sleep(std::time::Duration::from_millis(30));
    assert!(
        w.called(&go_on).is_empty(),
        "continue before the reset + 2 minutes"
    );
    let lines = w.lines();
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("timed out") || l.contains("stuck")),
        "the deadline fired during the hold: {lines:?}"
    );

    *clock.lock().unwrap() = at(25, 15, 47);
    run.wait();
    assert_eq!(w.called(&go_on).len(), 1);
    w.await_line("hx-1 claude session limit over: implement carries on (pane 1-1)");
    w.await_line("hx-1 PR #hx-1 opened");
    assert!(o.ticket("hx-1").limited.is_empty());
}

#[test]
fn an_old_limit_line_or_a_past_reset_still_wakes() {
    for tail in [
        format!("{CLAUDE}\n{}", "Ran the tests: 12 passed.\n".repeat(20)),
        "You've hit your session limit · resets Sep 24, 3pm".to_string(),
    ] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        clock(&mut o);
        hits(&w, "hx-1", "implement", "idle", &tail);
        let _run = spawn_ticket(Arc::new(o), "hx-1");
        w.await_line("hx-1 stuck in implement: went idle without a result (pane 1-1)");
    }
}

#[test]
fn a_long_limit_saves_the_ids_closes_the_tabs_and_ends_the_run_and_continue_resumes_by_id() {
    // (the pane's status, its tail, the line said)
    let cases = [
        (
            "idle",
            "You've hit your weekly limit · resets Mon 12:00am",
            "claude weekly limit until Mon 12:00am: sessions saved, panes closed, /continue after the reset",
        ),
        (
            "blocked",
            "You've hit your Opus limit · resets 3:45pm (Europe/Bucharest)\n\n What do you want to do?\n\n ❯ 1. Stop and wait for limit to reset\n",
            "claude Opus limit until 3:45pm: sessions saved, panes closed, /continue after the reset",
        ),
    ];
    for (status, tail, line) in cases {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        w.lock().integration = true;
        let clock = clock(&mut o);
        hits(&w, "hx-1", "implement", status, tail);
        let o = Arc::new(o);
        spawn_ticket(o.clone(), "hx-1").wait(); // it ends by itself

        w.await_line(line);
        assert!(o.stopping(), "{status}: the run did not end");
        assert!(w.lock().tabs.is_empty(), "{status}: the Ticket tab stayed");
        assert_eq!(w.called("herdr tab close ").len(), 1);
        let ts = o.ticket("hx-1");
        let id = ts.sessions["implement"].id.clone();
        assert!(
            ts.tab.is_empty() && ts.panes.is_empty() && !id.is_empty(),
            "{status}: {ts:?}"
        );
        let reset = o.state.lock().unwrap().limits["claude"];
        assert_eq!(load_state(&w.repo).unwrap().limits["claude"], reset);
        if status == "blocked" {
            continue;
        }

        // /continue before the reset holds; after it, resumes by id.
        let o = restarted(&w, &o);
        let before = w.calls().len();
        let mut run = spawn_ticket(o.clone(), "hx-1");
        wait_until("hx-1 held", || o.ticket("hx-1").limited == "claude");
        thread::sleep(std::time::Duration::from_millis(20));
        assert!(w.since(before, "herdr agent start ").is_empty());
        *clock.lock().unwrap() = at(28, 0, 2);
        run.wait();
        w.await_line("hx-1 implement resumed: claude (pane");
        w.await_line("hx-1 PR #hx-1 opened");
        let starts = w.since(before, "herdr agent start h-hx-1-implement ");
        assert!(
            starts.len() == 1 && starts[0].contains(&format!("--resume {id} ")),
            "{starts:?}"
        );
    }
}

#[test]
fn a_tab_that_would_not_close_at_a_long_limit_keeps_its_ids_and_continue_watches_it() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let clock = clock(&mut o);
    hits(
        &w,
        "hx-1",
        "implement",
        "idle",
        "You've hit your weekly limit · resets Mon 12:00am",
    );
    w.fail_once("herdr tab close ", "herdr is busy");
    let o = Arc::new(o);
    spawn_ticket(o.clone(), "hx-1").wait();
    let ts = o.ticket("hx-1");
    assert!(
        !ts.tab.is_empty() && ts.panes.contains_key("implement"),
        "{ts:?}"
    );

    // /continue after the reset watches the still-live pane: no second tab
    *clock.lock().unwrap() = at(28, 0, 2);
    let o = restarted(&w, &o);
    let before = w.calls().len();
    let _run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 stuck in implement: went idle without a result (pane 1-1)");
    assert!(w.since(before, "herdr tab create ").is_empty());
    assert!(w.since(before, "herdr agent start ").is_empty());
}

#[test]
fn a_limit_line_past_its_saved_reset_is_not_read_as_tomorrows() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let clock = clock(&mut o);
    hits(&w, "hx-1", "implement", "idle", CLAUDE);
    let o = Arc::new(o);
    let run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 claude session limit until 3:45pm: implement holds (pane 1-1)");
    drop(run); // /stop-work during the hold, the pane left as it was

    // /continue after the reset, in a new process: the pane still shows
    // "resets 3:45pm", which is today's, now past, not tomorrow's
    *clock.lock().unwrap() = at(25, 15, 50);
    let o = restarted(&w, &o);
    let _run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 stuck in implement: went idle without a result (pane 1-1)");
    assert!(o.ticket("hx-1").limited.is_empty());
}

#[test]
fn a_dated_limit_at_yesterdays_reset_time_is_a_new_limit() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let clock = clock(&mut o);
    *clock.lock().unwrap() = at(24, 14, 0);
    hits(&w, "hx-1", "implement", "idle", CLAUDE);
    let o = Arc::new(o);
    let run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 claude session limit until 3:45pm: implement holds (pane 1-1)");
    drop(run);

    // a day on, the same session hits a limit that names its day
    *clock.lock().unwrap() = at(25, 14, 0);
    let pane = o.ticket("hx-1").panes["implement"].clone();
    w.lock().tails.insert(
        pane,
        "You've hit your session limit · resets Sep 25, 3:45pm".to_string(),
    );
    let o = restarted(&w, &o);
    let _run = spawn_ticket(o.clone(), "hx-1");
    wait_until("claude limited until today's reset", || {
        o.state.lock().unwrap().limits["claude"] == at(25, 15, 45)
    });
    assert!(!w.lines().iter().any(|l| l.contains("stuck")));
}

#[test]
fn a_long_limits_line_replayed_after_its_reset_is_no_new_limit() {
    let weekly = "You've hit your weekly limit · resets Mon 12:00am";
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().integration = true;
    let clock = clock(&mut o);
    // the resumed session shows the weekly line again and stops
    let world = w.clone();
    w.session(move |p| {
        if p.stage == "implement" || p.text == "continue" {
            world
                .lock()
                .tails
                .insert(p.pane.clone(), weekly.to_string());
            return (String::new(), "idle".to_string());
        }
        succeed(p)
    });
    let o = Arc::new(o);
    spawn_ticket(o.clone(), "hx-1").wait();

    *clock.lock().unwrap() = at(28, 0, 2);
    let o = restarted(&w, &o);
    let _run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 implement resumed: claude (pane");
    w.await_line("hx-1 stuck in implement: went idle without a result");
    let ended: Vec<String> = w
        .lines()
        .into_iter()
        .filter(|l| l.contains("weekly limit until"))
        .collect();
    assert_eq!(
        ended.len(),
        1,
        "the old line ended the run again: {ended:?}"
    );
}

#[test]
fn a_limit_with_no_reset_is_looked_at_again_after_an_hour() {
    let later = "■ You’ve hit your usage limit. Try again later.";
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let typesafe = TypeSafeFake::down();
    o.cfg.typesafe = typesafe.clone();
    let clock = clock(&mut o);
    write_file(&o.run_dir("hx-1").join("implement.md"), "STATUS: done\n");
    let world = w.clone();
    w.session(move |p| {
        if p.stage == "review" {
            world.lock().tails.insert(p.pane.clone(), later.to_string());
        }
        (String::new(), "idle".to_string()) // continue changes nothing
    });
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 codex usage limit until 3:00pm: review 1 holds (pane 1-1)");

    *clock.lock().unwrap() = at(25, 15, 2);
    w.await_line("hx-1 codex usage limit until 4:02pm: review 1 holds (pane 1-1)");
    let stuck: Vec<String> = w
        .lines()
        .into_iter()
        .filter(|l| l.contains("stuck"))
        .collect();
    assert!(stuck.is_empty(), "a limit Woke: {stuck:?}");
    assert!(typesafe.requests().is_empty(), "a Judgment was asked");
}

/// The Questions the Orchestrator put about a limit on the Review.
fn review_questions(w: &World) -> Vec<(String, Option<String>)> {
    w.events()
        .into_iter()
        .filter_map(|e| match e.ask {
            Some(Ask::Limited { app, fallback }) => Some((app, fallback)),
            _ => None,
        })
        .collect()
}

/// The prompt Ticket `ticket`'s `file` Stage was started with.
fn prompt_for(w: &World, ticket: &str, file: &str) -> String {
    let file = format!("runs/{ticket}/{file}\n");
    w.called("herdr agent prompt")
        .into_iter()
        .find(|call| call.contains(&file))
        .unwrap_or_else(|| panic!("no prompt wrote {file}"))
}

#[test]
fn a_codex_limit_on_a_review_asks_once_and_unreviewed_skips_review_and_debate_until_the_reset() {
    let tickets = ["hx-1", "hx-2", "hx-3"].map(BdTicket::new).to_vec();
    let (w, mut o) = new_world(tickets);
    let clock = clock(&mut o);
    for ticket in ["hx-1", "hx-2", "hx-3"] {
        write_file(&o.run_dir(ticket).join("implement.md"), "STATUS: done\n");
    }
    hits(&w, "hx-1", "review", "idle", CODEX);
    let o = Arc::new(o);
    let _one = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 codex usage limit until 3:05pm: review 1 holds (pane 1-1)");
    // hx-2 reaches its Review meanwhile: it holds, and asks nothing
    let _two = spawn_ticket(o.clone(), "hx-2");
    wait_until("hx-2 held before its Review", || {
        o.ticket("hx-2").limited == "codex"
    });
    thread::sleep(std::time::Duration::from_millis(20));
    assert!(w.called("herdr agent start h-hx-2-review").is_empty());
    assert_eq!(review_questions(&w), [("codex".to_string(), None)]);

    o.review("codex", Review::Unreviewed);
    w.await_line("hx-1 review 1 and debate 1 skipped: codex was limited until 3:05pm");
    // Kept as any result, so /continue after the reset skips it too.
    assert_eq!(
        fs::read_to_string(o.run_dir("hx-1").join("review-1.md")).unwrap(),
        "STATUS: done\nUNREVIEWED: codex was limited until 3:05pm\n"
    );
    w.await_line("hx-1 PR #hx-1 opened");
    w.await_line("hx-2 PR #hx-2 opened");
    for ticket in ["hx-1", "hx-2"] {
        assert!(w
            .called(&format!("herdr agent start h-{ticket}-debate"))
            .is_empty());
        let fix = prompt_for(&w, ticket, "fix-1.md");
        for input in [
            "- Open PR: yes\n",
            "- Fix items: none\n",
            "- Unreviewed: codex was limited until 3:05pm\n",
        ] {
            assert!(fix.contains(input), "{ticket}: {input:?} not in {fix}");
        }
    }
    assert_eq!(w.called("herdr agent start h-hx-1-review").len(), 1);
    assert!(w.called("herdr agent start h-hx-2-review").is_empty());
    assert_eq!(review_questions(&w).len(), 1, "asked again");
    let fix = SKILLS
        .iter()
        .find(|(name, _)| *name == "stage-fix")
        .unwrap()
        .1;
    assert!(
        fix.contains("**Unreviewed**"),
        "stage-fix has no Unreviewed rule"
    );

    // After the reset Reviews start on codex again.
    *clock.lock().unwrap() = at(25, 15, 7);
    let _three = spawn_ticket(o.clone(), "hx-3");
    w.await_line("hx-3 review 1 started: codex");
}

#[test]
fn review_with_the_fallback_starts_stage_review_on_its_app_and_model() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    clock(&mut o);
    write_file(
        &w.repo.join(".harness/config.json"),
        r#"{"review_if_limited": {"app": "claude", "model": "opus"}}"#,
    );
    write_file(&o.run_dir("hx-1").join("implement.md"), "STATUS: done\n");
    // the codex Review stops at its limit; every session after it is done
    let (world, first) = (w.clone(), AtomicBool::new(true));
    w.session(move |p| {
        if p.stage == "review" && first.swap(false, SeqCst) {
            world.lock().tails.insert(p.pane.clone(), CODEX.to_string());
            return (String::new(), "idle".to_string());
        }
        succeed(p)
    });
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");
    wait_until("the Review's limit Question", || {
        !review_questions(&w).is_empty()
    });
    assert_eq!(
        review_questions(&w),
        [("codex".to_string(), Some("claude opus".to_string()))]
    );

    o.review("codex", Review::Fallback);
    w.await_line("hx-1 review 1 started: claude opus (pane");
    w.await_line("hx-1 PR #hx-1 opened");
    let starts = w.called("herdr agent start h-hx-1-review ");
    let worktree = o.worktree("hx-1").display().to_string();
    assert!(
        starts.len() == 2
            && starts[1].contains(" --kind claude ")
            && starts[1].contains(&format!(" --add-dir {worktree} "))
            && starts[1].ends_with(" --model opus"),
        "{starts:?}"
    );
    assert!(!w.called("herdr agent start h-hx-1-debate ").is_empty());
}

#[test]
fn wait_holds_later_reviews_and_carries_the_pane_on_at_the_reset() {
    let tickets = ["hx-1", "hx-2"].map(BdTicket::new).to_vec();
    let (w, mut o) = new_world(tickets);
    let clock = clock(&mut o);
    for ticket in ["hx-1", "hx-2"] {
        write_file(&o.run_dir(ticket).join("implement.md"), "STATUS: done\n");
    }
    hits(&w, "hx-1", "review", "idle", CODEX);
    let o = Arc::new(o);
    let _one = spawn_ticket(o.clone(), "hx-1");
    wait_until("the Review's limit Question", || {
        !review_questions(&w).is_empty()
    });
    o.review("codex", Review::Wait);
    let _two = spawn_ticket(o.clone(), "hx-2");
    wait_until("hx-2 held before its Review", || {
        o.ticket("hx-2").limited == "codex"
    });
    thread::sleep(std::time::Duration::from_millis(20));
    assert!(w.called("herdr agent start h-hx-2-review").is_empty());
    let pane = o.ticket("hx-1").panes["review"].clone();
    assert!(w
        .called(&format!("herdr agent prompt {pane} continue"))
        .is_empty());

    *clock.lock().unwrap() = at(25, 15, 7);
    w.await_line("hx-1 codex usage limit over: review 1 carries on (pane 1-1)");
    assert_eq!(
        w.called(&format!("herdr agent prompt {pane} continue"))
            .len(),
        1
    );
    w.await_line("hx-2 review 1 started: codex");
    w.await_line("hx-1 PR #hx-1 opened");
    w.await_line("hx-2 PR #hx-2 opened");
    assert_eq!(review_questions(&w).len(), 1);
}

#[test]
fn the_moderator_is_told_which_side_is_limited() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    clock(&mut o);
    o.state
        .lock()
        .unwrap()
        .limits
        .insert("codex".to_string(), at(25, 15, 5));
    write_file(&o.run_dir("hx-1").join("implement.md"), "STATUS: done\n");
    write_file(
        &o.run_dir("hx-1").join("review-1.md"),
        "STATUS: done\n\n## Findings\n",
    );
    o.run_ticket("hx-1");

    let inputs = prompt_for(&w, "hx-1", "verdict-1.md");
    let inputs = inputs.split_once("## Inputs").unwrap().1;
    assert!(
        inputs.contains("- Side B: limited until 3:05pm\n") && !inputs.contains("- Side A: "),
        "{inputs}"
    );
    let moderate = SKILLS
        .iter()
        .find(|(name, _)| *name == "stage-moderate")
        .unwrap()
        .1;
    assert!(
        moderate.contains("Side B: limited until"),
        "stage-moderate has no rule for a limited side"
    );
}

#[test]
fn a_fallback_review_at_its_own_limit_holds_and_asks_nothing_more() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    clock(&mut o);
    write_file(
        &w.repo.join(".harness/config.json"),
        r#"{"review_if_limited": {"app": "claude", "model": "opus"}}"#,
    );
    write_file(&o.run_dir("hx-1").join("implement.md"), "STATUS: done\n");
    // codex's Review stops at its limit, then the fallback's at claude's
    let (world, reviews) = (w.clone(), AtomicUsize::new(0));
    w.session(move |p| {
        if p.stage == "review" {
            let tail = match reviews.fetch_add(1, SeqCst) {
                0 => CODEX,
                _ => CLAUDE,
            };
            world.lock().tails.insert(p.pane.clone(), tail.to_string());
            return (String::new(), "idle".to_string());
        }
        succeed(p)
    });
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");
    wait_until("the Review's limit Question", || {
        !review_questions(&w).is_empty()
    });
    o.review("codex", Review::Fallback);
    w.await_line("hx-1 claude session limit until 3:45pm: review 1 holds (pane");
    wait_until("hx-1 held on claude", || {
        o.ticket("hx-1").limited == "claude"
    });
    thread::sleep(std::time::Duration::from_millis(20));
    assert_eq!(review_questions(&w).len(), 1, "a second Question");
}

#[test]
fn continue_with_the_question_unanswered_asks_it_again_rather_than_wait() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().integration = true;
    clock(&mut o);
    write_file(&o.run_dir("hx-1").join("implement.md"), "STATUS: done\n");
    hits(&w, "hx-1", "review", "idle", CODEX);
    let o = Arc::new(o);
    let run = spawn_ticket(o.clone(), "hx-1");
    wait_until("the Review's limit Question", || {
        !review_questions(&w).is_empty()
    });
    drop(run); // /stop-work, the Question unanswered
               // its pane gone since, its session id saved
    let ts = o.ticket("hx-1");
    assert!(!ts.sessions["review"].id.is_empty(), "{ts:?}");
    w.run(&w.repo, &["herdr", "pane", "close", &ts.panes["review"]])
        .unwrap();

    let o = restarted(&w, &o);
    let _run = spawn_ticket(o.clone(), "hx-1");
    wait_until("the Question asked again", || {
        review_questions(&w).len() == 2
    });
    o.review("codex", Review::Unreviewed);
    w.await_line("hx-1 PR #hx-1 opened");
}
