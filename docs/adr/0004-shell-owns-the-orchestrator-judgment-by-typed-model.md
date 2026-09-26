# The Shell owns the Orchestrator, and judgment goes to a typed model

`orqa` alone opens the Shell, a full-terminal screen that runs the Orchestrator inside its own process and takes slash commands; there is no detached run, no attach command, and no Main session. The judgment ADR 0001 gave the Main session (a Stage that cannot advance by rule: nudge, retry, or park) is a Choice put to TypeSafe over the Wake's evidence, logged on the Shell with a score per action, and acted on above a confidence floor; below it, or when TypeSafe is unreachable, the Shell asks the user. A Claude Main session was rejected because in practice it was never run: launches came from a plain terminal, Wakes went to the log, and the user retried or parked by hand, which a typed judgment does cheaper and faster, and a screen Question does with less to read. Blocked sessions (permission prompts, trust dialogs) stay with the user, as before.

## Consequences

- ADR 0001's "judgment never in Orchestrator code" narrows: the Orchestrator makes bounded, logged judgments from a fixed set of actions through a typed model, and never composes text. Follow-up prompts are canned per outcome.
- The start-work skill and the `[harness]` line protocol go with the Main session. Every event is a line in the Shell's RECENT panel and the log.
- The CLI is `orqa` and `orqa init`. start, status, stop, retry, park and address become slash commands in the Shell; `orqa init` stays a command because it is a stdin prompt, not a run.
- Without `TYPESAFE_API_KEY` every Wake is a Question; the run still works, with the user as the judge.
- Exiting the Shell ends scheduling, saves state and releases the lock, and leaves Stage panes running in herdr; `/continue` resumes them.
