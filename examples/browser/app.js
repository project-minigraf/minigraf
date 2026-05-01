// Minigraf browser demo — no bundler required.
// Build first: wasm-pack build --target web --features browser --out-dir minigraf-wasm
// Then serve from repo root: python3 -m http.server 8080
// Open: http://localhost:8080/examples/browser/

import init, { BrowserDb } from "../../minigraf-wasm/minigraf.js";

async function main() {
  // Initialise the WASM module (loads minigraf_bg.wasm).
  await init();

  // Open a database backed by IndexedDB (persists across page reloads).
  const db = await BrowserDb.open("minigraf-demo");

  // Assert some facts.
  await db.execute(`(transact [
    [:alice :person/name "Alice"]
    [:alice :person/age  30]
    [:alice :friend      :bob]
    [:bob   :person/name "Bob"]
  ])`);

  // Query with Datalog.
  const raw = await db.execute(`
    (query [:find ?friend-name
            :where [:alice :friend ?f]
                   [?f :person/name ?friend-name]])
  `);
  const result = JSON.parse(raw);
  console.log("Alice's friends:", result.results.map(row => row[0]));
  // Expected: ["Bob"]

  // Export to a portable .graph blob.
  const blob = db.exportGraph();
  console.log(".graph blob size:", blob.byteLength, "bytes");

  // Import into a fresh in-memory db.
  const db2 = BrowserDb.openInMemory();
  await db2.importGraph(blob);
  const raw2 = await db2.execute(
    `(query [:find ?name :where [?e :person/name ?name]])`
  );
  console.log("After import, names:", JSON.parse(raw2).results.map(r => r[0]));
  // Expected: ["Alice", "Bob"] (order may vary)
}

main().catch(console.error);
