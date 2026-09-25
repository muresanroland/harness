# harness-7nq.6 — Epic summary pager, /summary, close-Epic confirmation

## Context

Ticket harness-7nq.6 (layout B "Pager" of map ticket harness-0sx.5). When an Epic run has every
Ticket's PR open or merged (or the Ticket Parked), the Shell opens a read-only, full-terminal summary
once: per Ticket its PR, Rounds, Findings fixed / skipped / left on the PR, then PARKED with reasons.
`/summary [@epic]` rebuilds it on demand from bd, the State and the Run directory evidence. When the
last Ticket merges ("Epic done, every Ticket closed") a yes/no confirmation offers `bd close <epic>`.

## Changes

### Data: `src/shell/summary.rs` (new)
- `Summary { epic, title, tickets: Vec<Done>, scroll: Cell<usize>, opened: DateTime<Local> }`
- `Done { id, title, pr, merged, rounds, fixed, skipped: Vec<String>, left: Vec<String>, parked: Option<String> }`
- `Summary::build(repo, issues: &[BdIssue], state, epic) -> Result<Summary, String>`:
  - Epic title from the issues (`no Epic <id> in bd` otherwise); children = issues whose `parent` is the Epic, sorted by suffix.
  - Per Ticket, from its Run directory: Rounds = `verdict-1.md`, `verdict-2.md`… that exist, each read with
    the existing `read_stage_result`; PR = last `fix-N.md` with a `PR:` line (same parser).
  - left = fix items of the last Verdict when Rounds == `MAX_ROUNDS` (the cap, no Review re-checked them);
    fixed = all fix items minus left (disjoint counts); skipped = every Verdict's skip items.
  - Finding text shown = the Verdict line minus `- [fix] ` / `- [skip] ` and minus ` | reason: …`.
  - merged = bd `closed` or State `merged`; parked = State `parked` → its reason.
  - No Ticket with a Run directory or a State entry → `Err("no evidence for <epic>: none of its Tickets has run")`.

### Orchestrator touch-ups (reuse, no new logic)
- `result.rs`: `StageResult.skips: usize` → `Vec<String>` (the skip lines, like `fixes`); `pipeline.rs` uses `.len()`; `result_test.rs` expectation updated.
- `pipeline.rs`: `MAX_ROUNDS` → `pub(crate)`.
- `stage.rs`: free `pub(crate) fn run_dir(repo, ticket)`; `Orchestrator::run_dir` calls it.
- `world.rs` (test fake): `bd close <EPIC>` answers Ok instead of panicking on an unknown Ticket.

### Shell: `src/shell.rs`
- `COMMANDS` gains `("/summary", "[<epic>]", "the Epic's PRs, Rounds and Findings")` before `/exit`.
- `list()`: narrow by `takes.contains("<ticket>")` / `contains("<epic>")` (was `starts_with`) so `/summary @` lists Epics only; Enter runs a command typed whole when its args don't start with `<` (optional args), so `/summary` + Enter runs.
- `bd_list(repo, tools)` factored out of `load_epics` (same `bd list --json --brief --all` call and parse); used by the summary.
- `Screen` gains `summary: Option<Summary>` and `last_epic: String` (the Epic of the run that just finished, since Epic done clears the State; in memory only). `Run` gains `summarized: bool`.
- `summarize(epic)`: `bd_list` + `Summary::build` → `self.summary`, or the error as a notice.
- `poll()`: State snapshot moved after the scheduler join (so a finished run's snapshot is final). Then, for an Epic run not yet `summarized`: when every Ticket of the Epic on the bd cache is closed in bd or has State `pr-open`/`merged`/`parked`, at least one with its PR → `summarized = true`, `summarize`. Epic-done branch: remember `last_epic`, then `confirm("close Epic <id> <title>?", Pending::Close(id))`.
- `Pending::Close(id)`: yes runs `bd close <id> --reason "every Ticket merged"` then `reload_epics`; a failure is a notice. No → the existing "cancelled" path, nothing run.
- `/summary`: with a query → `reload_epics` + `resolve(query, epics)`; alone → `state.epic`, else `last_epic`, else notice `no Epic run yet, /summary @<epic> shows one`.
- `key()`: summary open takes the keys first (after Ctrl-C): Esc closes; ↑↓ PgUp PgDn Space Home End Tab Shift-Tab scroll; everything else ignored (read only).
- `scroll_plan(code)` → `scroll_rows(&Cell, code)`, shared by the plan modal and the summary; Tab/Shift-Tab jump by `heads` (headings in the plan, Tickets in the summary). `page`/`heads` Screen cells reused (only one of the two draws at a time).

### Draw: `src/shell/draw/pager.rs` (new), `draw.rs` dispatches to it when `s.summary` is Some
- Title bar: ` EPIC DONE · <epic> <title> ` (black on purple; `EPIC SUMMARY` when some Ticket has no PR yet and is not Parked) then `N PRs · N parked`, and the badges `N questions waiting`, `N new on RECENT`.
- Lead `Every Ticket has its PR. Review and merge them; each Ticket closes as its PR merges.` (or `Not every Ticket has its PR yet.`), then totals `N Rounds · N Findings fixed · N skipped · N left on its PR · N parked`.
- From 100 columns a 30-column TICKETS outline on the left (the Ticket in view marked ›), dropped below.
- Body: per non-Parked Ticket a rule in its color `<suffix> <title> ───── merged|to merge|no PR yet`, `PR #N <url>` (url cyan underlined, never cut, Cmd-clickable), `N Rounds · N fixed · N skipped · N left`, each `skipped: …` and `left on the PR: …` wrapped with `modal::wrap_spans`; then `PARKED` with each Ticket and `parked: <reason>`.
- Foot: ` rows a–b of n · p%` left, key hint right. Draw keeps the scroll inside and sets `page`/`heads`.

## Decisions (Ticket left open)
- fixed and left are disjoint; left only when Rounds hit the cap.
- A Ticket still in the Pipeline (a mid-run /summary) gets a section ending `no PR yet`; the title then reads EPIC SUMMARY.
- Auto-open needs at least one PR (an all-parked Epic never announces "Every Ticket has its PR").
- Most recent run's Epic is kept in memory only; after a restart /summary alone uses the saved State's Epic.
- No new RECENT line for closing the Epic; the tree reloads.

## Tests (`src/shell/shell_test.rs`, TDD, over the fake world)
1. Fake Run directories (hx-1: verdict-1..2, fix-2 with PR, bd closed; hx-2: verdict-1..3, fix-3 with PR; hx-3 Parked in State) → `/summary`: per-Ticket counts, totals line, PARKED with reason; render 120x40 (outline shown, position line) and 90x30 (outline dropped); Tab moves to the next Ticket; Esc closes. After bd closes hx-2, `/summary` shows it merged.
2. `/summary @hy` (bd hook with a second Epic) shows that Epic; an Epic with no evidence and `/summary` with no run are notices; `/summary @` lists Epics only; `/summary` + Enter runs.
3. Live run, hx-2 depends on hx-1: after hx-1's PR opens the summary is not open; hx-1 merges (per-PR gh state), hx-2's PR opens → summary opens by itself; Esc, more polls → it stays closed (once); `/summary` shows hx-1 merged.
4. Close confirmation: one-Ticket Epic to Epic done → summary open, Esc → `close Epic hx Epic hx?`; `y` runs `bd close hx --reason every Ticket merged`; a second world answers `n` → no `bd close hx` call.
- Existing tests adjusted: `/` list render test (10 commands), the Epic-done render test (Esc the summary first), `result_test` skips.

## Verification
- `cargo check`, `cargo clippy`, `cargo test summary`, then full `cargo test`.
- /code-review against main; fix findings; commit; write the result file.
