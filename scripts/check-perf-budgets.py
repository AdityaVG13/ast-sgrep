#!/usr/bin/env python3
"""Validate published performance baselines against variance-aware budgets."""

import argparse
import json
from pathlib import Path


def validate(path: Path) -> list[str]:
    document = json.loads(path.read_text())
    errors: list[str] = []
    for metric in document.get("metrics", []):
        name = metric["name"]
        baseline = float(metric["measured_p95_ms"])
        variance = float(metric["variance_percent"])
        budget = float(metric["budget_ms"])
        envelope = baseline * (1.0 + variance / 100.0)
        if baseline > budget:
            errors.append(f"{name}: baseline {baseline:g} ms exceeds budget {budget:g} ms")
        elif envelope > budget:
            errors.append(
                f"{name}: variance envelope {envelope:.1f} ms exceeds budget {budget:g} ms"
            )
    return errors

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("path", nargs="?", default="benchmarks/perf-budgets.json")
    args = parser.parse_args()
    errors = validate(Path(args.path))
    if errors:
        for error in errors:
            print(error)
        return 1
    print(f"performance budgets valid: {args.path}")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
