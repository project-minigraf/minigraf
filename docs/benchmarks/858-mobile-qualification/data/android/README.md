# android decision input

`raw-samples.json` is the decision input read by `decide.py` and `reproduce.py`
(SCHEMA.md). `provenance.json` records the source files, hashes and pins used to
assemble it, with paths relative to the evidence root. The archive was built by
`build_archive.py` (see `collection-tooling.tgz`): retained text artifacts are
redacted for device identifiers and every output is scanned before it is
published. `device-farm-file-manifest.tsv` lists every collected file with its
original size and SHA-256 and whether it is retained.
