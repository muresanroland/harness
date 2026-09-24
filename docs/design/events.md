# Events: the RECENT panel and the log

Every Orchestrator event is one plain-language line, the same words on the Shell's RECENT panel and in `.harness/orchestrator.log`. There is no machine-readable line; `.harness/state.json` is the machine record. Decided on the map ticket "Human-readable event vocabulary for the RECENT panel and the log" (harness-7bj.6).

## Line shape

Panel: `HH:MM:SS  <child suffix> <title, truncated to the column>  <event>`
Log:   `YYYY-MM-DD HH:MM:SS <bd id> <event>`

Run-level lines have no Ticket; the panel's Ticket column reads `harness`. Pane locations are always named, as `(pane 2-1)`, on started, retrying, stuck, waiting and blocked lines.

## What shows

- Every Ticket event shows on the panel, except `prompted` (log only).
- Run-level errors show: state not saved, bd list failed, bd ready failed, Epic done, stopped.
- Housekeeping stays in the log only: dropped a leftover pane, merged but not closed (will retry), scratch left in the run directory, prompted, waiting for the result file, an answer that came after its session moved on (dropped your park: that session has moved on), a Judgment that could not be had (no Judgment: `err`, the key never in it).
- A Judgment below the floor logs its judged line only: its scores show in the Wake's Question, and a panel line would close that Question.

## Vocabulary

| Moment | Wording |
|---|---|
| worktree created | branch `b` created |
| Stage started | implement started: claude (pane 2-1) · review 1 started: codex (pane 2-2) · with a model and effort set in .harness/config.json: implement started: claude opus/high (pane 2-1) · on a split, a plan model other than Implement's: implement started: claude fable→opus/high (pane 2-1) |
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
| Shell refuses a command | *(harness)* refused: a run is live, /stop-work first · refused: a run is stopping · refused: no run is live, /start-epic or /continue starts one · refused: no saved Ticket to continue |
| Epic done | *(harness)* Epic done, every Ticket closed |
| stopped | *(harness)* stopped, panes left running, /continue resumes *(once every Ticket thread has left; the status row reads STOPPING until then)* |
| errors | *(harness)* state not saved: `err` · bd list failed: `err` · bd ready failed: `err` |

## Wake reasons

session reported failure · went idle without a result · wrote a result file whose first line is not STATUS: · timed out after 30m · session died · finished without a PR link · never took the Stage skill · never took the nudge · has no plan hook · never took the answer to its plan · left plan mode before your feedback · feedback not sent: `why` · the cursor never reached Yes, clear context

The last five are plan failures: a Question for the user, no Judgment asked.

The Judgment is a TypeSafe Choice over the Ticket, the Wake reason, the result file and the pane tail; its actions, prompts and floor (0.7) were settled on the map ticket "Prototype: the Wake Judgment" (harness-7bj.13), prototype in docs/design/judgment-prototype.

## Plans

Decided on the map ticket "Plan approval: a Judgment approves, the Shell asks when unsure" (harness-7bj.9). Implement starts in plan mode; a hook copies the plan into the run directory as `plan.md`, and the Judgment is a TypeSafe Noul over the plan, the Ticket and any earlier feedback, floor 0.8.

| Moment | Wording |
|---|---|
| plan ready | plan ready in implement (pane 2-1) |
| Judgment, line 1 | judged: plan follows the Ticket 0.93 · judged: plan strays from the Ticket 0.88 |
| Judgment, line 2 | plan approved · asking you: plan ready in implement (pane 2-1) |
| below the floor, or no TypeSafe | asking you: plan ready in implement (pane 2-1) |
| user feedback delivered | plan sent back with your feedback |
| feedback not delivered, no Enter sent | feedback not sent: the plan dialog is not on screen (pane 2-1) · feedback not sent: the cursor never reached Tell Claude what to change (pane 2-1) *(then the plan Question again, the feedback kept to resend)* |
| a plan failure | stuck in implement: `reason` (pane 2-1) *(a Question for you, no Judgment asked: open the pane, park, retry, resend the feedback)* |
| split session switched model (its PostModelSwitch hook) | *log only:* implement switched to `model` |
| blocked at another prompt, or at the plan dialog with no new plan.md | waiting at a prompt in implement (pane 2-1) *(an ordinary blocked session)* |

## Questions and answers

Decided on the map ticket "The Shell's Question panel" (harness-7bj.7). A Question is a form above the input line; answering it logs two lines, the first naming the user, the second the outcome in the Judgment's own words.

| Moment | Wording |
|---|---|
| Question raised | asking you: stuck in fix 1 · asking you: waiting at a prompt in fix 1 (pane 2-1) · asking you: plan ready in implement (pane 2-1) |
| answer, line 1 | you answered: nudge · retry · park · wait · your prompt · approve · feedback · I answered it |
| answer, line 2 | the Judgment's line 2 wording where it has one: nudged: write the result file · retrying fix 1 with a fresh session (pane 2-3) · parked: `reason` |
| answer, line 2, no Judgment equivalent | nudged with your prompt · plan approved · plan sent back with your feedback · carrying on |
| blocked session cleared in the pane | carrying on |
| open the pane, Esc, /questions | nothing |
| /park on a running Ticket | parked: by you at fix 1 |
| command refused by a waiting Question | refused: Ticket 5 has a Question waiting |
| confirmations, /continue checklist | nothing beyond the command's own lines (stopped, started) |
