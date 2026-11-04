import { createServer } from 'node:http'
import { readFile, writeFile, unlink } from 'node:fs/promises'
import { resolve, sep, extname, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium, firefox, webkit } from 'playwright'

const root = dirname(fileURLToPath(import.meta.url))
const [engine = 'chromium', output, count = '5', flag] = process.argv.slice(2)
if (!['chromium', 'firefox', 'webkit'].includes(engine) || !output) {
  throw new Error('usage: node browser-runner.mjs chromium|firefox|webkit OUTPUT [SAMPLES] [--test]')
}
const samples = Number(count)
if (!Number.isInteger(samples) || samples < 1) throw new Error('invalid samples')
const maxFacts = Number(process.env.SWEEP_MAX_FACTS ?? Number.MAX_SAFE_INTEGER)
if (!Number.isSafeInteger(maxFacts) || maxFacts < 1) throw new Error('invalid SWEEP_MAX_FACTS')
const types = { '.mjs': 'text/javascript', '.js': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json' }
const server = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost')
  if (url.pathname === '/') return res.writeHead(200, { 'content-type': 'text/html' }).end('<!doctype html><title>Migration sweep</title>')
  const path = resolve(root, '.' + decodeURIComponent(url.pathname))
  if (!path.startsWith(root + sep)) return res.writeHead(403).end()
  try {
    const bytes = await readFile(path)
    res.writeHead(200, { 'content-type': types[extname(path)] ?? 'application/octet-stream' }).end(bytes)
  } catch { res.writeHead(404).end() }
})
await new Promise(r => server.listen(0, '127.0.0.1', r))
let browser
try {
  browser = await ({ chromium, firefox, webkit }[engine]).launch()
  const fixtures = JSON.parse(await readFile(resolve(root, 'fixtures/manifest.json'), 'utf8'))
    .filter(f => f.facts <= maxFacts)
  if (!fixtures.length) throw new Error('no selected fixtures')
  const result = { results: [], test: flag === '--test',
    browser: { engine, version: browser.version() }, node: process.version,
    maxFacts, selectedFixtures: fixtures.map(f => f.file) }
  // Retain completed samples if a later case fails; only a complete run gets
  // the requested filename. The summarizer still requires the entire matrix.
  const partial = output + '.partial'
  await writeFile(partial, JSON.stringify(result, null, 2) + '\n', { flag: 'wx' })
  for (const fixture of fixtures) {
    for (let sample = 0; sample < samples; sample++) {
      const context = await browser.newContext()
      try {
        const page = await context.newPage()
        page.on('console', message => console.error(message.text()))
        await page.goto(`http://127.0.0.1:${server.address().port}/`)
        const run = await page.evaluate(async options => {
          const { run } = await import('./browser.mjs')
          return run(options)
        }, { fixture: fixture.file, sample, test: result.test })
        result.results.push(...run.results)
        result.userAgent = run.userAgent
        result.persistence = run.persistence
        result.clock = run.clock
        await writeFile(partial, JSON.stringify(result, null, 2) + '\n')
      } finally { await context.close() }
    }
  }
  result.completedUtc = new Date().toISOString()
  await writeFile(output, JSON.stringify(result, null, 2) + '\n', { flag: 'wx' })
  await unlink(partial)
} finally { await browser?.close(); await new Promise(r => server.close(r)) }
