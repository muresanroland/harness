#!/usr/bin/env python3
"""PROTOTYPE, throwaway: the plan Judgment, today's one Noul against one Noul per criterion.

Usage: python3 judge_plan.py [case ...]     ask TypeSafe (default: every case)
       python3 judge_plan.py --report       the tables from answers/, no calls
The key is TYPESAFE_API_KEY, else .orqadence/typesafe-key. Each case is a directory
under cases/: plan.md, ticket.json (bd show <id> --json: id, title, description,
acceptance_criteria) and, for a built negative, case.json (expect, built). Answers
land in answers/<case>.json.
"""
import json, os, sys, urllib.request
from pathlib import Path

HERE = Path(__file__).parent
REPO = HERE.parents[2]
TODAY_FLOOR = 0.75  # PLAN_FLOOR before this prototype
FLOOR = 0.65        # covers and in_scope at or above it, asks below 0.5

TODAY = {
    "follows": "Does this plan implement the Ticket, all of it and nothing more, without leaving decisions open?",
}
NOULS = {
    "covers": "Does the plan meet every acceptance criterion of the Ticket, with the change and the test each one needs?",
    "in_scope": "Does the plan stay within the Ticket? Work toward something the Ticket never asks for, such as another feature, command or rename, is beyond it; the tests, docs, refactors and plumbing the Ticket's own change needs are in scope.",
    "asks": "Does the plan put a question of its own to the person approving it, one they must answer before the work can start? Questions the planned code will put to its users are not that, and neither are decisions the plan makes itself with the answer it took.",
}


def key():
    return os.environ.get("TYPESAFE_API_KEY") or (REPO / ".orqadence" / "typesafe-key").read_text().strip()


def state_for(case):
    """The state judgment.rs sends: the plan, the Ticket with its criteria under its description."""
    t = json.loads((case / "ticket.json").read_text())
    description = t["description"]
    if t.get("acceptance_criteria"):
        description += "\n\nAcceptance criteria:\n" + t["acceptance_criteria"]
    return {
        "plan": (case / "plan.md").read_text(),
        "ticket": {"id": t["id"], "title": t["title"], "description": description},
        "prior_feedback": None,
    }


def ask(state, nouls):
    body = {"model": "jev-latest", "state": state,
            "questions": {k: {"type": "noul", "instructions": v} for k, v in nouls.items()}}
    req = urllib.request.Request("https://api.typesafe.ai/v1/systemone", data=json.dumps(body).encode(),
                                 headers={"Authorization": "Bearer " + key(), "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        resp = json.load(r)
    return {k: resp["answers"][k]["noul"] for k in nouls}, resp["usage"]


def expect(case):
    meta = case / "case.json"
    return json.loads(meta.read_text())["expect"] if meta.exists() else "approve"


def approved_today(a, floor=TODAY_FLOOR):
    return a["follows"] >= floor


def approved(a, floor=FLOOR):
    return a["covers"] >= floor and a["in_scope"] >= floor and a["asks"] < 0.5


def run(names):
    cases = [HERE / "cases" / n for n in names] or sorted((HERE / "cases").iterdir())
    for case in cases:
        state = state_for(case)
        today, u1 = ask(state, TODAY)
        nouls, u2 = ask(state, NOULS)
        answer = {"expect": expect(case), "today": today, "nouls": nouls,
                  "usage": {"today": u1, "nouls": u2}}
        (HERE / "answers" / f"{case.name}.json").write_text(json.dumps(answer, indent=1) + "\n")
        print(f"{case.name:18} {answer['expect']:8} follows {today['follows']:.2f}  "
              + "  ".join(f"{k} {v:.2f}" for k, v in nouls.items()))


def report():
    answers = {p.stem: json.loads(p.read_text()) for p in sorted((HERE / "answers").glob("*.json"))}
    print(f"{'case':18} {'expect':8} {'follows':>7} {'covers':>7} {'in_scope':>8} {'asks':>5}  today  nouls")
    for name, a in answers.items():
        t, n = a["today"], a["nouls"]
        mark = lambda ok: "approve" if ok else "ask"
        print(f"{name:18} {a['expect']:8} {t['follows']:7.2f} {n['covers']:7.2f} {n['in_scope']:8.2f} {n['asks']:5.2f}"
              f"  {mark(approved_today(t)):6} {mark(approved(n))}")
    pos = [a for a in answers.values() if a["expect"] == "approve"]
    neg = [a for a in answers.values() if a["expect"] == "reject"]

    def score(ok):
        return f"{sum(ok(a) for a in pos)}/{len(pos)} positives approved, " \
               f"{sum(not ok(a) for a in neg)}/{len(neg)} negatives stopped"
    print(f"\ntoday's question at {TODAY_FLOOR}: " + score(lambda a: approved_today(a["today"])))
    for floor in [0.5, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9]:
        print(f"the Nouls at {floor}: " + score(lambda a: approved(a["nouls"], floor)))


if __name__ == "__main__":
    report() if sys.argv[1:] == ["--report"] else run(sys.argv[1:])
