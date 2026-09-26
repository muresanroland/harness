<p align="center">
  <img src="assets/brand/orqadence-banner.gif" alt="Orqadence — terminal orchestrator" width="640">
</p>

Drives a [beads](https://github.com/gastownhall/beads) Epic through a fixed multi-agent pipeline, from Tickets to open pull requests, with every agent visible in [herdr](https://herdr.dev).

Each Ticket gets its own git worktree and herdr tab. It passes through:

1. **Implement**: Claude Code writes a plan, which is approved before any edit, then implements it.
2. **Review**: Codex reviews the change.
3. **Debate**: a Claude moderator settles each Finding as fix or skip.
4. **Fix**: Claude Code applies the fix items.

Review, Debate and Fix repeat for up to 3 rounds, then a pull request opens. You merge it. Orqadence then closes the Ticket and cleans up its worktree. Vocabulary: [CONTEXT.md](CONTEXT.md).

## Requirements

- macOS on Apple Silicon or Linux x86_64 (other platforms: build from source)

| Tool | Used for | Setup |
|---|---|---|
| [herdr](https://herdr.dev) | Runs every agent in a visible pane | Start `orqa` from a pane inside herdr (`HERDR_ENV=1`) |
| [beads (`bd`)](https://github.com/gastownhall/beads) | The Epic and its Tickets | `bd init` in the Target repo, which `orqa init` offers to run |
| [git](https://git-scm.com) | Branches and worktrees | The repo needs a remote on GitHub |
| [GitHub CLI (`gh`)](https://cli.github.com) | Opens PRs, watches for merges | `gh auth login` |
| [Claude Code (`claude`)](https://claude.com/claude-code) | Implement, Debate, Fix, Address | Logged in |
| [Codex CLI (`codex`)](https://github.com/openai/codex) | Review | Logged in |
| [TypeSafe](https://docs.typesafe.ai) API key | Optional: the Judgment approves plans and handles stuck sessions, and the Debate settles disputed Findings | `TYPESAFE_API_KEY`, or say yes and paste it at `orqa init` |

With TypeSafe off, Orqadence works the same but asks you instead: every plan approval and every stuck session becomes a Question in the Shell, and a Finding the Debate's sides still dispute is skipped and listed in the PR.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/muresanroland/orqadence/main/install.sh | sh
```

This installs the latest release as `~/.local/bin/orqa`. Set `ORQADENCE_INSTALL_DIR` to install somewhere else. Keep that directory writable by you and on your `PATH`.

**Updates are automatic.** A release build checks GitHub Releases each time the Shell opens, and once a day while it stays open. It downloads a newer version and swaps it in when no run is live in that repo. `orqa --version` prints the version, for example `v1.0.0`.

To build from source instead, run `cargo install --git https://github.com/muresanroland/orqadence`. That gives a `-dev` build, which never updates itself.

## Set up a Target repo

A Target repo is the repository whose Epic you want worked on. Once, from a herdr pane inside it:

```bash
orqa init
```

`init` does these things:
- It asks where the skills go:
  - this checkout, uncommitted: this is the default, and what a non-interactive `init` takes. The skills go in `.orqadence/skills`, and each Ticket's worktree gets links to them that git ignores.
  - the repo, committed: the skills go in `.agents/skills`, linked from `.claude/skills`. You commit them.
  - user level: the skills go in `~/.agents/skills`, linked from `~/.claude/skills`.

  Running `init` again asks again, with the current place as the default, and moves the skills it installed.
- It installs the Stage skills and `create-pr` there. They are yours to edit from then on, and running `init` again asks before touching them. If the repo already has them committed in `.agents/skills`, those stay.
- With no `bd` workspace, it offers to run `bd init`.
- It offers to write the beads `docs/agents` setup the skills read, only what is missing: `docs/agents/issue-tracker.md`, `triage-labels.md`, `domain.md`, and an Agent skills block in `CLAUDE.md`, or `AGENTS.md` when there is no `CLAUDE.md`. A non-interactive `init` writes them too.
- It asks whether to use TypeSafe, yes by default. Yes asks for the key, which it keeps in `.orqadence/typesafe-key`, and installs the `typesafe-ai` skill; an empty key is no. With `TYPESAFE_API_KEY` set it is on without asking. Without it, a non-interactive `init` leaves TypeSafe on only where it is already on with a kept key, and off everywhere else. On or off is kept in `.orqadence/config.json`.
- It installs each job's default skill, pinned by commit: `tdd`, `code-review`, `ponytail`, `caveman`, `ponytail-review` and `resolving-merge-conflicts`. At user level, a skill of the same name you already have stays as it is.
- It offers to install herdr's integration for claude or codex when it is not installed or outdated, listing what each install writes. Without it herdr does not know a session's id, so `/continue` starts those Stages fresh instead of resuming them.
- It runs a preflight that reports anything still missing: the `bd` workspace, `gh` auth, the git remote, the `create-pr` skill, a job's skill, an App a Stage runs on that is not on `PATH`, or herdr. It also warns about a personal skill that shadows an installed one on Claude, and about the superpowers plugin being enabled.

`.orqadence/` is added to `.gitignore`.

## Run an Epic

1. Create the Epic and its Tickets in beads. Each Ticket should say what to build and how to check it. Blockers work as usual: a Ticket that depends on another waits for that Ticket's PR to merge.

   ```bash
   epic=$(bd create --silent --type=epic --title="...")
   bd create --parent "$epic" --type=task --title="..." --description="..." --acceptance="..."
   ```

2. Open the Shell from a herdr pane in the Target repo:

   ```bash
   orqa
   ```

   It shows the open Epics and their Tickets. Type `/` to pick a command, then `@` to pick an Epic or Ticket by its id or part of its title; Tab or Enter fills in the one under the cursor.

3. Watch progress in the Shell. Each Ticket's panes appear in its own herdr tab. Answer Questions as they come up: plan approvals, sessions waiting at a prompt, and Wakes the Judgment wasn't sure about.

4. Review and merge the PRs on GitHub. Orqadence closes each merged Ticket and starts the Tickets that were waiting on it. When every Ticket is closed, the Epic is done.

### Shell commands

| Command | What it does |
|---|---|
| `/start-epic <epic> [--max N]` | Run every Ticket of the Epic, at most N at once (default 3) |
| `/start-ticket <ticket>` | Run one Ticket |
| `/continue [<ticket>]` | Resume the saved run, e.g. after `/stop-work` or a restart. With a Ticket: unpark that Ticket at its Stage and put its question to you first |
| `/stop-work` | Stop scheduling. Agent panes keep running and the state is saved |
| `/retry <ticket>` | Rerun the Ticket's failed Stage with a fresh session |
| `/park <ticket>` | Take the Ticket out of the pipeline; the others keep going |
| `/address <ticket>` | Act on the review comments or merge conflicts on the Ticket's open PR |
| `/questions` | Show the Questions waiting for you |
| `/config` | Pick the App, model and effort each Stage runs on, and a plan model other than Implement's; every change saves at once to `.orqadence/config.json`, and during a run the Stages that start after it use it. A change that breaks a check (the Review on Implement's model, the Debate's sides in one family) is refused; the Apps page shows which Apps are installed. Each Stage's page also picks its jobs' Delegate skills; the Skills page adds (`a`), updates (`u`, `U` for all) and removes (`d`) the skills Orqadence installed; the TypeSafe page turns TypeSafe on or off and asks the key when there is none |
| `/away` | Toggle Away: a Stage's question parks its Ticket, with a bd comment, until you `/continue @ticket` it |
| `/summary [<epic>]` | Show the Epic's PRs, Rounds and Findings; it also opens by itself once every Ticket has its PR or is Parked |
| `/demo` | Play a made-up run to see the Shell at work: Tickets moving through their Stages, RECENT filling, each kind of Question waiting for your answer, a Ticket waiting on a merge, a usage limit, and the Epic summary at the end. Nothing starts and nothing is written |
| `/stop-demo` | End the demo; the Shell is back as it was. `/stop-work` does the same |
| `/exit` | Leave the Shell (asks first during a run). Ctrl-C twice does the same |

Leaving the Shell never kills agent panes.

### What it writes

Everything goes under `.orqadence/` in the Target repo:
- `orchestrator.log`: one line per event.
- `state.json`: the run, so it can be resumed.
- `runs/<ticket>/`: each Stage's results, diffs and Debate transcripts.
- `worktrees/`: one worktree per Ticket.

## Releasing

Cargo.toml's `version` is the source of truth. After the merge, push the matching `vX.Y.Z` tag, or create the release on GitHub. Either one starts the release workflow, which builds both binaries on GitHub and attaches them to the release. To rebuild an existing tag, run the workflow from the Actions tab (**release**, then **Run workflow**) with that tag, or run `gh workflow run release.yml -f tag=vX.Y.Z`. Feature tickets bump minor, fixes bump patch.

## Build

See the build and test commands in [AGENTS.md](AGENTS.md).
