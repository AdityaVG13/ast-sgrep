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


    def test_surface_burn_rates_are_independent(self):
        semantic = MODULE.evaluate([0.011, 0.012] + [0.009] * 18, 10.0, 0.95)
        literal = MODULE.evaluate([0.011, 0.012] + [0.009] * 18, 10.0, 0.95)
        natural_language = MODULE.evaluate([0.011] + [0.009] * 19, 10.0, 0.95)

        self.assertAlmostEqual(semantic["burn_rate"], 2.0)
        self.assertAlmostEqual(literal["burn_rate"], 2.0)
        self.assertAlmostEqual(natural_language["burn_rate"], 1.0)
        self.assertFalse(semantic["gates"]["p95_within_threshold"])
        self.assertTrue(natural_language["gates"]["burn_rate_within_budget"])

    def test_variance_envelope_does_not_override_hard_threshold(self):
        result = MODULE.evaluate(
            [0.2584] * 10,
            threshold_ms=250.0,
            slo=0.95,
            prior_p95_ms=250.0,
            fingerprint="same-host",
            prior_fingerprint="same-host",
        )

        self.assertFalse(result["claim_within_slo"])
        self.assertTrue(result["variance_gate"]["within_envelope"])
        self.assertAlmostEqual(result["variance_gate"]["drift_fraction"], 0.0336)
        self.assertFalse(result["claim_within_all_gates"])

if __name__ == "__main__":
    unittest.main()
