# #858 mobile qualification preparation

**No real timing data, mobile qualification, lane activation or Part B completion.**
The protocol must be committed with completed configuration pins before timing;
this preparation leaves changes uncommitted as requested.

- [PROTOCOL.md](PROTOCOL.md): copied AC12 gate, fixed user decisions, categories,
  candidate TODOs and pre-timing checklist.
- [SCHEMA.md](SCHEMA.md): run/attempt input, provenance requirements, calculations
  and deterministic report/exit-code contract.
- [decide.py](decide.py): Python standard-library per-case Welch/CV decision tool.
- [test_decide.py](test_decide.py): positive, negative, boundary, retry,
  degenerate-data, serialization and reproduction controls.
- [reproduce.py](reproduce.py): read-only byte comparison of regenerated reports.
- [fixtures/](fixtures/): **synthetic, NOT real data**, used only by tests/examples;
  `synthetic.json`, `synthetic-report.json`, `synthetic-report.md`.

From this directory, with Python 3.10+:

```sh
python3 -I -m unittest discover -s . -p 'test_*.py'
python3 -I reproduce.py fixtures/synthetic.json fixtures/synthetic-report
```

From the repository root:

```sh
python3 -I -m unittest discover -s docs/benchmarks/858-mobile-qualification -p 'test_*.py'
```

To exercise generation without replacing the stored fixture:

```sh
python3 -I decide.py fixtures/synthetic.json --output /tmp/858-synthetic-report
python3 -I reproduce.py fixtures/synthetic.json /tmp/858-synthetic-report
```

Both input tools also accept a directory containing `raw-samples.json`.
A generated report or successful reproduction is not study completion. See
AC14a in the protocol for the remaining matched iOS/Android collections and
per-case dispositions. No AWS, benchmark timers or network access is used here.
