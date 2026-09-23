use super::judgment::fake::Fake;
use super::judgment::{offered, request};
use super::stage::{nudges, Answer, Ask, Config, Orchestrator};
use super::state::{State, TicketState, STATUS_PARKED, STATUS_PR_OPEN};
use super::world::{new_world, spawn_ticket, succeed, working, BdTicket, World};
use crate::tempdir::TempDir;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    let scores: serde_json::Map<String, Value> = offered
        .iter()
        .map(|a| {
            (
                a.clone(),
                json!(if a == choice { confidence } else { rest }),
            )
        })
        .collect();
    reply(choice, confidence, scores)
}

fn reply(choice: &str, confidence: f64, probabilities: serde_json::Map<String, Value>) -> Value {
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

/// Every action at or above the floor acts and logs both lines, the Wake
/// raises no Question, and the Judgment scores only the offered actions,
/// highest first.
#[test]
fn each_action_at_the_floor_acts_and_logs_both_lines() {
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
        let fake = typesafe(move |n, body| {
            if n > 0 {
                return choose(body, "park", 0.9); // the second Wake ends the run
            }
            let others = [
                "nudge_proceed",
                "nudge_write_result",
                "park",
                "retry",
                "wait",
            ]
            .into_iter()
            .filter(|a| *a != action);
            let mut scores = serde_json::Map::new();
            scores.insert(action.to_string(), json!(0.84));
            for (a, score) in others.zip([0.07, 0.05, 0.03, 0.01]) {
                scores.insert(a.to_string(), json!(score));
            }
            reply(action, 0.7, scores)
        });
        o.cfg.typesafe = fake.clone();
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");
        w.await_line(line2);
        run.wait();

        let others: Vec<_> = [
            "nudge_proceed",
            "nudge_write_result",
            "park",
            "retry",
            "wait",
        ]
        .into_iter()
        .filter(|a| *a != action)
        .collect();
        let judged = format!(
            "hx-1 judged: {action} 0.84, {} 0.07, {} 0.05, {} 0.03, {} 0.01",
            others[0], others[1], others[2], others[3]
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
        assert!(
            criteria(&fake.requests()[0]).len() == 5,
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

/// Below the floor the Wake is the Question, with the scores shown and the
/// higher-scored nudge as its one nudge; the judged line goes to the log
/// alone, so it does not close that Question.
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
        reply(
            "retry",
            0.69,
            scores
                .iter()
                .map(|(a, s)| (a.to_string(), json!(s)))
                .collect(),
        )
    });
    let file = o.run_dir("hx-1").join("implement.md");
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let wake = w.await_event("stuck in implement: went idle without a result (pane 1-1)");
    let Some(Ask::Wake {
        pane,
        nudges: offered,
        scores,
        ..
    }) = wake.ask
    else {
        panic!("below the floor the Wake asks nothing: {wake:?}");
    };
    assert_eq!(
        scores,
        "retry 0.50, nudge_proceed 0.30, park 0.10, nudge_write_result 0.06, wait 0.04"
    );
    let [_, proceed] = nudges(&file);
    assert_eq!(
        offered,
        [(1, proceed)],
        "the Question's nudge is not the Judgment's pick"
    );
    assert!(
        w.log()
            .contains(" hx-1 judged: retry 0.50, nudge_proceed 0.30, park 0.10"),
        "log:\n{}",
        w.log()
    );
    assert!(w
        .lines()
        .iter()
        .all(|l| !l.contains("judged") && !l.contains("retrying")));

    o.answer("hx-1", &pane, Answer::Park);
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PARKED);
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

/// Without a key no request is sent and the Wake is the Question as it was,
/// both nudges and no scores.
#[test]
fn no_key_means_no_request_and_a_question() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    idle_once(&w);
    let fake = typesafe(|_, body| choose(body, "park", 1.0));
    o.cfg.typesafe = fake.clone();
    o.cfg.api_key = String::new();
    let file = o.run_dir("hx-1").join("implement.md");
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let wake = w.await_event("stuck in implement");
    let Some(Ask::Wake {
        pane,
        nudges: offered,
        scores,
        ..
    }) = wake.ask
    else {
        panic!("the Wake asks nothing: {wake:?}");
    };
    let [write, proceed] = nudges(&file);
    assert_eq!(offered, [(0, write), (1, proceed)]);
    assert_eq!(scores, "");
    o.answer("hx-1", &pane, Answer::Park);
    run.wait();
    assert!(
        fake.requests().is_empty(),
        "a request went out without a key"
    );
    assert!(
        w.called("bd show").is_empty(),
        "the state was built without a key"
    );
}

/// A wait Wakes the Ticket again once the wait has passed; the third wait
/// is the last one offered.
#[test]
fn a_wait_wakes_again_after_the_wait_and_the_third_is_the_last() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    idle_once(&w);
    let wait = Duration::from_millis(30);
    o.cfg.wait = Some(wait);
    let times = Arc::new(Mutex::new(Vec::new()));
    let at = times.clone();
    let fake = typesafe(move |_, body| {
        at.lock().unwrap().push(Instant::now());
        let choice = if criteria(body).iter().any(|a| a == "wait") {
            "wait"
        } else {
            "park"
        };
        choose(body, choice, 0.9)
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
}

/// An error from TypeSafe is no Judgment: the Wake is a Question, and the
/// log line saying so carries no key.
#[test]
fn an_error_is_no_judgment_and_keeps_the_key_out_of_the_log() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    idle_once(&w);
    o.cfg.api_key = "sk-secret".to_string();
    o.cfg.typesafe = Fake::new(|_| Err("401: bad key sk-secret".to_string()));
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    let wake = w.await_event("stuck in implement");
    let Some(Ask::Wake { pane, scores, .. }) = wake.ask else {
        panic!("the Wake asks nothing: {wake:?}");
    };
    assert_eq!(scores, "");
    o.answer("hx-1", &pane, Answer::Park);
    run.wait();
    assert!(
        w.log().contains(" hx-1 no Judgment: 401: bad key ***\n"),
        "log:\n{}",
        w.log()
    );
    assert!(!w.log().contains("sk-secret"));
}

/// judge.py's source, the reference the request is checked against.
fn judge_py() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/design/judgment-prototype/judge.py");
    fs::read_to_string(path).unwrap()
}

/// A Python dict of string literals from judge.py, `NAME = {` to `}`.
fn py_dict(src: &str, name: &str) -> serde_json::Map<String, Value> {
    let body = src.split(&format!("{name} = {{\n")).nth(1).unwrap();
    let body = body
        .split("\n}")
        .next()
        .unwrap()
        .trim_end()
        .trim_end_matches(',');
    serde_json::from_str(&format!("{{{body}}}")).unwrap()
}

/// The state and request built for every case in the prototype match what
/// judge.py sends for it, field for field (its two diagnostic Nouls left
/// out, as the README records), and so do the canned prompts.
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
    let prompts = py_dict(&src, "PROMPTS");
    let file = Path::new(".harness/runs/hx-1/implement.md");
    assert_eq!(
        nudges(file).to_vec(),
        ["nudge_write_result", "nudge_proceed"].map(|a| prompts[a]
            .as_str()
            .unwrap()
            .replace("{result_file}", &file.display().to_string()))
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
        let mut criteria = actions.clone();
        if flag("nudged") {
            criteria.remove("nudge_write_result");
            criteria.remove("nudge_proceed");
        }
        if flag("retried") {
            criteria.remove("retry");
        }
        if reason.starts_with("timed out") {
            criteria.remove("wait");
        }
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
            "questions": { "action": { "type": "choice", "instructions": instructions, "criteria": criteria } },
        });

        // The Orchestrator: the Ticket from bd, the result file in the run
        // directory, the Stage and spending from the Ticket's state.
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
        let got = request(state, &offered(&ts, reason, 0));
        assert_eq!(got, want, "{name}");
        seen += 1;
    }
    assert_eq!(seen, 10, "cases");
}
