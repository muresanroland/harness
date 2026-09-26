use super::cost::{costs, logged, wall_clock, Cost};
use super::stage::{run_dir, worktree};
use super::trust::claude_slug;
use super::write_file;
use crate::tempdir::TempDir;
use chrono::{NaiveDateTime, TimeDelta};
use std::path::Path;

const CLAUDE: &str = include_str!("testdata/cost/claude-session.jsonl");
const CLAUDE_SUBAGENT: &str = include_str!("testdata/cost/claude-subagent.jsonl");
const CODEX: &str = include_str!("testdata/cost/codex-rollout.jsonl");
const PI: &str = include_str!("testdata/cost/pi-session.jsonl");
const LOG: &str = include_str!("testdata/cost/orchestrator.log");

/// A fixture transcript at path under home, its session started in cwd.
fn place(home: &Path, path: &str, fixture: &str, cwd: &Path, model: &str) {
    let body = fixture
        .replace("{cwd}", &cwd.display().to_string())
        .replace("{model}", model);
    write_file(&home.join(path), &body);
}

/// claude's transcript of a session started in cwd, on model.
fn claude(home: &Path, cwd: &Path, model: &str) {
    let path = format!(".claude/projects/{}/s1.jsonl", claude_slug(cwd));
    place(home, &path, CLAUDE, cwd, model);
}

fn cost_of(home: &Path, repo: &Path, ticket: &str) -> Cost {
    costs(home, repo, &[ticket])
        .remove(ticket)
        .unwrap_or_default()
}

/// Its tokens, and its dollars to the cent.
fn spent(cost: &Cost) -> (u64, f64, bool) {
    let cents = (cost.dollars * 100.0).round() / 100.0;
    (cost.tokens, cents, cost.unpriced)
}

/// claude writes one API message on several lines: each (message id,
/// request id) counts once, with its last line's output. A zero usage line
/// on <synthetic> leaves it priced.
#[test]
fn claude_counts_a_message_written_on_several_lines_once() {
    let (home, repo) = (TempDir::new(), TempDir::new());
    claude(
        home.path(),
        &worktree(repo.path(), "hx.1"),
        "claude-opus-5-5",
    );
    let cost = cost_of(home.path(), repo.path(), "hx.1");
    // msg_a: 400k 5m writes $2, 500k 1h writes $4, 1M reads $0.20, 100k out $2;
    // msg_b: 1M in $4
    assert_eq!(spent(&cost), (3_000_000, 12.2, false));
    assert_eq!(cost.apps.iter().collect::<Vec<_>>(), ["claude"]);
}

/// A subagent's transcript, in its session's folder, is the Ticket's too;
/// its model priced with its date after it.
#[test]
fn claude_subagent_transcripts_count_too() {
    let (home, repo) = (TempDir::new(), TempDir::new());
    let dir = worktree(repo.path(), "hx.1");
    claude(home.path(), &dir, "claude-opus-5-5");
    let path = format!(
        ".claude/projects/{}/s1/subagents/agent-a1.jsonl",
        claude_slug(&dir)
    );
    place(home.path(), &path, CLAUDE_SUBAGENT, &dir, "");
    // plus 1M in $1 and 200k out $1 on haiku
    let cost = cost_of(home.path(), repo.path(), "hx.1");
    assert_eq!(spent(&cost), (4_200_000, 14.2, false));
}

/// A transcript is a Ticket's only when its session started in exactly the
/// Ticket's worktree or Run directory: hx.10's is not hx.1's, nor is one
/// whose folder claude slugs the same as hx.1's worktree.
#[test]
fn a_session_is_a_tickets_only_in_exactly_its_folder() {
    let (home, repo) = (TempDir::new(), TempDir::new());
    claude(
        home.path(),
        &worktree(repo.path(), "hx.10"),
        "claude-opus-5-5",
    );
    let (hx1, twin) = (worktree(repo.path(), "hx.1"), worktree(repo.path(), "hx-1"));
    assert_eq!(claude_slug(&hx1), claude_slug(&twin));
    claude(home.path(), &twin, "claude-opus-5-5");
    place(
        home.path(),
        ".codex/sessions/2026/09/24/rollout-1.jsonl",
        CODEX,
        &run_dir(repo.path(), "hx.1"),
        "",
    );
    let mut got = costs(home.path(), repo.path(), &["hx.1", "hx.10"]);
    let one = got.remove("hx.1").unwrap();
    assert_eq!(one.apps.iter().collect::<Vec<_>>(), ["codex"]);
    assert_eq!(one.tokens, 2_200_000, "hx.1 took another folder's session");
    assert_eq!(got.remove("hx.10").map(|c| c.tokens), Some(3_000_000));
}

/// codex's token counts are running totals, some null: the last one not
/// null is the rollout's. Cached input is part of input, and reasoning part
/// of output, so neither counts twice.
#[test]
fn codex_takes_its_last_running_total() {
    let (home, repo) = (TempDir::new(), TempDir::new());
    let dir = worktree(repo.path(), "hx.1");
    place(
        home.path(),
        ".codex/sessions/2026/09/24/rollout-1.jsonl",
        CODEX,
        &dir,
        "",
    );
    // gpt-6-sol: 500k in $1, 1.5M cached $0.30, 200k out $2
    let cost = cost_of(home.path(), repo.path(), "hx.1");
    assert_eq!(spent(&cost), (2_200_000, 3.3, false));
}

/// pi logs each message's cost: that is used, not the price table, which
/// has no row for its model.
#[test]
fn pi_uses_the_cost_it_logged() {
    let (home, repo) = (TempDir::new(), TempDir::new());
    let dir = run_dir(repo.path(), "hx.1");
    place(
        home.path(),
        ".pi/agent/sessions/--x--/p1.jsonl",
        PI,
        &dir,
        "",
    );
    let cost = cost_of(home.path(), repo.path(), "hx.1");
    assert_eq!(spent(&cost), (3_000, 0.75, false));
    assert_eq!(cost.apps.iter().collect::<Vec<_>>(), ["pi"]);
}

/// A model with no price counts its tokens but no dollars, and marks the
/// cost unpriced; the priced ones still count.
#[test]
fn a_model_with_no_price_counts_its_tokens_but_no_dollars() {
    let (home, repo) = (TempDir::new(), TempDir::new());
    let dir = worktree(repo.path(), "hx.1");
    claude(home.path(), &dir, "claude-nope-9");
    let path = format!(
        ".claude/projects/{}/s1/subagents/agent-a1.jsonl",
        claude_slug(&dir)
    );
    place(home.path(), &path, CLAUDE_SUBAGENT, &dir, "");
    let cost = cost_of(home.path(), repo.path(), "hx.1");
    assert_eq!(spent(&cost), (4_200_000, 2.0, true));
}

fn at(time: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(&format!("2026-09-24 {time}"), "%Y-%m-%d %H:%M:%S").unwrap()
}

fn log_repo() -> TempDir {
    let repo = TempDir::new();
    write_file(&repo.path().join(".harness/orchestrator.log"), LOG);
    repo
}

/// A Ticket's time runs from its first line to its PR's opening: a merge
/// waited on after it is not counted. Lines that do not parse are skipped.
#[test]
fn a_tickets_time_ends_at_its_pr_opening() {
    let repo = log_repo();
    let one = logged(repo.path()).remove("hx.1").unwrap();
    let span = one.span.unwrap();
    assert_eq!(
        (span.start, span.end, span.pr),
        (at("17:00:00"), at("18:32:00"), true)
    );
    assert_eq!(span.length(), TimeDelta::minutes(92));
    assert_eq!(one.apps.iter().collect::<Vec<_>>(), ["claude"]);
}

/// With no PR yet, a Parked or running Ticket's time runs to its last line.
#[test]
fn a_ticket_with_no_pr_runs_to_its_last_line() {
    let repo = log_repo();
    let span = logged(repo.path())["hx.10"].span.unwrap();
    assert_eq!(
        (span.start, span.end, span.pr),
        (at("17:10:00"), at("19:05:00"), false)
    );
}

/// The id is matched whole, as the third field: hx.1's lines are not
/// hx.10's, and a Ticket not in the log has no time.
#[test]
fn the_log_matches_a_ticket_id_exactly() {
    let repo = log_repo();
    let log = logged(repo.path());
    let apps: Vec<_> = log["hx.10"].apps.iter().collect();
    assert_eq!(apps, ["codex", "opencode"]);
    assert_eq!(log["hx.10"].span.unwrap().start, at("17:10:00"));
    assert!(!log.contains_key("hx.2"));
    assert!(!log.contains_key("hx"));
}

/// The Epic's time is the wall clock over its Tickets, which overlap: the
/// earliest start to the latest end, 2h 5m, not the 3h 27m they add up to.
#[test]
fn the_epics_time_is_the_wall_clock_over_overlapping_tickets() {
    let repo = log_repo();
    let log = logged(repo.path());
    let spans = ["hx.1", "hx.10"].map(|t| log[t].span.unwrap());
    assert_eq!(wall_clock(spans.iter()), Some(TimeDelta::minutes(125)));
    assert_eq!(wall_clock([].iter()), None);
}
