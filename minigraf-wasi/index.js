import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const DEFAULT_WASM_URL = new URL("./minigraf-wasi.wasm", import.meta.url);

export function getMinigrafWasiPath(options = {}) {
  return (
    options.wasmPath ??
    process.env.MINIGRAF_WASI_WASM_PATH ??
    fileURLToPath(DEFAULT_WASM_URL)
  );
}

export async function readMinigrafWasi(options = {}) {
  const buffer = await readFile(getMinigrafWasiPath(options));
  return buffer.buffer.slice(
    buffer.byteOffset,
    buffer.byteOffset + buffer.byteLength,
  );
}

export async function compileMinigrafWasi(options = {}) {
  return WebAssembly.compile(await readMinigrafWasi(options));
}

export async function instantiateMinigrafWasi(imports = {}, options = {}) {
  const module = await compileMinigrafWasi(options);
  return WebAssembly.instantiate(module, imports);
}

export async function instantiateMinigrafWasiWith(wasi, options = {}) {
  return instantiateMinigrafWasi(importObjectFromWasi(wasi), options);
}

export async function startMinigrafWasi(wasi, options = {}) {
  const instance = await instantiateMinigrafWasiWith(wasi, options);

  if (typeof wasi.start !== "function") {
    throw new TypeError("WASI object must expose start(instance)");
  }

  wasi.start(instance);
  return instance;
}

function importObjectFromWasi(wasi) {
  if (!wasi || typeof wasi !== "object") {
    throw new TypeError("WASI object is required");
  }

  if (typeof wasi.getImportObject === "function") {
    return wasi.getImportObject();
  }

  if (wasi.wasiImport) {
    return { wasi_snapshot_preview1: wasi.wasiImport };
  }

  if (wasi.wasi_snapshot_preview1) {
    return wasi;
  }

  throw new TypeError(
    "WASI object must expose getImportObject(), wasiImport, or wasi_snapshot_preview1",
  );
}
