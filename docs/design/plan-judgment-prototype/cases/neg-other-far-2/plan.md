# harness-7nq.17: Skill manifest, git fetch, suggestions per job

## Context
Tickets 18 (init), 20 (TypeSafe skill) and 21 (/config) need a library that records the skills the Harness installed for a checkout (the Skill manifest), fetches a third-party skill with git only (no node, no skills CLI), updates or removes one, knows each job's compiled-in suggestions and default, and lists the skills the user already has. This ticket has no UI and makes no change to init. Scope comes from the ticket plus the map decisions (harness-0sx.7, harness-0sx.12) and the research branches.

## Files
- **New `src/skills/manifest.rs`**: the whole library. It is declared in `src/skills.rs` as `pub(crate) mod manifest;` with `#[allow(dead_code)] // until init (Ticket 18) and /config (Ticket 21) call it`, because the repo has no dead-code allowances and nothing outside tests calls this code yet.
- **New `src/skills/manifest_test.rs`**: `#[cfg(test)] mod manifest_test;` in `src/skills.rs`, following the repo's `<area>/<name>_test.rs` convention.
- **`AGENTS.md`**: the one-line layout entry for `src/skills.rs` also mentions the Skill manifest.
- `src/setup.rs` is not changed. The manifest sits beside `.harness/installed-skills.json`, so init's refresh keeps working unchanged. The ticket allows either option.

## Design (`src/skills/manifest.rs`)
**Jobs and suggestions** (compiled in, default first): `JOBS: &[(&str, &[(&str, &str)])]` holds each job with its suggestions as (skill name, source). An empty source means built into the App, with nothing to install. `NONE = "none"`. All paths below were checked with `gh api` against each repo's HEAD.
- `test-first`: tdd `mattpocock/skills/skills/engineering/tdd`; test-driven-development `obra/superpowers/skills/test-driven-development`; test-driven-development `addyosmani/agent-skills/skills/test-driven-development`
- `self-review`: code-review `mattpocock/skills/skills/engineering/code-review`; requesting-code-review `obra/superpowers/skills/requesting-code-review`
- `working-mode`: ponytail `DietrichGebert/ponytail/skills/ponytail`; karpathy-guidelines `multica-ai/andrej-karpathy-skills/skills/karpathy-guidelines`
- `prose`: caveman `JuliusBrussee/caveman/skills/caveman`; caveman-commit `JuliusBrussee/caveman/skills/caveman-commit`
- `review`: none; review-agent (empty source: codex built-in); requesting-code-review (superpowers)
- `audit`: ponytail-review `DietrichGebert/ponytail/skills/ponytail-review`
- `merge-conflicts`: resolving-merge-conflicts `mattpocock/skills/skills/engineering/resolving-merge-conflicts`; resolve-merge-conflicts `warpdotdev/common-skills/.agents/skills/resolve-merge-conflicts`

**Manifest**, saved as `.harness/skills.json` (serde, pretty JSON; the fields follow the existing `state.rs` idioms):
```rust
struct Manifest { skills: BTreeMap<String, Installed>, picks: BTreeMap<String, String> } // job -> name | "none"; absent = default
struct Installed { repo, #[serde(rename="ref")] git_ref, path, commit, at, shipped: bool } // empty fields skipped
impl Manifest { fn load(repo) -> io::Result<Self>; fn save(&self, repo) -> io::Result<()>; fn pick(&self, job) -> &str }
```
- `load`: a missing file loads as the default. A garbled file is an error, so a later save cannot wipe it.
- `pick`: returns the job's recorded pick, or else the job's first suggestion.
- `at` is where the skill was put, relative to the repo. Ticket 18 may store an absolute path, and `repo.join(at)` handles both.

**Source**: `parse_source(&str) -> Result<Source{repo, git_ref, path}, String>`. `repo` is normalized to a clone URL, so two spellings of the same source compare equal.
- `owner/repo[/path…]` becomes `https://github.com/owner/repo` with that path.
- `https://github.com/o/r[.git][/]` and `…/tree/<ref>/<path>` also work. The ref is the first segment after `tree/`; a `ponytail:` note says a branch name containing slashes is not handled.
- Any other git URL (`git@…:`, `ssh://`, `https://gitlab…`, `file://`) is used as given, with an empty path.
- A bare name (no `/` and no `:`) is refused: "'tdd' is a skill name, not a source: find its source on skills.sh (https://skills.sh/?q=tdd) and add owner/repo or its URL".
- A path containing `..` is refused.

**Fetch**:
- `add(repo, tools, source, name: Option<&str>) -> Result<Added, String>`, where `enum Added { Installed(String), Choose(Vec<String>) }`.
- Steps: parse, then a shallow clone through Tools into a unique temp dir, which is removed before and after use.
  - Command: `env GIT_TERMINAL_PROMPT=0 git clone --depth 1 --quiet [--branch <ref>] -- <url> <tmp>`. Without `GIT_TERMINAL_PROMPT=0`, a typo'd GitHub repo would hang the Shell on a password prompt.
  - Then `git rev-parse HEAD` for the commit.
- The clone is walked under `path` for `SKILL.md` files, skipping `.git`, and each one's frontmatter `name:` is read.
- Choosing the skill:
  - When `name` is given, the folder with that name is taken.
  - When `name` is not given and there is one skill, that skill is taken.
  - When there are several, `Choose(sorted names)` is returned for the caller to pick.
  - When there are none, an error.
- Refusals, checked before anything is written:
  - A name that is not a safe path component, because it comes from a foreign repo.
  - The same repo+path already in the manifest: "already installed, update it instead".
  - The same name from another source, or a Shipped entry: "a skill named X from … is installed: remove it first".
  - A `.agents/skills/<name>` or `.claude/skills/<name>` the Harness did not install.
- The folder is copied with a small recursive copy (files and directories only; symlinks and `.git` skipped) to `.agents/skills/<name>` and linked as init does (`.claude/skills/<name>` points to `../../.agents/skills/<name>`). Then repo, ref, path, commit and `at` are recorded and the manifest is saved.
- `update(repo, tools, name)` re-clones the recorded repo+ref, replaces the folder from the recorded path and records the new commit. It refuses a Shipped skill and a name that is not in the manifest.
- `update_all(repo, tools) -> Vec<(String, String)>` calls `update` for every non-shipped skill and returns the failures (name, reason).
- `remove(repo, name)` refuses a name that is not in the manifest and refuses a Shipped skill ("X is a Shipped skill: it cannot be removed"). Otherwise it deletes the `.claude/skills` link and the folder, drops the entry, and sets every job whose `pick()` was this name to `none`. This covers both an explicit pick and a default.

**Listing**: `list(repo, home, tools) -> Vec<(String, PathBuf)>` returns each skill's name and where it is.
- Directories: `<repo>/.agents/skills`, `<repo>/.claude/skills`, `~/.claude/skills`, `~/.agents/skills`. Each `*/SKILL.md` counts, named by its folder, the way the agents name it.
- Plugins: `claude plugin list --json` through Tools, run in the repo. Only enabled plugins count, and their skills come from `installPath/skills/*/SKILL.md`, named `plugin:skill`, where the plugin is the `id` before `@`.
- If `claude` fails, no plugin entries are listed.
- The Harness's own installs are included. The manifest tells them apart, and Ticket 18's shadow warning needs user-level duplicates.

## Tests (`manifest_test.rs`, fake Tools and `TempDir`), written first with /tdd
1. `parse_source` over a table of every form, including the bare name refused with the skills.sh hint. `add("tdd")` makes no Tools call.
2. A fake clone holding `tdd` and `code-review`:
   - `add(.., Some("tdd"))` installs only tdd, readable through the `.claude/skills` link, and records repo `https://github.com/mattpocock/skills`, path `skills/engineering/tdd` and the commit.
   - `add(.., None)` returns `Choose(["code-review","tdd"])`.
3. Update: the fake answers a second sha and new text, and `update` records the new commit and the new file.
4. Remove: `test-first` is picked explicitly as tdd and `self-review` is left unset (its default is code-review). Removing both sets both jobs to none and deletes the files and entries.
5. A Shipped entry refuses removal and its files stay.
6. Duplicates: the same source added twice is refused ("already installed"). The same name from another repo is refused ("remove it first").
7. Defaults: `Manifest::default().pick` gives tdd for test-first and none for review. A recorded pick wins.
8. Listing: a temp HOME with `~/.claude/skills/mine` and a fake `claude plugin list --json` naming an enabled plugin whose installPath holds `skills/ponytail-review`. The list contains `mine` and `ponytail:ponytail-review` with their paths.

## Decisions taken without asking
- The manifest file sits beside `installed-skills.json` rather than absorbing it.
- The file is named `.harness/skills.json`.
- Job keys: `test-first`, `self-review`, `working-mode`, `prose`, `review`, `audit`, `merge-conflicts`.
- The manifest records the ref as well, because `update` must re-fetch a `/tree/<ref>` source.
- `add` installs one skill per call. A checklist in /config calls it once per ticked skill, so each call clones again.
- Errors are `String` messages meant for the Shell.
- The listing names a skill by its folder.
- Skipped until needed: plugin skills declared in a plugin manifest's `skills` array (only `skills/*` is read), a ref given as a sha (a shallow `--branch` clone cannot fetch one), and concurrent adds racing on the manifest file.

## Verification
- `cargo test skills::manifest`, then `cargo check`, `cargo clippy --all-targets`, and the full `cargo test`.
- /code-review against `main`, then fix what it finds.
- Commit on `harness-7nq.17`, then write the result file.
