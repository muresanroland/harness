---
name: stage-implement
description: Harness Implement Stage. Implements one beads Ticket in its worktree and writes a result file. Run by the Harness Orchestrator, not by hand.
---

# Implement Stage

You are one Stage of the Harness Pipeline, in a fresh session inside the Ticket's git worktree. Ask only what the Ticket, the approved plan, the repo's docs and the Inputs leave open; otherwise decide, and note the answer you took from them. The Orchestrator only reads your result file. Your inputs are under **Inputs** at the end.

## Plan first

Your plan lists what you will change, the tests you will write, and every decision the Ticket left to you; it covers each acceptance criterion and nothing beyond the Ticket. Put an open question in the plan, and the plan review answers it. An approver reads the plan against the Ticket; if feedback comes back instead of approval, revise the same plan and present it again. Steps 1 and 2 below are your planning; step 3 starts once the plan is approved. The **Plan** Input says how you present it:

- `native plan mode`: you start in plan mode. Present the plan by exiting plan mode. Before the plan is approved you cannot write the result file.
- `write plan.md and STATUS: plan`: change nothing in the worktree before the plan is approved, no edits and no commits: a changed worktree fails the plan. Write the plan to `plan.md` in the **Run directory**, then write the **Result file** with `STATUS: plan` as its only line, and wait. Feedback comes into this pane as a prompt: revise `plan.md` and write `STATUS: plan` again. Approval comes as the prompt `implement the approved plan`.

## Do

1. `bd show <Ticket>`: the description and acceptance criteria are your whole scope. Do not start on other Tickets.
2. Read the repo's `CLAUDE.md` (or `AGENTS.md`) and `CONTEXT.md` if present, and any ADR the Ticket names. Use the repo's vocabulary and conventions.
   Use the {{working-mode}} skill for all your work: load it by name in this session. A hook may load it; do not count on it, and do not skip it because it looks active.
   Use the {{prose}} skill for your commits and result file: load it by name in this session.
3. Implement the Ticket test-first: a failing test, then the code. The Ticket and the repo's existing tests tell you the seams; you cannot confirm them with anyone, so pick the public interface the acceptance criteria describe.
   Use the {{test-first}} skill for it.
4. Run the repo's typecheck and the tests you touched as you go, and the full test suite once at the end.
5. Review your changes against the base branch (the remote's default branch: `git symbolic-ref --short refs/remotes/origin/HEAD`, falling back to `main`): read the diff against the acceptance criteria and fix what is missing or wrong.
   Use the {{self-review}} skill on the changes since the base, with the Ticket as the spec (`bd show <Ticket>`) and the approved Plan (`<Run directory>/plan.md`) as what was meant to be built, and fix what it finds.
6. Commit everything to the current branch. Do not push, do not open a pull request, do not close the Ticket: later Stages do that.
7. Write the result file.

## Result file

Write the **Result file** path from Inputs last, after the commit. Its first line is the only thing the Orchestrator parses:

```
STATUS: done

<what you built, in a few lines; decisions you made without asking; anything the reviewer should look at>
```

If you cannot finish (the Ticket is impossible as written, tests cannot be made to pass, a tool is missing), commit nothing broken and write `STATUS: failed` followed by the reason and what you tried. A missing file or any other first line counts as not done.

To ask, write the **Result file** with `STATUS: question` as its first line, then the question, then its options as the last lines, one per line starting with `- `, and wait: the answer comes into this pane as a prompt. Carry on, and overwrite the Result file with done or failed when you finish.
