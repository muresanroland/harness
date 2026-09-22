//! The Orchestrator: the deterministic process that owns Ticket state, pane
//! placement and Stage transitions (ADR 0001). Ported by harness-kqe.2 to .4.

// ponytail: nothing outside the tests calls into here until the Stage loop
// (harness-kqe.3) and the scheduler (harness-kqe.4) land; drop this line then.
#![allow(dead_code)]

pub(crate) mod herdr;
pub(crate) mod result;
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
mod result_test;
#[cfg(test)]
mod stage_test;
#[cfg(test)]
mod state_test;
#[cfg(test)]
mod trust_test;
