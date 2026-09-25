# harness-7nq.7: the App table and .harness/config.json

## Context

Today each Stage hard-codes its agent in the Stage table (`kind` in `src/orchestrator/stage.rs:23-44`), and `attempt()` picks the args by `kind == "codex"` (`stage.rs:703-731`). The Ticket moves the App out of the Stage table: a compiled-in **App table** (claude, codex) holds each App's arg forms and trust reader, and `.harness/config.json` holds one row per Stage: an App, a model and an effort. With no file, every argv stays as it is today. Later Tickets (/config 7nq.9, two-step Plan 7nq.12, side commands 7nq.11, Delegate skills 7nq.19, preflight 7nq.20) build on this.

## Changes

### New `src/orchestrator/app.rs` (registered in `src/orchestrator.rs`)

- `App` struct, `static APPS: [App; 2]`, and `app(name) -> Option<&'static App>`. Fields:
  - `name`: the name used in config.json and passed to herdr `--kind`.
  - `binary`, `homepage`: nothing reads these yet, so each gets `#[allow(dead_code)]` and a comment naming the Ticket that will read it.
  - `worktree` args (for a Stage in the worktree; `{}` stands for the Run directory).
  - `run_dir` args (for the Review, which runs in the Run directory; `{}` stands for the worktree).
  - `network` args (added for Fix and Address).
  - `model` and `effort` arg forms (`{}` stands for the value).
  - `mention`: the Delegate-skill mention form, also `#[allow(dead_code)]`, read by 7nq.19.
  - `trust`: `fn(&Path, &Path) -> Option<bool>`, the trust reader.
- Rows:
  - **claude**:
    - worktree and run_dir args: both `--permission-mode auto --add-dir {}`
    - network: none
    - model: `--model {}`; effort: `--effort {}`
    - mention: `the {} skill`
    - trust: `claude_records`
    - homepage: https://claude.com/product/claude-code
  - **codex**:
    - run_dir args: `--sandbox workspace-write` (today's)
    - worktree args: `--sandbox workspace-write -a never --add-dir {}` (the unattended args from the research)
    - network: `-c sandbox_workspace_write.network_access=true` (the key is confirmed in the strings of the installed codex 0.156.1 binary)
    - model: `-m {}`; effort: `-c model_reasoning_effort={}`
    - mention: `${}`
    - trust: `codex_records`
    - homepage: https://developers.openai.com/codex
- `ROWS: [(&str, &str); 8]`: each config.json key and the App it starts on:
  - `implement` claude, `review` codex, `review_if_limited` none, `moderator` claude
  - `side_a` claude, `side_b` codex, `fix` claude, `address` claude
- `Row { app: &'static App, model: String, effort: String }`:
  - `flags()`: the model and effort args, leaving out any field set to `default`.
  - `said()`: `claude`, `claude opus`, `claude opus/high`, or `claude default/high` when only the effort is set.
- `fill(form, value) -> Vec<String>`: puts the value in place of `{}` in each arg.
- `stage_row(repo, &Stage) -> Result<Row, String>`: reads `.harness/config.json` as a `serde_json::Value`.
  - A missing file, row or field falls back to the default. The Debate Stage reads the `moderator` row.
  - Refused, with the error returned:
    - a file that cannot be read or parsed: `"<path>: <err>"`
    - an unknown App: `"<path>: no App named \"x\" for fix"`
    - Implement on an App other than claude: `"Implement off claude needs the two-step Plan"`

### `src/orchestrator/trust.rs`
- Make `claude_records` and `codex_records` `pub(super)`.
- `trusts(app: &App, home, dir, repo)` reads trust through `app.trust`, so the `kind == "codex"` branch goes away.

### `src/orchestrator/stage.rs`
- `Stage` drops `kind`. It keeps name, skill and timeout, and the `stage()` const fn loses its `kind` parameter.
- `Orchestrator::new` calls `stage_row` for each of the 5 Stages before `load_state`, and turns an error into `io::Error::other`. The Shell's `prepare()` already shows that error as a notice, so a run from /start-epic, /start-ticket or /continue is refused, naming the file.
- `attempt()`:
  - It calls `stage_row` first. An error becomes `Held::Woke(err)`, the same pattern as "has no Stage skill". The file is read each time a Stage starts, so a change reaches only the Stages that start after it.
  - Implement keeps its plan-mode args as they are. `stage_row` has already refused any App but claude.
  - The Review uses `fill(app.run_dir, worktree)`. Every other Stage uses `fill(app.worktree, run_dir)`.
  - Fix and Address also get `app.network`.
  - Then `row.flags()` is appended, and `--kind row.app.name` is passed.
  - The started line becomes `"{label} started: {row.said()} {at}"`.
- `await_trust` takes `&App` and uses `app.name` in its lines.
- `stage_cwd`: the Review runs in the Run directory and every other Stage in the worktree (today this depended on `kind == codex`).

### Docs
- `docs/design/events.md:24`: add a started line that names a model and effort, `implement started: claude opus/high (pane 2-1)`.

## Tests (test-first, `/tdd`)

New `src/orchestrator/app_test.rs`, on the fake world:
1. **Model and effort flags**, from a partial config.json:
   - Implement ends `... --add-dir <run> --model opus --effort high`.
   - The Review, with no app set, falls back to codex and ends `--sandbox workspace-write -m gpt-6-sol -c model_reasoning_effort=low`.
   - The Moderator on codex ends `--sandbox workspace-write -a never --add-dir <run>`, with no network.
   - Fix on codex ends with the same args plus `-c sandbox_workspace_write.network_access=true -m gpt-6-luna`.
   - Started lines: `implement started: claude opus/high (pane 1-1)` and `review 1 started: codex gpt-6-sol/low (pane 1-2)`.
2. **Review on claude, and trust read through each row's App.** The home trusts the repo only in `.claude.json`, and config.json sets `review: claude, fix: codex`.
   - The Review starts without a trust wait, with its pane cwd in the Run directory and argv ending `--permission-mode auto --add-dir <worktree>`.
   - Fix then waits: `waiting: codex does not trust <worktree>`.
3. **config.json changed between two Stages reaches the second.** The session hook writes `{"review":{"model":"gpt-6-sol"}}` during Implement. Implement's argv has no model, and the Review's argv ends `-m gpt-6-sol`.
4. **config.json that cannot be parsed, read when a Stage starts, Wakes that Stage.** The line reads `stuck in implement: <path>: …`, and no agent start is made.

In `src/shell/shell_test.rs`:
5. **Run-start refusals.**
   - config.json that cannot be parsed, then `/start-epic hx`: the notice starts with `<path>: ` and no run starts.
   - `{"implement":{"app":"codex"}}`: the notice reads `Implement off claude needs the two-step Plan`, no run starts, and no worktree is created.

Existing tests stay unchanged. They cover "no config.json gives today's argv", including `plan_test.rs:112-125` and `pipeline_test.rs:46-58,110-113`. The only exception is `trust_test.rs`, which moves to the `trusts(&App, …)` signature through `app("claude")` and `app("codex")`.

## Decisions made without asking (these go in the result file)
- The config.json shape is `{"<row>": {"app", "model", "effort"}}`. The row keys are snake_case, the missing parts fall back to defaults, and an empty string counts as default.
- The started line reads `default/high` when only the effort is set.
- The file is also checked at run start, so a run is refused up front rather than Waking on its first Stage.
- Network goes only to Fix and Address, as the Ticket says. A Moderator on codex runs `claude -p`, `codex exec` and curl without network. That is for Ticket 11 or the reviewer to decide.
- Untested live: a Stage on codex in the worktree runs `git commit`, which writes into `<repo>/.git`, outside the sandbox's writable roots.
- `binary`, `homepage` and `mention` are in the table, as the Ticket lists them, but nothing reads them yet (`allow(dead_code)`).
- The stale codex comment in trust.rs's header is left alone, because the trust-readers Ticket owns it.

## Verification
- `cargo check`, `cargo clippy`, `cargo test app_test`, `cargo test trust`, `cargo test shell_test`, then the full `cargo test`.
- `/code-review` against `main`, then commit to `harness-7nq.7`, then write `/Users/rolandmuresan/Documents/Projects/harness/.harness/runs/harness-7nq.7/implement.md` with `STATUS: done`.
