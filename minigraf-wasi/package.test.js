import { mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import assert from "node:assert/strict";

import {
  compileMinigrafWasi,
  getMinigrafWasiPath,
  instantiateMinigrafWasi,
  instantiateMinigrafWasiWith,
  readMinigrafWasi,
} from "./index.js";
import pkg from "./package.json" with { type: "json" };

const EMPTY_WASM_MODULE = Uint8Array.from([
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
]);

async function writeFixtureWasm() {
  const dir = join(tmpdir(), `minigraf-wasi-${process.pid}`);
  await mkdir(dir, { recursive: true });
  const path = join(dir, "fixture.wasm");
  await writeFile(path, EMPTY_WASM_MODULE);
  return path;
}

test("package metadata describes the public WASI npm package", () => {
  assert.equal(pkg.name, "@minigraf/wasi");
  assert.equal(pkg.type, "module");
  assert.equal(pkg.main, "index.js");
  assert.equal(pkg.types, "index.d.ts");
  assert.ok(pkg.files.includes("minigraf-wasi.wasm"));
});

test("loader reads and compiles the configured WASI wasm path", async () => {
  const path = await writeFixtureWasm();

  assert.equal(getMinigrafWasiPath({ wasmPath: path }), path);

  const bytes = await readMinigrafWasi({ wasmPath: path });
  assert.deepEqual(new Uint8Array(bytes), EMPTY_WASM_MODULE);

  const module = await compileMinigrafWasi({ wasmPath: path });
  assert.ok(module instanceof WebAssembly.Module);
});

test("loader instantiates the configured WASI wasm module", async () => {
  const path = await writeFixtureWasm();

  const instance = await instantiateMinigrafWasi({}, { wasmPath: path });

  assert.ok(instance instanceof WebAssembly.Instance);
});

test("loader accepts a WASI object exposing getImportObject", async () => {
  const path = await writeFixtureWasm();
  const wasi = {
    getImportObject() {
      return {};
    },
  };

  const instance = await instantiateMinigrafWasiWith(wasi, { wasmPath: path });

  assert.ok(instance instanceof WebAssembly.Instance);
});
