# @minigraf/wasi

WASI WebAssembly package for Minigraf.

This package contains the `wasm32-wasip1` CLI binary as
`minigraf-wasi.wasm` plus a small Node.js ESM loader. It is intended for
server-side WASM runtimes such as Wasmtime, Wasmer, and Node's WASI support.

```js
import { WASI } from "node:wasi";
import { startMinigrafWasi } from "@minigraf/wasi";

const wasi = new WASI({
  version: "preview1",
  args: ["minigraf"],
  env: process.env,
  preopens: { "/tmp": "/tmp" },
});

await startMinigrafWasi(wasi);
```

Use `MINIGRAF_WASI_WASM_PATH` or the `wasmPath` option to point the loader at
an alternate `.wasm` file during tests or custom packaging.
