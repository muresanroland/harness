use super::stage::{result_name, Config, Orchestrator, DEBATE, FIX, IMPLEMENT};
use super::state::load_state;
use super::world::LogBuf;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use std::sync::atomic::Ordering;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

/// Every event goes through emit: a timestamped log line with the bd id (none
/// for a run-level line) and an Event to the receiver, without a tool call.
#[test]
fn emit_writes_the_log_line_and_the_event() {
    let repo = TempDir::new();
    let home = TempDir::new();
    let pane = Fake::new(|_, _| Ok("{}".to_string()));
    let mut o = Orchestrator::with_state(
        Config::for_tests(pane.clone(), repo.path(), home.path()),
        Default::default(),
    );
    let log = Arc::new(Mutex::new(String::new()));
    o.cfg.log = Arc::new(Mutex::new(Box::new(LogBuf(log.clone()))));
    let (events, received) = channel();
    o.cfg.events = events;

    o.report("hx-1", "implemented");
    o.log("hx-1", "implement prompted, waiting for implement.md");
    o.report("", "stopped, panes left running, /continue resumes");
    o.emit(
        "hx-1",
        "PR #7 opened after 1 round",
        true,
        "https://example.test/pr/7",
    );

    let log = log.lock().unwrap();
    let lines: Vec<&str> = log.lines().collect();
    let stamped = |line: &str| {
        // 'YYYY-MM-DD HH:MM:SS ' in local time, then the id and the event
        line.len() > 20
            && chrono::NaiveDateTime::parse_from_str(&line[..19], "%Y-%m-%d %H:%M:%S").is_ok()
            && &line[19..20] == " "
    };
    assert!(lines.iter().all(|l| stamped(l)), "log:\n{log}");
    assert_eq!(
        lines.iter().map(|l| &l[20..]).collect::<Vec<_>>(),
        [
            "hx-1 implemented",
            "hx-1 implement prompted, waiting for implement.md",
            "stopped, panes left running, /continue resumes",
            "hx-1 PR #7 opened after 1 round (https://example.test/pr/7)",
        ],
        "log:\n{log}"
    );
    assert!(
        pane.calls().is_empty(),
        "an event ran a tool: {:?}",
        pane.calls()
    );
    let events: Vec<_> = received.try_iter().collect();
    assert!(
        events.iter().all(|e| e.time <= chrono::Local::now()),
        "an Event carries the moment it was said: {events:?}"
    );
    let got: Vec<(Option<String>, String, bool)> = events
        .into_iter()
        .map(|e| (e.ticket, e.text, e.panel))
        .collect();
    assert_eq!(
        got,
        [
            (Some("hx-1".to_string()), "implemented".to_string(), true),
            (
                Some("hx-1".to_string()),
                "implement prompted, waiting for implement.md".to_string(),
                false
            ),
            (
                None,
                "stopped, panes left running, /continue resumes".to_string(),
                true
            ),
            (
                Some("hx-1".to_string()),
                "PR #7 opened after 1 round".to_string(),
                true
            ),
        ]
    );
}

#[test]
fn state_that_cannot_be_saved_is_said_on_the_panel() {
    let repo = TempDir::new();
    let home = TempDir::new();
    let mut o = Orchestrator::with_state(
        Config::for_tests(Fake::quiet(), repo.path(), home.path()),
        Default::default(),
    );
    let (events, received) = channel();
    o.cfg.events = events;
    std::fs::create_dir_all(repo.path().join(".harness/state.json")).unwrap(); // a directory in the file's place
    o.update("hx-1", |_| {});
    let got: Vec<_> = received.try_iter().collect();
    assert!(
        got.len() == 1 && got[0].panel && got[0].text.starts_with("state not saved: "),
        "{got:?}"
    );
}

#[test]
fn update_saves_the_state_file_and_ticket_snapshots_it() {
    let repo = TempDir::new();
    let home = TempDir::new();
    let o = Orchestrator::new(Config::for_tests(Fake::quiet(), repo.path(), home.path())).unwrap();
    assert_eq!(
        o.ticket("hx-1").status,
        "",
        "an unknown Ticket is the default"
    );
    o.update("hx-1", |ts| {
        ts.stage = "review".to_string();
        ts.round = 1;
    });
    let ts = o.ticket("hx-1");
    assert!(
        ts.status == "running" && ts.stage == "review" && ts.round == 1,
        "{ts:?}"
    );
    assert_eq!(load_state(repo.path()).unwrap().tickets["hx-1"], ts);
    assert_eq!(o.run_dir("hx-1"), repo.path().join(".harness/runs/hx-1"));
    assert_eq!(
        o.worktree("hx-1"),
        repo.path().join(".harness/worktrees/hx-1")
    );
    assert!(!o.stopping());
    o.stop.store(true, Ordering::SeqCst);
    assert!(o.stopping());
    assert_eq!(result_name(&DEBATE, 2), "verdict-2.md");
    assert_eq!(result_name(&FIX, 1), "fix-1.md");
    assert_eq!(result_name(&IMPLEMENT, 1), "implement.md");
}
