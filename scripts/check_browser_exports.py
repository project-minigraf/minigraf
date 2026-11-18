#!/usr/bin/env python3
"""Open recovered browser exports with the native CLI and compare their rows.

Capture a successful browser test run with MINIGRAF_EXPORT_RECOVERY=1 and
--nocapture, then pass its log and the native minigraf binary to this script.
Only the Python standard library is needed. Scratch files stay under target/.
"""

import argparse
import base64
import json
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    target = Path(__file__).resolve().parents[1] / "target"
    target.mkdir(exist_ok=True)
    count = 0
    with tempfile.TemporaryDirectory(prefix="browser-exports-", dir=target) as scratch:
        for line in args.log.read_text().splitlines():
            if not line.startswith("MINIGRAF_RECOVERED_EXPORT "):
                continue
            record = json.loads(line.split(" ", 1)[1])
            path = Path(scratch) / f"recovered-{count}.graph"
            path.write_bytes(base64.b64decode(record["bytes"], validate=True))
            result = subprocess.run(
                [str(binary), "--file", str(path)],
                input="(query [:find ?n :where [?e :n ?n]])\n",
                text=True,
                capture_output=True,
                check=True,
            )
            if result.stderr:
                raise RuntimeError(f"native open/query failed for export {count}: {result.stderr}")
            rows = sorted(
                int(row) for row in result.stdout.splitlines() if row.isdecimal()
            )
            if rows != record["expected"]:
                raise RuntimeError(f"native rows differ from the successful prefix in export {count}")
            count += 1
    if not count:
        raise RuntimeError("no exports found; enable MINIGRAF_EXPORT_RECOVERY=1 and --nocapture")
    print(f"Native open and query passed for {count} recovered browser exports.")


if __name__ == "__main__":
    main()
