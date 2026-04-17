# Minigraf Browser Demo

Demonstrates `@minigraf/core` running in a plain browser page with no bundler.

## Build

From the repo root:

```bash
wasm-pack build --target web --features browser
```

This produces `pkg/` containing `minigraf.js`, `minigraf_bg.wasm`, and
`minigraf.d.ts`.

## Serve

```bash
# From the repo root (not the examples/browser/ directory):
python3 -m http.server 8080
```

Open `http://localhost:8080/examples/browser/` in Chrome or Firefox.

## What it does

- Opens an IndexedDB-backed database named `"minigraf-demo"`.
- Transacts facts about Alice and Bob.
- Queries Alice's friends with Datalog.
- Exports the `.graph` blob and imports it into a fresh in-memory database.
- Logs all results to the browser console (open with F12).

## Notes

- Data persists across page reloads (stored in IndexedDB).
- The `pkg/` directory is gitignored — rebuild after pulling changes.
- This package (`@minigraf/core`) is **browser-only**. For Node.js, use
  `@minigraf/node` (Phase 8.3).
