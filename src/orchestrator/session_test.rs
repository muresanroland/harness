//! Session ids in the State, and /continue resuming a Stage by its id.

use super::question_test::ASKS;
use super::stage::{Answer, Orchestrator};
use super::state::{load_state, Session};
use super::world::{
    new_world, restarted, spawn_epic, spawn_ticket, succeed, wait_until, working, BdTicket, World,
};
use super::write_file;
use std::sync::Arc;

/// hx-1 stopped as a killed run leaves it: its `stage` session working in
/// its pane, with its id saved when herdr's integration is installed (`ids`).
fn stopped_at(stage: &'static str, label: &str, ids: bool) -> (Arc<World>, Arc<Orchestrator>) {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().integration = ids;
    w.session(move |p| match p.stage == stage {
        true => working(p),
        false => succeed(p),
    });
    let o = Arc::new(o);
    let run = spawn_ticket(o.clone(), "hx-1");
    w.await_line(&format!("hx-1 {label} started"));
    if ids {
        wait_until("the session id saved", || {
            !o.ticket("hx-1").sessions[stage].id.is_empty()
        });
    }
    drop(run); // stops the run and joins it
    (w, o)
}

/// herdr restarted without its panes: every session is gone.
fn panes_gone(w: &World) {
    let mut w = w.lock();
    w.agents.clear();
    w.panes.clear();
    w.tabs.clear();
}

/// The Stage's agent starts since call `before`.
fn starts(w: &World, before: usize, stage: &str) -> Vec<String> {
    w.since(before, &format!("herdr agent start h-hx-1-{stage} "))
}

/// The session in `pane` writes its result later, and goes idle.
fn finishes(w: &World, o: &Orchestrator, pane: &str, file: &str) {
    write_file(&o.run_dir("hx-1").join(file), "STATUS: done\n");
    w.lock().agents.insert(pane.to_string(), "idle".to_string());
}

#[test]
fn a_stage_start_saves_its_session_id_and_app_beside_its_pane() {
    let (w, _o) = stopped_at("review", "review 1", true);

    let ts = load_state(&w.repo).unwrap().tickets["hx-1"].clone();
    let pane = &ts.panes["review"];
    let reported = w.lock().sessions[pane].clone();
    assert_eq!(
        ts.sessions["review"],
        Session {
            app: "codex".to_string(),
            id: reported,
            reset: None,
        },
        "state.json keeps the Review's session id, as herdr's agent get reported it, and its App"
    );
}

#[test]
fn another_agent_in_the_stage_pane_never_has_its_session_id_saved() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().integration = true;
    w.session(working);
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");
    wait_until("the session id saved", || {
        o.ticket("hx-1")
            .sessions
            .get("implement")
            .is_some_and(|s| !s.id.is_empty())
    });
    let saved = o.ticket("hx-1").sessions["implement"].id.clone();
    let pane = o.ticket("hx-1").panes["implement"].clone();
    let before = w.calls().len();
    {
        let mut world = w.lock(); // another agent in that pane now
        world.names.remove("h-hx-1-implement");
        world.names.insert("someone-else".to_string(), pane.clone());
        world.sessions.insert(pane.clone(), "other".to_string());
    }
    let get = format!("herdr agent get {pane}");
    wait_until("the pane watched twice more", || {
        w.calls()[before..].iter().filter(|c| **c == get).count() >= 2
    });

    assert_eq!(
        o.ticket("hx-1").sessions["implement"].id,
        saved,
        "another agent's session id was saved as the Stage's"
    );
}

#[test]
fn stopped_run_whose_pane_is_gone_resumes_the_stage_by_its_session_id() {
    // (Stage, its label, its App, result file, the resume args)
    let cases = [
        (
            "implement",
            "implement",
            "claude",
            "implement.md",
            "--resume",
        ),
        ("review", "review 1", "codex", "review-1.md", "resume"),
    ];
    for (stage, label, app, file, resume) in cases {
        let (w, stopped) = stopped_at(stage, label, true);
        let id = stopped.ticket("hx-1").sessions[stage].id.clone();
        panes_gone(&w);
        let o = restarted(&w, &stopped);
        w.session(|p| match p.text == "continue" {
            true => working(p),
            false => succeed(p),
        });
        let before = w.calls().len();
        let mut run = spawn_ticket(o.clone(), "hx-1");

        let pane = || o.ticket("hx-1").panes[stage].clone();
        wait_until(&format!("{label} told to continue"), || {
            w.calls()[before..].contains(&format!("herdr agent prompt {} continue", pane()))
        });
        let pane = pane();
        finishes(&w, &o, &pane, file);
        run.wait();

        w.await_line(&format!("hx-1 {label} resumed: {app} (pane"));
        w.await_line("hx-1 PR #hx-1 opened");
        let starts = starts(&w, before, stage);
        let want = format!("--kind {app} --pane {pane} -- {resume} {id} ");
        assert!(
            starts.len() == 1 && starts[0].contains(&want),
            "{label} starts after the restart = {starts:?}, want one resuming by id: {want:?}"
        );
        let prompts = w.since(before, &format!("herdr agent prompt {pane} "));
        assert_eq!(
            prompts,
            [format!("herdr agent prompt {pane} continue")],
            "the resumed {label} is prompted with continue alone, never its Stage skill"
        );
    }
}

#[test]
fn a_stage_resumed_by_id_that_asked_is_put_its_question_not_continue() {
    let (w, stopped) = stopped_at("implement", "implement", true);
    write_file(&stopped.run_dir("hx-1").join("implement.md"), ASKS);
    panes_gone(&w);
    let o = restarted(&w, &stopped);
    w.session(|p| match p.text.as_str() {
        "ours" => (String::new(), "working".to_string()),
        _ => succeed(p),
    });
    let before = w.calls().len();
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let asked = w.await_event("question in implement");
    let pane = o.ticket("hx-1").panes["implement"].clone();
    assert!(asked.text.ends_with(&o.locate(&pane)), "{}", asked.text);
    o.answer("hx-1", &pane, Answer::Prompt("ours".to_string()));
    w.await_line("hx-1 sent your answer");
    finishes(&w, &o, &pane, "implement.md");
    run.wait();

    w.await_line("hx-1 implement resumed: claude (pane");
    assert_eq!(starts(&w, before, "implement").len(), 1);
    assert_eq!(
        w.since(before, &format!("herdr agent prompt {pane} ")),
        [format!("herdr agent prompt {pane} ours")],
        "the resumed session was prompted before its answer"
    );
}

#[test]
fn a_live_pane_is_watched_even_with_a_saved_session_id() {
    let (w, stopped) = stopped_at("review", "review 1", true);
    let o = restarted(&w, &stopped);
    let pane = o.ticket("hx-1").panes["review"].clone();
    let before = w.calls().len();
    let mut run = spawn_ticket(o.clone(), "hx-1");
    wait_until("the live Review watched", || {
        w.calls()[before..].contains(&format!("herdr agent get {pane}"))
    });
    finishes(&w, &o, &pane, "review-1.md");
    run.wait();

    w.await_line("hx-1 PR #hx-1 opened");
    let starts = starts(&w, before, "review");
    let prompted = w.since(before, &format!("herdr agent prompt {pane} "));
    assert!(
        starts.is_empty() && prompted.is_empty(),
        "the live Review is watched, not started {starts:?} or prompted"
    );
}

#[test]
fn a_changed_app_or_no_session_id_starts_the_stage_fresh() {
    // (Stage, its label, herdr's integration, config.json, the App it starts on)
    let cases = [
        (
            "review",
            "review 1",
            true,
            r#"{"review": {"app": "claude"}}"#,
            "claude",
        ),
        ("implement", "implement", false, "", "claude"),
    ];
    for (stage, label, ids, config, app) in cases {
        let (w, stopped) = stopped_at(stage, label, ids);
        if !config.is_empty() {
            write_file(&w.repo.join(".orqadence/config.json"), config);
        }
        panes_gone(&w);
        let o = restarted(&w, &stopped);
        w.session(succeed);
        let before = w.calls().len();
        spawn_ticket(o.clone(), "hx-1").wait();

        w.await_line("hx-1 PR #hx-1 opened");
        let starts = starts(&w, before, stage);
        assert!(
            starts.len() == 1
                && starts[0].contains(&format!("--kind {app} "))
                && !starts[0].contains("resume"),
            "{label} starts after the restart = {starts:?}, want one fresh on {app}"
        );
        w.await_line(&format!("hx-1 {label} started: {app} (pane"));
    }
}

#[test]
fn a_failed_resumed_start_or_continue_starts_the_stage_fresh() {
    for failing in ["herdr agent start h-hx-1-review ", "herdr agent prompt "] {
        let (w, stopped) = stopped_at("review", "review 1", true);
        panes_gone(&w);
        let o = restarted(&w, &stopped);
        w.session(succeed);
        w.fail_once(failing, "boom");
        let before = w.calls().len();
        spawn_ticket(o.clone(), "hx-1").wait();

        w.await_line("hx-1 PR #hx-1 opened");
        let starts = starts(&w, before, "review");
        assert!(
            starts.len() == 2 && starts[0].contains("resume") && !starts[1].contains("resume"),
            "with {failing:?} failing, Review starts = {starts:?}, want the resume then a fresh one"
        );
        let pane = |start: &str| {
            start
                .split(' ')
                .skip_while(|a| *a != "--pane")
                .nth(1)
                .unwrap_or_default()
                .to_string()
        };
        let fresh = pane(&starts[1]);
        assert_ne!(
            pane(&starts[0]),
            fresh,
            "the fresh Review starts in a pane of its own"
        );
        let prompts = w.since(before, &format!("herdr agent prompt {fresh} "));
        assert!(
            prompts.len() == 1 && prompts[0].contains("## Inputs"),
            "the fresh Review is prompted with its Stage skill, got {prompts:?}"
        );
        let said = w.await_event("review 1 not resumed: ").text;
        assert!(said.ends_with(": boom, starting it fresh"), "{said}");
    }
}

#[test]
fn retry_of_a_parked_ticket_never_resumes() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    {
        let mut w = w.lock();
        w.integration = true;
        w.merged = true;
    }
    w.session(|p| match p.stage.as_str() {
        "implement" => working(p),
        _ => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 implement started");
    wait_until("the session id saved", || {
        !o.ticket("hx-1").sessions["implement"].id.is_empty()
    });
    o.command("park-hx-1");
    w.await_line("hx-1 parked: by you at implement");

    w.session(succeed);
    o.command("retry-hx-1");
    run.wait();
    o.wait_in_flight();

    let starts = w.called("herdr agent start h-hx-1-implement ");
    assert!(
        starts.len() == 2 && starts.iter().all(|s| !s.contains("resume")),
        "Implement starts = {starts:?}, want the retry a fresh session"
    );
}
