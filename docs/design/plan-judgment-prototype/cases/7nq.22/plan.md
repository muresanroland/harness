# harness-7nq.22: four experimental Apps (pi, opencode, copilot, cursor)

## Context

The App table (`src/orchestrator/app.rs`, `APPS`) holds only claude and codex. The map ticket harness-0sx.14 (amended) adds pi, opencode, copilot and cursor for other devs. The rows come from `docs/research/agent-clis.md` on branch `research/agent-clis`. They ship unverified, with no live checks. A wrong row shows up as a Wake or a trust wait the dev can see. Each row runs on every Stage. Implement already takes the two-step Plan off claude (`stage.rs:793`, `plan.rs:205`), and Ticket 11's Review guard already covers every App, so neither needs a change here.

## Changes

### `src/orchestrator/app.rs`
- **New `App` fields.**
  - `bin`: the binary for `which`, `--version`, the side command and the model list. It is `cursor-agent` for cursor, because herdr launches and resumes `cursor-agent`. For every other App it is the App's name.
  - `side_effort`: the effort form on the Debate side. It is `--variant {}` for opencode. Every other App uses its pane form.
  - `experimental`: true for the four new Apps.
- **`APPS: [App; 6]`**. Each new row:

  | | pi | opencode | copilot | cursor |
  |---|---|---|---|---|
  | worktree / run_dir args | none | `--auto` | `--allow-all --add-dir <other dir>` | `--force --add-dir <other dir>` |
  | model | `--model {}` | `-m {}` | `--model {}` | `--model {}` |
  | effort (pane) | `--thinking {}` | none | `--effort {}` | `ON_MODEL` = `[effort={}]`, appended to the model's value |
  | resume | `--session {}` | `--session {}` | `--resume={}` | `--resume {}` |
  | side | `pi --tools read,grep,find,ls -p` | `env OPENCODE_PERMISSION={"edit":"deny","bash":"deny","external_directory":"allow"} opencode run` | `copilot --deny-tool=write --deny-tool=shell --add-dir {} -p` | `cursor-agent --mode ask --add-dir {} -p` |
  | home | https://pi.dev | https://opencode.ai | https://github.com/features/copilot/cli | https://cursor.com/cli |
  | trust | `pi_records` | `\|_, _\| Some(true)` (no dialog) | `copilot_records` | `cursor_records` |
  | models | `pi --list-models` table: `provider/model`, each with pi's thinking levels | `opencode models`: one `provider/model` per line, no efforts | `Ok(vec![])`, so /config shows only default and 'type an id…' | `cursor-agent models`: `<id> - <name>` lines, no efforts |

  Every new row also has `mention: ""` (the skill named in words), `built_in: &[]`, `skill_dir: ".agents/skills"`, `plugins: false`, `family: ""` (told from the model) and `experimental: true`.
- **`Row::args(effort_form)`**, shared by `flags()` (panes) and `side_command()` (with `side_effort`). The `ON_MODEL` form appends the effort to the model's value.
- **`side_argv`**, used by `side_command` and `probe`. It puts the model and effort args before the side form's closing `-p`, so the brief comes right after `-p`. copilot's `-p` takes the brief as its value. claude's side line order changes the same way; claude's own behavior does not change.
- **Family per model.** This replaces the ponytail comment that deferred it until pi joined the table.
  - New `family_of(app, model) -> Option<&str>`: the App's own family when it has one (claude, codex). For the four new Apps it is read from `canonical(model)` through `FAMILIES`: `claude-` is Anthropic, `gpt-` OpenAI, `gemini-` Google, `grok-` xAI.
  - `default` on these Apps, or an unlisted name, has an unknown family.
  - `model_id` returns None for the default of an App without its own family.
  - `debate()` takes two families. The Review check and the Debate check fail when a family or a model cannot be told. This follows 7nq.10's rule: "a family that cannot be told counting as a failure; rows a check reads need a named model".
  - `debate_inputs` uses `family_of`.
- **`canonical`** also removes pi's `:thinking` suffix.
- **`runs_on`**: only codex is limited to Implement, the Review, the fallback and the sides. Its message becomes "{key} does not run on codex".

### `src/orchestrator/trust.rs`
- **`pi_records`**: reads `~/.pi/agent/trust.json` (dir → bool). The nearest recorded ancestor of dir applies, above the repo too.
- **`copilot_records`**: `trustedFolders` in `~/.copilot/config.json`. Any listed ancestor gives Some(true).
- **`cursor_records`**: the marker `~/.cursor/projects/<cursor_slug(d)>/.workspace-trusted` for dir or any ancestor.
- **`cursor_slug`**: drops the leading `/` and turns every non-alphanumeric character into `-`. This is unverified and goes on the live-check list.
- The header comment names every App.

### `src/orchestrator/limit.rs`
- **The new Apps' limit patterns** (in app.rs):
  - **pi:** `You have hit your (?P<what>…usage limit) (…). Try again (?P<reset>in ~N min)`.
  - **opencode, Go limit:** `(?P<what>… usage limit) reached. It will reset (?P<reset>in …).`
  - **opencode, retry:** `[retrying (?P<reset>in …) attempt #N]`. A retry in seconds alone is no limit.
  - **copilot, rate limits:** the three rate limits, with an optional lead-in, then `Please wait for your limit to reset (?P<reset>in N minutes|on <date> at <time>) or switch`.
  - **copilot, credits:** `run out of your included (?P<what>AI credits)`. It gives no reset.
  - **cursor:** `You['’]re out of usage`. It gives no reset.
  - A pattern with no reset is looked at again an hour later, as codex's "Try again later." is.
- **`parse_reset`**:
  - It removes a leading `on `.
  - An `in …` reset counts from now: `~45 min`, `12 minutes`, `3h 12m` and `3 hours 12 minutes`, through a small `from_now`.
  - The date regex accepts `at ` before the time, for copilot's `September 28, 2026 at 3:00 PM`.
  - A `ponytail:` note: an old relative line inside the last 20 lines holds the Ticket again instead of Waking it, the same as "later" does today.

### `/config` (`src/shell/config.rs`, `src/shell/draw/config.rs`)
- **`bin` in `which` and `--version`.**
- **Model family shown per model** (`family_of`, else "family unknown"). A pick that fails because of an unknown family is marked `? family unknown`.
- **App pick list:** an installed experimental App shows `experimental, unverified` where claude and codex show their family.
- **Apps page:** a blank line, then a heading `experimental, unverified`, then the four new Apps.

### Other
- `src/setup.rs`: the integration offer and the preflight run `which app.bin`.
- `src/orchestrator/stage.rs:982`: the comment now says only codex is limited.

## Tests (TDD, in the existing `_test.rs` files)
- **`app_test.rs`:**
  - One fake-world table over the four Apps, with `fix`, `review`, `moderator` and `side_a` on the App and a model and effort set. It checks:
    - the Fix argv (the worktree args, then the model and effort flags)
    - the Review argv (the run-dir args)
    - `--kind <app>` on the Fix and Debate starts
    - the "Side A command" Input: its exact quoted line, with the flags before `-p`, opencode's `--variant` and cursor's `[effort=high]` suffix
  - The existing Moderator-Inputs expectation for claude is updated.
  - Rule cases:
    - a pi side on an Anthropic model against codex holds
    - a pi side on its default is broken: family unknown
    - a Review on pi's default is broken
    - `canonical` of `anthropic/claude-opus-5-5:high`
  - The error table: the moderator on codex gives "moderator does not run on codex"; the "no App named" case now uses `gemini`.
  - A model-list parse over fake Tools output for pi, opencode and cursor.
- **`trust_test.rs`:**
  - `trust_home` also writes pi's `trust.json`, cursor's marker and copilot's `config.json` for the repo.
  - The loop covers every App except opencode, and checks an explicitly untrusted subdirectory only on the Apps that record one (claude, codex, pi).
  - Readers: the parent of the repo trusted means the worktree is trusted (pi, cursor, copilot); an unknown dir is untrusted; opencode trusts every directory.
  - `cursor_slug("/Users/me/my.repo") == "Users-me-my-repo"`.
- **`limit_test.rs`:**
  - A sample from the research for each new App, with its reset, whether it is long, and its `what`.
  - The same samples under 20 newer lines are no limit.
  - A copilot date already past is no limit; a retry in seconds is no limit; another App's text is no limit.
- **`config_test.rs`:** the Apps page render shows the six Apps, the `experimental, unverified` heading, and one not installed with its homepage. The pipeline summary becomes `6 of 6 installed`.
- **`shell_test.rs`:** the refusal messages and the `pi` → `gemini` case are updated.

## Decisions the Ticket left open (these go into the result file)
- **Side commands:**
  - copilot and cursor also get `--add-dir {run}`, and opencode also gets `external_directory: allow`. The diff sits in the Run directory, outside the worktree, and a side has to read it.
  - opencode's variable goes through `env`, because a quoted `VAR=…` is not an assignment.
- **No trust skips** (cursor `--trust`, pi `--approve`, `COPILOT_ALLOW_ALL`): trust stays the user's one accept, read by the readers.
- **No sandbox** on a copilot or cursor Review; Ticket 11's guard covers it.
- **opencode has no real retry bound** (retry.ts), so the Moderator's Stage timeout covers a side stuck in a retry.
- **/config effort lists:** copilot, cursor and opencode's `--variant` list no efforts, so /config offers only default for them. Their efforts are set in config.json by hand.
- **Left out:** the `?` in the left-hand list and the per-model skill folders beyond `.agents/skills`.

## Verification
- `cargo check`, then `cargo clippy`.
- `cargo test app_test trust_test limit_test config_test shell_test`, then the full `cargo test`.
- `/code-review` against `main`, then fix what it finds.
- Commit to the branch; no push.
- The result file lists each App's live checks:
  - unattended launch through herdr
  - reading idle, working and blocked (the two-step Plan's wait, a Stage question)
  - the trust reader and cursor's slug
  - the limit text and its reset
  - the read-only side command, including opencode's `external_directory`
  - the Delegate-skill mention
  - resume by id
  - the shape of each model listing
