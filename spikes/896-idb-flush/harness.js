// #896 spike: fault injection around IndexedDB for BrowserDb (v2.x).
//
// Loaded by harness.html?build=<main|v2.0.11|published>. Wraps
// IDBDatabase.prototype.transaction and IDBObjectStore.prototype.put BEFORE the
// WASM module initialises, records every readwrite transaction's puts and
// events, and exposes window.mg for run.mjs to drive through page.evaluate.
//
// Fault modes (armed for the next N readwrite transactions):
//   request-abort  after the first put succeeds, tx.abort() while later puts are
//                  still pending: those requests fire error events (AbortError)
//                  that bubble to the transaction, then the abort event.
//   abort-only     tx.abort() after the LAST put succeeds: no request errors, only
//                  the transaction's abort event (the shape of a commit-time
//                  failure such as QuotaExceededError).
//   sync-throw     the second put() call throws synchronously (SYNTHETIC: no
//                  realistic production trigger established); the first put is
//                  already queued on a still-active transaction.

const builds = {
  main: '/pkgs/main/minigraf.js',
  'v2.0.11': '/pkgs/v2.0.11/minigraf.js',
  published: '/pkgs/published/minigraf_wasm.js',
}
const build = new URL(location.href).searchParams.get('build')

const t0 = performance.now()
const trace = []
const log = (ev, data = {}) => trace.push({ t: +(performance.now() - t0).toFixed(2), ev, ...data })

// FNV-1a 32 of a page, enough to tell page versions apart in traces.
function fnv(bytes) {
  let h = 0x811c9dc5
  for (let i = 0; i < bytes.length; i++) {
    h ^= bytes[i]
    h = Math.imul(h, 0x01000193) >>> 0
  }
  return h.toString(16).padStart(8, '0')
}

const fault = { mode: null, remaining: 0 }
const meta = new WeakMap()
let txSeq = 0

const origTx = IDBDatabase.prototype.transaction
IDBDatabase.prototype.transaction = function (stores, mode, opts) {
  const tx = origTx.call(this, stores, mode, opts)
  if (mode !== 'readwrite') return tx
  const m = { id: ++txSeq, puts: [], fault: null, reqs: [], aborted: false }
  if (fault.remaining > 0) {
    fault.remaining--
    m.fault = fault.mode
  }
  meta.set(tx, m)
  log('tx-create', { tx: m.id, fault: m.fault })
  tx.addEventListener('complete', () => log('tx-complete', { tx: m.id }))
  tx.addEventListener('abort', () => log('tx-abort', { tx: m.id, error: tx.error?.name ?? null }))
  tx.addEventListener('error', (e) =>
    log('tx-error-event', { tx: m.id, target: e.target === tx ? 'tx' : 'request', error: e.target.error?.name ?? null }),
  )
  return tx
}

const origPut = IDBObjectStore.prototype.put
IDBObjectStore.prototype.put = function (value, key) {
  const tx = this.transaction
  const m = meta.get(tx)
  if (!m) return origPut.call(this, value, key)
  const n = m.puts.length + 1
  if (m.fault === 'sync-throw' && n === 2) {
    log('inject-sync-throw', { tx: m.id, key })
    throw new DOMException('synthetic #896 put failure', 'DataError')
  }
  const req = origPut.call(this, value, key)
  m.puts.push({ key, hash: fnv(value) })
  m.reqs.push(req)
  log('put', { tx: m.id, key, hash: fnv(value) })
  req.addEventListener('success', () => {
    if (m.aborted) return
    if (m.fault === 'request-abort' && n === 1) {
      m.aborted = true
      log('inject-abort', { tx: m.id, pending: m.reqs.length - 1 })
      tx.abort()
    } else if (m.fault === 'abort-only' && req === m.reqs[m.reqs.length - 1]) {
      m.aborted = true
      log('inject-abort', { tx: m.id, pending: 0 })
      tx.abort()
    }
  })
  return req
}

// Settles a promise or reports it still pending after `ms`.
async function settle(p, ms = 3000, id = null) {
  let timer
  const timeout = new Promise((r) => (timer = setTimeout(() => r({ state: 'pending' }), ms)))
  const out = await Promise.race([
    p.then(
      (value) => ({ state: 'resolved', value }),
      (e) => ({ state: 'rejected', reason: String(e?.message ?? e) }),
    ),
    timeout,
  ])
  clearTimeout(timer)
  log('settle', { id, state: out.state })
  return out
}

// Raw object-store contents, independent of BrowserDb.
function rawPages(name) {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(name, 1)
    req.onupgradeneeded = () => req.result.createObjectStore(name)
    req.onerror = () => reject(req.error)
    req.onsuccess = () => {
      const db = req.result
      const tx = db.transaction(name, 'readonly')
      const st = tx.objectStore(name)
      const k = st.getAllKeys()
      const v = st.getAll()
      tx.oncomplete = () => {
        const pages = {}
        k.result.forEach((key, i) => (pages[key] = v.result[i]))
        db.close()
        resolve(pages)
      }
      tx.onerror = () => reject(tx.error)
    }
  })
}

function header(bytes) {
  if (!bytes) return null
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const u64 = (o) => Number(dv.getBigUint64(o, true))
  return {
    magic: String.fromCharCode(...bytes.slice(0, 4)),
    version: dv.getUint32(4, true),
    page_count: u64(8),
    node_count: u64(16),
    last_checkpointed_tx_count: u64(24),
    roots: [u64(32), u64(40), u64(48), u64(56)],
    index_checksum: dv.getUint32(64, true).toString(16),
    fact_page_count: u64(72),
  }
}

// Page-by-page view of the raw store: header, missing and extra ids, hashes.
async function inspect(name) {
  const pages = await rawPages(name)
  const ids = Object.keys(pages).map(Number).sort((a, b) => a - b)
  const h = header(pages[0])
  const want = h ? h.page_count : 0
  const missing = []
  for (let i = 0; i < want; i++) if (!(i in pages)) missing.push(i)
  const extra = ids.filter((i) => i >= want)
  const hashes = Object.fromEntries(ids.map((i) => [i, fnv(pages[i])]))
  return { header: h, ids, missing, extra, hashes }
}

const handles = {}
let mod

window.mg = {
  build,
  handles,
  trace,
  async init() {
    mod = await import(builds[build])
    await mod.default()
    const pkg = await (await fetch(builds[build].replace(/[^/]+$/, 'package.json'))).json()
    return { name: pkg.name, version: pkg.version }
  },
  async open(h, name) {
    try {
      handles[h] = name === null ? mod.BrowserDb.openInMemory() : await mod.BrowserDb.open(name)
      return { ok: true }
    } catch (e) {
      return { ok: false, reason: String(e?.message ?? e) }
    }
  },
  arm(mode, count = 1) {
    fault.mode = mode
    fault.remaining = count
    log('arm', { mode, count })
  },
  disarm() {
    fault.mode = null
    fault.remaining = 0
    log('disarm')
  },
  // Starts an operation without awaiting it; `await mg.wait(id)` later.
  pending: {},
  start(id, h, op, arg) {
    log('start', { id, h, op, arg: typeof arg === 'string' ? arg : undefined })
    const db = handles[h]
    let p
    if (op === 'execute') p = db.execute(arg)
    else if (op === 'checkpoint') p = db.checkpoint()
    else if (op === 'import') p = db.importGraph(new Uint8Array(arg))
    else throw new Error(`unknown op ${op}`)
    // A handler now, so a rejection before wait() is not reported as an
    // unhandled-rejection page error (a harness artifact, not BrowserDb's).
    p.catch(() => {})
    this.pending[id] = p
  },
  wait(id, ms) {
    return settle(this.pending[id], ms, id)
  },
  runs: 0,
  async run(h, op, arg, ms) {
    const id = `_run${++this.runs}`
    this.start(id, h, op, arg)
    return this.wait(id, ms)
  },
  async names(h) {
    try {
      const r = JSON.parse(await handles[h].execute('(query [:find ?n :where [?e :item/name ?n]])'))
      return { ok: true, names: r.results.map((x) => x[0]).sort() }
    } catch (e) {
      return { ok: false, reason: String(e?.message ?? e) }
    }
  },
  exportHashes(h) {
    const blob = handles[h].exportGraph()
    const out = {}
    for (let i = 0; i * 4096 < blob.length; i++) out[i] = fnv(blob.subarray(i * 4096, (i + 1) * 4096))
    return out
  },
  exportBytes(h) {
    return Array.from(handles[h].exportGraph())
  },
  inspect,
}
window.__ready = true
