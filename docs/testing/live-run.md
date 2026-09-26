# Controlled live Orqadence test

The live run on the Rust binary, driven through the Shell. It is the merge gate
for the Rust port (harness-kqe.5) and the Shell running the Orchestrator
(harness-kqe.10), and it gives plan mode (harness-kqe.13) its one real
plan-mode Implement. A green `cargo test` does not establish any of this: the
fake world never starts a real session.

It runs in `~/Documents/Projects/test-harness-repo`. The human launches the
Shell, answers its Questions and merges PRs. The preparation in step 0 launches
no agent; an agent may run it.

The run has two phases:

- **With the TypeSafe key**, Tickets A, B and C: each plan goes to the plan Judgment,
  and Wakes go to the Wake Judgment. This is the configuration users run.
- **Without the key**, from the resume onward (D and whatever is still running):
  every plan and Wake becomes a Question. This is the only reliable way to use
  the plan Question's feedback path, because a Judgment that approves every plan
  never asks.

## 0. Prepare

### 0.1 Build the binary

```bash
cd ~/Documents/Projects/orqadence
cargo build --release     # target/release/orqa
cargo test
```

The binary is `../orqadence/target/release/orqa` from inside the target repo.
It reports `v1.0.0-dev` and a dev build never self-updates. The globally
installed `harness` is the old Go build, which has no version and no
updater: do not use it for this run.

### 0.2 Archive the earlier run in the target repo

The Go binary ran Epic `test-harness-repo-6fs`. Its PRs #3, #4 and #5 are still
open and are not part of this run: leave them alone, and do not merge them
during the run. Bring `main` up to date (A of that run merged as PR #2), then
move that run's files aside so the new Epic starts from clean state:

```bash
cd ~/Documents/Projects/test-harness-repo
git pull --ff-only
a=.orqadence/archive/2026-09-23-go-regression-run
mkdir -p "$a"
mv -f .orqadence/state.json .orqadence/orchestrator.log .orqadence/runs .orqadence/control .orqadence/live-test.json "$a"/
```

The old worktrees under `.orqadence/worktrees` stay, and so do their branches.
They belong to the earlier run, not to this cleanup.

### 0.3 Refresh the Stage skills

The installed `stage-implement` skill predates plan mode: it lacks the "Plan
first" section. The other shipped skills already match. From a Herdr shell pane
(the preflight wants `HERDR_ENV=1`), in the target repo:

```bash
../orqadence/target/release/orqa init
```

- At the gate ("the shipped skills are already installed here"), choose
  **overwrite everything with the shipped skills**.
- If asked about create-pr, keep the shipped one; it is identical.
- If asked for a TypeSafe key, press Enter when `TYPESAFE_API_KEY` is exported
  in the shell you will launch from. Otherwise paste it; init keeps it in
  `.orqadence/typesafe-key`.

The preflight should report nothing missing. Check the refresh landed:

```bash
grep -c "Plan first" .orqadence/skills/stage-implement/SKILL.md   # 1, in the checkout location
```

The leftover `start-work` skill is unused by the Shell. Leave it.

### 0.4 Prepare the Epic

Four Tickets, with D blocked by A. Run this in the target repo, then note the
printed ids: the Shell takes them.

```bash
rules='This ticket is part of the approved controlled live Orqadence test. Keep changes limited to the named files, with Go standard library only. Read AGENTS.md and run its build, vet, and race-test gates. The run authorizes commits and pushing/opening one PR on this ticket branch. Never merge a PR or close a ticket yourself; the human merges and the orchestrator closes. Do not work on other epics or the open PRs #1, #3, #4 and #5. Do not add unrelated improvements or alter agent/skill configuration.'
label=orqadence-live-20260923

epic=$(bd create --silent --type=epic --priority=2 --labels=$label \
  --title="Controlled live run on the Rust binary: plan mode, Questions, recovery, merge dependency" \
  --description="Four small tickets for the Rust Orqadence live run (docs/testing/live-run.md in the orqadence repo). D waits for A to merge.")

a=$(bd create --silent --parent "$epic" --type=task --priority=1 --labels=$label \
  --title="slug.Make turns text into a lowercase hyphenated slug" \
  --description="Add slug/slug.go and slug/slug_test.go. Export Make(text string) string: ASCII letters are lowercased, ASCII letters and digits are kept, and every run of other characters becomes one hyphen, with none leading or trailing. Do not change main.go.

$rules" \
  --acceptance='Make("")=""; Make("Hello, World!")="hello-world"; Make("  a--b  ")="a-b"; Make("Go 1.23")="go-1-23". Tests cover all cases. main.go unchanged.')

b=$(bd create --silent --parent "$epic" --type=task --priority=2 --labels=$label \
  --title="reverse.String reverses text by rune" \
  --description="Add reverse/reverse.go and reverse/reverse_test.go. Export String(text string) string: the text's runes in reverse order. Do not change main.go.

$rules" \
  --acceptance='String("")=""; String("abc")="cba"; String("héllo")="olléh". Tests cover all cases. main.go unchanged.')

c=$(bd create --silent --parent "$epic" --type=task --priority=2 --labels=$label \
  --title="vowels.Count counts the vowels in text" \
  --description="Add vowels/vowels.go and vowels/vowels_test.go. Export Count(text string) int: how many of a, e, i, o and u the text holds, in either case. Do not change main.go.

$rules" \
  --acceptance='Count("")=0; Count("Orqadence")=4; Count("AEIOU xyz")=5. Tests cover all cases. main.go unchanged.')

d=$(bd create --silent --parent "$epic" --type=task --priority=1 --labels=$label --deps "$a" \
  --title="The command prints slug.Make of an explicit --slug flag" \
  --description="Change main.go and add main_test.go: move the logic into run(args []string, out io.Writer) error, using the flag package. With --slug <text> it prints slug.Make(text) and a newline; without the flag it still prints hello, world. Uses slug from $a, which must be merged first.

$rules" \
  --acceptance='No flag prints "hello, world"; --slug "Hello, World!" prints "hello-world"; --slug "" prints an empty line. main_test.go covers all three through run.')

echo "epic=$epic A=$a B=$b C=$c D=$d"
bd show "$d"    # DEPENDS ON lists A
```

## Steps for the human

### 1. Open the Shell with the key

Open a shell pane in the Herdr workspace where the Ticket tabs should appear.
The checks below report missing variables without printing the key.
`TYPESAFE_API_KEY` may instead come from `.orqadence/typesafe-key` (step 0.3);
drop that line if it does.

```bash
cd ~/Documents/Projects/test-harness-repo
(
  : "${HERDR_ENV:?Open a Herdr terminal pane first}"
  : "${HERDR_WORKSPACE_ID:?This shell needs a Herdr workspace ID}"
  : "${TYPESAFE_API_KEY:?Load your TypeSafe API key into this shell}"
  ../orqadence/target/release/orqa
)
```

The Shell shows the open Epics and their Tickets in TICKETS. On the input line:

```
/start-epic <epic> --max 2
```

Tab completes the Epic from its id or a title substring. The status row reads
RUNNING, and RECENT (and `.orqadence/orchestrator.log`) shows
`implement started: claude (pane 2-1)` for A and B. C waits for a slot, and D
waits for A. Leave the Shell open.

### 2. Plan mode, approved by the Judgment (A, B, then C)

Every Implement starts in plan mode. When its plan is up, RECENT shows:

```
plan ready in implement (pane 2-1)
judged: plan covers the Ticket 0.91, stays in scope 0.88, asks nothing 0.95
plan approved
```

The Judgment sent Enter in the pane, and the pane's footer turns from
`⏸ plan mode on` to `⏵⏵ auto mode on`. If instead a score is under the floor
(`covers the Ticket 0.62 < 0.65`), or it reads `asks you a question`, a
Question comes up. Handle it as in step 6: read the plan, then approve or send
feedback.

For A, check the plan hook's evidence:

```bash
cat .orqadence/runs/<A>/settings.json   # one PreToolUse hook on ExitPlanMode running orqa __plan-hook
head .orqadence/runs/<A>/plan.md        # the plan Claude presented
```

Record for each Ticket whether the Judgment approved its plan or asked you.

### 3. Answer prompts and Wakes through their Questions

A Question is the form above the input line. Up/Down or a digit picks an
option, Enter answers, and Esc hides it (`/questions` brings it back).

- **Trust**: `waiting: claude does not trust <dir> yet, open it there once and accept (pane 2-1)`.
  Open that agent in the exact directory, accept, then exit it. Orqadence goes on by
  itself (`claude trusts <dir> now, carrying on`). Do not edit trust files by hand.
- **Other prompts**: `asking you: waiting at a prompt in implement (pane 2-1)`. Pick
  **open the pane**, answer the prompt there, and the Question closes on
  `carrying on`. Choose **I answered it** only once you really have.
- **Wakes**: `stuck in debate 1: went idle without a result (pane 2-2)`. With the key,
  the Wake Judgment answers at 0.7 or above: `judged: …` and then its action
  (`nudged: …`, `retrying …`, `waiting: still working …`). Below the floor you get a
  Question with the actions on offer. Note every Wake and how it was answered.

### 4. A's PR: check D waits, but do not merge yet

Let A finish an uninterrupted Implement → Review → Debate → Fix pipeline until
RECENT shows `PR #N opened after K rounds`. Review A's code, tests, stage
results (`.orqadence/runs/<A>/`) and PR description.

D must not have started. Its row shows `waiting for PR #N to merge`, and there
is no `.orqadence/worktrees/<D>`. An open PR does not unblock it. **Do not merge
A yet**: step 5 merges it while the Shell is closed.

### 5. Stop, resume, exit mid-run, then resume without the key

Do this while B or C is still active.

1. `/stop-work`. RECENT: `stopped, panes left running, /continue resumes`. The
   status row returns to IDLE with the saved run, and the Epic row reads
   RESUMABLE. The agent panes stay open and may keep working: `/stop-work` does
   not freeze them.
2. `/continue`. A checklist lists each saved Ticket. Leave every row on resume
   (Space would toggle a row to reset it to Implement), then press Enter. A Stage
   with an accepted result is skipped, and an unfinished one either picks its live
   session back up or starts a fresh one replacing its pane. No Ticket gets a
   second PR.
3. `/exit` while the run is live. It asks to stop the run and exit; answer
   yes. The Shell closes and the panes stay.
4. Now merge A's PR in GitHub.
5. Relaunch the Shell **without** the key:

   ```bash
   cd ~/Documents/Projects/test-harness-repo
   (
     unset TYPESAFE_API_KEY
     [ -f .orqadence/typesafe-key ] && mv -f .orqadence/typesafe-key .orqadence/typesafe-key.off
     ../orqadence/target/release/orqa
   )
   ```

   Then `/continue` and Enter. Within a 30-second merge poll RECENT shows
   `merged, Ticket closed` for A, A's worktree goes, and D starts from the updated
   `main`.

From here on nothing is judged. Every plan and every Wake is a Question. A
Debate settles its disputed Findings as skip (`flagged: TypeSafe unreachable`),
which is expected in this phase.

### 6. D's plan Question: feedback, then approve

`asking you: plan ready in implement (pane 2-x)`. The Question shows the plan
and no score, because there is no Judgment. Its options are approve, feedback
of your own, park, and open the pane.

1. Scroll the plan with PageDown and PageUp. Long lines wrap, and it scrolls by
   row.
2. Pick **feedback of your own** and type:
   `Revise the plan: list every test case by name.` (PageUp/PageDown still
   scroll the plan while you type.) Press Enter.
3. Watch D's pane. The cursor moves down one line at a time to `Tell Claude what
   to change`, then Enter lands there with the field empty. The dialog closes,
   the pane stays in `⏸ plan mode on`, and your feedback arrives as the next
   prompt. RECENT: `you answered: feedback`, then `plan sent back with your
   feedback`.
4. The revised plan raises the Question again (`plan ready in implement …`).
   Check it lists the test cases, then pick **approve**. RECENT: `plan approved`.
   The pane goes to `⏵⏵ auto mode on`.

If RECENT instead shows `feedback not sent: …` with the plan Question back
(and a "resend your feedback" option), no Enter was sent. Record what the pane
showed, then resend or approve.

### 7. One failed Stage, and its retry

While D's **Review** is working, close only that Review pane in Herdr. RECENT:
`stuck in review 1: session died (pane 2-x)`. D's row turns ◆ BLOCKED, and the
Wake Question offers retry, park and the rest. `/retry <D>` is refused while
that Question waits (`refused: Ticket … has a Question waiting`), which is
expected. Answer the Question with **retry**. A fresh Review session appears
and the pipeline goes on. Other active Tickets keep moving.

If the Stage finished before you could close it, record this as not exercised
rather than disrupting a finished Ticket. If the retry fails again, keep the
evidence and look at the state before issuing more commands.

### 8. Merge the rest

Review B, C and D, check their tests, and merge them yourself. Check D's branch
includes the commit that merged A. Keep the Shell open until RECENT says
`Epic done, every Ticket closed`. All four Tickets should then be closed and
their worktrees removed. Then restore the key file if step 5 moved it:

```bash
[ -f .orqadence/typesafe-key.off ] && mv -f .orqadence/typesafe-key.off .orqadence/typesafe-key
```

## Evidence and acceptance

From another pane in the target repo, while the run is live or after it:

```bash
bd list --parent <epic> --all
gh pr list --state all --json number,headRefName,state,url
git worktree list
```

Keep `.orqadence/orchestrator.log`, `.orqadence/state.json` and
`.orqadence/runs/<epic>.*` (each with `settings.json` and `plan.md` for
Implement). Note the PR URLs, which plans the Judgment approved and which you
did, where stop/resume and the pane loss were exercised, and every Wake with
its answer.

The live run passes when all of the following hold:

- There are never more than two Tickets in the pipeline.
- Each Ticket produces one valid PR.
- D waits for A's merge.
- Every Implement ran in plan mode, and its plan was approved, by the Judgment
  or by you, before any edit.
- D's feedback reached the session with no Enter on the wrong option, and its
  revised plan asked again.
- Stop preserves the panes and a resumable state.
- `/continue` resumes without a second PR.
- `/exit` mid-run leaves the panes.
- Retry replaces only the failed Stage.
- Every merged Ticket closes and cleans up.

Trust dialogs, plan approvals you answer, and PR merges are expected human
actions.

If anything differs, keep the state and log and report the Ticket, the Stage
and what you saw. A clean restart must not erase the evidence being tested.

## After the run: closing the tickets

In the orqadence repo:

1. **Record the evidence** on each ticket, then close them:

   ```bash
   bd comments add harness-kqe.5 "Live run passed on target/release/orqa: <epic>, PRs <urls>; evidence in test-harness-repo/.orqadence"
   bd comments add harness-kqe.10 "Live run passed through the Shell: /start-epic, /stop-work, /continue, /exit mid-run, retry via the Wake Question"
   bd comments add harness-kqe.13 "Real plan-mode Implement: <A> approved by the Judgment <score>; <D> feedback then approve via the Question"
   bd close harness-kqe.5 harness-kqe.10
   ```

2. **Merge the port.** If it is not up yet, push `build/rust-port` and open its
   PR to `main`, with this checklist in the description (`/create-pr`). Merge it
   once the run has passed.
3. **Release v1.0.0.** On the merged `main`, run
   `git tag v1.0.0 && git push origin v1.0.0`. The release workflow builds the
   binaries and attaches them to the release. Reinstall the global `orqa`
   (`cargo install --path .`, or the release binary): from then on, it updates
   itself.
4. **Close the Epic**: `bd close harness-kqe`.
5. **Optional, in the target repo**: close the Go run's PRs #3–#5 once its
   evidence is no longer needed.
