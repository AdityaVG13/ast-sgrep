#!/usr/bin/env python3
"""Emit a Pass/Fail/Not-run compliance matrix from tests/conformance/registry.toml.

Always writes the report, including when a suite fails. Exit 1 if any executed
suite failed. Never invents a MUST% score.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tomllib
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REGISTRY = ROOT / "tests/conformance/registry.toml"
DEFAULT_REPORT = ROOT / "tests/artifacts/compliance/COMPLIANCE_REPORT.md"
DEFAULT_JSONL = ROOT / "tests/artifacts/compliance/COMPLIANCE_REPORT.jsonl"


def load_registry(path: Path) -> list[dict[str, Any]]:
    data = tomllib.loads(path.read_text())
    suites = data.get("suite")
    if not isinstance(suites, list) or not suites:
        raise SystemExit(f"no [[suite]] entries in {path}")
    return suites


def run_suite(suite: dict[str, Any], *, registry_only: bool, simulate_fail: str | None) -> str:
    ident = str(suite["id"])
    if simulate_fail == ident:
        return "Fail"
    if registry_only:
        return "Not-run"
    required_env = [str(name) for name in suite.get("required_env", [])]
    if any(not os.environ.get(name) for name in required_env):
        return "Not-run"
    command = [str(part) for part in suite["command"]]
    completed = subprocess.run(command, cwd=ROOT, check=False)
    return "Pass" if completed.returncode == 0 else "Fail"


def render_markdown(
    suites: list[dict[str, Any]],
    scores: list[str],
    *,
    mode: str,
) -> str:
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    lines = [
        "# Compliance report",
        "",
        f"Generated: `{now}`",
        f"Mode: `{mode}`",
        "Score column is Pass / Fail / Not-run only. No MUST%.",
        "",
        "| ID | Label | Tier | Score |",
        "|---|---|---|---|",
    ]
    for suite, score in zip(suites, scores, strict=True):
        lines.append(
            f"| `{suite['id']}` | {suite['label']} | {suite['tier']} | **{score}** |"
        )
    lines += [
        "",
        "## Intentional discrepancies",
        "",
        "Non-claims: `docs/validation/DISCREPANCIES.md`.",
        "Coverage skeleton: `docs/validation/COVERAGE.md`.",
        "Verdicts: `docs/validation/conformance-verdicts.md`.",
        "",
        "Not-run is not Pass. Do not quote bench MRR or latency here.",
        "",
    ]
    return "\n".join(lines)


def write_outputs(
    report: Path,
    jsonl: Path | None,
    suites: list[dict[str, Any]],
    scores: list[str],
    *,
    mode: str,
) -> None:
    report.parent.mkdir(parents=True, exist_ok=True)
    report.write_text(render_markdown(suites, scores, mode=mode))
    if jsonl is not None:
        jsonl.parent.mkdir(parents=True, exist_ok=True)
        with jsonl.open("w") as handle:
            for suite, score in zip(suites, scores, strict=True):
                handle.write(
                    json.dumps(
                        {
                            "id": suite["id"],
                            "label": suite["label"],
                            "tier": suite["tier"],
                            "score": score,
                        }
                    )
                    + "\n"
                )


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--out", type=Path, default=DEFAULT_REPORT)
    parser.add_argument("--jsonl", type=Path, default=DEFAULT_JSONL)
    parser.add_argument("--no-jsonl", action="store_true")
    parser.add_argument(
        "--registry-only",
        action="store_true",
        help="Do not execute suites; every row is Not-run.",
    )
    parser.add_argument(
        "--simulate-fail",
        metavar="ID",
        help="Force one suite id to Fail (emitter fail-path; still writes report).",
    )
    parser.add_argument(
        "--tier",
        default="proof-pack",
        help="proof-pack (default), extended, or all",
    )
    return parser.parse_args(argv)


def selected(suites: list[dict[str, Any]], tier: str) -> list[dict[str, Any]]:
    if tier == "all":
        return suites
    return [suite for suite in suites if suite.get("tier") == tier]


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    suites = selected(load_registry(args.registry), args.tier)
    if not suites:
        raise SystemExit(f"no suites for tier {args.tier}")
    mode = "registry-only" if args.registry_only else "run"
    if args.simulate_fail:
        mode = f"{mode}+simulate-fail:{args.simulate_fail}"
    scores = [
        run_suite(
            suite,
            registry_only=args.registry_only,
            simulate_fail=args.simulate_fail,
        )
        for suite in suites
    ]
    jsonl = None if args.no_jsonl else args.jsonl
    write_outputs(args.out, jsonl, suites, scores, mode=mode)
    if any(score == "Fail" for score in scores):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
