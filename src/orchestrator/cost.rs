//! What each Ticket of an Epic cost, for the Epic summary: the tokens and
//! API-equivalent dollars of every session started in its worktree or its
//! Run directory, read from the transcripts claude, codex and pi keep under
//! home, and its time, read from the orchestrator log. TypeSafe's
//! Judgments are not counted.

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{NaiveDateTime, TimeDelta};
use serde_json::Value;

use super::stage::{run_dir, worktree};
use super::trust::claude_slug;

/// The Apps whose transcripts are read; any other is not supported yet.
pub(crate) const SUPPORTED: [&str; 3] = ["claude", "codex", "pi"];

/// Sessions' tokens and their cost at list prices: what the API would have
/// charged, not what a subscription billed.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Cost {
    pub(crate) tokens: u64,
    pub(crate) dollars: f64,
    /// Some tokens were on a model with no price: counted, not in dollars.
    pub(crate) unpriced: bool,
    /// The Apps its sessions ran on.
    pub(crate) apps: BTreeSet<String>,
}

impl Cost {
    pub(crate) fn add(&mut self, other: &Cost) {
        self.tokens += other.tokens;
        self.dollars += other.dollars;
        self.unpriced |= other.unpriced;
        self.apps.extend(other.apps.iter().cloned());
    }

    /// Tokens of kind [input, cache write 5m, cache write 1h, cache read,
    /// output] on model, priced by PRICES.
    fn charge(&mut self, model: &str, tokens: [u64; 5]) {
        self.tokens += tokens.iter().sum::<u64>();
        match price(model) {
            Some(per) => {
                let millionths: f64 = tokens.iter().zip(per).map(|(n, p)| *n as f64 * p).sum();
                self.dollars += millionths / 1e6;
            }
            None => self.unpriced |= tokens.iter().any(|n| *n > 0),
        }
    }
}

/// List prices per million tokens: input, cache write 5m, cache write 1h,
/// cache read, output. OpenAI charges no cache write. Claude's from the
/// claude-api skill, OpenAI's from its pricing page's Standard tier, both
/// read 2026-09-26.
// ponytail: short-context rates; OpenAI doubles them past its long-context
// threshold, and Claude's fast mode is priced as standard.
const PRICES: [(&str, [f64; 5]); 18] = [
    ("claude-fable-5-1", [10.0, 12.5, 20.0, 0.25, 50.0]),
    ("claude-mythos-5-1", [10.0, 12.5, 20.0, 0.25, 50.0]),
    ("claude-fable-5", [10.0, 12.5, 20.0, 1.0, 50.0]),
    ("claude-mythos-5", [10.0, 12.5, 20.0, 1.0, 50.0]),
    ("claude-opus-5-5", [4.0, 5.0, 8.0, 0.2, 20.0]),
    ("claude-opus-5", [5.0, 6.25, 10.0, 0.5, 25.0]),
    ("claude-opus-4-8", [5.0, 6.25, 10.0, 0.5, 25.0]),
    ("claude-opus-4-7", [5.0, 6.25, 10.0, 0.5, 25.0]),
    ("claude-opus-4-6", [5.0, 6.25, 10.0, 0.5, 25.0]),
    ("claude-sonnet-5", [2.0, 2.5, 4.0, 0.2, 10.0]),
    ("claude-sonnet-4-6", [3.0, 3.75, 6.0, 0.3, 15.0]),
    ("claude-haiku-4-5", [1.0, 1.25, 2.0, 0.1, 5.0]),
    ("gpt-6-astra", [10.0, 0.0, 0.0, 1.0, 50.0]),
    ("gpt-6-sol", [2.0, 0.0, 0.0, 0.2, 10.0]),
    ("gpt-6-luna", [0.1, 0.0, 0.0, 0.01, 0.5]),
    ("gpt-5.6-sol", [4.0, 0.0, 0.0, 0.4, 20.0]),
    ("gpt-5.6-luna", [0.2, 0.0, 0.0, 0.02, 1.2]),
    ("gpt-5.3-codex", [1.75, 0.0, 0.0, 0.175, 14.0]),
];

/// A model's prices: its id alone, or with a date after it
/// (claude-haiku-4-5-20251001).
fn price(model: &str) -> Option<[f64; 5]> {
    let dated = |rest: &str| {
        rest.strip_prefix('-')
            .is_some_and(|date| date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()))
    };
    PRICES
        .iter()
        .find(|(id, _)| {
            model
                .strip_prefix(id)
                .is_some_and(|rest| rest.is_empty() || dated(rest))
        })
        .map(|(_, per)| *per)
}

/// Each Ticket's Cost by its id: a transcript is a Ticket's when its
/// session started in exactly the Ticket's worktree or Run directory. A
/// Ticket with none has no entry.
pub(crate) fn costs(home: &Path, repo: &Path, tickets: &[&str]) -> HashMap<String, Cost> {
    let folders: HashMap<PathBuf, &str> = tickets
        .iter()
        .flat_map(|t| [(worktree(repo, t), *t), (run_dir(repo, t), *t)])
        .collect();
    let mut out: HashMap<String, Cost> = HashMap::new();
    let mut found = |ticket: &str, app: &str| {
        let cost = out.entry(ticket.to_string()).or_default();
        cost.apps.insert(app.to_string());
    };

    // claude writes one API message on several lines, the last with its
    // final output count: (message id, request id) -> its Ticket, model and
    // tokens, the last line winning.
    let mut messages = HashMap::new();
    for (folder, ticket) in &folders {
        let dir = home.join(".claude/projects").join(claude_slug(folder));
        for file in jsonl(&dir) {
            if started(&file).as_ref() != Some(folder) {
                continue;
            }
            found(ticket, "claude");
            for line in lines(&file, &["\"usage\""]) {
                let (m, usage) = (&line["message"], &line["message"]["usage"]);
                if !usage.is_object() {
                    continue;
                }
                let n = |key: &str| count(&usage[key]);
                let write = n("cache_creation_input_tokens");
                let write_1h = count(&usage["cache_creation"]["ephemeral_1h_input_tokens"]);
                let tokens = [
                    n("input_tokens"),
                    write.saturating_sub(write_1h),
                    write_1h,
                    n("cache_read_input_tokens"),
                    n("output_tokens"),
                ];
                let key = (text(&m["id"]), text(&line["requestId"]));
                messages.insert(key, (*ticket, text(&m["model"]), tokens));
            }
        }
    }

    // codex's token counts are running totals: each is priced on the model
    // of the turn it closed, as far as it grew. A sub-thread's rollout is its
    // own file and total, not in its parent's.
    let mut codex = Vec::new();
    for file in jsonl(&home.join(".codex/sessions")) {
        let Some(ticket) = opened(&file).and_then(|f| folders.get(&f).copied()) else {
            continue;
        };
        found(ticket, "codex");
        let (mut model, mut last) = (String::new(), [0; 3]);
        for line in lines(&file, &["\"turn_context\"", "\"token_count\""]) {
            let payload = &line["payload"];
            match (
                text(&line["type"]).as_str(),
                text(&payload["type"]).as_str(),
            ) {
                ("turn_context", _) => model = text(&payload["model"]),
                ("event_msg", "token_count") if !payload["info"].is_null() => {
                    let total = &payload["info"]["total_token_usage"];
                    // cached is part of input; reasoning part of output
                    let now = ["input_tokens", "cached_input_tokens", "output_tokens"]
                        .map(|key| count(&total[key]));
                    let [input, cached, output] = [0, 1, 2].map(|i| now[i].saturating_sub(last[i]));
                    let tokens = [input.saturating_sub(cached), 0, 0, cached, output];
                    codex.push((ticket, model.clone(), tokens));
                    last = now;
                }
                _ => {}
            }
        }
    }

    // pi logs each message's cost itself.
    let mut pi = Vec::new();
    for file in jsonl(&home.join(".pi/agent/sessions")) {
        let Some(ticket) = opened(&file).and_then(|f| folders.get(&f).copied()) else {
            continue;
        };
        found(ticket, "pi");
        for line in lines(&file, &["\"usage\""]) {
            let usage = &line["message"]["usage"];
            if line["type"] == "message" && usage.is_object() {
                pi.push((
                    ticket,
                    count(&usage["totalTokens"]),
                    usage["cost"]["total"].as_f64(),
                ));
            }
        }
    }

    for (ticket, model, tokens) in messages.into_values().chain(codex) {
        out.entry(ticket.to_string())
            .or_default()
            .charge(&model, tokens);
    }
    for (ticket, tokens, dollars) in pi {
        let cost = out.entry(ticket.to_string()).or_default();
        cost.tokens += tokens;
        match dollars {
            Some(dollars) => cost.dollars += dollars,
            None => cost.unpriced |= tokens > 0,
        }
    }
    out
}

/// Every .jsonl file under dir, at any depth; none when dir is missing.
fn jsonl(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(jsonl(&path));
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(path);
        }
    }
    out
}

/// A transcript's lines that parse as JSON, read as they are needed. Only
/// those having one of the words are parsed: most of a transcript is
/// messages and tool output, which cost nothing to skip.
fn lines<'a>(file: &Path, having: &'a [&'a str]) -> impl Iterator<Item = Value> + 'a {
    File::open(file)
        .into_iter()
        .flat_map(|f| BufReader::new(f).lines())
        .map_while(Result::ok)
        .filter(|line| having.iter().any(|word| line.contains(word)))
        .filter_map(|line| serde_json::from_str(&line).ok())
}

/// Where a transcript's session started: the first cwd on its lines,
/// codex's in its payload.
fn started(file: &Path) -> Option<PathBuf> {
    lines(file, &["\"cwd\""]).find_map(|line| {
        let cwd = line.get("cwd").or(line["payload"].get("cwd"))?;
        cwd.as_str().map(PathBuf::from)
    })
}

/// Where a codex or pi session started: the cwd on its first line, codex's
/// in its payload. Only that line is read, as every session on the machine is.
fn opened(file: &Path) -> Option<PathBuf> {
    let line = BufReader::new(File::open(file).ok()?)
        .lines()
        .next()?
        .ok()?;
    let line: Value = serde_json::from_str(&line).ok()?;
    let cwd = line.get("cwd").or(line["payload"].get("cwd"))?;
    cwd.as_str().map(PathBuf::from)
}

fn count(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// A Ticket's time in the log: its first line to its PR's opening, or to
/// its last line while it has no PR, so waiting on a merge is not counted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Span {
    pub(crate) start: NaiveDateTime,
    pub(crate) end: NaiveDateTime,
    /// It ends at its PR's opening.
    pub(crate) pr: bool,
}

impl Span {
    pub(crate) fn length(&self) -> TimeDelta {
        self.end - self.start
    }
}

/// A Ticket as the orchestrator log has it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Logged {
    /// None with no lines.
    pub(crate) span: Option<Span>,
    /// The Apps its Stages started on, from each started line.
    pub(crate) apps: BTreeSet<String>,
}

/// Every Ticket in .orqadence/orchestrator.log by its id, the third field of
/// a line 'YYYY-MM-DD HH:MM:SS <id> <text>' (stage::log_line); a line that
/// does not parse is skipped. The file is kept across runs.
pub(crate) fn logged(repo: &Path) -> HashMap<String, Logged> {
    let raw = fs::read_to_string(repo.join(".orqadence/orchestrator.log")).unwrap_or_default();
    let mut out: HashMap<String, Logged> = HashMap::new();
    for line in raw.lines() {
        let mut fields = line.splitn(4, ' ');
        let (Some(day), Some(time), Some(id), Some(text)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Ok(at) = NaiveDateTime::parse_from_str(&format!("{day} {time}"), "%Y-%m-%d %H:%M:%S")
        else {
            continue;
        };
        let logged = out.entry(id.to_string()).or_default();
        let span = logged.span.get_or_insert(Span {
            start: at,
            end: at,
            pr: false,
        });
        // "PR #22 opened after 2 rounds"; a later one is a re-run's, ignored
        let opened = text.starts_with("PR #") && text.contains(" opened ");
        if !span.pr {
            span.end = at;
            span.pr |= opened;
        }
        if let Some((_, row)) = text.split_once(" started: ") {
            logged
                .apps
                .extend(row.split(' ').next().map(str::to_string));
        }
    }
    out
}

/// The Epic's time on the wall clock: its earliest Ticket's start to its
/// latest's end, since Tickets run side by side. None with no Span.
pub(crate) fn wall_clock<'a>(spans: impl Iterator<Item = &'a Span>) -> Option<TimeDelta> {
    let (starts, ends): (Vec<_>, Vec<_>) = spans.map(|s| (s.start, s.end)).unzip();
    Some(*ends.iter().max()? - *starts.iter().min()?)
}
