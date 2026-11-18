"""Nightly conversion keeps #839's strict oracle and raw-sample statistic."""
import copy
import contextlib
import hashlib
import io
import os
import importlib.util
import json
import pathlib
import statistics
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

HERE = pathlib.Path(__file__).resolve().parent
def fixture():
    # Synthetic unit-test data, never qualification or baseline measurements.
    # Self-contained so the main CI companion needs no historical evidence copy.
    records = [dict(type='environment', mode='benchmark', core_revision='c' * 40,
                    wasm_sha256='d' * 64,
                    runtime_sha256='d0a014e0d5b0cf48dd3549c38e2e6ecd78ff09fb3e3f10d751b87b72bdfd8635',
                    runtime_version='wasmtime 49.0.2 (3c8a3e79a 2026-10-02)',
                    runtime_flags=['run', '-C', 'cache=n', '-C', 'parallel-compilation=n'],
                    storage='in-memory; no preopened directories', runtime_cache=False,
                    warmups=3, samples=10)]
    ids = ['binding/wasi/' + suffix for suffix in
           ('repl_startup_empty', 'repl_script/1k', 'repl_query_roundtrip/1k')]
    for index, name in enumerate(ids):
        iterations = 20 if index == 2 else 1
        for sample in range(10):
            total = (101 + sample * 17) * iterations + index
            records.append(dict(type='sample', id=name, sample=sample,
                                iterations=iterations, total_ns=total, ns_per_op=total / iterations))
        records.append(dict(type='checked', id=name))
    records.append(dict(type='complete', mode='benchmark', count=3, ids=ids))
    return records


class NightlyTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue((HERE / 'nightly.py').is_file(), 'nightly adapter is required')
        spec = importlib.util.spec_from_file_location('nightly', HERE / 'nightly.py')
        self.adapter = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.adapter)
        self.records = fixture()

    def test_raw_recomputation_of_every_metric(self):
        metrics = self.adapter.metrics(self.records)
        self.assertEqual(len(metrics), 3)
        for name, metric in metrics.items():
            samples = [r for r in self.records if r.get('id') == name and r['type'] == 'sample']
            expected = statistics.median(r['total_ns'] / r['iterations'] for r in samples)
            self.assertEqual(metric, {'latency': {'value': expected}})

    def test_rejects_invalid_or_incomplete_streams(self):
        sample = next(i for i, r in enumerate(self.records) if r['type'] == 'sample')
        variants = [self.records[:-1], self.records + [self.records[-1]], self.records + [self.records[sample]]]
        for key, value in [('total_ns', 0), ('total_ns', -1), ('iterations', 2),
                           ('ns_per_op', float('nan')), ('ns_per_op', float('inf')),
                           ('ns_per_op', 123), ('id', 'binding/wasi/extra')]:
            records = copy.deepcopy(self.records)
            records[sample][key] = value
            variants.append(records)
        smoke = copy.deepcopy(self.records)
        smoke[0]['mode'] = 'smoke'
        variants.append(smoke)
        for records in variants:
            with self.subTest(records=records[sample]):
                with self.assertRaises((AssertionError, ValueError)):
                    self.adapter.metrics(records)

    def test_runtime_pin_and_flags_are_enforced(self):
        env = self.records[0]
        self.adapter.check_environment(env, env['core_revision'], env['wasm_sha256'])
        for key, value in [('runtime_sha256', '0' * 64), ('runtime_version', 'wasmtime 49.0.3'),
                           ('runtime_flags', ['run', '--dir', '.']), ('runtime_cache', True),
                           ('warmups', 0), ('samples', 1), ('core_revision', 'a' * 40),
                           ('wasm_sha256', 'b' * 64)]:
            with self.subTest(key=key), self.assertRaises((AssertionError, ValueError)):
                self.adapter.check_environment(dict(env, **{key: value}), env['core_revision'], env['wasm_sha256'])

    def test_optimized_adapter_is_rejected(self):
        result = subprocess.run([sys.executable, '-I', '-O', str(HERE / 'nightly.py'), '--help'],
                                capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('do not disable', result.stderr)

    def test_publish_only_after_successful_exit_and_reject_stale_outputs(self):
        for mode in ('success', 'smoke', 'late-exit-failure'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                root = pathlib.Path(tmp)
                runtime, wasm = root / 'runtime', root / 'minigraf.wasm'
                runtime.write_bytes(b'test runtime')
                wasm.write_bytes(b'test artifact')
                runtime_sha = hashlib.sha256(runtime.read_bytes()).hexdigest()
                records = copy.deepcopy(self.records)
                records[0]['runtime_sha256'] = runtime_sha
                records[0]['wasm_sha256'] = hashlib.sha256(wasm.read_bytes()).hexdigest()
                if mode == 'smoke':
                    records = [r for r in records if r['type'] != 'sample']
                    records[0]['mode'] = records[-1]['mode'] = 'smoke'

                def driver(command, **kwargs):
                    output = pathlib.Path(command[command.index('--output') + 1])
                    output.mkdir()  # the real driver's fresh-directory rule
                    (output / 'samples.jsonl').write_text('\n'.join(map(json.dumps, records)))
                    if mode == 'late-exit-failure':
                        # Even a complete stream cannot override process failure.
                        raise subprocess.CalledProcessError(1, command)

                argv = ['nightly.py', '--runtime', str(runtime), '--wasm', str(wasm),
                        '--core-dir', str(root), '--output', str(root)]
                if mode == 'smoke':
                    argv.append('--test')
                with mock.patch.object(sys, 'argv', argv), \
                     mock.patch.dict(os.environ, BENCH_SHA=records[0]['core_revision'], WORKFLOW_SHA='a' * 40), \
                     mock.patch.object(self.adapter, 'WASMTIME_SHA256', runtime_sha), \
                     mock.patch.object(self.adapter.subprocess, 'check_output', side_effect=[records[0]['core_revision'], self.adapter.WASMTIME_VERSION]), \
                     mock.patch.object(self.adapter.subprocess, 'run', side_effect=driver), \
                     contextlib.redirect_stdout(io.StringIO()) as stdout:
                    if mode == 'late-exit-failure':
                        with self.assertRaises(subprocess.CalledProcessError):
                            self.adapter.main()
                    else:
                        self.adapter.main()
                metrics_file = root / 'binding-results.json'
                self.assertEqual(metrics_file.exists(), mode == 'success')
                self.assertEqual((root / 'binding-provenance.json').exists(), mode == 'success')
                self.assertEqual(stdout.getvalue().count('ok: '), 3 if mode == 'success' else 0)
                if mode == 'success':
                    self.assertEqual(json.loads(metrics_file.read_text()), self.adapter.metrics(records))
                    with mock.patch.object(sys, 'argv', argv), \
                         mock.patch.dict(os.environ, BENCH_SHA=records[0]['core_revision'], WORKFLOW_SHA='a' * 40), \
                         mock.patch.object(self.adapter, 'WASMTIME_SHA256', runtime_sha), \
                         mock.patch.object(self.adapter.subprocess, 'check_output', side_effect=[records[0]['core_revision'], self.adapter.WASMTIME_VERSION]), \
                         mock.patch.object(self.adapter.subprocess, 'run') as run:
                        with self.assertRaisesRegex(AssertionError, 'stale'):
                            self.adapter.main()
                        run.assert_not_called()

    def test_driver_failure_cannot_publish_metrics(self):
        # Exercise the real runner/REPL parser with wrong-but-plausible output.
        # This substitute runtime is test-only; production's binary pin rejects it.
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            runtime = tmp / 'fake-runtime'
            runtime.write_text('''#!/usr/bin/env python3
import sys
if '--version' in sys.argv:
    print('fake runtime')
    sys.exit(0)
script = sys.stdin.read()
if script:
    print('✓ Transacted successfully (tx: 1)')
    print('?n\\n' + '-' * 20)
    # 1000 rows with the right sum, but wrong values.
    print('\\n'.join(map(str, [0, 0, 3, 3] + list(range(4, 1000)))))
    print('\\n1000 result(s) found.')
''')
            runtime.chmod(0o755)
            wasm = tmp / 'fake.wasm'
            wasm.write_bytes(b'fixture')
            output = tmp / 'run'
            result = subprocess.run([sys.executable, '-I', str(HERE / 'wasi.py'),
                                     '--runtime', str(runtime), '--wasm', str(wasm),
                                     '--core-revision', 'test', '--mode', 'benchmark',
                                     '--output', str(output)], capture_output=True, text=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('wrong row values', result.stderr)
            records = [json.loads(line) for line in (output / 'samples.jsonl').read_text().splitlines()]
            with self.assertRaises(AssertionError):
                self.adapter.metrics(records)
            self.assertFalse((output / 'binding-results.json').exists())


if __name__ == '__main__':
    unittest.main()
