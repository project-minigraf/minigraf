# minigraf (Python)

[![PyPI](https://img.shields.io/pypi/v/minigraf.svg)](https://pypi.org/project/minigraf)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/project-minigraf/minigraf#license)

> Embedded bi-temporal graph database for Python — Datalog queries, time travel, pre-built wheels

Minigraf for Python: pre-built wheels for Linux x86_64/aarch64, macOS universal2, and Windows x86_64. No Rust toolchain required.

## Install

```sh
pip install minigraf
```

## Quick start

```python
from minigraf import MiniGrafDb

# File-backed database
db = MiniGrafDb.open("myapp.graph")

# In-memory database (ephemeral / testing)
mem = MiniGrafDb.open_in_memory()

# Transact facts
r = db.execute('(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])')
# '{"transacted": 1}'

# Query with Datalog
q = db.execute('(query [:find ?name ?age :where [?e :person/name ?name] [?e :person/age ?age]])')
# '{"variables": ["?name", "?age"], "results": [["Alice", 30]]}'

# Time travel — state as of transaction 1
snap = db.execute('(query [:find ?age :as-of 1 :where [:alice :person/age ?age]])')

# Flush dirty pages to disk
db.checkpoint()
```

## Response shapes

| Command | JSON |
|---|---|
| `transact` | `{"transacted": <tx_count>}` |
| `retract` | `{"retracted": <tx_count>}` |
| `query` | `{"variables": [...], "results": [[...]]}` |
| `rule` | `{"ok": true}` |

## Links

- [Full Python integration guide](https://github.com/project-minigraf/minigraf/wiki/Use-Cases#python)
- [Repository](https://github.com/project-minigraf/minigraf)
- [Datalog Reference](https://github.com/project-minigraf/minigraf/wiki/Datalog-Reference)

## License

MIT OR Apache-2.0
