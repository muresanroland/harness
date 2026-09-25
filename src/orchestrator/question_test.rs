//! A Stage's own question, STATUS: question: put to you and never judged,
//! or with Away on, its Ticket parked with a bd comment and its pane open.

use super::judgment::fake::Fake;
use super::judgment::Action;
use super::result::ResultRequirements;
use super::stage::{Answer, Ask, ADDRESS, AWAY};
use super::state::STATUS_PARKED;
use super::world::{new_world, spawn_ticket, succeed, BdTicket};
use super::write_file;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// What a Stage writes to ask, then waits in its session.
pub(crate) const ASKS: &str = "STATUS: question\n\nWhich parser stays?\n- ours\n- theirs\n";

#[test]
fn a_stage_question_is_put_to_you_never_judged_and_the_deadline_waits() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let typesafe = Fake::down();
    o.cfg.typesafe = typesafe.clone();
    o.cfg.timeout = Some(Duration::from_millis(100));
    w.session(|p| match p.text.as_str() {
        "ours" => (String::new(), "working".to_string()), // the answer taken up
        _ if p.stage == "implement" => (ASKS.to_string(), "idle".to_string()),
        _ => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let asked = w.await_event("question in implement");
    assert_eq!(asked.text, "question in implement (pane 1-1)");
    let Some(Ask::StageQuestion {
        pane,
        question,
        options,
    }) = asked.ask
    else {
        panic!("a question raised {:?}, not a Question", asked.ask);
    };
    assert_eq!(question, "Which parser stays?");
    assert_eq!(options, ["ours", "theirs"]);
    thread::sleep(Duration::from_millis(200)); // twice the Stage's deadline

    o.answer("hx-1", &pane, Answer::Prompt("ours".to_string()));
    w.await_line("hx-1 sent your answer");
    // Working on the answer, the session has a whole deadline again.
    write_file(&o.run_dir("hx-1").join("implement.md"), "STATUS: done\n");
    w.lock().agents.insert(pane.clone(), "idle".to_string());
    run.wait();

    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(
        w.called(&format!("herdr agent prompt {pane} ")).last(),
        Some(&format!("herdr agent prompt {pane} ours")),
        "the answer went into the pane as a prompt"
    );
    let lines = w.lines();
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("timed out") || l.contains("stuck in")),
        "a question woke the Ticket: {lines:#?}"
    );
    assert!(
        typesafe.requests().is_empty(),
        "a Judgment was asked about a question"
    );
}

#[test]
fn an_answered_question_left_unwritten_is_no_result_not_asked_again() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.typesafe = Fake::down();
    w.session(|p| match p.text.as_str() {
        "ours" => (String::new(), "idle".to_string()), // idle, the file not rewritten
        _ if p.stage == "implement" => (ASKS.to_string(), "idle".to_string()),
        _ => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let Some(Ask::StageQuestion { pane, .. }) = w.await_event("question in implement").ask else {
        panic!("no Question raised");
    };
    o.answer("hx-1", &pane, Answer::Prompt("ours".to_string()));
    w.await_line("hx-1 stuck in implement: went idle without a result");
    o.answer("hx-1", &pane, Answer::Act(Action::Park));
    run.wait();

    let asked = w
        .lines()
        .iter()
        .filter(|l| l.contains("question in"))
        .count();
    assert_eq!(asked, 1, "the answered question was asked again");
}

#[test]
fn a_question_answered_in_the_pane_left_unwritten_is_no_result_not_asked_again() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.typesafe = Fake::down();
    w.session(|p| match p.stage == "implement" {
        true => (ASKS.to_string(), "idle".to_string()),
        false => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let Some(Ask::StageQuestion { pane, .. }) = w.await_event("question in implement").ask else {
        panic!("no Question raised");
    };
    // answered by typing into the pane, then idle, the file not rewritten
    w.lock().agents.insert(pane.clone(), "working".to_string());
    w.await_line("hx-1 carrying on");
    w.lock().agents.insert(pane.clone(), "idle".to_string());
    w.await_line("hx-1 stuck in implement: went idle without a result");
    o.answer("hx-1", &pane, Answer::Act(Action::Park));
    run.wait();

    let asked = w
        .lines()
        .iter()
        .filter(|l| l.contains("question in"))
        .count();
    assert_eq!(
        asked, 1,
        "the question answered in the pane was asked again"
    );
}

#[test]
fn a_question_written_while_its_status_is_read_is_kept() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.typesafe = Fake::down();
    w.session(|p| match p.stage == "implement" {
        true => (ASKS.to_string(), "idle".to_string()),
        false => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let Some(Ask::StageQuestion { pane, .. }) = w.await_event("question in implement").ask else {
        panic!("no Question raised");
    };
    // answered in the pane, the session asks anew while its status is read
    let file = o.run_dir("hx-1").join("implement.md");
    let (get, armed) = (format!("herdr agent get {pane}"), AtomicBool::new(true));
    let at = pane.clone();
    w.hook(move |_, argv| {
        if argv.join(" ") != get || !armed.swap(false, Ordering::SeqCst) {
            return None;
        }
        write_file(&file, "STATUS: question\n\nWhich lexer stays?\n- ours\n");
        let agent = serde_json::json!({ "agent_status": "working", "pane_id": at });
        Some(Ok(
            serde_json::json!({ "result": { "agent": agent } }).to_string()
        ))
    });
    let asked = w.await_nth("question in implement", 2);
    let Some(Ask::StageQuestion { question, .. }) = asked.ask else {
        panic!("a question raised {:?}, not a Question", asked.ask);
    };
    assert_eq!(question, "Which lexer stays?");
    o.answer("hx-1", &pane, Answer::Act(Action::Park));
    run.wait();
}

#[test]
fn a_question_written_after_a_wake_is_put_to_you() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.typesafe = Fake::down();
    w.session(|p| match p.stage == "implement" {
        true => (String::new(), "idle".to_string()), // no result: a Wake
        false => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let woke = w.await_event("stuck in implement: went idle without a result");
    let Some(Ask::Wake { pane, .. }) = woke.ask else {
        panic!("no Wake raised: {:?}", woke.ask);
    };
    // taken up again in its pane, the session asks
    write_file(&o.run_dir("hx-1").join("implement.md"), ASKS);
    let asked = w.await_event("question in implement");
    let Some(Ask::StageQuestion { question, .. }) = asked.ask else {
        panic!("a question raised {:?}, not a Question", asked.ask);
    };
    assert_eq!(question, "Which parser stays?");
    o.answer("hx-1", &pane, Answer::Act(Action::Park));
    run.wait();
}

#[test]
fn away_parks_a_question_with_a_bd_comment_and_the_pane_open() {
    for (stage, label) in [("implement", "implement"), ("review", "review 1")] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        o.cfg.away.store(true, Ordering::SeqCst);
        w.session(move |p| match p.stage == stage {
            true => (ASKS.to_string(), "idle".to_string()),
            false => succeed(p),
        });
        let o = Arc::new(o);
        spawn_ticket(o.clone(), "hx-1").wait();

        w.await_line("hx-1 parked: asked you while away");
        let ts = o.ticket("hx-1");
        assert_eq!(
            (ts.status.as_str(), ts.reason.as_str()),
            (STATUS_PARKED, AWAY)
        );
        let comments = w.called("bd comments add hx-1 ");
        assert!(
            comments.len() == 1
                && comments[0].contains(&format!("{label} asked"))
                && comments[0].contains("/continue @hx-1")
                && comments[0].contains("Which parser stays?"),
            "{label}: bd comments = {comments:?}"
        );
        assert!(
            w.called("herdr pane close").is_empty(),
            "{label}: the asking session's pane was closed"
        );
        assert_eq!(w.lock().agents[&ts.panes[stage]], "idle");
        assert!(
            w.events().iter().all(|e| e.ask.is_none()),
            "{label}: Away still put a Question"
        );
    }
}

/// Address runs on your command over a Ticket with its PR open, which has
/// no Parked to go to: Away, its question still waits as a Question.
#[test]
fn away_leaves_an_address_question_waiting_for_you() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.away.store(true, Ordering::SeqCst);
    w.session(|p| match p.text.as_str() {
        "ours" => (String::new(), "working".to_string()),
        _ => (ASKS.to_string(), "idle".to_string()),
    });
    let o = Arc::new(o);
    let run = {
        let o = o.clone();
        thread::spawn(move || o.run_stage("hx-1", &ADDRESS, 0, &[], ResultRequirements::default()))
    };
    let asked = w.await_event("question in address");
    let Some(Ask::StageQuestion { pane, .. }) = asked.ask else {
        panic!("Away parked an address question: {:?}", asked.ask);
    };
    o.answer("hx-1", &pane, Answer::Prompt("ours".to_string()));
    w.await_line("hx-1 sent your answer");
    write_file(&o.run_dir("hx-1").join("address.md"), "STATUS: done\n");
    w.lock().agents.insert(pane, "idle".to_string());
    assert!(run.join().unwrap().is_ok());
    assert!(w.called("bd comments add").is_empty());
}
