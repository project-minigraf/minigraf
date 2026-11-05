import unittest

from analyze import ratios


def pair(session, baseline_node, ratio):
    common = {"disposition": "measured", "digest": {"rows": 1, "sum": 9},
              "original_case": "recursion/example", "session": session,
              "block": f"{session}/0/control/0", "case": "expanded"}
    return [{**common, "variant": "baseline", "node": baseline_node, "elapsed_ns": 100},
            {**common, "variant": "control", "node": "3" if baseline_node == "1" else "1",
             "elapsed_ns": 100*ratio}]


class RatioTests(unittest.TestCase):
    def test_pooled_ratio_retains_opposite_session_and_placement_effects(self):
        result = ratios(pair(0, "1", 2) + pair(1, "3", 0.5) + pair(2, "1", 3))[0]
        self.assertEqual(result["median_ratio_to_baseline"], 2)
        self.assertIn("sessions", result)
        self.assertEqual([r["median_ratio_to_baseline"] for r in result["sessions"]], [2, 0.5, 3])
        self.assertEqual([r["baseline_node"] for r in result["sessions"]], ["1", "3", "1"])
        self.assertEqual([r["pairs"] for r in result["placements"]], [2, 1])
        self.assertEqual([r["median_ratio_to_baseline"] for r in result["placements"]], [2.5, 0.5])

    def test_mismatched_sessions_cannot_form_a_pair(self):
        rows = pair(0, "1", 2)
        rows[1]["session"] = 1
        with self.assertRaises(ValueError):
            ratios(rows)


if __name__ == "__main__":
    unittest.main()
