"""Evaluate fixed, synthetic ECS choices without executing any ECS operations."""

import argparse
import json
import statistics
import time
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default="http://127.0.0.1:3031")
    parser.add_argument("--cases", type=Path, default=Path(__file__).with_name("cases.jsonl"))
    parser.add_argument("--output", type=Path, default=Path("outputs/laya/baseline.json"))
    args = parser.parse_args()
    with urlopen(args.url + "/health", timeout=5) as stream:
        health = json.load(stream)
    results = []
    for line in args.cases.read_text().splitlines():
        case = json.loads(line)
        data = {key: case[key] for key in ("goal", "state", "candidates")}
        req = Request(args.url + "/decide", json.dumps(data).encode(), {"Content-Type": "application/json"})
        started = time.perf_counter()
        try:
            with urlopen(req, timeout=60) as stream:
                prediction = json.load(stream)
            results.append({"case_id": case["id"], "expected": case["expected"],
                            "correct": prediction["operation"] == case["expected"],
                            "elapsed_ms": round((time.perf_counter() - started) * 1000, 2),
                            "prediction": prediction})
        except (HTTPError, URLError) as error:
            detail = error.read().decode() if isinstance(error, HTTPError) else str(error)
            results.append({"case_id": case["id"], "correct": False, "error": detail})
    completed = [r for r in results if "prediction" in r]
    correct = sum(r["correct"] for r in results)
    report = {"dataset": "ecs-operation-smoke-v1", "synthetic": True,
              "use_for": "baseline evaluation, not training or production acceptance",
              "health": health, "total": len(results), "completed": len(completed),
              "correct": correct, "accuracy": correct / len(results),
              "warm_median_ms": statistics.median([r["elapsed_ms"] for r in completed[1:]]) if len(completed) > 1 else None,
              "results": results}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "results"}, ensure_ascii=False, indent=2))
    if len(completed) != len(results):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
