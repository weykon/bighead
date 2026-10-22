"""Join shadow evidence into review rows. Never infer labels from later success."""

import argparse
import json
from pathlib import Path


def review_rows(root):
    rows = []
    for path in sorted(root.glob("shadow-*/events.jsonl")):
        incidents = {}
        ended = False
        drops = None
        incomplete_line = False
        session = None
        for line in path.read_text().splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                incomplete_line = True
                continue
            if event["kind"] == "session_ended":
                ended, drops = True, event.get("dropped_records")
            if event["kind"] == "session_started":
                session = event
            incident_id = event.get("incident_id")
            if incident_id:
                incidents.setdefault(incident_id, []).append(event)
        for incident_id, events in incidents.items():
            opened = next((e for e in events if e["kind"] == "incident_opened"), None)
            decision = next((e for e in events if e["kind"] == "shadow_prediction"), None)
            followup = next((e for e in events if e["kind"] == "next_execution"), None)
            recovered = next((e for e in events if e["kind"] == "execution_recovered"), None)
            rows.append({"incident_id": incident_id, "evidence_file": str(path),
                         "session": session,
                         "request": opened.get("request") if opened else None,
                         "source_evidence": opened.get("evidence") if opened else None,
                         "prediction": decision, "next_execution": followup, "recovery": recovered,
                         "session_ended": ended, "dropped_records": drops,
                         "incomplete_line": incomplete_line,
                         "expected_route": None, "decision_correct": None,
                         "goal_achieved": None, "reviewer": None, "rationale": None,
                         "training_eligible": False})
    return rows


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path("outputs/laya"))
    parser.add_argument("--output", type=Path, default=Path("outputs/laya/review-queue.jsonl"))
    args = parser.parse_args()
    rows = review_rows(args.root)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows))
    print(json.dumps({"incidents": len(rows), "with_predictions": sum(bool(r["prediction"] and r["prediction"].get("prediction")) for r in rows),
                      "with_next_execution": sum(r["next_execution"] is not None for r in rows),
                      "execution_recovered": sum(r["recovery"] is not None for r in rows),
                      "training_eligible": 0, "output": str(args.output)}, indent=2))


if __name__ == "__main__":
    main()
