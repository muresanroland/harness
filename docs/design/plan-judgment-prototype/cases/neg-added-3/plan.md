# harness-7nq.19 — Delegate skills: {{job}} placeholder lines + output mapping

## Context
Stage skills call Delegate skills as `/tdd`, `/code-review`, `/ponytail`, `/caveman` and hard-code `ponytail-review` / `resolving-merge-conflicts`. The Skill manifest (harness-7nq.18) already records each job's pick (`Manifest::pick`, `JOBS`, `NONE` in `src/skills/manifest.rs`), but nothing reads it at run time. The ticket: each use becomes its own line holding a `{{job}}` placeholder. The Orchestrator fills it with the pick in the App's mention form, or drops the line (none, or not installed). Preflight warns when an installed Stage skill lost a placeholder.

## Changes

### Skill text (`skills/`)
Every placeholder line reads `Use the {{job}} skill …`. The generic instruction always stands on the line before it.
- **stage-implement**: drop `/ponytail`, `/caveman`, `/tdd` and `/code-review`. The Do list becomes:
  1. bd show.
  2. Read docs, then `Use the {{working-mode}} skill …` and `Use the {{prose}} skill …` (both loaded by name, a hook may already load them).
  3. Test-first: "a failing test, then the code", then `Use the {{test-first}} skill for it.`
  4. Typecheck/tests.
  5. Self review: "read the diff against the acceptance criteria and fix what is missing or wrong", then `Use the {{self-review}} skill on the changes since the base, with the Ticket as the spec (bd show <Ticket>) and the approved Plan (<Run directory>/plan.md) as what was meant to be built, and fix what it finds.`
  6. Commit.
  7. Result.
  "Steps 1 to 3 are planning" becomes "1 and 2". The "if one is not installed…" sentence goes; the Orchestrator's Input now does that job.
- **stage-review** step 3 keeps its text, plus `Use the {{review}} skill for this review, then rewrite its output as Findings in the shape below: P0/P1/Critical/blocking → high, P2/Important → medium, P3/Minor/nit → low; drop style-only items; step 4 holds for them too.` The parser is unchanged.
- **stage-moderate** step 1: saving the diff becomes its own bullet. Next is a generic line: "with no audit line below there is none: note 'no over-engineering audit (none picked)' under Notes (or that it was not installed, if **Not installed** names it)". Then the audit line: `` Run `<Side A command> "Use the {{audit}} skill on the diff in <that file>. Output one line per finding: …"` ``.
- **stage-address** step 2: rebase keeping both sides' intent, then `Use the {{merge-conflicts}} skill for the rebase.` Then a generic line: when both intents cannot be kept, stop mid-rebase and ask with STATUS: question. The question gives the hunk and what each side meant. The options are ours, theirs, or a merge you describe. The answer resumes the rebase. Never abort.
- **stage-fix**: `/create-pr skill` becomes `the create-pr skill` (AC: no `/name` call in the shipped Stage skills).

### Code
- `src/skills/manifest.rs`:
  - `placeholder(job) -> "{{job}}"`.
  - `lacks(job, pick, have) -> bool`: the check moved out of `setup::preflight` (not none, not built into its App, not in `list()` names). Preflight now calls it.
  - `Manifest::delegate(&self, skill, have, mention: fn(&str)->String) -> (String, Vec<String>)`: for each line with a placeholder, a pick of none drops the line. A lacking pick drops it too and is returned as `"pick (job)"`. Otherwise the placeholder is replaced with `mention(pick)`.
- `src/orchestrator/app.rs`: new App-table field `mention: fn(&str) -> String`. claude gives the pick as is (in words, plugin-qualified when it is a plugin's `plugin:skill` name). codex gives `$` + the name without its plugin prefix.
- `src/skills.rs`: `stage_skill(repo, home, name) -> Option<Result<String,String>>`, the installed-copy lookup moved out of `attempt()` (.agents/skills, then .harness/skills, then ~/.agents/skills).
- `src/orchestrator/stage.rs` `attempt()`: after reading the skill, load the Manifest (an error Wakes) and `have` = `manifest::list(repo, home, tools)` names. Fill with `delegate`, using the mention form of the row's App. For the Debate it uses side A's App, since the audit prompt runs on the Side A command. Lacking picks add one Input, `Not installed: tdd (test-first), …: their lines are left out; say so in the result file`.
- `src/setup.rs` `warnings()`: for each shipped skill, a placeholder in the shipped body that is missing from the installed copy (`stage_skill`) warns: `the installed stage-implement lacks {{test-first}}: the test first skill you pick never runs there; put the line back, or refresh it with harness init`.

### `harness doctor`
- A new command in `src/cli.rs`: runs preflight and prints each check as ok or failed, exit 1 on a failure, for use in CI.

## Tests (test-first)
- `src/orchestrator/app_test.rs`, fake world:
  - Implement on claude with `tdd` installed in `.agents/skills` gets `Use the tdd skill`.
  - working-mode `ponytail:ponytail`, with the plugin given by a hook on `claude plugin list --json`, gets `Use the ponytail:ponytail skill`.
  - Review on codex with pick `review-agent` gets `$review-agent`.
  - prose none: the line is gone and no `{{` is left.
  - self-review missing: the line is gone, the generic "fix what is missing or wrong" stays, and Inputs carry `Not installed: code-review (self-review)` with "say so in the result file".
  - Moderator with audit none: no audit command line; the "no over-engineering audit (none picked)" note stays.
- `src/skills/manifest_test.rs`:
  - Each job's placeholder is in its shipped Stage skill (implement: 4; review, moderate, address: 1 each).
  - No shipped `stage-*` skill has a `/name` call (regex `(^|[\s\`(])/[a-z]`).
- `src/setup/setup_test.rs`: `warnings` names a Stage skill installed without `{{test-first}}`, and stays silent for the shipped copy.

- `cli_test`: `harness doctor` prints every preflight check and exits 1 when one fails.

## Decisions (Ticket left open)
- The placeholder is filled with the name alone, and the Stage skill supplies "Use the … skill", as the ticket's self-review line does. This gives "Use the tdd skill" on claude and "Use the $review-agent skill" on codex.
- The audit uses side A's App's mention form: the audit runs on side A's command, not on the Moderator's App.
- A codex plugin pick drops the plugin prefix: codex has no Claude plugins, so it gets `$name`.
- A missing pick is noted through one `Not installed` Input. The Orchestrator cannot write the result itself, and there is no panel event: preflight already fails on a missing pick, so this only happens mid-run.
- A built-in pick (`review-agent`) counts as present on any App, as preflight already treats it.

## Verification
`cargo check`, `cargo clippy`, `cargo test` (full). Then /code-review against `main`, fix what it finds, commit, and write `/Users/rolandmuresan/Documents/Projects/harness/.harness/runs/harness-7nq.19/implement.md`.
