# Harness

Drives a [beads](https://github.com/gastownhall/beads) Epic through a fixed multi-agent pipeline, from Tickets to open pull requests, with every agent visible in [herdr](https://herdr.dev).

Each Ticket gets its own git worktree and herdr tab. It passes through:

1. **Implement**: Claude Code writes a plan, which is approved before any edit, then implements it.
2. **Review**: Codex reviews the change.
3. **Debate**: a Claude moderator settles each Finding as fix or skip.
4. **Fix**: Claude Code applies the fix items.

Review, Debate and Fix repeat for up to 3 rounds, then a pull request opens. You merge it. The Harness then closes the Ticket and cleans up its worktree. Vocabulary: [CONTEXT.md](CONTEXT.md).

## Requirements

- macOS on Apple Silicon or Linux x86_64 (other platforms: build from source)

| Tool | Used for | Setup |
|---|---|---|
| [herdr](https://herdr.dev) | Runs every agent in a visible pane | Start `harness` from a pane inside herdr (`HERDR_ENV=1`) |
| [beads (`bd`)](https://github.com/gastownhall/beads) | The Epic and its Tickets | `bd init` in the Target repo |
| [git](https://git-scm.com) | Branches and worktrees | The repo needs a remote on GitHub |
| [GitHub CLI (`gh`)](https://cli.github.com) | Opens PRs, watches for merges | `gh auth login` |
| [Claude Code (`claude`)](https://claude.com/claude-code) | Implement, Debate, Fix, Address | Logged in |
| [Codex CLI (`codex`)](https://github.com/openai/codex) | Review | Logged in |
| [TypeSafe](https://docs.typesafe.ai) API key | Optional: the Judgment approves plans and handles stuck sessions | `TYPESAFE_API_KEY`, or paste it at `harness init` |

Without a TypeSafe key, the Harness works the same but asks you instead: every plan approval and every stuck session becomes a Question in the Shell.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/muresanroland/harness/main/install.sh | sh
```

This installs the latest release as `~/.local/bin/harness`. Set `HARNESS_INSTALL_DIR` to install somewhere else. Keep that directory writable by you and on your `PATH`.

**Updates are automatic.** A release build checks GitHub Releases each time the Shell opens, and once a day while it stays open. It downloads a newer version and swaps it in when no run is live in that repo. `harness --version` prints the version, for example `v1.0.0`.

To build from source instead, run `cargo install --git https://github.com/muresanroland/harness`. That gives a `-dev` build, which never updates itself.

## Set up a Target repo

A Target repo is the repository whose Epic you want worked on. Once, from a herdr pane inside it:

```bash
harness init
```

`init` does three things:
- It installs the Stage skills and `create-pr` into `.agents/skills`, linked from `.claude/skills`. They are yours to edit from then on, and running `init` again asks before touching them.
- It asks for the TypeSafe key. Press Enter to use `TYPESAFE_API_KEY`, or paste a key, which it keeps in `.harness/typesafe-key`.
- It runs a preflight that reports anything still missing: the `bd` workspace, `gh` auth, the git remote, the `create-pr` skill, or herdr.

`.harness/` is added to `.gitignore`.

## Run an Epic

1. Create the Epic and its Tickets in beads. Each Ticket should say what to build and how to check it. Blockers work as usual: a Ticket that depends on another waits for that Ticket's PR to merge.

   ```bash
   epic=$(bd create --silent --type=epic --title="...")
   bd create --parent "$epic" --type=task --title="..." --description="..." --acceptance="..."
   ```

2. Open the Shell from a herdr pane in the Target repo:

   ```bash
   harness
   ```

   It shows the open Epics and their Tickets. Type `/start-epic`, then press Tab to complete the Epic from its id or part of its title.

3. Watch progress in the Shell. Each Ticket's panes appear in its own herdr tab. Answer Questions as they come up: plan approvals, sessions waiting at a prompt, and Wakes the Judgment wasn't sure about.

4. Review and merge the PRs on GitHub. The Harness closes each merged Ticket and starts the Tickets that were waiting on it. When every Ticket is closed, the Epic is done.

### Shell commands

| Command | What it does |
|---|---|
| `/start-epic <epic> [--max N]` | Run every Ticket of the Epic, at most N at once (default 3) |
| `/start-ticket <ticket>` | Run one Ticket |
| `/continue` | Resume the saved run, e.g. after `/stop-work` or a restart |
| `/stop-work` | Stop scheduling. Agent panes keep running and the state is saved |
| `/retry <ticket>` | Rerun the Ticket's failed Stage with a fresh session |
| `/park <ticket>` | Take the Ticket out of the pipeline; the others keep going |
| `/address <ticket>` | Act on the review comments or merge conflicts on the Ticket's open PR |
| `/questions` | Show the Questions waiting for you |
| `/exit` | Leave the Shell (asks first during a run). Ctrl-C twice does the same |

Leaving the Shell never kills agent panes.

### What it writes

Everything goes under `.harness/` in the Target repo:
- `orchestrator.log`: one line per event.
- `state.json`: the run, so it can be resumed.
- `runs/<ticket>/`: each Stage's results, diffs and Debate transcripts.
- `worktrees/`: one worktree per Ticket.

## Releasing

Cargo.toml's `version` is the source of truth. After the merge, push the matching `vX.Y.Z` tag, or create the release on GitHub. Either one starts the release workflow, which builds both binaries on GitHub and attaches them to the release. To rebuild an existing tag, run the workflow from the Actions tab (**release**, then **Run workflow**) with that tag, or run `gh workflow run release.yml -f tag=vX.Y.Z`. Feature tickets bump minor, fixes bump patch.

## Build

See the build and test commands in [CLAUDE.md](CLAUDE.md).
