//! The harness command line.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::orchestrator::stage::{control_file, Config, Orchestrator};
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
        "start" => start(&args[1..], out, repo, tools, env),
        "status" => print_status(out, repo),
        "stop" | "retry" | "park" | "address" => command(args, out, repo),
        _ => {
            let _ = out.write_all(USAGE.as_bytes());
            2
        }
    }
}

/// Leaves a control file for the running Orchestrator, which owns the state
/// file and is the only process that acts on it.
fn command(args: &[String], out: &mut dyn Write, repo: &Path) -> i32 {
    let mut name = args[0].clone();
    if name != "stop" {
        if args.len() != 2 {
            let _ = writeln!(out, "usage: harness {name} <ticket>");
            return 2;
        }
        name = format!("{name}-{}", args[1]);
    }
    if lock_holder(repo) == 0 {
        let _ = writeln!(
            out,
            "no Orchestrator is running in this repo; 'harness start <epic>' starts or resumes one"
        );
        return 1;
    }
    let path = control_file(repo, &name);
    if let Err(err) = fs::create_dir_all(path.parent().unwrap()).and_then(|()| fs::write(&path, ""))
    {
        let _ = writeln!(out, "{err}");
        return 1;
    }
    let _ = writeln!(out, "sent: {name}");
    0
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

/// Every event line the run says is shown on the terminal and kept in the
/// log file, so a finished run can still be read back.
// ponytail: stdout rather than `out`, which a Ticket thread cannot borrow;
// the Shell (harness-kqe.10) replaces both with its RECENT panel.
struct Events(Option<File>);

impl Write for Events {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stdout().write_all(buf);
        if let Some(file) = &mut self.0 {
            let _ = file.write_all(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Preflights, then runs the Orchestrator in this process, where its log is
/// the pane's output and Ctrl-C ends it.
fn start(
    args: &[String],
    out: &mut dyn Write,
    repo: &Path,
    tools: Arc<dyn Tools>,
    env: &dyn Fn(&str) -> String,
) -> i32 {
    let parsed = match parse_start(args) {
        Ok(parsed) => parsed,
        Err(err) => {
            let _ = writeln!(out, "{err}");
            let _ = out.write_all(USAGE.as_bytes());
            return 2;
        }
    };
    let code = setup::report_missing(out, &setup::preflight(repo, &*tools, env));
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
    let log_path = repo.join(".harness").join("orchestrator.log");
    let log_file = File::options()
        .create(true)
        .append(true)
        .open(log_path)
        .ok();
    let o = match Orchestrator::new(Config {
        tools,
        repo: repo.to_path_buf(),
        main_pane: env("HERDR_PANE_ID"),
        workspace: env("HERDR_WORKSPACE_ID"),
        api_key: env("TYPESAFE_API_KEY"),
        home: PathBuf::new(),
        tick: Duration::from_secs(5),
        poll_prs: Duration::from_secs(30),
        max: parsed.max,
        log: Mutex::new(Box::new(Events(log_file))),
        events: std::sync::mpsc::channel().0, // the Shell (harness-kqe.9) holds the receiver
        #[cfg(test)]
        timeout: None,
    }) {
        Ok(o) => o,
        Err(err) => {
            let _ = writeln!(out, "start: {err}");
            return 1;
        }
    };
    let _ = writeln!(
        out,
        "Orchestrator running here (pid {}). Ctrl-C, or 'harness stop' from another pane, ends it.",
        std::process::id()
    );
    if !parsed.ticket.is_empty() {
        return i32::from(o.run_single(&parsed.ticket));
    }
    match o.run(&parsed.epic) {
        Ok(()) => 0,
        Err(err) => {
            let _ = writeln!(out, "start: {err}");
            1
        }
    }
}

#[cfg(test)]
mod cli_test;
#[cfg(test)]
mod init_test;
#[cfg(test)]
mod start_test;
