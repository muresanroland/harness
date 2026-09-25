# harness-7nq.16 — Stage questions: STATUS: question, /away, /continue @ticket

## Context

Stage skills today say "Nobody is watching this pane: do not ask questions". Map ticket harness-0sx.12 (Asks) replaces that: a Stage may write `STATUS: question` and wait; the Orchestrator treats it as neither done nor a Wake, never judges it, and either puts it to the user (Present) or parks the Ticket (Away). `/continue @ticket` unparks one Ticket and puts its question first. CONTEXT.md already has the vocabulary (Stage result, Question, Parked, Away) — no change there.

## Decisions the Ticket left open

1. **Result shape**: line 1 `STATUS: question`; `- ` lines are the options; every other nonblank line is the question text.
2. **Deadline**: nothing runs while the question waits. Once the answer goes in, the Stage deadline starts over (a nudge and a plan answer already do this). Simpler than keeping the time that was left, and the AC holds.
3. **Answer = prompt**: a picked option goes into the pane as its own text; "an answer of your own" goes in as typed. The Shell logs `you answered: <option>` / `you answered: your answer`. The Orchestrator then logs `sent your answer`. If the session carries on in the pane instead, it logs `carrying on`.
4. **`/continue @ticket` turns Away off** (and says `away: off`). The user is clearly back. If Away stayed on, the continued question would park the Ticket again at once.
5. **`/continue @ticket` works only on a Parked Ticket** (any other Ticket: `refused: Ticket N is not parked`).
   - With no run live, it unparks that Ticket and resumes the saved run through the existing `resume(&[(id, false)])`, so other Tickets saved as running resume too.
   - In a live Epic run it is the command `continue-<id>` to the scheduler.
   - In a live single-Ticket run it is refused with `continue refused: not an Epic run`, the same way /address is refused there.
6. **Read-only Stages (Review, Debate)** that park while Away keep their pane and their worktree snapshot. Their pane is not closed and the guard is not run. The continued Stage watches the same live session and is later compared with the tree it started from.
7. **Resuming by session id when the pane is gone**: `resume()` skips its `continue` prompt when the result file still reads `STATUS: question`, so the question comes back to the user instead of being answered by "continue".
8. **The Stage skills' ask rule**:
   - Ask only what the Ticket, the approved Plan, the repo docs and the Inputs leave open. Otherwise decide, and note where the answer came from.
   - Implement: before its plan is approved, an open question goes into the plan.
   - Moderator: the headless audit and side briefs say "nobody can answer: decide and note".
   - The nudge_proceed wording (judge.py) stays as it is.

## Changes

**`src/orchestrator/result.rs`**
- `read_stage_result`: `STATUS: question` is rejected with a new reason `ASKED` ("asked a question"), so every existing caller keeps reading it as not done.
- New `read_question(path) -> Option<(String, Vec<String>)>`: returns the question text and its options (decision 1).

**`src/orchestrator/stage.rs`**
- `Config.away: Arc<AtomicBool>`. The Shell's Config shares it with every run: `/away` flips it, and `Screen::open` starts it off. `for_tests` sets it to false.
- `Ask::Question { pane, question, options }`, `Held::Away`, and `pub(crate) const AWAY = "asked you while away"`.
- `run_stage` inner loop:
  - `Held::Woke(r) if r == ASKED` calls `self.question(...)`. If that returns `None`, re-arm with `hold(.., armed = true, None)`. Otherwise handle the `Held` it returns.
  - `Held::Away` becomes `Err(Parked(AWAY))`.
  - `attempt` and `hold` need no change: they already return `Woke(reason)` for an idle session whose result was not accepted.
- New `fn question(ticket, label, pane, file) -> Option<Held>`. It is modeled on `plan_answer` and `blocked`. On each tick:
  - **Away on**: run `bd comments add <ticket> <text>`. The text names the Stage, says the Ticket needs a manual resume with `/continue @<ticket>`, and quotes the question. A failed call is logged. Return `Held::Away`. This also covers Away turned on while the Question waits.
  - **First time round**: raise the Question with `asks(ticket, "question in {label} {at}", Ask::Question{..})`.
  - **Answers**:
    - `Prompt(text)`: send it with `herdr agent prompt`, report `sent your answer`, `settle(idle, done)`, then return `None`.
    - `Act(Park)` or `/park`: return `Held::Park`.
    - Anything else is dropped.
  - **Session state**:
    - Session gone: `Woke("session died")`.
    - No longer idle, or the file no longer holds a question: report `carrying on` (only if the Question was raised) and return `None`.
    - Stop: `Held::Stopped`.
  - No deadline and no Judgment are involved anywhere in this function.
- `resume()`: skip the `continue` prompt when `read_question(file)` returns a question (decision 7).

**`src/orchestrator/pipeline.rs`**
- `run_read_only`: on `Parked(AWAY)`, skip closing the pane, the guard and the snapshot removal (decision 6).

**`src/orchestrator/scheduler.rs`**
- In the command loop, `continue-<id>` for a Parked Ticket that is not busy sets its status to running and keeps its panes and sessions; `resumable()` then launches it. Unlike the retry branch, nothing is closed.

**`src/shell.rs`**
- `COMMANDS`:
  - Add `("/away", "", "a Stage's question parks its Ticket, again to turn off")`.
  - `/continue` args become `[<ticket>]`.
  - `list()` strips a leading `[` when working out what a command takes, so the @ list after `/continue` shows Tickets only.
  - Enter on a command typed whole still runs it when its argument is optional (`[`).
- `Screen.first: Option<String>`: set by `/continue @ticket`, cleared when the run ends. In `push()`, a Question for that Ticket goes in ahead of every other Ticket's Question. It still goes after a confirmation or the /continue checklist, and after the Question being composed if there is one. It also unhides.
- `/away`: toggle, then say `away: on, a Stage's question parks its Ticket` or `away: off`.
- `/continue <id>`: decisions 4 and 5. A plain `/continue` behaves as today.
- `Ask::Question` in the Shell:
  - `options()`: the Stage's options, then `an answer of your own`, `open the pane`, `park`.
  - `answer()`: handles those options.
  - `reply()`: gets the pane.
  - `push()`: uses the whole line as the short text.
  - While composing, the answer is labelled `your answer`.

**`src/shell/draw.rs`**
- `question_lines` shows the question text in the room the plan uses: the plan branch's or-pattern, not scrolled.
- `waiting()` puts `· AWAY` (orange) on the status row, live and idle, while Away is on.

**`skills/stage-{implement,review,moderate,fix,address}/SKILL.md`**
- The "Nobody is watching…" sentence becomes the ask rule plus the result shape (decision 8).
- stage-moderate: the audit prompt and the side brief each gain "Nobody can answer questions: decide and note."

**`docs/design/events.md`**
- New rows:
  - question in implement (pane 1-1)
  - asking you: question in …
  - you answered: `<option>` / your answer
  - sent your answer
  - parked: asked you while away
  - away: on … / away: off
  - continue refused: not an Epic run
  - refused: Ticket N is not parked

## Tests (test-first, fake world, no terminal)

- **`result_test.rs`**:
  - New `stage_result_acceptance` case: `STATUS: question` gives the reason "asked a question".
  - `read_question` returns the text and options; it returns `None` for done or a missing file.
- **New `src/orchestrator/question_test.rs`** (registered in `orchestrator.rs`):
  - Test 1, `a_stage_question_is_put_to_you_never_judged_and_the_deadline_waits`:
    - The session writes a question and goes idle. The test uses `cfg.timeout` of 20ms and a recording `Fake::down()` TypeSafe.
    - A `Question` Event arrives with its options.
    - After sleeping past the timeout there is no `timed out` line.
    - `o.answer(.., Prompt("ours"))` makes `herdr agent prompt <pane> ours` happen. The session then writes done, the run reaches the PR, and TypeSafe has had no requests.
  - Test 2, `away_parks_a_question_with_a_bd_comment_and_the_pane_open`: two cases, implement and review 1. Each checks:
    - the line `parked: asked you while away`
    - a `bd comments add hx-1 …` call
    - no `herdr pane close` of the Stage's pane, and the agent still alive
- **`shell_test.rs`**:
  - Test 1, `away_parks_a_stage_question_and_continue_at_ticket_puts_it_first`:
    - `/away` shows AWAY on the status row.
    - `/start-ticket hx-1`: the question parks the Ticket, the Shell shows `parked: asked you while away`, and the pane stays.
    - With no run live, `/continue @hx-1` turns Away off, starts no new agent, and puts the Question up with options `[ours, theirs, an answer of your own, open the pane, park]`.
    - Picking `ours` sends it, and the PR opens.
  - Test 2, `away_on_parks_a_question_already_waiting`: the Question is raised, then `/away` parks the Ticket and the Question goes away.
  - Test 3, `continue_at_ticket_in_a_live_run_unparks_it_and_puts_its_question_first`:
    - Epic run with hx-1 and hx-2, Away on.
    - hx-1 asks and parks. hx-2 goes idle without a result, which is a Wake Question.
    - `/continue @hx-1` makes hx-1's Question `questions[0]`, ahead of hx-2's Wake.
  - Update the existing slash-list tests for `/away` and `/continue [<ticket>]`.
- **`session_test.rs`**: one test where the pane is gone and the result file reads question. The Stage is resumed by id with no `continue` prompt, and its Question is raised.

## Verification

`cargo build --release`, `cargo check`, `cargo clippy`, `cargo test`, the full suite at the end. Then run /code-review against origin's default branch (`main`) and fix what it finds. Commit to `harness-7nq.16` (no push), then write `.harness/runs/harness-7nq.16/implement.md` with `STATUS: done`.
