# harness-7nq.14 — Limited: a usage limit is not a Wake; hold the App, resume at the reset

## Context
Today a claude/codex session that hits its provider usage limit sits idle with no result, so the Orchestrator Wakes it (Judgment, nudges, retries) — and a nudge prompt would even cancel Claude's own auto-continue. The Ticket asks: detect the limit from the pane's last lines, hold (never Wake/Judge), hold every other Stage on that App, resume at reset + 2 min; a long limit (>24h or Claude's options menu) saves ids, closes panes/tabs and ends the run for /continue later; state.json keeps limits; the Shell shows an amber box and "limited until" rows.

## Changes

**Cargo.toml**: add `regex = "1"` (allowed crate, already in Cargo.lock transitively); chrono `features = ["serde"]` (State stores `DateTime<Local>`).

**src/orchestrator/app.rs**: App row gets `limits: &'static [&'static str]` — regexes, named groups `what` (optional) and `reset` (optional):
- claude: `You['’]ve hit your (?P<what>.*?limit) · resets (?P<reset>.+)`, `Usage limit reached · continuing automatically at (?P<reset>.+?)(?: · |$)`
- codex: `You['’]ve hit your (?P<what>usage limit)\..*?(?:try again at (?P<reset>.+?)\.|Try again later\.)`

**src/orchestrator/limit.rs (new)** + `limit_test.rs`:
- `Limit { app, what, reset, long, line }`; `find(app, tail, now) -> Option<Limit>`: last 20 lines, newest first; no reset group → now + 1h; unparseable or past reset → not a limit; `long` = reset > now + 24h or `What do you want to do?` in those lines.
- `parse_reset(text, now)`: one regex over `[weekday] [Mon D[st|nd|rd|th][,] [YYYY]] H[:MM] am|pm`, trailing `(Zone)` ignored (machine's zone). Time only → next local occurrence; weekday → next such day; date without year → this year (next year if >180 days back).
- `until(reset, now)`: `3:45pm` same day, else `Mon 12:00am`. `holds(reset, now)`: now < reset + GRACE (2 min).
- Orchestrator methods:
  - `limited_until(app)`: the App's saved reset while it holds; a passed one is removed from State.
  - `wait_limit(ticket, label, app) -> Option<Held>`: while held, sets `ts.limited = app` (log-only "review 1 holds: codex limited until 3:05pm"), sleeps; /park → Park, /stop-work → Stopped (keeps `limited` for the saved row), free → clears and None. No deadline checked.
  - `limit_shown(ts, st, pane, tail)`: App from `ts.sessions[st].app`; skips the line this pane was last resumed from (in-memory `resumed: Mutex<BTreeMap<pane, line>>`, so an old time-only line is not re-read as tomorrow's limit).
  - `limited(ticket, st, label, pane, file, want, limit) -> Held`: records limit in State (the later reset wins). Long → `closed` flag + stop, run-level line `claude weekly limit until Mon 12:00am: sessions saved, panes closed, /continue after the reset`, Held::Stopped. Short → `claude session limit until 3:45pm: implement holds (pane 1-1)`, `wait_limit`, then if the pane is idle/done with no result sends `continue`, says `claude session limit over: implement carries on (pane 1-1)`, returns `hold(..., armed=true)` (fresh deadline and settle).

**src/orchestrator/stage.rs**:
- `Config.clock: Arc<dyn Fn() -> DateTime<Local> + Send + Sync>` (Shell: `Local::now`; tests settable).
- `Held::Limited(Limit)`.
- `run_stage` Wake handler: read the tail (moved up, reused for the Wake), and before `offered`/Judgment: `Held::Limited` or `limit_shown` → `held = self.limited(...)`, continue. So idle-without-result, timeout, anything that would Wake, checks first; no nudge/wait/retry spent.
- `wait_unblocked`: after plan_ready, before the blocked Question, read 20 lines by pane; a limit → `Some(Held::Limited)` (Claude's options menu → long).
- `attempt`: `wait_limit(row.app)` before `fresh_pane` (covers first starts and retries). `run_stage`: `wait_limit(session.app)` before resume-by-id.
- `Orchestrator.closed: AtomicBool`; small `change_state(f)` that `update` reuses for State-level saves.

**src/orchestrator/state.rs**: `State.limits: BTreeMap<String, DateTime<Local>>` and `TicketState.limited: String`, both skipped when empty (Go-state byte-for-byte test stays green).

**src/orchestrator/pipeline.rs / scheduler.rs**: `run_ticket` and `address` on `Stopped` with `closed` set: close the Ticket tab, clear `tab` and `panes` (sessions kept, so /continue resumes by id).

**src/orchestrator/world.rs**: `Inner.tails` (pane → text) answered by `herdr agent read` (name resolved as `agent get` does); `restarted` copies the test clock.

**src/shell.rs**: `poll` skips "stopped, panes left running" when `closed` (the long-limit line already said what happened).

**src/shell/draw.rs**: amber (ORANGE) box titled LIMITED below MERGE TO UNBLOCK, one line per held App: `CLAUDE LIMITED until 3:45pm · resumes by itself` (idle: `· /continue after the reset`). TICKETS: a Ticket whose `limited` App holds reads `limited until 3:45pm` as its stage; its status stays Working (no status-row count).

**docs/design/events.md**: the four new lines.

## Decisions (Ticket left open)
- The Review is held like any Stage for now: it equals Ticket 15's "wait" answer, which Ticket 15 puts a Question in front of. Only way the codex AC is reachable (codex runs the Review alone). Debate sides untouched (Ticket 15).
- The App is held until reset + 2 min for new Stages too (one rule).
- Long limit closes the tabs of Tickets running in this run; Parked Tickets' panes stay.
- /retry is not consumed during a hold (only /park, /stop-work); it drops at the next Wake as today.
- Menu marker is one constant, not an App field (only claude has one).

## Tests (TDD, fake world + unit)
- limit_test (unit): claude both forms, codex time/date/"later" (U+2019), zone ignored, time-only → next occurrence, weekday, dated past → None, line older than 20 → None, >24h and menu → long, `until` forms.
- limit_test (world): claude on Implement and codex on Review idle with limit text → hit line, no "stuck", no TypeSafe request, held, State limits set; with hx-1 held, hx-2's Implement never starts while hx-3's Review starts on codex; old line above 20 newer lines, and a past dated reset, still Wake; clock at reset+1m no continue, at reset+2m `herdr agent prompt <pane> continue` and the PR opens; timeout 5ms never fires during the hold; long (weekly reset, and blocked + menu): long line, tab closed, run ends, ids kept, limits saved; restarted before reset holds (limited row), after the clock passes resumes by `--resume <id>` + continue.
- state_test: limits and limited round-trip.
- shell_test: TestBackend render of the LIMITED box (ORANGE, text) and a held Ticket's row (`limited until 3:45pm`, WORKING, counted working); long limit in the Shell ends the run without "panes left running".

## Verification
`cargo check`, `cargo clippy`, `cargo test limit`, `cargo test state`, `cargo test shell`, then full `cargo test`; /code-review against main; commit; result file.
