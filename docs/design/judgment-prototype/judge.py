#!/usr/bin/env python3
"""PROTOTYPE, throwaway: the Wake Judgment as a TypeSafe Choice.

Usage: TYPESAFE_API_KEY=... python3 judge.py [case ...]   (default: every case)
Each case is a directory under cases/: wake.json (ticket, stage, round, reason,
nudged, retried, expect), tail.txt (the pane tail) and optionally result.md.
Answers land in answers/<case>.json.
"""
import json, os, sys, urllib.request
from pathlib import Path

HERE = Path(__file__).parent
FLOOR = 0.7  # confidence floor; below it the Wake becomes a Question

ACTIONS = {
    "nudge_write_result": "The session did the Stage's work and stopped, but never wrote the result file, or wrote it without the STATUS line first. One prompt asking for the file is enough.",
    "nudge_proceed": "The session stopped to ask a question, offer options, or wait for an approval, and nobody is there to answer. One prompt telling it to decide by itself and carry on is enough.",
    "retry": "The session is unusable and a fresh one would likely succeed: it crashed, lost its connection, hit an API or tool error, ran out of context, or repeats the same step without progress.",
    "park": "A person has to look: the session reports the Ticket is wrong, contradictory or impossible, a tool needs login or setup, or the same failure would meet a fresh session too.",
    "wait": "The session is still working: a tool call is running or output is still arriving, and no prompt or question is waiting at the end.",
}
PROMPTS = {
    "nudge_write_result": "The Orchestrator is waiting for your result file {result_file} and cannot read anything else. Write it now, the first line exactly 'STATUS: done' (or 'STATUS: failed' and why), then stop.",
    "nudge_proceed": "Nobody is watching this pane and no one will answer. The Ticket is the spec: decide yourself, note the decision in the result file, carry on to the end, then write {result_file} with 'STATUS: done' as its first line.",
}

def state_for(case):
    wake = json.loads((case / "wake.json").read_text())
    result = case / "result.md"
    return wake, {
        "ticket": wake["ticket"],
        "stage": wake["stage"], "round": wake["round"],
        "why_woken": wake["reason"],
        "already_nudged": wake.get("nudged", False),
        "already_retried": wake.get("retried", False),
        "result_file": {"path": wake["result_file"], "content": result.read_text() if result.exists() else "missing"},
        "pane_tail": (case / "tail.txt").read_text(),
    }

def offered(wake):
    acts = dict(ACTIONS)
    if wake.get("nudged"):
        acts.pop("nudge_write_result"); acts.pop("nudge_proceed")
    if wake.get("retried"):
        acts.pop("retry")
    if wake["reason"].startswith("timed out"):
        acts.pop("wait")
    return acts

def ask(state, acts):
    body = {"model": "jev-latest", "state": state, "questions": {
        "action": {"type": "choice",
                   "instructions": "An agent session running one Stage of a Ticket cannot advance by rule (see `why_woken`). From `pane_tail` and `result_file.content`, which action should the Orchestrator take?",
                   "criteria": acts},
        "finished": {"type": "noul", "instructions": "Did the session complete the Stage's work, whatever it wrote or failed to write in `result_file`?"},
        "asking": {"type": "noul", "instructions": "Is the session stopped at a question or a request for approval that a person would have to answer?"},
    }}
    req = urllib.request.Request("https://api.typesafe.ai/v1/systemone", data=json.dumps(body).encode(),
                                 headers={"Authorization": "Bearer " + os.environ["TYPESAFE_API_KEY"], "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return body, json.load(r)

def main(names):
    cases = [HERE / "cases" / n for n in names] or sorted((HERE / "cases").iterdir())
    for case in cases:
        wake, state = state_for(case)
        acts = offered(wake)
        print(f"== {case.name}: {wake['stage']} {wake['round']}, {wake['reason']}  (expect {wake.get('expect')})")
        if list(acts) == ["park"]:
            print("   parked by rule: nudge and retry already spent, no Judgment asked\n"); continue
        req, resp = ask(state, acts)
        a = resp["answers"]["action"]
        scores = " ".join(f"{k} {v:.2f}" for k, v in sorted(a["probabilities"].items(), key=lambda kv: -kv[1]))
        print(f"   judged: {scores}  confidence {a['confidence']:.2f}  finished {resp['answers']['finished']['noul']:.2f}  asking {resp['answers']['asking']['noul']:.2f}")
        if a["confidence"] >= FLOOR:
            print(f"   -> {a['choice']}" + (": " + PROMPTS[a['choice']].format(result_file=wake['result_file']) if a['choice'] in PROMPTS else ""))
        else:
            print(f"   -> asking you: stuck in {wake['stage']} {wake['round']} (confidence {a['confidence']:.2f} < {FLOOR})")
        (HERE / "answers" / f"{case.name}.json").write_text(json.dumps({"offered": list(acts), "answers": resp["answers"], "usage": resp["usage"]}, indent=1))
        print()

if __name__ == "__main__":
    main(sys.argv[1:])
