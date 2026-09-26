use super::shell_test::{find, pick, render, screen_at, type_line};
use crate::orchestrator::stage::Ask;
use crate::shell::About;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;

/// /demo plays the whole script: every kind of Question asks and holds it
/// until answered, a park skips the rest of that Ticket, MERGE TO UNBLOCK
/// and LIMITED show, the summary opens at the end, and the Shell is put
/// back as it was with nothing written to the log.
#[test]
fn the_demo_plays_a_run_asks_and_puts_the_shell_back() {
    let repo = TempDir::new();
    let mut s = screen_at(Fake::quiet(), repo.path());
    let (epics, state) = (s.epics[0].id.clone(), s.state.clone());
    type_line(&mut s, "/demo");
    assert!(s.running && s.demo.is_some());
    type_line(&mut s, "/start-epic harness-kqe");
    assert!(s.run.is_none(), "a run starts over the demo");
    s.tick();
    type_line(&mut s, "/demo");
    assert_eq!(
        s.demo.as_ref().unwrap().step,
        1,
        "the demo starts over itself"
    );
    let (mut asked, mut boxes) = (Vec::new(), Vec::new());
    for _ in 0..10_000 {
        if s.demo.is_none() {
            break;
        }
        s.tick();
        let buf = render(&s, 160, 48);
        for title in ["MERGE TO UNBLOCK", "LIMITED", "NEEDS YOU"] {
            if find(&buf, title).is_some() && !boxes.contains(&title) {
                boxes.push(title);
            }
        }
        if !s.showing() {
            continue;
        }
        let About::Asked(ask) = &s.questions[0].about else {
            panic!("a confirmation in the demo");
        };
        let (kind, option) = match ask {
            Ask::Plan { .. } => ("plan", 1),              // approve
            Ask::StageQuestion { .. } => ("question", 1), // its first option
            Ask::Limited { .. } => ("limited", 1),        // wait for the reset
            Ask::Blocked { .. } => ("blocked", 2),        // park
            Ask::Wake { .. } => ("wake", 1),              // nudge
            Ask::PlanFailed { .. } => panic!("the demo asks no plan failure"),
        };
        asked.push(kind);
        pick(&mut s, option);
    }
    assert_eq!(asked, ["plan", "question", "limited", "blocked", "wake"]);
    assert_eq!(boxes.len(), 3, "{boxes:?}");
    let texts: Vec<&str> = s.events.iter().map(|e| e.text.as_str()).collect();
    assert!(texts.is_empty(), "RECENT is put back: {texts:?}");
    let summary = s.summary.as_ref().expect("the summary opens at the end");
    assert_eq!(summary.epic, "orqa-demo");
    let parked: Vec<_> = summary
        .tickets
        .iter()
        .map(|t| t.parked.as_deref())
        .collect();
    assert_eq!(parked, [None, None, None, Some("by you at implement")]);
    let prs: Vec<_> = summary.tickets.iter().map(|t| t.pr.as_str()).collect();
    assert!(prs[..3].iter().all(|pr| !pr.is_empty()), "{prs:?}");
    assert!(!s.running && s.questions.is_empty());
    assert_eq!((s.epics[0].id.clone(), s.state.clone()), (epics, state));
    assert!(!repo.path().join(".orqadence/orchestrator.log").exists());
}

/// /stop-demo, and /stop-work too, ends the demo mid-Question; with no
/// demo on it says so.
#[test]
fn stop_demo_ends_the_demo_where_it_stands() {
    for stop in ["/stop-demo", "/stop-work"] {
        let mut s = screen_at(Fake::quiet(), TempDir::new().path());
        type_line(&mut s, "/demo");
        while !s.showing() {
            s.tick();
        }
        type_line(&mut s, stop);
        assert!(s.demo.is_none() && !s.running && s.questions.is_empty());
        assert_eq!(s.epics[0].id, "harness-kqe");
        assert!(s.summary.is_none());
        type_line(&mut s, "/stop-demo");
        assert_eq!(s.notice.as_ref().unwrap().0, "no demo is running");
    }
}
