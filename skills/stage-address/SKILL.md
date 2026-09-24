---
name: stage-address
description: Harness address Stage. Acts on a Ticket's pull request review comments and merge conflicts, then pushes to the same PR. Run by the Harness Orchestrator on the user's command, not by hand.
---

# Address Stage

You are in a fresh session inside the kept worktree of a Ticket whose pull request is open. A human reviewed it, or merged work now conflicts with it. Ask only what the Ticket, the review comments, the repo's docs and the Inputs leave open; otherwise decide, and note the answer you took from them. Inputs are under **Inputs** at the end.

## Do

1. Read the feedback. **Review comments (gh JSON)** holds the PR's reviews and conversation comments. Inline review comments are not in it; fetch them with `gh api repos/{owner}/{repo}/pulls/<number>/comments --paginate` (`{owner}` and `{repo}` are filled in by gh). `bd show <Ticket>` reminds you what the change is for.
2. If **Conflicts with main** is yes: `git fetch origin`, then rebase the branch onto the remote's default branch (`git symbolic-ref --short refs/remotes/origin/HEAD`, fall back to `origin/main`) using the resolving-merge-conflicts skill. Keep both sides' intent; never resolve a conflict by dropping the other change.
3. Apply each requested change. A comment that asks a question, or that you judge wrong, gets no code change: answer it in the result file instead and let the human decide.
4. Run the repo's tests until they pass. Commit.
5. Push to the same PR: `git push` normally, `git push --force-with-lease` after a rebase. Never open a second PR, never merge, never close the Ticket.
6. Check `gh pr view <PR> --json mergeable` reports `MERGEABLE` (GitHub may need a few seconds after the push).

## Result file

Write the **Result file** from Inputs last. The Orchestrator parses only the first line:

```
STATUS: done

<per comment: what you changed, or your answer; whether you rebased; the PR's mergeable state>
```

Write `STATUS: failed` with the reason if the tests cannot be made to pass, the rebase cannot be completed sensibly, or the push is rejected.

To ask, write the **Result file** with `STATUS: question` as its first line, then the question, then its options as the last lines, one per line starting with `- `, and wait: the answer comes into this pane as a prompt. Carry on, and overwrite the Result file with done or failed when you finish.
