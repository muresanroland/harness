use super::judgment::fake::Fake;
use super::judgment::Action;
use super::stage::{Answer, Ask, Config, Orchestrator};
use super::state::{load_state, STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};
use super::world::{new_world, spawn_ticket, succeed, BdTicket, Prompt, World};
use super::write_file;
use crate::tools::Tools;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const PLAN: &str = "# Plan\n\n- change src/x.rs\n- test: x_works\n";
const REVISED: &str = "# Plan\n\n- change src/x.rs and src/y.rs\n- test: x_works, y_works\n";
const FEEDBACK: &str = "cover y too";

/// TypeSafe's answer to a plan: the Noul's score for yes.
pub(crate) fn noul(score: f64) -> Value {
    json!({ "answers": { "follows": { "type": "noul", "noul": score } } })
}

/// A TypeSafe that answers the plans put to it in turn, by number.
fn typesafe(answer: impl Fn(usize) -> Result<Value, String> + Send + Sync + 'static) -> Arc<Fake> {
    let n = AtomicUsize::new(0);
    Fake::new(move |_| answer(n.fetch_add(1, Ordering::SeqCst)))
}

/// The hook copies the plan into the run directory, and the session stops
/// at the plan dialog.
pub(crate) fn at_dialog(run: &Path, plan: &str) -> (String, String) {
    write_file(&run.join("plan.md"), plan);
    (String::new(), "plan".to_string())
}

/// Implement plans; approved, it finishes ("idle") or settles in `then`;
/// feedback brings the revised plan; every other Stage succeeds.
fn plans(w: &World, then: &'static str) {
    let run = w.repo.join(".harness/runs/hx-1");
    w.session(move |p: &Prompt| match (p.stage.as_str(), p.approved) {
        ("implement", false) => at_dialog(&run, PLAN),
        ("implement", true) if then != "idle" => (String::new(), then.to_string()),
        ("", _) if p.text == FEEDBACK => at_dialog(&run, REVISED),
        _ => succeed(p),
    });
}

/// What the Harness prompts a two-step Plan's session with on approval.
const APPROVED: &str = "implement the approved plan";

/// Implement runs on codex, whose session writes plan.md and STATUS: plan
/// and waits; feedback brings the revised plan, approval the implementation.
fn writes(w: &World) {
    write_file(
        &w.repo.join(".harness/config.json"),
        r#"{"implement": {"app": "codex"}}"#,
    );
    let run = w.repo.join(".harness/runs/hx-1");
    w.session(move |p: &Prompt| {
        let plan = match (p.stage.as_str(), p.text.as_str()) {
            ("implement", _) => PLAN,
            ("", FEEDBACK) => REVISED,
            ("", APPROVED) => {
                write_file(&run.join("implement.md"), "STATUS: done\n");
                return (String::new(), "idle".to_string());
            }
            _ => return succeed(p),
        };
        write_file(&run.join("plan.md"), plan);
        write_file(&run.join("implement.md"), "STATUS: plan\n");
        (String::new(), "idle".to_string())
    });
}

/// How many times a two-step Plan was approved in its pane.
fn approvals(w: &World) -> usize {
    let prompts = w.called("herdr agent prompt ");
    let approved = format!(" {APPROVED}");
    prompts.iter().filter(|c| c.ends_with(&approved)).count()
}

/// bd shows hx-1 with a description and acceptance criteria.
fn bd_show(w: &World) {
    w.hook(|_, argv| {
        (argv.join(" ") == "bd show hx-1 --json").then(|| {
            let issue = json!([{ "id": "hx-1", "title": "Ticket hx-1", "description": "Do x.",
                "acceptance_criteria": "x works", "status": "in_progress", "issue_type": "task" }]);
            Ok(issue.to_string())
        })
    });
}

/// The nth plan Question: its pane, plan, score and kept feedback.
fn plan_question(w: &World, n: usize) -> (String, String, Option<f64>, Option<String>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let asked = w
            .events()
            .into_iter()
            .filter_map(|e| e.ask)
            .filter(|ask| matches!(ask, Ask::Plan { .. }))
            .nth(n - 1);
        if let Some(Ask::Plan {
            pane,
            plan,
            judged,
            feedback,
        }) = asked
        {
            return (pane, plan, judged, feedback);
        }
        assert!(
            Instant::now() < deadline,
            "no plan Question {n}: {:#?}",
            w.lines()
        );
        thread::sleep(Duration::from_millis(1));
    }
}

/// The keys sent to panes, in order.
fn keys(w: &World) -> Vec<String> {
    let sent = w.called("herdr agent send-keys");
    sent.iter()
        .map(|call| call.rsplit(' ').next().unwrap().to_string())
        .collect()
}

/// Implement starts in plan mode with its own settings file, which holds
/// the one hook: the harness binary, as the Shell resolved it, in its
/// hidden mode. The other claude Stages launch as before.
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
    assert_eq!(
        settings,
        json!({ "hooks": { "PreToolUse": [{ "matcher": "ExitPlanMode", "hooks": [{
            "type": "command",
            "command": format!("'/opt/the harness/harness' __plan-hook '{}'", run.join("plan.md").display()),
        }] }] } })
    );
}

/// Implement on codex starts with no plan mode and no settings file, its
/// sandbox writing the Run directory too, and is told
/// to write its plan; claude's is told to use its native plan mode.
#[test]
fn implement_on_codex_starts_with_no_plan_mode_and_is_told_to_write_its_plan() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    writes(&w);
    w.hook(|_, argv| (argv == ["bd", "show", "hx-1"]).then(|| Ok("hx-1 · the Ticket\n".into())));
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 implement started: codex (pane 1-1)");
    w.await_line("hx-1 plan ready in implement (pane 1-1)");

    let run = o.run_dir("hx-1");
    let start = w.called("herdr agent start h-hx-1-implement");
    assert_eq!(start.len(), 1);
    assert!(start[0].contains(" --kind codex "), "{start:?}");
    assert!(
        start[0].ends_with(&format!(
            " -- --sandbox workspace-write --add-dir {}",
            run.display()
        )),
        "{start:?}"
    );
    assert!(!run.join("settings.json").exists());
    let prompt = &w.called("herdr agent prompt")[0];
    assert!(
        prompt.contains("\n- Plan: write plan.md and STATUS: plan\n"),
        "{prompt}"
    );
    // codex's sandbox cannot run bd: its Ticket is shown in the Run directory
    let ticket = run.join("ticket.md");
    assert!(
        prompt.contains(&format!("\n- Ticket file: {}\n", ticket.display())),
        "{prompt}"
    );
    assert_eq!(fs::read_to_string(ticket).unwrap(), "hx-1 · the Ticket\n");

    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    o.run_ticket("hx-1");
    let prompt = &w.called("herdr agent prompt")[0];
    assert!(prompt.contains("\n- Plan: native plan mode\n"), "{prompt}");
}

/// STATUS: plan is a plan ready, with no dialog: yes at the floor prompts
/// "implement the approved plan"; below it the plan Question, whose approve
/// does the same. No key is ever sent.
#[test]
fn status_plan_is_judged_and_approval_prompts_implement_the_approved_plan() {
    for score in [0.9, 0.5] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        writes(&w);
        o.cfg.typesafe = typesafe(move |_| Ok(noul(score)));
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");
        if score < 0.75 {
            let (pane, plan, judged, _) = plan_question(&w, 1);
            assert_eq!((plan.as_str(), judged), (PLAN, Some(score)));
            assert_eq!(approvals(&w), 0);
            o.answer("hx-1", &pane, Answer::Approve);
        }
        run.wait();
        assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN, "{score}");
        assert_eq!(approvals(&w), 1, "{score}");
        assert!(keys(&w).is_empty(), "{score}");
        let lines = w.lines();
        let at = lines
            .iter()
            .position(|l| l == "hx-1 plan ready in implement (pane 1-1)")
            .unwrap_or_else(|| panic!("no plan ready line in {lines:#?}"));
        assert_eq!(
            lines[at + 1..at + 4],
            [
                format!("hx-1 judged: plan follows the Ticket {score:.2}"),
                "hx-1 plan approved".to_string(),
                "hx-1 implemented".to_string(),
            ],
            "{score}"
        );
    }
}

/// Feedback goes into the pane as a prompt, no key sent; the session's next
/// STATUS: plan is judged again, the feedback kept.
#[test]
fn feedback_on_a_written_plan_is_a_prompt_and_the_next_status_plan_is_judged_again() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    writes(&w);
    let fake = typesafe(|n| Ok(noul([0.3, 0.95][n])));
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let (pane, _, _, _) = plan_question(&w, 1);
    o.answer("hx-1", &pane, Answer::Prompt(FEEDBACK.to_string()));
    w.await_line("hx-1 plan sent back with your feedback");
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);

    let prompts: Vec<String> = w
        .called(&format!("herdr agent prompt {pane} "))
        .into_iter()
        .filter(|c| !c.contains("# Implement Stage"))
        .collect();
    assert_eq!(
        prompts,
        [
            format!("herdr agent prompt {pane} {FEEDBACK}"),
            format!("herdr agent prompt {pane} {APPROVED}"),
        ]
    );
    assert!(keys(&w).is_empty());
    let asked = fake.requests();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[1]["state"]["plan"], REVISED);
    assert_eq!(asked[1]["state"]["prior_feedback"], FEEDBACK);
    w.await_line("hx-1 judged: plan follows the Ticket 0.95");
}

/// A worktree the session changed before its plan was approved, HEAD moved
/// or the tree dirty, fails the Plan: the user's Question, no approval sent.
/// The session putting the worktree back and writing a newer plan carries
/// on: that plan is judged, and approved.
#[test]
fn a_worktree_changed_before_approval_fails_the_written_plan() {
    for case in ["HEAD moved", "a dirty tree"] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        writes(&w);
        let run = o.run_dir("hx-1");
        let planned = run.join("plan.md");
        // changed while the first plan is written, put back with the revision
        w.hook(move |_, argv| {
            let changed = fs::read_to_string(&planned).is_ok_and(|plan| plan == PLAN);
            match (case, argv.join(" ").as_str()) {
                ("HEAD moved", "git rev-parse HEAD") => {
                    Some(Ok(if changed { "b2\n" } else { "a1\n" }.to_string()))
                }
                ("a dirty tree", "git status --porcelain") => {
                    Some(Ok(if changed { " M src/x.rs\n" } else { "" }.to_string()))
                }
                _ => None,
            }
        });
        o.cfg.typesafe = typesafe(|_| Ok(noul(0.9)));
        let o = Arc::new(o);
        let mut running = spawn_ticket(o.clone(), "hx-1");

        let stuck = w.await_event(
            "stuck in implement: changed the worktree before its plan was approved (pane 1-1)",
        );
        let Some(Ask::PlanFailed { pane, feedback }) = stuck.ask else {
            panic!("{case}: not a plan failure's Question: {stuck:?}");
        };
        assert_eq!(feedback, None, "{case}");
        assert_eq!(approvals(&w), 0, "{case}");
        assert!(
            w.lines().iter().all(|l| l != "hx-1 plan approved"),
            "{case}"
        );
        assert_eq!(pane, o.ticket("hx-1").panes["implement"], "{case}");

        write_file(&run.join("plan.md"), REVISED);
        write_file(&run.join("implement.md"), "STATUS: plan\n");
        w.await_line("hx-1 carrying on");
        running.wait();
        assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN, "{case}");
        assert_eq!(approvals(&w), 1, "{case}");
        w.await_line("hx-1 plan approved");
    }
}

/// Blocked at the plan dialog with a fresh plan reaches the Noul over the
/// plan and the Ticket; yes at 0.76 approves it with enter, each step a line.
#[test]
fn a_fresh_plan_at_its_dialog_reaches_the_noul_and_yes_at_the_floor_approves_it() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    plans(&w, "idle");
    bd_show(&w);
    let fake = typesafe(|_| Ok(noul(0.76)));
    o.cfg.typesafe = fake.clone();
    o.run_ticket("hx-1");
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);

    assert_eq!(
        fake.requests(),
        [json!({
            "model": "jev-latest",
            "state": {
                "plan": PLAN,
                "ticket": { "id": "hx-1", "title": "Ticket hx-1", "description": "Do x.\n\nAcceptance criteria:\nx works" },
                "prior_feedback": null,
            },
            "questions": { "follows": {
                "type": "noul",
                "instructions": "Does this plan implement the Ticket, all of it and nothing more, without leaving decisions open?",
            } },
        })]
    );
    assert_eq!(keys(&w), ["enter"]);
    let lines = w.lines();
    let at = lines
        .iter()
        .position(|l| l == "hx-1 plan ready in implement (pane 1-1)")
        .unwrap_or_else(|| panic!("no plan ready line in {lines:#?}"));
    assert_eq!(
        lines[at + 1..at + 4],
        [
            "hx-1 judged: plan follows the Ticket 0.76",
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
/// the judged line, then the plan Question, with the plan and the score;
/// approve sends enter.
#[test]
fn below_the_floor_a_no_or_no_judgment_raises_the_plan_question() {
    for (name, answer, said) in [
        (
            "yes below the floor",
            Ok(0.74),
            Some("plan follows the Ticket 0.74"),
        ),
        ("no", Ok(0.12), Some("plan strays from the Ticket 0.88")),
        ("error", Err("401: bad key sk-test"), None),
        ("no key", Ok(1.0), None),
        ("TypeSafe off", Ok(1.0), None),
    ] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        plans(&w, "idle");
        let fake = typesafe(move |_| answer.map(noul).map_err(str::to_string));
        o.cfg.typesafe = fake.clone();
        if name == "no key" {
            o.cfg.api_key = String::new();
        }
        if name == "TypeSafe off" {
            super::write_file(
                &w.repo.join(".harness/config.json"),
                r#"{"typesafe": false}"#,
            );
        }
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");

        let (pane, plan, judged, feedback) = plan_question(&w, 1);
        assert_eq!((plan.as_str(), feedback), (PLAN, None), "{name}");
        assert_eq!(judged, answer.ok().filter(|_| said.is_some()), "{name}");
        assert!(keys(&w).is_empty(), "{name}");
        // the lines, then the Question, which has none of its own
        let texts: Vec<(String, bool)> = w
            .events()
            .iter()
            .map(|e| (e.text.clone(), e.ask.is_some()))
            .filter(|(t, _)| t.starts_with("plan ready") || t.starts_with("judged"))
            .collect();
        let ready = "plan ready in implement (pane 1-1)".to_string();
        let mut want = vec![(ready.clone(), false)];
        want.extend(said.map(|said| (format!("judged: {said}"), false)));
        want.push((ready, true));
        assert_eq!(texts, want, "{name}");
        assert!(
            w.events().iter().all(|e| e.ask.is_none() || !e.panel),
            "{name}"
        );
        if name == "error" {
            assert!(
                w.log().contains(" hx-1 no Judgment: 401: bad key ***\n"),
                "{}",
                w.log()
            );
        }
        assert_eq!(
            fake.requests().len(),
            usize::from(name != "no key" && name != "TypeSafe off"),
            "{name}"
        );

        o.answer("hx-1", &pane, Answer::Approve);
        w.await_line("hx-1 plan approved");
        run.wait();
        assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN, "{name}");
        assert_eq!(keys(&w), ["enter"], "{name}");
        assert!(
            !w.log().contains("sk-test"),
            "{name}: the key is in the log"
        );
    }
}

/// Feedback reads the pane before every key: down a key a call to "Tell
/// Claude what to change", enter, idle in plan mode confirmed, then the
/// prompt. The feedback is kept once sent, and the revised plan is judged
/// again with it. Never esc, never 3.
#[test]
fn feedback_reads_the_pane_before_each_key_and_the_revised_plan_is_judged_again() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    plans(&w, "idle");
    let fake = typesafe(|n| Ok(noul([0.3, 0.95][n])));
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let (pane, _, judged, _) = plan_question(&w, 1);
    assert_eq!(judged, Some(0.3));
    let before = w.calls().len();
    o.answer("hx-1", &pane, Answer::Prompt(FEEDBACK.to_string()));
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
    let key = |k: &str| format!("herdr agent send-keys {pane} {k}");
    assert_eq!(
        driven[..8],
        [
            read.clone(),
            key("down"),
            read.clone(),
            key("down"),
            read.clone(),
            key("enter"),
            read,
            format!("herdr agent prompt {pane} {FEEDBACK}"),
        ]
    );
    assert_eq!(keys(&w), ["down", "down", "enter", "enter"]);

    let asked = fake.requests();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[1]["state"]["plan"], REVISED);
    assert_eq!(asked[1]["state"]["prior_feedback"], FEEDBACK); // kept once sent
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

/// Enter goes only to "Tell Claude what to change": past an extra first
/// option it still lands there and the prompt follows; a dropped key or no
/// dialog on screen sends no Enter, and the plan Question comes back with
/// the feedback kept to resend.
#[test]
fn feedback_enters_only_on_tell_claude_what_to_change() {
    for case in [
        "an extra first option",
        "a dropped key",
        "no dialog on screen",
    ] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        plans(&w, "idle");
        o.cfg.api_key = String::new();
        match case {
            "an extra first option" => {
                w.lock().options = [
                    "Yes, clear context and use auto mode",
                    "Yes, and use auto mode",
                    "Yes, manually approve edits",
                    "Tell Claude what to change",
                ]
                .map(str::to_string)
                .to_vec()
            }
            "a dropped key" => w.lock().dropped = true,
            _ => {}
        }
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");
        let (pane, _, _, _) = plan_question(&w, 1);
        if case == "no dialog on screen" {
            w.lock().dialogs.clear(); // still blocked, at some other prompt
        }
        o.answer("hx-1", &pane, Answer::Prompt(FEEDBACK.to_string()));

        let (_, plan, _, kept) = plan_question(&w, 2);
        let prompted = w.called(&format!("herdr agent prompt {pane} {FEEDBACK}"));
        match case {
            "an extra first option" => {
                assert_eq!(keys(&w), ["down", "down", "down", "enter"]);
                assert_eq!(prompted.len(), 1);
                assert_eq!((plan.as_str(), kept), (REVISED, None));
            }
            _ => {
                let (want, why) = match case {
                    // the cursor did not move: no second down, no Enter
                    "a dropped key" => (
                        vec!["down"],
                        "the cursor never reached Tell Claude what to change",
                    ),
                    _ => (vec![], "the plan dialog is not on screen"),
                };
                assert_eq!(keys(&w), want, "{case}");
                assert!(prompted.is_empty(), "{case}");
                w.await_line(&format!("hx-1 feedback not sent: {why} (pane 1-1)"));
                assert_eq!(
                    (plan.as_str(), kept.as_deref()),
                    (PLAN, Some(FEEDBACK)),
                    "{case}"
                );
                assert!(load_state(&w.repo).unwrap().tickets["hx-1"]
                    .feedback
                    .is_empty());
            }
        }
        o.answer("hx-1", &pane, Answer::Act(Action::Park));
        run.wait();
        assert_eq!(o.ticket("hx-1").status, STATUS_PARKED, "{case}");
    }
}

/// A split, a plan model other than Implement's, starts opusplan with its
/// halves remapped in the settings, which also show the clear-context
/// option and hold the switch hook; the plan is approved on that option,
/// the cursor moved to it, up or down. No split keeps plain
/// --model, today's settings, and enter on option 1.
#[test]
fn a_split_starts_opusplan_and_approves_by_clearing_the_context() {
    let split = r#"{"implement": {"model": "claude-opus-5-5", "effort": "high", "plan_model": "claude-fable-5-1"}}"#;
    let clear_second = [
        "Yes, and use auto mode",
        "Yes, clear context and use auto mode",
        "Yes, manually approve edits",
        "Tell Claude what to change",
    ];
    let mut clear_first = clear_second;
    clear_first.swap(0, 1);
    for (config, options, cursor, want) in [
        (split, clear_second, 0, vec!["down", "enter"]),
        (split, clear_second, 2, vec!["up", "enter"]),
        (split, clear_first, 0, vec!["enter"]),
        (
            r#"{"implement": {"model": "opus", "effort": "high", "plan_model": "opus"}}"#,
            clear_second,
            0,
            vec!["enter"],
        ),
        (
            r#"{"implement": {"model": "opus", "effort": "high"}}"#,
            clear_second,
            0,
            vec!["enter"],
        ),
    ] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        write_file(&w.repo.join(".harness/config.json"), config);
        plans(&w, "idle");
        bd_show(&w);
        o.cfg.typesafe = typesafe(|_| Ok(noul(0.9)));
        w.lock().options = options.map(str::to_string).to_vec();
        w.lock().cursor = cursor;
        o.run_ticket("hx-1");
        assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN, "{config}");

        let run = o.run_dir("hx-1");
        let start = &w.called("herdr agent start h-hx-1-implement")[0];
        let settings = fs::read_to_string(run.join("settings.json")).unwrap();
        let settings: Value = serde_json::from_str(&settings).unwrap();
        let plan_hook = json!([{ "matcher": "ExitPlanMode", "hooks": [{
            "type": "command",
            "command": format!("'/opt/the harness/harness' __plan-hook '{}'", run.join("plan.md").display()),
        }] }]);
        assert_eq!(keys(&w), want, "{config} {options:?}");
        if config == split {
            assert!(
                start.ends_with(" --model opusplan --effort high"),
                "{start}"
            );
            let log = w.repo.join(".harness/orchestrator.log");
            assert_eq!(
                settings,
                json!({
                    "env": {
                        "ANTHROPIC_DEFAULT_OPUS_MODEL": "claude-fable-5-1",
                        "ANTHROPIC_DEFAULT_SONNET_MODEL": "claude-opus-5-5",
                    },
                    "showClearContextOnPlanAccept": true,
                    "hooks": {
                        "PreToolUse": plan_hook,
                        "PostModelSwitch": [{ "hooks": [{
                            "type": "command",
                            "command": format!("'/opt/the harness/harness' __switch-hook '{}' 'hx-1'", log.display()),
                        }] }],
                    },
                })
            );
            w.await_line(
                "hx-1 implement started: claude claude-fable-5-1→claude-opus-5-5/high (pane 1-1)",
            );
        } else {
            assert!(start.ends_with(" --model opus --effort high"), "{start}");
            assert_eq!(settings, json!({ "hooks": { "PreToolUse": plan_hook } }));
            w.await_line("hx-1 implement started: claude opus/high (pane 1-1)");
        }
    }
}

/// A split whose dialog shows no clear-context option gets no Enter: the
/// cursor stops at the last option, and it is a plan failure for the user.
#[test]
fn a_split_with_no_clear_context_option_is_a_plan_failure() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    write_file(
        &w.repo.join(".harness/config.json"),
        r#"{"implement": {"model": "claude-opus-5-5", "plan_model": "claude-fable-5-1"}}"#,
    );
    plans(&w, "idle");
    o.cfg.typesafe = typesafe(|_| Ok(noul(0.9)));
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let stuck =
        w.await_event("stuck in implement: the cursor never reached Yes, clear context (pane 1-1)");
    let Some(Ask::PlanFailed { pane, .. }) = stuck.ask else {
        panic!("not a plan failure's Question: {stuck:?}");
    };
    assert_eq!(keys(&w), ["down", "down", "down"]);
    o.answer("hx-1", &pane, Answer::Act(Action::Park));
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PARKED);
}

/// A plan that changes while the Noul judges it gets no Enter: the new one
/// is judged, and only then approved.
#[test]
fn a_plan_changed_during_its_judgment_gets_no_enter_for_the_old_one() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    plans(&w, "idle");
    let (world, run) = (w.clone(), o.run_dir("hx-1"));
    let before_second = Arc::new(AtomicUsize::new(usize::MAX));
    let seen = before_second.clone();
    let fake = typesafe(move |n| {
        if n == 0 {
            write_file(&run.join("plan.md"), REVISED); // the session revised it meanwhile
        } else {
            seen.store(keys(&world).len(), Ordering::SeqCst);
        }
        Ok(noul(0.9))
    });
    o.cfg.typesafe = fake.clone();
    o.run_ticket("hx-1");
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);

    let plans: Vec<Value> = fake
        .requests()
        .iter()
        .map(|r| r["state"]["plan"].clone())
        .collect();
    assert_eq!(plans, [PLAN, REVISED]);
    assert_eq!(
        before_second.load(Ordering::SeqCst),
        0,
        "an Enter went to the old plan"
    );
    assert_eq!(keys(&w), ["enter"]);
}

/// Feedback for a plan the session has since replaced sends no key: the
/// newer plan is judged and asked about instead.
#[test]
fn feedback_for_a_replaced_plan_sends_no_key() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    plans(&w, "idle");
    o.cfg.api_key = String::new();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    let (pane, _, _, _) = plan_question(&w, 1);
    write_file(&o.run_dir("hx-1").join("plan.md"), REVISED); // presented within a tick
    o.answer("hx-1", &pane, Answer::Prompt(FEEDBACK.to_string()));

    let (_, plan, _, kept) = plan_question(&w, 2);
    assert_eq!((plan.as_str(), kept), (REVISED, None));
    assert!(keys(&w).is_empty(), "{:?}", keys(&w));
    assert!(w
        .called(&format!("herdr agent prompt {pane} {FEEDBACK}"))
        .is_empty());
    o.answer("hx-1", &pane, Answer::Act(Action::Park));
    run.wait();
}

/// A new process over the saved state, its Events to the same place.
fn restarted(w: &Arc<World>, o: &Orchestrator, typesafe: Arc<Fake>) -> Arc<Orchestrator> {
    let mut cfg = Config::for_tests(w.clone(), &w.repo, &w.home);
    cfg.events = o.cfg.events.clone();
    cfg.typesafe = typesafe;
    Arc::new(Orchestrator::with_state(cfg, load_state(&w.repo).unwrap()))
}

/// Only the plan dialog on screen is a plan ready: a blocked Implement with
/// no plan.md is the ordinary blocked Question, and so is a permission
/// prompt with a stale plan.md, after /stop-work left a plan Question the
/// user then answered in the pane.
#[test]
fn a_prompt_that_is_not_the_plan_dialog_is_the_ordinary_blocked_question() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| (String::new(), "blocked".to_string()));
    let fake = typesafe(|_| Ok(noul(0.9)));
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    let blocked = w.await_event("waiting at a prompt in implement (pane 1-1)");
    assert!(
        matches!(blocked.ask, Some(Ask::Blocked { .. })),
        "{blocked:?}"
    );
    o.command("park-hx-1");
    run.wait();
    assert!(
        fake.requests().is_empty(),
        "a prompt without a plan was judged"
    );

    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    plans(&w, "blocked"); // approved, it stops at a permission prompt
    o.cfg.typesafe = typesafe(|_| Ok(noul(0.3)));
    let o = Arc::new(o);
    let (pane, _, _, _) = {
        let _run = spawn_ticket(o.clone(), "hx-1");
        plan_question(&w, 1)
    }; // /stop-work with the plan Question up
    w.run(&w.repo, &["herdr", "agent", "send-keys", &pane, "enter"])
        .unwrap(); // approved in the pane
    assert_eq!(w.lock().agents[&pane], "blocked");
    assert!(o.run_dir("hx-1").join("plan.md").exists());

    let fake = typesafe(|_| Ok(noul(0.9)));
    let o = restarted(&w, &Arc::try_unwrap(o).ok().unwrap(), fake.clone());
    let mut run = spawn_ticket(o.clone(), "hx-1");
    let blocked = w.await_nth("waiting at a prompt in implement (pane 1-1)", 1);
    assert!(
        matches!(blocked.ask, Some(Ask::Blocked { .. })),
        "{blocked:?}"
    );
    assert!(
        fake.requests().is_empty(),
        "a stale plan was judged at a permission prompt"
    );
    write_file(&o.run_dir("hx-1").join("implement.md"), "STATUS: done\n");
    w.lock().agents.insert(pane, "idle".to_string());
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);
}

/// /stop-work while TypeSafe judges a plan: nothing is said or sent. Park
/// at the plan Question parks.
#[test]
fn stop_during_the_plan_judgment_takes_no_action_and_park_parks() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    plans(&w, "idle");
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
    assert!(keys(&w).is_empty());
    assert_eq!(o.ticket("hx-1").status, STATUS_RUNNING);

    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    plans(&w, "idle");
    o.cfg.api_key = String::new();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    let (pane, _, _, _) = plan_question(&w, 1);
    o.answer("hx-1", &pane, Answer::Act(Action::Park));
    run.wait();
    let ts = o.ticket("hx-1");
    assert_eq!(
        (ts.status.as_str(), ts.reason.as_str()),
        (STATUS_PARKED, "by you at implement")
    );
}

/// A plan failure is the user's Question, never the Wake
/// Judgment's: open the pane, park, retry; retry starts a fresh session,
/// whose plan is judged afresh.
#[test]
fn a_plan_failure_is_the_users_question_not_the_wake_judgment() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    plans(&w, "idle");
    w.fail_once("herdr agent send-keys", "pane gone");
    let fake = typesafe(|_| Ok(noul(0.9)));
    o.cfg.typesafe = fake.clone();
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let stuck = w.await_event("stuck in implement: never took the answer to its plan: ");
    let Some(Ask::PlanFailed { pane, feedback }) = stuck.ask.clone() else {
        panic!("not a plan failure's Question: {stuck:?}");
    };
    assert!(stuck.text.ends_with("pane gone (pane 1-1)"), "{stuck:?}");
    assert_eq!(feedback, None);
    assert_eq!(fake.requests().len(), 1, "the Wake Judgment was asked");
    o.answer("hx-1", &pane, Answer::Act(Action::Retry));
    w.await_line("hx-1 retrying implement with a fresh session (pane 1-1)");
    run.wait();
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);
    assert_eq!(fake.requests().len(), 2);
    assert_eq!(keys(&w), ["enter", "enter"]);
}

/// Planning spends none of Implement's time: no deadline runs while the
/// plan Question waits, and approval, answered or in the pane, starts a
/// full one.
#[test]
fn planning_does_not_use_up_the_implement_deadline() {
    for in_pane in [false, true] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        plans(&w, "working");
        let timeout = Duration::from_millis(60);
        o.cfg.timeout = Some(timeout);
        o.cfg.api_key = String::new();
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");
        let (pane, _, _, _) = plan_question(&w, 1);
        thread::sleep(3 * timeout);
        assert!(
            w.lines().iter().all(|l| !l.contains("stuck in")),
            "{:#?}",
            w.lines()
        );

        let approved = Instant::now();
        match in_pane {
            true => drop(w.run(&w.repo, &["herdr", "agent", "send-keys", &pane, "enter"])),
            false => o.answer("hx-1", &pane, Answer::Approve),
        }
        w.await_line("hx-1 stuck in implement: timed out after 60ms (pane 1-1)");
        assert!(
            approved.elapsed() >= timeout,
            "in the pane {in_pane}: the deadline did not start at approval"
        );
        o.answer("hx-1", &pane, Answer::Act(Action::Park));
        run.wait();
        assert_eq!(o.ticket("hx-1").status, STATUS_PARKED);
    }
}
