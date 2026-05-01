/* tslint:disable */
/* eslint-disable */

/**
 * Browser-only Minigraf database handle backed by IndexedDB.
 *
 * All public methods return `Promise`s. Use `await` in JavaScript.
 *
 * **Not compatible with Node.js.** Use `@minigraf/node` for server-side use.
 */
export class BrowserDb {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Flush all dirty pages to IndexedDB.
     *
     * Write-through means individual `execute()` calls already flush dirty pages,
     * so `checkpoint()` is only needed after `import_graph()` or explicit bulk ops.
     * No-op for in-memory databases.
     */
    checkpoint(): Promise<void>;
    /**
     * Execute a Datalog command string and return a JSON-encoded result.
     *
     * Returns a `Promise<string>` in JavaScript. The JSON shape is:
     * - Query: `{"variables": [...], "results": [[...], ...]}`
     * - Transact: `{"transacted": <tx_id>}`
     * - Retract: `{"retracted": <tx_id>}`
     * - Rule: `{"ok": true}`
     */
    execute(datalog: string): Promise<string>;
    /**
     * Serialise the current database to a portable `.graph` blob.
     *
     * The blob is byte-for-bit compatible with native `.graph` files opened by
     * `Minigraf::open()`. Pages are always in ascending `page_id` order.
     *
     * Call `checkpoint()` on native before importing a file here to ensure
     * no WAL entries are missing from the main file.
     */
    exportGraph(): Uint8Array;
    /**
     * Replace the current database with a `.graph` blob.
     *
     * The blob must be a checkpointed native `.graph` file (no pending WAL sidecar).
     * All existing data is overwritten. After import, the new data is immediately
     * queryable and all dirty pages are flushed to IndexedDB.
     */
    importGraph(data: Uint8Array): Promise<void>;
    /**
     * Open or create a database backed by IndexedDB.
     *
     * `db_name` is used as both the IndexedDB database name and object store name.
     * Called as `await BrowserDb.open("mydb")` — NOT `new BrowserDb()`.
     */
    static open(db_name: string): Promise<BrowserDb>;
    /**
     * Open an in-memory database (no IndexedDB — for testing only).
     *
     * Data is lost when the page is closed. Use `BrowserDb.open()` for persistence.
     */
    static openInMemory(): BrowserDb;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_browserdb_free: (a: number, b: number) => void;
    readonly browserdb_checkpoint: (a: number) => number;
    readonly browserdb_execute: (a: number, b: number, c: number) => number;
    readonly browserdb_exportGraph: (a: number, b: number) => void;
    readonly browserdb_importGraph: (a: number, b: number) => number;
    readonly browserdb_open: (a: number, b: number) => number;
    readonly browserdb_openInMemory: (a: number) => void;
    readonly __wasm_bindgen_func_elem_2769: (a: number, b: number, c: number, d: number) => void;
    readonly __wasm_bindgen_func_elem_2781: (a: number, b: number, c: number, d: number) => void;
    readonly __wasm_bindgen_func_elem_658: (a: number, b: number, c: number) => void;
    readonly __wbindgen_export: (a: number, b: number) => number;
    readonly __wbindgen_export2: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_export3: (a: number) => void;
    readonly __wbindgen_export4: (a: number, b: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
