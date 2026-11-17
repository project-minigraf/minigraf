// #896 spike driver: runs every case against every build in headless Chromium
// and writes results.json (raw traces) and prints a result matrix.
//
// usage: PKGS=<dir with main/ v2.0.11/ published/> node run.mjs [build ...]
// Each case runs in a fresh browser context (its own disposable profile and
// IndexedDB), with a unique database name.
import { createServer } from 'node:http'
import { readFile, writeFile, mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { extname, join, normalize, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'

const here = dirname(fileURLToPath(import.meta.url))
const pkgs = process.env.PKGS
if (!pkgs) throw new Error('set PKGS')
const only = process.env.CASES?.split(',')
const builds = process.argv.slice(2).length ? process.argv.slice(2) : ['main', 'v2.0.11', 'published']
const types = { '.html': 'text/html', '.js': 'text/javascript', '.json': 'application/json', '.wasm': 'application/wasm' }

const server = createServer(async (req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname))
  const file = path.startsWith('/pkgs/') ? join(pkgs, path.slice(6)) : join(here, path)
  try {
    const body = await readFile(file)
    res.writeHead(200, { 'content-type': types[extname(file)] ?? 'application/octet-stream' })
    res.end(body)
  } catch {
    res.writeHead(404).end()
  }
})
await new Promise((r) => server.listen(0, '127.0.0.1', r))
const origin = `http://127.0.0.1:${server.address().port}`

const browser = await chromium.launch()
const env = { browser: `chromium ${browser.version()}`, playwright: (await import('playwright/package.json', { with: { type: 'json' } })).default.version, os: `${process.platform} ${process.arch}`, node: process.version }

const tx = (...names) => `(transact [${names.map((n) => `[:${n} :item/name "${n}"]`).join(' ')}])`
// Enough facts that one transaction packs more than one fact page.
const big = (prefix, n) => tx(...Array.from({ length: n }, (_, i) => `${prefix}${i}`))

async function session(build) {
  // Disposable on-disk profile per case, so IndexedDB has a real quota.
  const dir = await mkdtemp(join(tmpdir(), 'spike896-profile-'))
  const ctx = await chromium.launchPersistentContext(dir)
  ctx.on('close', () => rm(dir, { recursive: true, force: true }))
  const page = ctx.pages()[0] ?? (await ctx.newPage())
  const errors = []
  page.on('pageerror', (e) => errors.push(String(e)))
  const load = async () => {
    await page.goto(`${origin}/harness.html?build=${build}`)
    await page.waitForFunction(() => window.__ready === true)
    return page.evaluate(() => window.mg.init())
  }
  const pkg = await load()
  const mg = (fn, ...args) => page.evaluate(fn, args)
  return { ctx, page, errors, pkg, load, mg, traces: [] }
}

// Names a session's writes acknowledged, from the traces: every `transact`
// on the persisted handle 'a' whose promise resolved. A resolved import makes
// the set unknown (null). Handles 'r'/'r2' (reopen checks) and 'src' (an
// in-memory import source) do not count.
function acknowledged(traces) {
  const args = {}
  const out = new Set()
  for (const ev of traces.flat()) {
    if (ev.ev === 'start' && ev.h === 'a') args[ev.id] = ev
    if (ev.ev === 'settle' && ev.state === 'resolved' && args[ev.id]) {
      const st = args[ev.id]
      if (st.op === 'import') return null
      if (st.op === 'execute' && st.arg?.startsWith('(transact')) for (const m of st.arg.matchAll(/"([^"]+)"/g)) out.add(m[1])
    }
  }
  return out
}

// Which injected fault the traces show actually happened.
function faultsSeen(traces) {
  const seen = new Set()
  for (const ev of traces.flat()) {
    if (ev.ev === 'inject-abort') seen.add(ev.pending > 0 ? 'request-abort' : 'abort-only')
    if (ev.ev === 'inject-sync-throw') seen.add('sync-throw')
    if (ev.ev === 'tx-abort' && ev.error === 'QuotaExceededError') seen.add('quota')
  }
  return [...seen]
}

// Reopen in a fresh page load and compare with what was acknowledged.
// `expected` is the case's intended acknowledged set; the verdict uses the set
// derived from actual settlements, and the two are reported separately.
async function reopen(s, db, expected) {
  const before = await s.mg(([h]) => window.mg.exportHashes(h), 'a').catch(() => null)
  const memNames = await s.mg(() => window.mg.names('a')).catch(() => null)
  s.traces.push(await s.page.evaluate(() => window.mg.trace))
  const ackSet = acknowledged(s.traces)
  const acked = ackSet ? [...ackSet] : expected
  await s.load()
  const raw = await s.mg(([n]) => window.mg.inspect(n), db)
  const open = await s.mg(([n]) => window.mg.open('r', n), db)
  const names = open.ok ? await s.mg(() => window.mg.names('r')) : null
  const want = [...acked].sort()
  const got = names?.ok ? names.names : null
  const lost = got ? want.filter((n) => !got.includes(n)) : want
  const unacked = got ? got.filter((n) => !want.includes(n)) : []
  // Page-set and content comparison; an export failure is a failure, not a match.
  const differs = before
    ? [...new Set([...Object.keys(before), ...Object.keys(raw.hashes)])]
        .filter((k) => before[k] !== raw.hashes[k])
        .map(Number)
        .sort((x, y) => x - y)
    : null
  // Then a write on top of the reopened database, reopened again, read through
  // the full scan and the AVET / EAVT index paths.
  let after = null
  if (open.ok) {
    const d = await s.mg(([q]) => window.mg.run('r', 'execute', q), tx('D'))
    const cp = await s.mg(() => window.mg.run('r', 'checkpoint'))
    s.traces.push(await s.page.evaluate(() => window.mg.trace))
    await s.load()
    const open2 = await s.mg(([n]) => window.mg.open('r2', n), db)
    const q = async (text) => s.mg(async ([t]) => {
      try { return { ok: true, rows: JSON.parse(await window.mg.handles.r2.execute(t)).results } }
      catch (e) { return { ok: false, reason: String(e?.message ?? e) } }
    }, text)
    after = {
      write_D: d,
      checkpoint: cp,
      open2,
      names2: open2.ok ? await s.mg(() => window.mg.names('r2')) : null,
      by_value: open2.ok ? Object.fromEntries(await Promise.all(['A', 'F', 'C', 'D'].map(async (n) => [n, await q(`(query [:find ?e :where [?e :item/name "${n}"]])`)]))) : null,
      by_entity: open2.ok ? Object.fromEntries(await Promise.all(['A', 'F', 'C', 'D'].map(async (n) => [n, await q(`(query [:find ?v :where [:${n} :item/name ?v]])`)]))) : null,
    }
    // Expected after the second reopen: what the first reopen read, plus D.
    const want2 = [...new Set([...(got ?? []), ...(d.state === 'resolved' ? ['D'] : [])])].sort()
    const names2 = after.names2?.ok ? after.names2.names : null
    const rowsOk = (n) => {
      const present = names2?.includes(n)
      const v = after.by_value[n], e = after.by_entity[n]
      return v.ok && e.ok && v.rows.length === (present ? 1 : 0) && JSON.stringify(e.rows) === (present ? JSON.stringify([[n]]) : '[]')
    }
    after.ok = !!(open2.ok && d.state === 'resolved' && cp.state === 'resolved' && names2 &&
      JSON.stringify(names2) === JSON.stringify(want2) && ['A', 'F', 'C', 'D'].every(rowsOk))
  }
  const differsOk = differs !== null && differs.length === 0
  return {
    in_memory_before_reopen: memNames,
    open,
    names,
    acked: want,
    acked_from_settlements: ackSet !== null,
    acked_matches_case_expectation: JSON.stringify(want) === JSON.stringify([...expected].sort()),
    faults_seen: faultsSeen(s.traces),
    lost,
    unacked_visible: unacked,
    raw: { header: raw.header, ids: raw.ids, missing: raw.missing, extra: raw.extra },
    pages_differing_from_last_in_memory_view: differs,
    after_reopen: after,
    stored_equals_in_memory: differsOk,
    consistent: !!(open.ok && names?.ok && lost.length === 0 && unacked.length === 0 && raw.missing.length === 0 && differsOk && after?.ok),
  }
}

const cases = {
  // 1. Baseline: writes, checkpoint, reopen.
  async baseline(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    steps.push(['write B (multi-page)', await s.mg(([q]) => window.mg.run('a', 'execute', q), big('B', 120))])
    steps.push(['checkpoint', await s.mg(() => window.mg.run('a', 'checkpoint'))])
    const acked = ['A', ...Array.from({ length: 120 }, (_, i) => `B${i}`)]
    return { steps, reopen: await reopen(s, db, acked) }
  },

  // 2. Recoverable failure, then checkpoint with injection disabled.
  async fail_then_checkpoint(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    await s.mg(() => window.mg.arm('request-abort'))
    steps.push(['write F (injected request-abort)', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('F'))])
    steps.push(['in-memory names after failure', await s.mg(() => window.mg.names('a'))])
    await s.mg(() => window.mg.disarm())
    steps.push(['checkpoint', await s.mg(() => window.mg.run('a', 'checkpoint'))])
    return { steps, reopen: await reopen(s, db, ['A']) }
  },

  // 3. Failure, then an independent successful write; also repeated failures.
  async fail_then_write(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    await s.mg(() => window.mg.arm('request-abort'))
    steps.push(['write F (injected request-abort)', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('F'))])
    await s.mg(() => window.mg.disarm())
    steps.push(['write C', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('C'))])
    return { steps, reopen: await reopen(s, db, ['A', 'C']) }
  },

  async repeated_fail_then_write(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    await s.mg(() => window.mg.arm('request-abort', 2))
    steps.push(['write F1 (injected)', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('F1'))])
    steps.push(['write F2 (injected)', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('F2'))])
    await s.mg(() => window.mg.disarm())
    steps.push(['write C', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('C'))])
    steps.push(['checkpoint', await s.mg(() => window.mg.run('a', 'checkpoint'))])
    return { steps, reopen: await reopen(s, db, ['A', 'C']) }
  },

  // 3b. As 3, but the failed write packs several fact pages, so the successor's
  // header names fact pages that IndexedDB never received.
  async fail_big_then_write(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    await s.mg(() => window.mg.arm('request-abort'))
    steps.push(['write F* (400 facts, injected request-abort)', await s.mg(([q]) => window.mg.run('a', 'execute', q), big('F', 400))])
    await s.mg(() => window.mg.disarm())
    steps.push(['write C', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('C'))])
    return { steps, reopen: await reopen(s, db, ['A', 'C']) }
  },

  // 3c. Candidate workaround: after a rejected write, drop the handle and open
  // a fresh one before writing again.
  async workaround_fresh_handle(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    await s.mg(() => window.mg.arm('request-abort'))
    steps.push(['write F* (400 facts, injected request-abort)', await s.mg(([q]) => window.mg.run('a', 'execute', q), big('F', 400))])
    await s.mg(() => window.mg.disarm())
    steps.push(['open a fresh handle (same page)', await s.mg(([n]) => window.mg.open('a', n), db)])
    steps.push(['names on the fresh handle', await s.mg(() => window.mg.names('a'))])
    steps.push(['write C', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('C'))])
    return { steps, reopen: await reopen(s, db, ['A', 'C']) }
  },

  // 4. Overlap: successor starts before the failing predecessor settles.
  async overlap(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    await s.mg(() => window.mg.arm('request-abort'))
    await s.mg(([q1, q2]) => {
      window.mg.start('F', 'a', 'execute', q1)
      window.mg.start('C', 'a', 'execute', q2)
    }, tx('F'), tx('C'))
    steps.push(['F settles', await s.mg(() => window.mg.wait('F'))])
    steps.push(['C settles', await s.mg(() => window.mg.wait('C'))])
    await s.mg(() => window.mg.disarm())
    const acked = ['A']
    if (steps[2][1].state === 'resolved') acked.push('C')
    return { steps, reopen: await reopen(s, db, acked) }
  },

  // 4a. Overlap with a multi-page failed predecessor: the successor's delta is
  // captured before the failure is known, so no handle-level workaround applies.
  async overlap_big(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    await s.mg(() => window.mg.arm('request-abort'))
    await s.mg(([q1, q2]) => {
      window.mg.start('F', 'a', 'execute', q1)
      window.mg.start('C', 'a', 'execute', q2)
    }, big('F', 400), tx('C'))
    steps.push(['F* settles', await s.mg(() => window.mg.wait('F'))])
    steps.push(['C settles', await s.mg(() => window.mg.wait('C'))])
    await s.mg(() => window.mg.disarm())
    const acked = ['A']
    if (steps[2][1].state === 'resolved') acked.push('C')
    return { steps, reopen: await reopen(s, db, acked) }
  },

  // 4b. Overlap control: both succeed.
  async overlap_control(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    await s.mg(([q1, q2]) => {
      window.mg.start('B', 'a', 'execute', q1)
      window.mg.start('C', 'a', 'execute', q2)
    }, tx('B'), tx('C'))
    steps.push(['B settles', await s.mg(() => window.mg.wait('B'))])
    steps.push(['C settles', await s.mg(() => window.mg.wait('C'))])
    return { steps, reopen: await reopen(s, db, ['A', 'B', 'C']) }
  },

  // 5. Abort with no request error; bounded wait.
  async abort_only(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    await s.mg(() => window.mg.arm('abort-only'))
    steps.push(['write F (injected abort-only), 5 s bound', await s.mg(([q]) => window.mg.run('a', 'execute', q, 5000), tx('F'))])
    await s.mg(() => window.mg.disarm())
    steps.push(['write C after', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('C'))])
    return { steps, reopen: await reopen(s, db, ['A', 'C']) }
  },

  // 5b. Realistic commit-time failure: origin quota override via CDP.
  async quota(s, db) {
    // Realistic trigger: QuotaExceededError at commit, an abort with no request
    // error. Chromium applies the override only if it is set before the origin
    // first uses storage, so it is set first, with room for A but not for Q.
    const steps = []
    const cdp = await s.ctx.newCDPSession(s.page)
    await cdp.send('Storage.overrideQuotaForOrigin', { origin, quotaSize: 100_000 })
    s.traces.push(await s.page.evaluate(() => window.mg.trace))
    await s.load()
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    steps.push(['write Q (400 facts, over quota), 5 s bound', await s.mg(([q]) => window.mg.run('a', 'execute', q, 5000), big('Q', 400))])
    await cdp.send('Storage.overrideQuotaForOrigin', { origin })
    steps.push(['write C after quota reset, 5 s bound', await s.mg(([q]) => window.mg.run('a', 'execute', q, 5000), tx('C'))])
    const acked = []
    if (steps[0][1].state === 'resolved') acked.push('A')
    if (steps[1][1].state === 'resolved') acked.push(...Array.from({ length: 400 }, (_, i) => `Q${i}`))
    if (steps[2][1].state === 'resolved') acked.push('C')
    return { steps, reopen: await reopen(s, db, acked) }
  },

  // 5c. Workaround for the hang: bound the write with a timeout, then drop the
  // handle and open a fresh one before writing again.
  async quota_workaround(s, db) {
    const steps = []
    const cdp = await s.ctx.newCDPSession(s.page)
    await cdp.send('Storage.overrideQuotaForOrigin', { origin, quotaSize: 100_000 })
    s.traces.push(await s.page.evaluate(() => window.mg.trace))
    await s.load()
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    steps.push(['write Q (400 facts, over quota), 5 s bound', await s.mg(([q]) => window.mg.run('a', 'execute', q, 5000), big('Q', 400))])
    await cdp.send('Storage.overrideQuotaForOrigin', { origin })
    steps.push(['open a fresh handle (same page)', await s.mg(([n]) => window.mg.open('a', n), db)])
    steps.push(['write C, 5 s bound', await s.mg(([q]) => window.mg.run('a', 'execute', q, 5000), tx('C'))])
    const acked = ['A']
    if (steps[3][1].state === 'resolved') acked.push('C')
    return { steps, reopen: await reopen(s, db, acked) }
  },

  // 6. Synchronous put exception after one put is queued (synthetic).
  async sync_throw(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    const pre = await s.mg(([n]) => window.mg.inspect(n), db)
    await s.mg(() => window.mg.arm('sync-throw'))
    steps.push(['write F (second put throws)', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('F'))])
    await s.mg(() => window.mg.disarm())
    await new Promise((r) => setTimeout(r, 500))
    const post = await s.mg(([n]) => window.mg.inspect(n), db)
    const changed = Object.keys(post.hashes).filter((k) => pre.hashes[k] !== post.hashes[k]).map(Number)
    steps.push(['raw pages changed by the failed write', { changed }])
    return { steps, reopen: await reopen(s, db, ['A']) }
  },

  // 7. Failed import, then checkpoint and a write.
  async import_fail(s, db) {
    const steps = []
    await s.mg(([n]) => window.mg.open('a', n), db)
    steps.push(['write A', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('A'))])
    await s.mg(() => window.mg.open('src', null))
    await s.mg(([q]) => window.mg.run('src', 'execute', q), big('I', 60))
    const blob = await s.mg(() => window.mg.exportBytes('src'))
    await s.mg(() => window.mg.arm('request-abort'))
    steps.push(['import I* (injected)', await s.mg(([b]) => window.mg.run('a', 'import', b), blob)])
    await s.mg(() => window.mg.disarm())
    steps.push(['in-memory names after failed import (count)', await s.mg(async () => {
      const r = await window.mg.names('a')
      return r.ok ? { count: r.names.length, hasA: r.names.includes('A') } : r
    })])
    steps.push(['checkpoint', await s.mg(() => window.mg.run('a', 'checkpoint'))])
    steps.push(['write C', await s.mg(([q]) => window.mg.run('a', 'execute', q), tx('C'))])
    // Acknowledged: A, then C. Import was rejected, so neither I* nor the loss of A was acknowledged.
    return { steps, reopen: await reopen(s, db, ['A', 'C']) }
  },
}

const results = { env, builds: {} }
let n = 0
for (const build of builds) {
  results.builds[build] = {}
  for (const [name, fn] of Object.entries(cases).filter(([k]) => !only || only.includes(k))) {
    const s = await session(build)
    results.builds[build].pkg = s.pkg
    const db = `spike896-${build}-${name}-${Date.now()}-${n++}`
    let out
    try {
      out = await fn(s, db)
    } catch (e) {
      out = { harness_error: String(e?.stack ?? e) }
    }
    s.traces.push(await s.page.evaluate(() => window.mg.trace).catch(() => []))
    out.traces = s.traces
    out.page_errors = s.errors
    out.fnonce_errors = s.errors.filter((e) => e.includes('FnOnce called more than once')).length
    out.other_page_errors = s.errors.filter((e) => !e.includes('FnOnce called more than once'))
    results.builds[build][name] = out
    await s.ctx.close()
    const r = out.reopen
    console.log(
      `${build.padEnd(10)} ${name.padEnd(26)} ` +
        (out.harness_error
          ? `HARNESS ERROR ${out.harness_error.split('\n')[0]}`
          : `${(out.steps ?? []).map(([, v]) => v?.state ?? '-').join(',').padEnd(40)} reopen=${r.open.ok ? 'ok' : 'FAIL'} ` +
            `lost=${r.lost.length} unacked=${r.unacked_visible.length} missing=${r.raw.missing.length} stale=[${r.pages_differing_from_last_in_memory_view ?? 'export failed'}] after=${r.after_reopen?.ok ? 'ok' : 'FAIL'} ` +
            `fault=[${r.faults_seen}] ack=${r.acked_from_settlements ? 'settled' : 'ASSUMED'}${r.acked_matches_case_expectation ? '' : '!=expected'} fnonce=${out.fnonce_errors} other_errors=${out.other_page_errors.length} ` +
            `${r.consistent ? 'CONSISTENT' : 'INCONSISTENT'}${r.open.ok ? '' : ' (' + r.open.reason + ')'}`),
    )
  }
}
await writeFile(join(process.env.OUT ?? here, 'results.json'), JSON.stringify(results, null, 1))
await browser.close()
server.close()
