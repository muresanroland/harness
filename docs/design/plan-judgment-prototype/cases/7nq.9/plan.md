# Plan: harness-7nq.9, /config: the docked modal, the Pipeline sections and their rows, saved at once

## Context

Ticket harness-7nq.7 added `.harness/config.json`: an App, a model and an effort for each Stage row. Each Stage reads it when it starts. So far you can only change it by editing the file by hand. This Ticket adds `/config` to the Shell. It is a modal docked beside the live Shell, built from prototype C (`prototype/config` @ b4700d2). You pick a Stage, then a row's App, model or effort. Every change saves at once to config.json. A named model is probed first. During a run, the Stages that start later use the new value, and RECENT logs `config: …`. Not in this Ticket (they come later): checks, the `✗ N checks` badge, the same-model toggle and the Apps page (Ticket 10); the Skills and TypeSafe pages (Ticket 21).

## Changes

### 1. `src/orchestrator/app.rs`: the App table and config.json

- Two new fields on `App`:
  - `family: &'static str`: "Anthropic" for claude, "OpenAI" for codex. It labels each model in the pick lists.
  - `models: fn(&dyn Tools, &Path) -> Result<Vec<(String, Vec<String>)>, String>`: each model id with its effort levels.
    - claude gives its aliases `fable opus sonnet haiku`, each with `low medium high xhigh max`. It makes no call.
    - codex runs `codex debug models --bundled` through Tools. From `models[]` it keeps the entries with `visibility == "list"`: each `slug` with its `supported_reasoning_levels[].effort`. The format was checked against codex 0.156.1.
- Split `row()` into three reusable parts (the error messages stay exactly as they are):
  - `pub(crate) fn read(repo) -> Result<(PathBuf, Value), String>`: the file read and JSON parse. A missing file reads as Null.
  - `pub(crate) fn field(doc, key, name) -> Result<String, String>`: the closure pulled out. It fills in the default: the App from the key, `"default"` otherwise.
  - `pub(crate) fn row_in(doc, key, path) -> Result<Row, String>`: the rest of `row()`. `row()` becomes `read` + `row_in`.
- `pub(crate) fn runs_on(key, app) -> Result<(), String>`: the existing "`{key} runs on claude only`" rule pulled out. `row_in` and /config's App pick both use it.
- A new row key, `review_if_limited` (a const). It may run off claude, like the Review. Its App defaults to claude. Its model defaults to `"none"`, which means no fallback. Ticket 15 reads it.
- `pub(crate) fn probe(app, dir, model) -> Vec<String>`: the App's existing headless read-only `side` command, plus its model flag, plus `"Reply with ok"`. This gives `claude --tools … -p --model X` and `codex exec --sandbox read-only -m X`. It reuses `fill`.
- `pub(crate) fn write(path, doc)`: pretty JSON written to `config.json.tmp`, then renamed into place. This is the same pattern as `State::save`. A Stage can read the file at any moment during a run, so it must never see a half-written file.

### 2. `src/shell/config.rs` (new): what /config holds and does, as `impl Screen` plus `Settings`

- `ROWS`: the key, the name, the prefix used in labels, the Stage it belongs to, and a one-line note. The rows are:
  - Implement (Plan+Impl)
  - Review, and Review if limited (key `review_if_limited`)
  - Moderator, Debate side A, Debate side B (all under Debate)
  - Fix
  - Address
- `STAGES`: the title, the short name and the description for Plan + Implement, Review, Debate, Fix and Address. The wording comes from the prototype.
- `Settings`:
  - The config doc and its path.
  - The model list of each App, read once when /config opens.
  - How many Apps are installed, and how many skills are installed.
  - The cursor: which Stage, whether its page is open, which row.
  - The open pick list, if any: the row, the field (App, model or effort), the new App a model list belongs to, the cursor and the filter.
  - The id being typed, if any.
  - The probe in progress, if any: a receiver and the pending change.
  - The foot's note and its color.
  - The time of the last save.
- `Screen::open_config()`, run by the `/config` command:
  - It reads config.json. If the file is unreadable, a notice names the file and /config does not open, so nothing can save over the file.
  - It calls each App's `models`.
  - For the Apps summary it runs `which <app>` through Tools.
  - For the Skills summary it counts the Skills that are not Shipped in `Manifest::load`.
- Keys, as the Ticket lists them (Ctrl-C stays global):
  - On the Pipeline list: ↑↓ picks a Stage, Enter or → opens it, Esc closes /config.
  - On a Stage page: ↑↓ picks a setting, Enter changes it, ← or Esc goes back.
  - In a pick list: typing filters, ↑↓ moves, Enter picks, Esc goes back.
  - While typing an id: Enter probes the id and saves it, Esc cancels.
  - While a probe runs, only Esc works: it drops the probe and nothing is saved.
- Picking:
  - An App leads straight into that App's model list, and the App and the model save together.
    - On a row that runs on claude only, codex is refused at once, with the reason from `runs_on`.
    - Picking the current App changes nothing.
    - A new App resets the effort to default.
  - A model list has these entries, each labelled with its family:
    - `none`, on the Review if limited row only
    - `default`
    - the App's models
    - `type an id…`
  - An effort list has `default`, then the effort levels of the row's model. For `default` or a typed id, it offers the levels of every listed model. An App with no effort flag shows `— <app> has no effort flag`.
- `change(row, fields)`:
  1. Read config.json again, apply the new fields and check the whole row with `row_in`. A refusal is shown and nothing changes.
  2. If the model is named (not default or none), run `app::probe` on a thread through `cfg.tools`, with the result coming back over a channel. The foot shows the probe in progress.
  3. `Screen::poll()` takes the result:
     - On success it saves.
     - On failure the old value stays and the foot shows the App's stderr (or its status when stderr is empty), followed by "Nothing changed."
- Saving:
  - `app::write`.
  - The badge `saved <time>`.
  - A green note `saved: …`.
  - While a run is live, `tell(None, "config: <Row> <old> → <new>")` goes to RECENT and the log. The old and new values are written as `Row::said()` writes them (`codex gpt-6-sol/high`), or as `none`.

### 3. `src/shell/draw/config.rs` (new): the drawing, from the prototype's `by_stage`

- The frame: `modal::dock()`, made `pub(super)` and reused as it is. From 110 columns it takes the right 58% with a thick purple border. Under 110 columns it is a rounded box over the dimmed Shell.
- The title is ` /config `. The right-aligned badges are `N waiting`, `run live` and `saved <time>`. The bottom title is the key hint.
- The left side, PIPELINE:
  - Plan+Impl, Review, Debate, Fix and Address, each with a summary (the App, plus the model when one is set; Debate shows its Apps joined by +). The rows are joined by `│` when there is room.
  - A divider.
  - Apps `N of M installed`, Skills `N installed` and TypeSafe `on/off`. These are summary lines only; the cursor skips them.
- The right side is either the Stage page or the pick list.
  - The Stage page:
    - The title and its Apps.
    - The description, wrapped with `modal::wrap_spans`.
    - Each row's app, model and effort, with a blank line between rows.
  - The pick list:
    - A title with `filter ›`.
    - The entries, with `✓ current` marked.
    - A scroll that keeps the cursor in view.
- The foot has two lines, showing the first of these that applies:
  - the probe in progress, with the spinner
  - the id being typed, with its help line
  - the note
  - the note on the row under the cursor
  - the note for the Pipeline list
- `draw()` in `draw.rs`: when `s.settings` is Some, it draws /config ahead of the plan modal.

### 4. `src/shell.rs`

- `mod config;`
- The field `settings: Option<config::Settings>`.
- A new entry in `COMMANDS`, before `/exit`: `("/config", "", "the App, model and effort each Stage runs on")`.
- A `/config` arm in `command()`.
- `key()`: after Ctrl-C, when /config is open, every key goes to it.
- `poll()`: takes a finished probe, before its early return.

### 5. Docs

- The README command table gets `/config`.
- `docs/design/events.md` gets the `config: …` line.

## Decisions the Ticket left open

1. The fallback row's key is `review_if_limited`. When the row is missing it means none: model `"none"` on App claude.
2. Rows that run on claude only (Implement, Moderator, Fix, Address) still list codex, but refuse it with the reason from `runs_on`. This lasts until Tickets 12 and 22.
3. An App change resets the effort to default, because each App has its own levels. Picking the current App again changes nothing.
4. The probe runs the App's read-only side command, once per save, with no cache. The command is `claude … -p --model X` or `codex exec … -m X`.
5. Before each save, config.json is read again and the whole row is checked with the same reader Stages use. The write is atomic.
6. `config: …` goes to RECENT only while a run is live, as in the prototype. The badge `saved <time>` is the time of the last save made in this /config.
7. Apps, Skills and TypeSafe are summary lines only. Installed means `which` finds the App. The Skills count leaves out Shipped skills.
8. Implement's `plan_model` is not touched. The same-model toggle is Ticket 10.

## Tests (test-first, `src/shell/config_test.rs`, over the fake world and fake Tools)

1. `/config` opens docked at 160x45: the Shell's left 42% is drawn, and the thick box titled `/config` starts at column 67. At 100x30 it opens as a rounded box over the dimmed Shell, with the input line kept below it.
2. The Review is `{"app":"claude","model":"opus","effort":"max"}`. The steps are: the Review page, then the app row, then codex. That leads to codex's model list, which comes from a fake `codex debug models --bundled` reply. The reply includes a `hide` model, and the test checks it is absent from the list. Picking `gpt-6-sol` runs the probe (the fake answers ok). Then the effort row, then `high`. config.json then holds `review: {app: codex, model: gpt-6-sol, effort: high}`.
3. A typed id whose probe fails: the fake fails `claude … --model claude-nope …` with stderr "model not found". config.json is unchanged, and the foot shows the error.
4. During a fake run (World, with the Implement session held on a channel), the Review model changes. RECENT and the log get `config: Review codex → codex gpt-6-sol`. Once Implement is released, `herdr agent start h-hx-1-review` carries `-m gpt-6-sol`, and the started line names it.
5. TestBackend renders of the Pipeline list (the rows, the summaries, the `│` joins, the divider, the Apps, Skills and TypeSafe lines), of a Stage page (the Debate: the title, the Apps, the grouped rows) and of a pick list (claude's models with their families, `✓ current`, `type an id…`).
6. An existing test changes: `shell_test` `the_slash_list_renders_above_the_input_with_its_hint` now counts one more command.

## Verification

- `cargo check`, `cargo clippy`, `cargo test config` while working, then the full `cargo test`.
- `cargo build --release`.
- /code-review against `main`, and fix what it finds.
- One commit on the branch. Then write `implement.md` (`STATUS: done`).
