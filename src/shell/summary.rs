//! The Epic summary: each Ticket's PR, Rounds and Findings, and the Parked
//! ones with their reasons, built fresh from bd, the State and each
//! Ticket's Run directory evidence (harness-0sx.5, layout B). The pager
//! that shows it is draw/pager.rs.

use std::cell::Cell;
use std::path::Path;

use super::suffix;
use crate::orchestrator::pipeline::MAX_ROUNDS;
use crate::orchestrator::result::{read_stage_result, ResultRequirements, StageResult};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::{result_name, run_dir, DEBATE, FIX};
use crate::orchestrator::state::{State, STATUS_MERGED, STATUS_PARKED};

pub(crate) struct Summary {
    pub(crate) epic: String,
    pub(crate) title: String,
    /// In suffix order.
    pub(crate) tickets: Vec<Done>,
    /// The first body row shown; the draw keeps it inside.
    pub(crate) scroll: Cell<usize>,
    /// When it was built, for its count of the lines on RECENT since.
    pub(crate) opened: chrono::DateTime<chrono::Local>,
}

/// One Ticket of the Epic summary.
pub(crate) struct Done {
    pub(crate) id: String,
    pub(crate) title: String,
    /// The Fix result's PR url; empty before one opened.
    pub(crate) pr: String,
    /// Closed in bd, or merged by the run.
    pub(crate) merged: bool,
    /// One per Verdict.
    pub(crate) rounds: usize,
    pub(crate) fixed: usize,
    pub(crate) skipped: Vec<String>,
    /// The cap's last fix items, which no Review re-checked: listed on the PR.
    pub(crate) left: Vec<String>,
    /// Why it is Parked.
    pub(crate) parked: Option<String>,
}

impl Summary {
    /// The Epic's summary from bd's issues, the State and the Run
    /// directories; an unknown Epic, or one none of whose Tickets has run,
    /// is the error.
    pub(crate) fn build(
        repo: &Path,
        issues: &[BdIssue],
        state: &State,
        epic: &str,
    ) -> Result<Summary, String> {
        let title = issues
            .iter()
            .find(|i| i.id == epic)
            .ok_or(format!("no Epic {epic} in bd"))?
            .title
            .clone();
        let mut children: Vec<&BdIssue> = issues
            .iter()
            .filter(|i| i.parent == epic && i.issue_type != "epic")
            .collect();
        children.sort_by_key(|i| suffix(&i.id).parse::<usize>().unwrap_or(usize::MAX));
        let ran = children
            .iter()
            .any(|t| run_dir(repo, &t.id).exists() || state.tickets.contains_key(&t.id));
        if !ran {
            return Err(format!(
                "no evidence for {epic}: none of its Tickets has run"
            ));
        }
        Ok(Summary {
            epic: epic.to_string(),
            title,
            tickets: children.into_iter().map(|t| done(repo, state, t)).collect(),
            scroll: Cell::new(0),
            opened: chrono::Local::now(),
        })
    }
}

/// One Ticket from its Run directory: a Round per verdict-N.md, the PR from
/// the last Fix result with one, merged and parked from bd and the State.
fn done(repo: &Path, state: &State, t: &BdIssue) -> Done {
    let dir = run_dir(repo, &t.id);
    let read = |name: String| read_stage_result(&dir.join(name), ResultRequirements::default()).0;
    let verdicts: Vec<StageResult> = (1..)
        .take_while(|n| dir.join(result_name(&DEBATE, *n)).exists())
        .map(|n| read(result_name(&DEBATE, n)))
        .collect();
    let rounds = verdicts.len();
    let left = match verdicts.last() {
        Some(last) if rounds == MAX_ROUNDS => last.fixes.clone(),
        _ => Vec::new(),
    };
    let fixes: usize = verdicts.iter().map(|v| v.fixes.len()).sum();
    let pr = (1..=rounds)
        .rev()
        .map(|n| read(result_name(&FIX, n)).pr)
        .find(|pr| !pr.is_empty())
        .unwrap_or_default();
    let ts = state.tickets.get(&t.id);
    Done {
        id: t.id.clone(),
        title: t.title.clone(),
        pr,
        merged: t.status == "closed" || ts.is_some_and(|ts| ts.status == STATUS_MERGED),
        rounds,
        fixed: fixes - left.len(),
        skipped: verdicts
            .iter()
            .flat_map(|v| &v.skips)
            .map(|l| finding(l))
            .collect(),
        left: left.iter().map(|l| finding(l)).collect(),
        parked: ts
            .filter(|ts| ts.status == STATUS_PARKED)
            .map(|ts| ts.reason.clone()),
    }
}

/// A Verdict item as the summary shows it: its severity, place and problem,
/// without the mark, the reason or how it was settled.
fn finding(line: &str) -> String {
    let item = line.split_once("] ").map_or(line, |(_, rest)| rest);
    item.split(" | reason:").next().unwrap_or(item).to_string()
}
