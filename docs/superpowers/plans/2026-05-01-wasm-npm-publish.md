# WASM npm Publish + Package Folder Naming Consistency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish the browser WASM package to npm as `@minigraf/browser`, and rename `pkg/` → `minigraf-wasm/` and `swift/` → `minigraf-swift/` for consistent top-level package folder naming.

**Architecture:** Mechanical rename of two directories plus path updates in four CI workflows, two doc files, and the browser example. A new `publish-npm` job is added to `wasm-release.yml` that rebuilds WASM fresh at release time, patches the package name and version, then publishes to npm. The GitHub Release tarball upload is retained alongside npm for CDN/direct-download consumers.

**Tech Stack:** GitHub Actions, wasm-pack, npm, sed (for bulk path replacement in mobile.yml)

---

## File Map

**Renamed (git mv):**
- `pkg/` → `minigraf-wasm/`
- `swift/` → `minigraf-swift/`

**Deleted:**
- `minigraf-wasm/.gitignore` — was `pkg/.gitignore`, contained `*` (wasm-pack default); not wanted in a committed source directory

**Modified:**
- `minigraf-wasm/package.json` — `"name"` → `"@minigraf/browser"`, `"version"` → `"0.0.0"`
- `.github/workflows/wasm-browser.yml` — add `--out-dir minigraf-wasm`, update size check and artifact paths
- `.github/workflows/wasm-release.yml` — add `--out-dir minigraf-wasm`, update tar path, add `publish-npm` job
- `.github/workflows/mobile.yml` — all `swift/` path references → `minigraf-swift/`
- `Package.swift` — `swift/Sources/MinigrafKit` → `minigraf-swift/Sources/MinigrafKit`
- `examples/browser/app.js` — update import path and build comment
- `examples/browser/README.md` — update `pkg/` references and remove stale "gitignored" note
- `ROADMAP.md` — two `pkg/` mentions
- `CHANGELOG.md` — one `pkg/` mention
- `TEST_COVERAGE.md` — one `pkg/` mention

---

## Important: wasm-pack regenerates package.json

`wasm-pack build` always regenerates `minigraf-wasm/package.json` from the Cargo.toml crate name (`minigraf`). This means the published `package.json` will have `"name": "minigraf"` unless patched after build. The `publish-npm` CI job must patch **both** `name` and `version` after running `wasm-pack build`. The committed `minigraf-wasm/package.json` with `@minigraf/browser` serves as documentation and source-of-truth for metadata (description, keywords, etc.) but will be overwritten locally on every `wasm-pack build`.

---

## Task 1: Rename directories + delete internal .gitignore

**Files:**
- Rename: `pkg/` → `minigraf-wasm/`
- Rename: `swift/` → `minigraf-swift/`
- Delete: `minigraf-wasm/.gitignore`

- [ ] **Step 1: Create worktree**

```bash
git worktree add .worktrees/wasm-npm-publish -b wasm-npm-publish
cd .worktrees/wasm-npm-publish
```

- [ ] **Step 2: Rename pkg/ → minigraf-wasm/**

```bash
git mv pkg minigraf-wasm
```

- [ ] **Step 3: Delete the internal .gitignore**

The file `.gitignore` inside the (now renamed) `minigraf-wasm/` contains a single `*`, which was wasm-pack's default behaviour to gitignore its own output. Since we're committing this directory, delete it:

```bash
git rm minigraf-wasm/.gitignore
```

- [ ] **Step 4: Rename swift/ → minigraf-swift/**

```bash
git mv swift minigraf-swift
```

- [ ] **Step 5: Verify the renames look right**

```bash
git status
```

Expected: three staged changes — `renamed: pkg -> minigraf-wasm`, `deleted: minigraf-wasm/.gitignore` (formerly `pkg/.gitignore`), `renamed: swift -> minigraf-swift`. No untracked files.

- [ ] **Step 6: Commit**

```bash
git commit -m "refactor: rename pkg/ → minigraf-wasm/, swift/ → minigraf-swift/ for naming consistency"
```

---

## Task 2: Update minigraf-wasm/package.json

**Files:**
- Modify: `minigraf-wasm/package.json`

- [ ] **Step 1: Update name and version**

Edit `minigraf-wasm/package.json`. Change:
- `"name": "minigraf"` → `"name": "@minigraf/browser"`
- `"version": "0.19.0"` → `"version": "0.0.0"`

The file should look like:

```json
{
  "name": "@minigraf/browser",
  "type": "module",
  "collaborators": [
    "Aditya Mukhopadhyay"
  ],
  "description": "Zero-config, single-file, embedded graph database with bi-temporal Datalog queries",
  "version": "0.0.0",
  "license": "MIT OR Apache-2.0",
  "repository": {
    "type": "git",
    "url": "https://github.com/project-minigraf/minigraf"
  },
  "files": [
    "minigraf_bg.wasm",
    "minigraf.js",
    "minigraf.d.ts"
  ],
  "main": "minigraf.js",
  "types": "minigraf.d.ts",
  "sideEffects": [
    "./snippets/*"
  ],
  "keywords": [
    "graph",
    "datalog",
    "bitemporal",
    "embedded",
    "database"
  ]
}
```

- [ ] **Step 2: Commit**

```bash
git add minigraf-wasm/package.json
git commit -m "feat: rename wasm npm package to @minigraf/browser"
```

---

## Task 3: Update wasm-browser.yml

**Files:**
- Modify: `.github/workflows/wasm-browser.yml`

- [ ] **Step 1: Add --out-dir to wasm-pack build**

In `.github/workflows/wasm-browser.yml`, find:

```yaml
      - name: Build (release)
        run: wasm-pack build --target web --features browser
```

Replace with:

```yaml
      - name: Build (release)
        run: wasm-pack build --target web --features browser --out-dir minigraf-wasm
```

- [ ] **Step 2: Update gzip size check path**

Find:

```yaml
          SIZE=$(gzip -c pkg/minigraf_bg.wasm | wc -c)
```

Replace with:

```yaml
          SIZE=$(gzip -c minigraf-wasm/minigraf_bg.wasm | wc -c)
```

- [ ] **Step 3: Update artifact upload path**

Find:

```yaml
      - name: Upload pkg artifact
        uses: actions/upload-artifact@v4
        with:
          name: wasm-pkg
          path: pkg/
```

Replace with:

```yaml
      - name: Upload pkg artifact
        uses: actions/upload-artifact@v4
        with:
          name: wasm-pkg
          path: minigraf-wasm/
```

Note: `wasm-pack test` does NOT produce output in `pkg/` — it only runs tests — so that step needs no changes.

- [ ] **Step 4: Verify no remaining pkg/ references in the file**

```bash
grep 'pkg/' .github/workflows/wasm-browser.yml
```

Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/wasm-browser.yml
git commit -m "ci: update wasm-browser.yml for minigraf-wasm/ rename"
```

---

## Task 4: Update wasm-release.yml (paths + publish-npm job)

**Files:**
- Modify: `.github/workflows/wasm-release.yml`

- [ ] **Step 1: Add --out-dir to wasm-pack build in build-wasm-browser job**

Find:

```yaml
      - name: Build browser WASM
        run: wasm-pack build --target web --features browser
```

Replace with:

```yaml
      - name: Build browser WASM
        run: wasm-pack build --target web --features browser --out-dir minigraf-wasm
```

- [ ] **Step 2: Update the tar step to use minigraf-wasm/**

Find:

```yaml
      - name: Package pkg/ directory
        run: |
          mkdir -p target/wasm-artifacts
          tar -czf target/wasm-artifacts/minigraf-browser-wasm.tar.gz -C . pkg/
```

Replace with:

```yaml
      - name: Package minigraf-wasm/ directory
        run: |
          mkdir -p target/wasm-artifacts
          tar -czf target/wasm-artifacts/minigraf-browser-wasm.tar.gz -C . minigraf-wasm/
```

- [ ] **Step 3: Add publish-npm job**

At the end of `.github/workflows/wasm-release.yml`, after the closing of the `release-upload-wasm` job, append:

```yaml
  publish-npm:
    name: Publish @minigraf/browser to npm
    needs: [build-wasm-browser]
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v6
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-unknown-unknown
      - name: Install wasm-pack
        run: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
      - name: Set up Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'
          registry-url: 'https://registry.npmjs.org'
      - name: Build browser WASM
        run: wasm-pack build --target web --features browser --out-dir minigraf-wasm
      - name: Stamp package name and version
        run: |
          TAG="${{ inputs.tag || github.ref_name }}"
          VERSION="${TAG#v}"
          node -e "const fs=require('fs'); const p='minigraf-wasm/package.json'; const pkg=JSON.parse(fs.readFileSync(p,'utf8')); pkg.name='@minigraf/browser'; pkg.version='${VERSION}'; fs.writeFileSync(p,JSON.stringify(pkg,null,2)+'\n');"
      - name: Publish to npm
        working-directory: minigraf-wasm
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
        run: npm publish --access public
```

- [ ] **Step 4: Verify no remaining pkg/ references in the file**

```bash
grep 'pkg/' .github/workflows/wasm-release.yml
```

Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/wasm-release.yml
git commit -m "ci: add @minigraf/browser npm publish job to wasm-release.yml"
```

---

## Task 5: Update mobile.yml + Package.swift

**Files:**
- Modify: `.github/workflows/mobile.yml`
- Modify: `Package.swift`

- [ ] **Step 1: Replace all swift/ path references in mobile.yml**

All occurrences of `swift/` in `mobile.yml` are file paths (none are the `--language swift` flag which lacks a trailing slash). Run:

```bash
sed -i 's|swift/|minigraf-swift/|g' .github/workflows/mobile.yml
```

- [ ] **Step 2: Verify the replacement**

```bash
grep -n 'swift/' .github/workflows/mobile.yml
```

Expected: all lines contain `minigraf-swift/`. No plain `swift/` without the `minigraf-` prefix.

Also verify `--language swift` was NOT changed:

```bash
grep 'language swift' .github/workflows/mobile.yml
```

Expected: one line: `            --language swift \`

- [ ] **Step 3: Update Package.swift**

In `Package.swift`, find:

```swift
            path: "swift/Sources/MinigrafKit"
```

Replace with:

```swift
            path: "minigraf-swift/Sources/MinigrafKit"
```

- [ ] **Step 4: Verify no remaining swift/ path references**

```bash
grep 'swift/' .github/workflows/mobile.yml Package.swift
```

Expected: no output (all have been prefixed with `minigraf-`).

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/mobile.yml Package.swift
git commit -m "ci: update mobile.yml and Package.swift for minigraf-swift/ rename"
```

---

## Task 6: Update examples/browser/ and living docs

**Files:**
- Modify: `examples/browser/app.js`
- Modify: `examples/browser/README.md`
- Modify: `ROADMAP.md`
- Modify: `CHANGELOG.md`
- Modify: `TEST_COVERAGE.md`

- [ ] **Step 1: Update import path in app.js**

In `examples/browser/app.js`, find:

```js
// Build first: wasm-pack build --target web --features browser
```

Replace with:

```js
// Build first: wasm-pack build --target web --features browser --out-dir minigraf-wasm
```

Then find:

```js
import init, { BrowserDb } from "../../pkg/minigraf.js";
```

Replace with:

```js
import init, { BrowserDb } from "../../minigraf-wasm/minigraf.js";
```

- [ ] **Step 2: Update examples/browser/README.md**

Find:

```markdown
```bash
wasm-pack build --target web --features browser
```

This produces `pkg/` containing `minigraf.js`, `minigraf_bg.wasm`, and
`minigraf.d.ts`.
```

Replace with:

```markdown
```bash
wasm-pack build --target web --features browser --out-dir minigraf-wasm
```

This produces `minigraf-wasm/` containing `minigraf.js`, `minigraf_bg.wasm`, and
`minigraf.d.ts`.
```

Then find and remove the stale gitignore note:

```markdown
- The `pkg/` directory is gitignored — rebuild after pulling changes.
```

Replace with:

```markdown
- The `minigraf-wasm/` directory is committed — the files are up to date after pulling.
```

- [ ] **Step 3: Update ROADMAP.md**

Find (line ~1131):

```markdown
- ✅ Built with `wasm-pack` — generates `pkg/` with JS glue and TypeScript `.d.ts`
```

Replace with:

```markdown
- ✅ Built with `wasm-pack` — generates `minigraf-wasm/` with JS glue and TypeScript `.d.ts`
```

Find (line ~1176):

```markdown
# Build for browser (generates pkg/ with .js, .d.ts, .wasm)
```

Replace with:

```markdown
# Build for browser (generates minigraf-wasm/ with .js, .d.ts, .wasm)
```

- [ ] **Step 4: Update CHANGELOG.md**

Find:

```markdown
  - `wasm-pack` build workflow (`wasm32-unknown-unknown --features browser`) generating `pkg/` with JS glue and TypeScript definitions
```

Replace with:

```markdown
  - `wasm-pack` build workflow (`wasm32-unknown-unknown --features browser`) generating `minigraf-wasm/` with JS glue and TypeScript definitions
```

- [ ] **Step 5: Update TEST_COVERAGE.md**

Find:

```markdown
- ✅ `wasm-pack` build generating `pkg/` with JS glue and TypeScript `.d.ts`
```

Replace with:

```markdown
- ✅ `wasm-pack` build generating `minigraf-wasm/` with JS glue and TypeScript `.d.ts`
```

- [ ] **Step 6: Final check — no remaining pkg/ references in living docs or examples**

```bash
grep -r 'pkg/' examples/ ROADMAP.md CHANGELOG.md TEST_COVERAGE.md README.md 2>/dev/null
```

Expected: no output.

- [ ] **Step 7: Commit**

```bash
git add examples/browser/app.js examples/browser/README.md \
        ROADMAP.md CHANGELOG.md TEST_COVERAGE.md
git commit -m "docs: update pkg/ → minigraf-wasm/ references in examples and living docs"
```

---

## Task 7: Create GitHub issue for WASI npm publish

- [ ] **Step 1: Create the issue**

```bash
gh issue create \
  --title "Publish @minigraf/wasi to npm" \
  --body "$(cat <<'EOF'
The WASI binary (\`minigraf-wasi.wasm\`) is currently distributed only as a GitHub Release asset. To publish it to npm as \`@minigraf/wasi\`, it needs:

- A dedicated \`minigraf-wasi/\` directory (parallel to \`minigraf-wasm/\`)
- A \`package.json\` with \`"name": "@minigraf/wasi"\`
- A thin JS loader/wrapper so consumers can import it without raw WASM handling
- A \`publish-npm\` job in \`wasm-release.yml\` analogous to the browser one

The WASI binary itself is already built and released — this is purely the npm packaging layer.

Tracked as a post-1.0 follow-up from the minigraf-wasm npm publish work.
EOF
)"
```

- [ ] **Step 2: Note the issue URL**

The `gh issue create` command will print the issue URL. No further action needed.

---

## Task 8: Open PR

- [ ] **Step 1: Push the branch**

```bash
git push -u origin wasm-npm-publish
```

- [ ] **Step 2: Create the PR**

```bash
gh pr create \
  --title "feat: publish @minigraf/browser to npm, rename pkg/ and swift/ for naming consistency" \
  --body "$(cat <<'EOF'
## Summary

- Renames `pkg/` → `minigraf-wasm/` and `swift/` → `minigraf-swift/` for uniform `minigraf-*` top-level package folder naming (consistent with `minigraf-node/` and `minigraf-c/`)
- Deletes `minigraf-wasm/.gitignore` (was wasm-pack's default `*`); `minigraf-wasm/` is now a committed source directory
- Updates `minigraf-wasm/package.json` name to `@minigraf/browser`
- Adds `publish-npm` job to `wasm-release.yml` — publishes `@minigraf/browser` to npm on every tagged release; GitHub Release tarball is retained for CDN/direct-download consumers
- Updates all path references in CI workflows, `Package.swift`, `examples/browser/`, and living docs

## Test plan

- [ ] `wasm-browser.yml` CI passes on this PR (build + headless browser tests)
- [ ] `mobile.yml` CI passes (Swift path references updated correctly)
- [ ] No `pkg/` or `swift/` path references remain in CI workflows or living docs (checked by grep in Task 6 Step 6)
- [ ] After merge and tag: verify `@minigraf/browser` appears on npm

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```
