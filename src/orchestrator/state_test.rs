use super::state::{acquire_lock, load_state, lock_holder, State, TicketState};
use crate::tempdir::TempDir;
use std::fs;

/// state.json as the Go binary saved it after a real run on test-harness-repo.
const GO_STATE: &str = include_str!("testdata/state.json");
/// A run in flight, written by Go's encoder from state.go's structs: every
/// field of TicketState set somewhere.
const GO_RUNNING_STATE: &str = include_str!("testdata/state-running.json");

fn repo_with(state: &str) -> TempDir {
    let repo = TempDir::new();
    fs::create_dir_all(repo.path().join(".harness")).unwrap();
    fs::write(repo.path().join(".harness/state.json"), state).unwrap();
    repo
}

#[test]
fn go_written_state_loads_intact_and_round_trips() {
    let repo = TempDir::new();
    fs::create_dir_all(repo.path().join(".harness")).unwrap();
    fs::write(repo.path().join(".harness/state.json"), GO_STATE).unwrap();
    let state = load_state(repo.path()).unwrap();
    assert_eq!(state.epic, "test-harness-repo-6fs");
    let ids: Vec<&String> = state.tickets.keys().collect();
    assert_eq!(
        ids,
        [
            "test-harness-repo-6fs.1",
            "test-harness-repo-6fs.2",
            "test-harness-repo-6fs.3",
            "test-harness-repo-6fs.4"
        ]
    );
    assert_eq!(
        state.tickets["test-harness-repo-6fs.1"],
        TicketState {
            status: "merged".to_string(),
            stage: "fix".to_string(),
            round: 1,
            pr: "https://github.com/muresanroland/test-harness-repo/pull/2".to_string(),
            ..Default::default()
        }
    );
    let last = &state.tickets["test-harness-repo-6fs.4"];
    assert!(
        last.status == "pr-open"
            && last.round == 2
            && last.pr == "https://github.com/muresanroland/test-harness-repo/pull/5",
        "{last:?}"
    );

    // Re-saved, it is the same file byte for byte: the same field names, order
    // and omissions as Go's encoding, so either binary can pick up a run.
    state.save(repo.path()).unwrap();
    assert_eq!(
        fs::read_to_string(repo.path().join(".harness/state.json")).unwrap(),
        GO_STATE
    );
    assert_eq!(load_state(repo.path()).unwrap(), state);
    assert!(
        !repo.path().join(".harness/state.json.tmp").exists(),
        "the temp file outlived the rename"
    );
}

#[test]
fn go_written_running_state_round_trips_every_field() {
    let repo = repo_with(GO_RUNNING_STATE);
    let state = load_state(repo.path()).unwrap();
    assert_eq!(state.epic, "hx");
    assert_eq!(
        state.tickets["hx-1"],
        TicketState {
            status: "running".to_string(),
            stage: "review".to_string(),
            round: 2,
            tab: "w1:t2".to_string(),
            panes: [("implement", "w1:p3"), ("review", "w1:p5")]
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .into(),
            retried: true,
            ..Default::default()
        }
    );
    let parked = &state.tickets["hx-2"];
    assert!(
        parked.status == "parked"
            && parked.tab == "w1:t3"
            && parked.panes["fix"] == "w1:p7"
            && parked.reason == "fix reported STATUS: failed again after a retry"
            && !parked.retried,
        "{parked:?}"
    );
    let open = &state.tickets["hx-3"];
    assert!(
        open.status == "pr-open" && open.pr == "https://github.com/o/r/pull/9" && open.conflict,
        "{open:?}"
    );
    state.save(repo.path()).unwrap();
    assert_eq!(
        fs::read_to_string(repo.path().join(".harness/state.json")).unwrap(),
        GO_RUNNING_STATE
    );
}

#[test]
fn every_field_survives_a_save_and_a_missing_file_is_an_empty_state() {
    let repo = TempDir::new();
    assert_eq!(load_state(repo.path()).unwrap(), State::default());
    // Go's nil map: a null tickets field is no Tickets.
    assert_eq!(
        load_state(repo_with("{\"tickets\": null}").path()).unwrap(),
        State::default()
    );
    let mut state = State::default();
    state.tickets.insert(
        "hx-1".to_string(),
        TicketState {
            status: "parked".to_string(),
            stage: "review".to_string(),
            round: 2,
            tab: "w1:t1".to_string(),
            panes: [("review".to_string(), "w1:p2".to_string())].into(),
            pr: String::new(),
            reason: "review went idle".to_string(),
            retried: true,
            nudged: true,
            conflict: true,
        },
    );
    state.save(repo.path()).unwrap();
    let raw = fs::read_to_string(repo.path().join(".harness/state.json")).unwrap();
    for field in [
        "\"tab\"",
        "\"panes\"",
        "\"reason\"",
        "\"retried\"",
        "\"nudged\"",
        "\"conflict_reported\"",
    ] {
        assert!(raw.contains(field), "saved state lacks {field}:\n{raw}");
    }
    assert!(
        !raw.contains("\"pr\""),
        "an empty pr is not omitted:\n{raw}"
    );
    assert!(
        !raw.contains("\"epic\""),
        "an empty epic is not omitted:\n{raw}"
    );
    assert_eq!(load_state(repo.path()).unwrap(), state);
}
#[test]
fn a_second_lock_on_the_same_repo_fails_and_names_the_holder() {
    let repo = TempDir::new();
    let pid = std::process::id();
    assert_eq!(lock_holder(repo.path()), 0, "nothing holds a fresh repo");
    let lock = acquire_lock(repo.path()).unwrap();
    assert_eq!(lock_holder(repo.path()), pid);
    // A second open of the same lock file is a second flock: it must fail
    // even from the process that holds it.
    let err = acquire_lock(repo.path()).unwrap_err().to_string();
    assert_eq!(
        err,
        format!("a run is live in this repo (pid {pid}); /stop-work there ends it")
    );
    drop(lock);
    assert_eq!(
        lock_holder(repo.path()),
        0,
        "released lock still reads as held"
    );

    // A lock file left by a killed Orchestrator holds no flock: stale, taken over.
    fs::write(repo.path().join(".harness/lock"), "999999").unwrap();
    assert_eq!(
        lock_holder(repo.path()),
        0,
        "a stale lock file reads as held"
    );
    let lock = acquire_lock(repo.path()).unwrap();
    assert_eq!(lock_holder(repo.path()), pid);
    drop(lock);
}
