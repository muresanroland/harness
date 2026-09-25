# harness-7nq.15 — Limited on the Review and on a Debate side

## Context
Ticket 14 made a usage limit hold its App until the reset, on every Stage. Ticket 15 carves out the two Stages the Pipeline can go on without: the Review (one Question for the run: wait / review with the fallback / open the PR unreviewed, the answer kept in the State until the reset) and a Debate side (the Moderator is told a side is Limited, and settles Findings without it). Scope = the Ticket's AC, nothing more.

## Changes

### State (`src/orchestrator/state.rs`)
- `enum Review { Wait, Fallback, Unreviewed }` (serde lowercase): the answer to the Review's limit Question.
- `State.reviews: BTreeMap<String, Review>` (App → answer), serde default/skip-empty. Stands while that App's limit holds.

### App table (`src/orchestrator/app.rs`)
- `IF_LIMITED = "review_if_limited"`: same key and semantics as the unmerged /config branch (harness-7nq.9): App default claude, model default `none` = no fallback; may run off claude (added to the off-claude allow list).
- `fallback_row(repo) -> Result<Option<Row>, String>`: the row, or None while its model is `none`.
- `debate_inputs(repo, run_dir, limited: impl Fn(&str) -> Option<String>)`: after the two commands, adds `("Side A"|"Side B", "limited until <t>")` for a side whose App is Limited. `Orchestrator::new` passes `|_| None`.

### Limit logic (`src/orchestrator/limit.rs`)
- `limited_until` → `pub(super)`.
- `review_answer(ticket, label, app) -> Result<Option<Review>, Held>`: None if the App is not Limited; the saved answer if there is one; else the Ticket holds (ts.limited set, log line "holds"). The first Ticket to wait asks the Question (`ask_only`, `Ask::Limited { app, fallback }`, where fallback is `fallback_row().said()` when set and on another App). An in-memory `asked: Mutex<BTreeSet<String>>` on the Orchestrator makes sure only one Question is out per App. The Ticket that asked clears the entry when it leaves, so a later waiter asks again after a /park. /park → Park; /stop → Stopped; the limit ends → None.
- `review_row(ticket, label, row) -> Result<Row, Held>`: while the Review's App is Limited, the answer decides what happens:
  - Wait: its own row (wait_limit holds next).
  - Fallback: `fallback_row` (unset since then: its own row, so it waits; a config error: Woke).
  - Unreviewed: `Err(Held::Done(StageResult { unreviewed: "<app> was limited until <t>" }))`.
- `limited()`: saving a limit whose previous reset no longer holds drops that App's saved answer (a new limit gets a new Question). For a Review with a short limit, after the hit line, it calls `review_answer`:
  - Fallback or Unreviewed: close the limited pane, drop its pane and session from the State, return `Held::Restart`.
  - Wait, or the limit is over: Ticket 14's flow as it is today (hold, `continue` at reset + 2 min).
  - A long limit on the Review still ends the run (Ticket 14's rule).
- `pub(crate) fn review(app, answer)`: the Shell's answer, saved into `State.reviews`.

### Stage loop (`src/orchestrator/stage.rs`)
- `Held::Restart`: the session gives way to a fresh one, and no retry is spent. run_stage: `retry = false; break` → attempt.
- `Ask::Limited { app: String, fallback: Option<String> }`.
- `Orchestrator.asked` field (and in `with_state`).
- attempt(): the Review's row goes through `review_row` before `wait_limit(row.app)`, so the fallback's App is held on if it is Limited too. The Debate's `debate_inputs` gets the closure `limited_until → until(reset, now)`. The fallback Review runs with the fallback App's `run_dir_args` (claude: run dir + `--add-dir <worktree>`), its trust and its session App. Nothing else changes, and run_read_only's guard still covers it.

### Result (`src/orchestrator/result.rs`)
- `StageResult.unreviewed: String`: the Review did not run, and why (the Fix's Input). Because it comes back as an Ok result, run_read_only's guard runs on it too.

### Pipeline (`src/orchestrator/pipeline.rs`)
- A Review result with `unreviewed` set: report `review <n> and debate <n> skipped: <why>`, then skip the Debate. fixes are empty, so this is the last Round. Fix inputs: `Open PR: yes`, `Fix items: none`, `Verdict history`, plus `Unreviewed: <app> was limited until <t>`.

### Shell (`src/shell.rs`)
- `options()` for `Ask::Limited`: `wait for the reset`, `review with <fallback>` (only when set), `open the PR unreviewed`.
- `answer()`: maps the choice to a `Review`, removes the Question, tells `you answered: <option>`, and calls `run.o.review(app, answer)`. `Ask::Limited` goes in the whole-text arm of the "asking you" short text. draw.rs needs no change: the Question box is generic.

### Skills
- `skills/stage-moderate/SKILL.md`: new section "A side at its usage limit".
  - A side is limited when an Input says so, or when its command fails with the limit text: codex exits 1 with it on stderr, claude exits non-zero with it on stdout. It is not a failed side.
  - A side on the Moderator's own App: the Moderator is limited too; it reruns that side after the reset.
  - Otherwise the Debate stops (no argument). If side A is limited, the audit is skipped.
  - TypeSafe on: every Finding, the audit's too, goes to the TypeSafe score with empty arguments; 0.5 or more is fix; settled `typesafe <score>, <app> limited`.
  - TypeSafe off (Input `TypeSafe: off`, or no key): every Finding is skip, settled `<app> limited, no TypeSafe`.
  - A Notes line says which side was limited and until when.
  - The literal strings `claude -p` and `codex exec` are avoided: an existing test forbids them in this skill.
- `skills/stage-fix/SKILL.md`: with Input **Unreviewed**, the PR description says at the top that no second model reviewed it, why, and that a human review is required.
- `docs/design/events.md` Limited table: rows for the Review's Question, the answer, the skipped line, and the Moderator's Side Input.

## Decisions the Ticket left open
- The Question also comes up when a Ticket reaches the Review while the Review's App is already Limited and no answer exists, for example after a restart, since the Question is never saved. It is asked once per App through `asked`.
- The Review Question only covers short limits. A long limit keeps Ticket 14's rule: the run ends.
- If the fallback runs on the limited App itself, the Question does not offer it.
- "No argument" on a limited side means the Debate is not argued: TypeSafe gets each Finding with empty arguments.
- A Review resumed by id after /continue while its App is Limited still waits for the reset instead of applying the answer. Rare case; noted in the result file.
- LIMITED box wording such as "reviews by claude": not in this Ticket's AC, skipped.

## Tests (test-first)
`src/orchestrator/limit_test.rs` (fake world, clock 2:00pm, codex tail `Try again at 3:05 PM`):
1. A codex limit on hx-1's Review raises one Question (no fallback offered). hx-2, spawned after it, holds before its Review, and no review session starts. Answering Unreviewed opens both PRs with no debate session and hx-2 never starting a review. Each fix-1 prompt carries `Open PR: yes`, `Fix items: none` and `Unreviewed: codex was limited until 3:05pm`. After the clock passes the reset, hx-3's review starts on codex. The shipped stage-fix names the Unreviewed Input.
2. config `review_if_limited {app: claude, model: opus}`: answering Fallback starts `review 1 started: claude opus`. The second review agent start is `--kind claude` with `--add-dir <worktree>` and `--model opus`, and the PR opens.
3. Answering Wait: hx-2 holds before its Review. At 3:07pm hx-1's pane gets `continue` and carries on, and hx-2's review starts on codex.
4. codex Limited in the State, Implement and Review done: the Moderator's Inputs carry `- Side B: limited until 3:05pm` and no Side A line. The shipped stage-moderate names that Input.

Other test files:
- `src/orchestrator/state_test.rs`: the round trip also covers `reviews`.
- `src/shell/shell_test.rs`: through the Shell, the Question shows its 3 options including `review with claude opus`. Picking "open the PR unreviewed" logs `you answered: …`, the PR opens, and the saved state holds `reviews.codex = unreviewed`.
- Existing Limited tests keep passing: the codex Review hold and the "Try again later" re-look.

## Verification
`cargo check`, `cargo clippy`, `cargo test` (the new tests, then the full suite), then /code-review against `main`, fix what it finds, commit, and write the result file.
