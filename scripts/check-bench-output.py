#!/usr/bin/env python3
"""Fail a release benchmark when identity or latency thresholds regress."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any


def validate(payload: dict[str, Any], max_average_ms: float) -> list[tuple[str, float]]:
    cases = payload.get("cases")
    if not isinstance(cases, list) or not cases:
        raise ValueError("benchmark payload must contain non-empty cases")
    measured: list[tuple[str, float]] = []
    for case in cases:
        if not isinstance(case, dict):
            raise ValueError("every benchmark case must be an object")
        name = case.get("name")
        average = case.get("avg_search_ms")
        if not isinstance(name, str) or not name:
            raise ValueError("every benchmark case needs a name")
        if not isinstance(average, int | float) or not math.isfinite(average):
            raise ValueError(f"{name}: avg_search_ms must be finite")
        if case.get("ok") is not True or case.get("identity_ok") is not True:
            raise ValueError(f"{name}: correctness or result identity failed")
        if average > max_average_ms:
            raise ValueError(
                f"{name}: average {average:.3f} ms exceeds {max_average_ms:.3f} ms"
            )
        measured.append((name, float(average)))
    return measured


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("payload", type=Path)
    parser.add_argument("--max-average-ms", type=float, required=True)
    args = parser.parse_args()
    if not math.isfinite(args.max_average_ms) or args.max_average_ms <= 0:
        parser.error("--max-average-ms must be positive and finite")
    payload = json.loads(args.payload.read_text(encoding="utf-8"))
    measured = validate(payload, args.max_average_ms)
    summary = ", ".join(f"{name}={average:.3f}ms" for name, average in measured)
    print(f"benchmark gate passed: {summary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
