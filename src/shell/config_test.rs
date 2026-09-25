//! /config over fake Tools and the fake world: the docked modal, its pick
//! lists, the probe, and a change saved during a run.

use super::config::{put, Field};
use super::logo::PURPLE;
use super::shell_test::{
    asking, await_line, cols, find, key, logged, render, row, rows, screen_at, shell, type_in,
    type_line,
};
use super::Screen;
use crate::orchestrator::stage::Ask;
use crate::orchestrator::world::{new_world, succeed, BdTicket};
use crate::orchestrator::write_file;
use crate::skills::manifest::{add, Manifest, NONE};
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use crate::tools::{RunError, Tools};
use crossterm::event::KeyCode;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// codex's catalog as `codex debug models --bundled` prints it, a hidden
/// model among the listed ones.
const CATALOG: &str = r#"{"models": [
  {"slug": "gpt-6-sol", "visibility": "list", "supported_reasoning_levels":
    [{"effort": "low"}, {"effort": "medium"}, {"effort": "high"}, {"effort": "xhigh"}]},
  {"slug": "gpt-5.6-sol", "visibility": "list", "supported_reasoning_levels": [{"effort": "low"}]},
  {"slug": "codex-auto-review", "visibility": "hide", "supported_reasoning_levels": []}
]}"#;

/// Fake Tools answering codex's catalog, `fail` with stderr, the rest "".
fn apps(fail: &'static str) -> std::sync::Arc<Fake> {
    Fake::new(move |_, argv| match argv.join(" ") {
        cmd if cmd == "codex debug models --bundled" => Ok(CATALOG.to_string()),
        cmd if !fail.is_empty() && cmd.contains(fail) => Err(format!("model {fail} not found")),
        _ => Ok(String::new()),
    })
}

fn keys(s: &mut Screen, codes: &[KeyCode]) {
    for code in codes {
        s.key(key(*code));
    }
}

/// Polls until /config's probe has answered.
fn await_probe(s: &mut Screen) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.settings.as_ref().unwrap().probe.is_some() {
        assert!(Instant::now() < deadline, "the probe never answered");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

fn config_json(repo: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(repo.join(".harness/config.json")).unwrap())
        .unwrap()
}

/// The foot's note, "" when none.
fn note(s: &Screen) -> String {
    let st = s.settings.as_ref().unwrap();
    st.note
        .as_ref()
        .map_or(String::new(), |(text, _)| text.clone())
}

/// /config docks in the right 58% as a thick purple box titled /config,
/// the live Shell in the left 42%; under 110 columns it folds to a rounded
/// box over the dimmed Shell, leaving it the input line.
#[test]
fn config_opens_docked_at_160x45_and_as_a_box_at_100x30() {
    let repo = TempDir::new();
    let mut s = screen_at(Fake::quiet(), repo.path());
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert_eq!(find(&buf, "┏"), Some((67, 0)), "{:#?}", rows(&buf));
    assert_eq!(buf[(67, 0)].fg, PURPLE);
    assert!(row(&buf, 0).contains("┏ /config ━"), "{:?}", row(&buf, 0));
    assert_eq!(find(&buf, "PIPELINE"), Some((69, 1)), "{:#?}", rows(&buf));
    let (x, _) = find(&buf, "── RECENT").unwrap();
    assert!(x < 67, "RECENT is not in the Shell's left part");

    let buf = render(&s, 100, 30);
    assert_eq!(find(&buf, "╭"), Some((8, 2)), "{:#?}", rows(&buf));
    assert!(row(&buf, 2).contains("╭ /config ─"), "{:?}", row(&buf, 2));
    assert_eq!(find(&buf, "PIPELINE"), Some((10, 3)), "{:#?}", rows(&buf));
    assert!(row(&buf, 29).starts_with('›'), "{:#?}", rows(&buf));
}

/// The Review from claude to codex: picking the App leads into codex's
/// models, read from its catalog (the hidden one left out); the model is
/// probed and saves with the App, the effort back to default though codex
/// lists `high` too, and the effort, from that model's levels, saves at once.
#[test]
fn review_to_codex_a_listed_model_and_an_effort_save_all_three() {
    let repo = TempDir::new();
    let file = repo.path().join(".harness/config.json");
    write_file(
        &file,
        r#"{"review": {"app": "claude", "model": "opus", "effort": "high"}}"#,
    );
    let tools = apps("");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "Review model · codex (new App)").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "gpt-5.6-sol").is_some(), "{:#?}", rows(&buf));
    assert!(
        find(&buf, "codex-auto-review").is_none(),
        "{:#?}",
        rows(&buf)
    );
    type_in(&mut s, "6-sol");
    s.key(key(KeyCode::Enter));
    await_probe(&mut s);
    assert!(
        tools
            .calls()
            .contains(&"codex exec --sandbox read-only -m gpt-6-sol Reply with ok".to_string()),
        "{:#?}",
        tools.calls()
    );
    assert_eq!(
        config_json(repo.path()),
        json!({"review": {"app": "codex", "model": "gpt-6-sol", "effort": "default"}})
    );
    // gpt-6-sol's levels: default, low, medium, high, xhigh.
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    keys(
        &mut s,
        &[KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter],
    );
    assert_eq!(
        config_json(repo.path()),
        json!({"review": {"app": "codex", "model": "gpt-6-sol", "effort": "high"}})
    );
    assert_eq!(
        note(&s),
        "saved: Review codex gpt-6-sol/high, in .harness/config.json"
    );
    assert!(s.settings.as_ref().unwrap().saved.is_some());
}

/// A typed id is probed on the row's App before it saves; one the App
/// refuses keeps the old value, and the App's error shows in the foot.
#[test]
fn a_typed_id_whose_probe_fails_keeps_the_old_value_and_shows_the_error() {
    let repo = TempDir::new();
    let file = repo.path().join(".harness/config.json");
    let before = r#"{"implement": {"model": "opus"}}"#;
    write_file(&file, before);
    let tools = apps("claude-nope");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(
        &mut s,
        &[KeyCode::Enter, KeyCode::Down, KeyCode::Down, KeyCode::Enter],
    );
    type_in(&mut s, "type"); // only 'type an id…' is left
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "claude-nope");
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "Implement model id › claude-nope▏").is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(
        find(
            &buf,
            "probing claude-nope on claude with a one-line prompt…"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    await_probe(&mut s);
    let probe = tools
        .calls()
        .into_iter()
        .find(|c| c.contains("claude-nope"))
        .unwrap();
    assert!(
        probe.starts_with("claude --tools Read,Grep,Glob,Skill --add-dir ")
            && probe.ends_with(" -p --model claude-nope Reply with ok"),
        "{probe}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    let error = "claude refused claude-nope: model claude-nope not found. Nothing changed.";
    assert_eq!(note(&s), error);
    let buf = render(&s, 160, 45);
    assert!(find(&buf, error).is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "opus").is_some(), "the old model is not shown");
}

/// none, typed for a row other than the Review's fallback, is refused
/// before any probe: it would run as a model.
#[test]
fn none_typed_off_the_fallback_is_refused_unprobed() {
    let repo = TempDir::new();
    let tools = apps("");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(
        &mut s,
        &[KeyCode::Enter, KeyCode::Down, KeyCode::Down, KeyCode::Enter],
    );
    type_in(&mut s, "type");
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "none");
    let calls = tools.calls().len();
    s.key(key(KeyCode::Enter));
    assert!(s.settings.as_ref().unwrap().probe.is_none());
    assert_eq!(tools.calls().len(), calls, "{:#?}", tools.calls());
    let file = repo.path().join(".harness/config.json");
    assert_eq!(
        note(&s),
        format!(
            "Refused: {}: implement model none: only review_if_limited takes none. \
             Nothing changed.",
            file.display()
        )
    );
    assert!(!file.exists());
}

/// During a run a change saves at once and RECENT and the log say so; the
/// Stage that starts after it runs on it.
#[test]
fn a_change_during_a_run_logs_config_and_the_next_stage_starts_on_it() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.hook(|_, argv| {
        (argv.join(" ") == "codex debug models --bundled").then(|| Ok(CATALOG.to_string()))
    });
    // Implement holds until the change is saved.
    let (release, held) = channel::<()>();
    let held = Mutex::new(held);
    w.session(move |p| {
        if p.stage == "implement" {
            let _ = held.lock().unwrap().recv();
        }
        succeed(p)
    });
    let mut s = shell(&w);
    s.command("/start-ticket hx-1");
    await_line(&mut s, "hx-1 implement started: claude");
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(row(&buf, 0).contains(" run live "), "{:?}", row(&buf, 0));
    // Review, its model, gpt-6-sol
    keys(
        &mut s,
        &[KeyCode::Down, KeyCode::Enter, KeyCode::Down, KeyCode::Enter],
    );
    type_in(&mut s, "6-sol");
    s.key(key(KeyCode::Enter));
    await_probe(&mut s);
    assert_eq!(
        note(&s),
        "saved: Review codex gpt-6-sol. Stages that start from now use it; running ones keep theirs."
    );
    await_line(&mut s, "config: Review codex → codex gpt-6-sol");
    assert!(logged(&w, "config: Review codex → codex gpt-6-sol"));
    drop(release);
    await_line(&mut s, "hx-1 review 1 started: codex gpt-6-sol");
    let start = &w.called("herdr agent start h-hx-1-review")[0];
    assert!(
        start.ends_with("-- --sandbox workspace-write -m gpt-6-sol"),
        "{start}"
    );
}

/// The Pipeline list with each section's summary, a section's page with its
/// rows grouped, and a pick list with each model's family and the current
/// one marked.
#[test]
fn the_pipeline_list_a_stage_page_and_a_pick_list_render() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".harness/config.json"),
        r#"{"implement": {"model": "opus", "effort": "high"}, "side_b": {"model": "gpt-6-sol"}}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    let blocked = Ask::Blocked {
        pane: "2-1".to_string(),
    };
    s.push(asking("harness-kqe.10", "blocked in fix 1", blocked));
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(
        row(&buf, 0).ends_with("━ 1 waiting ┓"),
        "{:?}",
        row(&buf, 0)
    );
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let left: Vec<String> = (1..16).map(|y| text(&buf, y, 69, 97)).collect();
    assert_eq!(
        left,
        [
            "PIPELINE",
            "▸ Plan+Impl claude opus",
            "  │",
            "  Review    codex",
            "  │",
            "  Debate    claude+codex",
            "  │",
            "  Fix       claude",
            "  │",
            "  Address   claude",
            "────────────────────────────",
            "  Apps      2 of 2 installed",
            "  Skills    0 installed",
            "  TypeSafe  on",
            "",
        ]
    );
    assert_eq!(buf[(71, 2)].fg, PURPLE, "the picked section is not marked");

    // The Review's page: its fallback starts as none.
    keys(&mut s, &[KeyCode::Down]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "  if limited: model     none  no fallback").is_some(),
        "{:#?}",
        rows(&buf)
    );

    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Down]);
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..24).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Debate  claude, codex",
            "The Moderator puts each disputed Finding to side A and side",
            "B, two models from different families; TypeSafe scores what",
            "they still dispute.",
            "",
            "  Moderator app         claude",
            "▸ Moderator model       default  Anthropic",
            "  Moderator effort      default",
            "",
            "  side A app            claude",
            "  side A model          default  Anthropic",
            "  side A effort         default",
            "",
            "  side B app            codex",
            "  side B model          gpt-6-sol  OpenAI",
            "  side B effort         default",
            "",
            "DELEGATE SKILLS",
            "  over-engineering audit  ponytail-review  not installed",
            "",
            "CHECKS",
            "  ✓ The sides come from two families: Anthropic and OpenAI.",
            "",
        ]
    );
    assert!(
        row(&buf, 42).contains(
            "stage-moderate's pane: runs the Debate and settles each Finding. Default passes no flag."
        ),
        "{:#?}",
        rows(&buf)
    );

    keys(
        &mut s,
        &[KeyCode::Esc, KeyCode::Up, KeyCode::Up, KeyCode::Enter],
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..9).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Implement model · claude   filter › ▏",
            "  default           claude's own · Anthropic",
            "  fable             Anthropic",
            "▸ opus              Anthropic                    ✓ current",
            "  sonnet            Anthropic",
            "  haiku             Anthropic",
            "  type an id…       probed before it saves",
            "",
        ]
    );
    assert!(
        row(&buf, 44).contains("↑↓ move · type to filter · Enter picks · Esc back"),
        "{:?}",
        row(&buf, 44)
    );
}

/// Tools that answer codex's catalog and refuse every probe with `0`.
struct Refusing(RunError);

impl Tools for Refusing {
    fn run(&self, _: &Path, argv: &[&str]) -> Result<String, RunError> {
        match argv {
            ["codex", "debug", ..] => Ok(CATALOG.to_string()),
            [.., "Reply with ok"] => Err(self.0.clone()),
            _ => Ok(String::new()),
        }
    }
}

/// The App's own error, as each App prints it: claude -p on stdout under a
/// warning on stderr, codex exec a JSON line ending its stderr.
#[test]
fn a_refused_probe_shows_what_the_app_said() {
    let claude = RunError {
        command: String::new(),
        status: "exit status 1".to_string(),
        stderr: "\"opus\" isn't described by this version's model catalog".to_string(),
        stdout: "There's an issue with the selected model (opus). It may not exist.\n".to_string(),
    };
    let codex = RunError {
        stderr: "codex v0.156.1\nmodel: gpt-6-sol\nERROR: {\"type\":\"error\",\"status\":400,\
            \"error\":{\"type\":\"invalid_request_error\",\"message\":\"The 'gpt-6-sol' model \
            is not supported when using Codex with a ChatGPT account.\"}}"
            .to_string(),
        stdout: String::new(),
        ..claude.clone()
    };
    for (err, section, want) in [
        (
            claude,
            0,
            "claude refused opus: There's an issue with the selected model (opus). \
             It may not exist. Nothing changed.",
        ),
        (
            codex,
            1,
            "codex refused gpt-6-sol: The 'gpt-6-sol' model is not supported when \
             using Codex with a ChatGPT account. Nothing changed.",
        ),
    ] {
        let repo = TempDir::new();
        let mut s = screen_at(Arc::new(Refusing(err)), repo.path());
        type_line(&mut s, "/config");
        for _ in 0..section {
            s.key(key(KeyCode::Down));
        }
        keys(&mut s, &[KeyCode::Enter, KeyCode::Down]);
        if section == 0 {
            s.key(key(KeyCode::Down)); // past the toggle
        }
        s.key(key(KeyCode::Enter));
        type_in(&mut s, if section == 0 { "opus" } else { "6-sol" });
        s.key(key(KeyCode::Enter));
        await_probe(&mut s);
        assert_eq!(note(&s), want);
        assert!(!repo.path().join(".harness/config.json").exists());
    }
}

/// A model that does not list the row's effort takes it back to default;
/// picking the value a row already has changes nothing, not even a probe.
#[test]
fn a_model_without_the_effort_resets_it_and_the_current_pick_changes_nothing() {
    let repo = TempDir::new();
    let file = repo.path().join(".harness/config.json");
    write_file(
        &file,
        r#"{"review": {"model": "gpt-6-sol", "effort": "xhigh"}}"#,
    );
    let tools = apps("");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(
        &mut s,
        &[KeyCode::Down, KeyCode::Enter, KeyCode::Down, KeyCode::Enter],
    );
    type_in(&mut s, "5.6-sol"); // it lists low alone
    s.key(key(KeyCode::Enter));
    await_probe(&mut s);
    assert_eq!(
        config_json(repo.path()),
        json!({"review": {"model": "gpt-5.6-sol", "effort": "default"}})
    );
    let (calls, saved) = (tools.calls().len(), std::fs::read_to_string(&file).unwrap());
    keys(&mut s, &[KeyCode::Enter, KeyCode::Enter]); // the cursor opens on the current one
    assert!(s.settings.as_ref().unwrap().probe.is_none());
    assert_eq!(tools.calls().len(), calls, "{:#?}", tools.calls());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), saved);
    assert_eq!(note(&s), "");
}

/// A row on an App the table does not have still lists the Apps to move it
/// to one.
#[test]
fn a_row_on_an_unknown_app_can_be_moved_to_a_listed_one() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".harness/config.json"),
        r#"{"fix": {"app": "gemini"}}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    let open_fix = [
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Enter,
        KeyCode::Enter,
    ];
    keys(&mut s, &open_fix);
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "▸ claude").is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "  codex").is_some(), "{:#?}", rows(&buf));
    keys(&mut s, &[KeyCode::Enter, KeyCode::Enter]); // claude, then its default
    assert_eq!(
        config_json(repo.path()),
        json!({"fix": {"app": "claude", "model": "default"}})
    );
}

/// TypeSafe reads off while config.json turns it off, the key set or not.
#[test]
fn typesafe_reads_off_while_config_json_turns_it_off() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".harness/config.json"),
        r#"{"typesafe": false}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "TypeSafe  off").is_some(), "{:#?}", rows(&buf));
}

/// A config.json that is not an object is refused, never replaced: /config
/// will not open on it, and a change after a hand edit to one saves nothing.
#[test]
fn a_config_json_not_an_object_is_refused_not_replaced() {
    let repo = TempDir::new();
    let file = repo.path().join(".harness/config.json");
    write_file(&file, r#"{"fix": {"app": "gemini"}}"#);
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    write_file(&file, "[1]");
    let fix_to_claude_default = [
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Enter,
        KeyCode::Enter,
        KeyCode::Enter,
        KeyCode::Enter,
    ];
    keys(&mut s, &fix_to_claude_default);
    let why = format!("{}: not a JSON object", file.display());
    assert_eq!(note(&s), format!("Refused: {why}. Nothing changed."));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "[1]");
    s.key(key(KeyCode::Esc));
    s.key(key(KeyCode::Esc));
    s.key(key(KeyCode::Esc));
    type_line(&mut s, "/config");
    assert!(s.settings.is_none());
    assert_eq!(super::shell_test::notice(&s), why);
}

/// A config.json edited by hand to break a rule refuses the run, naming
/// the rule, and /config marks it: ✗ on the Review, the badge counting it,
/// the rule spelt out under CHECKS.
#[test]
fn a_hand_edited_config_that_breaks_a_rule_refuses_the_run_and_shows_it() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    write_file(
        &w.repo.join(".harness/config.json"),
        r#"{"implement": {"model": "claude-opus-5-5"}, "review": {"app": "claude", "model": "opus"}}"#,
    );
    let mut s = shell(&w);
    s.command("/start-epic hx");
    let rule = "The Review would run on Implement's model, claude-opus-5-5: it must not review its own work";
    assert_eq!(super::shell_test::notice(&s), rule);
    assert!(s.run.is_none() && w.called("bd worktree create").is_empty());

    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(row(&buf, 0).contains("━ ✗ 1 check ┓"), "{:?}", row(&buf, 0));
    assert!(
        find(&buf, "  Review    claude opus ✗").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert_eq!(buf[(93, 4)].fg, super::logo::RED);
    keys(&mut s, &[KeyCode::Down]);
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "CHECKS").is_some(), "{:#?}", rows(&buf));
    assert!(
        find(&buf, &format!("✗ {}", &rule[..40])).is_some(),
        "{:#?}",
        rows(&buf)
    );
}

/// A pick that would break a rule is refused before any probe: the rule
/// shows in the foot and config.json stays byte for byte as it was.
#[test]
fn a_refused_pick_says_the_rule_and_leaves_config_json_as_it_was() {
    let repo = TempDir::new();
    let file = repo.path().join(".harness/config.json");
    let before = "{ \"implement\":{\"model\":\"opus\"} }\n";
    write_file(&file, before);
    let tools = apps("");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    // The Review's App, claude, then its opus-5.5: Implement's opus.
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Up, KeyCode::Enter]);
    type_in(&mut s, "type");
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "opus-5.5");
    s.key(key(KeyCode::Enter));
    assert!(s.settings.as_ref().unwrap().probe.is_none());
    assert!(
        !tools.calls().iter().any(|c| c.ends_with("Reply with ok")),
        "{:#?}",
        tools.calls()
    );
    assert_eq!(
        note(&s),
        "Refused: The Review would run on Implement's model, claude-opus-5-5: \
         it must not review its own work. Nothing changed."
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
}

/// Implement's fields put into config.json: a plan other than Implement's
/// model splits, both halves by full id; the plan on Implement's own model,
/// or a new App for Plan + Implement, plans on one model again.
// ponytail: a unit test, as Implement cannot leave claude on screen until
// harness-7nq.12 lifts runs_on; a screen test of the App change then.
#[test]
fn a_split_takes_full_ids_and_an_app_change_plans_on_one_model_again() {
    let implement = |doc: Value, fields: &[(Field, &str)]| {
        let mut doc = doc;
        let fields: Vec<_> = fields.iter().map(|(f, v)| (*f, v.to_string())).collect();
        put(&mut doc, "implement", &fields);
        doc
    };
    let split = implement(
        json!({"implement": {"model": "opus", "effort": "high"}}),
        &[(Field::Plan, "fable")],
    );
    assert_eq!(
        split,
        json!({"implement": {"model": "claude-opus-5-5", "plan_model": "claude-fable-5-1", "effort": "high"}})
    );
    let one = json!({"implement": {"model": "claude-opus-5-5", "effort": "high"}});
    assert_eq!(implement(split.clone(), &[(Field::Plan, "opus")]), one);
    assert_eq!(implement(split.clone(), &[(Field::Plan, "default")]), one);
    assert_eq!(
        implement(split, &[(Field::App, "codex"), (Field::Model, "gpt-6-sol")]),
        json!({"implement": {"app": "codex", "model": "gpt-6-sol", "effort": "high"}})
    );
    // Another row keeps what it is given.
    let mut doc = json!({});
    put(&mut doc, "review", &[(Field::Model, "opus".to_string())]);
    assert_eq!(doc, json!({"review": {"model": "opus"}}));
}

/// 'Same model for plan and implementation' is on by default; turning it
/// off with Implement's model at default is refused, as the split needs a
/// named model for each half.
#[test]
fn the_toggle_off_with_implement_at_default_is_refused() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ [x] Same model for plan and implementation").is_some(),
        "{:#?}",
        rows(&buf)
    );
    for code in [KeyCode::Enter, KeyCode::Char(' ')] {
        s.key(key(code));
        assert!(s.settings.as_ref().unwrap().pick.is_none());
        assert_eq!(
            note(&s),
            "Pick Implement's model first: the split needs a named model for each half."
        );
    }
    assert!(!repo.path().join(".harness/config.json").exists());
}

/// Turning the toggle off opens the plan's model list, Implement's App's
/// models of its family; Esc keeps one model. A plan model picked is probed
/// and splits, both halves saved by full id, with plan model and implement
/// model rows and the rule under CHECKS; the plan on Implement's own model
/// is one model again, and so is the toggle turned on.
#[test]
fn the_toggle_splits_on_a_plan_model_and_joins_again() {
    let repo = TempDir::new();
    let file = repo.path().join(".harness/config.json");
    write_file(
        &file,
        r#"{"implement": {"model": "opus", "effort": "high"}}"#,
    );
    let tools = apps("");
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Char(' ')]);
    let buf = render(&s, 160, 45);
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let right: Vec<String> = (1..8).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Implement plan model · claude   filter › ▏",
            "▸ fable             Anthropic",
            "  opus              Anthropic",
            "  sonnet            Anthropic",
            "  haiku             Anthropic",
            "  type an id…       probed before it saves",
            "",
        ]
    );
    assert!(
        find(
            &buf,
            "Pick the plan's model to split planning from implementing; Esc keeps one model for both."
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Esc));
    assert_eq!(
        config_json(repo.path()),
        json!({"implement": {"model": "opus", "effort": "high"}})
    );

    s.key(key(KeyCode::Char(' ')));
    s.key(key(KeyCode::Enter)); // fable
    await_probe(&mut s);
    assert!(
        tools
            .calls()
            .iter()
            .any(|c| c.ends_with("-p --model claude-fable-5-1 Reply with ok")),
        "{:#?}",
        tools.calls()
    );
    let split = json!({"implement": {
        "model": "claude-opus-5-5", "plan_model": "claude-fable-5-1", "effort": "high"
    }});
    assert_eq!(config_json(repo.path()), split);
    assert_eq!(
        note(&s),
        "saved: Implement claude claude-fable-5-1→claude-opus-5-5/high, in .harness/config.json"
    );
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (5..16).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "  app                   claude",
            "▸ [ ] Same model for plan and implementation",
            "  plan model            claude-fable-5-1  Anthropic",
            "  implement model       claude-opus-5-5  Anthropic",
            "  effort                high",
            "",
            "DELEGATE SKILLS",
            "  test-first              tdd  not installed",
            "  self review             code-review  not installed",
            "  working mode            ponytail  not installed",
            "  prose                   caveman  not installed",
        ]
    );

    // Implement's model list offers no default: the split needs a named one.
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "Implement model · claude").is_some() && find(&buf, "claude's own").is_none(),
        "{:#?}",
        rows(&buf)
    );
    keys(&mut s, &[KeyCode::Esc, KeyCode::Up, KeyCode::Up]);

    // The plan on Implement's own model: one model again, nothing probed.
    let calls = tools.calls().len();
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    type_in(&mut s, "opus");
    s.key(key(KeyCode::Enter));
    assert_eq!(tools.calls().len(), calls, "{:#?}", tools.calls());
    let one = json!({"implement": {"model": "claude-opus-5-5", "effort": "high"}});
    assert_eq!(config_json(repo.path()), one);

    // Split again, then the toggle turned on.
    keys(&mut s, &[KeyCode::Up, KeyCode::Char(' '), KeyCode::Enter]);
    await_probe(&mut s);
    assert_eq!(config_json(repo.path()), split);
    s.key(key(KeyCode::Enter));
    assert_eq!(config_json(repo.path()), one);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "▸ [x] Same model for plan and implementation").is_some(),
        "{:#?}",
        rows(&buf)
    );
}

/// A pick list marks each model that would break a rule: every claude
/// model for side B, as side A is Anthropic; Implement's model in the
/// Review's list; the Review's model in Implement's.
#[test]
fn a_pick_list_marks_each_model_that_would_break_a_rule() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".harness/config.json"),
        r#"{"implement": {"model": "opus"}, "review": {"app": "claude", "model": "sonnet"}}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    // side B's App, claude
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Down; 6]);
    keys(&mut s, &[KeyCode::Enter, KeyCode::Up, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..8).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Debate side B model · claude (new App)   filter › ▏",
            "▸ default           claude's own · Anth… ✗ side A's family",
            "  fable             Anthropic            ✗ side A's family",
            "  opus              Anthropic            ✗ side A's family",
            "  sonnet            Anthropic            ✗ side A's family",
            "  haiku             Anthropic            ✗ side A's family",
            "  type an id…       probed before it sa…",
        ]
    );
    let (x, y) = find(&buf, "✗ side A's family").unwrap();
    assert_eq!(buf[(x, y)].fg, super::logo::RED);

    // The Review's model list: Implement's opus.
    keys(
        &mut s,
        &[KeyCode::Esc, KeyCode::Left, KeyCode::Up, KeyCode::Enter],
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    assert!(
        text(&buf, find(&buf, "  opus ").unwrap().1, 99, 158)
            .ends_with("Anthropic          ✗ Implement's model"),
        "{:#?}",
        rows(&buf)
    );
    // Implement's: the Review's sonnet.
    keys(
        &mut s,
        &[KeyCode::Esc, KeyCode::Left, KeyCode::Up, KeyCode::Enter],
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    assert!(
        text(&buf, find(&buf, "  sonnet ").unwrap().1, 99, 158)
            .ends_with("Anthropic         ✗ the Review's model"),
        "{:#?}",
        rows(&buf)
    );
    assert!(
        text(&buf, find(&buf, "▸ opus ").unwrap().1, 99, 158).ends_with("✓ current"),
        "{:#?}",
        rows(&buf)
    );
}

/// Fake Tools where codex is not on PATH and claude says its version.
fn codex_missing() -> Arc<Fake> {
    Fake::new(|_, argv| match argv.join(" ").as_str() {
        "which codex" => Err("codex not found".to_string()),
        "claude --version" => Ok("2.1.282 (Claude Code)\n".to_string()),
        _ => Ok(String::new()),
    })
}

/// The Apps page: each App of the table installed with its version, or
/// greyed not installed with its homepage.
#[test]
fn the_apps_page_renders_each_app_installed_or_not() {
    let repo = TempDir::new();
    let mut s = screen_at(codex_missing(), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down; 5]);
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let buf = render(&s, 160, 45);
    assert_eq!(text(&buf, 12, 69, 97), "▸ Apps      1 of 2 installed");
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..8).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Apps  1 of 2 installed",
            "The agent CLIs a Stage runs on, found on PATH when /config",
            "opened; harness init installs herdr's integration for each.",
            "",
            "▸ claude      installed      2.1.282 (Claude Code)",
            "  codex       not installed  https://developers.openai.com…",
            "",
        ]
    );
    let (x, y) = find(&buf, "codex       not installed").unwrap();
    assert_eq!(buf[(x, y)].fg, super::logo::MUTED);
}

/// An App not installed is greyed in an App list; picking it says where
/// to get it and changes nothing. So does the Apps page's foot on it.
#[test]
fn an_app_not_installed_says_where_to_get_it() {
    let repo = TempDir::new();
    let tools = codex_missing();
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    // The Review's App list, codex: its current App, greyed.
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    let (x, y) = find(&buf, "codex").unwrap();
    assert_eq!(buf[(x, y)].fg, super::logo::MUTED);
    assert!(
        cols(&buf, y, 99, 158).contains("not installed"),
        "{:#?}",
        rows(&buf)
    );
    let calls = tools.calls().len();
    s.key(key(KeyCode::Enter));
    let where_ = "codex is not on PATH: get it at https://developers.openai.com/codex";
    assert_eq!(note(&s), where_);
    assert!(s.settings.as_ref().unwrap().pick.is_none());
    let buf = render(&s, 160, 45);
    assert!(find(&buf, where_).is_some(), "{:#?}", rows(&buf));
    assert_eq!(tools.calls().len(), calls, "{:#?}", tools.calls());

    // From the Apps page.
    keys(
        &mut s,
        &[KeyCode::Left, KeyCode::Down, KeyCode::Down, KeyCode::Down],
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Down]);
    let buf = render(&s, 160, 45);
    assert!(find(&buf, where_).is_some(), "{:#?}", rows(&buf));
    assert_eq!(tools.calls().len(), calls, "{:#?}", tools.calls());
    assert!(!repo.path().join(".harness/config.json").exists());
}

/// A config.json broken by hand on two rules mends one rule at a time: a
/// change that leaves a rule broken as it was saves, one that breaks a
/// rule that held is refused.
#[test]
fn a_config_broken_on_two_rules_mends_one_at_a_time() {
    let repo = TempDir::new();
    let file = repo.path().join(".harness/config.json");
    write_file(
        &file,
        r#"{"implement": {"model": "opus"}, "review": {"app": "claude", "model": "opus"},
            "side_b": {"app": "claude"}}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    let buf = render(&s, 160, 45);
    assert!(
        row(&buf, 0).contains("━ ✗ 2 checks ┓"),
        "{:?}",
        row(&buf, 0)
    );
    // The Review to codex, its default: the Debate stays broken.
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter, KeyCode::Enter]);
    assert_eq!(
        config_json(repo.path())["review"],
        json!({"app": "codex", "model": "default"})
    );
    let buf = render(&s, 160, 45);
    assert!(
        row(&buf, 0).contains(" ✗ 1 check · saved "),
        "{:?}",
        row(&buf, 0)
    );
    // The Review back on Implement's model breaks a rule that held.
    let saved = std::fs::read_to_string(&file).unwrap();
    keys(&mut s, &[KeyCode::Enter, KeyCode::Up, KeyCode::Enter]);
    type_in(&mut s, "opus");
    s.key(key(KeyCode::Enter));
    assert!(note(&s).starts_with("Refused: The Review would run on Implement's model"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), saved);
}

const TDD: &str = "---\nname: tdd\n---\ntest first\n";

/// A two-skill pack as a clone finds it.
const PACK: &[(&str, &str)] = &[
    ("skills/engineering/tdd/SKILL.md", TDD),
    (
        "skills/engineering/code-review/SKILL.md",
        "---\nname: code-review\n---\n",
    ),
];

/// Fake Tools where git clone writes the pack into its destination and
/// rev-parse answers abc1234def; everything else is "".
fn clones() -> Arc<Fake> {
    Fake::new(|_, argv| {
        if argv.contains(&"clone") {
            let dest = Path::new(argv.last().unwrap());
            for (path, text) in PACK {
                write_file(&dest.join(path), text);
            }
            Ok(String::new())
        } else if argv.contains(&"rev-parse") {
            Ok("abc1234def\n".to_string())
        } else {
            Ok(String::new())
        }
    })
}

/// Polls until /config's clone or update has finished.
fn await_busy(s: &mut Screen) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.settings.as_ref().unwrap().busy.is_some() {
        assert!(Instant::now() < deadline, "the clone never finished");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

fn manifest(repo: &Path) -> Manifest {
    Manifest::load(repo).unwrap()
}

/// /config open on the Skills page.
fn skills_page(tools: Arc<Fake>, repo: &Path) -> Screen {
    let mut s = screen_at(tools, repo);
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down; 6]);
    s.key(key(KeyCode::Enter));
    s
}

/// A bare name is not a source: refused at once with where to find one,
/// the text kept to mend, nothing cloned.
#[test]
fn adding_a_bare_name_is_refused_before_any_clone() {
    let repo = TempDir::new();
    let tools = clones();
    let mut s = skills_page(tools.clone(), repo.path());
    s.key(key(KeyCode::Char('a')));
    type_in(&mut s, "tdd");
    s.key(key(KeyCode::Enter));
    assert!(note(&s).contains("skills.sh"), "{}", note(&s));
    assert!(s.settings.as_ref().unwrap().typing.is_some());
    assert!(
        !tools.calls().iter().any(|c| c.contains("clone")),
        "{:#?}",
        tools.calls()
    );
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "source › tdd▏").is_some(), "{:#?}", rows(&buf));
}

/// Removing a skill a job uses asks first, naming the job; no keeps it,
/// yes removes it and sets that job to none.
#[test]
fn removing_a_skill_a_job_uses_asks_then_sets_the_job_to_none() {
    let repo = TempDir::new();
    let tools = clones();
    add(
        repo.path(),
        Path::new(""),
        &*tools,
        "mattpocock/skills",
        Some("tdd"),
    )
    .unwrap();
    let mut s = skills_page(tools, repo.path());
    s.key(key(KeyCode::Down)); // the location, then tdd
    s.key(key(KeyCode::Char('d')));
    let buf = render(&s, 160, 45);
    assert!(
        find(
            &buf,
            "tdd is the Delegate skill for Plan + Implement test-first"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Char('n')));
    assert!(manifest(repo.path()).skills.contains_key("tdd"));
    s.key(key(KeyCode::Char('d')));
    s.key(key(KeyCode::Char('y')));
    let m = manifest(repo.path());
    assert!(!m.skills.contains_key("tdd"));
    assert_eq!(m.pick("test-first"), NONE);
    assert!(!repo.path().join(".agents/skills/tdd").exists());
    assert_eq!(note(&s), "removed tdd; Plan + Implement test-first is none");
}

/// A source with two skills, one installed from it already: the checklist
/// shows that one ticked; the other ticked installs, and the note says so.
#[test]
fn adding_a_pack_shows_the_checklist_and_installs_the_ticked_skill() {
    let repo = TempDir::new();
    let tools = clones();
    add(
        repo.path(),
        Path::new(""),
        &*tools,
        "mattpocock/skills",
        Some("tdd"),
    )
    .unwrap();
    let mut s = skills_page(tools.clone(), repo.path());
    s.key(key(KeyCode::Char('a')));
    type_in(&mut s, "mattpocock/skills");
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "cloning mattpocock/skills…").is_some(),
        "{:#?}",
        rows(&buf)
    );
    await_busy(&mut s);
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..5).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Skills in mattpocock/skills",
            "▸ [ ] code-review",
            "  [x] tdd                     installed",
            "",
        ]
    );
    s.key(key(KeyCode::Enter));
    assert_eq!(note(&s), "Space ticks a skill to install.");
    keys(&mut s, &[KeyCode::Char(' '), KeyCode::Enter]);
    await_busy(&mut s);
    let m = manifest(repo.path());
    assert_eq!(
        m.skills.keys().collect::<Vec<_>>(),
        ["code-review", "tdd"],
        "{m:?}"
    );
    assert_eq!(m.skills["code-review"].commit, "abc1234def");
    assert_eq!(
        note(&s),
        "installed code-review from mattpocock/skills @ abc1234"
    );
    assert!(s.settings.as_ref().unwrap().listing.is_none());
}

/// A job's suggestion not installed: picking it clones its source, installs
/// it and picks it.
#[test]
fn picking_a_suggestion_not_installed_clones_it_and_picks_it() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".harness/skills.json"),
        r#"{"picks": {"test-first": "none"}}"#,
    );
    let tools = clones();
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "  tdd ").is_some_and(|(_, y)| cols(&buf, y, 99, 158).contains("not installed")),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "▸ none").is_some(), "{:#?}", rows(&buf));
    // from none, the current pick, up past the other two suggestions
    keys(
        &mut s,
        &[KeyCode::Up, KeyCode::Up, KeyCode::Up, KeyCode::Enter],
    );
    await_busy(&mut s);
    assert!(
        tools
            .calls()
            .iter()
            .any(|c| c.contains("clone") && c.contains("https://github.com/mattpocock/skills")),
        "{:#?}",
        tools.calls()
    );
    let m = manifest(repo.path());
    assert!(m.skills.contains_key("tdd"), "{m:?}");
    assert_eq!(m.pick("test-first"), "tdd");
    assert!(repo.path().join(".agents/skills/tdd/SKILL.md").is_file());
    assert_eq!(
        note(&s),
        "installed tdd from mattpocock/skills @ abc1234; Plan + Implement test-first uses it"
    );
}

/// A job lists only the skills its Stage's App loads: the Review on codex
/// hides a personal skill only Claude loads, and lists codex's built-in;
/// Implement on claude lists it.
#[test]
fn the_review_job_on_codex_hides_a_claude_only_skill() {
    let (repo, home) = (TempDir::new(), TempDir::new());
    write_file(
        &home.path().join(".claude/skills/grilling/SKILL.md"),
        "---\nname: grilling\n---\n",
    );
    write_file(
        &home.path().join(".agents/skills/archify/SKILL.md"),
        "---\nname: archify\n---\n",
    );
    let mut s = screen_at(apps(""), repo.path());
    s.cfg.home = home.path().to_path_buf();
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
    keys(&mut s, &[KeyCode::Down; 6]);
    s.key(key(KeyCode::Enter));
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let buf = render(&s, 160, 45);
    let right: Vec<String> = (1..10).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Review review · codex   filter › ▏",
            "  SUGGESTED",
            "  review-agent            built into codex built in",
            "  requesting-code-review  obra/superpowers not installed",
            "▸ none                    the Stage skill… ✓ current",
            "  YOUR OTHER SKILLS CODEX CAN SEE",
            "  archify                 ~/.agents/skills yours",
            "",
            "",
        ]
    );
    assert!(find(&buf, "grilling").is_none(), "{:#?}", rows(&buf));

    keys(
        &mut s,
        &[KeyCode::Esc, KeyCode::Left, KeyCode::Up, KeyCode::Enter],
    );
    keys(&mut s, &[KeyCode::Down; 4]);
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "grilling").is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "archify").is_none(), "{:#?}", rows(&buf));
}

/// The Skills page: the location read-only, then each skill, a Shipped one
/// said so, a fetched one with its source @ commit and the jobs using it.
#[test]
fn the_skills_page_renders_the_location_and_each_skill() {
    let repo = TempDir::new();
    let tools = clones();
    add(
        repo.path(),
        Path::new(""),
        &*tools,
        "mattpocock/skills",
        Some("tdd"),
    )
    .unwrap();
    let mut m = manifest(repo.path());
    m.skills.insert(
        "create-pr".to_string(),
        crate::skills::manifest::Installed {
            shipped: true,
            ..Default::default()
        },
    );
    m.save(repo.path()).unwrap();
    let mut s = skills_page(tools, repo.path());
    let text =
        |buf: &_, y: u16, from: usize, to: usize| cols(buf, y, from, to).trim_end().to_string();
    let buf = render(&s, 160, 45);
    assert_eq!(text(&buf, 13, 69, 97), "▸ Skills    1 installed");
    let right: Vec<String> = (1..11).map(|y| text(&buf, y, 99, 158)).collect();
    assert_eq!(
        right,
        [
            "Skills  1 installed",
            "The skills the Harness installed, from their sources; the",
            "Skill manifest is .harness/skills.json.",
            "",
            "▸ location    the repo, committed  .agents/skills",
            "",
            "  create-pr               shipped with the Harness",
            "  tdd                     mattpocock/skills @ abc1234",
            "    ← Plan + Implement test-first",
            "",
        ]
    );
    assert!(
        find(
            &buf,
            "Run harness init again to change where skills are installed."
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    keys(&mut s, &[KeyCode::Down, KeyCode::Char('d')]);
    assert_eq!(
        note(&s),
        "create-pr is a Shipped skill: it cannot be removed."
    );
    assert!(s.settings.as_ref().unwrap().confirm.is_none());
}

/// /config open on the TypeSafe page.
fn typesafe_page(s: &mut Screen) {
    type_line(s, "/config");
    keys(s, &[KeyCode::Down; 7]);
    s.key(key(KeyCode::Enter));
}

/// TypeSafe on: turning it off asks first, saying what changes; yes saves
/// off in config.json.
#[test]
fn typesafe_off_asks_then_saves_off() {
    let repo = TempDir::new();
    let mut s = screen_at(apps(""), repo.path());
    typesafe_page(&mut s);
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "TypeSafe  on").is_some(), "{:#?}", rows(&buf));
    s.key(key(KeyCode::Enter));
    let buf = render(&s, 160, 45);
    assert!(
        find(
            &buf,
            "Turn TypeSafe off? Every Wake and Plan becomes a Question"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );
    s.key(key(KeyCode::Char('n')));
    assert!(!repo.path().join(".harness/config.json").exists());
    s.key(key(KeyCode::Enter));
    s.key(key(KeyCode::Char('y')));
    assert_eq!(config_json(repo.path()), json!({"typesafe": false}));
    assert_eq!(note(&s), "TypeSafe off");
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "TypeSafe  off").is_some(), "{:#?}", rows(&buf));
}

/// TypeSafe on without a key asks the key, shown as dots; it is kept as
/// init keeps it, readable only by you, and TypeSafe saves on.
#[test]
fn typesafe_on_without_a_key_asks_it_masked() {
    use std::os::unix::fs::PermissionsExt;
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".harness/config.json"),
        r#"{"typesafe": false}"#,
    );
    let mut s = screen_at(apps(""), repo.path());
    s.cfg.api_key = String::new();
    typesafe_page(&mut s);
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "ts_live_51c9d0e7");
    let buf = render(&s, 160, 45);
    assert!(
        find(&buf, "TypeSafe key › ••••••••••••••••▏").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "ts_live").is_none(), "{:#?}", rows(&buf));
    s.key(key(KeyCode::Enter));
    let file = repo.path().join(".harness/typesafe-key");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "ts_live_51c9d0e7\n"
    );
    let mode = std::fs::metadata(&file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(config_json(repo.path()), json!({"typesafe": true}));
    assert_eq!(s.cfg.api_key, "ts_live_51c9d0e7");
    assert_eq!(note(&s), "TypeSafe on");
    let buf = render(&s, 160, 45);
    assert!(find(&buf, "••••d0e7").is_some(), "{:#?}", rows(&buf));
}

/// u fetches a skill's source again and says the new commit; U fetches
/// every one, which here is up to date.
#[test]
fn updating_says_the_new_commit_or_up_to_date() {
    let repo = TempDir::new();
    let tools = clones();
    add(
        repo.path(),
        Path::new(""),
        &*tools,
        "mattpocock/skills",
        Some("tdd"),
    )
    .unwrap();
    let mut m = manifest(repo.path());
    m.skills.get_mut("tdd").unwrap().commit = "0000000old".to_string();
    m.save(repo.path()).unwrap();
    let mut s = skills_page(tools, repo.path());
    keys(&mut s, &[KeyCode::Down, KeyCode::Char('u')]);
    await_busy(&mut s);
    assert_eq!(note(&s), "updated tdd 0000000 → abc1234");
    assert_eq!(manifest(repo.path()).skills["tdd"].commit, "abc1234def");
    s.key(key(KeyCode::Char('U')));
    await_busy(&mut s);
    assert_eq!(note(&s), "every skill is up to date");
}

/// A suggested skill you have from another source is used as it is:
/// picked, nothing cloned.
#[test]
fn a_suggestion_installed_from_a_fork_is_picked_as_it_is() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".harness/skills.json"),
        r#"{"picks": {"test-first": "none"}}"#,
    );
    let tools = clones();
    add(
        repo.path(),
        Path::new(""),
        &*tools,
        "someone/fork",
        Some("tdd"),
    )
    .unwrap();
    let calls = tools.calls().len();
    let mut s = screen_at(tools.clone(), repo.path());
    type_line(&mut s, "/config");
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    let buf = render(&s, 160, 45);
    let (_, y) = find(&buf, "  tdd ").unwrap();
    assert!(
        cols(&buf, y, 99, 158).contains("someone/fork     installed"),
        "{:#?}",
        rows(&buf)
    );
    keys(
        &mut s,
        &[KeyCode::Up, KeyCode::Up, KeyCode::Up, KeyCode::Enter],
    );
    assert!(s.settings.as_ref().unwrap().busy.is_none());
    assert_eq!(manifest(repo.path()).pick("test-first"), "tdd");
    assert_eq!(note(&s), "Plan + Implement test-first picks tdd");
    let cloned = tools.calls()[calls..].iter().any(|c| c.contains("clone"));
    assert!(!cloned, "{:#?}", tools.calls());
}

/// A garbled Skill manifest still opens /config, saying why no skills
/// show, and a pick refuses rather than save over it.
#[test]
fn a_garbled_manifest_opens_config_and_refuses_a_pick() {
    let repo = TempDir::new();
    let file = repo.path().join(".harness/skills.json");
    write_file(&file, "{");
    let mut s = screen_at(apps(""), repo.path());
    type_line(&mut s, "/config");
    assert!(
        note(&s).starts_with(".harness/skills.json: "),
        "{}",
        note(&s)
    );
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down]);
    keys(&mut s, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
    type_in(&mut s, "none");
    s.key(key(KeyCode::Enter));
    assert!(note(&s).ends_with("Nothing changed."), "{}", note(&s));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{");
}
