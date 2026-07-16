#!/usr/bin/env python3
"""Gate hyperfine samples against a hard latency error budget."""

import argparse
import json
import math
from pathlib import Path


def percentile(values, quantile):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(quantile * len(ordered)) - 1)]


def evaluate_variance(current_p95_ms, prior_p95_ms, max_drift_fraction, fingerprint, prior_fingerprint):
    same_host = bool(fingerprint and prior_fingerprint and fingerprint == prior_fingerprint)
    evaluated = prior_p95_ms is not None and prior_p95_ms > 0 and same_host
    drift_fraction = (current_p95_ms - prior_p95_ms) / prior_p95_ms if evaluated else None
    return {
        "evaluated": evaluated,
        "same_host": same_host,
        "fingerprint": fingerprint,
        "prior_fingerprint": prior_fingerprint,
        "prior_p95_ms": prior_p95_ms,
        "drift_fraction": drift_fraction,
        "max_drift_fraction": max_drift_fraction,
        "within_envelope": None if not evaluated else drift_fraction <= max_drift_fraction,
    }

def evaluate(times_seconds, threshold_ms, slo, baseline_p95_ms=None, *, prior_p95_ms=None, max_drift_fraction=0.10, fingerprint=None, prior_fingerprint=None):
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
    hard_gate_passes = p95_ms <= threshold_ms and burn_rate <= 1.0 and baseline_within_threshold
    variance = evaluate_variance(p95_ms, prior_p95_ms, max_drift_fraction, fingerprint, prior_fingerprint)
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
        "variance_gate": variance,
        "claim_within_slo": hard_gate_passes,
        "claim_within_all_gates": hard_gate_passes and (not variance["evaluated"] or variance["within_envelope"]),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("--threshold-ms", type=float, required=True)
    parser.add_argument("--slo", type=float, default=0.95)
    parser.add_argument("--baseline-p95-ms", type=float)
    parser.add_argument("--prior-p95-ms", type=float)
    parser.add_argument("--max-drift-fraction", type=float, default=0.10)
    parser.add_argument("--fingerprint")
    parser.add_argument("--prior-fingerprint")
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
        prior_p95_ms=args.prior_p95_ms,
        max_drift_fraction=args.max_drift_fraction,
        fingerprint=args.fingerprint,
        prior_fingerprint=args.prior_fingerprint,
    )
    result["label"] = args.label
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(encoded)
    else:
        print(encoded, end="")
    return 0 if result["claim_within_all_gates"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
