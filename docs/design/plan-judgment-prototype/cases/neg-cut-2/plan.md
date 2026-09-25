# harness-7nq.21 — /config: Skills page, Delegate skill per job

## Context
/config (src/shell/config.rs + src/shell/draw/config.rs) edits config.json rows only. The left list already shows `Skills N installed`, but it cannot be opened. The skill backend already exists in `src/skills/manifest.rs` (`add`, `update`, `update_all`, `remove`, `list`, `parse_source`, `JOBS`, `Manifest::pick`). It is `#[allow(dead_code)]` "until /config (Ticket 21)". This Ticket wires that backend into /config and adds the Delegate skills to the Stage pages. Every change saves at once. The prototype is `git show prototype/config:src/shell/draw/prototype_config.rs`. The decisions are in harness-0sx.7 and harness-0sx.9.

## Changes

### src/skills/manifest.rs, src/skills.rs, src/setup.rs (small reuse refactors)
- Move preflight's inline job-to-row match (setup.rs:755) into `manifest::job_row(job) -> &'static str`: review→review, audit→side_a, merge-conflicts→address, else implement. Preflight calls it. /config uses it for the job's section and App.
- skills.rs: drop `#[allow(dead_code)]` and its comment on `mod manifest`.

### src/shell/config.rs
- Pages: `SKILLS_PAGE = APPS_PAGE + 1`. The left list's Down stops at SKILLS_PAGE.
- `Field::Job(usize)`, an index into `JOBS`. `items()` appends the jobs whose `job_row` is in the section: Implement gets test-first, self review, working mode and prose; Review gets review; Debate gets over-engineering audit; Address gets merge conflicts. `Settings::value` reads `manifest.pick(job)` for a job. `note_of`, `label` and the draw handle the Job arm. `job_name()` gives the display name ("self review", "over-engineering audit", "merge conflicts"; "test-first" keeps its hyphen).
- `Settings`: replace `skills: usize` with `manifest: Manifest` and `found: Vec<(String, PathBuf)>` (`manifest::list`, read at open and again after each install, update or remove). If skills.json is garbled, /config refuses to open with a notice. This is the same rule as an unreadable config.json: nothing saves over it.
- Visibility: `seen(app)` = the `found` entries where `app.loads(name, dir)`, Shipped skills left out. `have(app, name) -> Option<(mark, Color, detail)>` returns one of:
  - "built in": the name is in the App's `built_in`.
  - "installed": the Skill manifest has it, detail owner/repo.
  - "yours": the App can see it but the Harness did not install it, detail the folder (`~/.claude/skills`, `plugin X`).
  - None: not installed.
- A job's pick list (`entries` returns early for `Field::Job`):
  - Heading SUGGESTED. Then each suggestion from `JOBS`, with `NONE` skipped. A built-in suggestion (empty source) shows only when the job's App has it built in. A plugin copy (`p:name`) counts as "yours" and picks `p:name`. A same-named skill installed from another source counts as not installed; `add` then refuses it with "remove it first".
  - `none`: "the Stage skill's own instructions".
  - Heading `YOUR OTHER SKILLS <APP> CAN SEE`. Then every other skill the App can see, plus its built-ins, each name once.
  - Typing filters by name. The current pick gets ✓.
- `Entry.mark` becomes `Option<(&'static str, Color)>`. `mark_of` gives RED.
- `Picked::Install(name, source)` covers a suggestion that is not installed. `choose`: `Value` on a Job calls `set_pick` (load the manifest fresh, insert the pick, save; picking the current pick is a no-op). `Install` starts a background add and then picks the skill.
- `typing: Option<(Typing, String)>` with `enum Typing { Model(Pick), Source }`.
- `busy: Option<Busy { text, done: Receiver<Done> }>`. A std thread runs `manifest::add`, `update` or `update_all` over `cfg.tools`, `cfg.repo` and `cfg.home`. `Done` is one of Added(source, result), Ticked(results), ForJob(job, result) or Updated(one or all, result). `Screen::finished()` runs from `poll()` next to `probed()`. It reloads `manifest` and `found`, then shows the note. Esc drops the wait, the same as a probe.
- `listing: Option<Listing { source, names: Vec<(name, installed, ticked)>, cursor }>` is the checklist for `Added::Choose`. A skill already installed from the same repo is ticked and locked. Space ticks. Enter installs the ticked ones in the background, one `add(source, Some(name))` each. Esc cancels.
- `confirm: Option<(String, Confirm)>` with `enum Confirm { Remove(String) }`. y or Enter says yes; n or Esc says no.
- Skills page keys. Rows: location (read-only), then every skill in the manifest (BTreeMap order).
  - `a`: type a source. Enter runs `parse_source` first, so a bare name is refused at once: the skills.sh hint shows in red and the typed text stays. Otherwise a background add runs. It installs the source's single skill, opens the checklist when there are several, or reports the error.
  - `u`: update one skill. `U`: update all. The note gives the old→new commit, "up to date", or the ones that failed.
  - `d`: asks first. The question names the jobs that use the skill, which will be set to none. Yes calls `manifest::remove` on the screen thread (no clone) and reports "removed X; <job> is none".
  - A Shipped skill refuses `u` and `d` at once.
  - `st.doc` is read again after each save.
- `done(text)` helper: sets the green note and the saved time. During a run it also logs `config: <text>`, as `save()` does.

### src/shell/draw/config.rs
- The pipeline's Skills row gets the cursor. Its summary comes from `manifest`.
- Stage pages: a `DELEGATE SKILLS` block after the rows, before CHECKS. It shows each job's pick and where it comes from, "not installed" in red, or none with its muted line.
- `skills_page`: title plus count, the description, the location row, then each skill. A skill row shows owner/repo @ commit (7 characters) and `← <jobs using it>`, or "shipped with the Harness".
- The checklist (`[x]` / `[ ]`), and the job pick list (title `<Section> <job> · <app>`, a wider name column, colored marks).
- Foot: the busy spinner in orange; the confirmation with `y/n`; the source prompt; the row notes, including the location's "Run harness init again to change where skills are installed."
- Hints for each new mode.

## Tests (src/shell/config_test.rs, over a fake Tools whose `git clone` writes a two-skill repo and whose `rev-parse` gives a commit)
1. Adding `owner/repo` with two skills, one already installed, shows the checklist with that one ticked. Ticking the other and pressing Enter installs it. The manifest then records both.
2. A bare name is refused with the skills.sh hint and nothing is cloned.
3. Removing a skill a job uses: `d` shows the confirmation, n keeps the skill, then `d` and y remove it and set the job to none in skills.json.
4. Picking the not-installed tdd for test-first clones and picks it: the clone call is seen, the manifest has tdd, and the pick is tdd.
5. With a home that has `.claude/skills/grilling` (claude-only) and `.agents/skills/archify`, the Review job's list on codex shows archify and hides grilling. Implement's list on claude shows grilling.
6. TestBackend renders of the Skills page and of a job's pick list, with exact lines.

Existing tests: update the Debate page render (the DELEGATE SKILLS lines come before CHECKS). Fix anything else that breaks when `skills`, `typing` or `mark` change.

## Decisions (they go in the result file)
- A built-in suggestion is marked "built in" rather than installed, yours or not installed. It shows only on an App that has it built in.
- Every ticked skill is its own clone (a ponytail comment says so).
- A garbled skills.json keeps /config closed with a notice.

## Verification
`cargo check`, `cargo clippy`, `cargo test config` while working; then `cargo test` and `cargo build --release`. Then /code-review against main and fix what it finds, commit, and write the result file.
