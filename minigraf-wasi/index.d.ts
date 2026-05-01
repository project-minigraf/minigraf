export interface MinigrafWasiOptions {
  wasmPath?: string;
}

export interface WasiWithImportObject {
  getImportObject(): WebAssembly.Imports;
  start?(instance: WebAssembly.Instance): void;
}

export interface WasiWithLegacyImport {
  wasiImport: Record<string, WebAssembly.ImportValue>;
  start?(instance: WebAssembly.Instance): void;
}

export type MinigrafWasiRuntime =
  | WasiWithImportObject
  | WasiWithLegacyImport
  | ({ wasi_snapshot_preview1: Record<string, WebAssembly.ImportValue> } & {
      start?(instance: WebAssembly.Instance): void;
    });

export function getMinigrafWasiPath(options?: MinigrafWasiOptions): string;

export function readMinigrafWasi(
  options?: MinigrafWasiOptions,
): Promise<ArrayBuffer>;

export function compileMinigrafWasi(
  options?: MinigrafWasiOptions,
): Promise<WebAssembly.Module>;

export function instantiateMinigrafWasi(
  imports?: WebAssembly.Imports,
  options?: MinigrafWasiOptions,
): Promise<WebAssembly.Instance>;

export function instantiateMinigrafWasiWith(
  wasi: MinigrafWasiRuntime,
  options?: MinigrafWasiOptions,
): Promise<WebAssembly.Instance>;

export function startMinigrafWasi(
  wasi: MinigrafWasiRuntime & {
    start(instance: WebAssembly.Instance): void;
  },
  options?: MinigrafWasiOptions,
): Promise<WebAssembly.Instance>;
