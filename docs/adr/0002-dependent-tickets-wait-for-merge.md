# Dependent Tickets wait for the merge, not for the pull request

A Ticket's Pipeline ends with an open pull request, but the beads ticket closes only when that pull request is merged, so `bd ready` keeps dependent Tickets blocked until a human has merged what they build on; they then branch from the updated main. Stacked pull requests (branching a dependent from the unmerged branch) were rejected because review changes to the lower pull request force rebases of everything above it, and a Ticket with two dependencies has no single base. A shared epic branch that the Harness merges into was rejected because it merges code no human has looked at.

## Consequences

- An Epic pauses at every dependency edge until the user merges. Runs last days, which is why the Orchestrator keeps a state file and resumes.
- The Orchestrator polls `gh` for merges; on merge it closes the Ticket, removes the worktree and deletes the branch.
- Parallel Tickets all branch from main, so merged work can conflict with open pull requests; the Orchestrator detects and reports this, and `harness address <ticket>` resolves it on request.
