use super::init_test::{herdr_env, ok_tools, prepared_repo, run_with};
use super::{parse_start, StartArgs};
use crate::orchestrator::stage::control_file;
use crate::orchestrator::state::acquire_lock;
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
    for (args, file) in [
        (&["stop"][..], "stop"),
        (&["retry", "hx-1"], "retry-hx-1"),
        (&["park", "hx-1"], "park-hx-1"),
        (&["address", "hx-1"], "address-hx-1"),
    ] {
        let (code, out) = run_with(args, repo.path(), Fake::quiet(), &herdr_env);
        assert_eq!(code, 0, "{args:?}: exit {code}, output {out:?}");
        assert!(
            control_file(repo.path(), file).exists(),
            "{args:?} left no control file {file}"
        );
    }
    let (code, _) = run_with(&["retry"], repo.path(), Fake::quiet(), &herdr_env);
    assert_ne!(code, 0, "retry without a Ticket accepted");
}
