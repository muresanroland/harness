# PROTOTYPE: the Wake Judgment

Throwaway. Answers the map ticket "Prototype: the Wake Judgment: state, criteria,
canned prompts, confidence floor" (harness-7bj.13). `judge.py` builds the TypeSafe
request for a Wake from a case directory and prints the scores; `cases/` holds one
Wake from the last live run (`still-working`, the Debate that looked idle twice and
finished on its own) plus invented ones; `answers/` holds what Jev answered.

```bash
TYPESAFE_API_KEY=... python3 judge.py            # every case
TYPESAFE_API_KEY=... python3 judge.py crashed    # one
```

## What was settled

**The request.** One Choice, `action`, over named state:

| field | from |
|---|---|
| `ticket` (id, title, spec) | `bd show` |
| `stage`, `round`, `why_woken` | the Orchestrator's Wake reason, verbatim |
| `already_nudged`, `already_retried` | Ticket state |
| `result_file` (path, content or `"missing"`) | the run directory |
| `pane_tail` | `herdr agent read <agent> --source recent-unwrapped --lines 120` |

**The criteria**, offered only while unspent:

- `nudge_write_result`: did the work, no result file (or STATUS not first). Canned prompt asks for the file.
- `nudge_proceed`: stopped at a question or an approval nobody will give. Canned prompt: the Ticket is the spec, decide and carry on.
- `retry`: crashed, API error, out of context, looping. Fresh session.
- `park`: a person must look: impossible Ticket, tool needs login, a fresh session would hit the same wall.
- `wait`: still working, a tool call running, no prompt at the end. Not offered after a timeout.

Nudge is spent after one use per session (a retry starts a new session and re-arms it); retry once per Stage, as today. Wait is offered while fewer than three waits have been taken. When only park is left, park by rule without asking (the `crashed-after-retry` case).

**Two diagnostic Nouls** (`finished`, `asking`) were tried alongside; they agreed with the Choice and changed no decision, so the design keeps the one Choice.

**The floor: 0.7.** Clear cases scored 0.81 to 1.00; the one muddled case (`bad-result` before its reason was sharpened) scored 0.60 with the wrong action; the one genuinely ambiguous case (`asked-after-nudge`) scored 0.25. 0.6 would have let the muddled one through.

**Sharpen the reason, not the model.** The result file with a `# heading` above STATUS was misjudged as "asking" under the generic reason "went idle without a result" and judged right (0.82) once the reason said "wrote a result file whose first line is not STATUS:". Whatever the code can tell by rule goes into `why_woken`.

**After a nudge or a wait.** The hold re-arms the same idle check: the Ticket Wakes again when the session is idle without a result (or at the Stage deadline). The second Wake goes to the Judgment with the spent actions removed. A wait Wakes again after ten minutes.

## Results

| case | expect | judged | confidence |
|---|---|---|---|
| forgot-result | nudge_write_result | nudge_write_result 0.99 | 0.97 |
| bad-result (STATUS not first) | nudge_write_result | nudge_write_result 0.86 | 0.82 |
| asked-question | nudge_proceed | nudge_proceed 1.00 | 1.00 |
| crashed (API 500, gave up) | retry | retry 0.98 | 0.97 |
| looped (codex, same edit x6) | retry | retry 0.86 | 0.81 |
| still-working (live run) | wait | wait 0.88 | 0.86 |
| failed-impossible | park | park 0.98 | 0.97 |
| failed-transient (push timed out) | retry | retry 0.89 | 0.86 |
| asked-after-nudge | unclear | park 0.50, retry 0.49 | 0.25, asks you |
| crashed-after-retry | park by rule | not asked | |

Each request cost about 1.5K input tokens.
