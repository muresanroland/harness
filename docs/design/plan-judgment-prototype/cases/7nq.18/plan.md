# harness-7nq.18: init location, job defaults, preflight on picks

## Context
Ticket harness-7nq.18 (map harness-0sx.7, Jobs and defaults, Location). `harness init` today always writes the Shipped skills to `<repo>/.agents/skills` with `.claude/skills` links, and installs no Delegate skills. The manifest code from Ticket 17 (`src/skills/manifest.rs`) hardcodes `.agents/skills`, and nothing outside its tests calls it yet. This ticket:
- makes init ask where the skills go (checkout / repo / user);
- installs every job's default through `manifest::add`;
- moves the installed skills when the answer changes;
- links the checkout's skills into each Ticket worktree and the Run directory;
- makes `attempt()` find the Stage skill wherever it is;
- makes the preflight fail on a missing job pick, and warn about shadowing and superpowers.

## Changes

### `src/skills/manifest.rs`
- `enum Location { Checkout, Repo, User }` (serde lowercase). `Manifest.location: Option<Location>`. None means init has not asked yet: the skills are in the repo's `.agents/skills`, as before.
- `struct Place { root, files, links: Option<&str> }` with these values:
  - Checkout: `(repo, ".harness/skills", None)`
  - Repo: `(repo, ".agents/skills", Some(".claude/skills"))`
  - User: `(home, ".agents/skills", Some(".claude/skills"))`
- `Manifest::place(repo, home)` gives the Place.
- `add`, `update`, `update_all` and `remove` take `home` and work at `manifest.place(...)`, not at the hardcoded `.agents/skills`. The link target stays `../../<files>/<name>`. Only `own()` guards the repo root: home is the user's own, and a dotfiles-linked `~/.claude` is legitimate there. `put` and `third_party` are parametrized the same way.
- `Manifest::relocate(&mut self, repo, home, to) -> Vec<String>`:
  - Moves every manifest skill (Shipped included) from the old Place to the new one. It tries `rename` first and falls back to `copy_dir` + `remove_dir_all` across filesystems.
  - Removes the old Harness link and makes the new one, then sets `location`.
  - A skill whose new place is taken stays where it is, reported. The own() guard applies when the new place is in the repo.
- `link_checkout_skills(repo, dirs)`:
  - Links every folder in `<repo>/.harness/skills` into each dir's `.claude/skills/<name>` and `.agents/skills/<name>`, with absolute links. Anything already in the way is skipped.
  - Appends `/.claude/skills/<name>` and `/.agents/skills/<name>` to `<repo>/.git/info/exclude`, each line once.
- `list()` also reads `<repo>/.harness/skills`, and skips the home dirs when home is empty. Its plugin read is split out as `plugins(repo, tools)`, which gives the enabled `(name, install_path)` pairs so the preflight can reuse it.

### `src/setup.rs`
- `install_skills(repo, home, force, out, input, tty)`:
  1. Load the manifest. Register each RECORD name as Shipped, for records an older init left.
  2. Current location = `manifest.location`. If unset: Repo when Stage skills are already in `.agents/skills` (a legacy install, or ones a teammate committed), else Checkout.
  3. Ask the location question with the current one as the default. `--force` and a silent stdin take the default.
  4. If the answer changed: `relocate`, print any problems, save.
  5. Gate, create-pr question and Mode as today. "Installed" and "has create-pr" now look at the Place, and still at the repo's `.agents/skills`/`.claude/skills`.
  6. Write each Shipped skill. The target is `<repo>/.agents/skills/<name>` if that already exists, so a repo that committed them keeps them, and "replace" still replaces the repo's own create-pr. Otherwise the target is the Place.
  7. Record in RECORD under the unchanged key `.agents/skills/<name>/SKILL.md`, now an opaque id, so old records and tests keep working. Register the skill in the manifest as `shipped`.
  8. Link at the Place when it has links, as today. Save the manifest. Add `.harness/` to `.gitignore`.
- `choose()` gains a `default` index. Ctrl-C/q and a closed stdin return it. Existing callers pass 0.
- `install_defaults(repo, home, tools, out)`:
  - For each job's default that has a source and is not in the manifest, `manifest::add(..., Some(name))`, which pins the commit.
  - In User mode, a skill you already have in `~/.agents/skills` or `~/.claude/skills` is kept, not cloned: "keeping your X". Other failures print and init goes on; the preflight then names the job.
- `add_lines(path, lines)` generalizes `ignore_run_dir`, which becomes a one-line wrapper. `link_checkout_skills` uses it for `info/exclude`.
- `preflight`:
  - Home comes from `env("HOME")`.
  - The create-pr check uses the `list()` names.
  - Per job: pick = `manifest.pick(job)`. It is skipped when it is `none` or built in (empty source). Otherwise it must be in `list()`, else: `the <job with '-'→' '> skill <pick> is missing: harness init installs it, or /config picks another`. A garbled manifest is itself a failure.
- New `warnings(repo, tools, env)`:
  - Shadowing: outside User mode, a manifest skill that also exists as `~/.claude/skills/<name>/SKILL.md` warns that Claude Code runs a personal skill over a project one.
  - superpowers: an enabled `superpowers` plugin from `plugins()` warns that its SessionStart hook can stall Stages.

### `src/cli.rs`
- init runs `install_skills` (with `home` from `env("HOME")`), then the TypeSafe key, then `install_defaults`, then the preflight. `cli::preflight` prints `warnings` as `preflight:` lines before the missing ones.
- USAGE line updated.

### `src/orchestrator/pipeline.rs`
- `prepare_worktree` calls `link_checkout_skills(repo, &[worktree, run_dir])` after creating the worktree. A failure is logged, not parked.

### `src/orchestrator/stage.rs`
- `attempt()` reads `<skill>/SKILL.md` from the first of these that has it: `repo/.agents/skills` (a committed copy wins), `repo/.harness/skills`, `home/.agents/skills` (skipped when home is empty). With none it wakes with "has no Stage skill (run 'harness init')".

### Other
- `src/orchestrator/world.rs`: create home before `install_skills` and pass it in. The fake world now installs in Checkout mode, so every world test exercises the `.harness/skills` path.
- `src/skills.rs`: update the dead_code comment, since only update/remove wait for /config now.
- README line 51 and `docs/testing/live-run.md:75`: describe the location question and the defaults, and the path under `.harness/skills`.

## Tests (test-first)
- `src/cli/init_test.rs`: `ok_tools()` fakes `git clone` by writing every job default's SKILL.md at its source path, and `rev-parse` answers `abc123`.
  - **each location writes where it says**: answers `""` (non-interactive default), `"1"`, `"2"` and `"3"`, with a temp HOME. For each, check that stage-implement and tdd are at the Place, that the links resolve (none in the repo for Checkout), and that `manifest.location` and tdd's commit `abc123` are recorded.
  - **re-running with another answer moves**: checkout, then repo, then user. Files, links and the manifest location follow, and the old place is emptied.
  - **a user-level copy of yours is not overwritten**: `~/.agents/skills/tdd` stays "mine" and is not in the manifest.
  - Existing tests adapted: default paths become `.harness/skills`, and the typesafe test's scripted keys get a location keystroke first.
- `src/setup/setup_test.rs`:
  - The `install()` helper answers the location question with "2" (repo) first, so existing expectations hold. The force test's path becomes `.harness/skills`.
  - New preflight tests: a missing pick fails naming its job (exact ticket text); all-`none` picks pass; a shadowing `~/.claude/skills/<name>` warns; a fake `claude plugin list --json` with superpowers enabled warns.
- `src/orchestrator/pipeline_test.rs`: after `run_ticket`, the worktree's `.claude/skills/stage-implement` and `.agents/skills/stage-implement` link to `repo/.harness/skills/stage-implement`, and `repo/.git/info/exclude` holds both lines.
- `src/skills/manifest_test.rs`: add `home` at the call sites. The behaviour is unchanged for location None.
- `src/shell/shell_test.rs:541`: the expected calls gain `claude plugin list --json`.

## Decisions left to me
- Exclude lines: one per link path (`/.claude/skills/<name>`, `/.agents/skills/<name>`), not whole folders, so that a Ticket's own new skills are not hidden.
- Links into the worktree and Run directory are absolute: they are local and never committed.
- The location question comes before the gate. Cancel at the gate after a move keeps the move and stops as today.
- Moving out of User level moves the files, so other user-level checkouts lose them (ponytail note). Worktrees prepared before a move keep dangling links (ponytail note).
- Warnings print at init only, like the TypeSafe one. The Shell still refuses a run on the failures.

## Verification
- `cargo check`, `cargo clippy`, `cargo test`.
- `/code-review` against main, then fix what it finds.
- Commit on harness-7nq.18, then write `implement.md`.
