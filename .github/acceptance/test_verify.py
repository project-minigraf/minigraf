"""Regression checks for false-green acceptance output (no third-party modules)."""
import unittest

from verify import verify


class AcceptanceOutputTests(unittest.TestCase):
    def output(self, names, passed=None, ignored=0):
        body = ''.join(f'test {name} ... measured output\nok\n' for name in names)
        n = len(names) if passed is None else passed
        return body + f'test result: ok. {n} passed; 0 failed; {ignored} ignored; 0 measured; 0 filtered out; finished in 1.00s\n'

    def test_all_named_tests_pass(self):
        verify(self.output(['a', 'b']), ['a', 'b'], [])

    def test_zero_tests_rejected(self):
        with self.assertRaises(ValueError):
            verify(self.output([]), ['a'], [])

    def test_missing_or_renamed_test_rejected(self):
        for names in (['a'], ['a', 'renamed'], ['a', 'a']):
            with self.subTest(names=names), self.assertRaises(ValueError):
                verify(self.output(names), ['a', 'b'], [])

    def test_ignored_or_failed_test_rejected(self):
        for output in (self.output(['a'], passed=0, ignored=1), self.output(['a']).replace('ok. 1 passed; 0 failed', 'FAILED. 0 passed; 1 failed')):
            with self.subTest(output=output), self.assertRaises(ValueError):
                verify(output, ['a'], [])

    def test_truncated_or_multiple_summaries_rejected(self):
        for output in ('test a ... ok\n', self.output(['a']) * 2):
            with self.subTest(output=output), self.assertRaises(ValueError):
                verify(output, ['a'], [])

    def test_required_rss_check_cannot_skip(self):
        for marker in ('', 'RSS_CHECK probe skipped platform=macos\n'):
            with self.subTest(marker=marker), self.assertRaises(ValueError):
                verify(self.output(['a']) + marker, ['a'], ['probe'])

    def test_measured_rss_check_accepted(self):
        verify(self.output(['a']) + 'RSS_CHECK probe measured added_bytes=32 bound_bytes=64\n', ['a'], ['probe'])

    def test_storage_test_full_name_is_required(self):
        name = 'storage::persistent_facts::tests::size_per_fact_in_433_shape_1m'
        verify(self.output([name]), [name], [])
        with self.assertRaises(ValueError):
            verify(self.output(['unrelated']), [name], [])


if __name__ == '__main__':
    unittest.main()
