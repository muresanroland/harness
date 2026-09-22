# The Rust port runs on std threads, with no async runtime

The Rust port keeps the Go shape: one package at the repo root, one thread per Ticket that is never joined, a tick loop that sleeps and polls, and a `stopped` flag that every sleep checks. tokio was rejected because nothing here waits on many sockets: the Orchestrator waits on subprocesses and files, the screen is one thread that polls crossterm, and an async runtime would pull every Stage and test through `.await` for no concurrency gain. Every external command (herdr, bd, gh, git) goes through the Tools seam, a one-method trait with the real `Exec` and the fake world behind it, so tests never start a process.

## Considered options

- A Cargo workspace with a crate per Go package: scaffolding for four thousand lines; `pub(crate)` gives the same privacy `internal/` did.
- Signal handling (`ctrlc`, `libc`): not needed. A killed run leaves an atomically saved state file, the lock is an advisory `File::try_lock` the kernel releases on exit, and the screen runs in raw mode where Ctrl-C is a key event.
- `--detach` and `Setsid`: not carried over. The screen owns the Orchestrator in-process.

## Consequences

- Dependencies are serde, serde_json, regex and chrono (local time for log lines), with ratatui, crossterm and ureq arriving with their features. Anything else needs a ticket.
- `harness stop` may take up to one tick to be noticed, since sleep is a plain `thread::sleep`.
- The shipped skills are a hand-listed `include_str!` array; a new skill is one more line.
- Tests are in-crate `#[cfg(test)]` modules, one file per Go test file, so the fake world keeps reaching private state.
