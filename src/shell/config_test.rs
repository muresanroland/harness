//! /config over fake Tools and the fake world: the docked modal, its pick
//! lists, the probe, and a change saved during a run.

use super::logo::PURPLE;
use super::shell_test::{
    asking, await_line, cols, find, key, logged, render, row, rows, screen_at, shell, type_in,
    type_line,
};
use super::Screen;
use crate::orchestrator::stage::Ask;
use crate::orchestrator::world::{new_world, succeed, BdTicket};
use crate::orchestrator::write_file;
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
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);
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
    keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);
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
    let right: Vec<String> = (1..19).map(|y| text(&buf, y, 99, 158)).collect();
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
    keys(&mut s, &[KeyCode::Down, KeyCode::Enter]);
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
        keys(&mut s, &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);
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
