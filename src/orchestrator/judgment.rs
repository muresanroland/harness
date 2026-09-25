//! The Judgments (ADR 0004): for a Wake one TypeSafe Choice over its
//! evidence, acted on at or above a confidence floor, its request judge.py's
//! from docs/design/judgment-prototype, keys in its order, without the two
//! diagnostic Nouls it dropped; for a Plan one TypeSafe Noul per criterion
//! over the plan and the Ticket, judge_plan.py's from
//! docs/design/plan-judgment-prototype (harness-cq7). Each floor is
//! config.json's, read at each use.

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::ser::{SerializeMap, Serializer};
use serde::Serialize;
use serde_json::Value;

use super::app::{self, typesafe, Floor};
use super::stage::Orchestrator;
use super::state::TicketState;

/// At or above this confidence the Judgment acts; below it the Wake is a
/// Question.
pub(crate) const WAKE_FLOOR: Floor = Floor {
    key: "wake_floor",
    default: 0.7,
};

/// With covers and in_scope at or above this score for yes, and asks below
/// 0.5, a plan is approved; otherwise it is a Question. The prototype's
/// replay settled it.
pub(crate) const PLAN_FLOOR: Floor = Floor {
    key: "plan_floor",
    default: 0.65,
};

/// How many waits a session may take.
const WAITS: usize = 3;

/// What a Wake can come to, by a Judgment or the user's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    NudgeWriteResult,
    NudgeProceed,
    Retry,
    Park,
    Wait,
}

use Action::*;

impl Action {
    /// In the order judge.py offers them.
    pub(crate) const ALL: [Action; 5] = [NudgeWriteResult, NudgeProceed, Retry, Park, Wait];

    /// The name TypeSafe knows it by.
    pub(crate) fn wire(self) -> &'static str {
        match self {
            NudgeWriteResult => "nudge_write_result",
            NudgeProceed => "nudge_proceed",
            Retry => "retry",
            Park => "park",
            Wait => "wait",
        }
    }

    fn criterion(self) -> &'static str {
        match self {
            NudgeWriteResult => "The session did the Stage's work and stopped, but never wrote the result file, or wrote it without the STATUS line first. One prompt asking for the file is enough.",
            NudgeProceed => "The session stopped to ask a question, offer options, or wait for an approval, and nobody is there to answer. One prompt telling it to decide by itself and carry on is enough.",
            Retry => "The session is unusable and a fresh one would likely succeed: it crashed, lost its connection, hit an API or tool error, ran out of context, or repeats the same step without progress.",
            Park => "A person has to look: the session reports the Ticket is wrong, contradictory or impossible, a tool needs login or setup, or the same failure would meet a fresh session too.",
            Wait => "The session is still working: a tool call is running or output is still arriving, and no prompt or question is waiting at the end.",
        }
    }

    /// How the judged line names it.
    fn short(self) -> &'static str {
        match self {
            NudgeWriteResult => "nudge to write the result",
            NudgeProceed => "nudge to carry on",
            other => other.word(),
        }
    }

    /// How "you answered:" names it.
    pub(crate) fn word(self) -> &'static str {
        match self {
            NudgeWriteResult | NudgeProceed => "nudge",
            Retry => "retry",
            Park => "park",
            Wait => "wait",
        }
    }

    pub(crate) fn is_nudge(self) -> bool {
        matches!(self, NudgeWriteResult | NudgeProceed)
    }

    /// A canned nudge from docs/design/judgment-prototype: its prompt over
    /// the Stage's result file, and the words the line says once it is sent.
    pub(crate) fn nudge(self, file: &Path) -> Option<(String, &'static str)> {
        let (prompt, said) = match self {
            NudgeWriteResult => ("The Orchestrator is waiting for your result file {result_file} and cannot read anything else. Write it now, the first line exactly 'STATUS: done' (or 'STATUS: failed' and why), then stop.", "write the result file"),
            NudgeProceed => ("Nobody is watching this pane and no one will answer. The Ticket is the spec: decide yourself, note the decision in the result file, carry on to the end, then write {result_file} with 'STATUS: done' as its first line.", "carry on, the Ticket is the spec"),
            _ => return None,
        };
        Some((
            prompt.replace("{result_file}", &file.display().to_string()),
            said,
        ))
    }

    /// The Question's option for it, a nudge's prompt in full.
    pub(crate) fn option(self, file: &Path) -> String {
        match (self.nudge(file), self) {
            (Some((prompt, _)), _) => format!("nudge: {prompt}"),
            (None, Retry) => "retry with a fresh session".to_string(),
            (None, Wait) => "wait ten minutes".to_string(),
            (None, other) => other.word().to_string(),
        }
    }
}

/// The actions still unspent: a nudge once per session, a retry once per
/// Stage, three waits per session and those only while the session lives
/// and has not timed out. Park is always there.
pub(crate) fn offered(ts: &TicketState, reason: &str, alive: bool) -> Vec<Action> {
    Action::ALL
        .into_iter()
        .filter(|action| match action {
            NudgeWriteResult | NudgeProceed => !ts.nudged,
            Retry => !ts.retried,
            Wait => ts.waits < WAITS && alive && !reason.starts_with("timed out"),
            Park => true,
        })
        .collect()
}

/// The seam to TypeSafe: posts a request body under the key and returns the
/// reply. An error never carries the key.
pub(crate) trait TypeSafe: Send + Sync {
    fn systemone(&self, key: &str, body: &str) -> Result<Value, String>;
}

/// The real TypeSafe, over ureq.
pub(crate) struct Api;

/// An answer takes a few seconds; a Ticket thread in the call is one that
/// cannot notice /stop-work.
const TIMEOUT: Duration = Duration::from_secs(5);

impl TypeSafe for Api {
    fn systemone(&self, key: &str, body: &str) -> Result<Value, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        let mut resp = agent
            .post("https://api.typesafe.ai/v1/systemone")
            .header("Authorization", &format!("Bearer {key}"))
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(|err| err.to_string())?;
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|err| err.to_string())?;
        serde_json::from_str(&text).map_err(|err| format!("unreadable reply: {err}"))
    }
}

/// judge.py's state for a Wake, its fields in judge.py's order.
#[derive(Debug, Serialize)]
pub(crate) struct WakeState {
    ticket: TicketSpec,
    stage: String,
    round: usize,
    why_woken: String,
    already_nudged: bool,
    already_retried: bool,
    result_file: ResultFile,
    pane_tail: String,
}

#[derive(Debug, Serialize)]
struct TicketSpec {
    id: String,
    title: String,
    spec: String,
}

#[derive(Debug, Serialize)]
struct ResultFile {
    path: String,
    /// The file's text, or "missing".
    content: String,
}

/// The offered actions' criteria, in the order offered.
struct Criteria<'a>(&'a [Action]);

impl Serialize for Criteria<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for action in self.0 {
            map.serialize_entry(action.wire(), action.criterion())?;
        }
        map.end()
    }
}

/// The request judge.py sends: one Choice, `action`, over the offered
/// actions, as JSON text with judge.py's key order.
pub(crate) fn request(state: &WakeState, offered: &[Action]) -> String {
    #[derive(Serialize)]
    struct Request<'a> {
        model: &'a str,
        state: &'a WakeState,
        questions: Questions<'a>,
    }
    #[derive(Serialize)]
    struct Questions<'a> {
        action: Choice<'a>,
    }
    #[derive(Serialize)]
    struct Choice<'a> {
        #[serde(rename = "type")]
        kind: &'a str,
        instructions: &'a str,
        criteria: Criteria<'a>,
    }
    let body = Request {
        model: "jev-latest",
        state,
        questions: Questions {
            action: Choice {
                kind: "choice",
                instructions: "An agent session running one Stage of a Ticket cannot advance by rule (see `why_woken`). From `pane_tail` and `result_file.content`, which action should the Orchestrator take?",
                criteria: Criteria(offered),
            },
        },
    };
    serde_json::to_string(&body).expect("a request of strings serializes")
}

/// The plan Judgment's state: the plan, the Ticket, and the user's feedback
/// on the plan before this revision, null when there was none.
#[derive(Debug, Serialize)]
pub(crate) struct PlanState {
    plan: String,
    ticket: PlanTicket,
    prior_feedback: Option<String>,
}

/// The Ticket as the plan Judgment names it: its description holds the
/// acceptance criteria.
#[derive(Debug, Serialize)]
struct PlanTicket {
    id: String,
    title: String,
    description: String,
}

/// The plan Nouls, judge_plan.py's, in its order: each criterion and its
/// instructions.
const PLAN_NOULS: [(&str, &str); 3] = [
    ("covers", "Does the plan meet every acceptance criterion of the Ticket, with the change and the test each one needs?"),
    ("in_scope", "Does the plan stay within the Ticket? Work toward something the Ticket never asks for, such as another feature, command or rename, is beyond it; the tests, docs, refactors and plumbing the Ticket's own change needs are in scope."),
    ("asks", "Does the plan put a question of its own to the person approving it, one they must answer before the work can start? Questions the planned code will put to its users are not that, and neither are decisions the plan makes itself with the answer it took."),
];

/// The plan Nouls over a plan, in the one request.
pub(crate) fn plan_request(state: &PlanState) -> String {
    #[derive(Serialize)]
    struct Request<'a> {
        model: &'a str,
        state: &'a PlanState,
        questions: Nouls,
    }
    #[derive(Serialize)]
    struct Noul<'a> {
        #[serde(rename = "type")]
        kind: &'a str,
        instructions: &'a str,
    }
    struct Nouls;
    impl Serialize for Nouls {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let mut map = s.serialize_map(Some(PLAN_NOULS.len()))?;
            for (name, instructions) in PLAN_NOULS {
                let noul = Noul {
                    kind: "noul",
                    instructions,
                };
                map.serialize_entry(name, &noul)?;
            }
            map.end()
        }
    }
    let body = Request {
        model: "jev-latest",
        state,
        questions: Nouls,
    };
    serde_json::to_string(&body).expect("a request of strings serializes")
}

/// A Judgment of a plan: each Noul's score for yes, and the floor it was
/// put against, None for one config.json holds that is not a number from 0
/// to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlanJudged {
    pub(crate) covers: f64,
    pub(crate) in_scope: f64,
    pub(crate) asks: f64,
    pub(crate) floor: Option<f64>,
}

impl PlanJudged {
    /// Approved: covers and in_scope at or above the floor, and no question
    /// put to the user, since only they can answer it. Never on a bad floor.
    pub(crate) fn approves(&self) -> bool {
        let clears = |floor| self.covers >= floor && self.in_scope >= floor;
        self.floor.is_some_and(clears) && self.asks < 0.5
    }

    /// The scores as the judged line says them, a yes as its score, marked
    /// when short of the floor, and a no as one minus it: "plan covers the
    /// Ticket 0.62 < 0.65, stays in scope 0.88, asks nothing 0.95".
    pub(crate) fn said(&self) -> String {
        let say = |score: f64, yes: &str, no: &str| match score >= 0.5 {
            true => format!("{yes} {}", self.against(score)),
            false => format!("{no} {:.2}", 1.0 - score),
        };
        let asks = match self.asks >= 0.5 {
            true => format!("asks you a question {:.2}", self.asks),
            false => format!("asks nothing {:.2}", 1.0 - self.asks),
        };
        format!(
            "plan {}, {}, {asks}",
            say(
                self.covers,
                "covers the Ticket",
                "misses an acceptance criterion"
            ),
            say(self.in_scope, "stays in scope", "goes beyond the Ticket"),
        )
    }

    /// Each score for yes, marked as in said(), where said() does not fit.
    pub(crate) fn short(&self) -> String {
        format!(
            "covers {}, in scope {}, asks {:.2}",
            self.against(self.covers),
            self.against(self.in_scope),
            self.asks
        )
    }

    /// A score the floor applies to, "< floor" after it when short.
    fn against(&self, score: f64) -> String {
        match self.floor {
            Some(floor) if score < floor => format!("{score:.2} < {floor:.2}"),
            _ => format!("{score:.2}"),
        }
    }
}

/// A Judgment of a Wake: the chosen action, how sure, and every offered
/// action's score, highest first.
#[derive(Clone, Debug)]
pub(crate) struct Judged {
    pub(crate) choice: Action,
    pub(crate) confidence: f64,
    pub(crate) scores: Vec<(Action, f64)>,
}

impl Judged {
    /// The scores as the judged line says them: "retry 0.98, park 0.02".
    pub(crate) fn said(&self) -> String {
        self.scores
            .iter()
            .map(|(action, score)| format!("{} {score:.2}", action.short()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The reply's `action` answer, scored over the offered actions; None
/// unless it chose one of them with a confidence between 0 and 1.
fn parse(reply: &Value, offered: &[Action]) -> Option<Judged> {
    let answer = &reply["answers"]["action"];
    let named = |wire: &str| offered.iter().copied().find(|a| a.wire() == wire);
    let choice = named(answer["choice"].as_str()?)?;
    let confidence = answer["confidence"]
        .as_f64()
        .filter(|c| (0.0..=1.0).contains(c))?;
    let mut scores: Vec<(Action, f64)> = answer["probabilities"]
        .as_object()?
        .iter()
        .filter_map(|(wire, score)| Some((named(wire)?, score.as_f64()?)))
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    Some(Judged {
        choice,
        confidence,
        scores,
    })
}

impl Orchestrator {
    /// The TypeSafe key, "" when there is none or TypeSafe is off.
    pub(crate) fn typesafe_key(&self) -> &str {
        if typesafe(&self.cfg.repo) {
            &self.cfg.api_key
        } else {
            ""
        }
    }

    /// Puts a Wake to TypeSafe. None without a key, on any error, or for a
    /// reply that chose nothing offered: the Wake is then a Question.
    pub(crate) fn judge(
        &self,
        ticket: &str,
        ts: &TicketState,
        reason: &str,
        file: &Path,
        tail: &str,
        offered: &[Action],
    ) -> Option<Judged> {
        let key = self.typesafe_key();
        if key.is_empty() {
            return None;
        }
        let body = request(&self.wake_state(ticket, ts, reason, file, tail), offered);
        self.ask_typesafe(ticket, key, &body, |reply| {
            parse(reply, offered).ok_or("no choice in the reply")
        })
    }

    /// Puts a plan to TypeSafe: each Noul's score for yes, with the plan
    /// floor as config.json has it now. None without a key, on any error, or
    /// for a reply short of a score: the plan is then a Question.
    pub(crate) fn judge_plan(&self, ticket: &str, plan: &str) -> Option<PlanJudged> {
        let key = self.typesafe_key();
        if key.is_empty() {
            return None;
        }
        let body = plan_request(&self.plan_state(ticket, plan));
        let (covers, in_scope, asks) = self.ask_typesafe(ticket, key, &body, |reply| {
            let score = |noul: &str| {
                reply["answers"][noul]["noul"]
                    .as_f64()
                    .filter(|score| (0.0..=1.0).contains(score))
            };
            Ok((
                score("covers").ok_or("no covers in the reply")?,
                score("in_scope").ok_or("no in_scope in the reply")?,
                score("asks").ok_or("no asks in the reply")?,
            ))
        })?;
        Some(PlanJudged {
            covers,
            in_scope,
            asks,
            floor: self.floor(ticket, &PLAN_FLOOR),
        })
    }

    /// The plan Judgment's state: the plan, the Ticket as bd shows it, and
    /// the feedback kept from the plan before.
    pub(crate) fn plan_state(&self, ticket: &str, plan: &str) -> PlanState {
        let feedback = self.ticket(ticket).feedback;
        let spec = self.ticket_spec(ticket);
        PlanState {
            plan: plan.to_string(),
            ticket: PlanTicket {
                id: spec.id,
                title: spec.title,
                description: spec.spec,
            },
            prior_feedback: (!feedback.is_empty()).then_some(feedback),
        }
    }

    /// A floor as config.json has it now; None, and the log says why, for
    /// one that is not a number from 0 to 1: the Judgment is then not acted
    /// on, and its Question asks.
    pub(crate) fn floor(&self, ticket: &str, floor: &Floor) -> Option<f64> {
        app::floor(&self.cfg.repo, floor)
            .map_err(|err| self.log(ticket, &format!("{err}: the Judgment is not acted on")))
            .ok()
    }

    /// Posts a request under the key and reads the reply; an error, or a
    /// reply `read` cannot use, is logged without the key.
    fn ask_typesafe<T>(
        &self,
        ticket: &str,
        key: &str,
        body: &str,
        read: impl FnOnce(&Value) -> Result<T, &'static str>,
    ) -> Option<T> {
        let reply = self.cfg.typesafe.systemone(key, body);
        match reply.and_then(|reply| read(&reply).map_err(str::to_string)) {
            Ok(answer) => Some(answer),
            Err(err) => {
                self.log(ticket, &format!("no Judgment: {}", err.replace(key, "***")));
                None
            }
        }
    }

    /// The Ticket as bd shows it: its description with the acceptance
    /// criteria under it.
    fn ticket_spec(&self, ticket: &str) -> TicketSpec {
        let issue = self
            .cfg
            .tools
            .run(&self.cfg.repo, &["bd", "show", ticket, "--json"])
            .ok()
            .and_then(|out| serde_json::from_str::<Vec<Value>>(&out).ok())
            .and_then(|issues| issues.into_iter().next())
            .unwrap_or_default();
        let field = |name: &str| issue[name].as_str().unwrap_or_default().to_string();
        let spec = match field("acceptance_criteria") {
            criteria if criteria.is_empty() => field("description"),
            criteria => format!(
                "{}\n\nAcceptance criteria:\n{criteria}",
                field("description")
            ),
        };
        TicketSpec {
            id: ticket.to_string(),
            title: field("title"),
            spec,
        }
    }

    /// judge.py's state for a Wake: the Ticket as bd shows it, where the
    /// Stage stands, the reason verbatim, the result file and the pane tail.
    pub(crate) fn wake_state(
        &self,
        ticket: &str,
        ts: &TicketState,
        reason: &str,
        file: &Path,
        tail: &str,
    ) -> WakeState {
        WakeState {
            ticket: self.ticket_spec(ticket),
            stage: ts.stage.clone(),
            round: ts.round,
            why_woken: reason.to_string(),
            already_nudged: ts.nudged,
            already_retried: ts.retried,
            result_file: ResultFile {
                path: file
                    .strip_prefix(&self.cfg.repo)
                    .unwrap_or(file)
                    .display()
                    .to_string(),
                content: fs::read_to_string(file).unwrap_or_else(|_| "missing".to_string()),
            },
            pane_tail: tail.to_string(),
        }
    }
}

/// The test double for TypeSafe: records every request body and answers it
/// from `answer`.
#[cfg(test)]
pub(crate) mod fake {
    use super::TypeSafe;
    use serde_json::Value;
    use std::sync::{Arc, Mutex};

    type Answer = dyn Fn(&Value) -> Result<Value, String> + Send + Sync;

    pub(crate) struct Fake {
        requests: Mutex<Vec<Value>>,
        answer: Box<Answer>,
    }

    impl Fake {
        pub(crate) fn new(
            answer: impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
        ) -> Arc<Self> {
            Arc::new(Fake {
                requests: Mutex::new(Vec::new()),
                answer: Box::new(answer),
            })
        }

        /// A TypeSafe that is never reachable: every Wake is a Question.
        pub(crate) fn down() -> Arc<Self> {
            Fake::new(|_| Err("TypeSafe is down".to_string()))
        }

        pub(crate) fn requests(&self) -> Vec<Value> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl TypeSafe for Fake {
        fn systemone(&self, _key: &str, body: &str) -> Result<Value, String> {
            let body: Value = serde_json::from_str(body).unwrap();
            self.requests.lock().unwrap().push(body.clone());
            (self.answer)(&body)
        }
    }
}
