# Events: the RECENT panel and the log

Every Orchestrator event is one plain-language line, the same words on the Shell's RECENT panel and in `.harness/orchestrator.log`. There is no machine-readable line; `.harness/state.json` is the machine record. Decided on the map ticket "Human-readable event vocabulary for the RECENT panel and the log" (harness-7bj.6).

## Line shape

Panel: `HH:MM:SS  <child suffix> <title, truncated to the column>  <event>`
Log:   `YYYY-MM-DD HH:MM:SS <bd id> <event>`

Run-level lines have no Ticket; the panel's Ticket column reads `harness`. Pane locations are always named, as `(pane 2-1)`, on started, retrying, stuck, waiting and blocked lines.

## What shows

- Every Ticket event shows on the panel, except `prompted` (log only).
- Run-level errors show: state not saved, bd list failed, Epic done, stopped.
- Housekeeping stays in the log only: dropped a leftover pane, merged but not closed (will retry), scratch left in the run directory, prompted, waiting for the result file.

## Vocabulary

| Moment | Wording |
|---|---|
| worktree created | branch `b` created |
| Stage started | implement started: claude (pane 2-1) · review 1 started: codex (pane 2-2) |
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
| Judgment, line 1 | judged: nudge 0.84, retry 0.10, park 0.06 *(only the actions offered: a spent nudge or retry is left out, wait after a timeout)* |
| Judgment, line 2 | nudged: write the result file · nudged: carry on, the Ticket is the spec · retrying fix 1 with a fresh session (pane 2-3) · parked: `reason` · waiting: still working (pane 2-2) |
| below the floor, or no TypeSafe | asking you: stuck in fix 1 |
| only park left (nudge and retry spent) | parked: fix 1 `reason` again after a retry *(no Judgment asked)* |
| retry command | retrying fix 1 with a fresh session (pane 2-3) |
| address | addressed PR #12 · address failed: `err` · address gave up: `err` · address refused: no open PR |
| retry or park refused | ignored: not waiting on a Wake · refused: not a Ticket of this run |
| Epic done | *(harness)* Epic done, every Ticket closed |
| stopped | *(harness)* stopped, panes left running, /continue resumes |
| errors | *(harness)* state not saved: `err` · bd list failed: `err` |

## Wake reasons

session reported failure · went idle without a result · wrote a result file whose first line is not STATUS: · timed out after 30m · session died · finished without a PR link · never took the Stage skill

The Judgment is a TypeSafe Choice over the Ticket, the Wake reason, the result file and the pane tail; its actions, prompts and floor (0.7) were settled on the map ticket "Prototype: the Wake Judgment" (harness-7bj.13), prototype in docs/design/judgment-prototype.

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
