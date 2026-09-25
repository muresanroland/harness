# harness-7nq.10: /config checks, same-model toggle, Apps page

## Context
Ticket 9 shipped /config (rows saved at once, probe first). This ticket adds:
1. **Checks**: rules that refuse any change leaving config.json broken. The left list, the title badge and a CHECKS block show them. A hand-edited config.json that breaks a rule refuses the run.
2. **Plan + Implement toggle**: `[x] Same model for plan and implementation`. Turning it off picks a plan model; that sets Ticket 8's `plan_model` split.
3. **Apps page**: each App installed or not, with its version. The other herdr Apps show greyed `not supported yet`. An App that is not installed opens a homepage window, opened through Tools.

Sources: map tickets harness-0sx.9 (revised) and 0sx.14; prototype `prototype/config:src/shell/draw/prototype_config.rs` (checks_of, family_of, canonical, rule_mark, home_window).

## Rules core: `src/orchestrator/app.rs`
- `App.family: &str` becomes `Option<&'static str>` (claude `Some("Anthropic")`, codex `Some("OpenAI")`). `None` means the App runs several families, so its default's family is unknown (Ticket 22's Apps).
- Add `App.home: &'static str` for the homepage. claude: `https://claude.com/product/claude-code`. codex: `https://developers.openai.com/codex` (both from research/agent-clis).
- `family(app, model) -> Option<&str>`: the App's own family when it has one. Otherwise the family its name tells (claude/fable/opus/sonnet/haiku = Anthropic, gpt = OpenAI, gemini = Google, grok = xAI). `None` for `default` or a name that tells nothing.
- `canonical(model) -> String`: strips a `provider/` prefix, a `[…]` suffix and a `-YYYYMMDD` date, and turns `.` into `-`. Then it maps the aliases in `ALIASES`: fable→claude-fable-5-1, opus→claude-opus-5-5, sonnet→claude-sonnet-5, haiku→claude-haiku-4-5, and `opus-5-5`-style names get `claude-` put in front. So `opus`, `opus-5.5` and `anthropic/claude-opus-5-5` are one model. A `ponytail:` comment marks the pinned alias versions.
- `full_id(model)`: the alias lookup only. The split uses it, because opusplan needs full ids.
- `Check { rows: &'static [&'static str], holds: Option<bool>, text: String }`. `holds: None` means the family or model cannot be told, and that counts as broken. Texts have no final period.
- `rules(row: impl Fn(&str) -> Option<(&App, String)>) -> Vec<Check>`. It runs over a row table keyed by config key; `"plan"` gives Implement's plan model.
  - Split (only when plan ≠ default and plan ≠ model): both on one known family. Holds: `Plans on {plan} and implements on {model}, both {F}`. Broken: `The plan ({plan}) and the implementation ({model}) must be one family`.
  - The Review, and the fallback unless its model is `none`, never on Implement's model, by `model_id`. `model_id` is `canonical`, or `<app>'s default`, which is None on an App with no family. Broken: `The Review would run on Implement's model, {id}: it must not review its own work`. Holds: `The Review runs on {id}, not Implement's {imp}`.
  - Debate sides: two known, different families. Broken: `Both sides would be {F}: the Debate needs two families`. Unknown: `side A's family can't be told ({app} {model}): pick a model that names it`.
- `checks(doc)` runs `rules` over config.json. A row that cannot be read is skipped, since its own read error refuses it.
- `check(repo) -> Result<(), String>`: the first broken rule. `Orchestrator::new` calls it (`stage.rs:255`) after its row reads, so a hand-edited config.json names the rule and refuses the run.
- `debate_inputs` drops its `app.family ==` test and uses the Debate rule, so one text serves both.

## /config: `src/shell/config.rs`
- `Field` gains `Plan` (JSON key `plan_model`, label "plan model") and `Same` (the checkbox). A new `Field::key()` gives the JSON name; `name()` stays the label.
- `put(doc, key, fields)` replaces the field-setting part of `staged`. On `implement`:
  - A new App in fields, a plan that is default, or a plan equal to the model (canonically) removes `plan_model`. That gives one model again, so an App change never strands the plan.
  - Otherwise both halves are saved by `full_id`. Picking `fable` over `opus` saves `claude-fable-5-1` / `claude-opus-5-5`, which Ticket 8's `row_in` needs.
  - A field that is not a string is left alone for `row_in` to refuse.
- `staged` puts the fields, runs `row_in`, then `checks` on the new doc. The first broken rule is Err, so `change` says `Refused: <rule>. Nothing changed.` before any probe or write, and `save` re-checks.
- `Settings`:
  - `installed: Vec<bool>`, `versions: Vec<String>` (first line of `<app> --version`), `others: Vec<String>` (names in `herdr integration status` not in APPS) and `home: Option<&'static App>` (the window).
  - `split() -> Option<String>`.
  - `items()` becomes a method. Section 0 gives App, Same, [Plan when split], Model, Effort. Section `APPS_PAGE = 5` (after Address) lists the Apps.
  - `note_of` becomes a method.
- `entries`:
  - An App that is not installed is `dim` with detail `not installed`.
  - Model entries show `family(app, id)`, or "family unknown".
  - The Plan list shows Implement's App's listed models of Implement's family, plus `type an id…`.
  - Each model entry gets a mark from `checks(put(doc.clone(), …))`, looking only at checks whose rows hold the picked row: `✗ not Implement's family`, `✗ not the plan's family`, `✗ the Review's model`, `✗ the fallback's model`, `✗ Implement's model`, `✗ side A's family`, `✗ side B's family`, `? family unknown`.
  - "Current" compares canonically for models.
- Keys:
  - On Same, Enter or Space toggles. Turning it off is refused when the App is not claude (`… only claude splits, through opusplan`). It is also refused when Implement's model is default (`Pick Implement's model first: the split needs a named model for each half.`). Otherwise it opens the Plan list; Esc keeps one model. Turning it on saves `plan_model` default at once.
  - Picking a Plan model (listed or typed) is probed first; picking Implement's own model makes it one model again.
  - Down from Address reaches Apps. On the Apps page, Enter on an installed App shows a note, and Enter on one that is not installed opens the window.
  - Picking an App that is not installed opens the window. There, Enter runs `open`/`xdg-open <home>` through Tools and closes it; Esc closes it.
- `said()` includes the split (`plan→model`) for RECENT's config line.

## Drawing: `src/shell/draw/config.rs`
- Title badge `✗ N check(s)` (RED) first.
- Left list: a section with a broken check gets ` ✗` (RED) or ` ?` (ORANGE) after its summary. The Apps row can be selected. Section 0's summary on a split reads `claude plan→model`.
- Section pages end with a CHECKS block: `✓`, `✗` or `?` and the text plus a period. The Plan+Impl split check shows there, the Review page shows the Review/fallback checks, and the Debate page shows the sides check.
- Section 0 labels: `[x]`/`[ ] Same model for plan and implementation`, `plan model`, and `implement model` when split.
- The pick list's mark column shows the rule mark (RED ✗ / ORANGE ?), else `✓ current`. A dim entry's name is MUTED.
- The Apps page shows the title, a description, each App's row (name, `installed` + version, or greyed `not installed` + homepage), then the others greyed `not supported yet`.
- The window: a rounded box over the dock titled ` <app> is not installed `, with "install it, then run harness init again", the homepage, and `Enter opens it · Esc closes`. The hint line follows.

## Tests (TDD, red first)
- `src/orchestrator/app_test.rs`:
  - Table tests of `rules`: the split's one family.
  - The Review or fallback on Implement's model, across Apps: `opus` / `opus-5.5` / `anthropic/claude-opus-5-5` are one model, and the fallback `none` is skipped.
  - The Debate's same family.
  - Unknown families: a test App `App { family: None, ..APPS[0] }` leaked to `'static`, with its default refused as unknown, and a named `google/gemini-3-pro` told.
  - `canonical` cases.
  - Update the Debate message in the existing Wake test.
- `src/shell/config_test.rs`:
  - A refused pick (the Review onto claude `claude-opus-5-5` with Implement `opus`) leaves config.json byte for byte, says the rule, and does not probe.
  - The toggle off with Implement at default is refused.
  - A split flow: Space opens the plan list, `fable` is probed and saves both full ids, the plan/implement rows and the CHECKS line render, and toggling on removes `plan_model`.
  - `put`: a split plus a new App gives one model again. This is a unit test, because Implement cannot leave claude until harness-7nq.12 lifts `runs_on`.
  - A hand-edited broken config.json: `/start-epic` is refused naming the rule, and /config shows the Review `✗` and the `✗ 1 check` badge.
  - Not installed (Fake failing `which codex`): the App list greys it, Enter opens the window, Enter runs `open <home>` through the Fake and closes, Esc closes with no call. The same from the Apps page.
  - TestBackend renders: the marks in a pick list (side B onto claude, all `✗ side A's family`), and the Apps page with version, `not installed` and `not supported yet`.
- Existing tests keep passing. Only message changes are expected (the Debate text).

## Decisions made without asking
- Family is App-level for single-family Apps: claude runs only Anthropic, codex only OpenAI. Name-based only for Apps with no one family.
- A split saves full ids for both halves: Implement's `opus` becomes `claude-opus-5-5`, the same model.
- Installed means `which <app>` through Tools, as today. The other herdr Apps come from `herdr integration status` text, not the `integration.list` socket, which has no CLI.
- Marks only consider checks involving the picked row. A save still refuses on any broken rule.
- Skills/TypeSafe pages are still not selectable (Ticket 21).

## Verification
`cargo check`, `cargo clippy`, `cargo test`, then /code-review against `main`. Commit on this branch, then write `implement.md` in the Run directory.
