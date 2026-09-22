//! The harness command line.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use crate::orchestrator::state::{acquire_lock, lock_holder, print_status};
use crate::setup;
use crate::tools::Tools;

const USAGE: &str = "usage: harness <command>

  init [--force]              install the shipped skills and preflight the Target repo
  start <epic> [--max N]      run an Epic's Tickets through the Pipeline, here
  start --ticket <id>         run one Ticket through the Pipeline, here
  status                      print the state of the run
  retry <ticket>              restart a Ticket's current Stage with a fresh session
  park <ticket>               park a Ticket
  address <ticket>            act on a Ticket's PR review comments and conflicts
  stop                        stop scheduling and exit, leaving live panes alone
";

/// Runs one harness command inside the Target repo and returns the exit code.
/// `input` answers init's question; None is the terminal's stdin.
pub fn run(
    args: &[String],
    out: &mut dyn Write,
    input: Option<&mut dyn Read>,
    repo: &Path,
    tools: Arc<dyn Tools>,
    env: &dyn Fn(&str) -> String,
) -> i32 {
    let Some(name) = args.first() else {
        let _ = out.write_all(USAGE.as_bytes());
        return 2;
    };
    match name.as_str() {
        "init" => {
            let force = args.get(1).is_some_and(|a| a == "--force");
            if let Err(err) = setup::install_skills(repo, force, out, input) {
                let _ = writeln!(out, "init: {err}");
                return 1;
            }
            setup::report_missing(out, &setup::preflight(repo, &*tools, env))
        }
        "start" => start(&args[1..], out, repo, &*tools, env),
        "status" => print_status(out, repo),
        "stop" | "retry" | "park" | "address" => command(args, out, repo),
        _ => {
            let _ = out.write_all(USAGE.as_bytes());
            2
        }
    }
}

// ponytail: the Orchestrator lands with harness-kqe.2 to .4; until then every
// command that needs it stops here.
fn not_ported(name: &str, out: &mut dyn Write) -> i32 {
    let _ = writeln!(out, "harness {name}: not ported yet");
    2
}

/// The control commands for a running Orchestrator, which owns the state
/// file and is the only process that acts on them.
fn command(args: &[String], out: &mut dyn Write, repo: &Path) -> i32 {
    let name = &args[0];
    if name != "stop" && args.len() != 2 {
        let _ = writeln!(out, "usage: harness {name} <ticket>");
        return 2;
    }
    if lock_holder(repo) == 0 {
        let _ = writeln!(
            out,
            "no Orchestrator is running in this repo; 'harness start <epic>' starts or resumes one"
        );
        return 1;
    }
    // ponytail: the control file (stop, retry-<ticket>, ...) that the running
    // Orchestrator drains is harness-kqe.4's; until then the command stops here.
    not_ported(name, out)
}

#[derive(Debug, Default, PartialEq)]
struct StartArgs {
    epic: String,
    ticket: String,
    max: usize,
}

/// Accepts the Epic and the flags in any order.
fn parse_start(args: &[String]) -> Result<StartArgs, String> {
    let mut parsed = StartArgs {
        max: 3,
        ..Default::default()
    };
    let mut positional = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let Some(flag) = arg.strip_prefix('-') else {
            positional.push(arg.as_str());
            continue;
        };
        let flag = flag.strip_prefix('-').unwrap_or(flag);
        let (name, value) = match flag.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (flag, None),
        };
        let value = match value.or_else(|| args.next().map(String::as_str)) {
            Some(value) => value,
            None => return Err(format!("flag needs an argument: -{name}")),
        };
        match name {
            "ticket" => parsed.ticket = value.to_string(),
            "max" => {
                parsed.max = value
                    .parse()
                    .map_err(|_| format!("invalid value {value:?} for flag -max"))?
            }
            _ => return Err(format!("flag provided but not defined: -{name}")),
        }
    }
    if positional.len() == 1 {
        parsed.epic = positional[0].to_string();
    }
    if positional.len() > 1 || parsed.epic.is_empty() == parsed.ticket.is_empty() || parsed.max < 1
    {
        return Err("want one Epic or --ticket <id>, and --max of at least 1".to_string());
    }
    Ok(parsed)
}

/// Preflights, then runs the Orchestrator in this process.
fn start(
    args: &[String],
    out: &mut dyn Write,
    repo: &Path,
    tools: &dyn Tools,
    env: &dyn Fn(&str) -> String,
) -> i32 {
    if let Err(err) = parse_start(args) {
        let _ = writeln!(out, "{err}");
        let _ = out.write_all(USAGE.as_bytes());
        return 2;
    }
    let code = setup::report_missing(out, &setup::preflight(repo, tools, env));
    if code != 0 {
        return code;
    }
    if let Err(err) = setup::ignore_run_dir(repo) {
        let _ = writeln!(out, "start: {err}");
        return 1;
    }
    let _lock = match acquire_lock(repo) {
        Ok(lock) => lock,
        Err(err) => {
            let _ = writeln!(out, "start: {err}");
            return 1;
        }
    };
    not_ported("start", out)
}

#[cfg(test)]
mod cli_test;
#[cfg(test)]
mod init_test;
#[cfg(test)]
mod start_test;
