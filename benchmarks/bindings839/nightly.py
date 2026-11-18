"""Run the unchanged #839 WASI driver, then publish a validated nightly stream.

Metrics are nanoseconds: median of ten sample means (total_ns / iterations).
A driver failure, including teardown after the last sample, publishes no metrics.
"""
import argparse
import hashlib
import json
import os
import pathlib
import statistics
import subprocess
import sys

if not __debug__:
    raise RuntimeError('do not disable Python validation with -O')

HERE = pathlib.Path(__file__).resolve().parent
# -I excludes ambient imports; trust only this checkout's sibling validator.
sys.path.insert(0, str(HERE))
from validate import validate, WASI

# Official x86_64-linux release binary, also retained in #839's three raw streams.
WASMTIME_VERSION = 'wasmtime 49.0.2 (3c8a3e79a 2026-10-02)'
WASMTIME_SHA256 = 'd0a014e0d5b0cf48dd3549c38e2e6ecd78ff09fb3e3f10d751b87b72bdfd8635'
FLAGS = ['run', '-C', 'cache=n', '-C', 'parallel-compilation=n']


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def metrics(records):
    validate(records, 'wasi')
    return {name: {'latency': {'value': statistics.median(
        r['total_ns'] / r['iterations'] for r in records
        if r['type'] == 'sample' and r['id'] == name)}}
        for name in sorted('binding/wasi/' + suffix for suffix in WASI)}


def check_environment(env, core_sha, wasm_sha):
    assert env['core_revision'] == core_sha, 'wrong measured core'
    assert env['wasm_sha256'] == wasm_sha, 'wrong executed WASM'
    assert env['runtime_sha256'] == WASMTIME_SHA256, 'wrong Wasmtime binary'
    assert env['runtime_version'] == WASMTIME_VERSION, 'wrong Wasmtime version'
    assert env['runtime_flags'] == FLAGS, 'wrong runtime flags (no preopens allowed)'
    assert env['runtime_cache'] is False
    assert env['storage'] == 'in-memory; no preopened directories'
    assert env['warmups'] == 3 and env['samples'] == 10


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime', type=pathlib.Path, required=True)
    parser.add_argument('--wasm', type=pathlib.Path, required=True)
    parser.add_argument('--core-dir', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--test', action='store_true')
    args = parser.parse_args()
    core_sha = subprocess.check_output(['git', '-C', str(args.core_dir), 'rev-parse', 'HEAD'], text=True).strip()
    assert core_sha == os.environ['BENCH_SHA'], 'checkout differs from once-resolved SHA'
    workflow_sha = os.environ['WORKFLOW_SHA']
    assert len(workflow_sha) == 40 and all(c in '0123456789abcdef' for c in workflow_sha)
    # Check the executable before running even --version.
    assert sha(args.runtime) == WASMTIME_SHA256, 'wrong Wasmtime binary'
    assert subprocess.check_output([str(args.runtime.resolve()), '--version'], text=True).strip() == WASMTIME_VERSION
    wasm_sha = sha(args.wasm)
    harness_files = {name: sha(HERE / name) for name in ('nightly.py', 'wasi.py', 'validate.py')}
    args.output.mkdir(parents=True, exist_ok=True)
    result_path = args.output / 'binding-results.json'
    provenance_path = args.output / 'binding-provenance.json'
    assert not result_path.exists() and not provenance_path.exists(), 'stale nightly outputs'
    mode = 'smoke' if args.test else 'benchmark'
    output = args.output / mode  # driver refuses an existing directory
    subprocess.run([sys.executable, '-I', str(HERE / 'wasi.py'),
                    '--runtime', str(args.runtime.resolve()), '--wasm', str(args.wasm.resolve()),
                    '--core-revision', core_sha, '--output', str(output), '--mode', mode], check=True)
    records = [json.loads(line) for line in (output / 'samples.jsonl').read_text().splitlines()]
    check_environment(records[0], core_sha, wasm_sha)
    assert sha(args.wasm) == wasm_sha and sha(args.runtime) == WASMTIME_SHA256, 'artifact changed during run'
    if args.test:
        assert not any(r['type'] == 'sample' for r in records), 'smoke emitted timing data'
        return
    result = metrics(records)
    provenance = dict(coreSha=core_sha, workflowSha=workflow_sha, bindingSha=None,
                      harnessFiles=harness_files,
                      harnessSha256=hashlib.sha256(json.dumps(harness_files, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
                      artifactSha256=wasm_sha, artifactPath='minigraf.wasm',
                      runtime=dict(version=WASMTIME_VERSION, sha256=WASMTIME_SHA256, flags=FLAGS),
                      environment=records[0])
    with provenance_path.open('x') as stream:
        stream.write(json.dumps(provenance, indent=2) + '\n')
    with result_path.open('x') as stream:
        stream.write(json.dumps(result, indent=2) + '\n')
    for name in result:
        print('ok: ' + name)


if __name__ == '__main__':
    main()
