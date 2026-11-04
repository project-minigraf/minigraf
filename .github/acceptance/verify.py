"""Reject empty/partial acceptance runs; invoked from the pinned workflow checkout."""
import argparse
from pathlib import Path
import re

SUITES = {
    'recursive': (
        ['reach_5m', 'nonlinear_closure_scaling', 'multistratum_recursion_scaling'],
        ['reach_5m', 'nonlinear_256', 'nonlinear_512', 'multistratum_256', 'multistratum_512'],
    ),
    'memory-limit': (
        ['cartesian_k13_streams', 'count_holds_no_rows', 'grouped_count_1m_groups'],
        ['cartesian_k13_streams', 'count_holds_no_rows'],
    ),
    'limit': (['limit_and_top_k_over_10m_facts'], ['limit_10m', 'top_k_10m']),
    'large-transaction': (['one_million_fact_transact_holds_one_chunk', 'custom_chunk_bounds'], []),
    'size-per-fact': (['storage::persistent_facts::tests::size_per_fact_in_433_shape_1m'], []),
}


def verify(output, expected_tests, expected_rss):
    output = re.sub(r'\x1b\[[0-9;]*m', '', output)
    names = re.findall(r'^test (\S+) \.\.\.', output, re.MULTILINE)
    if not expected_tests or sorted(names) != sorted(expected_tests):
        raise ValueError(f'executed tests {names}, expected {expected_tests}')
    summaries = re.findall(r'^test result: (.+)$', output, re.MULTILINE)
    prefix = f'ok. {len(expected_tests)} passed; 0 failed; 0 ignored;'
    if len(summaries) != 1 or not summaries[0].startswith(prefix):
        raise ValueError(f'expected one complete passing summary, got {summaries}')
    # With --nocapture the first record can follow "test name ... " on a line.
    measured = re.findall(r'RSS_CHECK (\S+) measured added_bytes=\d+ bound_bytes=\d+', output)
    if sorted(measured) != sorted(expected_rss):
        raise ValueError(f'measured RSS checks {measured}, expected {expected_rss}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('suite', choices=SUITES)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    try:
        verify(args.output.read_text(), *SUITES[args.suite])
    except ValueError as error:
        parser.exit(1, f'Acceptance evidence rejected: {error}\n')
    print(f'Acceptance evidence verified: {args.suite}')
