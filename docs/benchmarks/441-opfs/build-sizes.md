# Build sizes (#441)

Measured 2026-11-14. No dependency was added. Before is core `9fd321e75`;
after is `6fa1582f8` (subsequent documentation commits do not change code).

| Artifact | Toolchain / profile | Before bytes | After bytes | Difference |
|---|---|---:|---:|---:|
| `minigraf_wasm_bg.wasm` in the packaging shim | Rust 1.94.0, release, Cargo default opt-level 3, wasm-opt disabled in both builds | 1,754,394 | 1,822,855 | +68,461 (+3.90%) |
| Native CLI | Rust 1.94.0, `--profile dist` | 1,306,088 | 1,307,088 | +1,000 |
| Native CLI | Rust 1.99.1, `--profile dist` | Not measured | 1,269,568 | Not compared |

The 1.99.1 distribution build **passes** the unchanged 1,280,000-byte
(1,250 KiB) gate, with 10,432 bytes left. The 1.94.0 build **fails** that gate
on both revisions. The newer installed compiler was used to reproduce the
repository's documented size environment; the gate was not raised.

Both wasm builds use the same minimal `minigraf-wasm` re-export shim, a local
path dependency on each core revision, wasm-bindgen 0.2.130 / web-sys 0.3.107,
and `wasm-pack build --mode no-install --target web --no-opt --features browser`.
These are the raw wasm sizes, not gzipped transfer sizes or linear memory.
The companion repository's pinned lockfile still uses its existing dependency
versions; a release package built with those versions may differ.

The after package was also stamped as `@minigraf/browser@0.0.0-ci` and packed
successfully (665,211 bytes) with JS glue, TypeScript definitions, README and both licenses.
No registry publish or remote branch operation was performed.
