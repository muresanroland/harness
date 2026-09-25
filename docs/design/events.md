# Events: the RECENT panel and the log

Every Orchestrator event is one plain-language line, the same words on the Shell's RECENT panel and in `.harness/orchestrator.log`. There is no machine-readable line; `.harness/state.json` is the machine record. Decided on the map ticket "Human-readable event vocabulary for the RECENT panel and the log" (harness-7bj.6).

## Line shape

Panel: `HH:MM:SS  <child suffix> <title, truncated to the column>  <event>`
Log:   `YYYY-MM-DD HH:MM:SS <bd id> <event>`

Run-level lines have no Ticket; the panel's Ticket column reads `harness`. Pane locations are always named, as `(pane 2-1)`, on started, resumed, retrying, stuck, waiting and blocked lines.

## What shows

- Every Ticket event shows on the panel, except `prompted` (log only).
- Run-level errors show: state not saved, bd list failed, bd ready failed, Epic done, stopped.
- Housekeeping stays in the log only: dropped a leftover pane, merged but not closed (will retry), scratch left in the run directory, prompted, waiting for the result file, an answer that came after its session moved on (dropped your park: that session has moved on), a Stage that could not be resumed (not resumed: `err`, starting it fresh), a Judgment that could not be had (no Judgment: `err`, the key never in it), a bd comment that could not be added for a question asked while Away (no bd comment: `err`).
- A Judgment below the floor logs its judged line only: its scores show in the Wake's Question, and a panel line would close that Question.

## Vocabulary

| Moment | Wording |
|---|---|
| worktree created | branch `b` created |
| Stage started | implement started: claude (pane 2-1) · review 1 started: codex (pane 2-2) · with a model and effort set in .harness/config.json: implement started: claude opus/high (pane 2-1) · on a split, a plan model other than Implement's: implement started: claude claude-fable-5-1→claude-opus-5-5/high (pane 2-1) |
| Stage resumed | *a Stage /continue finds with its pane gone, by its saved session id on an unchanged App:* implement resumed: claude (pane 2-1) · *log only, when it cannot be:* implement not resumed: its App is now claude, starting it fresh |
| Stage prompted | *log only:* implement prompted, waiting for implement.md |
| trust dialog | waiting: claude does not trust `dir` yet, open it there once and accept (pane 2-1) |
| trust accepted | claude trusts `dir` now, carrying on |
| Implement done | implemented |
| Review done | review N found K findings |
| Debate done | debate N settled: K to fix, J skipped |
| Fix done | fix N done |
| PR opened | PR #12 opened after N rounds *(log line adds the url)* |
| dependents now wait on a merge | *on each open dependent:* waiting for PR #12 to merge (Ticket 5) |
| PR conflicts | PR #12 conflicts with main, /address resolves it |
| merged | merged, Ticket closed |
| PR closed unmerged | parked: PR #12 closed without merging |
| parked | parked: `reason` |
| Wake | stuck in fix 1: `reason` (pane 2-1) |
| blocked session | waiting at a prompt in fix 1 (pane 2-1) |
| Judgment, line 1 | judged: nudge to write the result 0.84, nudge to carry on 0.07, retry 0.05, park 0.03, wait 0.01 *(the actions offered, highest score first: a spent nudge or retry is left out, and wait after a timeout, for a dead session or after three waits)* |
| Judgment, line 2 | nudged: write the result file · nudged: carry on, the Ticket is the spec · retrying fix 1 with a fresh session (pane 2-3) · parked: `reason` · waiting: still working (pane 2-2) |
| below the floor, or no TypeSafe | asking you: stuck in fix 1 *(below the floor the judged line goes to the log alone)* |
| only park left (nudge and retry spent) | parked: fix 1 `reason` again after a retry *(no Judgment asked)* |
| retry command | retrying fix 1 with a fresh session (pane 2-3) |
| address | addressed PR #12 · address failed: `err` · address gave up: `err` · address refused: no open PR · address refused: not an Epic run |
| retry or park refused | ignored: not waiting on a Wake · refused: not a Ticket of this run |
| Away | *(harness)* away: on, a Stage's question parks its Ticket · away: off *(/away again, or /continue @ticket)* |
| /continue @ticket refused | *(harness)* refused: Ticket 5 is not parked · *(on the Ticket, in a single-Ticket run)* continue refused: not an Epic run |
| Shell refuses a command | *(harness)* refused: a run is live, /stop-work first · refused: a run is stopping · refused: no run is live, /start-epic or /continue starts one · refused: no saved Ticket to continue |
| /config saved during a run | *(harness)* config: Review codex → codex gpt-6-sol/high · config: Review if limited none → claude sonnet *(the row, then what it was and what it is, as the started line names them; the Stages that start after it use it)* · config: TypeSafe off · config: plan floor 0.60 *(the next Judgment reads it)* |
| Epic done | *(harness)* Epic done, every Ticket closed |
| stopped | *(harness)* stopped, panes left running, /continue resumes *(once every Ticket thread has left; the status row reads STOPPING until then)* |
| errors | *(harness)* state not saved: `err` · bd list failed: `err` · bd ready failed: `err` |

## Limited

Decided on the map tickets "Limited" (harness-0sx.8) and "Apps per Stage" (harness-0sx.14). Before any Wake and any blocked Question, the pane's last 20 lines are matched against the App's limit patterns (the App table's `limits`); a match whose reset is still ahead is a usage limit, never a Wake or a Judgment. The App holds until the reset + 2 minutes: no Stage starts on it, and its panes are left alone.

| Moment | Wording |
|---|---|
| a Stage's session hits a limit | claude session limit until 3:45pm: implement holds (pane 2-1) · codex usage limit until 3:05pm: review 1 holds (pane 2-2) |
| a Stage about to start on an App at its limit | *log only:* review 1 holds: codex limited until 3:05pm |
| the reset + 2 minutes | claude session limit over: implement carries on (pane 2-1) *(a pane still idle with no result is sent `continue` first)* |
| a long limit (a reset more than a day away, or Claude's options menu) | *(harness)* claude weekly limit until Mon 12:00am: sessions saved, panes closed, /continue after the reset *(the run ends; no "stopped" line follows)* |
| a session that would not take the continue | Wake reason: never took the continue |
| the Review's App at a short limit, once for the run *(Ticket 15)* | asking you: codex limited until 3:05pm: how do Reviews go until then? *(options: wait for the reset · review with claude opus, when review_if_limited is set · open the PR unreviewed; every other Ticket reaching Review holds, log only, until the answer, which stands until the reset)* |
| the answer | you answered: wait for the reset *(the Review holds as any Stage)* · you answered: review with claude opus *(then: review 1 started: claude opus (pane 2-3))* · you answered: open the PR unreviewed |
| a Review skipped, the PR to open unreviewed | review 1 and debate 1 skipped: codex was limited until 3:05pm *(the last Fix gets the Input Unreviewed: codex was limited until 3:05pm)* |
| a Debate side's App at its limit | *no line:* the Moderator's Inputs carry Side B: limited until 3:05pm |

## Wake reasons

session reported failure · went idle without a result · wrote a result file whose first line is not STATUS: · timed out after 30m · session died · finished without a PR link · never took the Stage skill · never took the nudge · never took the continue · never took your answer · wrote STATUS: plan and no plan.md · has no plan hook · never took the answer to its plan · left plan mode before your feedback · feedback not sent: `why` · the cursor never reached Yes, clear context · changed the worktree before its plan was approved

The last six are plan failures: a Question for the user, no Judgment asked.

The Judgment is a TypeSafe Choice over the Ticket, the Wake reason, the result file and the pane tail; its actions, prompts and floor (0.7) were settled on the map ticket "Prototype: the Wake Judgment" (harness-7bj.13), prototype in docs/design/judgment-prototype. config.json's `wake_floor` replaces the floor; one that is not a number from 0 to 1 logs `wake_floor is not a number from 0 to 1: the Judgment is not acted on`, and the Wake is a Question.

## Plans

Decided on the map ticket "Plan approval: a Judgment approves, the Shell asks when unsure" (harness-7bj.9). Implement starts in plan mode; a hook copies the plan into the run directory as `plan.md`, and the Judgment is three TypeSafe Nouls over the plan, the Ticket and any earlier feedback: covers, in_scope and asks (harness-cq7, prototype in docs/design/plan-judgment-prototype). Approved when covers and in_scope reach the floor (0.65, or config.json's `plan_floor`) and asks is below 0.5. A bad `plan_floor` logs `plan_floor is not a number from 0 to 1: the Judgment is not acted on`, and the plan is a Question.

Off claude (harness-7nq.12) the Plan takes two steps: the session writes `plan.md` and a Stage result `STATUS: plan`, and waits. The lines are the same; feedback goes into the pane as a prompt, and approval prompts `implement the approved plan`. A worktree the session changed before approval (HEAD moved, or the tree changed) is a plan failure.

| Moment | Wording |
|---|---|
| plan ready | plan ready in implement (pane 2-1) |
| Judgment, line 1 | judged: plan covers the Ticket 0.91, stays in scope 0.88, asks nothing 0.95 · judged: plan misses an acceptance criterion 0.81, stays in scope 0.90, asks nothing 0.93 *(each score a yes as its score, a no as one minus it: goes beyond the Ticket, asks you a question; a yes short of the floor marked: covers the Ticket 0.62 < 0.65)* |
| Judgment, line 2 | plan approved · asking you: plan ready in implement (pane 2-1) |
| a Noul short of the floor, a question asked, or no TypeSafe | asking you: plan ready in implement (pane 2-1) |
| user feedback delivered | plan sent back with your feedback |
| feedback not delivered, no Enter sent | feedback not sent: the plan dialog is not on screen (pane 2-1) · feedback not sent: the cursor never reached Tell Claude what to change (pane 2-1) *(then the plan Question again, the feedback kept to resend)* |
| a plan failure | stuck in implement: `reason` (pane 2-1) *(a Question for you, no Judgment asked: open the pane, park, retry, resend the feedback)* |
| split session switched model (its PostModelSwitch hook) | *log only:* implement switched to `model` |
| blocked at another prompt, or at the plan dialog with no new plan.md | waiting at a prompt in implement (pane 2-1) *(an ordinary blocked session)* |

## Stage questions

Decided on the map ticket "Delegate skills" (harness-0sx.12, Asks). A Stage writes `STATUS: question`, then the question, then its options, one `- ` line each as the last lines, and waits in its session. It is never a Wake and never judged, and no deadline runs while it waits; once the answer goes in, the Stage's deadline starts over.

| Moment | Wording |
|---|---|
| a Stage asks | question in implement (pane 2-1) *(a Question: the Stage's options, an answer of your own, open the pane, park)* |
| answer sent into the pane | sent your answer |
| answered in the pane instead | carrying on |
| asked while Away | parked: asked you while away *(a bd comment on the Ticket asks for a manual resume, /continue @ticket; the pane stays open; an Address question, its PR open, waits as a Question instead)* |

## Questions and answers

Decided on the map ticket "The Shell's Question panel" (harness-7bj.7). A Question is a form above the input line; answering it logs two lines, the first naming the user, the second the outcome in the Judgment's own words.

| Moment | Wording |
|---|---|
| Question raised | asking you: stuck in fix 1 · asking you: waiting at a prompt in fix 1 (pane 2-1) · asking you: plan ready in implement (pane 2-1) · asking you: question in implement (pane 2-1) |
| answer, line 1 | you answered: nudge · retry · park · wait · your prompt · approve · feedback · I answered it · the Stage's option picked · your answer |
| answer, line 2 | the Judgment's line 2 wording where it has one: nudged: write the result file · retrying fix 1 with a fresh session (pane 2-3) · parked: `reason` |
| answer, line 2, no Judgment equivalent | nudged with your prompt · plan approved · plan sent back with your feedback · sent your answer · carrying on |
| blocked session cleared in the pane | carrying on |
| open the pane, Esc, /questions | nothing |
| /park on a running Ticket | parked: by you at fix 1 |
| command refused by a waiting Question | refused: Ticket 5 has a Question waiting |
| confirmations, /continue checklist | nothing beyond the command's own lines (stopped, started) |
