#!/usr/bin/env python3
"""Emit greenfield conformal parity_score.json (1vhy.6).

Optimistic present-ratio is not certified. Lower bound is 0 until an evidence
window maps executed correctness Passes onto features. Never writes
release_certificate.json.
"""

from __future__ import annotations

import argparse
import json
import tomllib
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_WEIGHTS = ROOT / "docs/contracts/parity_score_contract.toml"
DEFAULT_MATRIX = ROOT / "docs/contracts/supported_surface_matrix.toml"
DEFAULT_OUT = ROOT / "tests/conformance/parity_score.json"

SKIP_STATUS = {"n/a", "excluded"}
TRUNCATE_ZERO = {"partial", "missing"}


def load_toml(path: Path) -> dict[str, Any]:
    return tomllib.loads(path.read_text())


def optimistic_present_ratio(matrix: dict[str, Any], weights: dict[str, float]) -> float:
    by_cat: dict[str, list[float]] = defaultdict(list)
    for feature in matrix.get("feature") or []:
        category = str(feature.get("category") or "search")
        hosts = feature.get("hosts") or {}
        countable = [str(status) for status in hosts.values() if str(status) not in SKIP_STATUS]
        if not countable:
            continue
        present = sum(1 for status in countable if status == "present")
        by_cat[category].append(present / len(countable))
    scored = 0.0
    weight_sum = 0.0
    for category, weight in weights.items():
        samples = by_cat.get(category)
        if not samples:
            continue
        scored += weight * (sum(samples) / len(samples))
        weight_sum += weight
    if weight_sum <= 0:
        return 0.0
    return scored / weight_sum


def render(
    *,
    weights_path: Path,
    matrix_path: Path,
    lower_bound: float,
) -> dict[str, Any]:
    weights_doc = load_toml(weights_path)
    matrix = load_toml(matrix_path)
    category_weight = {str(k): float(v) for k, v in (weights_doc.get("category_weight") or {}).items()}
    optimistic = round(optimistic_present_ratio(matrix, category_weight), 4)
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return {
        "schema_version": "1",
        "subject_class": weights_doc.get("subject_class", "greenfield-hybrid-search"),
        "generated": now,
        "certified": False,
        "band": "red",
        "release_certificate": "refused",
        "lower_bound": lower_bound,
        "optimistic_present_ratio": optimistic,
        "interval": [lower_bound, optimistic],
        "point_estimate_is_certified": False,
        "truncate_policy": {
            "partial_is_not_present": True,
            "excluded_is_not_missing": True,
            "not_run_is_not_pass": True,
            "unreproducible_mrr_is_not_cert": True,
            "latency_only_never_correctness": True,
            "present_count_is_not_green": True,
        },
        "forbidden_victory": True,
        "inputs": {
            "wp1": "keep-gate / .bench-history",
            "wp2": "benchmarks/results/baselines.md",
            "wp4": "docs/validation/oracle-dispatch.md",
            "wp5": str(matrix_path.relative_to(ROOT)),
            "ghiw.5": "scripts/generate-compliance-report.py",
            "nz7i": "docs/validation/golden-files.md",
            "b8q3": "bounded-fuzz workflow_dispatch",
            "weights": str(weights_path.relative_to(ROOT)),
        },
        "deviations": [
            "H8 lower_bound is 0: no evidence window mapped executed correctness Pass onto features.",
            "H9 multi-ref bundle 0/8 green.",
            "H12 live-embed P1s not run.",
            "H14 release_certificate.json not emitted.",
            "Canonical MRR rows remain UNREPRODUCIBLE.",
        ],
        "checklist": "docs/validation/multi-ref-checklist.md",
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--weights", type=Path, default=DEFAULT_WEIGHTS)
    parser.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args(argv)
    payload = render(weights_path=args.weights, matrix_path=args.matrix, lower_bound=0.0)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
