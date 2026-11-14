// One worker = one fresh wasm instance. Seeding workers are always destroyed.
import init, { BrowserDb } from '/pkg/minigraf_wasm.js'

const wasm = await init()
self.onmessage = async ({ data: { operation, name, mode, fixture } }) => {
  let db
  try {
    if (operation === 'seed') {
      const bytes = new Uint8Array(await (await fetch(`/fixtures/${fixture}`)).arrayBuffer())
      if (mode === 'opfs') {
        const root = await navigator.storage.getDirectory()
        const file = await root.getFileHandle(`${name}.graph`, { create: true })
        const handle = await file.createSyncAccessHandle()
        try {
          if (handle.write(bytes, { at: 0 }) !== bytes.length) throw new Error('short fixture write')
          handle.truncate(bytes.length)
          handle.flush()
        } finally { handle.close() }
      } else {
        db = await BrowserDb.open(name)
        await db.importGraph(bytes)
      }
      self.postMessage({ ok: true })
    } else {
      let reads = 0, bytesRead = 0, indexedDbPages = 0
      const originalGetAll = IDBObjectStore.prototype.getAll
      IDBObjectStore.prototype.getAll = function (...args) {
        const request = originalGetAll.apply(this, args)
        request.addEventListener('success', () => { indexedDbPages += request.result.length })
        return request
      }
      const originalRead = FileSystemSyncAccessHandle.prototype.read
      FileSystemSyncAccessHandle.prototype.read = function (buffer, options) {
        reads++
        const transferred = originalRead.call(this, buffer, options)
        bytesRead += transferred
        return transferred
      }
      const start = performance.now()
      db = mode === 'opfs' ? await BrowserDb.openOpfs(name) : await BrowserDb.open(name)
      const openMs = performance.now() - start
      const openReads = reads
      const answer = JSON.parse(await db.execute('(query [:find ?a :where [:probe :answer ?a]])')).results
      if (JSON.stringify(answer) !== '[[42]]') throw new Error('point query answer changed')
      // memory.buffer.byteLength is the allocated linear-memory high-water
      // mark. It is not live memory and does not include browser-side IDB state.
      self.postMessage({ mode, fixture, openMs, openReads, readCalls: reads,
        pagesRead: mode === 'opfs' ? bytesRead / 4096 : indexedDbPages, memoryHighWaterBytes: wasm.memory.buffer.byteLength })
    }
  } catch (error) {
    self.postMessage({ error: String(error) })
  } finally { db?.close() }
}
self.postMessage({ ready: true })
