use super::herdr::PaneInfo;
use super::state::STATUS_PR_OPEN;
use super::world::{new_world, spawn_ticket, succeed, BdTicket, Prompt, World};
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
        got.len() == 1 && got[0].ends_with(".orqadence/worktrees/hx-12 --branch hx-12"),
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
    // Implement plans first (harness-kqe.13).
    for want in [
        "--kind claude".to_string(),
        "--permission-mode plan".to_string(),
        format!("--add-dir {}", o.run_dir("hx-12").display()),
    ] {
        assert!(start.contains(&want), "agent start lacks {want:?}: {start}");
    }
    // The panel is told where the session is and that it finished.
    w.await_line("hx-12 implement started: claude (pane 2-1)");
    w.await_line("hx-12 implemented");
    w.await_line("hx-12 review 1 started: codex (pane 2-2)");
    assert!(
        !w.lines().iter().any(|l| l.contains("prompted")),
        "'prompted' is for the log alone: {:?}",
        w.lines()
    );
    assert!(
        w.log()
            .contains(" hx-12 implement prompted, waiting for implement.md\n"),
        "log:\n{}",
        w.log()
    );
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
    w.await_line("hx-1 review 1 found 0 findings");
    w.await_line("hx-1 debate 1 settled: 0 to fix, 0 skipped");
    w.await_line("hx-1 fix 1 done");
    assert_eq!(
        w.await_line("hx-1 PR #hx-1 opened"),
        "hx-1 PR #hx-1 opened after 1 round",
        "the panel line carries no url"
    );
    assert!(
        w.log()
            .contains(" hx-1 PR #hx-1 opened after 1 round (https://example.test/pr/hx-1)\n"),
        "the log line adds the url:\n{}",
        w.log()
    );
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
    w.await_line("hx-1 PR #hx-1 opened after 3 rounds");
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

    w.await_line("hx-1 PR #hx-1 opened after 1 round");
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

#[test]
fn a_prepared_worktree_links_the_checkouts_skills_and_hides_the_links() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    // The Review runs in the Run directory, pruned once the PR is open: look
    // while it runs.
    let review = o.run_dir("hx-1").join(".agents/skills/stage-review");
    let seen = Arc::new(Mutex::new(false));
    let saw = seen.clone();
    w.session(move |p| {
        *saw.lock().unwrap() |= review.join("SKILL.md").exists();
        succeed(p)
    });

    o.run_ticket("hx-1");

    assert!(*seen.lock().unwrap(), "the Run directory got no links");

    // The fake world's init put the skills in the checkout.
    let skill = w.repo.join(".orqadence/skills/stage-implement");
    for sub in [".claude/skills", ".agents/skills"] {
        let link = o.worktree("hx-1").join(sub).join("stage-implement");
        assert_eq!(
            std::fs::read_link(&link).ok(),
            Some(skill.clone()),
            "{link:?}"
        );
    }
    let exclude = std::fs::read_to_string(w.repo.join(".git/info/exclude")).unwrap_or_default();
    for line in [
        "/.claude/skills/stage-implement",
        "/.agents/skills/stage-implement",
    ] {
        assert!(exclude.lines().any(|l| l == line), "{line}:\n{exclude}");
    }
}

#[test]
fn a_resumed_tickets_worktree_is_linked_too() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    // Made before init put the skills in the checkout, say.
    std::fs::create_dir_all(o.worktree("hx-1")).unwrap();
    w.session(succeed);

    o.run_ticket("hx-1");

    let link = o.worktree("hx-1").join(".claude/skills/stage-implement");
    assert_eq!(
        std::fs::read_link(&link).ok(),
        Some(w.repo.join(".orqadence/skills/stage-implement")),
        "{link:?}"
    );
}

/// A Review or a Debate that leaves the worktree dirty, or commits, is put
/// back to the HEAD recorded before it through git, and said on RECENT.
#[test]
fn a_review_or_debate_that_dirties_or_commits_the_worktree_is_restored_and_says_so() {
    // The world names a Stage by its result file: the Debate's is verdict.
    for (stage, prompt) in [("review", "review"), ("debate", "verdict")] {
        for (head, status) in [("a11ce", " M src/lib.rs\n"), ("c0ffee", "")] {
            let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
            // The worktree as git reports it: HEAD and the status's output.
            let tree = Arc::new(Mutex::new(("a11ce".to_string(), String::new())));
            let changed = tree.clone();
            w.session(move |p| {
                if p.stage == prompt {
                    *changed.lock().unwrap() = (head.to_string(), status.to_string());
                }
                succeed(p)
            });
            let git = tree.clone();
            w.hook(move |_, argv| {
                let mut tree = git.lock().unwrap();
                match argv {
                    ["git", "rev-parse", "HEAD"] => Some(Ok(format!("{}\n", tree.0))),
                    ["git", "status", "--porcelain"] => Some(Ok(tree.1.clone())),
                    ["git", "reset", "--hard", to] => {
                        *tree = (to.to_string(), String::new());
                        Some(Ok(String::new()))
                    }
                    _ => None,
                }
            });
            o.run_ticket("hx-1");

            w.await_line(&format!("hx-1 {stage} 1 changed the worktree: restored"));
            let case = format!("{stage} {head} {status:?}");
            assert_eq!(w.called("git reset --hard a11ce").len(), 1, "{case}");
            assert_eq!(w.called("git clean -fd").len(), 1, "{case}");
            w.await_line("hx-1 PR #hx-1 opened");
        }
    }
}

/// A Review parked after it dirtied the worktree has its session ended and
/// the tree put back at once, since the Ticket may never continue. The
/// parked tree is then the user's: an edit made while parked survives the
/// continued Review.
#[test]
fn a_review_parked_after_dirtying_the_worktree_is_restored_at_once() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let status = Arc::new(Mutex::new(String::new()));
    let changed = status.clone();
    w.session(move |p| {
        if p.stage != "review" {
            return succeed(p);
        }
        *changed.lock().unwrap() = " M src/lib.rs\n".to_string();
        (String::new(), "blocked".to_string())
    });
    let git = status.clone();
    w.hook(move |_, argv| match argv {
        ["git", "status", "--porcelain"] => Some(Ok(git.lock().unwrap().clone())),
        ["git", "reset", "--hard", _] => {
            git.lock().unwrap().clear();
            Some(Ok(String::new()))
        }
        _ => None,
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 waiting at a prompt in review 1");
    o.command("park-hx-1");
    run.wait();

    w.await_line("hx-1 review 1 changed the worktree: restored");
    w.await_line("hx-1 parked: by you at review 1");
    assert_eq!(w.called("git clean -fd").len(), 1);
    assert!(!o.run_dir("hx-1").join("before-review-1.json").exists());
    // no session is left to write to the tree once it is put back
    assert_eq!(w.called("herdr pane close").len(), 1);
    assert!(!o.ticket("hx-1").panes.contains_key("review"));

    // the user edits the parked tree, then continues with a fresh Review
    *status.lock().unwrap() = " M notes.md\n".to_string();
    w.session(succeed);
    o.run_ticket("hx-1");
    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(w.called("git clean -fd").len(), 1, "user edit reset");
}

/// A Review parked after changing a worktree it cannot put back says so in
/// the park reason, and keeps its snapshot for the continued Review.
#[test]
fn a_parked_review_whose_tree_is_not_restored_says_so() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let tree = Arc::new(Mutex::new(" M src/lib.rs\n"));
    let reviewed = tree.clone();
    w.session(move |p| {
        if p.stage != "review" {
            return succeed(p);
        }
        *reviewed.lock().unwrap() = " M src/lib.rs\n M src/main.rs\n";
        (String::new(), "blocked".to_string())
    });
    w.hook(move |_, argv| match argv {
        ["git", "status", "--porcelain"] => Some(Ok(tree.lock().unwrap().to_string())),
        _ => None,
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 waiting at a prompt in review 1");
    o.command("park-hx-1");
    run.wait();

    w.await_line(
        "hx-1 parked: by you at review 1; \
         review 1 changed a worktree already dirty before it, not restored",
    );
    assert!(o.run_dir("hx-1").join("before-review-1.json").exists());
}

/// A Ticket parked because its Review changed a worktree already dirty is
/// guarded again when continued, though its Review is done: it parks again,
/// with no Debate, until the tree is put back by hand.
#[test]
fn a_continued_ticket_is_guarded_against_its_done_review() {
    let parked = "hx-1 parked: review 1 changed a worktree already dirty before it, not restored";
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let tree = Arc::new(Mutex::new(" M src/lib.rs\n"));
    let reviewed = tree.clone();
    w.session(move |p| {
        if p.stage == "review" {
            *reviewed.lock().unwrap() = " M src/lib.rs\n M src/main.rs\n";
        }
        succeed(p)
    });
    let git = tree.clone();
    w.hook(move |_, argv| match argv {
        ["git", "status", "--porcelain"] => Some(Ok(git.lock().unwrap().to_string())),
        _ => None,
    });

    for times in 1..=2 {
        o.run_ticket("hx-1");
        let lines = w.lines();
        assert_eq!(lines.iter().filter(|l| l.contains(parked)).count(), times);
        assert_eq!(stages_run(&w), ["implement", "review"]);
    }
    *tree.lock().unwrap() = " M src/lib.rs\n";
    o.run_ticket("hx-1");
    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(stages_run(&w), ["implement", "review", "debate", "fix"]);
}

#[test]
fn a_clean_review_changes_nothing() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    assert!(
        !w.called("git rev-parse HEAD").is_empty(),
        "HEAD never recorded"
    );
    assert!(
        w.called("git reset").is_empty() && w.called("git clean").is_empty(),
        "{}",
        w.calls().join("\n")
    );
    assert!(!w.lines().iter().any(|l| l.contains("changed the worktree")));
}

/// A resumed run skips a Review already done, and its guard with it: the
/// tree may hold a later Stage's work.
#[test]
fn a_review_already_done_is_not_guarded_again() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    for name in ["implement.md", "review-1.md"] {
        write_file(&o.run_dir("hx-1").join(name), "STATUS: done\n");
    }
    w.hook(|_, argv| match argv {
        ["git", "status", "--porcelain"] => Some(Ok(" M src/lib.rs\n".to_string())),
        _ => None,
    });
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    assert!(w.called("git reset").is_empty(), "{}", w.calls().join("\n"));
}

/// A worktree already dirty before the Review holds work that is not the
/// Review's: it is never reset. Left as it was, the Pipeline goes on; changed
/// by the Review, even only in the content of a file already changed or
/// untracked, the Ticket parks with the work kept.
#[test]
fn a_worktree_dirty_before_the_review_is_never_reset() {
    let parked = "hx-1 parked: review 1 changed a worktree already dirty before it, not restored";
    // The tree as git reports it: the status, `git diff HEAD`, the one
    // untracked file's path and its hash.
    let dirty = (
        " M src/lib.rs\n?? notes/\n",
        "+one\n",
        "notes/a.txt\0",
        "e69de29\n",
    );
    for (after, want) in [
        (dirty, "hx-1 PR #hx-1 opened"),
        ((" M src/lib.rs\n", dirty.1, dirty.2, dirty.3), parked),
        ((dirty.0, "+two\n", dirty.2, dirty.3), parked),
        ((dirty.0, dirty.1, "notes/b.txt\0", dirty.3), parked),
        ((dirty.0, dirty.1, dirty.2, "d00491f\n"), parked),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        let tree = Arc::new(Mutex::new(dirty));
        let reviewed = tree.clone();
        w.session(move |p| {
            if p.stage == "review" {
                *reviewed.lock().unwrap() = after;
            }
            succeed(p)
        });
        w.hook(move |_, argv| {
            let tree = tree.lock().unwrap();
            match argv {
                ["git", "status", "--porcelain"] => Some(Ok(tree.0.to_string())),
                ["git", "diff", "HEAD", "--binary"] => Some(Ok(tree.1.to_string())),
                ["git", "ls-files", "--others", ..] => Some(Ok(tree.2.to_string())),
                ["git", "hash-object", "--", _] => Some(Ok(tree.3.to_string())),
                _ => None,
            }
        });
        o.run_ticket("hx-1");

        w.await_line(want);
        assert!(
            w.called("git reset").is_empty() && w.called("git clean").is_empty(),
            "{}",
            w.calls().join("\n")
        );
    }
}
