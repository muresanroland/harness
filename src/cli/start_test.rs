use super::init_test::{herdr_env, ok_tools, prepared_repo, run_with};
use super::{parse_start, StartArgs};
use crate::orchestrator::state::{acquire_lock, lock_holder};
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;

fn parse(args: &[&str]) -> Result<StartArgs, String> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    parse_start(&args)
}

#[test]
fn start_arguments_parse_in_any_order() {
    for args in [
        &["hx", "--max", "2"][..],
        &["--max", "2", "hx"],
        &["hx", "--max=2"],
        &["-max", "2", "hx"],
    ] {
        let got = parse(args);
        assert_eq!(
            got,
            Ok(StartArgs {
                epic: "hx".to_string(),
                ticket: String::new(),
                max: 2
            }),
            "parse_start({args:?})"
        );
    }
    let got = parse(&["--ticket", "hx-1"]).unwrap();
    assert!(got.ticket == "hx-1" && got.max == 3, "--ticket: {got:?}");
    // A run stays in the pane it was started from: --detach is gone (ADR 0003).
    for bad in [
        &[][..],
        &["hx", "--ticket", "hx-1"],
        &["hx", "extra"],
        &["hx", "--max", "0"],
        &["hx", "--detach"],
    ] {
        assert!(parse(bad).is_err(), "parse_start({bad:?}) accepted");
    }
    // A bad flag is named before the usage, as Go's flag package did.
    let (code, out) = run_with(
        &["start", "hx", "--max", "lots"],
        prepared_repo().path(),
        ok_tools(),
        &herdr_env,
    );
    assert!(
        code == 2 && out.starts_with("invalid value \"lots\" for flag -max\nusage:"),
        "bad --max: exit {code}, output {out:?}"
    );
}

#[test]
fn second_start_in_the_same_repo_refuses() {
    let repo = prepared_repo();
    let _lock = acquire_lock(repo.path()).unwrap(); // a live Orchestrator: this process
    let (code, out) = run_with(&["start", "hx"], repo.path(), ok_tools(), &herdr_env);
    assert!(
        code != 0 && out.contains("already running"),
        "second start: exit {code}, output {out:?}"
    );
}

#[test]
fn control_commands_reach_only_a_running_orchestrator() {
    let repo = TempDir::new();
    let (code, out) = run_with(&["stop"], repo.path(), Fake::quiet(), &herdr_env);
    assert!(
        code != 0 && out.contains("no Orchestrator is running"),
        "stop with nothing running: exit {code}, {out:?}"
    );

    let _lock = acquire_lock(repo.path()).unwrap();
    // ponytail: with an Orchestrator running, Go left a control file (stop,
    // retry-hx-1, ...) for it to drain; that wiring is harness-kqe.4's, so for
    // now the commands get past the lock check and stop at "not ported yet".
    for args in [
        &["stop"][..],
        &["retry", "hx-1"],
        &["park", "hx-1"],
        &["address", "hx-1"],
    ] {
        let (code, out) = run_with(args, repo.path(), Fake::quiet(), &herdr_env);
        assert!(
            !out.contains("no Orchestrator is running")
                && out.ends_with(&format!("harness {}: not ported yet\n", args[0])),
            "{args:?}: exit {code}, output {out:?}"
        );
    }
    let (code, _) = run_with(&["retry"], repo.path(), Fake::quiet(), &herdr_env);
    assert_ne!(code, 0, "retry without a Ticket accepted");
}

// ponytail: 'start' holds the lock and then stops, until harness-kqe.3 and .4
// bring the Pipeline and the scheduler it would run.
#[test]
fn start_is_not_ported_yet() {
    let repo = prepared_repo();
    let (code, out) = run_with(&["start", "hx"], repo.path(), ok_tools(), &herdr_env);
    assert!(
        code == 2 && out.ends_with("harness start: not ported yet\n"),
        "start: exit {code}, output {out:?}"
    );
    assert_eq!(lock_holder(repo.path()), 0, "start left the lock behind");
}
