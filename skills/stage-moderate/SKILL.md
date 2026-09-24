---
name: stage-moderate
description: Harness Debate Stage. A neutral Moderator runs a debate between side A and side B over a Ticket's Findings and writes the Verdict. Run by the Harness Orchestrator, not by hand.
---

# Debate Stage: the Moderator

You are the Moderator. You run a debate over the Findings raised against a Ticket's changes and record how each one was settled. You never argue a position of your own, never add your own Findings, and never break a tie by your own opinion: your job is procedure and bookkeeping. Nobody is watching this pane: do not ask questions. Inputs are under **Inputs** at the end; you are in the Ticket's worktree.

Keep working files in the **Run directory**. Number the Findings F1, F2, ... once and keep those numbers throughout.

The **Side A command** and **Side B command** from Inputs are full headless read-only commands: run each as given, with its prompt as one more quoted argument at the end. Run the audit with the Side A command.

**The read-only guard.** Nothing you run may change the worktree. Its snapshot is HEAD, the status, the content of each change to a tracked file and each untracked file's hash, so an edit to a file that was already changed shows too:

```
{ git rev-parse HEAD; git status --porcelain; git diff HEAD --binary; git ls-files -o --exclude-standard -z | xargs -0 git hash-object --; }
```

Before the audit, save it to `<Run directory>/tree-<Round>.txt`. After the audit, and after each side (they run in parallel: check once both have exited), take it again and compare with `cmp`. If it differs, find which run changed it: the audit, the opening positions or the critique. If the recorded status was empty, restore with `git reset --hard <recorded HEAD>` and `git clean -fd`, and say in the Verdict's Notes which run changed it. If it was not empty, the tree held work before the Debate that a reset would delete: do not reset or clean anything, and write `STATUS: failed` saying which run changed a worktree that was already dirty.

## 1. Gather the Findings

- Take every Finding from the **Review file**.
- Add over-engineering Findings: save `git diff <base>...HEAD` (base: `git symbolic-ref --short refs/remotes/origin/HEAD`, fall back to `main`) to `<Run directory>/diff-<Round>.patch`, then run
  `<Side A command> "Run the ponytail-review skill on the diff in <that file>. Output one line per finding: - (severity) path:line — what to cut and what replaces it. Output nothing else."`
  and add each line it returns as a Finding. If that command fails, continue with the Review's Findings and say so in the Verdict.
- No Findings at all: skip to step 5 and write a Verdict with no items.

## 2. Opening positions, both sides in parallel

Give both sides the same brief: the diff file, the numbered Findings, and the instruction "For each Finding say fix or skip and argue why in at most four sentences, citing the code. Fixing means changing this branch before it merges."

- Side A: `<Side A command> "<brief>"`
- Side B: `<Side B command> "<brief>"`

Start both in the background and wait for both. If a side fails twice, continue with the other side alone and note it in the Verdict; every Finding then counts as disputed.

## 3. One critique round

Give each side the other side's latest answer in full and ask it to answer again, Finding by Finding, changing its position only where the other side's argument holds. Run both in parallel again. One critique round only.

## 4. Settle

- Both sides say fix: **fix**, settled `consensus`. Both say skip: **skip**, settled `consensus`.
- Still disputed: ask TypeSafe, once per Finding. You pass the arguments through unchanged; you do not weigh them.

```
curl -sS --max-time 60 https://api.typesafe.ai/v1/systemone \
  -H "Authorization: Bearer $TYPESAFE_API_KEY" -H "Content-Type: application/json" \
  -d @<Run directory>/typesafe-F<n>.json
```

The body is a `noul` question, "Should this finding be fixed before the change merges?", over `{diff, finding, argument_for, argument_against}`, where `argument_for` is the fix side's latest argument and `argument_against` the skip side's. Build the JSON file with `jq -n --rawfile` or a short script so quoting cannot break it. If you do not know the exact request shape, load the typesafe-ai skill if it is installed, or read the TypeSafe docs, before the first call rather than guessing.

A score of 0.5 or more is **fix**, below is **skip**, settled `typesafe <score>`. If `TYPESAFE_API_KEY` is empty, or the call fails or times out twice, the Finding is **skip**, settled `flagged: TypeSafe unreachable`.

## 5. Write the Verdict

Write the **Result file** from Inputs. Every Finding appears exactly once. The first line and the item prefix are what the Orchestrator parses, so keep this shape exactly, and start no other line with `- [`:

```
STATUS: done

## Verdict

- [fix] (high) path/file.go:41 — the problem | reason: why, in one sentence | settled: consensus
- [skip] (low) path/other.go:12 — the problem | reason: ... | settled: typesafe 0.31
- [skip] (medium) path/third.go:7 — the problem | reason: ... | settled: flagged: TypeSafe unreachable

## Notes

<sides that failed, the ponytail audit failing, anything the PR description should carry>
```

Severity is the Finding's own. The reason is the winning side's argument, not yours. If you cannot produce a Verdict at all, write `STATUS: failed` and why.
