#!/usr/bin/env python3
"""Fail a release benchmark when identity or keep-gate thresholds regress.

`--max-average-ms` / `--smoke-max-average-ms` is a host-labeled smoke ceiling,
not the keep oracle. Keep compares against committed `.bench-history/*.latest.json`
using `.bench-history/thresholds.json` (−3% primary / −5% geomean / cv>5 quarantine).
Competitor latency is never keep or correctness.
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any


def _finite(value: Any, label: str) -> float:
    if not isinstance(value, int | float) or not math.isfinite(value):
        raise ValueError(f"{label} must be finite")
    return float(value)


def load_thresholds(history_dir: Path) -> dict[str, float]:
    path = history_dir / "thresholds.json"
    raw = json.loads(path.read_text(encoding="utf-8"))
    return {
        "primary_regression_pct": float(raw["primary_regression_pct"]),
        "geomean_regression_pct": float(raw["geomean_regression_pct"]),
        "cv_ineligible_pct": float(raw["cv_ineligible_pct"]),
    }


def sanitize_label(label: str) -> str:
    return "".join(ch if ch.isalnum() or ch == "-" else "-" for ch in label)


def evaluate_keep(
    avg_ms: float,
    cv_pct: float,
    geomean_ms: float | None,
    prior: dict[str, Any],
    thresholds: dict[str, float],
) -> str:
    if cv_pct > thresholds["cv_ineligible_pct"]:
        return "quarantine_cv"
    placeholder = bool(prior.get("placeholder")) or prior.get("keep_eligible") is False
    prior_avg = prior.get("avg_search_ms")
    if placeholder or not isinstance(prior_avg, int | float) or not math.isfinite(prior_avg) or prior_avg <= 0:
        return "establish_baseline"
    regression_pct = ((avg_ms - prior_avg) / prior_avg) * 100.0
    if regression_pct > thresholds["primary_regression_pct"]:
        return "reject_regression"
    prior_geo = prior.get("geomean_search_ms")
    if (
        geomean_ms is not None
        and isinstance(prior_geo, int | float)
        and math.isfinite(prior_geo)
        and prior_geo > 0
        and ((geomean_ms - prior_geo) / prior_geo) * 100.0 > thresholds["geomean_regression_pct"]
    ):
        return "reject_regression"
    return "keep"


def validate(
    payload: dict[str, Any],
    smoke_max_average_ms: float | None,
    history_dir: Path | None,
    label: str | None,
) -> list[tuple[str, float]]:
    cases = payload.get("cases")
    if not isinstance(cases, list) or not cases:
        raise ValueError("benchmark payload must contain non-empty cases")
    measured: list[tuple[str, float]] = []
    cvs: list[float] = []
    for case in cases:
        if not isinstance(case, dict):
            raise ValueError("every benchmark case must be an object")
        name = case.get("name")
        average = case.get("avg_search_ms")
        if not isinstance(name, str) or not name:
            raise ValueError("every benchmark case needs a name")
        avg = _finite(average, f"{name}: avg_search_ms")
        if case.get("ok") is not True or case.get("identity_ok") is not True:
            raise ValueError(f"{name}: correctness or result identity failed")
        if smoke_max_average_ms is not None and avg > smoke_max_average_ms:
            raise ValueError(
                f"{name}: smoke ceiling {avg:.3f} ms exceeds {smoke_max_average_ms:.3f} ms "
                "(host-labeled secondary; not the keep oracle)"
            )
        cv = case.get("cv_pct")
        if isinstance(cv, int | float) and math.isfinite(cv):
            cvs.append(float(cv))
        measured.append((name, avg))

    if history_dir is not None:
        thresholds = load_thresholds(history_dir)
        suite_label = label or str(payload.get("bench_history", {}).get("label") or "")
        if not suite_label:
            fixture = payload.get("fixture") or "sample"
            suite = payload.get("suite") or "default"
            suite_label = f"suite:{fixture}:{suite}"
        prior_path = history_dir / f"{sanitize_label(suite_label)}.latest.json"
        prior = json.loads(prior_path.read_text(encoding="utf-8")) if prior_path.exists() else {
            "placeholder": True,
            "keep_eligible": False,
        }
        avgs = [avg for _, avg in measured]
        suite_avg = sum(avgs) / len(avgs)
        suite_cv = (sum(cvs) / len(cvs)) if cvs else 0.0
        pos = [a for a in avgs if a > 0]
        geomean = math.exp(sum(math.log(a) for a in pos) / len(pos)) if pos else None
        verdict = evaluate_keep(suite_avg, suite_cv, geomean, prior, thresholds)
        if verdict in {"quarantine_cv", "reject_regression"}:
            raise ValueError(
                f"keep-gate {verdict} for {suite_label} "
                f"(avg={suite_avg:.3f}ms cv={suite_cv:.2f}% prior={prior_path})"
            )
    return measured


def _self_test() -> None:
    th = {
        "primary_regression_pct": 3.0,
        "geomean_regression_pct": 5.0,
        "cv_ineligible_pct": 5.0,
    }
    prior = {"placeholder": False, "keep_eligible": True, "avg_search_ms": 100.0}
    assert evaluate_keep(103.0, 1.0, None, prior, th) == "keep"
    assert evaluate_keep(103.1, 1.0, None, prior, th) == "reject_regression"
    assert evaluate_keep(90.0, 5.01, None, prior, th) == "quarantine_cv"
    assert evaluate_keep(12.0, 1.0, None, {"placeholder": True, "keep_eligible": False}, th) == "establish_baseline"
    print("keep-gate self-test passed")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("payload", type=Path, nargs="?")
    parser.add_argument("--max-average-ms", type=float, dest="smoke_max_average_ms")
    parser.add_argument("--smoke-max-average-ms", type=float, dest="smoke_max_average_ms")
    parser.add_argument("--history-dir", type=Path, default=Path(".bench-history"))
    parser.add_argument("--label", default=None)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        _self_test()
        return 0
    if args.payload is None:
        parser.error("payload is required unless --self-test")
    if args.smoke_max_average_ms is not None and (
        not math.isfinite(args.smoke_max_average_ms) or args.smoke_max_average_ms <= 0
    ):
        parser.error("--max-average-ms / --smoke-max-average-ms must be positive and finite")
    payload = json.loads(args.payload.read_text(encoding="utf-8"))
    measured = validate(payload, args.smoke_max_average_ms, args.history_dir, args.label)
    summary = ", ".join(f"{name}={average:.3f}ms" for name, average in measured)
    print(f"benchmark gate passed: {summary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
