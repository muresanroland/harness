---
name: create-pr
description: Open a pull request for the current branch using the repo PR template. Use whenever a ticket's implementation is complete and reviewed with /code-review. Never merges — merging is human-only.
---

# create-pr

Open a PR for the current branch. This skill ends at "PR created, review requested".
**You never merge.** `gh pr merge`, merge commits into `main`, and approving your own PR are
all forbidden (also blocked by hooks and branch protection).

## Steps

1. **Verify the ship gate locally** (ADR-0014/0017): `pnpm gate:affected` — the same program
   CI runs, scoped to what this branch changed. If it fails: stop, fix, re-run. Do not open a
   PR on a red gate.
2. **Check hygiene**: branch is not `main`; migrations in the diff are new files only
   (append-only, ADR-0014); if the diff changed domain terms or decisions, `CONTEXT.md` /
   `docs/adr/` were updated in this same branch.
3. **Push** the branch.
4. **Create the PR** with `gh pr create`, filling `.github/pull_request_template.md` fully:
   - **What**: concrete change, one or two sentences.
   - **Why**: the motivating ticket/decision; cite ADR numbers where relevant.
   - **Impact**: apps/packages touched; flag explicitly if the diff contains a **Drizzle
     migration**, a **`packages/core` Zod schema change** (contract change → `api-client`
     regenerated), or touches **auth/RLS/organization_id paths**.
   - **Ticket**: `Closes #<issue>` — every PR closes exactly one ticket from the build plan.
5. **Request review from the human** and report the PR URL. If the diff touched tenancy/auth
   or schema, note that the matching reviewer subagent
   (`tenancy-security-reviewer` / `schema-contract-reviewer`) was already run in step 0 of
   your implementation loop — if it wasn't, run it now and post findings as a PR comment.
6. **Stop.** Do not merge, do not enable auto-merge, do not dismiss reviews.
