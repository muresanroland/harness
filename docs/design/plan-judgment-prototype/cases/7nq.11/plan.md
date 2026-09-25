# harness-7nq.11: Debate side commands as Inputs, read-only guards on Review and audit

## Context

stage-moderate hard-codes `claude -p` (audit, Claude side) and `codex exec --sandbox read-only` (GPT side). Ticket 7 put an App, model and effort per Stage row in `.harness/config.json`, but the Debate sides still ignore it. This Ticket: the Orchestrator builds each side's headless read-only command from the App table plus the row's model and effort, and passes the commands to stage-moderate as Inputs. The skill runs what it is given. Plus two guards: the skill checks the worktree after the audit and the sides, and the Orchestrator checks the worktree after every Review (only codex has a sandbox).

## Changes

### 1. `src/orchestrator/app.rs`: side rows and side commands
- New `App` field `side: &'static [&'static str]`, the headless read-only command head:
  - claude: `["claude", "--disallowedTools", "Edit,Write,NotebookEdit", "-p"]`. `--disallowedTools <tools...>` is variadic, so it goes before `-p`. Next to the brief it would take the brief as a tool name.
  - codex: `["codex", "exec", "--sandbox", "read-only"]`.
- `stage_row(repo, st)` keeps its signature. Its body moves into `fn row(repo, key)`, so side rows reuse it.
  - Defaults: `review` and `side_b` default to codex. Every other row defaults to claude.
  - The "runs on claude only" refusal skips `review`, `side_a` and `side_b`.
- `Row::side_command() -> String`: the App's `side` head plus `flags()`, each arg a shell word, space-joined.
- `fn word(arg)`: bare when every char is in `[A-Za-z0-9-_./=,:@%+]`, else single-quoted like `plan.rs`'s `quoted`. A model id like `claude-opus-5-5[1m]` is a zsh glob unquoted.
- `pub(crate) fn debate_inputs(repo) -> Result<Vec<(&'static str, String)>, String>` returns three Inputs:
  - `Side A command` (row `side_a`)
  - `Side B command` (row `side_b`)
  - `Audit command`: the same string as side A's. The side A row "also runs the ponytail audit" (Ticket 7).

### 2. `src/orchestrator/stage.rs`
- In `attempt()`, right after `stage_row`: for the Debate only, call `debate_inputs`. `Err` returns `Held::Woke(err)` before any pane opens, the same path a bad row takes. Its Inputs are appended to `inputs` for `stage_prompt`. Config is still read when the Stage starts, so a retry picks up a change.
- `Orchestrator::new`: also calls `debate_inputs`, so a bad side row refuses the run like any other row.

### 3. `src/orchestrator/pipeline.rs`: Review guard
- Before `run_stage(REVIEW)`: `head = git rev-parse HEAD` in the worktree, through Tools. `None` if git fails.
- After it returns `Ok`: `guard_review(ticket, round, head)`.
  - If HEAD moved, or `git status --porcelain` is non-empty (or fails): run `git reset --hard <head>` then `git clean -fd`, and report `review <n> changed the worktree: restored` on RECENT.
  - If the restore fails, the Ticket parks: `review <n> changed the worktree, not restored: <err>`. A dirty tree must not reach Fix, which commits it.
  - A clean Review: two read-only git calls and nothing else.
- Runs for every App. `-fd` without `-x` keeps ignored build caches.

### 4. `skills/stage-moderate/SKILL.md`
- Frontmatter and body say side A and side B, never "Claude side" or "GPT side".
- Step 1: `<Audit command> "Run the ponytail-review skill …"` replaces the `claude -p` line.
- Step 2: `<Side A command> "<brief>"` and `<Side B command> "<brief>"` replace the hard-coded lines.
- New guard paragraph: record `git rev-parse HEAD` before the audit. Check `git status --porcelain` and HEAD after the audit and after each side. Both sides run in parallel, so each round is checked once both have exited. On a change: run `git reset --hard <recorded>` then `git clean -fd`, and note under the Verdict's Notes which run changed it (the audit, or the sides' opening or critique round).
- No `claude -p` or `codex exec` text is left anywhere in the skill.

### 5. `CONTEXT.md`
- Moderator entry: "between side A and side B" instead of "a Claude side and a GPT side". One line, so the glossary matches the skill.

## Tests (TDD, fake world)

`src/orchestrator/app_test.rs`:
1. `the_moderators_inputs_carry_each_sides_command_and_the_audits`:
   - With no config.json: `Side A command: claude --disallowedTools Edit,Write,NotebookEdit -p`, `Side B command: codex exec --sandbox read-only`, and Audit equal to side A.
   - With `side_a` on codex (`gpt-6-sol`/`low`) and `side_b` on claude (`claude-opus-5-5[1m]`/`high`): `codex exec --sandbox read-only -m gpt-6-sol -c model_reasoning_effort=low` and `claude --disallowedTools Edit,Write,NotebookEdit -p --model 'claude-opus-5-5[1m]' --effort high`.
   - The assertions read the Debate's `herdr agent prompt`.
   - The same test checks the prompt's skill body (before `## Inputs`) has no `claude -p` and no `codex exec`.
2. Extend `an_unreadable_config_or_a_stage_off_claude_wakes_the_stage_that_reads_it` with one case: `{"side_b": {"app": "pi"}}` Wakes `debate 1` with `no App named "pi" for side_b`.

`src/orchestrator/pipeline_test.rs`:
3. `a_review_that_dirties_or_commits_the_worktree_is_restored_and_says_so`: two cases. A world hook answers `git rev-parse HEAD` and `git status --porcelain` from state that the Review session changes (dirty tree / new HEAD), and `git reset --hard` clears it. Asserts `git reset --hard <recorded>` and `git clean -fd` each run once, and the line `hx-1 review 1 changed the worktree: restored`.
4. `a_clean_review_changes_nothing`: default world. Asserts `git rev-parse HEAD` ran, no `git reset` or `git clean`, and no "changed the worktree" line.

## Decisions the Ticket left open
- Config keys `side_a` and `side_b`, beside `moderator`. Ticket 9's /config writes them.
- The Audit command is side A's full command, model and effort included.
- Either side may run on claude or codex. The two-families check is Ticket 10's.
- The Review guard runs only when the Review returns `Ok`. On Parked or Stopped the session may still be live.
- A failed restore parks the Ticket.
- A resumed Review records HEAD at resume time. A commit made before the stop shows only if it left the tree dirty. Accepted edge, marked with a `ponytail:` comment.

## Verification
- `cargo check`, `cargo clippy`, then `cargo test app_test pipeline_test` while working, and the full `cargo test` at the end.
- `/code-review` against `main`, and fix what it finds. Commit to `harness-7nq.11`. Write the result file.
