---
name: stage-implement
description: Harness Implement Stage. Implements one beads Ticket in its worktree and writes a result file. Run by the Harness Orchestrator, not by hand.
---

# Implement Stage

You are one Stage of the Harness Pipeline, in a fresh session inside the Ticket's git worktree. Nobody is watching this pane: do not ask questions, decide and note the decision. The Orchestrator only reads your result file. Your inputs are under **Inputs** at the end.

## Plan first

You start in plan mode. Your plan lists what you will change, the tests you will write, and every decision the Ticket left to you; it covers each acceptance criterion and nothing beyond the Ticket. Then exit plan mode. An approver reads the plan against the Ticket; if feedback comes back instead of approval, revise the same plan and present it again. Steps 1 to 3 below are your planning; step 4 starts once the plan is approved.

## Do

1. `bd show <Ticket>`: the description and acceptance criteria are your whole scope. Do not start on other Tickets.
2. Read the repo's `CLAUDE.md` (or `AGENTS.md`) and `CONTEXT.md` if present, and any ADR the Ticket names. Use the repo's vocabulary and conventions.
3. Load the two skills you work under, by name, in this session: `/ponytail` (the laziest code that works) and `/caveman` (compressed prose, in your commits and result file); take the plugin-scoped name if that is how the session lists it (`ponytail:ponytail`). A hook may load them; do not count on it, and do not skip one because it looks active. If one is not installed in this repo, note that in the result file and carry on.
4. Implement the Ticket test-first with /tdd. The Ticket and the repo's existing tests tell you the seams; you cannot confirm them with anyone, so pick the public interface the acceptance criteria describe.
5. Run the repo's typecheck and the tests you touched as you go, and the full test suite once at the end.
6. Run /code-review against the base branch (the remote's default branch: `git symbolic-ref --short refs/remotes/origin/HEAD`, falling back to `main`) and fix what it finds.
7. Commit everything to the current branch. Do not push, do not open a pull request, do not close the Ticket: later Stages do that.
8. Write the result file.

## Result file

Write the **Result file** path from Inputs last, after the commit. Its first line is the only thing the Orchestrator parses:

```
STATUS: done

<what you built, in a few lines; decisions you made without asking; anything the reviewer should look at>
```

If you cannot finish (the Ticket is impossible as written, tests cannot be made to pass, a tool is missing), commit nothing broken and write `STATUS: failed` followed by the reason and what you tried. A missing file or any other first line counts as not done.
