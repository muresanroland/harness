//! The harness command line.

use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::sync::Arc;

use crate::setup;
use crate::tools::Tools;

const USAGE: &str = "usage: harness [command]

  (no command)                open the Shell, which runs the Epics
  init [--force]              install the shipped skills, keep the TypeSafe key and preflight the Target repo
  --version                   print the version
";

/// Runs one harness command inside the Target repo and returns the exit code.
/// `input` answers init's questions; None is the terminal's stdin.
pub fn run(
    args: &[String],
    out: &mut dyn Write,
    input: Option<&mut dyn Read>,
    repo: &Path,
    tools: Arc<dyn Tools>,
    env: &dyn Fn(&str) -> String,
) -> i32 {
    // harness alone opens the Shell (ADR 0004).
    let Some(name) = args.first() else {
        return match crate::shell::open(repo, tools, env) {
            Ok(()) => 0,
            Err(err) => {
                let _ = writeln!(out, "harness: {err}");
                1
            }
        };
    };
    match name.as_str() {
        "init" => {
            crate::update::at_init(repo, out); // before the gate: a greater release re-execs
            let force = args.get(1).is_some_and(|a| a == "--force");
            // The questions read the terminal, a scripted input, or, when
            // stdin is neither, nothing: a non-interactive init cancels and skips.
            let (mut stdin, mut silent) = (io::stdin(), io::empty());
            let tty = input.is_none() && stdin.is_terminal();
            let input: &mut dyn Read = match input {
                Some(scripted) => scripted,
                None if tty => &mut stdin,
                None => &mut silent,
            };
            let asked = match setup::install_skills(repo, force, out, &mut *input, tty) {
                Ok(false) => return 0, // cancelled at the gate: nothing else runs
                Ok(true) => {
                    setup::ask_typesafe_key(repo, &env("TYPESAFE_API_KEY"), out, input, tty)
                }
                Err(err) => Err(err),
            };
            if let Err(err) = asked {
                let _ = writeln!(out, "init: {err}");
                return 1;
            }
            preflight(out, repo, &*tools, env)
        }
        "--version" | "version" => {
            let _ = writeln!(out, "{}", crate::version::version());
            0
        }
        _ => {
            let _ = out.write_all(USAGE.as_bytes());
            2
        }
    }
}

/// Prints what is missing and returns the exit code. A missing TypeSafe key is
/// a warning, not a failure: the run works with the user as the judge.
fn preflight(
    out: &mut dyn Write,
    repo: &Path,
    tools: &dyn Tools,
    env: &dyn Fn(&str) -> String,
) -> i32 {
    if setup::typesafe_key(repo, env).is_none() {
        let _ = writeln!(
            out,
            "preflight: no TypeSafe key: every Wake will be a Question"
        );
    }
    setup::report_missing(out, &setup::preflight(repo, tools, env))
}

#[cfg(test)]
mod cli_test;
#[cfg(test)]
mod init_test;
