import copy
import unittest

from evidence import validate


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.cases = [{"id": "scalar/100", "family": "scalar"}]
        self.rows = [{"case": "scalar/100", "session": 0, "sample": 0,
                      "verified": True, "elapsed_ns": 12, "rows": 10,
                      "digest": {"rows": 10, "sum": 123},
                      "expected": {"rows": 10, "sum": 123},
                      "exit_status": 0, "disposition": "measured"}]

    def test_complete_checked_sample(self):
        self.assertEqual(validate(self.cases, self.rows, 1, 1)["measured"], 1)

    def test_missing_duplicate_unknown_or_wrong_answer_is_rejected(self):
        for change in ("missing", "duplicate", "unknown", "wrong", "nan", "unchecked", "exit"):
            rows = copy.deepcopy(self.rows)
            if change == "missing": rows = []
            if change == "duplicate": rows += copy.deepcopy(rows)
            if change == "unknown": rows[0]["case"] = "other"
            if change == "wrong": rows[0]["digest"]["sum"] = 999
            if change == "nan": rows[0]["elapsed_ns"] = float("nan")
            if change == "unchecked": rows[0]["verified"] = False
            if change == "exit": rows[0]["exit_status"] = 1
            with self.subTest(change=change), self.assertRaises(ValueError):
                validate(self.cases, rows, 1, 1)

    def test_timeout_is_a_disposition_not_a_zero_latency(self):
        row = {"case": "scalar/100", "session": 0, "sample": 0,
               "disposition": "timeout", "elapsed_lower_bound_ns": 1000,
               "exit_status": -9, "reason": "per-case deadline"}
        result = validate(self.cases, [row], 1, 1)
        self.assertEqual(result["measured"], 0)
        self.assertEqual(result["timeout"], 1)
        row["elapsed_ns"] = 0
        with self.assertRaises(ValueError):
            validate(self.cases, [row], 1, 1)


if __name__ == "__main__":
    unittest.main()
