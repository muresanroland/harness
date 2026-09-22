use super::stage::{result_name, Config, Orchestrator, DEBATE, FIX, IMPLEMENT};
use super::state::load_state;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use std::sync::atomic::Ordering;

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

#[test]
fn report_keeps_undelivered_lines_and_gives_up_on_a_shell_pane() {
    let repo = TempDir::new();
    let home = TempDir::new();
    let blocked = Fake::new(|_, _| Err("agent_blocked".to_string()));
    let o = Orchestrator::with_state(
        Config::for_tests(blocked.clone(), repo.path(), home.path()),
        Default::default(),
    );
    o.report("one");
    o.report("two");
    assert_eq!(
        blocked.called("herdr agent prompt main").len(),
        2,
        "{:?}",
        blocked.calls()
    );
    assert_eq!(
        o.unsent.lock().unwrap().lines,
        ["[harness] one", "[harness] two"]
    );

    let shell = Fake::new(|_, _| Err("agent_not_found".to_string()));
    let o = Orchestrator::with_state(
        Config::for_tests(shell.clone(), repo.path(), home.path()),
        Default::default(),
    );
    o.report("one");
    o.report("two");
    assert_eq!(shell.calls(), ["herdr agent prompt main [harness] one"]);
    assert!(o.unsent.lock().unwrap().lines.is_empty());
}
