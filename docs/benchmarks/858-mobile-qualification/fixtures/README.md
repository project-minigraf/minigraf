# Synthetic fixture — NOT real data

These files are only a deterministic decision-tool test/example. No app,
simulator, emulator, physical device or provider was run. Revision strings,
artifact hashes and runtime descriptions are fabricated. The synthetic iOS
physical/candidate pair covers all 14 case suffixes and passes every gate.
Passing these invented numbers cannot qualify a lane or complete #858.

`synthetic.json` retains ten identical synthetic sample means per run; the six
independent-run placeholders vary, so each set has nonzero sample variance.
`synthetic-report.json` and `synthetic-report.md` are the expected outputs.
Negative controls mutate independent in-memory copies in `test_decide.py`;
they never modify or replace real evidence.
