//! /demo: a scripted run on a made-up Epic, to see what a live run looks
//! like with nothing started. Its Tickets move through their Stages, RECENT
//! fills, each kind of Question asks and the script holds until you answer,
//! a Ticket waits on a merge, an App hits its usage limit, and the Epic
//! summary opens at the end. Nothing reaches herdr, bd, the state file or
//! the log; /stop-demo (or /stop-work) puts the Shell back as it was.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use super::summary::{Summary, Ticket};
use super::{About, Epic, Screen};
use crate::orchestrator::cost::{Cost, Span};
use crate::orchestrator::judgment::{Action, Judged, PlanJudged};
use crate::orchestrator::limit::until;
use crate::orchestrator::scheduler::{BdDependency, BdIssue};
use crate::orchestrator::stage::{Answer, Ask, Event};
use crate::orchestrator::state::{
    State, STATUS_MERGED, STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING,
};

const EPIC: &str = "orqa-demo";
/// Ticks between two steps: 1.25 s at a live run's 50 ms tick.
const STEP: u64 = 25;
/// How long the made-up usage limit holds from when it is hit.
const LIMIT: chrono::TimeDelta = chrono::TimeDelta::minutes(2);

/// A Ticket's title and the suffix of the Ticket it is blocked on, and the
/// summary's Findings fixed and skipped, tokens and dollars.
type Made = (&'static str, Option<usize>, usize, &'static str, u64, f64);

const TICKETS: [Made; 4] = [
    (
        "Parse the config file",
        None,
        3,
        "low src/config.rs:88 parse is one long function",
        2_840_000,
        6.12,
    ),
    (
        "Retry failed uploads",
        None,
        1,
        "low src/upload.rs:41 the backoff is not configurable",
        1_960_000,
        4.37,
    ),
    (
        "Show upload progress",
        Some(1),
        0,
        "low src/progress.rs:12 no bar per chunk",
        910_000,
        1.84,
    ),
    (
        "Rate-limit the API client",
        None,
        0,
        "medium src/client.rs:30 requests past the limit fail, not queue",
        1_420_000,
        3.05,
    ),
];

/// What a step does besides its line.
enum Then {
    Line,
    /// The Ticket runs at this Stage and Round.
    Stage(&'static str, usize),
    /// Its PR, by number, is open.
    Pr(u32),
    Merged,
    /// codex hits its usage limit, holding the Ticket.
    Limit,
    Unlimit,
    /// The line asks you; the script holds until it is answered.
    Asks(Kind),
}

#[derive(Clone, Copy)]
enum Kind {
    Plan,
    Question,
    Blocked,
    Limited,
    Wake,
}

use Then::*;

/// The run: a Ticket's suffix (0 for a run-level line), its line in the
/// Orchestrator's words (docs/design/events.md), and what else it does.
const SCRIPT: &[(usize, &str, Then)] = &[
    (
        0,
        "demo: a made-up Epic, nothing starts and nothing is written, /stop-demo ends it",
        Line,
    ),
    (1, "branch orqa-demo.1 created", Stage("implement", 0)),
    (1, "implement started: claude opus/high (pane 2-1)", Line),
    (2, "branch orqa-demo.2 created", Stage("implement", 0)),
    (2, "implement started: claude opus/high (pane 3-1)", Line),
    (4, "branch orqa-demo.4 created", Stage("implement", 0)),
    (4, "implement started: codex gpt-5.5/high (pane 4-1)", Line),
    (1, "plan ready in implement (pane 2-1)", Line),
    (
        1,
        "judged: plan covers the Ticket 0.91, stays in scope 0.88, asks nothing 0.95",
        Line,
    ),
    (1, "plan approved", Line),
    (2, "plan ready in implement (pane 3-1)", Line),
    (2, "plan ready in implement (pane 3-1)", Asks(Kind::Plan)),
    (4, "question in implement (pane 4-1)", Asks(Kind::Question)),
    (1, "implemented", Line),
    (
        1,
        "review 1 started: codex gpt-5.5/high (pane 2-2)",
        Stage("review", 1),
    ),
    (2, "implemented", Line),
    (
        2,
        "review 1 started: codex gpt-5.5/high (pane 3-2)",
        Stage("review", 1),
    ),
    (1, "review 1 found 4 findings", Line),
    (
        1,
        "debate 1 started: claude opus/high (pane 2-3)",
        Stage("debate", 1),
    ),
    (
        2,
        "codex usage limit until {until}: review 1 holds (pane 3-2)",
        Limit,
    ),
    (
        2,
        "codex limited until {until}: how do Reviews go until then?",
        Asks(Kind::Limited),
    ),
    (
        4,
        "waiting at a prompt in implement (pane 4-1)",
        Asks(Kind::Blocked),
    ),
    (1, "debate 1 settled: 3 to fix, 1 skipped", Line),
    (
        1,
        "fix 1 started: claude opus/high (pane 2-4)",
        Stage("fix", 1),
    ),
    (
        1,
        "stuck in fix 1: went idle without a result (pane 2-4)",
        Asks(Kind::Wake),
    ),
    (4, "implemented", Line),
    (1, "fix 1 done", Line),
    (1, "PR #41 opened after 1 round", Pr(41)),
    (3, "waiting for PR #41 to merge (Ticket 1)", Line),
    (
        2,
        "codex usage limit over: review 1 carries on (pane 3-2)",
        Unlimit,
    ),
    (
        4,
        "review 1 started: codex gpt-5.5/high (pane 4-2)",
        Stage("review", 1),
    ),
    (2, "review 1 found 2 findings", Line),
    (
        2,
        "debate 1 started: claude opus/high (pane 3-3)",
        Stage("debate", 1),
    ),
    (4, "review 1 found 1 finding", Line),
    (1, "merged, Ticket closed", Merged),
    (3, "branch orqa-demo.3 created", Stage("implement", 0)),
    (
        3,
        "implement started: claude sonnet/medium (pane 5-1)",
        Line,
    ),
    (2, "debate 1 settled: 1 to fix, 1 skipped", Line),
    (
        2,
        "fix 1 started: claude opus/high (pane 3-4)",
        Stage("fix", 1),
    ),
    (
        4,
        "debate 1 started: claude opus/high (pane 4-3)",
        Stage("debate", 1),
    ),
    (3, "plan ready in implement (pane 5-1)", Line),
    (
        3,
        "judged: plan covers the Ticket 0.93, stays in scope 0.97, asks nothing 0.99",
        Line,
    ),
    (3, "plan approved", Line),
    (4, "debate 1 settled: 0 to fix, 1 skipped", Line),
    (
        4,
        "fix 1 started: claude opus/high (pane 4-4)",
        Stage("fix", 1),
    ),
    (2, "fix 1 done", Line),
    (2, "PR #42 opened after 1 round", Pr(42)),
    (3, "implemented", Line),
    (
        3,
        "review 1 started: codex gpt-5.5/high (pane 5-2)",
        Stage("review", 1),
    ),
    (4, "fix 1 done", Line),
    (4, "PR #43 opened after 1 round", Pr(43)),
    (4, "PR #43 conflicts with main, /address resolves it", Line),
    (3, "review 1 found 1 finding", Line),
    (
        3,
        "debate 1 settled: 0 to fix, 1 skipped",
        Stage("debate", 1),
    ),
    (
        3,
        "fix 1 started: claude opus/high (pane 5-4)",
        Stage("fix", 1),
    ),
    (3, "fix 1 done", Line),
    (3, "PR #44 opened after 1 round", Pr(44)),
];

const PLAN: &str = "\
# Plan: Retry failed uploads

## Changes
- `upload::send` retries a failed chunk up to 3 times, backing off 1s, 2s, 4s
- a chunk that still fails marks the upload failed and names the chunk

## Tests
- a chunk failing twice, then passing, uploads whole
- a chunk failing four times fails the upload, naming the chunk

## Decisions
- Retries live in `send`, not its callers: every upload path gets them
- The backoff is fixed: nobody has asked to tune it
";

const TAIL: &str = "\
⏺ Fixed F1: the parser names the line of a bad key
⏺ Fixed F2: an empty file loads as the defaults
⏺ Fixed F3: a test for a missing file
  cargo test: 48 passed
>";

/// What the Shell showed before the demo, put back when it ends, and where
/// the script is.
pub(crate) struct Demo {
    epics: Vec<Epic>,
    state: State,
    events: Vec<Event>,
    /// The next step of SCRIPT.
    pub(super) step: usize,
    /// The tick it plays on.
    at: u64,
    started: chrono::DateTime<chrono::Local>,
}

/// /demo: the made-up Epic in place of the Shell's, as a live run.
pub(super) fn start(s: &mut Screen) {
    let id = |n: usize| format!("{EPIC}.{n}");
    let tickets = TICKETS
        .iter()
        .enumerate()
        .map(|(k, (title, blocker, ..))| BdIssue {
            id: id(k + 1),
            title: title.to_string(),
            status: "open".to_string(),
            issue_type: "task".to_string(),
            parent: EPIC.to_string(),
            dependencies: blocker
                .map(|b| BdDependency {
                    depends_on_id: id(b),
                    kind: "blocks".to_string(),
                })
                .into_iter()
                .collect(),
            ..Default::default()
        })
        .collect();
    let epic = Epic {
        id: EPIC.to_string(),
        title: "Demo: resumable uploads".to_string(),
        tickets,
    };
    let state = State {
        epic: EPIC.to_string(),
        ..Default::default()
    };
    s.demo = Some(Demo {
        epics: std::mem::replace(&mut s.epics, vec![epic]),
        state: std::mem::replace(&mut s.state, state),
        events: std::mem::take(&mut s.events),
        step: 0,
        at: s.ticks,
        started: (s.cfg.clock)(),
    });
    s.running = true;
    s.started = s.ticks;
    s.recent.set(0);
    s.scroll.set(0);
}

/// Puts the Shell back as it was before the demo; its Questions go.
pub(super) fn stop(s: &mut Screen) {
    let Some(demo) = s.demo.take() else {
        return;
    };
    s.epics = demo.epics;
    s.state = demo.state;
    s.events = demo.events;
    s.questions.retain(|q| q.ticket.is_none());
    if s.composing {
        s.composing = false;
        s.input.clear();
    }
    s.hidden = false;
    s.running = false;
    s.recent.set(0);
    s.scroll.set(0);
}

/// Plays the next step once its tick comes; while a Question waits the
/// script holds, and the step after your answer comes a step on. A parked
/// Ticket's steps are skipped, and those of a Ticket blocked on it. Past
/// the last step the summary opens and the demo ends.
pub(super) fn tick(s: &mut Screen) {
    let waiting = !s.questions.is_empty();
    let Some(demo) = &mut s.demo else {
        return;
    };
    if waiting {
        demo.at = s.ticks + STEP;
    }
    if waiting || s.ticks < demo.at {
        return;
    }
    demo.at = s.ticks + STEP;
    let from = demo.step;
    match (from..SCRIPT.len()).find(|&k| !skipped(s, SCRIPT[k].0)) {
        Some(k) => {
            s.demo.as_mut().unwrap().step = k + 1;
            play(s, &SCRIPT[k]);
        }
        None => {
            s.summary = Some(summary(s));
            stop(s);
        }
    }
}

/// Whether Ticket `n` or the Ticket it is blocked on is Parked.
fn skipped(s: &Screen, n: usize) -> bool {
    let parked = |id: &str| {
        s.state
            .tickets
            .get(id)
            .is_some_and(|ts| ts.status == STATUS_PARKED)
    };
    n.checked_sub(1)
        .and_then(|k| s.epics[0].tickets.get(k))
        .is_some_and(|t| parked(&t.id) || t.blockers().any(parked))
}

/// One step: its change to the State, then its line as the Orchestrator's
/// Event, which the Shell shows, and asks with, as in a live run.
fn play(s: &mut Screen, (n, text, then): &(usize, &str, Then)) {
    let id = (*n > 0).then(|| format!("{EPIC}.{n}"));
    let now = (s.cfg.clock)();
    let reset = s.state.limits.get("codex").copied().unwrap_or(now + LIMIT);
    if let (Some(id), false) = (&id, matches!(then, Line | Asks(_))) {
        let ts = s.state.tickets.entry(id.clone()).or_default();
        match *then {
            Stage(stage, round) => {
                ts.status = STATUS_RUNNING.to_string();
                ts.stage = stage.to_string();
                ts.round = round;
            }
            Pr(pr) => {
                ts.status = STATUS_PR_OPEN.to_string();
                ts.pr = format!("https://github.com/you/uploads/pull/{pr}");
            }
            Merged => ts.status = STATUS_MERGED.to_string(),
            Limit => {
                ts.limited = "codex".to_string();
                s.state.limits.insert("codex".to_string(), reset);
            }
            Unlimit => {
                ts.limited.clear();
                s.state.limits.clear();
            }
            Line | Asks(_) => {}
        }
    }
    let ask = match *then {
        Asks(kind) => Some(asked(kind)),
        _ => None,
    };
    s.push(Event {
        time: now,
        ticket: id,
        text: text.replace("{until}", &until(reset, now)),
        // a plan and the Review's limit ask with no line of their own
        panel: !matches!(then, Asks(Kind::Plan | Kind::Limited)),
        ask,
    });
}

/// The Question each kind of ask puts to you.
fn asked(kind: Kind) -> Ask {
    match kind {
        Kind::Plan => Ask::Plan {
            pane: "3-1".to_string(),
            plan: PLAN.to_string(),
            judged: Some(PlanJudged {
                covers: 0.58,
                in_scope: 0.91,
                asks: 0.12,
                floor: Some(0.65),
            }),
            feedback: None,
        },
        Kind::Question => Ask::StageQuestion {
            pane: "4-1".to_string(),
            question: "The API allows 100 requests a minute. Should the client queue the requests past it, or fail them at once with the time to retry?".to_string(),
            options: vec![
                "queue them, in order".to_string(),
                "fail them with a retry-after".to_string(),
            ],
        },
        Kind::Blocked => Ask::Blocked {
            pane: "4-1".to_string(),
        },
        Kind::Limited => Ask::Limited {
            app: "codex".to_string(),
            fallback: Some("claude opus".to_string()),
        },
        Kind::Wake => Ask::Wake {
            pane: "2-4".to_string(),
            tail: TAIL.to_string(),
            file: PathBuf::from(".orqadence/runs/orqa-demo.1/fix-1.md"),
            actions: Action::ALL.to_vec(),
            judged: Some(Judged {
                choice: Action::NudgeWriteResult,
                confidence: 0.55,
                scores: vec![
                    (Action::NudgeWriteResult, 0.55),
                    (Action::NudgeProceed, 0.21),
                    (Action::Retry, 0.14),
                    (Action::Park, 0.06),
                    (Action::Wait, 0.04),
                ],
            }),
        },
    }
}

/// Your answer's second line, as the Orchestrator says it once it acts; a
/// park takes the Ticket out, and the script skips the rest of its steps.
pub(super) fn answered(s: &mut Screen, id: &str, about: &About, answer: Answer) {
    let ts = s.state.tickets.entry(id.to_string()).or_default();
    let at = match ts.round {
        0 => ts.stage.clone(),
        round => format!("{} {round}", ts.stage),
    };
    let text = match (about, answer) {
        (_, Answer::Approve) => "plan approved".to_string(),
        (About::Asked(Ask::Plan { .. }), Answer::Prompt(_)) => {
            "plan sent back with your feedback".to_string()
        }
        (About::Asked(Ask::StageQuestion { .. }), Answer::Prompt(_)) => {
            "sent your answer".to_string()
        }
        (_, Answer::Prompt(_)) => "nudged with your prompt".to_string(),
        (_, Answer::Act(Action::Park)) => {
            ts.status = STATUS_PARKED.to_string();
            ts.reason = format!("by you at {at}");
            format!("parked: {}", ts.reason)
        }
        (_, Answer::Act(Action::Retry)) => format!("retrying {at} with a fresh session"),
        (_, Answer::Act(Action::Wait)) => "waiting: still working".to_string(),
        (_, Answer::Act(nudge)) => {
            let said = nudge.nudge(Path::new("")).map_or("", |(_, said)| said);
            format!("nudged: {said}")
        }
    };
    s.push(Event {
        time: (s.cfg.clock)(),
        ticket: Some(id.to_string()),
        text,
        panel: true,
        ask: None,
    });
}

/// The Epic summary the run ends on, from the State the script left and
/// TICKETS' made-up Findings and cost.
fn summary(s: &Screen) -> Summary {
    let now = (s.cfg.clock)();
    let started = s.demo.as_ref().map_or(now, |d| d.started);
    let tickets: Vec<Ticket> = s.epics[0]
        .tickets
        .iter()
        .zip(TICKETS)
        .map(|(t, (_, _, fixed, skipped, tokens, dollars))| {
            let ts = s.state.tickets.get(&t.id).cloned().unwrap_or_default();
            let pr = !ts.pr.is_empty();
            Ticket {
                id: t.id.clone(),
                title: t.title.clone(),
                merged: ts.status == STATUS_MERGED,
                rounds: usize::from(pr),
                fixed,
                skipped: vec![skipped.to_string()],
                left: Vec::new(),
                parked: (ts.status == STATUS_PARKED).then_some(ts.reason),
                cost: Cost {
                    tokens,
                    dollars,
                    unpriced: false,
                    apps: ["claude", "codex"].map(String::from).into(),
                },
                time: Some(Span {
                    start: started.naive_local(),
                    end: now.naive_local(),
                    pr,
                }),
                pr: ts.pr,
            }
        })
        .collect();
    let mut cost = Cost::default();
    for t in &tickets {
        cost.add(&t.cost);
    }
    Summary {
        epic: EPIC.to_string(),
        title: s.epics[0].title.clone(),
        tickets,
        cost,
        time: Some(now - started),
        scroll: Cell::new(0),
        opened: now,
    }
}
