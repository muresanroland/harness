//! What the Orchestrator reads from herdr and how it names things there.

use serde::{Deserialize, Serialize};

use super::stage::Orchestrator;
use crate::tools::RunError;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct TabInfo {
    pub(crate) tab_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct PaneInfo {
    pub(crate) pane_id: String,
    pub(crate) tab_id: String,
}

/// One pane's place in its tab, in terminal cells.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct PaneRect {
    pub(crate) pane_id: String,
    pub(crate) rect: Rect,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Rect {
    pub(crate) width: usize,
    pub(crate) height: usize,
}

/// Every herdr response shape the Orchestrator reads.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct HerdrReply {
    #[serde(default)]
    pub(crate) result: HerdrResult,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct HerdrResult {
    #[serde(default)]
    pub(crate) tab: TabInfo,
    #[serde(default)]
    pub(crate) root_pane: PaneInfo,
    #[serde(default)]
    pub(crate) pane: PaneInfo,
    #[serde(default)]
    pub(crate) tabs: Vec<TabInfo>,
    #[serde(default)]
    pub(crate) panes: Vec<PaneInfo>,
    #[serde(default)]
    pub(crate) layout: Layout,
    #[serde(default)]
    pub(crate) agent: Agent,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Layout {
    #[serde(default)]
    pub(crate) panes: Vec<PaneRect>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Agent {
    #[serde(default, rename = "agent_status")]
    pub(crate) status: String,
}

impl Orchestrator {
    /// A failed command's RunError comes back whole, so a caller can read
    /// herdr's stderr (agent_pane_busy, agent_not_ready, ...).
    pub(crate) fn herdr(&self, args: &[&str]) -> Result<HerdrReply, RunError> {
        let mut argv = vec!["herdr"];
        argv.extend_from_slice(args);
        let out = self.cfg.tools.run(&self.cfg.repo, &argv)?;
        serde_json::from_str(&out).map_err(|err| RunError {
            command: format!("herdr {}", args[0]),
            status: "unreadable reply".to_string(),
            stderr: err.to_string(),
        })
    }

    pub(crate) fn locate(&self, pane_id: &str) -> String {
        let workspace = self.cfg.workspace.as_str();
        let Ok(tabs) = self.herdr(&["tab", "list", "--workspace", workspace]) else {
            return "?".to_string();
        };
        let Ok(panes) = self.herdr(&["pane", "list", "--workspace", workspace]) else {
            return "?".to_string();
        };
        location(&tabs.result.tabs, &panes.result.panes, pane_id)
    }

    /// The herdr lifecycle state of the agent in a pane; None when no agent
    /// lives there any more.
    pub(crate) fn agent_status(&self, pane_id: &str) -> Option<String> {
        self.herdr(&["agent", "get", pane_id])
            .ok()
            .map(|reply| reply.result.agent.status)
    }
}

/// A terminal cell is about twice as tall as it is wide, so a pane of equal
/// rows and columns is a tall sliver on screen, not a square.
const CELL_ASPECT: usize = 2;

/// Picks which pane a new Stage pane is split out of, and which way to cut
/// it: the roomiest pane, along its longer side. Splitting the same pane every
/// time halves it again and again, and four Stage panes in a tab that was only
/// ever cut one way are four slivers too narrow for an agent to draw in.
pub(crate) fn split_target(panes: &[PaneRect]) -> (String, String) {
    let mut best = 0;
    let (mut pane, mut direction) = (String::new(), String::new());
    for p in panes {
        let area = p.rect.width * p.rect.height;
        if area <= best {
            continue;
        }
        best = area;
        pane = p.pane_id.clone();
        direction = if p.rect.width > CELL_ASPECT * p.rect.height {
            "right"
        } else {
            "down"
        }
        .to_string();
    }
    (pane, direction)
}

/// Names a pane as <tab>-<pane> by its position in herdr's tab and pane
/// lists, because herdr ids are opaque and never reused.
pub(crate) fn location(tabs: &[TabInfo], panes: &[PaneInfo], pane_id: &str) -> String {
    let tab_id = panes
        .iter()
        .rev()
        .find(|p| p.pane_id == pane_id)
        .map_or("", |p| p.tab_id.as_str());
    for (t, tab) in tabs.iter().enumerate() {
        if tab.tab_id != tab_id {
            continue;
        }
        let mut n = 0;
        for p in panes {
            if p.tab_id == tab_id {
                n += 1;
            }
            if p.pane_id == pane_id {
                return format!("{}-{n}", t + 1);
            }
        }
    }
    "?".to_string()
}

/// Builds a herdr agent name: [a-z][a-z0-9_-]{0,31}, unique per live agent.
/// The tail of the ticket id is its most distinctive part, so that is what
/// survives truncation.
pub(crate) fn agent_name(ticket: &str, stage: &str) -> String {
    let mut name = String::new();
    let mut in_run = false; // a run of other characters becomes one dash
    for c in format!("{ticket}-{stage}").to_lowercase().chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-';
        if ok {
            name.push(c);
        } else if !in_run {
            name.push('-');
        }
        in_run = !ok;
    }
    if name.len() > 30 {
        name = name[name.len() - 30..].to_string();
    }
    format!("h-{}", name.trim_start_matches(['-', '_']))
}
