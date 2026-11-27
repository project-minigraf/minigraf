#!/usr/bin/env python3
"""Recompute both reports locally and compare bytes without rewriting evidence."""
import argparse
import importlib.util
from pathlib import Path
import sys

# Explicit sibling loading works with Python -I, without a sys.path override.
_spec = importlib.util.spec_from_file_location('qualification_decide', Path(__file__).with_name('decide.py'))
decide = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(decide)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('raw', help='JSON file or directory containing raw-samples.json')
    parser.add_argument('report', help='stored report prefix (or its .json path)')
    args = parser.parse_args()
    try:
        report = decide.decide(decide.load_raw(args.raw))
        prefix = Path(args.report)
        mismatches = []
        for suffix, text in (('.json', decide.render_json(report)),
                             ('.md', decide.render_markdown(report))):
            if prefix.with_suffix(suffix).read_bytes() != text.encode('utf-8'):
                mismatches.append(suffix)
        if mismatches:
            print('report mismatch: ' + ', '.join(mismatches), file=sys.stderr)
            return 1
    except (ValueError, OSError, OverflowError) as error:
        print('error: ' + str(error), file=sys.stderr)
        return 2
    print('JSON and Markdown reports reproduce byte-for-byte.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
