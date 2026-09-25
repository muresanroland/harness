# PROTOTYPE: the plan Judgment

Throwaway. Answers harness-cq7: in the harness-7nq run the plan Judgment scored 21 plans
and approved none, and each became a Question the user approved unchanged. `judge_plan.py`
sends the plan Judgment's request, the state `judgment.rs` builds, twice per case: once
with today's one Noul and once with one Noul per criterion. `cases/` holds the 21 real plans
and 12 negatives built from them. `answers/` holds what Jev answered in the recorded run;
`first-run.txt` holds the settled Nouls' scores from the run before it.

```bash
python3 judge_plan.py            # every case; TYPESAFE_API_KEY or .harness/typesafe-key
python3 judge_plan.py 7nq.14     # one
python3 judge_plan.py --report   # the tables from answers/, no calls
```

## Cases

**Positives:** the 21 judged plans of the harness-7nq run (`cases/7nq.<n>`): plan.md from
the run's `.harness/runs`, the Ticket from `bd show <id> --json`. The user approved each
one unchanged. harness-7nq.3 is left out: its plan was never judged.

**Negatives:** each has a `case.json` that says how it was built.

| kind | cases |
|---|---|
| a plan paired with another Ticket | `neg-other-far-1` (7nq.5's plan, 7nq.14's Ticket), `neg-other-far-2` (17 on 2), `neg-other-near` (14 on 15, its neighbour: Limited on Implement vs Limited on the Review) |
| one acceptance criterion's work cut out | `neg-cut-1` (7nq.1 without the colours), `neg-cut-2` (7nq.21 without the TypeSafe page), `neg-cut-3` (7nq.6 without the close-Epic confirmation) |
| an unrelated change added, with its test | `neg-added-1` (7nq.4 plus an hourly update check), `neg-added-2` (7nq.13 plus a rename of `Held`), `neg-added-3` (7nq.19 plus a `harness doctor` command) |
| a plan ending in an open question | `neg-question-1..3` (7nq.12, 18 and 10; the third is one line with no heading) |

## What was settled

**One Noul per criterion, in one request.** Approved when `covers` and `in_scope` are at or
above the plan floor and `asks` is below 0.5.

- `covers`: "Does the plan meet every acceptance criterion of the Ticket, with the change and the test each one needs?"
- `in_scope`: "Does the plan stay within the Ticket? Work toward something the Ticket never asks for, such as another feature, command or rename, is beyond it; the tests, docs, refactors and plumbing the Ticket's own change needs are in scope."
- `asks`: "Does the plan put a question of its own to the person approving it, one they must answer before the work can start? Questions the planned code will put to its users are not that, and neither are decisions the plan makes itself with the answer it took."

**The floor: 0.65.** Two runs of the settled wording (`first-run.txt`, then `answers/`) agree: at 0.65, 20 of 21 positives
are approved and every negative is stopped, with a margin of at least 0.10 on `in_scope`
(positives 0.73 and up, added work 0.55 at most). At 0.6 all 21 are approved, but
`neg-added-3` sits 0.05 under the floor and 7nq.14 sits on it (`covers` 0.60 and 0.62), so
0.6 would flip on Jev's run-to-run noise: between the two runs a score moved by 0.01 on
average and at most 0.05 on `covers` and `in_scope` (0.09 on `asks`). The positive that misses is
7nq.14, whose Ticket packs about ten behaviours into one acceptance line. Its Question
says `covers the Ticket 0.62 < 0.65`, so the user sees why.

**Why today's question says no.** It asks three things at once. Every "decisions the
Ticket left open" heading reads to it as open decisions: the highest of the 21 is 0.73, so
none clears 0.75. It stops the negatives too, so it cannot tell a good plan from a bad
one. The Stage skill now heads that section "Decisions I made" and keeps a real open
question in a section of its own.

**Wordings tried.**

1. First wording. `covers` asked for "a change that meets it and a test that checks it".
   `in_scope` named the plumbing that is in scope. `asks` asked for "a question to the
   user". At 0.6: 19 of 21 approved, 12 of 12 stopped. The two misses were 7nq.18 and
   7nq.20, whose `asks` read 0.82 and 0.67. Both are Tickets about questions (init's
   prompts), and Jev took the questions the planned code puts to its users as the plan
   asking. The added-work negatives reached 0.59 on `in_scope`, too close to any floor.
2. `asks` named the planned code's questions as not counting, and asked whether the plan
   "stops to wait for the answer". That fixed 7nq.18 and 7nq.20 but raised 7nq.12 (a plan
   about a session that stops and waits for approval) to 0.71. `in_scope` listed examples
   of work beyond the Ticket. That pushed the added work down to 0.15–0.29, but positives
   fell to 0.53.
3. Settled (above): `asks` without the waiting, `in_scope` with fewer examples and
   refactors named as in scope.

The subject leaks into the Nouls. A plan about questions or waiting scores higher on
`asks`, and naming that in the instructions is what holds it down. The Stage skill's
section names help as well.

## Results (answers/, the settled wording)

| case | expect | today: follows | covers | in_scope | asks | today at 0.75 | Nouls at 0.65 |
|---|---|---|---|---|---|---|---|
| 7nq.1 | approve | 0.73 | 0.90 | 0.94 | 0.07 | ask | approve |
| 7nq.2 | approve | 0.42 | 0.74 | 0.85 | 0.09 | ask | approve |
| 7nq.4 | approve | 0.37 | 0.71 | 0.82 | 0.11 | ask | approve |
| 7nq.5 | approve | 0.56 | 0.78 | 0.82 | 0.10 | ask | approve |
| 7nq.6 | approve | 0.18 | 0.80 | 0.86 | 0.17 | ask | approve |
| 7nq.7 | approve | 0.38 | 0.81 | 0.85 | 0.07 | ask | approve |
| 7nq.8 | approve | 0.38 | 0.79 | 0.84 | 0.15 | ask | approve |
| 7nq.9 | approve | 0.24 | 0.76 | 0.87 | 0.08 | ask | approve |
| 7nq.10 | approve | 0.50 | 0.68 | 0.84 | 0.07 | ask | approve |
| 7nq.11 | approve | 0.14 | 0.83 | 0.88 | 0.13 | ask | approve |
| 7nq.12 | approve | 0.13 | 0.78 | 0.87 | 0.28 | ask | approve |
| 7nq.13 | approve | 0.28 | 0.85 | 0.88 | 0.12 | ask | approve |
| 7nq.14 | approve | 0.10 | 0.62 | 0.82 | 0.17 | ask | **ask** |
| 7nq.15 | approve | 0.17 | 0.76 | 0.86 | 0.14 | ask | approve |
| 7nq.16 | approve | 0.57 | 0.80 | 0.88 | 0.13 | ask | approve |
| 7nq.17 | approve | 0.35 | 0.84 | 0.84 | 0.07 | ask | approve |
| 7nq.18 | approve | 0.17 | 0.83 | 0.85 | 0.25 | ask | approve |
| 7nq.19 | approve | 0.22 | 0.70 | 0.81 | 0.11 | ask | approve |
| 7nq.20 | approve | 0.62 | 0.78 | 0.87 | 0.08 | ask | approve |
| 7nq.21 | approve | 0.58 | 0.83 | 0.87 | 0.08 | ask | approve |
| 7nq.22 | approve | 0.15 | 0.68 | 0.73 | 0.11 | ask | approve |
| neg-added-1 | reject | 0.31 | 0.70 | 0.49 | 0.11 | ask | ask |
| neg-added-2 | reject | 0.25 | 0.85 | 0.33 | 0.11 | ask | ask |
| neg-added-3 | reject | 0.20 | 0.69 | 0.55 | 0.12 | ask | ask |
| neg-cut-1 | reject | 0.11 | 0.12 | 0.61 | 0.07 | ask | ask |
| neg-cut-2 | reject | 0.14 | 0.12 | 0.41 | 0.08 | ask | ask |
| neg-cut-3 | reject | 0.13 | 0.25 | 0.59 | 0.20 | ask | ask |
| neg-other-far-1 | reject | 0.15 | 0.13 | 0.56 | 0.09 | ask | ask |
| neg-other-far-2 | reject | 0.03 | 0.02 | 0.20 | 0.07 | ask | ask |
| neg-other-near | reject | 0.05 | 0.04 | 0.30 | 0.24 | ask | ask |
| neg-question-1 | reject | 0.07 | 0.72 | 0.84 | 0.86 | ask | ask |
| neg-question-2 | reject | 0.09 | 0.69 | 0.84 | 0.85 | ask | ask |
| neg-question-3 | reject | 0.27 | 0.61 | 0.83 | 0.92 | ask | ask |

| | positives approved | negatives stopped |
|---|---|---|
| today's question at 0.75 | 0 / 21 | 12 / 12 |
| the Nouls at 0.6 | 21 / 21 | 12 / 12 |
| **the Nouls at 0.65** | **20 / 21** | **12 / 12** |
| the Nouls at 0.7 | 18 / 21 | 12 / 12 |
| the Nouls at 0.75 | 15 / 21 | 12 / 12 |

Each request cost about 3.3K input tokens (1.6K to 4.5K), about as much as today's one Noul.
