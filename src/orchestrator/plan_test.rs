use super::judgment::fake::Fake;
use super::stage::{Answer, Ask};
use super::state::{STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};
use super::world::{new_world, spawn_ticket, succeed, BdTicket, Prompt, World};
use super::write_file;
use serde_json::{json, Value};
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const PLAN: &str = "# Plan\n\n- change src/x.rs\n- test: x_works\n";
const REVISED: &str = "# Plan\n\n- change src/x.rs and src/y.rs\n- test: x_works, y_works\n";

/// TypeSafe's answer to a plan: the Noul's score for yes.
pub(crate) fn noul(score: f64) -> Value {
    json!({ "answers": { "follows": { "type": "noul", "noul": score } } })
}

/// A TypeSafe that answers the plans put to it in turn, by number.
fn typesafe(answer: impl Fn(usize) -> Result<Value, String> + Send + Sync + 'static) -> Arc<Fake> {
    let n = AtomicUsize::new(0);
    Fake::new(move |_| answer(n.fetch_add(1, Ordering::SeqCst)))
}

/// The Implement session plans: the hook copies its plan into the run
/// directory and the session stops at the plan dialog. Later Stages succeed.
fn plans(w: &World) {
    let run = w.repo.join(".harness/runs/hx-1");
    w.session(move |p| match p.stage.as_str() {
        "implement" => at_dialog(&run, PLAN),
        _ => succeed(p),
    });
}

pub(crate) fn at_dialog(run: &std::path::Path, plan: &str) -> (String, String) {
    write_file(&run.join("plan.md"), plan);
    (String::new(), "blocked".to_string())
}

/// herdr and bd as a plan dialog needs them: bd shows the Ticket; a visible
/// read shows plan mode; enter after down, down closes the dialog on its
/// empty third option, leaving the session idle in plan mode; enter alone
/// approves, and the session moves to `approved` ("idle" having written its
/// result, or "blocked" at a permission prompt).
pub(crate) fn dialog(w: &Arc<World>, approved: &'static str) {
    let world = w.clone();
    w.hook(move |_, argv| {
        let cmd = argv.join(" ");
        if cmd == "bd show hx-1 --json" {
            let issue = json!([{ "id": "hx-1", "title": "Ticket hx-1", "description": "Do x.",
                "acceptance_criteria": "x works", "status": "in_progress", "issue_type": "task" }]);
            return Some(Ok(issue.to_string()));
        }
        if cmd.starts_with("herdr agent read") && cmd.ends_with(" --source visible") {
            return Some(Ok("⏸ plan mode on (shift+tab to cycle)".to_string()));
        }
        if !cmd.starts_with("herdr agent send-keys") {
            return None;
        }
        if argv[4] == "enter" {
            let keys = world.called("herdr agent send-keys");
            let dismissed = keys.len() >= 2 && keys[keys.len() - 2].ends_with(" down");
            let status = if dismissed { "idle" } else { approved };
            if !dismissed && approved == "idle" {
                write_file(
                    &world.repo.join(".harness/runs/hx-1/implement.md"),
                    "STATUS: done\n",
                );
            }
            world
                .lock()
                .agents
                .insert(argv[3].to_string(), status.to_string());
        }
        Some(Ok(String::new()))
    });
}

/// The plan Question's pane, plan and score.
fn plan_question(w: &World, n: usize) -> (String, String, Option<f64>) {
    let ready = w.await_nth("plan ready in implement (pane 1-1)", n);
    let Some(Ask::Plan { pane, plan, judged }) = ready.ask else {
        panic!("the plan asks nothing: {ready:?}");
    };
    (pane, plan, judged)
}

/// Implement starts in plan mode with its own settings file, which holds
/// the one hook: ExitPlanMode's plan copied into the run directory by the
/// harness binary's hidden mode. The other claude Stages launch as before.
#[test]
fn implement_starts_in_plan_mode_with_the_hook_in_the_run_directory() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    o.run_ticket("hx-1");
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);

    let run = o.run_dir("hx-1");
    let settings = run.join("settings.json");
    let start = w.called("herdr agent start h-hx-1-implement");
    assert_eq!(start.len(), 1);
    assert!(
        start[0].ends_with(&format!(
            " -- --permission-mode plan --settings {} --add-dir {}",
            settings.display(),
            run.display()
        )),
        "{start:?}"
    );
    let fix = w.called("herdr agent start h-hx-1-fix");
    assert!(
        fix[0].ends_with(&format!(
            " -- --permission-mode auto --add-dir {}",
            run.display()
        )),
        "{fix:?}"
    );

    let settings: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    let exe = crate::update::exe_path().unwrap();
    assert_eq!(
        settings,
        json!({ "hooks": { "PreToolUse": [{ "matcher": "ExitPlanMode", "hooks": [{
            "type": "command",
            "command": format!("'{}' __plan-hook '{}'", exe.display(), run.join("plan.md").display()),
        }] }] } })
    );
}

/// A blocked Implement with a fresh plan reaches the Noul over the plan and
/// the Ticket; yes at 0.9 approves it with enter, and each step is a line.
#[test]
fn a_fresh_plan_reaches_the_noul_and_yes_at_the_floor_approves_it() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    plans(&w);
    dialog(&w, "idle");
    let fake = typesafe(|_| Ok(noul(0.9)));
    let mut o = o;
    o.cfg.typesafe = fake.clone();
    o.run_ticket("hx-1");
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);

    let asked = fake.requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0],
        json!({
            "model": "jev-latest",
            "state": {
                "plan": PLAN,
                "ticket": { "id": "hx-1", "title": "Ticket hx-1", "spec": "Do x.\n\nAcceptance criteria:\nx works" },
                "prior_feedback": "none",
            },
            "questions": { "follows": {
                "type": "noul",
                "instructions": "Does `plan` implement `ticket`, all of it and nothing more, without leaving decisions open? `prior_feedback` is what the user asked of the plan before this revision, or none.",
            } },
        })
    );
    let start = w.called("herdr agent start h-hx-1-implement").remove(0);
    let pane = start
        .split(" --pane ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    assert_eq!(
        w.called("herdr agent send-keys"),
        [format!("herdr agent send-keys {pane} enter")]
    );
    let lines = w.lines();
    let at = lines
        .iter()
        .position(|l| l == "hx-1 plan ready in implement (pane 1-1)")
        .unwrap_or_else(|| panic!("no plan ready line in {lines:#?}"));
    assert_eq!(
        lines[at + 1..at + 4],
        [
            "hx-1 judged: plan follows the Ticket 0.90",
            "hx-1 plan approved",
            "hx-1 implemented"
        ]
    );
    assert!(
        w.events().iter().all(|e| e.ask.is_none()),
        "approved, yet asked"
    );
    assert!(!w.log().contains("sk-test"));
}

/// Yes below the floor, a no at any score, no Judgment to be had or no key:
/// the plan Question, with the plan and the score; approve sends enter.
#[test]
fn below_the_floor_a_no_or_no_judgment_raises_the_plan_question() {
    for (name, answer, said) in [
        (
            "yes below the floor",
            Ok(0.7),
            Some("plan follows the Ticket 0.70"),
        ),
        ("no", Ok(0.12), Some("plan strays from the Ticket 0.88")),
        ("error", Err("401: bad key sk-test"), None),
        ("no key", Ok(1.0), None),
    ] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        plans(&w);
        dialog(&w, "idle");
        let fake = typesafe(move |_| answer.map(noul).map_err(str::to_string));
        o.cfg.typesafe = fake.clone();
        if name == "no key" {
            o.cfg.api_key = String::new();
        }
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");

        let (pane, plan, judged) = plan_question(&w, 1);
        assert_eq!(plan, PLAN, "{name}");
        assert_eq!(judged, answer.ok().filter(|_| said.is_some()), "{name}");
        assert!(w.called("herdr agent send-keys").is_empty(), "{name}");
        assert!(
            w.lines()
                .iter()
                .all(|l| !l.contains("judged") && !l.contains("waiting at")),
            "{name}: {:#?}",
            w.lines()
        );
        if let Some(said) = said {
            assert!(
                w.log().contains(&format!(" hx-1 judged: {said}\n")),
                "{name}:\n{}",
                w.log()
            );
        }
        if name == "error" {
            assert!(
                w.log().contains(" hx-1 no Judgment: 401: bad key ***\n"),
                "{}",
                w.log()
            );
        }
        assert_eq!(
            fake.requests().len(),
            usize::from(name != "no key"),
            "{name}"
        );

        o.answer("hx-1", &pane, Answer::Approve);
        w.await_line("hx-1 plan approved");
        run.wait();
        assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN, "{name}");
        assert_eq!(
            w.called("herdr agent send-keys"),
            [format!("herdr agent send-keys {pane} enter")],
            "{name}"
        );
        assert!(
            !w.log().contains("sk-test"),
            "{name}: the key is in the log"
        );
    }
}

/// Feedback closes the dialog on its empty third option, one key a call with
/// a pane re-read between, confirms the session idle and still in plan
/// mode, and sends the feedback as its prompt; the revised plan is judged
/// again with that feedback. Never esc, never 3.
#[test]
fn feedback_goes_back_by_keys_and_the_prompt_and_the_revised_plan_is_judged_again() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let run = w.repo.join(".harness/runs/hx-1");
    w.session(move |p: &Prompt| match p.stage.as_str() {
        "implement" => at_dialog(&run, PLAN),
        "" if p.text == "cover y too" => at_dialog(&run, REVISED),
        _ => succeed(p),
    });
    dialog(&w, "idle");
    let fake = typesafe(|n| Ok(noul([0.3, 0.95][n])));
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let (pane, _, judged) = plan_question(&w, 1);
    assert_eq!(judged, Some(0.3));
    let before = w.calls().len();
    o.answer("hx-1", &pane, Answer::Prompt("cover y too".to_string()));
    w.await_line("hx-1 plan sent back with your feedback");
    w.await_line("hx-1 plan approved");
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);

    let driven: Vec<String> = w.calls()[before..]
        .iter()
        .filter(|c| {
            [
                "herdr agent send-keys",
                "herdr agent read",
                "herdr agent prompt",
            ]
            .iter()
            .any(|p| c.starts_with(p))
        })
        .cloned()
        .collect();
    let read = format!("herdr agent read {pane} --source visible");
    assert_eq!(
        driven[..7],
        [
            format!("herdr agent send-keys {pane} down"),
            read.clone(),
            format!("herdr agent send-keys {pane} down"),
            read.clone(),
            format!("herdr agent send-keys {pane} enter"),
            read,
            format!("herdr agent prompt {pane} cover y too"),
        ]
    );
    let keys = w.called("herdr agent send-keys");
    assert!(
        keys.iter()
            .all(|k| !k.ends_with(" esc") && !k.ends_with(" 3")),
        "{keys:?}"
    );

    let asked = fake.requests();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[1]["state"]["plan"], REVISED);
    assert_eq!(asked[1]["state"]["prior_feedback"], "cover y too");
    let lines = w.lines();
    let sent = lines
        .iter()
        .position(|l| l == "hx-1 plan sent back with your feedback")
        .unwrap();
    assert_eq!(
        lines[sent + 1..sent + 4],
        [
            "hx-1 plan ready in implement (pane 1-1)",
            "hx-1 judged: plan follows the Ticket 0.95",
            "hx-1 plan approved"
        ],
        "{lines:#?}"
    );
    assert!(lines
        .iter()
        .all(|l| !l.contains("stuck in") && !l.contains("carrying on")));
}

/// A blocked Implement with no plan in the run directory is the ordinary
/// blocked Question, and so is a prompt after the plan was answered: only a
/// plan newer than the last one judged is a plan ready.
#[test]
fn a_blocked_implement_without_a_fresh_plan_is_the_ordinary_blocked_question() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|p| match p.stage.as_str() {
        "implement" => (String::new(), "blocked".to_string()), // a permission prompt first
        _ => succeed(p),
    });
    dialog(&w, "blocked");
    let fake = typesafe(|_| Ok(noul(0.9)));
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    let set = |status: &str| {
        for s in w.lock().agents.values_mut() {
            *s = status.to_string();
        }
    };

    let blocked = w.await_event("waiting at a prompt in implement (pane 1-1)");
    assert!(
        matches!(blocked.ask, Some(Ask::Blocked { .. })),
        "{blocked:?}"
    );
    assert!(
        fake.requests().is_empty(),
        "a prompt without a plan was judged"
    );
    set("working"); // answered in the pane
    w.await_line("hx-1 carrying on");

    write_file(&o.run_dir("hx-1").join("plan.md"), PLAN);
    set("blocked"); // the plan dialog
    w.await_line("hx-1 plan approved");
    // After approval the session stops at a permission prompt: not a plan.
    let again = w.await_nth("waiting at a prompt in implement (pane 1-1)", 2);
    assert!(matches!(again.ask, Some(Ask::Blocked { .. })), "{again:?}");
    assert_eq!(
        fake.requests().len(),
        1,
        "the answered plan was judged again"
    );

    write_file(&o.run_dir("hx-1").join("implement.md"), "STATUS: done\n");
    set("idle");
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);
    assert_eq!(
        w.lines()
            .iter()
            .filter(|l| l.contains("plan ready"))
            .count(),
        1
    );
}

/// /stop-work while TypeSafe judges a plan: nothing is said or sent, and the
/// plan stays for /continue to judge again. Park at the plan Question parks.
#[test]
fn stop_during_the_plan_judgment_takes_no_action_and_park_parks() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    plans(&w);
    dialog(&w, "idle");
    let (entered, release) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let (inside, go) = (entered.clone(), release.clone());
    o.cfg.typesafe = Fake::new(move |_| {
        inside.store(true, Ordering::SeqCst);
        while !go.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        Ok(noul(0.95))
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !entered.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the plan was never judged");
        thread::sleep(Duration::from_millis(1));
    }
    o.stop();
    release.store(true, Ordering::SeqCst);
    run.wait();
    assert!(
        w.lines()
            .iter()
            .all(|l| !l.contains("plan") && !l.contains("judged")),
        "{:#?}",
        w.lines()
    );
    assert!(w.called("herdr agent send-keys").is_empty());
    assert!(o.run_dir("hx-1").join("plan.md").exists());
    assert_eq!(o.ticket("hx-1").status, STATUS_RUNNING);

    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    plans(&w);
    o.cfg.api_key = String::new();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    let (pane, _, _) = plan_question(&w, 1);
    o.answer("hx-1", &pane, Answer::Act(super::judgment::Action::Park));
    run.wait();
    let ts = o.ticket("hx-1");
    assert_eq!(
        (ts.status.as_str(), ts.reason.as_str()),
        (STATUS_PARKED, "by you at implement")
    );
}
