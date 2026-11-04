"""Checks cannot silently disappear under Python's optimization flag."""
import pathlib
import subprocess
import sys
import unittest

HERE = pathlib.Path(__file__).resolve().parent


class GuardrailTests(unittest.TestCase):
    def test_optimized_driver_is_rejected(self):
        result = subprocess.run([sys.executable, '-O', str(HERE / 'wasi.py'), '--help'],
                                capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('do not disable', result.stderr)

    def test_optimized_evidence_validator_is_rejected(self):
        evidence = HERE.parent.parent / 'docs/benchmarks/839-bindings/evidence/wasi-1.jsonl'
        result = subprocess.run([sys.executable, '-O', str(HERE / 'validate.py'), 'wasi', str(evidence)],
                                capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('do not disable', result.stderr)


if __name__ == '__main__':
    unittest.main()
