# harness-7nq.8: plan on one model, implement on another (opusplan, halves remapped)

## Context

Implement plans first, then implements, in one claude session. The map tickets harness-0sx.6 and harness-0sx.9 decided that Implement's config row can name a plan model. On a split the session runs `--model opusplan`, and its `--settings` file remaps the two halves:
- `ANTHROPIC_DEFAULT_OPUS_MODEL` is the plan model.
- `ANTHROPIC_DEFAULT_SONNET_MODEL` is the implement model.

The Harness then approves the Plan with the "Yes, clear context and …" option, so the implementing model starts from the Plan alone. With no split nothing changes. Research: `research/stage-models:docs/research/stage-models.md`.

## Changes

### 1. `src/orchestrator/app.rs`: the plan model in Implement's row
- `Row` gets `plan: Option<String>`. It is Some only on a split: the plan model differs from Implement's model and is not `default`.
- `stage_row` reads a new field, `plan_model`, for the `implement` key only, with the same `field()` closure. A missing or empty field defaults to Implement's model, which means no split.
- A split with Implement's model at `default` is refused: `Err("{file}: implement plan_model is set with model default: the split needs a named model for each half")`. The refusal happens where the config is read. `Orchestrator::new` therefore refuses the run, and `attempt` Wakes.
- `Row::flags()`: on a split the model value is `opusplan`. Effort is unchanged, and it covers both halves.
- `Row::said()`: on a split the model reads `{plan}→{model}`, so the started line becomes `implement started: claude fable→opus/high (pane …)`.
- Decision: the key is `plan_model`, beside `app`/`model`/`effort`.
- Decision: no family check here. Implement is claude-only already (`stage_row`), and the family rules belong to /config's checks (harness-7nq.10).

### 2. `src/orchestrator/plan.rs`: the settings and the approval
- `plan_settings(ticket, &row)`. The call site is `stage.rs` `attempt()`, which passes the `row` it already has. On a split it adds three things to today's JSON:
  - `env`: `{ANTHROPIC_DEFAULT_OPUS_MODEL: plan, ANTHROPIC_DEFAULT_SONNET_MODEL: model}`
  - `showClearContextOnPlanAccept: true`
  - `hooks.PostModelSwitch: [{hooks: [{type: command, command: "'<exe>' __switch-hook '<repo>/.harness/orchestrator.log' '<ticket>'"}]}]`

  With no split the file stays byte for byte as today. A new const `SETTINGS = "settings.json"`.
- Extract send_back's cursor loop into `fn cursor_to(&self, pane, dialog: Dialog, label) -> Moved`. The loop is: down one key per call, re-read the pane after each key, at most one down per option, stop after a down that did not move the cursor. `enum Moved { On, Stalled, Gone, Failed(RunError) }`. send_back keeps its current outcomes: NotSent(stalled), NotSent(not on screen), Failed(unanswered).
- `approve()`: the guard stays as today (the dialog shows, plan.md is the judged plan, the cursor is on a Yes). When the session's settings file has `showClearContextOnPlanAccept: true`, it first calls `cursor_to(pane, dialog, "Yes, clear context")`. The outcomes map as follows:
  - Stalled: `Approval::Failed("the cursor never reached Yes, clear context")`, which is a plan failure Question.
  - Gone: `Approval::Gone`.
  - Failed: `Approval::Failed(unanswered(err))`.

  Then it sends enter as today. With no split, approval stays enter on option 1.
- Decision: the split is detected from the run dir's settings.json and not from an in-memory flag. That file is exactly what the live session started with, it outlives an Orchestrator restart, and a config change mid-Stage cannot touch it.

### 3. `src/cli.rs`: the hidden `__switch-hook` mode
- `"__plan-hook" | "__switch-hook"` share the stdin handling and the "exit 1, never 2" rule. A small `hook_input(input) -> Result<Value, String>` does the read and JSON parse for both hooks.
- `switch_hook(args, input)` takes the log path and the ticket. It reads `to_model` from the hook's input and appends `log_line(now, ticket, "implement switched to <model>")` in one write to the log file, as the Shell and the Orchestrator already do. It is not in the usage.
- Decision: the switched line goes to `orchestrator.log` and not to RECENT. The hook runs in Claude Code's process, outside the Shell. Getting the line onto RECENT would need the Stage loop to poll a file, and the Ticket says "logs".

### 4. `docs/design/events.md`
Add the split started line, `implement switched to <model>` (log only), and the Wake reason `the cursor never reached Yes, clear context`.

## Tests (test-first, /tdd)
- `app_test.rs`: one more row in `an_unreadable_config_or_a_stage_off_claude_wakes_the_stage_that_reads_it`. The row `{"implement": {"plan_model": "fable"}}` must Wake implement with the refusal, and no agent may start.
- `plan_test.rs`: a new table test runs a split and a no-split config on a fake 4-option dialog: auto, "Yes, clear context and use auto mode", manual, and feedback, with clear-context second so the cursor has to move. It sets `w.lock().options`.
  - The split (`model opus, effort high, plan_model fable`) must show:
    - argv `… --model opusplan --effort high`
    - settings.json equal to the JSON with the env, `showClearContextOnPlanAccept`, and both hooks
    - keys `["down", "enter"]`
    - the line `implement started: claude fable→opus/high (pane 1-1)`
  - The no split (`plan_model` equal to the model, or absent) must show `--model opus`, no env or showClearContext keys in the settings, and keys `["enter"]`.
- `cli_test.rs`: `the_switch_hook_logs_the_model_it_switched_to`. Given PostModelSwitch input `{from_model, to_model}`, it must exit 0 with an empty stdout, and the log file must end with `<ticket> implement switched to claude-opus-5-5`. Bad input must exit non-zero, never 2. The mode must not be in the usage.
- The existing tests pass unchanged. That covers no config and the exact no-split settings JSON.

## Verification
- `cargo check`, `cargo clippy`, then `cargo test plan_test app_test cli_test` as I go, and the full `cargo test` at the end.
- Run /code-review against `main`, then fix what it finds.
- Commit on harness-7nq.8. Do not push, open a PR, or close the Ticket.
- Result file `.harness/runs/harness-7nq.8/implement.md`, first line `STATUS: done`. It records the decisions above and the **live checks for the PR**, which I do not run:
  - The model still switches after a clear-context approval. Fallback: approve with option 1 and keep the context.
  - Aliases work in `ANTHROPIC_DEFAULT_*_MODEL`. Fallback: resolve the alias to a full id with a probe.
  - The PostModelSwitch hook's name and its input field `to_model`.
