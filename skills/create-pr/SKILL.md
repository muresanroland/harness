---
name: create-pr
description: Open a pull request for the current branch in any repo - find and run the repo's own checks, push, and create the PR with gh. Use when a ticket's work is committed and ready for review. Never merges; merging is human-only.
---

# create-pr

Open a pull request for the current branch. This skill ends at "PR created".

**You never merge.** No `gh pr merge`, no `--auto`, no merge into the default branch, no
approving your own PR. Merging is the human's.

Nothing here assumes a stack. The repo's own conventions win over everything below: find them
in step 1 and follow them. You may be running unwatched (the Orqadence Fix Stage runs this in a
worktree), so never ask a question you can decide: choose, and say what you chose in the PR body.

## 1. Find the repo's checks

Read until you find how this repo builds and tests, then stop looking:

- `CLAUDE.md` / `AGENTS.md`, a "Build and test" or "Commands" section
- `README.md` / `CONTRIBUTING.md`
- the build file itself: `Makefile`, `package.json` scripts, `go.mod`, `Cargo.toml`,
  `pyproject.toml`, `mix.exs`, `build.gradle`, `justfile`
- `.github/workflows/*.yml`: what CI runs on a pull request is the gate you have to pass

## 2. Run them

Run the build, the typecheck or lint, and the tests, over the whole repo unless the repo
documents a scoped variant (`--affected`, `--since`, `--changed`). A red gate stops the PR: fix
it and re-run. Never skip a check because it looks unrelated to the diff, and never weaken a
check to make it pass. If the repo has no checks at all, note that in the PR body instead of
inventing one.

## 3. Check hygiene

- Not on the default branch: `git symbolic-ref --short refs/remotes/origin/HEAD` (fall back to
  `main`). If the work is on it, stop and tell the user; do not open a PR from it.
- Nothing uncommitted: `git status --porcelain` is empty.
- The branch has commits the default branch lacks, and its messages say what changed and why.
- If the repo keeps decision or vocabulary docs (`CONTEXT.md`, `docs/adr/`, an equivalent) and
  this diff changed a domain term or a decision, they are updated in this same branch.

## 4. Push

`git push -u origin HEAD`

## 5. Create the PR

Write the body to a file and pass `--body-file`, so markdown and quotes survive the shell.

If the repo has a template (`.github/pull_request_template.md`, `docs/`, or
`.github/PULL_REQUEST_TEMPLATE/`), fill every section of it. Otherwise:

```
## What      one or two sentences, concrete
## Why       the ticket or decision behind it
## Impact    what a reviewer should look at; call out migrations, public API or contract
            changes, security- or auth-adjacent paths, anything not covered by the checks
## Testing   the exact commands from step 2 and their result
## Ticket    the issue this closes
```

Link the ticket the way the tracker needs: `Closes #<n>` only closes a GitHub issue, so for any
other tracker (a beads id, Jira, Linear) name the id plainly instead. Base the PR on origin's
default branch unless the repo documents an integration branch.

## 6. Report and stop

Print the PR URL. Add a reviewer with `gh pr edit --add-reviewer` only if the repo names one
(CODEOWNERS is applied by GitHub on its own). Do not merge, do not enable auto-merge, do not
dismiss reviews.
