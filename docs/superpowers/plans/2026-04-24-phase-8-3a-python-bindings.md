# Phase 8.3a: Python Bindings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish `minigraf` to PyPI as a platform wheel — Python bindings generated from the existing `minigraf-ffi` UniFFI crate via `maturin`.

**Architecture:** `maturin` with `bindings = "uniffi"` compiles `minigraf-ffi`, runs `uniffi-bindgen generate --language python`, and bundles the generated `.py` + compiled shared library into platform wheels. A thin `minigraf/__init__.py` re-exports `MiniGrafDb` and `MiniGrafError` from the generated `minigraf_ffi` module. No new Rust code is required.

**Tech Stack:** maturin ≥1.5, uniffi 0.31.1 (already in `minigraf-ffi`), pytest, PyO3/maturin-action (CI), PyPI trusted publishing or API token.

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| CREATE | `minigraf-ffi/python/pyproject.toml` | maturin project config, PyPI metadata |
| CREATE | `minigraf-ffi/python/minigraf/__init__.py` | Re-export `MiniGrafDb`, `MiniGrafError` from generated module |
| CREATE | `minigraf-ffi/python/tests/__init__.py` | Empty — marks tests as a package |
| CREATE | `minigraf-ffi/python/tests/test_basic.py` | pytest tests covering the four test scenarios |
| CREATE | `.github/workflows/python-ci.yml` | PR test matrix (4 platforms, maturin develop + pytest) |
| CREATE | `.github/workflows/python-release.yml` | Release matrix (maturin build + publish to PyPI) |
| MODIFY | `Cargo.toml` (root) | Bump version to `0.22.0` |
| MODIFY | `minigraf-ffi/Cargo.toml` | Bump version to `0.22.0` |
| MODIFY | `CHANGELOG.md` | Add 8.3a entry |
| MODIFY | `ROADMAP.md` | Mark 8.3a complete |

---

## Task 1: Create maturin project

**Files:**
- Create: `minigraf-ffi/python/pyproject.toml`
- Create: `minigraf-ffi/python/minigraf/__init__.py`
- Create: `minigraf-ffi/python/tests/__init__.py`

- [ ] **Step 1: Create `minigraf-ffi/python/pyproject.toml`**

```toml
[build-system]
requires = ["maturin>=1.5,<2"]
build-backend = "maturin"

[project]
name = "minigraf"
version = "0.22.0"
description = "Zero-config, single-file, embedded graph database with bi-temporal Datalog queries"
license = { text = "MIT OR Apache-2.0" }
requires-python = ">=3.9"
keywords = ["graph", "datalog", "bitemporal", "embedded", "database"]
classifiers = [
    "Programming Language :: Rust",
    "Programming Language :: Python :: Implementation :: CPython",
    "Programming Language :: Python :: 3",
    "License :: OSI Approved :: MIT License",
    "License :: OSI Approved :: Apache Software License",
]

[tool.maturin]
manifest-path = "../Cargo.toml"
bindings = "uniffi"
python-packages = ["minigraf"]
```

- [ ] **Step 2: Create `minigraf-ffi/python/minigraf/__init__.py`**

The uniffi-bindgen generates a Python file named after the library (`minigraf_ffi.py`) and places it inside the `minigraf` package. Re-export the public API from it:

```python
from .minigraf_ffi import MiniGrafDb, MiniGrafError

__all__ = ["MiniGrafDb", "MiniGrafError"]
```

- [ ] **Step 3: Create `minigraf-ffi/python/tests/__init__.py`**

```python
```
(empty file)

- [ ] **Step 4: Commit**

```bash
git add minigraf-ffi/python/
git commit -m "feat(python): add maturin project scaffold for PyPI wheel"
```

---

## Task 2: Write Python tests

**Files:**
- Create: `minigraf-ffi/python/tests/test_basic.py`

- [ ] **Step 1: Create `minigraf-ffi/python/tests/test_basic.py`**

```python
import json
import os
import pytest
from minigraf import MiniGrafDb, MiniGrafError


def test_open_in_memory():
    db = MiniGrafDb.open_in_memory()
    assert db is not None


def test_transact_and_query():
    db = MiniGrafDb.open_in_memory()
    result = json.loads(db.execute('(transact [[:alice :name "Alice"]])'))
    assert "transacted" in result

    result = json.loads(db.execute("(query [:find ?n :where [?e :name ?n]])"))
    assert result["variables"] == ["?n"]
    assert result["results"][0][0] == "Alice"


def test_invalid_datalog_raises():
    db = MiniGrafDb.open_in_memory()
    with pytest.raises(Exception):
        db.execute("not valid datalog !!!")


def test_file_backed_roundtrip(tmp_path):
    path = str(tmp_path / "test.graph")

    db = MiniGrafDb.open(path)
    db.execute('(transact [[:bob :name "Bob"]])')
    db.checkpoint()
    del db

    db2 = MiniGrafDb.open(path)
    result = json.loads(db2.execute("(query [:find ?n :where [?e :name ?n]])"))
    assert result["results"][0][0] == "Bob"
```

- [ ] **Step 2: Commit**

```bash
git add minigraf-ffi/python/tests/test_basic.py
git commit -m "test(python): add basic pytest suite for maturin wheel"
```

---

## Task 3: Verify local build

**Files:** none new

- [ ] **Step 1: Install maturin and pytest**

```bash
pip install "maturin>=1.5" pytest
```

- [ ] **Step 2: Build and install in development mode**

```bash
cd minigraf-ffi/python
maturin develop
```

Expected: compiles `minigraf-ffi`, generates `minigraf/minigraf_ffi.py`, installs the package into the current Python environment. No errors.

- [ ] **Step 3: Run tests**

```bash
cd minigraf-ffi/python
pytest tests/ -v
```

Expected: 4 tests pass.

- [ ] **Step 4: If `__init__.py` import fails** (module name mismatch)

Run `python -c "import minigraf; print(dir(minigraf))"` to inspect. If the generated module is named differently (e.g., `minigraf_ffi` is at `minigraf.minigraf_ffi`), update `__init__.py` to match the actual generated import path, then re-run `maturin develop && pytest`.

---

## Task 4: Add PR CI workflow

**Files:**
- Create: `.github/workflows/python-ci.yml`

- [ ] **Step 1: Create `.github/workflows/python-ci.yml`**

```yaml
name: Python CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  test:
    name: Python tests (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, ubuntu-24.04-arm, macos-14, windows-latest]

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Set up Python
        uses: actions/setup-python@v5
        with:
          python-version: '3.11'

      - name: Install maturin and pytest
        run: pip install "maturin>=1.5" pytest

      - name: Build and install (dev mode)
        working-directory: minigraf-ffi/python
        run: maturin develop

      - name: Run tests
        working-directory: minigraf-ffi/python
        run: pytest tests/ -v
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/python-ci.yml
git commit -m "ci(python): add PR test matrix for Python wheel (4 platforms)"
```

---

## Task 5: Add release workflow

**Files:**
- Create: `.github/workflows/python-release.yml`

- [ ] **Step 1: Create `.github/workflows/python-release.yml`**

```yaml
name: Python Release

on:
  workflow_call:
    inputs:
      tag:
        required: true
        type: string
  workflow_dispatch:
    inputs:
      tag:
        required: true
        type: string

jobs:
  build-wheels:
    name: Build wheel (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            args: --release --manylinux 2014
          - os: ubuntu-24.04-arm
            args: --release --manylinux 2014
          - os: macos-14
            args: --release --universal2
          - os: windows-latest
            args: --release

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: x86_64-apple-darwin  # needed for universal2 on macos-14 (arm)

      - name: Build wheel
        uses: PyO3/maturin-action@v1
        with:
          command: build
          args: ${{ matrix.args }} --out dist
          working-directory: minigraf-ffi/python

      - name: Upload wheel
        uses: actions/upload-artifact@v4
        with:
          name: python-wheel-${{ matrix.os }}
          path: minigraf-ffi/python/dist/*.whl

  publish:
    name: Publish to PyPI
    needs: build-wheels
    runs-on: ubuntu-latest
    environment:
      name: pypi
      url: https://pypi.org/p/minigraf
    permissions:
      id-token: write  # for PyPI trusted publishing

    steps:
      - name: Download all wheels
        uses: actions/download-artifact@v4
        with:
          pattern: python-wheel-*
          path: dist
          merge-multiple: true

      - name: Publish to PyPI
        uses: pypa/gh-action-pypi-publish@release/v1
        with:
          packages-dir: dist/
```

Note: this workflow uses PyPI [trusted publishing](https://docs.pypi.org/trusted-publishers/) (OIDC, no API token needed). Configure a trusted publisher on PyPI for this repo before the first release.

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/python-release.yml
git commit -m "ci(python): add release workflow — maturin build + PyPI trusted publish"
```

---

## Task 6: Bump version and update docs

**Files:**
- Modify: `Cargo.toml` (root)
- Modify: `minigraf-ffi/Cargo.toml`
- Modify: `minigraf-ffi/python/pyproject.toml`
- Modify: `CHANGELOG.md`
- Modify: `ROADMAP.md`

- [ ] **Step 1: Bump versions to `0.22.0`**

In `Cargo.toml` (root), change:
```toml
version = "0.21.1"
```
to:
```toml
version = "0.22.0"
```

In `minigraf-ffi/Cargo.toml`, change:
```toml
version = "0.21.1"
```
to:
```toml
version = "0.22.0"
```

In `minigraf-ffi/python/pyproject.toml`, change:
```toml
version = "0.22.0"
```
(already set correctly in Task 1 — verify it matches)

- [ ] **Step 2: Run `cargo check` to verify the version bump doesn't break anything**

```bash
cargo check --workspace
```

Expected: compiles cleanly with no errors.

- [ ] **Step 3: Add CHANGELOG entry**

Add at the top of `CHANGELOG.md` (after the header):

```markdown
## [0.22.0] — 2026-04-XX

### Added
- **Phase 8.3a**: Python bindings published to PyPI as `minigraf`.
  Install with `pip install minigraf`. API: `MiniGrafDb.open(path)`,
  `MiniGrafDb.open_in_memory()`, `.execute(datalog)`, `.checkpoint()`.
  Pre-built wheels for Linux x86_64/aarch64, macOS universal2, Windows x86_64.
```

- [ ] **Step 4: Mark 8.3a complete in ROADMAP.md**

Find the Phase 8.3a section and update its status to `✅ COMPLETE` following the same format as prior completed phases.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml minigraf-ffi/Cargo.toml minigraf-ffi/python/pyproject.toml \
        CHANGELOG.md ROADMAP.md
git commit -m "chore(release): bump version to v0.22.0 — Phase 8.3a Python bindings"
```

- [ ] **Step 6: Tag the release**

```bash
git tag -a v0.22.0 -m "Phase 8.3a complete — Python bindings published to PyPI"
git push origin v0.22.0
```
