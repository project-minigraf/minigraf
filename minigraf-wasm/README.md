# @minigraf/browser

[![npm](https://img.shields.io/npm/v/@minigraf/browser.svg)](https://www.npmjs.com/package/@minigraf/browser)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/project-minigraf/minigraf#license)

> Embedded bi-temporal graph database for the browser — Datalog queries, time travel, IndexedDB persistence

Minigraf in the browser: zero configuration, persistent graph storage backed by IndexedDB, Datalog queries with recursive rules and time travel. One API, no server required.

## Install

```sh
npm install @minigraf/browser
```

## Quick start

```javascript
import init, { BrowserDb } from '@minigraf/browser';
await init();

// Persistent database (survives page reloads — backed by IndexedDB)
const db = await BrowserDb.open('my-graph');

// In-memory database (ephemeral / testing)
const mem = BrowserDb.openInMemory();

// Transact facts
const r = JSON.parse(await db.execute(
  '(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])'
));
// { "transacted": 1 }

// Query with Datalog
const q = JSON.parse(await db.execute(
  '(query [:find ?name ?age :where [?e :person/name ?name] [?e :person/age ?age]])'
));
// { "variables": ["?name", "?age"], "results": [["Alice", 30]] }

// Time travel — state as of transaction 1
const snap = JSON.parse(await db.execute(
  '(query [:find ?age :as-of 1 :where [:alice :person/age ?age]])'
));
```

## Response shapes

| Command | JSON |
|---|---|
| `transact` | `{"transacted": <tx_count>}` |
| `retract` | `{"retracted": <tx_count>}` |
| `query` | `{"variables": [...], "results": [[...]]}` |
| `rule` | `{"ok": true}` |

## Export / import

The `.graph` binary format is byte-identical between browser and native builds. Export a snapshot or load a native-generated file:

```javascript
// Export (Uint8Array — byte-identical to a native .graph file)
const bytes = db.exportGraph();

// Import a native .graph file (must be checkpointed — no pending WAL)
const file = document.querySelector('input[type=file]').files[0];
await db.importGraph(new Uint8Array(await file.arrayBuffer()));
```

## Constraints

- Requires a browser environment with IndexedDB. Not compatible with Node.js — use the [`minigraf` npm package](https://www.npmjs.com/package/minigraf) for Node.js instead.
- Single-threaded. Runs on the main thread or in a Web Worker; no shared state across workers.

## wasm-pack clobbering note (for maintainers)

`wasm-pack build` may overwrite this file. If it does, `wasm-release.yml` must copy the canonical
README from a stable source (e.g. a root-level `minigraf-wasm.README.md`) into the build output
directory before `npm publish`. Verify this on first release after any `wasm-pack` version upgrade.

## Links

- [Full browser integration guide](https://github.com/project-minigraf/minigraf/wiki/Use-Cases#wasm--browser)
- [Repository](https://github.com/project-minigraf/minigraf)
- [Datalog Reference](https://github.com/project-minigraf/minigraf/wiki/Datalog-Reference)

## License

MIT OR Apache-2.0
