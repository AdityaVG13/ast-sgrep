import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "check-error-budget.py"
SPEC = importlib.util.spec_from_file_location("check_error_budget", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ErrorBudgetTests(unittest.TestCase):
    def test_hard_threshold_counts_every_exceedance(self):
        result = MODULE.evaluate([0.251] * 10, threshold_ms=250.0, slo=0.95, baseline_p95_ms=258.4)

        self.assertEqual(result["exceedance_count"], 10)
        self.assertEqual(result["error_rate"], 1.0)
        self.assertAlmostEqual(result["burn_rate"], 20.0)
        self.assertFalse(result["gates"]["baseline_within_threshold"])
        self.assertFalse(result["claim_within_slo"])


if __name__ == "__main__":
    unittest.main()
