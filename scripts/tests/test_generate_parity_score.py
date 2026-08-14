#!/usr/bin/env python3
from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EMITTER = ROOT / "scripts/generate-parity-score.py"


class ParityScoreTest(unittest.TestCase):
    def test_seed_is_red_and_uncertified(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            out = Path(raw) / "parity_score.json"
            completed = subprocess.run(
                ["python3", str(EMITTER), "--out", str(out)],
                cwd=ROOT,
                check=False,
            )
            self.assertEqual(completed.returncode, 0)
            payload = json.loads(out.read_text())
            self.assertFalse(payload["certified"])
            self.assertEqual(payload["band"], "red")
            self.assertEqual(payload["release_certificate"], "refused")
            self.assertEqual(payload["lower_bound"], 0.0)
            low, high = payload["interval"]
            self.assertEqual(low, 0.0)
            self.assertGreaterEqual(high, low)
            self.assertFalse(payload["point_estimate_is_certified"])
            self.assertTrue(payload["truncate_policy"]["present_count_is_not_green"])
            self.assertTrue(payload["forbidden_victory"])
            self.assertNotIn("green", payload["band"])


if __name__ == "__main__":
    unittest.main()
