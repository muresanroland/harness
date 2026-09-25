# harness-7nq.13: session ids in the State, /continue resumes a Stage by its id

## Context
Today a stopped run whose Stage pane is gone (herdr restarted, reboot, later Ticket 14's long limit) always restarts that Stage fresh, so the session's work-in-progress conversation is lost. The Ticket says: save each Stage's herdr session id (`herdr agent get`: `agent_session.value`) in `.harness/state.json` beside its pane, with the App it ran on. When /continue finds the pane gone, the Orchestrator resumes the Stage by that id in a fresh pane (`claude --resume <id>` / `codex resume <id>`), sends `continue`, and watches it as a live session. A changed App or no id starts fresh, as today. /retry is always fresh. Ticket 14 (Limited) builds on this.

Verified read-only on this machine: `herdr agent get` returns `result.agent.agent_session.value`. That is true for claude panes and for codex panes (`agent_session` kind id, source herdr:codex). `codex resume --help` accepts `-s/--sandbox`, `-m` and `-c`. `claude --resume` combines with other flags.

## Changes

**`src/orchestrator/state.rs`**
- New `Session { app: String, id: String }` (id skipped when empty).
- `TicketState.sessions: BTreeMap<String, Session>` (Stage name to its session), placed after `panes` and skipped when empty. Old state files and the Go round-trip fixtures stay byte-identical.

**`src/orchestrator/herdr.rs`**
- `Agent` gets `agent_session: serde_json::Value`, read as `["value"].as_str()`. A `null` or missing field from herdr without its integration must never break the parse. Every status read goes through this struct.

**`src/orchestrator/app.rs`**
- `App.resume: &[&str]` arg form: claude `["--resume", "{}"]`, codex `["resume", "{}"]`. `fill` becomes `pub(crate)`.

**`src/orchestrator/stage.rs`**
- `fresh_pane(ticket, st, session)` records the `Session` beside the pane, in the same state update.
  - `attempt` passes `{app: row.app.name, id: ""}`, so a fresh session never inherits an old id.
- Extract `stage_args(ticket, st, row) -> Result<Vec<String>, String>` from `attempt`: the Stage's args plus `row.flags()`. The Implement plan-hook error path in `attempt` stays as it is.
- `watch(ticket, st, pane) -> Option<String>`: the same `agent get <pane>` as `agent_status`. It also saves the reported id into `sessions[st]` when non-empty and changed, with at most one state write per change.
  - It replaces `agent_status` in `attempt`'s poll loop and `hold`'s loop, where the Stage session is watched.
  - That catches an id herdr reports late, and a changed one.
- `resume(ticket, st, label, session) -> Result<String, String>`:
  1. Check that `stage_row`'s App equals `session.app`. If not, return Err("its App is now X").
  2. Call `fresh_pane(.., session.clone())`.
  3. Run `herdr agent start <name> --kind <app> --pane <p> -- <resume form with id> <stage_args>` via `start_agent`.
  4. Send `herdr agent prompt <p> continue`.
  5. Report `"{label} resumed: {row.said()} (pane …)"` and return the pane.
- `run_stage`: when `resumed`, the pane is not live, and `saved.sessions[st]` has a non-empty id, call `resume`.
  - Ok gives `live = Some(pane)`, so the existing live path runs `hold(armed = true)`.
  - Err writes a log-only line, `"{label} not resumed: {err}, starting it fresh"`, and `attempt` runs as today.
  - /retry inside `run_stage` (`Held::Retry`) already goes to `attempt`, so it stays fresh.

**`src/orchestrator/scheduler.rs`, `pipeline.rs`**
- Clear `sessions` wherever `panes` is cleared or removed:
  - The scheduler's /retry of a Parked Ticket: `sessions.remove(stage)`. Without it, /retry would resume by id.
  - PR opened: `sessions.clear()`.
  - Address done: `sessions.clear()`. Without it, a second /address would resume the first one.
- `reset_ticket` already resets the whole TicketState.

**`docs/design/events.md`**
- New row: "Stage resumed | implement resumed: claude (pane 2-1)".
- Add "not resumed … starting it fresh" to the log-only housekeeping.

**Rename `Held` to `Hold`** in `src/orchestrator/stage.rs`, `plan.rs`, `pipeline.rs` and `limit.rs`, and its variants to verbs (`Woke` → `Wake`, `Stopped` → `Stop`): the enum reads better as what the loop does next.

## Decisions the Ticket left open
- **Resume args.** The resume argv carries the Stage's usual args and the row's model and effort flags as well: permission mode, `--add-dir`, the Review's sandbox and Edit-deny settings. Without them a resumed session stops at permission prompts, or a claude Review could edit the worktree.
- **Implement resumes in plan mode**, with a fresh plan hook. If it was mid-implementation, it re-presents its plan and the plan is judged again. That is safe: the Plan gate is never skipped.
- **Resume failures.** If pane, start or prompt fails, the Stage starts fresh. Resuming is best-effort.
- **No trust wait on resume.** The App already ran in that directory. A trust dialog fails the start, which falls back to fresh, and fresh does wait for trust.
- **Nudge and waits.** `fresh_pane` resets them on resume too: the resumed session gets its nudge and waits again.
- **App check.** Only the App is compared. A changed model or effort still resumes, on the current flags.

## Tests (TDD, new `src/orchestrator/session_test.rs`, fake world)
World (`world.rs`):
- `Inner.integration: bool`: herdr's integration is installed. Each `agent start` then gets a session id, which `agent get` reports as `agent_session.value`.
- The default is off, so existing tests are unchanged.

Helpers:
- `stopped_at(stage, label, ids)`: run hx-1 until that Stage's session is working and, with `ids`, its id is saved. Then stop.
- `restarted(w, o)`: a new Orchestrator from state.json.

Tests:
1. `a_stage_start_saves_its_session_id_and_app_beside_its_pane`: state.json holds `sessions.review = {app: codex, id: <what the fake agent get reported>}`.
2. `stopped_run_resumes_by_session_id_when_its_pane_is_gone_and_watches_a_live_one`, a table:
   - Implement on claude, pane gone: one agent start with `-- --resume <id>`.
   - Review 1 on codex, pane gone: `-- resume <id>`.
   - The same Stage with its pane live: no agent start, no `continue`.

   Every case then checks:
   - `herdr agent prompt <pane> continue` was sent, and no Stage-skill prompt went to that pane.
   - The result file written afterwards, with the pane set idle, is accepted.
   - The run goes on to "PR #hx-1 opened".
3. `a_changed_app_or_no_session_id_starts_the_stage_fresh`:
   - Review's App changed to claude in config.json: a fresh `--kind claude` start, with no `resume` and a "review 1 started: claude" line.
   - Integration off (id empty): a fresh Implement start with no `--resume`.
4. `retry_of_a_parked_ticket_never_resumes`: park at Implement with its id saved, then `retry-hx-1`. Want 2 Implement starts, neither with `--resume`.

## Verification
- `cargo check`, `cargo clippy`, `cargo test session_test`, then the full `cargo test`.
- /code-review against `main`, then commit.
- Result file notes the live check the PR must list for the user: `herdr agent start --kind codex -- resume <id>` starts codex resumed. The "agent_session reported for codex panes" half was seen read-only here. Not run in this session.
