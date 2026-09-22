# Controlled live Harness test

This run uses `~/Documents/Projects/test-harness-repo`, prepared on `main`
at `26c39551072069ae62ad2fcd300324a9ee9fd60e`. The human starts Harness and
merges PRs. The automation has prepared the tickets but has not launched agents.

Build the Rust binary first, then use it from inside the target repo:

```bash
cd ~/Documents/Projects/harness
cargo build --release
```

The binary is `../harness/target/release/harness` relative to the target repo.
The globally installed `harness` may be an older build (the Go one carried no
version and no updater); reinstall it by hand with `cargo install --path .`
from the harness checkout, or with the release binary once it ships.
`.harness/state.json` keeps its format, so a target repo mid-run resumes under
the new binary with the same commands.

## Prepared epic

Epic: **test-harness-repo-6fs**. Maximum concurrent tickets: **2**.

| Role | Ticket | Work | Initially |
| --- | --- | --- | --- |
| A | test-harness-repo-6fs.1 | `words.Count` and tests | Ready |
| B | test-harness-repo-6fs.2 | `label.Format` and tests | Ready |
| C | test-harness-repo-6fs.3 | `bounds.Clamp` and tests | Ready |
| D | test-harness-repo-6fs.4 | `--text` CLI using A | Blocked by A |

The previous epic, PR #1, branch, and worktree are preserved. This run uses new
ticket IDs. Old orchestrator state and log are archived under
`.harness/archive/2026-09-21-before-regression-run/`; old stage results must not
be copied into the new run. There is no need to reset Beads or run `harness init`
again: the installed stage skills match this checkout. The documented build's
`testharness` binary is ignored locally through Git's shared `info/exclude`.

`.harness/live-test.json` records preparation inputs and ticket IDs; it is a
snapshot, not the current run status. Use `harness status` for live state.

## Steps for the human

### 1. Start in a normal Herdr terminal pane

Use a shell pane in the workspace where the ticket tabs should appear. Keep
another pane available for control commands. The following checks only report
missing environment variables; they do not print the TypeSafe API key.

```bash
cd ~/Documents/Projects/test-harness-repo
(
  : "${HERDR_ENV:?Open a Herdr terminal pane first}"
  : "${HERDR_PANE_ID:?This shell needs a Herdr pane ID}"
  : "${HERDR_WORKSPACE_ID:?This shell needs a Herdr workspace ID}"
  : "${TYPESAFE_API_KEY:?Load your existing TypeSafe API key into this shell}"
  ../harness/target/release/harness start test-harness-repo-6fs --max 2
)
```

Leave this foreground command running. If a check reports a missing variable,
fix that environment condition before launching; do not invent pane IDs. A
plain shell as the launching pane is supported: events appear in its output
and `.harness/orchestrator.log`.

### 2. Answer trust or permission prompts when they appear

Harness may report that Claude or Codex does not yet trust a specific directory.
Open that agent in the exact directory named in the log, accept the trust
dialog, then exit that temporary session. Harness resumes automatically. For
other permission prompts, inspect and answer them in the indicated agent pane.
Do not change trust configuration files manually or retry a healthy stage that
is merely waiting for your answer.

### 3. Review and merge A's PR

First let A finish an uninterrupted Implement → Review → Debate → Fix pipeline.
Check its code, tests, recorded stage results, and PR description. Use the PR
for branch `test-harness-repo-6fs.1`, not the older PR #1.

Before merging, verify D has not started: it should have no worktree under
`.harness/worktrees/test-harness-repo-6fs.4`. An open A PR is insufficient to
unblock it. Then merge A in GitHub. Harness should close A's Beads ticket, clean
up A's worktree, and start D from the updated default branch. Merge polling is
every 30 seconds, so allow a polling interval before treating a delay as failure.

### 4. Exercise stop and resume while D is active

From the second shell pane:

```bash
cd ~/Documents/Projects/test-harness-repo
../harness/target/release/harness status
../harness/target/release/harness stop
```

Wait for the original foreground command to exit. Its agent panes should remain
open. Agents can continue working after the orchestrator stops; `stop` does not
freeze them. Restart with the same command from step 1. Do not clear state,
delete result files, or reset tickets. Stages with accepted completed results
should be skipped; an unfinished stage starts a fresh session replacing its old
pane. There should be no second PR for an already completed ticket.

### 5. Exercise one failed stage and retry

While D's **Review** is actively working, close only that Review pane in Herdr.
Wait for a `WAKE ... review pane died` event. Other active tickets should keep
moving. Then, from the second shell pane:

```bash
cd ~/Documents/Projects/test-harness-repo
../harness/target/release/harness retry test-harness-repo-6fs.4
```

A fresh Review session should appear and the pipeline should continue. If the
stage finished before you could close it, this fault was not exercised; record
it as untested rather than disrupting a completed ticket. If the retried stage
fails again, retain the evidence and investigate the parked state before issuing
further commands.

### 6. Review and merge the remaining test PRs

Review B, C, and D, check their test results, then merge them yourself. Keep
Harness running until it reports the epic complete. All four Beads tickets
should then be closed and their test worktrees removed. The old PR #1 and its
worktree belong to the earlier run and are not part of this cleanup expectation.

## Evidence and acceptance

During the run, these read-only commands are useful from the target repo:

```bash
../harness/target/release/harness status
bd list --parent test-harness-repo-6fs --all
gh pr list --state all --json number,headRefName,state,url
git worktree list
```

Preserve `.harness/orchestrator.log`, `.harness/state.json`, and the files in
`.harness/runs/test-harness-repo-6fs.*`. Note the PR URLs and the stages where
stop/resume and pane loss were exercised. Check D's branch includes the commit
that merged A before reviewing D's own changes.

The live test passes when there are never more than two tickets in the pipeline,
each ticket produces one valid PR, D waits for A's merge, stop preserves panes
and resumable state, retry replaces only the failed stage, and all merged
tickets close and clean up correctly. A green `cargo test` alone does not
establish these live results. Trust dialogs and PR merges are expected human actions.

If anything differs, preserve the state and log and report the ticket/stage and
observed behavior. A clean restart must not erase the evidence being tested.
