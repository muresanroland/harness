---
name: start-work
description: Launch the Harness Pipeline for a beads Epic from the Main session and handle its event lines and Wakes. Use when the user runs /start-work <epic-id> or asks to drive an Epic's Tickets to pull requests.
---

# Start work on an Epic

You are the Main session. A deterministic Orchestrator does the work; you launch it, relay its progress, and make the few judgment calls it cannot.

## Launch

1. Run `harness start <epic-id> --detach` from the Target repo's root, in this pane. Add `--max N` only if the user asked for a different number of Tickets at once (default 3). `--detach` is what gives you your prompt back; without it the run holds the pane and prints its progress there, which is what a person watching it wants.
2. If it prints `preflight:` lines, show them to the user and stop; each line names one missing prerequisite. `harness init` installs the Stage skills.
3. Otherwise it prints its pid and logs to `.harness/orchestrator.log`. Tell the user it is running, then wait. Do not poll: every event arrives here as a line.

Starting again after a kill, a `/clear`, or `harness stop` resumes the run from `.harness/state.json`. `harness status` prints that state.

Every event the Orchestrator has to say arrives in this pane as one plain-language line, `<ticket> <event>`, the same line it writes to `.harness/orchestrator.log`; a run-level line has no ticket. Pane locations are named `(pane <tab>-<pane>)` as herdr shows them. Answer progress lines with at most one short sentence. Never start, prompt, or close Stage panes on your own initiative.

## Wake rules

A Wake holds only that Ticket; the others keep running. For each Wake:

1. **`waiting at a prompt in <stage>`**: the session is waiting on a permission prompt or a question. Never answer a permission prompt. Tell the user which pane needs them (`<tab>-<pane>`) and do nothing else; the Stage continues by itself once they answer.
   **`never took the Stage skill`**: the session never received the Stage skill, usually because it is sitting at a dialog. Same answer: name the pane, let the user deal with it, then `harness retry <ticket>`.
2. **Any other `stuck in <stage>: <reason>`** (`session reported failure`, `went idle without a result`, `timed out ...`, `session died`, `finished without a PR link`): read the evidence first: `herdr agent read <pane-id> --source recent-unwrapped --lines 120` (find the pane id with `herdr pane list`; if the pane died there is nothing to read) and the Stage's result file in `.harness/runs/<ticket>/`.
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
- `harness stop`: only when the user asks; stops scheduling and leaves live panes alone. It reaches a run started either way, detached or in a pane; a run held in a pane also stops on Ctrl-C.
