import unittest

from wasi import check_query_output, check_process


class ReplOracleTests(unittest.TestCase):
    def test_accepts_unordered_complete_rows(self):
        check_query_output('?n\n--------------------\n3\n1\n0\n2\n\n4 result(s) found.\n', range(4))

    def test_rejects_empty_success(self):
        with self.assertRaises(AssertionError):
            check_query_output('No results found.\n', range(4))

    def test_rejects_wrong_rows_with_equal_count_and_sum(self):
        with self.assertRaises(AssertionError):
            check_query_output('?n\n--------------------\n0\n0\n3\n3\n\n4 result(s) found.\n', range(4))

    def test_rejects_error_even_with_zero_exit(self):
        with self.assertRaises(AssertionError):
            check_process(0, 'Error: query failed\n')

    def test_rejects_nonzero_exit(self):
        with self.assertRaises(AssertionError):
            check_process(1, '')


if __name__ == '__main__':
    unittest.main()
