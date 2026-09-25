# harness-7nq.20: init gets bd init, docs/agents, TypeSafe opt-in, herdr integrations; preflight checks row Apps

## Context
`harness init` today asks where skills go, installs them, asks the TypeSafe key, installs job defaults, preflights. Ticket adds: offer `bd init`; write missing beads docs/agents setup; TypeSafe as own [Y/n] opt-in kept in `.harness/config.json` (off = no Judgments, Moderator told `TypeSafe: off`); offer herdr integration install for App-table Apps; preflight fails when a config row's App not on PATH. Scope = ticket's AC only.

## Init order (src/cli.rs `init` arm)
`install_skills` (gate cancel still ends init) → new `setup::set_up(...)`, which runs in order:
1. `bd_init`: no `.beads` → `yes("Run bd init now?")`; yes → `tools.run(repo, ["bd","init"])` (says ok/failed); none → skip, preflight still fails.
2. `write_agent_docs`: anything missing → `yes("Write the beads docs/agents setup (issue tracker, triage labels, domain, Agent skills block)?")`; yes or non-interactive → write only missing ones.
3. `ask_typesafe` (replaces `ask_typesafe_key`) → returns on/off, writes config.json.
4. `install_defaults(…, typesafe)`: job defaults as now, plus `typesafe-ai` from `typesafe-ai/skills/skills/typesafe-ai` when on (same code path, same user-level keep rule).
5. `install_integrations`.
Then preflight as now.

## setup.rs changes
- Extract existing key-reading loop into `read_line(out, input, echo) -> io::Result<Option<String>>`: None at Ctrl-C/Ctrl-D or input end with nothing typed. Key prompt uses it with echo off (behaviour unchanged).
- `yes(out, input, tty, question) -> io::Result<Option<bool>>`: prints `init: <q> [Y/n] `, reads a line in raw mode with echo; empty or y… = yes, else no; None = nobody answered (EOF, Ctrl-C/D) = non-interactive. Line-based, not single-key, so a `y⏎` never leaks the ⏎ into the next question (the key prompt would read it as "no key").
- Non-interactive answers: bd init skip; docs write; TypeSafe off unless env key; integrations skip.
- docs/agents texts compiled in: `include_str!` of this repo's `docs/agents/{issue-tracker,triage-labels,domain}.md` (the models, one source). Agent skills block = const copied from AGENTS.md's section. Block present if CLAUDE.md or AGENTS.md has `## Agent skills`; else appended to CLAUDE.md if it exists, else AGENTS.md (created if needed).
- `ask_typesafe(repo, env_key, out, input, tty) -> io::Result<bool>`: env key → on, no question. Else ask [Y/n]; no/none → off; yes + key already stored → on without re-asking the key (decision); yes → ask key (echo off, as today), empty → off. Writes `app::set_typesafe(repo, on)`; key stays in `.harness/typesafe-key`.
- `install_integrations(repo, tools, out, input, tty)`: `herdr integration status` (text; herdr 0.9.1 CLI has no JSON form), for each `APPS` row find line `<name>: not installed|outdated … (<path>)`, keep those `which <name>` finds on PATH (through Tools). None → silent. Else list each with the path its install writes, ask once; yes → `herdr integration install <name>` per App; no/none → say those Stages cannot be resumed by id and /continue starts them fresh.
- `preflight`: for each config row (`app::ROWS`, readable ones only; an unreadable config stays the Orchestrator's refusal) check its App once with `which` → `"<Row> runs on <app>, which is not on PATH"` (e.g. "Review runs on codex, which is not on PATH").

## app.rs changes (owns config.json)
- Split file read out of `row` into `config(repo) -> Result<Value,String>`; make `row` pub(crate).
- `ROWS`: (key, label) for implement/Implement, review/Review, moderator/The Moderator, side_a/Debate side A, side_b/Debate side B, fix/Fix, address/Address.
- `typesafe(repo) -> bool`: `config.json` `"typesafe"` not `false` (missing = on, so existing setups keep behaviour).
- `set_typesafe(repo, on)`: read-modify-write, rows untouched; refuses a config that is not an object instead of clobbering.
- `debate_inputs`: TypeSafe off → add `("TypeSafe", "off")` → rendered `- TypeSafe: off`.

## Orchestrator (judgment.rs, stage.rs)
- `Orchestrator::typesafe_key(&self) -> &str`: `cfg.api_key`, or "" when `app::typesafe(repo)` is false, read per call so /config's later switch (Ticket 21) reaches the next call. `judge`, `judge_plan` and the Debate pane's `--env TYPESAFE_API_KEY` in `fresh_pane` use it: off → no request, Wake/Plan is a Question, Moderator gets no key.
- cli.rs preflight warning: off → `preflight: TypeSafe is off: every Wake and plan will be a Question`; else no key → existing warning.

## Skill / docs
- `skills/stage-moderate/SKILL.md` step 4: with **TypeSafe** `off` under Inputs, do not call TypeSafe: still-disputed Finding is **skip**, settled `disputed, no TypeSafe` (Fix Stage already lists skipped Findings and how settled in the PR's Verdict history).
- `docs/agents/issue-tracker.md`: drop "as on harness-7bj" (would dangle in other repos).
- README init bullets + cli USAGE line: new steps.

## Tests (test-first, /tdd)
- `src/cli/init_test.rs` (scripted input via `run`; `ok_tools` returns `Arc<Fake>` to read calls, and its clones also hold `skills/typesafe-ai`):
  - no `.beads`: answer yes → `bd init` called; non-interactive → no call, preflight names bd workspace.
  - docs/agents: own `domain.md` kept, other two written with compiled text; block into CLAUDE.md when present (AGENTS.md untouched), else AGENTS.md; rerun asks nothing.
  - TypeSafe table: yes + key → on, `typesafe-ai` in manifest, key file; yes + empty key → off, no skill; env key → on without the question; non-interactive no env → off.
  - herdr status reply with claude current and codex outdated → one question naming codex and its path; yes → `herdr integration install codex` only; non-interactive → no install.
- `src/setup/setup_test.rs`: preflight with `which codex` failing names "Review runs on codex, which is not on PATH" and Debate side B, no claude row; adapt `ask_typesafe_key` tests to `ask_typesafe` (a yes line first).
- `src/orchestrator/judgment_test.rs`: config `{"typesafe": false}` with key `sk-test` → Wake makes no TypeSafe request, raises the Question with both nudges.
- `src/orchestrator/app_test.rs`: typesafe off → Moderator prompt Inputs has `- TypeSafe: off` and the Debate pane gets no `TYPESAFE_API_KEY`; on → no `TypeSafe:` input.
- `src/shell/shell_test.rs`: exact preflight call list gains `which claude`, `which codex`.

## Verification
`cargo check`, `cargo clippy`, touched tests as I go, full `cargo test` at end; `/code-review` against `main`; commit; write result file.
