# harness-7nq.12 — Implement off claude: the two-step Plan

## Context
Implement is refused off claude (`app.rs` row(): "implement runs on claude only") because the Plan review rides on Claude Code's plan mode + ExitPlanMode hook. The map (harness-0sx.14) settles a two-step Plan for other Apps: the session writes `<Run directory>/plan.md` and a Stage result `STATUS: plan`, waits; the Orchestrator judges/asks as today (Ticket 5's modal); feedback goes in as a prompt; approval sends `implement the approved plan`; a worktree changed before approval is a plan failure (Ask::PlanFailed). claude keeps its native path; the split stays claude-only.

## Changes

**`src/orchestrator/result.rs`**
- `pub(crate) const PLANNED: &str = "wrote its plan";` and `"plan" => return rejected(PLANNED)` in `read_stage_result` (neither done nor a Wake, like ASKED).

**`src/orchestrator/app.rs`**
- New App field `worktree_args: fn(run_dir: &str, git_dir: &str) -> Vec<String>` — the unattended args of a Stage in the worktree.
  - claude: `--permission-mode auto --add-dir <run_dir>` (today's hard-coded Debate/Fix/Address args, moved here).
  - codex: `--sandbox workspace-write --add-dir <run_dir> --add-dir <git_dir>` — run dir for plan.md + result; the repo's `.git` so Implement can commit (the worktree's objects/refs live there; codex keeps a root's own `.git` read-only). No plan mode, no `--settings`.
- row(): lift the refusal for `implement` (allowed off claude: implement, review, side_a, side_b); update the comment (Fix/Address/Moderator still wait on network).
- row(): a plan_model split with `app != claude` is refused: `"{file}: implement plan_model splits from model: the split runs on claude only"`.

**`src/orchestrator/stage.rs`**
- `stage_args`: native plan branch only when `st == IMPLEMENT && app == claude`; Review unchanged; else `(row.app.worktree_args)(&run_dir, repo/.git)`.
- `attempt`: Implement gets Input `("Plan", "native plan mode")` on claude, `("Plan", "write plan.md and STATUS: plan")` otherwise. Off claude, after `fresh_pane` (where the old result is removed) call `plan_written_start(ticket)`; its error is a Wake.
- `run_stage`: `Held::Woke(PLANNED)` handled like ASKED: `self.plan_written(..)` then, on None, `hold(.., armed=true)`. The unarmed-hold branch `reason == ASKED && idle` also takes PLANNED.
- `resume`: a session whose result reads STATUS: plan is not sent `continue` (as a question isn't) — else it would implement an unapproved plan; the live watch finds its plan again.

**`src/orchestrator/pipeline.rs`**: `head()`/`tree()` become `pub(super)` (reused for the snapshot).

**`src/orchestrator/plan.rs`** (reuse `plan()`, `plan_failed()`, the Judgment, `Ask::Plan`, `plans` map)
- `const APPROVED = "implement the approved plan"`, `const BEFORE = "before-implement.json"`.
- `writes_plan(ticket)`: the Implement Session's saved App (`ts.sessions["implement"].app`) is not claude. Dispatch point, no flag threading.
- `plan_written_start(ticket)`: remove old plan.md, clear `plans[ticket]`, write `[head, tree]` snapshot to BEFORE.
- `plan_written(ticket, st, label, pane)`: reads plan.md → `plan(..)`; missing → `Held::Woke("wrote STATUS: plan and no plan.md")`.
- `approve()`: if `writes_plan` → plan.md differs from judged → `Changed`; worktree differs from snapshot (HEAD moved or tree changed) → `Failed("changed the worktree before its plan was approved")` → existing plan_failed → Ask::PlanFailed; else remove the result file (plan no longer open), prompt APPROVED, "plan approved", new deadline, settle.
- `send_back()`: if `writes_plan` → Changed check as today, remove result file, then the existing common tail (prompt feedback, keep ts.feedback, "plan sent back with your feedback", deadline, settle). Dialog path unchanged.
- `plan_answer()`: still-waiting status is `idle|done` for a written plan, `blocked` for the dialog; anything else is "carrying on" as today.
- Module doc: a paragraph on the two-step Plan.

**`skills/stage-implement/SKILL.md`**: "Plan first" keyed on the **Plan** Input: `native plan mode` (today's text) / `write plan.md and STATUS: plan` (no worktree change before approval; write plan.md in the Run directory, then the Result file `STATUS: plan`, wait; feedback arrives as a prompt → revise + STATUS: plan again; approval arrives as `implement the approved plan` → step 4).

## Tests (test-first, fake world)
`src/orchestrator/plan_test.rs` — helper `writes(w)`: config `{"implement":{"app":"codex"}}`; session writes plan.md + `implement.md` = `STATUS: plan`, idle; FEEDBACK prompt → REVISED; APPROVED prompt → `STATUS: done`.
1. codex argv: `--kind codex`, ends `-- --sandbox workspace-write --add-dir <run> --add-dir <repo>/.git`, no `--permission-mode`/`--settings`, no settings.json; prompt has `- Plan: write plan.md and STATUS: plan`. Existing claude test also asserts `- Plan: native plan mode`.
2. STATUS: plan at 0.9 → lines plan ready / judged / plan approved / implemented; `herdr agent prompt <pane> implement the approved plan`; no send-keys; PR open.
3. Below floor (0.5) → plan Question with PLAN; Approve → APPROVED prompt.
4. Feedback [0.3, 0.95]: Prompt(FEEDBACK) → prompt sent, REVISED judged again with prior_feedback, approved; no send-keys.
5. Worktree changed before approval, two cases via hook (`git rev-parse HEAD` changes / `git status --porcelain` dirty once plan.md exists) → "stuck in implement: changed the worktree before its plan was approved (pane 1-1)", Ask::PlanFailed, no APPROVED prompt; park → parked.
- `result_test.rs`: table row `STATUS: plan` → PLANNED.
- `app_test.rs`: table row codex + split → "the split runs on claude only".
- `shell_test.rs` refusal test: its `implement` codex case becomes `moderator` (implement now allowed).

## Decisions (Ticket left open)
- codex Implement argv: Review's sandbox + `--add-dir` run dir + repo `.git` (Git write path; unverified live, listed as a live check). No `-a never`: an escalation ask reads blocked → the Blocked Question.
- Worktree check = snapshot at session start vs approval (reuses pipeline head/tree), stored in the Run dir so a resumed session is compared with its start.
- Resume with STATUS: plan skips `continue`.

## Verify
`cargo check`, `cargo clippy`, `cargo test plan`, `cargo test` full; /code-review vs `main`; commit; write result file.

## Open question
Should feedback on a written plan go into the session as a prompt, or should the session be restarted with the feedback in its Inputs? Both meet the Ticket. I will wait for your answer before implementing.
