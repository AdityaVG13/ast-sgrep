#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import unittest


def load_limiter():
    path = Path(__file__).with_name("cpu-limit-exec.py")
    spec = importlib.util.spec_from_file_location("cpu_limit_exec", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class DutyCycleTest(unittest.TestCase):
    def test_matches_rust_millisecond_quantization(self):
        limiter = load_limiter()
        expected = {
            1: (0.001, 0.009),
            5: (0.001, 0.009),
            9: (0.001, 0.009),
            10: (0.001, 0.009),
            80: (0.008, 0.002),
        }
        for limit, quanta in expected.items():
            with self.subTest(limit=limit):
                self.assertEqual(limiter.duty_cycle_seconds(limit, 10), quanta)


if __name__ == "__main__":
    unittest.main()
