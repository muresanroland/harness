//! The Wake Judgment: one TypeSafe Choice over a Wake's evidence, acted on at
//! or above a confidence floor (ADR 0004). The request is judge.py's, from
//! docs/design/judgment-prototype, without the two diagnostic Nouls it
//! dropped.

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use super::stage::Orchestrator;
use super::state::TicketState;

/// At or above this confidence the Judgment acts; below it the Wake is a
/// Question.
pub(crate) const FLOOR: f64 = 0.7;

/// How many waits a Stage may take.
const WAITS: usize = 3;

/// The actions a Wake Judgment chooses from, each with its criterion, in the
/// order judge.py offers them.
const ACTIONS: [(&str, &str); 5] = [
    ("nudge_write_result", "The session did the Stage's work and stopped, but never wrote the result file, or wrote it without the STATUS line first. One prompt asking for the file is enough."),
    ("nudge_proceed", "The session stopped to ask a question, offer options, or wait for an approval, and nobody is there to answer. One prompt telling it to decide by itself and carry on is enough."),
    ("retry", "The session is unusable and a fresh one would likely succeed: it crashed, lost its connection, hit an API or tool error, ran out of context, or repeats the same step without progress."),
    ("park", "A person has to look: the session reports the Ticket is wrong, contradictory or impossible, a tool needs login or setup, or the same failure would meet a fresh session too."),
    ("wait", "The session is still working: a tool call is running or output is still arriving, and no prompt or question is waiting at the end."),
];

const INSTRUCTIONS: &str = "An agent session running one Stage of a Ticket cannot advance by rule (see `why_woken`). From `pane_tail` and `result_file.content`, which action should the Orchestrator take?";

/// The seam to TypeSafe: posts a request body under the key and returns the
/// reply. An error never carries the key.
pub(crate) trait TypeSafe: Send + Sync {
    fn systemone(&self, key: &str, body: &Value) -> Result<Value, String>;
}

/// The real TypeSafe, over ureq.
pub(crate) struct Api;

/// Long enough for one answer (a few seconds), short enough that a Ticket
/// thread stuck in the call still notices /stop-work soon after.
const TIMEOUT: Duration = Duration::from_secs(20);

impl TypeSafe for Api {
    fn systemone(&self, key: &str, body: &Value) -> Result<Value, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        let mut resp = agent
            .post("https://api.typesafe.ai/v1/systemone")
            .header("Authorization", &format!("Bearer {key}"))
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(|err| err.to_string())?;
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|err| err.to_string())?;
        serde_json::from_str(&text).map_err(|err| format!("unreadable reply: {err}"))
    }
}

/// The actions still unspent: a nudge once per session, a retry once per
/// Stage, three waits, and no wait after a timeout. Park is always there.
pub(crate) fn offered(ts: &TicketState, reason: &str, waits: usize) -> Vec<&'static str> {
    ACTIONS
        .iter()
        .map(|(action, _)| *action)
        .filter(|action| match *action {
            "nudge_write_result" | "nudge_proceed" => !ts.nudged,
            "retry" => !ts.retried,
            "wait" => waits < WAITS && !reason.starts_with("timed out"),
            _ => true,
        })
        .collect()
}

/// The request judge.py sends: one Choice, `action`, over the offered actions.
pub(crate) fn request(state: Value, offered: &[&str]) -> Value {
    let criteria: serde_json::Map<String, Value> = ACTIONS
        .iter()
        .filter(|(action, _)| offered.contains(action))
        .map(|(action, criterion)| (action.to_string(), json!(criterion)))
        .collect();
    json!({
        "model": "jev-latest",
        "state": state,
        "questions": {
            "action": { "type": "choice", "instructions": INSTRUCTIONS, "criteria": criteria },
        },
    })
}

/// A Judgment of a Wake: the chosen action, how sure, and every offered
/// action's score, highest first.
#[derive(Debug)]
pub(crate) struct Judged {
    pub(crate) choice: String,
    pub(crate) confidence: f64,
    pub(crate) scores: Vec<(String, f64)>,
}

impl Judged {
    /// The scores as the judged line says them: "retry 0.98, park 0.02".
    pub(crate) fn scores(&self) -> String {
        self.scores
            .iter()
            .map(|(action, score)| format!("{action} {score:.2}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The reply's `action` answer, scored over the offered actions; None
/// unless it chose one of them.
fn parse(reply: &Value, offered: &[&str]) -> Option<Judged> {
    let action = &reply["answers"]["action"];
    let choice = action["choice"].as_str().filter(|c| offered.contains(c))?;
    let mut scores: Vec<(String, f64)> = action["probabilities"]
        .as_object()?
        .iter()
        .filter(|(action, _)| offered.contains(&action.as_str()))
        .filter_map(|(action, score)| Some((action.clone(), score.as_f64()?)))
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    Some(Judged {
        choice: choice.to_string(),
        confidence: action["confidence"].as_f64()?,
        scores,
    })
}

impl Orchestrator {
    /// Puts a Wake to TypeSafe. None without a key, on any error, or for a
    /// reply that chose nothing offered: the Wake is then a Question.
    pub(crate) fn judge(
        &self,
        ticket: &str,
        ts: &TicketState,
        reason: &str,
        file: &Path,
        tail: &str,
        offered: &[&str],
    ) -> Option<Judged> {
        let key = self.cfg.api_key.as_str();
        if key.is_empty() {
            return None;
        }
        let body = request(self.wake_state(ticket, ts, reason, file, tail), offered);
        let judged = self.cfg.typesafe.systemone(key, &body).and_then(|reply| {
            parse(&reply, offered).ok_or_else(|| "no choice in the reply".to_string())
        });
        match judged {
            Ok(judged) => Some(judged),
            Err(err) => {
                self.log(ticket, &format!("no Judgment: {}", err.replace(key, "***")));
                None
            }
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
    ) -> Value {
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
        json!({
            "ticket": { "id": ticket, "title": field("title"), "spec": spec },
            "stage": ts.stage,
            "round": ts.round,
            "why_woken": reason,
            "already_nudged": ts.nudged,
            "already_retried": ts.retried,
            "result_file": {
                "path": file.strip_prefix(&self.cfg.repo).unwrap_or(file).display().to_string(),
                "content": fs::read_to_string(file).unwrap_or_else(|_| "missing".to_string()),
            },
            "pane_tail": tail,
        })
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
        fn systemone(&self, _key: &str, body: &Value) -> Result<Value, String> {
            self.requests.lock().unwrap().push(body.clone());
            (self.answer)(body)
        }
    }
}
