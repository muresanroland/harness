# Issue tracker: beads (bd)

Issues and specs for this repo live in beads, a local Dolt database driven by the `bd` CLI (see AGENTS.md; `bd prime` prints the full workflow). The GitHub remote carries code and PRs, not issues.

## Conventions

- **Create an issue**: `bd create --title="..." --type=task|bug|feature|epic --priority=2 --body-file=-` with the body on stdin (a heredoc), plus `--acceptance`, `--labels`, `--parent <id>` as needed. Priority is 0-4, never "high".
- **Read an issue**: `bd show <id>`; its NOTES often hold the handoff. `bd show <id> --json --include-comments` adds the comments.
- **List issues**: `bd list --status=open --json`, filtered with `--label`, `--parent`, `--status`. `bd ready` lists unblocked work.
- **Comment on an issue**: `bd comments add <id> "..."`
- **Apply / remove labels**: `bd label add <id> <label>` / `bd label remove <id> <label>`
- **Close**: `bd close <id> --reason="..."`
- Never `bd edit` (it opens $EDITOR). Never `bd dolt push` unless asked.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(beads holds no PRs; external GitHub PRs are not triaged.)_

## When a skill says "publish to the issue tracker"

Create a bead with `bd create`.

## When a skill says "fetch the relevant ticket"

Run `bd show <id>`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is an epic with **child** beads as tickets.

- **Map**: an epic labelled `wayfinder:map` (`bd create --type=epic --labels=wayfinder:map`), its description holding the Notes / Decisions so far / Fog sections.
- **Child ticket**: `bd create --parent <map> --labels=wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`); child ids read `<map>.<n>`.
- **Blocking**: `bd dep add <child> <blocker>`; `bd show` lists the blockers under DEPENDS ON. A ticket is unblocked when every blocker is closed.
- **Frontier query**: `bd ready --parent <map> --unassigned --json`; first in map order wins.
- **Claim**: `bd update <id> --claim`, the session's first write.
- **Resolve**: `bd comments add <id> "<answer>"`, then `bd close <id> --reason="..."`, then append a context pointer (gist + id) to the map's Decisions so far by rewriting its description with `bd update <map> --body-file -`.
