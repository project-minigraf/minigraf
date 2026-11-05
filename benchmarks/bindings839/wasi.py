"""Actual WASI REPL timings; host framing/pipe overhead is included explicitly."""
import argparse
import hashlib
import json
import pathlib
import queue
import re
import subprocess
import threading
import time

if not __debug__:
    raise RuntimeError('do not disable Python validation with -O')


def check_process(status, stderr):
    assert status == 0, 'WASI process failed'
    assert not stderr.strip(), 'WASI process reported an error'


def check_query_output(output, expected):
    lines = output.splitlines()
    if lines and re.fullmatch(r'✓ Transacted successfully \(tx: \d+\)', lines[0]):
        lines = lines[1:]
    assert lines[:2] == ['?n', '-' * 20], 'missing or wrong result header'
    assert lines[-2] == '', 'missing result terminator'
    expected = list(expected)
    assert lines[-1] == f'{len(expected)} result(s) found.', 'wrong reported count'
    rows = [int(line) for line in lines[2:-2]]
    assert len(rows) == len(expected), 'wrong row count'
    assert sum(rows) == sum(expected), 'wrong row sum'
    assert sorted(rows) == sorted(expected), 'wrong row values'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime', type=pathlib.Path, required=True)
    parser.add_argument('--wasm', type=pathlib.Path, required=True)
    parser.add_argument('--core-revision', required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--mode', choices=['smoke', 'benchmark'], default='smoke')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    samples_file = args.output / 'samples.jsonl'
    def emit(record):
        with samples_file.open('a') as stream:
            stream.write(json.dumps(record) + '\n')
    flags = ['run', '-C', 'cache=n', '-C', 'parallel-compilation=n']
    command = [str(args.runtime.resolve()), *flags, str(args.wasm.resolve())]
    sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    emit(dict(type='environment', mode=args.mode, core_revision=args.core_revision,
              wasm_sha256=sha(args.wasm), runtime_sha256=sha(args.runtime),
              runtime_version=subprocess.check_output([str(args.runtime), '--version'], text=True).strip(),
              runtime_flags=flags, storage='in-memory; no preopened directories',
              runtime_cache=False, host_driver='Python subprocess pipes; framing overhead included',
              warmups=3, samples=10, rss_verified=False))
    transaction = '(transact [' + ' '.join(f'[:bench/e{i} :bench/n {i}]' for i in range(1000)) + '])\n'
    query = '(query [:find ?n :where [?e :bench/n ?n]])\n'
    expected = list(range(1000))
    (args.output / 'fixture.datalog').write_text(transaction + query)
    checked = []
    def measure(name, action, validate, iterations=1):
        case = 'binding/wasi/' + name
        def invoke():
            start = time.perf_counter_ns()
            value = action()
            elapsed = time.perf_counter_ns() - start
            validate(value)
            return elapsed
        if args.mode == 'benchmark':
            for _ in range(3):
                invoke()
            for sample in range(10):
                total = sum(invoke() for _ in range(iterations))
                emit(dict(type='sample', id=case, sample=sample, iterations=iterations,
                          total_ns=total, ns_per_op=total / iterations))
        else:
            invoke()
        checked.append(case)
        emit(dict(type='checked', id=case))
    def process(script):
        return subprocess.run(command, input=script, text=True, capture_output=True, timeout=60)
    def empty_valid(result):
        check_process(result.returncode, result.stderr)
        assert result.stdout == '', 'unexpected empty-input output'
    def script_valid(result):
        check_process(result.returncode, result.stderr)
        check_query_output(result.stdout, expected)
    measure('repl_startup_empty', lambda: process(''), empty_valid)
    measure('repl_script/1k', lambda: process(transaction + query), script_valid)

    # Start/compile/seed outside timing; validate every timed response before
    # accepting its sample. No inferred engine-only latency by subtraction.
    with (args.output / 'persistent-stderr.txt').open('w+') as errors:
        proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=errors, text=True, bufsize=1)
        lines = queue.Queue()
        def reader():
            for line in proc.stdout:
                lines.put(line)
            lines.put(None)
        thread = threading.Thread(target=reader, daemon=True)
        thread.start()
        def get_line():
            line = lines.get(timeout=30)
            assert line is not None, 'REPL exited before completing its response'
            return line
        def query_once():
            proc.stdin.write(query)
            proc.stdin.flush()
            response = []
            while True:
                line = get_line()
                response.append(line)
                if re.fullmatch(r'\d+ result\(s\) found\.\n', line):
                    return ''.join(response)
        try:
            proc.stdin.write(transaction)
            proc.stdin.flush()
            assert re.fullmatch(r'✓ Transacted successfully \(tx: \d+\)\n', get_line())
            check_query_output(query_once(), expected)
            measure('repl_query_roundtrip/1k', query_once,
                    lambda output: check_query_output(output, expected), iterations=20)
            proc.stdin.close()
            proc.wait(timeout=30)
            thread.join(timeout=5)
            errors.flush()
            errors.seek(0)
            check_process(proc.returncode, errors.read())
            assert lines.get(timeout=5) is None, 'unexpected trailing REPL output'
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait()
            proc.stdout.close()
    assert len(checked) == 3
    emit(dict(type='complete', mode=args.mode, ids=checked, count=len(checked)))
    print('Validated', len(checked), 'WASI cases in', args.mode, 'mode')


if __name__ == '__main__':
    main()
