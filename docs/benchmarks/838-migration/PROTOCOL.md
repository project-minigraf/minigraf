# Manual migration sweep (#838)

**Measured on 2026-11-04.** See the [report](README.md) for 810 checked timing
samples, full native and bounded browser results, frozen fixtures, provenance,
and remaining exclusions. X03 remains MANUAL; this adds no nightly route or
production change. The work follows PHILOSOPHY.md's reliability, portability
and simplicity principles.

Core source measured:
`e07379323bca4c2c248c25c2e90707a8d8d9f134` on `v3`. The native fixture writer is
crates.io `minigraf =2.0.11` (format v7). Native reader and browser `BrowserDb`
are built from the same core checkout. The browser build uses the core's own
wasm-bindgen exports; **no minigraf-wasm repository or published npm binding is
measured**. The report commit pins the harness; evidence/run.json records its source hashes.

## Fixture ladder and independent expectations

The [generator](../../../scripts/migration-sweep/native/src/main.rs) creates
these nine fixtures, with three value shapes at each rung:

| Entities N | Facts (2N) | Payload attributes A | Total distinct attributes | Transactions | Unique long values, repeated / unique |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1,000 | 2,000 | 16 | 17 | 10 | 16 / 1,000 |
| 10,000 | 20,000 | 256 | 257 | 100 | 16 / 10,000 |
| 100,000 | 200,000 | 4,096 | 4,097 | 1,000 | 16 / 100,000 |

For each integer `i` in `[0,N)`, the fixture contains exactly:

```text
[:sweep/e{i} :sweep/id i]
[:sweep/e{i} :sweep/a{i % A} payload(i)]
```

Shapes: `integer` uses integer `i`; `repeated256` uses the zero-padded 16-digit
decimal `i % 16` followed by 240 ASCII `x` characters; `unique256` uses the
zero-padded 16-digit decimal `i` followed by the same suffix. The two long shapes
therefore have exactly 256 UTF-8 bytes per payload. Both have N entity dictionary
entries; attribute counts above are distinct attribute strings, not total DICT
entries or allocated dictionary pages. Long-value counts describe deduplication
inputs, not an assertion about the engine's internal allocation policy.

Each transaction inserts 100 entities (200 facts); there is one final checkpoint,
no pending WAL, no retractions, and no ambiguous same-transaction multivalue
facts (#371). The smaller `--smoke` ladder uses 100 entities, 16 payload attributes,
200 facts and one transaction per shape. It is a correctness prerequisite, not
an additional scale measurement.

The validator scans every fact, builds an entity-to-id map, and checks the recipe
independently. Native and browser validators check every id, attribute and payload,
reject duplicate/missing rows and orphan facts, and check
the old cursor's complete answer. Native also checks transaction counts and v8
integrity. Expected answers come from this recipe, never from recording engine
output. Native `verify().facts` must be 2N after migration.

**Freeze once, reuse everywhere.** Generation refuses an existing output directory.
It records file bytes and SHA-256 in `manifest.json`, and asserts format v7 and
absence of a WAL. The bytes contain real transaction timestamps, so regeneration
is not byte-identical. Retain the original compressed fixtures plus their manifest
as an evidence artifact and commit the manifest alongside the report. The
[archive](evidence/v7-fixtures.tar.xz) and [manifest](evidence/manifest.json) freeze
the measured corpus; their hashes are in the report and evidence/checksums.sha256.
Both runners reject fixture hashes that differ from the manifest.

## Timing boundaries and supported lifetimes

| Operation | Timed | Outside timing / correctness |
| --- | --- | --- |
| Native `read_only_v7` | `OpenOptions::open(read_only=true)` | Copy + fsync, all recipe checks, second read-only open, byte-for-byte hash check; file stays v7. **Not migration.** |
| Native `migrate_v7` | Writable `OpenOptions::open` on a v7 copy | Copy + fsync, check all facts/tx count, v8 header, `verify`, close |
| Native `reopen_v8` | Open migrated file after closing its first handle | All recipe checks, close; warm OS cache |
| Browser `migrate_v7` | Await `BrowserDb.open` on a v7-seeded IndexedDB store | WASM initialization, fetch + checksum, seed transaction completion; check persisted version 8 and all rows |
| Browser `reopen_v8` | Await `BrowserDb.open` after freeing the first handle | All rows checked again; same page/context, **not cold startup** |
| `churn_unpinned` | Sum of eight one-fact commits; native includes explicit checkpoint, browser includes IndexedDB flush | Fresh migrated fixture, statement construction, full data checks, final release/reclaim commit and reopen |
| `churn_pinned` | Same eight commits with an original cursor and one short-lived cursor per commit | Cursor creation/drain/drop; check original N ids and each short cursor's N+j ids, then release all pins, write/checkpoint a ninth id and verify reopen |

Churn values are `-1` through `-8` on fresh entities' `:sweep/id`; the untimed
post-release commit adds `-9`. Final live facts: `2N+9`; final id rows: `N+9`;
native final transaction count: initial + 9. Native `Cursor::close` consumes the
cursor; dropping releases its pin. Browser `close()` drops the underlying Rust
cursor, then `free()` releases its JS wrapper. Snapshot correctness is observed
through full cursor drains after writes. Public APIs do not expose the pin count
or page quarantine; this harness does **not** claim to measure their exact bytes
or prove reclamation bounds. Existing core pin/page-reuse tests remain relevant.

Migration completes during open, before a caller can obtain a cursor on that
handle. A pre-migration cursor held across the same open is unsupported. Native
independent read-only/writable handles have file-lock constraints; this harness
does not bypass them. Browser cross-tab independent writers, concurrent opens of
one v7 store, and cross-context snapshot sharing are not asserted contracts.
Churn is sequential persistence after migration, not concurrent migration.

Browser runs use Playwright's real headless Chromium, Firefox and WebKit engines,
with a fresh ephemeral context per fixture/sample. Set `TMPDIR` to a directory
on the intended disk filesystem before launching Playwright; the host default
`/tmp` may be tmpfs. Record that filesystem. The browser matrix is explicitly
capped at 20,000 facts with `SWEEP_MAX_FACTS=20000`. Each context contains three
fresh databases (migration and two churn controls), then is destroyed. IndexedDB
is seeded with 4096-byte `Uint8Array` pages keyed by page number, in an object
store named after the database, matching `src/browser/indexeddb.rs`. No IndexedDB
mock or Node persistence substitute is used. Browser process startup, page load,
WASM compile/init, seed/copy and validation are outside timing. Within each engine
the process is reused; read-only browser v7 open is not exposed by `BrowserDb`.

## Execute only on idle hardware

Keep machine identity, addresses and private hardware inventory out of published
evidence. Check idleness before builds and again before each measurement session;
keep private load/CPU-use observations before/after each suite. Stop and discard
a contaminated run. Run native and browser timings sequentially, with no builds
or tests competing. Use the established CPU/memory affinity for this host and
record the chosen affinity, OS/architecture, filesystem type, toolchain, build
profiles, browser/runtime versions and cache conditions in the public run metadata.
Do not change root-owned settings as part of this sweep.

Use Node 22.23.4 for this run and Playwright 1.64.1, pinned in the tool's npm
manifest/lock. Playwright before 1.55.1 is affected by
[CVE-2025-59288](https://github.com/advisories/GHSA-7mvr-c777-76hp).
Node 22 before 22.23.4 is affected by the recent
[Brotli CVE-2026-111497](https://nodejs.org/en/blog/vulnerability/october-2026-security-releases).
Keep these benchmark-tool dependencies separate from the library dependencies.

From an isolated checkout of the final harness revision on the idle host:

```sh
# Paths below are relative to the repository root. Use a disposable checkout.
export CARGO_TARGET_DIR="$PWD/target/migration-sweep"
export TMPDIR="$PWD/target/migration-sweep-tmp"
export SWEEP_MAX_FACTS=20000
mkdir -p "$TMPDIR"
findmnt -n -o FSTYPE -T "$TMPDIR" > /dev/null # record this filesystem in run metadata
mkdir -p scripts/migration-sweep/results
git rev-parse HEAD > scripts/migration-sweep/results/core-revision.txt
git diff --binary > scripts/migration-sweep/results/source.patch
rustc -Vv > scripts/migration-sweep/results/rustc.txt
cargo -V > scripts/migration-sweep/results/cargo.txt
node --version > scripts/migration-sweep/results/node.txt
wasm-pack --version > scripts/migration-sweep/results/wasm-pack.txt

# Baseline correctness and evidence checker; retain logs and exit statuses.
cargo test --test golden_corpus_test
cargo test --test migration_matrix_test
python3 -m unittest discover -s scripts/migration-sweep -p 'test_*.py'

# Reuse the measured resolution in this disposable checkout.
cp docs/benchmarks/838-migration/evidence/native-dependencies.lock scripts/migration-sweep/native/Cargo.lock
cp docs/benchmarks/838-migration/evidence/core-dependencies.lock Cargo.lock
cargo build --locked --release --manifest-path scripts/migration-sweep/native/Cargo.toml
cp scripts/migration-sweep/native/Cargo.lock scripts/migration-sweep/results/native-dependencies.lock
# The standalone native profile is opt-level 3, LTO, one codegen unit.
# Browser release follows the core's opt-level z profile; report separately.
wasm-pack build --release --target web --out-dir scripts/migration-sweep/pkg -- --features browser
cp Cargo.lock scripts/migration-sweep/results/core-dependencies.lock
npm ci --prefix scripts/migration-sweep
npm audit --prefix scripts/migration-sweep --json > scripts/migration-sweep/results/npm-audit.json
scripts/migration-sweep/node_modules/.bin/playwright install chromium firefox webkit
scripts/migration-sweep/node_modules/.bin/playwright --version > scripts/migration-sweep/results/playwright.txt
sha256sum target/migration-sweep/release/migration-sweep scripts/migration-sweep/pkg/minigraf* > scripts/migration-sweep/results/build-hashes.txt

# Smoke both paths before the full fixture generation. No clock reads in --test.
target/migration-sweep/release/migration-sweep generate scripts/migration-sweep/fixtures --smoke
target/migration-sweep/release/migration-sweep run scripts/migration-sweep/fixtures 1 --test > scripts/migration-sweep/results/native-smoke.jsonl
python3 scripts/migration-sweep/summarize.py scripts/migration-sweep/fixtures/manifest.json scripts/migration-sweep/results/native-smoke.jsonl --samples 1 --test
for engine in chromium firefox webkit; do
  node scripts/migration-sweep/browser-runner.mjs "$engine" "scripts/migration-sweep/results/$engine-smoke.json" 1 --test
  python3 scripts/migration-sweep/summarize.py scripts/migration-sweep/fixtures/manifest.json "scripts/migration-sweep/results/$engine-smoke.json" --samples 1 --browser --max-facts 20000 --test
done
mv scripts/migration-sweep/fixtures scripts/migration-sweep/results/smoke-fixtures

# Replay the exact measured nine-fixture ladder (do not regenerate timestamps).
mkdir scripts/migration-sweep/fixtures
tar -xJf docs/benchmarks/838-migration/evidence/v7-fixtures.tar.xz --strip-components=1 -C scripts/migration-sweep/fixtures
# For an independent new corpus instead, use the generator and freeze its bytes:
# target/migration-sweep/release/migration-sweep generate scripts/migration-sweep/fixtures
# tar -cJf scripts/migration-sweep/results/v7-fixtures.tar.xz -C scripts/migration-sweep fixtures

# Run a full untimed pass first; then recheck idle state before collecting data.
target/migration-sweep/release/migration-sweep run scripts/migration-sweep/fixtures 1 --test > scripts/migration-sweep/results/native-check.jsonl
python3 scripts/migration-sweep/summarize.py scripts/migration-sweep/fixtures/manifest.json scripts/migration-sweep/results/native-check.jsonl --samples 1 --test
for engine in chromium firefox webkit; do
  node scripts/migration-sweep/browser-runner.mjs "$engine" "scripts/migration-sweep/results/$engine-check.json" 1 --test
  python3 scripts/migration-sweep/summarize.py scripts/migration-sweep/fixtures/manifest.json "scripts/migration-sweep/results/$engine-check.json" --samples 1 --browser --max-facts 20000 --test
done

# Prefix timing commands with the established host affinity. Five raw samples.
target/migration-sweep/release/migration-sweep run scripts/migration-sweep/fixtures 5 > scripts/migration-sweep/results/native.jsonl
python3 scripts/migration-sweep/summarize.py scripts/migration-sweep/fixtures/manifest.json scripts/migration-sweep/results/native.jsonl --samples 5 > scripts/migration-sweep/results/native-summary.json
for engine in chromium firefox webkit; do
  node scripts/migration-sweep/browser-runner.mjs "$engine" "scripts/migration-sweep/results/$engine.json" 5
  python3 scripts/migration-sweep/summarize.py scripts/migration-sweep/fixtures/manifest.json "scripts/migration-sweep/results/$engine.json" --samples 5 --browser --max-facts 20000 > "scripts/migration-sweep/results/$engine-summary.json"
done
```

Run the command blocks in a shell with `set -euo pipefail`; any failure stops
the run. Retain stdout/stderr and exit status for each stage. Browser OS libraries
must already be installed; if a browser cannot launch, record the actual missing
dependency and mark that engine unmeasured. Do not report a substitute as that
engine's result. `wasm-pack` and Playwright browser downloads are setup, not timed.
Retain exact resolved locks after **both** builds, package versions, generated JS
and WASM hashes, and all raw samples. Browser output records its engine version,
user agent, Node version and completion timestamp. Native output is JSONL, one
row per successful checked operation; the summarizer rejects missing, duplicate,
unverified or substituted samples. Never invoke it with Python `-O` (assertions
are evidence gates). Failures are not zero-time samples.

## Publication and remaining exclusions

Publish medians and min/max of five raw samples per fixture/operation/engine,
with N→10N latency ratios and pinned/unpinned churn ratios. State that scale and
attribute cardinality grow together; these ratios do not isolate either factor.
The churn metric is the **sum of eight commits**, not a per-commit latency or a
sustained-throughput claim. Do not average different browser engines or compare
native and browser timings as equal persistence/durability contracts. Use at
least a second complete idle run if results are noisy; keep both sessions.

Expected measured rows: native 9×5 operations×5 samples = **225**; each browser
6×4×5 = **120**. The measured run retained a second complete native session
(another 225 samples) to expose large-case variance: **810 total**. These are manual samples, not Criterion registrations or Bencher
uploads. The frozen small C07 golden corpus and its existing timings are unchanged.

The [report](README.md) attaches the frozen fixture artifact/manifest, successful
correctness logs, source/build/runtime provenance, raw samples, summaries and
an explicit per-engine disposition. X03 links to these results and stays MANUAL.

Still excluded by this bounded sweep: more than 200k facts, independent cardinality
sweeps, multiple checkpoints/history/temporal windows, long values over 256 bytes,
v7 pending/stale WALs, crash/power-loss migration, real Safari/mobile devices,
persistent-profile/cold-start browser migration, concurrent tabs, OS cold-cache
opens, direct pin/quarantine accounting, and sustained many-pin/RSS scaling. The 200k-fact browser rung is excluded
from the measured matrix by the user’s scope decision after an exploratory
Chromium open took minutes; no numeric latency is inferred from that untimed run.
Coordinate browser lifecycle breadth with X04 and sustained pin scaling with
X06; this eight-commit/two-pin case does not complete either follow-up.

After copying verified evidence back, remove only this run's disposable checkout,
Cargo targets, fixtures, browser package/download scratch and profiles. Keep no
remote scratch unless it will be reused for the next run, and document why.
