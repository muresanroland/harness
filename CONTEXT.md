# Harness

Drives a beads epic through a fixed multi-agent pipeline, from tickets to open pull requests, with every agent visible in herdr.

## Language

**Harness**:
The globally installed tool as a whole: the Orchestrator and the Shell.

**Target repo**:
The repository whose epic is being worked on. The Harness is run from inside it; it scaffolds the repo's agent setup once, and the repo owns its conventions from then on.
_Avoid_: Project, host repo

**Stage skill**:
A skill owned and shipped by the Harness that holds the instructions for one Stage. Once installed in a Target repo, the repo's copy is the one that runs and may be edited there.
_Avoid_: Prompt, template

**Delegate skill**:
A third-party skill a Stage skill runs for one job of its Stage (test-first implementing, self review, the over-engineering audit, merge conflicts...), chosen per job by the user. A Stage can have several. The Stage skill still owns the Stage result; with no Delegate skill for a job it follows its own instructions.
_Avoid_: Override, replacement, work skill

**Shipped skill**:
Any skill the Harness installs into a Target repo: the Stage skills, plus create-pr, which the Fix Stage runs. A repo that already has a create-pr of its own is asked whether to keep it, replace it, or take the shipped one beside it as harness-create-pr.

**Skill manifest**:
One checkout's record of the skills the Harness installed for it: the Shipped skills, plus third-party skills named by their source, where they were put, and which of them is each Stage's Delegate skill. It belongs to the checkout, not the repo, even when the skill files themselves are committed.
_Avoid_: Config, lockfile

**Epic**:
The beads epic handed to the Harness. Its child Tickets are the whole scope of one run.

**Ticket**:
A beads issue that is a child of the Epic, and the unit that moves through the Pipeline. It ends as one pull request and closes only when that pull request is merged.
_Avoid_: Task, issue, story

**Pipeline**:
The fixed sequence of Stages every Ticket passes through: Implement, Review, Debate, Fix, then a pull request.
_Avoid_: Workflow, flow

**Round**:
One pass of Review, Debate and Fix over a Ticket. Rounds repeat until a Verdict has no fix items or the cap is reached, after which the pull request opens with any leftover Findings listed.
_Avoid_: Iteration, loop, cycle

**Stage**:
One step of the Pipeline, carried out by a fresh agent session in its own pane.
_Avoid_: Step, phase

**Stage result**:
The recorded outcome of a Stage, carrying its completion status and, as appropriate, Findings, a Verdict, or an opened pull request. The Orchestrator uses it together with the session's state to decide whether the Stage can advance.

**Run directory**:
The Ticket's directory under `.harness/runs/`, holding its Stages' evidence: the result files, diffs and Debate transcripts, all flat text. It doubles as the Review's sandbox, so build scratch lands there too and is pruned when the pull request opens.
_Avoid_: Logs, workdir, artifacts

**Finding**:
One claimed problem with a Ticket's changes, raised by the Review or by the over-engineering audit, and the unit the Debate argues over.
_Avoid_: Comment, issue, point

**Moderator**:
The neutral session that runs the Debate between a Claude side and a GPT side. It never argues a position of its own, and settles Findings the sides still dispute by an outside score.
_Avoid_: Judge, Debby

**Verdict**:
The Debate's result: every Finding marked fix or skip, with a severity and the reason. Only fix items reach the Fix Stage.
_Avoid_: Synthesis, summary, report

**Wake**:
The Orchestrator's request for judgment about a Stage that cannot advance by rule, answered by a Judgment or, failing that, by the user through a Question.
_Avoid_: Alert, escalation

**Judgment**:
The Orchestrator's answer to a Wake or a Plan, taken from a typed model over the evidence: for a Wake one of a fixed set of actions with a score each, for a Plan a yes or no with a score, acted on above a confidence floor and shown on the Shell.
_Avoid_: LLM call, Main session

**Plan**:
What an Implement session writes before it may edit: the changes, tests and decisions it intends for its Ticket. A Judgment approves it when it follows the Ticket; otherwise the user reads it and answers, and the session revises it.

**Question**:
What the Shell puts to the user when the Orchestrator cannot act alone: a Wake the Judgment was unsure about, a blocked session, a plan to approve, or a confirmation. It holds only its Ticket, is answered from a fixed set of options or a line of the user's own text, and is never saved: on resume it is derived again from the live session.
_Avoid_: Prompt, dialog, alert, form, popup

**Parked**:
A Ticket taken out of the Pipeline to wait for the user, after a Wake that a Judgment or the user settled as park. Other Tickets keep running.
_Avoid_: Stuck, paused, failed

**Limited**:
A Ticket held because the agent a Stage runs on hit its provider's usage limit. Limits belong to the account, so a claude limit holds every Ticket of the run: a short one resumes by itself at the reset; a long one (a reset more than a day away) ends the run with every session saved, and /continue resumes each where it stopped. A codex limit holds only the Review, through one Question whose answer stands for every Ticket until the reset. Unlike Parked, nothing in the Ticket's own work went wrong.
_Avoid_: Rate-limited, cooling down, throttled

**Ticket tab**:
The herdr tab belonging to one running Ticket, holding one pane per Stage.

**Tools**:
The one seam every external command (herdr, bd, gh, git) goes through; a test double stands behind it so tests never start a process.
_Avoid_: Runner, exec, shell

**Orchestrator**:
The deterministic process that owns ticket state, pane placement, and stage transitions. Its only judgment calls are bounded, logged Judgments through a typed model, and it never composes text.
_Avoid_: Script, runner, daemon

**Shell**:
The full-terminal screen that `harness` alone opens: it lists the Epics, takes slash commands, runs the Orchestrator inside its own process, and is where every event and Question appears.
_Avoid_: TUI, dashboard, Main session, attach
