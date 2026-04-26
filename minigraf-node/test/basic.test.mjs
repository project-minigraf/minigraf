import { test } from 'node:test'
import assert from 'node:assert/strict'
import { MiniGrafDb } from '../index.js'

test('open in-memory database', () => {
  const db = MiniGrafDb.inMemory()
  assert.ok(db, 'expected non-null db')
})

test('transact and query', () => {
  const db = MiniGrafDb.inMemory()
  const txJson = db.execute('(transact [[:alice :name "Alice"]])')
  const tx = JSON.parse(txJson)
  assert.ok('transacted' in tx, 'expected transacted key')

  const queryJson = db.execute('(query [:find ?n :where [?e :name ?n]])')
  const qr = JSON.parse(queryJson)
  assert.equal(qr.results[0][0], 'Alice')
})

test('invalid datalog throws', () => {
  const db = MiniGrafDb.inMemory()
  assert.throws(() => db.execute('not valid datalog !!!'))
})

test('file-backed roundtrip', async (t) => {
  const os = await import('node:os')
  const path = await import('node:path')
  const fs = await import('node:fs')

  const tmpDir = os.default.tmpdir()
  const dbPath = path.default.join(tmpDir, `minigraf_node_test_${Date.now()}.graph`)
  const walPath = dbPath + '.wal'

  t.after(() => {
    try { fs.default.unlinkSync(dbPath) } catch {}
    try { fs.default.unlinkSync(walPath) } catch {}
  })

  {
    const db = new MiniGrafDb(dbPath)
    db.execute('(transact [[:bob :name "Bob"]])')
    db.checkpoint()
  }

  const db2 = new MiniGrafDb(dbPath)
  const qr = JSON.parse(db2.execute('(query [:find ?n :where [?e :name ?n]])'))
  assert.equal(qr.results[0][0], 'Bob')
})
