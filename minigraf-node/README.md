# minigraf (Node.js)

[![npm](https://img.shields.io/npm/v/minigraf.svg)](https://www.npmjs.com/package/minigraf)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/project-minigraf/minigraf#license)

> Embedded bi-temporal graph database for Node.js — Datalog queries, time travel, native addon

Minigraf for Node.js: a native addon (no WASM, full file I/O) with pre-built binaries for Linux x86_64/aarch64, macOS universal2, and Windows x86_64. No build step required.

## Install

```sh
npm install minigraf
```

## Quick start

```typescript
import { MiniGrafDb } from 'minigraf';

// File-backed database
const db = new MiniGrafDb('myapp.graph');

// In-memory database (ephemeral / testing)
const mem = MiniGrafDb.inMemory();

// Transact facts
const r = JSON.parse(db.execute(
  '(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])'
));
// { "transacted": 1 }

// Query with Datalog
const q = JSON.parse(db.execute(
  '(query [:find ?name ?age :where [?e :person/name ?name] [?e :person/age ?age]])'
));
// { "variables": ["?name", "?age"], "results": [["Alice", 30]] }

// Time travel — state as of transaction 1
const snap = JSON.parse(db.execute(
  '(query [:find ?age :as-of 1 :where [:alice :person/age ?age]])'
));

// Flush dirty pages to disk
db.checkpoint();
```

## Response shapes

| Command | JSON |
|---|---|
| `transact` | `{"transacted": <tx_count>}` |
| `retract` | `{"retracted": <tx_count>}` |
| `query` | `{"variables": [...], "results": [[...]]}` |
| `rule` | `{"ok": true}` |

## Links

- [Full Node.js integration guide](https://github.com/project-minigraf/minigraf/wiki/Use-Cases#nodejs--typescript)
- [Repository](https://github.com/project-minigraf/minigraf)
- [Datalog Reference](https://github.com/project-minigraf/minigraf/wiki/Datalog-Reference)

## License

MIT OR Apache-2.0
