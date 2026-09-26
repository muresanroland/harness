use super::judgment::fake::Fake;
use super::judgment::{offered, plan_request, request, Action, PlanJudged, PLAN_FLOOR};
use super::stage::{Answer, Ask, Config, Orchestrator};
use super::state::{load_state, State, TicketState, STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};
use super::world::{new_world, spawn_ticket, succeed, working, BdTicket, World};
use crate::tempdir::TempDir;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use Action::*;

/// The actions a request offers, as its criteria name them.
pub(crate) fn criteria(body: &Value) -> Vec<String> {
    body["questions"]["action"]["criteria"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

/// TypeSafe's reply choosing `choice` at `confidence`: the choice scores
/// the confidence, the other offered actions share what is left.
pub(crate) fn choose(body: &Value, choice: &str, confidence: f64) -> Value {
    let offered = criteria(body);
    assert!(
        offered.iter().any(|a| a == choice),
        "{choice} is not offered: {offered:?}"
    );
    let rest = (1.0 - confidence) / (offered.len() - 1).max(1) as f64;
    let scores: Vec<(&str, f64)> = offered
        .iter()
        .map(|a| (a.as_str(), if a == choice { confidence } else { rest }))
        .collect();
    reply(choice, json!(confidence), &scores)
}

fn reply(choice: &str, confidence: Value, probabilities: &[(&str, f64)]) -> Value {
    let probabilities: serde_json::Map<String, Value> = probabilities
        .iter()
        .map(|(a, p)| (a.to_string(), json!(p)))
        .collect();
    json!({ "answers": { "action": {
        "type": "choice",
        "choice": choice,
        "confidence": confidence,
        "probabilities": probabilities,
    } } })
}

/// A TypeSafe that answers its requests in turn from `answer`, by number.
fn typesafe(answer: impl Fn(usize, &Value) -> Value + Send + Sync + 'static) -> Arc<Fake> {
    let n = AtomicUsize::new(0);
    Fake::new(move |body| Ok(answer(n.fetch_add(1, Ordering::SeqCst), body)))
}

/// A canned nudge's prompt over the result file.
fn prompt(action: Action, file: &Path) -> String {
    action.nudge(file).unwrap().0
}

/// The first Implement session goes idle without a result; a nudge
/// changes nothing, and a fresh session succeeds.
fn idle_once(w: &World) {
    let started = AtomicUsize::new(0);
    w.session(move |p| {
        let nudge = p.stage.is_empty();
        if nudge || (p.stage == "implement" && started.fetch_add(1, Ordering::SeqCst) == 0) {
            return (String::new(), "idle".to_string());
        }
        succeed(p)
    });
}

/// Every session goes idle without a result.
fn idle(w: &World) {
    w.session(|_| (String::new(), "idle".to_string()));
}

/// Sets every live session's herdr state.
fn set_agents(w: &World, status: &str) {
    for s in w.lock().agents.values_mut() {
        *s = status.to_string();
    }
}

/// The Wake's Question: its pane, its actions and its Judgment's line.
fn question(w: &World, n: usize) -> (String, Vec<Action>, Option<String>) {
    let wake = w.await_nth("stuck in implement", n);
    let Some(Ask::Wake {
        pane,
        actions,
        judged,
        ..
    }) = wake.ask
    else {
        panic!("the Wake asks nothing: {wake:?}");
    };
    (pane, actions, judged.map(|j| j.said()))
}

/// Every action at or above the floor acts and logs both lines, the Wake
/// raises no Question, and the judged line names the offered actions in
/// plain words, highest score first.
#[test]
fn each_action_at_the_floor_acts_and_logs_both_lines() {
    const ALL: [(&str, &str); 5] = [
        ("nudge_write_result", "nudge to write the result"),
        ("nudge_proceed", "nudge to carry on"),
        ("retry", "retry"),
        ("park", "park"),
        ("wait", "wait"),
    ];
    for (action, line2) in [
        ("nudge_write_result", "hx-1 nudged: write the result file"),
        (
            "nudge_proceed",
            "hx-1 nudged: carry on, the Ticket is the spec",
        ),
        (
            "retry",
            "hx-1 retrying implement with a fresh session (pane 1-1)",
        ),
        ("park", "hx-1 parked: implement went idle without a result"),
        ("wait", "hx-1 waiting: still working (pane 1-1)"),
    ] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        idle_once(&w);
        o.cfg.wait = Some(Duration::from_millis(5));
        let others: Vec<_> = ALL.iter().filter(|(a, _)| *a != action).collect();
        let scores: Vec<(&str, f64)> = [(action, 0.84)]
            .into_iter()
            .chain(others.iter().map(|(a, _)| *a).zip([0.07, 0.05, 0.03, 0.01]))
            .collect();
        let fake = typesafe(move |n, body| {
            if n > 0 {
                return choose(body, "park", 0.9); // the second Wake ends the run
            }
            reply(action, json!(0.7), &scores)
        });
        o.cfg.typesafe = fake.clone();
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");
        w.await_line(line2);
        run.wait();

        let short = ALL.iter().find(|(a, _)| *a == action).unwrap().1;
        let judged = format!(
            "hx-1 judged: {short} 0.84, {} 0.07, {} 0.05, {} 0.03, {} 0.01",
            others[0].1, others[1].1, others[2].1, others[3].1
        );
        let lines = w.lines();
        let at = lines
            .iter()
            .position(|l| l == "hx-1 stuck in implement: went idle without a result (pane 1-1)")
            .unwrap_or_else(|| panic!("{action}: no stuck line in {lines:#?}"));
        assert_eq!(
            lines[at + 1..at + 3],
            [judged, line2.to_string()],
            "{action}"
        );
        let stuck = w.await_event("stuck in implement");
        assert!(
            stuck.ask.is_none(),
            "{action}: a Judgment above the floor asked the user"
        );
        assert_eq!(
            criteria(&fake.requests()[0]).len(),
            5,
            "{action}: the first Wake did not offer every action"
        );
        let want = if action == "retry" {
            STATUS_PR_OPEN
        } else {
            STATUS_PARKED
        };
        assert_eq!(o.ticket("hx-1").status, want, "{action}");
        assert!(
            !w.log().contains("sk-test"),
            "the key is in the log:\n{}",
            w.log()
        );
    }
}

/// Below the floor the Wake is the Question, with the Judgment's scores and
/// the nudge it scored higher as its one nudge; the judged line goes to the
/// log alone, so it does not close that Question.
#[test]
fn below_the_floor_the_wake_is_a_question_with_the_scores() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    idle_once(&w);
    o.cfg.typesafe = typesafe(|_, _| {
        let scores = [
            ("retry", 0.5),
            ("nudge_proceed", 0.3),
            ("park", 0.1),
            ("nudge_write_result", 0.06),
            ("wait", 0.04),
        ];
        reply("retry", json!(0.69), &scores)
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let (pane, actions, judged) = question(&w, 1);
    let said =
        "retry 0.50, nudge to carry on 0.30, park 0.10, nudge to write the result 0.06, wait 0.04";
    assert_eq!(judged.as_deref(), Some(said));
    assert_eq!(actions, [NudgeProceed, Retry, Park, Wait]);
    assert!(w
        .lines()
        .iter()
        .all(|l| !l.contains("judged") && !l.contains("retrying")));

    o.answer("hx-1", &pane, Answer::Act(Park));
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PARKED);
    assert!(
        w.log().contains(&format!(" hx-1 judged: {said}\n")),
        "log:\n{}",
        w.log()
    );
}

/// A spent action is absent from the next request: a nudge until a retry
/// re-arms it, a retry for the rest of the Stage, a wait after a timeout.
/// When only park is left the Ticket parks by rule, with no request.
#[test]
fn spent_actions_are_not_offered_and_only_park_left_parks_by_rule() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.timeout = Some(Duration::from_millis(5));
    w.session(working);
    let fake = typesafe(|n, body| {
        let choice = ["nudge_write_result", "retry", "nudge_proceed"][n];
        choose(body, choice, 0.9)
    });
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    run.wait();

    let asked: Vec<Vec<String>> = fake.requests().iter().map(criteria).collect();
    assert_eq!(
        asked,
        [
            vec!["nudge_proceed", "nudge_write_result", "park", "retry"],
            vec!["park", "retry"],
            vec!["nudge_proceed", "nudge_write_result", "park"],
        ]
    );
    let ts = o.ticket("hx-1");
    assert_eq!(
        (ts.status.as_str(), ts.reason.as_str()),
        (
            STATUS_PARKED,
            "implement timed out after 5ms again after a retry"
        )
    );
    let wakes = w.lines().iter().filter(|l| l.contains("stuck in")).count();
    assert_eq!(wakes, 3, "the fourth Wake was not parked by rule");
}

/// The Question offers only what is unspent: no nudge once the session was
/// nudged, no retry once the Stage retried, a wait while the session may
/// still take one; a retry's fresh session has its nudge again.
#[test]
fn the_question_offers_only_unspent_actions() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    idle(&w);
    o.cfg.api_key = String::new();
    o.cfg.wait = Some(Duration::from_millis(5));
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let (pane, actions, judged) = question(&w, 1);
    assert_eq!(actions, Action::ALL);
    assert_eq!(judged, None);
    o.answer("hx-1", &pane, Answer::Act(NudgeWriteResult));
    let (pane, actions, _) = question(&w, 2);
    assert_eq!(actions, [Retry, Park, Wait], "after a nudge");
    o.answer("hx-1", &pane, Answer::Act(Wait));
    w.await_line("hx-1 waiting: still working (pane 1-1)");
    let (pane, actions, _) = question(&w, 3);
    assert_eq!(actions, [Retry, Park, Wait], "after a wait");
    o.answer("hx-1", &pane, Answer::Act(Retry));
    let (pane, actions, _) = question(&w, 4);
    assert_eq!(
        actions,
        [NudgeWriteResult, NudgeProceed, Park, Wait],
        "after a retry"
    );
    o.answer("hx-1", &pane, Answer::Act(Park));
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PARKED);
}

/// Without a key no request is sent and the Wake is the Question as it was:
/// both nudges, no Judgment.
#[test]
fn no_key_means_no_request_and_a_question() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    idle_once(&w);
    let fake = typesafe(|_, body| choose(body, "park", 1.0));
    o.cfg.typesafe = fake.clone();
    o.cfg.api_key = String::new();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let (pane, actions, judged) = question(&w, 1);
    assert_eq!(actions, Action::ALL);
    assert_eq!(judged, None);
    o.answer("hx-1", &pane, Answer::Act(Park));
    run.wait();
    assert!(
        fake.requests().is_empty(),
        "a request went out without a key"
    );
    assert!(
        w.called("bd show hx-1 --json").is_empty(),
        "the state was built without a key"
    );
}

/// TypeSafe off in config.json: a key or not, no request is sent and the
/// Wake is the Question.
#[test]
fn typesafe_off_means_no_request_and_a_question() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    super::write_file(
        &w.repo.join(".orqadence/config.json"),
        r#"{"typesafe": false}"#,
    );
    idle_once(&w);
    let fake = typesafe(|_, body| choose(body, "park", 1.0));
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let (pane, actions, judged) = question(&w, 1);
    assert_eq!(actions, Action::ALL);
    assert_eq!(judged, None);
    o.answer("hx-1", &pane, Answer::Act(Park));
    run.wait();
    assert!(
        fake.requests().is_empty(),
        "a request went out with TypeSafe off"
    );
}

/// A wait Wakes the Ticket again when the wait is over, whatever herdr says
/// of the session then; the third wait is the last one offered, and the
/// count is kept in the state file.
#[test]
fn a_wait_wakes_again_after_the_wait_and_the_third_is_the_last() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    idle_once(&w);
    let wait = Duration::from_millis(30);
    o.cfg.wait = Some(wait);
    let times = Arc::new(Mutex::new(Vec::new()));
    let (at, world) = (times.clone(), w.clone());
    let fake = typesafe(move |_, body| {
        at.lock().unwrap().push(Instant::now());
        set_agents(&world, "working"); // a busy session: only the wait's end Wakes it
        let waits = criteria(body).iter().any(|a| a == "wait");
        choose(body, if waits { "wait" } else { "park" }, 0.9)
    });
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    run.wait();

    let offered_wait: Vec<bool> = fake
        .requests()
        .iter()
        .map(|body| criteria(body).iter().any(|a| a == "wait"))
        .collect();
    assert_eq!(offered_wait, [true, true, true, false]);
    let times = times.lock().unwrap();
    for pair in times.windows(2) {
        assert!(
            pair[1] - pair[0] >= wait,
            "a Wake came before its wait was over"
        );
    }
    let waits = w
        .lines()
        .iter()
        .filter(|l| *l == "hx-1 waiting: still working (pane 1-1)")
        .count();
    assert_eq!(waits, 3);
    assert_eq!(o.ticket("hx-1").status, STATUS_PARKED);
    assert_eq!(load_state(&w.repo).unwrap().tickets["hx-1"].waits, 3);
}

/// A wait keeps the session's deadline: a session still working when its
/// Stage runs out of time Wakes as timed out, not a full Stage later.
#[test]
fn a_wait_keeps_the_sessions_deadline() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let timeout = Duration::from_millis(600);
    o.cfg.timeout = Some(timeout);
    o.cfg.wait = Some(Duration::from_secs(10));
    w.session(working);
    let times = Arc::new(Mutex::new(Vec::new()));
    let (at, world) = (times.clone(), w.clone());
    let fake = typesafe(move |n, body| {
        at.lock().unwrap().push(Instant::now());
        set_agents(&world, "working");
        choose(body, if n == 0 { "wait" } else { "park" }, 0.9)
    });
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    w.await_event("implement prompted");
    thread::sleep(timeout / 2);
    set_agents(&w, "idle"); // looks idle halfway through: the first Wake
    run.wait();

    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1]["state"]["why_woken"], "timed out after 600ms",
        "the waited session did not time out"
    );
    let times = times.lock().unwrap();
    assert!(
        times[1] - times[0] < timeout,
        "the wait gave the session a new deadline: timed out {:?} after the wait",
        times[1] - times[0]
    );
}

/// A session that died is offered no wait: nothing is still working.
#[test]
fn a_dead_session_is_offered_no_wait() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let world = w.clone();
    w.session(move |p| {
        world.lock().agents.remove(&p.pane);
        (String::new(), "idle".to_string())
    });
    let fake = typesafe(|_, body| choose(body, "park", 0.9));
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 stuck in implement: session died");
    run.wait();
    assert_eq!(
        criteria(&fake.requests()[0]),
        ["nudge_proceed", "nudge_write_result", "park", "retry"]
    );
}

/// /stop-work while TypeSafe is answering: the late Judgment is not acted
/// on, and the run ends with the Ticket as it was.
#[test]
fn stop_during_a_judgment_takes_no_action() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    idle(&w);
    let (entered, release) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let (inside, go) = (entered.clone(), release.clone());
    o.cfg.typesafe = Fake::new(move |body| {
        inside.store(true, Ordering::SeqCst);
        while !go.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        Ok(choose(body, "nudge_write_result", 0.9))
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !entered.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "no Judgment was asked");
        thread::sleep(Duration::from_millis(1));
    }
    o.stop();
    release.store(true, Ordering::SeqCst);
    run.wait();

    for line in w.lines() {
        assert!(
            !line.contains("stuck in") && !line.contains("judged") && !line.contains("nudged"),
            "acted after /stop-work: {line:?}"
        );
    }
    let ts = o.ticket("hx-1");
    assert!(ts.status == STATUS_RUNNING && !ts.nudged, "{ts:?}");
    assert_eq!(
        w.called("herdr agent prompt").len(),
        1,
        "a nudge was sent after /stop-work"
    );
}

/// A blocked session is the user's alone: no Judgment is asked.
#[test]
fn a_blocked_session_never_reaches_the_judgment() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| ("STATUS: done\n".to_string(), "blocked".to_string()));
    let fake = typesafe(|_, body| choose(body, "park", 0.9));
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 waiting at a prompt in implement (pane 1-1)");
    o.command("park-hx-1");
    run.wait();
    assert!(fake.requests().is_empty(), "a blocked session was judged");
}

/// An error, or a reply without a usable choice or confidence, is no
/// Judgment: the Wake is a Question, and the log line saying so carries no
/// key.
#[test]
fn no_usable_reply_is_no_judgment_and_keeps_the_key_out_of_the_log() {
    for (name, answer, logged) in [
        ("error", Err("401: bad key sk-secret"), "401: bad key ***"),
        (
            "confidence over 1",
            Ok(json!(1.5)),
            "no choice in the reply",
        ),
        (
            "confidence not a number",
            Ok(json!("high")),
            "no choice in the reply",
        ),
    ] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        idle_once(&w);
        o.cfg.api_key = "sk-secret".to_string();
        o.cfg.typesafe = Fake::new(move |_| {
            answer
                .clone()
                .map(|confidence| reply("park", confidence, &[("park", 1.0)]))
                .map_err(str::to_string)
        });
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");
        let (pane, _, judged) = question(&w, 1);
        assert_eq!(judged, None, "{name}");
        o.answer("hx-1", &pane, Answer::Act(Park));
        run.wait();
        assert!(
            w.log().contains(&format!(" hx-1 no Judgment: {logged}\n")),
            "{name}: log:\n{}",
            w.log()
        );
        assert!(!w.log().contains("sk-secret"), "{name}");
    }
}

/// judge.py's source, the reference the request is checked against.
fn judge_py() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/design/judgment-prototype/judge.py");
    fs::read_to_string(path).unwrap()
}

/// A Python dict of string literals from judge.py, `NAME = {` to `}`, one
/// entry a line, in its order.
fn py_dict(src: &str, name: &str) -> Vec<(String, String)> {
    let body = src.split(&format!("{name} = {{\n")).nth(1).unwrap();
    let body = body.split("\n}").next().unwrap();
    body.lines()
        .map(|line| {
            let entry = format!("{{{}}}", line.trim().trim_end_matches(','));
            let entry: serde_json::Map<String, Value> = serde_json::from_str(&entry).unwrap();
            let (key, value) = entry.into_iter().next().unwrap();
            (key, value.as_str().unwrap().to_string())
        })
        .collect()
}

/// Whether every key shows in the JSON text after the one before it.
fn in_order(raw: &str, keys: &[&str]) -> bool {
    let mut from = 0;
    keys.iter()
        .all(|key| match raw[from..].find(&format!("\"{key}\":")) {
            Some(at) => {
                from += at + 1;
                true
            }
            None => false,
        })
}

/// The state and request built for every case in the prototype match what
/// judge.py sends for it, field for field and key for key in its order (its
/// two diagnostic Nouls left out, as the README records), and so do the
/// canned prompts.
#[test]
fn the_prototype_cases_build_judge_pys_request() {
    let src = judge_py();
    let actions = py_dict(&src, "ACTIONS");
    let instructions: String = {
        let from =
            src.find("\"instructions\": \"An agent session").unwrap() + "\"instructions\": ".len();
        let line = src[from..].lines().next().unwrap();
        serde_json::from_str(line.trim_end_matches(',')).unwrap()
    };
    let file = Path::new(".orqadence/runs/hx-1/implement.md");
    let prompts: Vec<String> = py_dict(&src, "PROMPTS")
        .into_iter()
        .map(|(_, p)| p.replace("{result_file}", &file.display().to_string()))
        .collect();
    assert_eq!(
        prompts,
        [prompt(NudgeWriteResult, file), prompt(NudgeProceed, file)]
    );

    let cases = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/design/judgment-prototype/cases");
    let mut seen = 0;
    for case in fs::read_dir(cases).unwrap() {
        let case = case.unwrap().path();
        let name = case.file_name().unwrap().to_string_lossy().to_string();
        let wake: Value =
            serde_json::from_str(&fs::read_to_string(case.join("wake.json")).unwrap()).unwrap();
        let tail = fs::read_to_string(case.join("tail.txt")).unwrap();
        let result = fs::read_to_string(case.join("result.md")).ok();
        let flag = |k: &str| wake.get(k).and_then(Value::as_bool).unwrap_or(false);
        let reason = wake["reason"].as_str().unwrap();

        // judge.py: state_for, offered and ask.
        let criteria: Vec<&(String, String)> = actions
            .iter()
            .filter(|(a, _)| match a.as_str() {
                "nudge_write_result" | "nudge_proceed" => !flag("nudged"),
                "retry" => !flag("retried"),
                "wait" => !reason.starts_with("timed out"),
                _ => true,
            })
            .collect();
        let want = json!({
            "model": "jev-latest",
            "state": {
                "ticket": wake["ticket"],
                "stage": wake["stage"], "round": wake["round"],
                "why_woken": wake["reason"],
                "already_nudged": flag("nudged"),
                "already_retried": flag("retried"),
                "result_file": { "path": wake["result_file"], "content": result.as_deref().unwrap_or("missing") },
                "pane_tail": tail,
            },
            "questions": { "action": {
                "type": "choice",
                "instructions": instructions,
                "criteria": criteria.iter().map(|(a, c)| (a.clone(), json!(c))).collect::<serde_json::Map<_, _>>(),
            } },
        });
        let mut keys = vec![
            "model",
            "state",
            "ticket",
            "id",
            "title",
            "spec",
            "stage",
            "round",
            "why_woken",
            "already_nudged",
            "already_retried",
            "result_file",
            "path",
            "content",
            "pane_tail",
            "questions",
            "action",
            "type",
            "instructions",
            "criteria",
        ];
        keys.extend(criteria.iter().map(|(a, _)| a.as_str()));

        // The Orchestrator: the Ticket from bd, the result file in the run
        // directory, the Stage and spending from the Ticket's state; the
        // session of every case is alive.
        let repo = TempDir::new();
        let ticket = wake["ticket"].clone();
        let bd = crate::tools::fake::Fake::new(move |_, argv| {
            assert_eq!(
                argv,
                ["bd", "show", ticket["id"].as_str().unwrap(), "--json"]
            );
            Ok(json!([{
                "id": ticket["id"], "title": ticket["title"], "description": ticket["spec"],
                "status": "in_progress", "issue_type": "task",
            }])
            .to_string())
        });
        let o = Orchestrator::with_state(
            Config::for_tests(bd, repo.path(), repo.path()),
            State::default(),
        );
        let path = repo.path().join(wake["result_file"].as_str().unwrap());
        if let Some(result) = &result {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, result).unwrap();
        }
        let ts = TicketState {
            stage: wake["stage"].as_str().unwrap().to_string(),
            round: wake["round"].as_u64().unwrap() as usize,
            nudged: flag("nudged"),
            retried: flag("retried"),
            ..Default::default()
        };
        let state = o.wake_state(
            wake["ticket"]["id"].as_str().unwrap(),
            &ts,
            reason,
            &path,
            &tail,
        );
        let raw = request(&state, &offered(&ts, reason, true));
        let got: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(got, want, "{name}");
        assert!(
            in_order(&raw, &keys),
            "{name}: keys out of judge.py's order:\n{raw}"
        );
        seen += 1;
    }
    assert_eq!(seen, 10, "cases");
}

/// config.json's wake_floor, read at each Wake: 0.6 acts on a Judgment the
/// default floor asks about; missing or empty is the default. One that is
/// not a number from 0 to 1, null too, is never acted on: even a sure
/// Judgment is the Question, and the log says why.
#[test]
fn config_jsons_wake_floor_moves_the_action_and_a_bad_one_asks() {
    for (config, confidence, acted) in [
        ("", 0.65, false),
        (r#"{"wake_floor": ""}"#, 0.65, false),
        (r#"{"wake_floor": 0.6}"#, 0.65, true),
        (r#"{"wake_floor": -1}"#, 0.99, false),
        (r#"{"wake_floor": "low"}"#, 0.99, false),
        (r#"{"wake_floor": null}"#, 0.99, false),
    ] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        idle_once(&w);
        if !config.is_empty() {
            super::write_file(&w.repo.join(".orqadence/config.json"), config);
        }
        o.cfg.typesafe = typesafe(move |_, body| choose(body, "retry", confidence));
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");
        if !acted {
            let (pane, _, judged) = question(&w, 1);
            assert!(judged.is_some(), "{config}");
            o.answer("hx-1", &pane, Answer::Act(Retry));
        }
        run.wait();
        assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN, "{config}");
        let asked = w.events().iter().any(|e| e.ask.is_some());
        assert_eq!(asked, !acted, "{config}");
        let refused =
            " hx-1 wake_floor is not a number from 0 to 1: the Judgment is not acted on\n";
        let bad = ["-1", "low", "null"].iter().any(|v| config.contains(v));
        assert_eq!(w.log().contains(refused), bad, "{config}:\n{}", w.log());
    }
}

/// A plan is approved only with covers and in_scope at or above the floor
/// and asks below 0.5: a question goes to the user whatever else scores.
#[test]
fn a_plan_is_approved_with_covers_and_in_scope_at_the_floor_and_no_question() {
    let judged = |covers, in_scope, asks| PlanJudged {
        covers,
        in_scope,
        asks,
        floor: Some(0.65),
    };
    for (plan, approved) in [
        (judged(0.65, 0.65, 0.49), true),
        (judged(1.0, 1.0, 0.0), true),
        (judged(0.64, 1.0, 0.0), false),
        (judged(1.0, 0.64, 0.0), false),
        (judged(1.0, 1.0, 0.5), false),
        (judged(1.0, 1.0, 0.9), false),
        (
            PlanJudged {
                floor: None,
                ..judged(1.0, 1.0, 0.0)
            },
            false,
        ),
    ] {
        assert_eq!(plan.approves(), approved, "{plan:?}");
    }
}

/// The judged line names every score, a yes as its score and a no as one
/// minus it, a yes short of the floor marked, so a plan that reaches the
/// user says why; on a bad floor nothing is marked.
#[test]
fn the_plan_judged_line_names_every_score() {
    let yes = PlanJudged {
        covers: 0.91,
        in_scope: 0.88,
        asks: 0.93,
        floor: Some(0.65),
    };
    assert_eq!(
        yes.said(),
        "plan covers the Ticket 0.91, stays in scope 0.88, asks you a question 0.93"
    );
    let no = PlanJudged {
        covers: 0.19,
        in_scope: 0.3,
        asks: 0.05,
        floor: Some(0.65),
    };
    assert_eq!(
        no.said(),
        "plan misses an acceptance criterion 0.81, goes beyond the Ticket 0.70, asks nothing 0.95"
    );
    assert_eq!(
        no.short(),
        "covers 0.19 < 0.65, in scope 0.30 < 0.65, asks 0.05"
    );
    let short = PlanJudged {
        covers: 0.62,
        in_scope: 0.82,
        asks: 0.17,
        floor: Some(0.65),
    };
    assert_eq!(
        short.said(),
        "plan covers the Ticket 0.62 < 0.65, stays in scope 0.82, asks nothing 0.83"
    );
    let bad = PlanJudged {
        floor: None,
        ..short
    };
    assert_eq!(
        bad.said(),
        "plan covers the Ticket 0.62, stays in scope 0.82, asks nothing 0.83"
    );
}

/// judge_plan.py's source, the reference the plan request is checked against.
fn judge_plan_py() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/design/plan-judgment-prototype/judge_plan.py");
    fs::read_to_string(path).unwrap()
}

/// The plan request built for every case of the plan prototype matches what
/// judge_plan.py sends with its settled Nouls, field for field and key for
/// key in its order; the plan floor is the one its replay settled.
#[test]
fn the_plan_prototype_cases_build_judge_plan_pys_request() {
    let src = judge_plan_py();
    let nouls = py_dict(&src, "NOULS");
    let names: Vec<&str> = nouls.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["covers", "in_scope", "asks"]);
    let floor: f64 = src
        .lines()
        .find_map(|line| line.strip_prefix("FLOOR = "))
        .and_then(|rest| rest.split('#').next())
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(PLAN_FLOOR.default, floor);

    let cases =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/design/plan-judgment-prototype/cases");
    let mut seen = 0;
    for case in fs::read_dir(cases).unwrap() {
        let case = case.unwrap().path();
        let name = case.file_name().unwrap().to_string_lossy().to_string();
        let plan = fs::read_to_string(case.join("plan.md")).unwrap();
        let ticket: Value =
            serde_json::from_str(&fs::read_to_string(case.join("ticket.json")).unwrap()).unwrap();

        // judge_plan.py: state_for and ask.
        let criteria = ticket["acceptance_criteria"].as_str().unwrap_or_default();
        let mut description = ticket["description"].as_str().unwrap().to_string();
        if !criteria.is_empty() {
            description += &format!("\n\nAcceptance criteria:\n{criteria}");
        }
        let questions: serde_json::Map<String, Value> = nouls
            .iter()
            .map(|(name, text)| {
                (
                    name.clone(),
                    json!({ "type": "noul", "instructions": text }),
                )
            })
            .collect();
        let want = json!({
            "model": "jev-latest",
            "state": {
                "plan": plan,
                "ticket": { "id": ticket["id"], "title": ticket["title"], "description": description },
                "prior_feedback": null,
            },
            "questions": questions,
        });
        let mut keys = vec![
            "model",
            "state",
            "plan",
            "ticket",
            "id",
            "title",
            "description",
            "prior_feedback",
            "questions",
        ];
        for name in &names {
            keys.extend([*name, "type", "instructions"]);
        }

        // The Orchestrator: the Ticket from bd, no feedback kept.
        let repo = TempDir::new();
        let issue = json!([{
            "id": ticket["id"], "title": ticket["title"], "description": ticket["description"],
            "acceptance_criteria": ticket["acceptance_criteria"],
            "status": "closed", "issue_type": "task",
        }])
        .to_string();
        let bd = crate::tools::fake::Fake::new(move |_, _| Ok(issue.clone()));
        let o = Orchestrator::with_state(
            Config::for_tests(bd, repo.path(), repo.path()),
            State::default(),
        );
        let raw = plan_request(&o.plan_state(ticket["id"].as_str().unwrap(), &plan));
        let got: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(got, want, "{name}");
        assert!(
            in_order(&raw, &keys),
            "{name}: keys out of judge_plan.py's order"
        );
        seen += 1;
    }
    assert_eq!(seen, 33, "cases");
}
