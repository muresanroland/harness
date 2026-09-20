# Deterministic Orchestrator, with the Main session woken only for judgment

The Pipeline's transitions ("this Stage's result file says done, start the next Stage") involve no judgment, so a compiled, deterministic Orchestrator owns Ticket state, pane placement and Stage transitions, and LLMs work only inside the Stages. An LLM-driven loop (a skill telling the Main session to poll herdr and spawn sessions) was rejected: it spends tokens while waiting, drifts over runs that last days, and dies on `/clear` or compaction. The Orchestrator sends a Wake to the Main session only when a Stage cannot advance by rule (blocked, idle without a `done` result, failed, timed out, pane died); the Main session may nudge or retry once, then parks the Ticket, and never answers a permission prompt.

## Consequences

- The Orchestrator is an independent process; it survives the Main session being cleared or restarted, and reports by sending lines into the Main session's pane through herdr.
- Anything that needs judgment must live in a Stage skill or in the Main session's Wake handling, never in Orchestrator code.
