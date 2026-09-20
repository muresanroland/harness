---
name: start-work
description: Launch the Harness Pipeline for a beads Epic from the Main session and handle its [harness] lines and Wakes. Use when the user runs /start-work <epic-id> or asks to drive an Epic's Tickets to pull requests.
---

# Start work on an Epic

You are the Main session. A deterministic Orchestrator does the work; you launch it, relay its progress, and make the few judgment calls it cannot.

## Launch

1. Run `harness start <epic-id>` from the Target repo's root, in this pane. Add `--max N` only if the user asked for a different number of Tickets at once (default 3).
2. If it prints `preflight:` lines, show them to the user and stop; each line names one missing prerequisite. `harness init` installs the Stage skills.
3. Otherwise it detaches and prints its pid. It logs to `.harness/orchestrator.log`. Tell the user it is running, then wait. Do not poll: every event arrives here as a line.

Starting again after a kill, a `/clear`, or `harness stop` resumes the run from `.harness/state.json`. `harness status` prints that state.

## The [harness] line protocol

Lines beginning with `[harness]` are typed into this pane by the Orchestrator, not by the user. Locations are `<tab>-<pane>` as herdr shows them.

| Line | Meaning | You |
|---|---|---|
| `<ticket> <stage> started -> 2-1` | A Stage session started in that pane | nothing |
| `<ticket> implement done`, `review N done: ...`, `debate N done: ...`, `fix N done` | A Stage finished | nothing |
| `<ticket> pr open after N round(s): <url>` | The Ticket's Pipeline ended | tell the user the PR is ready to merge |
| `<ticket> merged: ...` | The user merged it; the Ticket is closed and dependents unblock | nothing |
| `<ticket> pr conflicts with main: <url>` | Merged work conflicts with this open PR | tell the user; `harness address <ticket>` resolves it, only on their say |
| `<ticket> parked: <reason>` | The Ticket left the Pipeline | tell the user why |
| `WAKE <ticket> <stage> <reason> at <tab>-<pane>` | A Stage cannot advance by rule | follow the Wake rules below |
| `epic <id> done: ...` | Every Ticket is closed | tell the user |
| `stopped; ...` | The Orchestrator exited on `harness stop` | nothing |

Answer progress lines with at most one short sentence. Never start, prompt, or close Stage panes on your own initiative.

## Wake rules

A Wake holds only that Ticket; the others keep running. For each Wake:

1. **`blocked`**: the session is waiting on a permission prompt or a question. Never answer a permission prompt. Tell the user which pane needs them (`<tab>-<pane>`) and do nothing else; the Stage continues by itself once they answer.
2. **Any other reason** (`reported STATUS: failed`, `went idle without a done result`, `timed out ...`, `pane died`, `wrote a done result without a 'PR:' line`): read the evidence first: `herdr agent read <pane-id> --source recent-unwrapped --lines 120` (find the pane id with `herdr pane list`; if the pane died there is nothing to read) and the Stage's result file in `.harness/runs/<ticket>/`.
3. Then do exactly one of:
   - **One follow-up prompt** when the session is alive and plainly close: it finished but forgot the result file, or stopped to ask something you can answer from the Ticket. Send it with `herdr agent prompt <pane-id> "<text>"`. If it then writes `STATUS: done`, the Orchestrator advances by itself.
   - **One retry** when the session is dead, timed out, or went wrong in a way a fresh session could avoid: `harness retry <ticket>`.
   - **Park** when neither fits, or the one thing you tried already failed: `harness park <ticket>`, then tell the user what you saw and what you tried.
4. One follow-up or one retry per Wake, never both, never twice. A Stage that fails again after a retry is parked by the Orchestrator.

`harness retry <ticket>` also returns a Parked Ticket to the Pipeline once the user has dealt with the cause.

## Commands

- `harness status`: state of every Ticket.
- `harness retry <ticket>` / `harness park <ticket>`: as above.
- `harness address <ticket>`: only when the user asks; a session acts on the PR's review comments and conflicts.
- `harness stop`: only when the user asks; stops scheduling and leaves live panes alone.
