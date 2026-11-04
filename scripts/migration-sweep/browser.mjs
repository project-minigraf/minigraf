// Runs inside a real browser. BrowserDb is the core's own wasm-bindgen facade.
import init, { BrowserDb } from './pkg/minigraf.js'

const QUERY = '(query [:find ?e ?a ?v :where [?e ?a ?v]])'
const IDS = '(query [:find ?i :where [?e :sweep/id ?i]])'
const require = (ok, message) => { if (!ok) throw new Error(message) }

function payload(f, i) {
  if (f.shape === 'integer') return i
  require(['repeated256', 'unique256'].includes(f.shape), 'unknown shape')
  return String(f.shape === 'repeated256' ? i % 16 : i).padStart(16, '0') + 'x'.repeat(240)
}

async function check(db, f, extra = 0) {
  const rows = JSON.parse(await db.execute(QUERY)).results
  require(rows.length === f.facts + extra, 'incorrect fact count')
  const ids = new Map()
  for (const row of rows) {
    const [e, a, v] = row
    require(row.length === 3 && typeof e === 'string', 'entity required')
    if (a === ':sweep/id') {
      require(!ids.has(e), 'duplicate entity id')
      ids.set(e, v)
    }
  }
  require(ids.size === f.entities + extra, 'incorrect entity count')
  const seen = new Set()
  for (const [e, a, v] of rows) {
    require(ids.has(e), 'fact without a recipe id')
    const i = ids.get(e)
    require(Number.isInteger(i) && i >= -extra && i < f.entities, 'id outside recipe')
    const key = JSON.stringify([i, a])
    require(!seen.has(key), 'duplicate row')
    seen.add(key)
    if (a === ':sweep/id') require(v === i, 'incorrect id')
    else {
      require(i >= 0 && a === `:sweep/a${i % f.attributes}`, 'incorrect attribute')
      require(v === payload(f, i), 'incorrect payload')
    }
  }
}

function checkCursor(cursor, n, extra) {
  const seen = new Set()
  for (;;) {
    const raw = cursor.nextBatch(256)
    if (raw === undefined) break
    const rows = JSON.parse(raw)
    require(rows.length > 0 && rows.length <= 256, 'invalid cursor batch')
    for (const row of rows) {
      const [i] = row
      require(row.length === 1 && Number.isInteger(i) && i >= -extra && i < n, 'snapshot value')
      require(!seen.has(i), 'duplicate snapshot value')
      seen.add(i)
    }
  }
  require(seen.size === n + extra, 'missing snapshot value')
  cursor.close()
  require(cursor.nextBatch(1) === undefined, 'closed cursor returned rows')
}

function idbOpen(name) {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(name, 1)
    request.onupgradeneeded = () => request.result.createObjectStore(name)
    request.onerror = () => reject(request.error)
    request.onsuccess = () => resolve(request.result)
  })
}

async function seed(name, bytes) {
  const db = await idbOpen(name)
  try {
    await new Promise((resolve, reject) => {
      const tx = db.transaction(name, 'readwrite')
      tx.oncomplete = resolve
      tx.onerror = tx.onabort = () => reject(tx.error ?? new Error('seed aborted'))
      const store = tx.objectStore(name)
      store.clear()
      for (let offset = 0; offset < bytes.length; offset += 4096) {
        store.put(bytes.slice(offset, offset + 4096), offset / 4096)
      }
    })
  } finally { db.close() }
}

async function version(name) {
  const db = await idbOpen(name)
  try {
    return await new Promise((resolve, reject) => {
      const request = db.transaction(name).objectStore(name).get(0)
      request.onerror = () => reject(request.error)
      request.onsuccess = () => {
        const bytes = request.result
        resolve(new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(4, true))
      }
    })
  } finally { db.close() }
}

async function fetchChecked(f) {
  const response = await fetch(`./fixtures/${f.file}`)
  require(response.ok, 'fixture fetch failed')
  const bytes = new Uint8Array(await response.arrayBuffer())
  require(bytes.length === f.bytes && bytes.length % 4096 === 0, 'invalid fixture length')
  const hash = [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))]
    .map(b => b.toString(16).padStart(2, '0')).join('')
  require(hash === f.sha256, 'fixture checksum mismatch')
  require(new DataView(bytes.buffer).getUint32(4, true) === 7, 'not v7')
  return bytes
}

export async function run({ fixture, sample, test }) {
  require(Number.isInteger(sample) && sample >= 0, 'invalid sample')
  await init()
  const response = await fetch('./fixtures/manifest.json')
  require(response.ok, 'manifest fetch failed')
  const fixtures = (await response.json()).filter(f => f.file === fixture)
  require(fixtures.length === 1, 'fixture must exist exactly once')
  const results = []
  const record = (f, operation, sample, ns) => results.push({
    fixture: f.file, sha256: f.sha256, operation, sample, ns, correctness: 'passed',
  })
  for (const f of fixtures) {
    const bytes = await fetchChecked(f)
      for (const mode of ['migration', 'churn_unpinned', 'churn_pinned']) {
        console.log(`checking ${f.file} sample=${sample} mode=${mode}`)
        const name = `sweep-${f.file}-${sample}-${mode}`
        await seed(name, bytes)
        console.log('seed complete')
        require(await version(name) === 7, 'seed is not v7')
        const start = !test && mode === 'migration' ? performance.now() : null
        let db = await BrowserDb.open(name)
        const elapsed = start === null ? null : (performance.now() - start) * 1e6
        console.log('open complete')
        try {
          require(await version(name) === 8, 'migration not persisted')
          await check(db, f)
          console.log('data check complete')
          if (mode === 'migration') {
            db.free(); db = null
            const start = test ? null : performance.now()
            db = await BrowserDb.open(name)
            const reopen = start === null ? null : (performance.now() - start) * 1e6
            await check(db, f)
            console.log('reopen check complete')
            record(f, 'migrate_v7', sample, elapsed)
            record(f, 'reopen_v8', sample, reopen)
          } else {
            const old = mode === 'churn_pinned' ? db.query(IDS) : null
            let ns = 0
            try {
              for (let j = 0; j < 8; j++) {
                const recent = old ? db.query(IDS) : null
                try {
                  const statement = `(transact [[:sweep/later${j} :sweep/id -${j + 1}]])`
                  const start = test ? null : performance.now()
                  await db.execute(statement) // includes IndexedDB completion
                  if (start !== null) ns += (performance.now() - start) * 1e6
                  if (recent) checkCursor(recent, f.entities, j)
                } finally { recent?.close(); recent?.free() }
              }
              if (old) checkCursor(old, f.entities, 0)
            } finally { old?.close(); old?.free() }
            await check(db, f, 8)
            // A subsequent write exercises reclamation after the old pin closes.
            await db.execute('(transact [[:sweep/later8 :sweep/id -9]])')
            await check(db, f, 9)
            db.free(); db = null
            db = await BrowserDb.open(name)
            await check(db, f, 9)
            record(f, mode, sample, test ? null : ns)
          }
        } finally { db?.free() }
        // BrowserDb currently retains IDB connections; the runner destroys the
        // entire fresh browser context after this run, including its storage.
      }
  }
  return { results, userAgent: navigator.userAgent, persistence: 'fresh ephemeral context; same page reopen',
    clock: 'performance.now in page; nanoseconds reported from milliseconds', test }
}
