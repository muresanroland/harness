//! The Orchestrator: the deterministic process that owns Ticket state, pane
//! placement and Stage transitions (ADR 0001). Ported by harness-kqe.2 to .4.

pub(crate) mod herdr;
pub(crate) mod judgment;
pub(crate) mod pipeline;
pub(crate) mod result;
pub(crate) mod scheduler;
pub(crate) mod stage;
pub(crate) mod state;
pub(crate) mod trust;

/// The port of the Go tests' writeFile: parents are created as needed.
#[cfg(test)]
pub(crate) fn write_file(path: &std::path::Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

#[cfg(test)]
mod async_result_test;
#[cfg(test)]
mod judgment_test;
#[cfg(test)]
mod panes_test;
#[cfg(test)]
mod pipeline_test;
#[cfg(test)]
mod reliability_test;
#[cfg(test)]
mod result_completion_test;
#[cfg(test)]
mod result_test;
#[cfg(test)]
mod retry_result_test;
#[cfg(test)]
mod scheduler_test;
#[cfg(test)]
mod stage_test;
#[cfg(test)]
mod state_test;
#[cfg(test)]
mod trust_test;
#[cfg(test)]
mod unattended_test;
#[cfg(test)]
mod wake_test;
#[cfg(test)]
pub(crate) mod world;
