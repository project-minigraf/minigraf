#!/usr/bin/env python3
"""Validate #760 negative controls in a disposable checkout; restore source.

Run from anywhere with cargo on PATH. Builds and tests can be expensive.
Writes JSON evidence to stdout; compiler/test diagnostics go to stderr.
"""

import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "src/graph/storage.rs"
PREFIX = "graph::storage::deterministic::"
CASES = [
    ("per-fact-commit", "batch_append_is_atomic", "partial or duplicated transaction"),
    ("live-snapshot-cutoff", "one_snapshot_keeps_its_cutoff_across_scans",
     "one snapshot saw two database states|snapshot grew after capture"),
    ("split-publication", "checkpoint_publication_preserves_readers_and_late_appends",
     "publication mixed committed and pending generations"),
]
ENV = dict(os.environ)
ENV["RUSTFLAGS"] = ENV.get("RUSTFLAGS", "") + " --cfg minigraf_shuttle"
ENV["MINIGRAF_DST_SEED"] = "760"
ENV["MINIGRAF_DST_ITERATIONS"] = "1000"
ENV.pop("MINIGRAF_DST_REPLAY", None)


def build():
    result = subprocess.run(
        ["cargo", "test", "--locked", "--lib", "--no-run", "--message-format=json"],
        cwd=ROOT, env=ENV, text=True, stdout=subprocess.PIPE, check=True,
    )
    executables = []
    for line in result.stdout.splitlines():
        item = json.loads(line)
        if item.get("reason") == "compiler-message":
            print(item["message"].get("rendered", ""), file=sys.stderr)
        if item.get("reason") == "compiler-artifact" and item.get("executable"):
            if item["target"]["name"] == "minigraf" and "rlib" in item["target"]["kind"]:
                executables.append(item["executable"])
    assert len(executables) == 1, "expected one library test executable"
    return executables[0]


def run(binary, test, schedule=None):
    env = dict(ENV)
    if schedule is not None:
        env["MINIGRAF_DST_REPLAY"] = schedule
    return subprocess.run(
        [binary, PREFIX + test, "--exact", "--nocapture"],
        cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )


def main():
    original = SOURCE.read_bytes()
    evidence = []
    try:
        for name, test, expected in CASES:
            SOURCE.write_bytes(original)
            subprocess.run(
                ["patch", "--batch", "--forward", "-p1", "-i",
                 str(ROOT / "tests/dst-mutations" / (name + ".patch"))],
                cwd=ROOT, stdout=sys.stderr, check=True,
            )
            binary = build()
            result = run(binary, test)
            print(result.stdout, file=sys.stderr)
            assert result.returncode == 101, "mutant must fail with a test assertion"
            assert re.search(expected, result.stdout), "wrong failure reason"
            match = re.search(r'failing schedule:\s*"\s*([0-9a-f]+)\s*"', result.stdout)
            assert match, "missing replayable schedule"
            schedule = match[1]
            for _ in range(10):
                replay = run(binary, test, schedule)
                assert replay.returncode == 101, "replay did not fail"
                assert re.search(expected, replay.stdout), "replay failed for another reason"
            evidence.append({"mutation": name, "test": test, "seed": 760,
                             "schedule": schedule, "reproduced": 10, "attempts": 10})
            print(f"{name}: detected, replayed 10/10", file=sys.stderr)
    finally:
        SOURCE.write_bytes(original)
    # Do not leave a mutant executable as the last build. This also checks
    # that the unchanged implementation survives the same search budget.
    binary = build()
    for _, test, _ in CASES:
        result = run(binary, test)
        print(result.stdout, file=sys.stderr)
        assert result.returncode == 0, "restored implementation failed"
        assert "schedules=1000" in result.stdout, "search budget was not completed"
    print(json.dumps({"negative_controls": evidence, "restored_schedules": 3000}, indent=2))


if __name__ == "__main__":
    main()
