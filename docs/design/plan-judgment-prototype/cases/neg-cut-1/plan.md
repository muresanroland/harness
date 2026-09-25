# harness-7nq.1: three small fixes

## Context
Ticket harness-7nq.1 (first of Epic harness-7nq). Three small changes settled on map harness-0sx / prototype harness-0sx.9 / research harness-0sx.13. Scope = ticket only.

## Changes

1. **Plan floor** `src/orchestrator/judgment.rs:24`: `PLAN_FLOOR` 0.8 → 0.75. `FLOOR` stays 0.7.
   - `docs/testing/live-run.md:167` says "scores below 0.8" → 0.75 (tester guidance, would mislead). `docs/design/events.md:62` left: historical decision note.
2. **Open the pane** `src/shell.rs:787` `open_pane`: argv `["herdr","pane","focus",&pane]` → `["herdr","agent","focus",&pane]`.
   - Decision: pass pane id, not `agent_name(ticket, stage)`. `herdr agent focus <target>` (0.9.1 `--help`) + herdr skill: "Agent commands accept either a unique live agent name or the pane ID currently hosting that agent". Every Question already carries pane id; no stage in scope at `open_pane`.
3. **trust.rs header** `src/orchestrator/trust.rs:1-6`: drop "Codex's does not: reads as idle, result file missing". New: both dialogs register as blocked pane (herdr's codex manifest reads Codex trust screen as blocked); neither answerable by Orchestrator, so it reads trust stores and waits. Grep found no other comment repeating the claim (stage.rs:753 is about any dialog with `--wait`, not Codex; left).

## Tests (test-first, /tdd: change tests, see red, then code)

- `src/orchestrator/plan_test.rs`
  - `a_fresh_plan_at_its_dialog_..._approves_it`: noul 0.9 → 0.76; expected line "judged: plan follows the Ticket 0.76"; doc comment "yes at 0.76". Red at 0.8 floor.
  - `below_the_floor_...`: "yes below the floor" `Ok(0.7)` → `Ok(0.74)`, said "0.74". (Passes either floor; the 0.76 case is the red one.)
- `src/shell/shell_test.rs`
  - line ~1661 (Wake "open the pane") and ~2233 (PlanFailed): expect `herdr agent focus {pane}` in world `called` / fake `calls()`.

## Verify
- `cargo check`, `cargo test` (touched tests while going, full suite at end), `cargo clippy --all-targets`.
- /code-review vs `git symbolic-ref --short refs/remotes/origin/HEAD` (fallback main); fix findings.
- Commit on branch harness-7nq.1. No push, no PR, no bd close.
- Write `/Users/rolandmuresan/Documents/Projects/harness/.harness/runs/harness-7nq.1/implement.md`, first line `STATUS: done`.
