use super::herdr::PaneInfo;
use super::state::STATUS_PR_OPEN;
use super::world::{new_world, succeed, BdTicket, Prompt, World};
use super::write_file;
use std::sync::{Arc, Mutex};

pub(crate) fn stages_run(w: &World) -> Vec<String> {
    w.called("herdr agent start")
        .iter()
        .map(|call| {
            let name = call.split_whitespace().nth(3).unwrap();
            name[name.rfind('-').unwrap() + 1..].to_string()
        })
        .collect()
}

#[test]
fn implement_stage_runs_in_a_ticket_tab_and_reports_to_main() {
    let (w, o) = new_world(vec![BdTicket::new("hx-12")]);
    {
        let mut inner = w.lock();
        inner.tabs = vec!["w1:main-tab".to_string()];
        inner.panes = vec![PaneInfo {
            pane_id: "main".to_string(),
            tab_id: "w1:main-tab".to_string(),
        }];
    }

    o.run_ticket("hx-12");

    let got = w.called("bd worktree create");
    assert!(
        got.len() == 1 && got[0].ends_with(".harness/worktrees/hx-12 --branch hx-12"),
        "worktree calls = {got:?}"
    );
    let got = w.called("bd update hx-12");
    assert!(
        got.len() == 1 && got[0].contains("in_progress"),
        "ticket not marked in_progress: {got:?}"
    );
    let tab = w.called("herdr tab create");
    assert!(
        tab.len() == 1 && tab[0].contains("--label hx-12") && tab[0].contains("--no-focus"),
        "tab create = {tab:?}"
    );
    let start = &w.called("herdr agent start")[0];
    for want in [
        "--kind claude".to_string(),
        "--permission-mode auto".to_string(),
        format!("--add-dir {}", o.run_dir("hx-12").display()),
    ] {
        assert!(start.contains(&want), "agent start lacks {want:?}: {start}");
    }
    // The launching pane is told where the session is and that it finished.
    let started = w.await_line("hx-12 implement started");
    assert!(
        started.ends_with("-> 2-1"),
        "started line does not locate the pane: {started:?}"
    );
    assert!(
        started.contains(&format!("claude in {}", o.worktree("hx-12").display())),
        "started line does not say what runs where: {started:?}"
    );
    w.await_line("hx-12 implement done");
}

#[test]
fn clean_first_verdict_opens_pr_after_one_round() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);

    o.run_ticket("hx-1");

    assert_eq!(stages_run(&w), ["implement", "review", "debate", "fix"]);
    let ts = o.ticket("hx-1");
    assert!(
        ts.status == STATUS_PR_OPEN && ts.pr == "https://example.test/pr/hx-1",
        "state = {ts:?}"
    );
    w.await_line("hx-1 pr open after 1 round(s): https://example.test/pr/hx-1");
    assert_eq!(
        w.called("herdr tab close").len(),
        1,
        "Ticket tab not closed once the PR opened"
    );
}

#[test]
fn review_runs_codex_in_the_run_directory_and_debate_gets_the_api_key() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    o.run_ticket("hx-1");

    let review = &w.called("herdr agent start")[1];
    assert!(
        review.contains("--kind codex") && review.contains("--sandbox workspace-write"),
        "review session = {review}"
    );
    let splits = w.called("herdr pane split");
    assert_eq!(
        splits.len(),
        3,
        "pane splits = {splits:?}, want one per Stage after Implement"
    );
    assert!(
        splits[0].contains(&format!("--cwd {}", o.run_dir("hx-1").display())),
        "Codex pane is not in the run directory: {}",
        splits[0]
    );
    assert!(
        splits[1].contains("--env TYPESAFE_API_KEY=sk-test") && !splits[2].contains("TYPESAFE"),
        "TYPESAFE_API_KEY must reach the Debate pane only: {:?}",
        &splits[1..]
    );
}

#[test]
fn ticket_that_keeps_producing_fix_items_gets_pr_after_exactly_three_rounds() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let fix_prompts: Arc<Mutex<Vec<Prompt>>> = Default::default();
    let seen = fix_prompts.clone();
    w.session(move |p| {
        match p.stage.as_str() {
            "verdict" => {
                return (
                    "STATUS: done\n- [fix] (high) a.go:1 — still broken | reason: agreed | settled: consensus\n".to_string(),
                    "idle".to_string(),
                )
            }
            "fix" => seen.lock().unwrap().push(p.clone()),
            _ => {}
        }
        succeed(p)
    });

    o.run_ticket("hx-1");

    let want = [
        "implement",
        "review",
        "debate",
        "fix",
        "review",
        "debate",
        "fix",
        "review",
        "debate",
        "fix",
    ];
    assert_eq!(stages_run(&w), want);
    let fix_prompts = fix_prompts.lock().unwrap();
    assert!(
        fix_prompts.len() == 3
            && !fix_prompts[0].open_pr
            && !fix_prompts[1].open_pr
            && fix_prompts[2].open_pr,
        "only the round 3 Fix session may open the PR: {fix_prompts:?}"
    );
    for round in 1..=3 {
        let file = format!("verdict-{round}.md");
        assert!(
            fix_prompts[2].text.contains(&file),
            "last Fix prompt lacks {file} for the PR's Verdict history"
        );
    }
    w.await_line("hx-1 pr open after 3 round(s)");
}

#[test]
fn open_pr_prunes_build_scratch_and_keeps_evidence() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    // The Review Stage compiles the branch, and the run directory is the only
    // place its sandbox may write, so its build cache lands there.
    let run_dir = o.run_dir("hx-1");
    let dir = run_dir.clone();
    w.session(move |p| {
        if p.stage == "review" {
            write_file(&dir.join(".review-cache/ab/obj-a"), "go object data");
            write_file(&dir.join("check-testharness"), "a compiled test binary");
            write_file(&dir.join("diff-1.patch"), "the diff it reviewed");
        }
        succeed(p)
    });

    o.run_ticket("hx-1");

    w.await_line("hx-1 pr open after 1 round(s)");
    for gone in [".review-cache", "check-testharness"] {
        assert!(
            !run_dir.join(gone).exists(),
            "{gone} still in the run directory after the PR opened"
        );
    }
    for kept in [
        "implement.md",
        "review-1.md",
        "verdict-1.md",
        "fix-1.md",
        "diff-1.patch",
    ] {
        assert!(run_dir.join(kept).exists(), "evidence pruned: {kept}");
    }
}
