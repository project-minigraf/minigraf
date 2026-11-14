// Usage: node measure.mjs <built-package-dir> <fixture-dir> [chromium|firefox|webkit]
// Install Playwright 1.55.0 and the selected engine in this script's directory.
import { createServer } from 'node:http'
import { readFile, stat } from 'node:fs/promises'
import { resolve, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium, firefox, webkit } from 'playwright'

const [packageDir, fixtureDir, engine = 'chromium'] = process.argv.slice(2)
if (!packageDir || !fixtureDir) throw new Error('expected package directory and fixture directory')
const here = fileURLToPath(new URL('.', import.meta.url))
const fixtures = ['4mib.graph', '40mib.graph']
const sizes = Object.fromEntries(await Promise.all(fixtures.map(async (name) => [name, (await stat(join(fixtureDir, name))).size])))
if (sizes[fixtures[0]] < 4 * 1024 ** 2 || sizes[fixtures[1]] < 40 * 1024 ** 2) throw new Error('undersized fixture')
const server = createServer(async (req, res) => {
  const path = new URL(req.url, 'http://localhost').pathname
  try {
    if (path === '/') { res.end('<!doctype html><title>OPFS memory measurement</title>'); return }
    const base = path.startsWith('/pkg/') ? resolve(packageDir) : path.startsWith('/fixtures/') ? resolve(fixtureDir) : here
    const relative = path.replace(/^\/(pkg|fixtures)\//, '').replace(/^\//, '')
    const file = resolve(base, relative)
    if (!file.startsWith(`${base}/`)) { res.writeHead(403).end(); return }
    const body = await readFile(file)
    res.setHeader('content-type', file.endsWith('.wasm') ? 'application/wasm' : 'text/javascript')
    res.end(body)
  } catch { res.writeHead(404).end() }
})
await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve) })
let browser
try {
  browser = await ({ chromium, firefox, webkit })[engine].launch()
  const page = await browser.newPage()
  await page.goto(`http://127.0.0.1:${server.address().port}`)
  const measurements = []
  for (const mode of ['opfs', 'indexeddb']) for (const fixture of fixtures) {
    const name = `memory-${crypto.randomUUID()}`
    const invoke = (operation) => page.evaluate(({ operation, mode, fixture, name }) => new Promise((resolve, reject) => {
      const worker = new Worker('/measure-worker.js', { type: 'module' })
      const timeout = setTimeout(() => { worker.terminate(); reject(new Error('worker timed out')) }, 180000)
      worker.onerror = (e) => { clearTimeout(timeout); worker.terminate(); reject(new Error(e.message)) }
      worker.onmessage = ({ data }) => {
        if (data.ready) worker.postMessage({ operation, mode, fixture, name })
        else {
          clearTimeout(timeout)
          worker.terminate()
          data.error ? reject(new Error(data.error)) : resolve(data)
        }
      }
    }), { operation, mode, fixture, name })
    await invoke('seed')
    const measurement = await invoke('measure')
    measurement.databaseBytes = sizes[fixture]
    measurement.indexedDbImagePages = mode === 'indexeddb' ? sizes[fixture] / 4096 : null
    measurements.push(measurement)
  }
  const opfs = measurements.filter((row) => row.mode === 'opfs')
  const delta = opfs[1].memoryHighWaterBytes - opfs[0].memoryHighWaterBytes
  console.log(JSON.stringify({ engine, version: browser.version(), measurements, opfsMemoryDeltaBytes: delta,
    within4MiB: delta <= 4 * 1024 ** 2 }, null, 2))
  if (delta > 4 * 1024 ** 2) process.exitCode = 1
} finally {
  await browser?.close()
  server.close()
}
