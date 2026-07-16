#!/usr/bin/env python3
"""Gate hyperfine samples against a hard latency error budget."""

import argparse
import json
import math
from pathlib import Path


def percentile(values, quantile):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(quantile * len(ordered)) - 1)]


def evaluate(times_seconds, threshold_ms, slo, baseline_p95_ms=None):
    if not times_seconds:
        raise ValueError("hyperfine result has no times")
    if not 0 < slo < 1:
        raise ValueError("SLO must be between zero and one")
    times_ms = [value * 1000.0 for value in times_seconds]
    exceedances = sum(value > threshold_ms for value in times_ms)
    error_rate = exceedances / len(times_ms)
    burn_rate = error_rate / (1.0 - slo)
    p95_ms = percentile(times_ms, 0.95)
    baseline_within_threshold = baseline_p95_ms is None or baseline_p95_ms <= threshold_ms
    return {
        "sample_count": len(times_ms),
        "threshold_ms": threshold_ms,
        "slo": slo,
        "p95_ms": p95_ms,
        "exceedance_count": exceedances,
        "error_rate": error_rate,
        "burn_rate": burn_rate,
        "baseline_p95_ms": baseline_p95_ms,
        "gates": {
            "p95_within_threshold": p95_ms <= threshold_ms,
            "burn_rate_within_budget": burn_rate <= 1.0,
            "baseline_within_threshold": baseline_within_threshold,
        },
        "claim_within_slo": p95_ms <= threshold_ms and burn_rate <= 1.0 and baseline_within_threshold,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("--threshold-ms", type=float, required=True)
    parser.add_argument("--slo", type=float, default=0.95)
    parser.add_argument("--baseline-p95-ms", type=float)
    parser.add_argument("--result-index", type=int, default=0)
    parser.add_argument("--label", default="latency")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    payload = json.loads(args.input.read_text())
    result = evaluate(
        payload["results"][args.result_index]["times"],
        args.threshold_ms,
        args.slo,
        args.baseline_p95_ms,
    )
    result["label"] = args.label
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(encoded)
    else:
        print(encoded, end="")
    return 0 if result["claim_within_slo"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
