//! The App table and .harness/config.json: each Stage's App, model and
//! effort, read when the Stage starts.

use super::app::app;
use super::world::{new_world, spawn_ticket, succeed, BdTicket, World};
use super::write_file;
use crate::skills::manifest::{Manifest, NONE};
use crate::skills::SKILLS;
use crate::tempdir::TempDir;
use std::sync::Arc;

fn config(w: &World, body: &str) {
    write_file(&w.repo.join(".harness/config.json"), body);
}

/// The args after herdr's `--` that the Stage's session started with.
fn argv(w: &World, stage: &str) -> String {
    let start = &w.called(&format!("herdr agent start h-hx-1-{stage}"))[0];
    start.split_once(" -- ").unwrap().1.to_string()
}

#[test]
fn a_rows_model_and_effort_add_the_flags_on_claude_and_on_codex() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    // Partial: the Review's app is left to its default, codex.
    config(
        &w,
        r#"{
  "implement": {"app": "claude", "model": "opus", "effort": "high"},
  "review": {"model": "gpt-6-sol", "effort": "low"},
  "fix": {"model": "sonnet"}
}"#,
    );
    o.run_ticket("hx-1");

    let run = o.run_dir("hx-1").display().to_string();
    assert!(
        argv(&w, "implement").ends_with(&format!("--add-dir {run} --model opus --effort high")),
        "{}",
        argv(&w, "implement")
    );
    assert_eq!(
        argv(&w, "review"),
        "--sandbox workspace-write -m gpt-6-sol -c model_reasoning_effort=low"
    );
    assert_eq!(
        argv(&w, "fix"),
        format!("--permission-mode auto --add-dir {run} --model sonnet")
    );
    assert!(w.called("herdr agent start h-hx-1-review")[0].contains("--kind codex"));
    w.await_line("hx-1 implement started: claude opus/high (pane 1-1)");
    w.await_line("hx-1 review 1 started: codex gpt-6-sol/low (pane 1-2)");
    w.await_line("hx-1 fix 1 started: claude sonnet (pane 1-4)");
}

/// A Review on claude starts in the Run directory with the worktree added
/// read-only, and waits on claude's trust, not on codex's, its default
/// App's.
#[test]
fn a_review_on_claude_runs_in_the_run_directory_and_trust_is_read_through_the_rows_app() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let home = TempDir::new(); // claude trusts the repo, codex nothing
    write_file(
        &home.path().join(".claude.json"),
        &format!(
            r#"{{"projects": {{"{}": {{"hasTrustDialogAccepted": true}}}}}}"#,
            w.repo.display()
        ),
    );
    o.cfg.home = home.path().to_path_buf();
    config(&w, r#"{"review": {"app": "claude"}}"#);
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");

    let worktree = o.worktree("hx-1").display().to_string();
    w.await_line("hx-1 review 1 started: claude (pane 1-2)");
    assert_eq!(
        argv(&w, "review"),
        format!(
            "--permission-mode auto --add-dir {worktree} --settings \
             {{\"permissions\":{{\"deny\":[\"Edit(/{worktree}/**)\"]}},\"sandbox\":\
             {{\"allowUnsandboxedCommands\":false,\"enabled\":true,\"failIfUnavailable\":true}}}}"
        )
    );
    let split = &w.called("herdr pane split")[0];
    assert!(
        split.contains(&format!("--cwd {} ", o.run_dir("hx-1").display())),
        "{split}"
    );
}

/// A worktree path with a quote or backslash still makes valid settings JSON.
#[test]
fn a_worktree_path_is_escaped_in_the_review_settings_on_claude() {
    let worktree = r#"/tmp/a"b\c"#;
    let args = (app("claude").unwrap().run_dir_args)(worktree);
    let settings: serde_json::Value = serde_json::from_str(args.last().unwrap()).unwrap();
    assert_eq!(
        settings["permissions"]["deny"][0],
        format!("Edit(/{worktree}/**)")
    );
}

#[test]
fn the_moderators_inputs_carry_each_sides_command() {
    let skill = SKILLS
        .iter()
        .find(|(name, _)| *name == "stage-moderate")
        .unwrap()
        .1;
    for own in ["claude -p", "codex exec"] {
        assert!(!skill.contains(own), "stage-moderate still runs {own:?}");
    }
    // Started in the worktree, a claude side is granted the Run directory,
    // its sibling, which holds the diff.
    let claude = "'claude' '--tools' 'Read,Grep,Glob,Skill' '--add-dir' '{run}' '-p'";
    let codex = "'codex' 'exec' '--sandbox' 'read-only'";
    for (body, side_a, side_b) in [
        ("", claude.to_string(), codex.to_string()),
        (
            r#"{
  "side_a": {"app": "codex", "model": "gpt-6-sol", "effort": "low"},
  "side_b": {"app": "claude", "model": "claude-opus-5-5[1m]", "effort": "high"}
}"#,
            format!("{codex} '-m' 'gpt-6-sol' '-c' 'model_reasoning_effort=low'"),
            format!("{claude} '--model' 'claude-opus-5-5[1m]' '--effort' 'high'"),
        ),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        if !body.is_empty() {
            config(&w, body);
        }
        o.run_ticket("hx-1");

        // The Moderator's prompt: the Stage skill's body, then its Inputs.
        let prompt = w
            .called("herdr agent prompt")
            .into_iter()
            .find(|call| call.contains("verdict-1.md"))
            .unwrap();
        let inputs = prompt.split_once("## Inputs").unwrap().1;
        let run = o.run_dir("hx-1").display().to_string();
        for want in [
            format!("- Side A command: {side_a}\n").replace("{run}", &run),
            format!("- Side B command: {side_b}\n").replace("{run}", &run),
        ] {
            assert!(inputs.contains(&want), "{want:?} not in:{inputs}");
        }
    }
}

/// TypeSafe off: the Moderator is told so under Inputs, and its pane gets no
/// key; on, neither changes.
#[test]
fn with_typesafe_off_the_moderator_gets_the_input_and_no_key() {
    for (body, off) in [("", false), (r#"{"typesafe": false}"#, true)] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        if !body.is_empty() {
            config(&w, body);
        }
        o.run_ticket("hx-1");

        let prompt = w
            .called("herdr agent prompt")
            .into_iter()
            .find(|call| call.contains("verdict-1.md"))
            .unwrap();
        let inputs = prompt.split_once("## Inputs").unwrap().1;
        assert_eq!(inputs.contains("- TypeSafe: off\n"), off, "{inputs}");
        let keyed = w
            .called("herdr")
            .iter()
            .any(|c| c.contains("TYPESAFE_API_KEY=sk-test"));
        assert_eq!(keyed, !off, "off {off}: the key reached a pane or not");
    }
}

/// Running Stages keep theirs; the Stages that start after a change use it.
#[test]
fn config_changed_between_two_stages_reaches_the_second() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let file = w.repo.join(".harness/config.json");
    w.session(move |p| {
        if p.stage == "implement" {
            write_file(&file, r#"{"review": {"model": "gpt-6-sol"}}"#);
        }
        succeed(p)
    });
    o.run_ticket("hx-1");

    assert!(!argv(&w, "implement").contains("gpt-6-sol"));
    assert_eq!(argv(&w, "review"), "--sandbox workspace-write -m gpt-6-sol");
}

/// A config.json no Stage can start on, read as a Stage starts, is a Wake
/// before its session starts: unreadable, naming the file, a field not a
/// string, a Stage other than the Review off claude, a plan model split
/// from Implement's model where either is not a full claude- id, none off
/// the Review's fallback, or both Debate sides on one family.
#[test]
fn an_unreadable_config_or_a_stage_off_claude_wakes_the_stage_that_reads_it() {
    const SPLIT_NOT_FULL: &str = "{file}: implement plan_model splits from model: \
         the split needs a full claude- model id for each half";
    for (body, label, reason) in [
        ("{ not json", "implement", "{file}: "),
        (
            r#"{"implement": {"model": 5}}"#,
            "implement",
            "{file}: implement model is not a string",
        ),
        (
            r#"{"moderator": {"app": "codex"}}"#,
            "debate 1",
            "moderator runs on claude only",
        ),
        (
            r#"{"implement": {"plan_model": "claude-fable-5-1"}}"#,
            "implement",
            SPLIT_NOT_FULL,
        ),
        (
            r#"{"implement": {"model": "opus", "plan_model": "claude-fable-5-1"}}"#,
            "implement",
            SPLIT_NOT_FULL,
        ),
        (
            r#"{"implement": {"model": "claude-opus-5-5", "plan_model": "fable"}}"#,
            "implement",
            SPLIT_NOT_FULL,
        ),
        (
            r#"{"side_b": {"app": "pi"}}"#,
            "debate 1",
            r#"{file}: no App named "pi" for side_b"#,
        ),
        (
            r#"{"implement": {"model": "none"}}"#,
            "implement",
            "{file}: implement model none: only review_if_limited takes none",
        ),
        (
            r#"{"side_b": {"app": "claude"}}"#,
            "debate 1",
            "side_a and side_b both run Anthropic models: the Debate needs two families",
        ),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        config(&w, body);
        let o = Arc::new(o);
        let _run = spawn_ticket(o.clone(), "hx-1");

        let file = w.repo.join(".harness/config.json");
        let reason = reason.replace("{file}", &file.display().to_string());
        w.await_line(&format!("hx-1 stuck in {label}: {reason}"));
        let stage = label.split(' ').next().unwrap();
        assert!(
            w.called(&format!("herdr agent start h-hx-1-{stage}"))
                .is_empty(),
            "{body}"
        );
    }
}

/// Records the picks in the world's Skill manifest.
fn pick(w: &World, picks: &[(&str, &str)]) {
    let mut manifest = Manifest::load(&w.repo).unwrap();
    for (job, pick) in picks {
        manifest.picks.insert(job.to_string(), pick.to_string());
    }
    manifest.save(&w.repo).unwrap();
}

/// The prompt the Stage writing `file` was sent.
fn prompt(w: &World, file: &str) -> String {
    w.called("herdr agent prompt")
        .into_iter()
        .find(|call| call.contains(file))
        .unwrap()
}

/// Each job's line names its pick in the mention form of the App it runs
/// on: in words on claude, plugin-qualified for a plugin's skill, $name on
/// codex. A none pick drops the line, and so does a pick not installed,
/// which the Inputs tell the Stage to note; the generic line stays.
#[test]
fn a_jobs_line_names_its_pick_in_the_apps_mention_form_or_is_dropped() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    write_file(&w.repo.join(".claude/skills/tdd/SKILL.md"), "tdd");
    let plugin = TempDir::new();
    write_file(&plugin.path().join("skills/ponytail/SKILL.md"), "lazy");
    let plugins = serde_json::json!([
        {"id": "ponytail@market", "enabled": true, "installPath": plugin.path()},
    ])
    .to_string();
    w.hook(move |_, argv| {
        (argv.join(" ") == "claude plugin list --json").then(|| Ok(plugins.clone()))
    });
    pick(
        &w,
        &[
            ("test-first", "tdd"),
            ("working-mode", "ponytail:ponytail"),
            ("prose", NONE),
            ("review", "review-agent"),
        ],
    );
    o.run_ticket("hx-1");

    let implement = prompt(&w, "implement.md");
    for want in [
        "   Use the tdd skill for it.\n",
        "   Use the ponytail:ponytail skill for all your work",
        "test-first: a failing test, then the code.",
        "read the diff against the acceptance criteria and fix what is missing or wrong.",
        "- Not installed: code-review (self-review): their lines are left out; say so in the result file\n",
    ] {
        assert!(implement.contains(want), "{want:?} not in:\n{implement}");
    }
    for gone in [
        "{{",
        "code-review skill",
        "for your commits and result file",
    ] {
        assert!(!implement.contains(gone), "{gone:?} in:\n{implement}");
    }
    let review = prompt(&w, "review-1.md");
    assert!(
        review.contains("   Use the $review-agent skill for this review"),
        "{review}"
    );
    assert!(!review.contains("Not installed"), "{review}");
}

/// A pick counts as installed only where the App running its line loads
/// it: codex, the Review's default, reads .agents/skills, never Claude's
/// .claude/skills or its plugins.
#[test]
fn a_pick_only_another_app_loads_is_not_installed() {
    for (dir, picked, installed) in [
        (".agents/skills", "requesting-code-review", true),
        (".claude/skills", "requesting-code-review", false),
        ("plugin/skills", "sp:requesting-code-review", false),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        write_file(
            &w.repo.join(dir).join("requesting-code-review/SKILL.md"),
            "review",
        );
        let plugins = serde_json::json!([
            {"id": "sp@market", "enabled": true, "installPath": w.repo.join("plugin")},
        ])
        .to_string();
        w.hook(move |_, argv| {
            (argv.join(" ") == "claude plugin list --json").then(|| Ok(plugins.clone()))
        });
        pick(&w, &[("review", picked)]);
        o.run_ticket("hx-1");

        let review = prompt(&w, "review-1.md");
        let line = review.contains("Use the $requesting-code-review skill");
        let noted = review.contains(&format!("- Not installed: {picked} (review)"));
        assert_eq!((line, noted), (installed, !installed), "{dir}:\n{review}");
    }
}

/// A pick built into codex is not installed on claude.
#[test]
fn a_built_in_pick_is_not_installed_on_another_app() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    config(&w, r#"{"review": {"app": "claude"}}"#);
    pick(&w, &[("review", "review-agent")]);
    o.run_ticket("hx-1");

    let review = prompt(&w, "review-1.md");
    assert!(!review.contains("review-agent skill"), "{review}");
    assert!(
        review.contains("- Not installed: review-agent (review)"),
        "{review}"
    );
}

/// The audit at none: no audit line, so the Moderator skips it and notes it.
#[test]
fn the_audit_at_none_is_skipped_and_noted() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    write_file(
        &w.repo.join(".claude/skills/ponytail-review/SKILL.md"),
        "cut",
    );
    o.run_ticket("hx-1");
    let audit = "Use the ponytail-review skill on the diff";
    assert!(prompt(&w, "verdict-1.md").contains(audit));

    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    pick(&w, &[("audit", NONE)]);
    o.run_ticket("hx-1");
    let debate = prompt(&w, "verdict-1.md");
    assert!(!debate.contains("Use the "), "{debate}");
    assert!(
        debate.contains("no over-engineering audit (none picked)"),
        "{debate}"
    );
    assert!(
        !debate.contains("audit)"),
        "noted as not installed:\n{debate}"
    );
    // The Review's own default, none: its step 3 stands alone.
    let review = prompt(&w, "review-1.md");
    assert!(!review.contains("Use the "), "{review}");
    assert!(
        review.contains("3. Look for real problems only"),
        "{review}"
    );
}

/// The Review's fallback: unset while config.json has no row for it or its
/// model is none; a row with no model runs its App's default.
#[test]
fn the_fallback_is_unset_at_none_and_runs_a_row_with_no_model() {
    let (w, _o) = new_world(vec![BdTicket::new("hx-1")]);
    let fallback = || super::app::fallback_row(&w.repo).unwrap().map(|r| r.said());
    assert_eq!(fallback(), None);
    config(
        &w,
        r#"{"review_if_limited": {"app": "claude", "model": "none"}}"#,
    );
    assert_eq!(fallback(), None);
    config(&w, r#"{"review_if_limited": {"app": "claude"}}"#);
    assert_eq!(fallback().as_deref(), Some("claude"));
}
