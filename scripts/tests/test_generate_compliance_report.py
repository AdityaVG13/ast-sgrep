#!/usr/bin/env python3
from __future__ import annotations

import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EMITTER = ROOT / "scripts/generate-compliance-report.py"
REGISTRY = ROOT / "tests/conformance/registry.toml"


class ComplianceEmitterTest(unittest.TestCase):
    def test_registry_only_writes_not_run_rows(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            out = Path(raw) / "COMPLIANCE_REPORT.md"
            jsonl = Path(raw) / "COMPLIANCE_REPORT.jsonl"
            completed = subprocess.run(
                [
                    "python3",
                    str(EMITTER),
                    "--registry",
                    str(REGISTRY),
                    "--out",
                    str(out),
                    "--jsonl",
                    str(jsonl),
                    "--registry-only",
                    "--tier",
                    "proof-pack",
                ],
                cwd=ROOT,
                check=False,
            )
            self.assertEqual(completed.returncode, 0)
            body = out.read_text()
            self.assertIn("**Not-run**", body)
            self.assertIn("`ranking_oracle`", body)
            self.assertIn("DISCREPANCIES.md", body)
            self.assertGreaterEqual(len(jsonl.read_text().splitlines()), 6)

    def test_simulate_fail_still_writes_report(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            out = Path(raw) / "COMPLIANCE_REPORT.md"
            completed = subprocess.run(
                [
                    "python3",
                    str(EMITTER),
                    "--registry",
                    str(REGISTRY),
                    "--out",
                    str(out),
                    "--no-jsonl",
                    "--registry-only",
                    "--simulate-fail",
                    "ranking_oracle",
                    "--tier",
                    "proof-pack",
                ],
                cwd=ROOT,
                check=False,
            )
            self.assertEqual(completed.returncode, 1)
            self.assertTrue(out.is_file())
            body = out.read_text()
            self.assertIn("| `ranking_oracle` |", body)
            self.assertIn("| **Fail** |", body)

    def test_missing_required_env_is_not_run(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            temp = Path(raw)
            registry = temp / "registry.toml"
            registry.write_text(
                textwrap.dedent(
                    """
                    [[suite]]
                    id = "external"
                    label = "external oracle"
                    tier = "extended"
                    required_env = ["ASGREP_TEST_MISSING_ORACLE"]
                    command = ["python3", "-c", "raise SystemExit(99)"]
                    """
                )
            )
            out = temp / "COMPLIANCE_REPORT.md"
            completed = subprocess.run(
                [
                    "python3",
                    str(EMITTER),
                    "--registry",
                    str(registry),
                    "--out",
                    str(out),
                    "--no-jsonl",
                    "--tier",
                    "extended",
                ],
                cwd=ROOT,
                check=False,
            )
            self.assertEqual(completed.returncode, 0)
            self.assertIn("| `external` | external oracle | extended | **Not-run** |", out.read_text())


if __name__ == "__main__":
    unittest.main()
