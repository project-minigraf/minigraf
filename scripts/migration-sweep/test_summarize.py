"""Evidence-integrity checks; run before collecting the manual sweep."""
import unittest
from summarize import summarize


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.fixtures = [{"file": "fixture.graph", "sha256": "frozen"}]
        self.rows = [{"fixture": "fixture.graph", "sha256": "frozen", "operation": op,
                      "sample": 0, "ns": 100, "correctness": "passed"}
                     for op in ("migrate_v7", "read_only_v7", "reopen_v8",
                                "churn_pinned", "churn_unpinned")]

    def test_complete_native_run(self):
        self.assertEqual(len(summarize(self.fixtures, self.rows, 1, False, False)), 5)

    def test_missing_or_duplicate_samples_rejected(self):
        for rows in (self.rows[:-1], self.rows + [self.rows[0]]):
            with self.assertRaises(AssertionError):
                summarize(self.fixtures, rows, 1, False, False)

    def test_fixture_substitution_rejected(self):
        self.rows[0]["sha256"] = "different"
        with self.assertRaises(AssertionError):
            summarize(self.fixtures, self.rows, 1, False, False)

    def test_correctness_run_cannot_be_reported_as_timing(self):
        for row in self.rows:
            row["ns"] = None
        self.assertEqual(summarize(self.fixtures, self.rows, 1, False, True), [])
        with self.assertRaises(AssertionError):
            summarize(self.fixtures, self.rows, 1, False, False)

    def test_browser_cannot_claim_read_only_coverage(self):
        with self.assertRaises(AssertionError):
            summarize(self.fixtures, self.rows, 1, True, False)


if __name__ == "__main__":
    unittest.main()
