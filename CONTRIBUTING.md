# Contributing to Minigraf

Thank you for your interest in contributing. Minigraf is a hobby project with a long-term vision — quality and correctness matter more than pace. Please read this document before opening an issue or pull request.

## Before You Contribute

1. **Read [PHILOSOPHY.md](PHILOSOPHY.md)** — understand the core principles before proposing anything. A well-intentioned feature that violates the philosophy (e.g., adds client-server architecture, breaks single-file storage, or adds heavy dependencies) will not be merged.

2. **Read [ROADMAP.md](ROADMAP.md)** — check whether your idea is already planned, deferred, or explicitly out of scope.

3. **Open an issue first** for non-trivial changes — a quick discussion before writing code avoids wasted effort.

## What We Welcome

- Bug fixes (with a test that reproduces the bug)
- Performance improvements with benchmarks showing the gain
- Documentation improvements and example additions
- Test coverage improvements (especially error-path coverage)
- Cross-platform compatibility fixes (Linux, macOS, Windows, eventually WASM/mobile)

## What We Will Not Merge

- Features that break the single-file storage philosophy
- Client-server architecture or network protocols in core
- Large dependency additions that increase binary size significantly
- Breaking changes to the public API or `.graph` file format without overwhelming justification
- Code without tests
- Features only useful for distributed or enterprise systems

## Development Setup

```bash
# Clone and build
git clone https://github.com/adityamukho/minigraf.git
cd minigraf
cargo build

# Activate the pre-push hook (runs fmt, clippy, and tests before every push)
git config core.hooksPath .githooks

# Run all tests
cargo test

# Run a specific test suite
cargo test --test bitemporal_test -- --nocapture
cargo test --test wal_test -- --nocapture

# Run clippy (must be clean before submitting a PR)
cargo clippy -- -D warnings

# Run the interactive REPL
cargo run

# Try the recursive rules demo
cargo run < demos/demo_recursive.txt
```

## Measuring Code Coverage

Install `cargo-llvm-cov` (one-time):

```bash
cargo install cargo-llvm-cov
```

Run branch coverage and open the HTML report:

```bash
cargo llvm-cov --branch --open
```

The Phase 7.5 target is ≥90% branch coverage on `src/query/datalog/` modules. Re-run after adding tests to confirm progress.

## Code Standards

- **No `unsafe` code** — the crate enforces `#![forbid(unsafe_code)]`; do not attempt to work around this
- **No `unwrap()` or `expect()` in library code paths** — use `?` and typed errors; `unwrap`/`expect` are only acceptable in tests and the binary
- **Every new feature needs tests** — unit tests in the relevant `src/` module plus integration tests in `tests/` where applicable
- **Clippy must pass** — `cargo clippy -- -D warnings` must be clean
- **Format your code** — `cargo fmt` before committing
- **Error paths matter** — test failure cases, not just happy paths

## Pull Request Process

1. Fork the repository and create a feature branch from `main`
2. Write tests for your change before or alongside the implementation
3. Ensure `cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt --check` all pass
4. Update `CHANGELOG.md` with a brief description of your change under an `Unreleased` section
5. Open a PR with a clear description of what the change does and why
6. Reference any related issues in the PR description

## Philosophy Check

Before submitting, ask yourself:

- Does this keep the single-file philosophy intact?
- Does this maintain zero-configuration?
- Does this add unnecessary complexity?
- Is this needed for embedded use cases (mobile, WASM, desktop)?
- Does this compromise reliability or crash safety?

If you answer "yes" to the last two questions, reconsider. If in doubt, open an issue and discuss first.

## Pre-Publishing Checklist (crates.io)

**Do not publish before Phase 7.8.** Before publishing, verify all of the following:

### Minimum Bar
- [x] Phase 6.4 benchmarks complete (`BENCHMARKS.md`)
- [x] Phase 6.5 complete — on-disk B+tree, file format v6
- [x] Phase 7.1 complete — stratified negation, 407 tests passing
- [ ] Checkpoint-during-crash recovery exercised (Phase 7.5)
- [ ] Error-path coverage ≥90% (currently ~82%) (Phase 7.5)
- [x] GitHub Discussions enabled

### API Cleanup (Phase 7.8)
- [ ] Narrow `lib.rs` exports — expose only `Minigraf`, `WriteTransaction`, and query/result types; hide `PersistentFactStorage`, `FileHeader`, `PAGE_SIZE`, `Repl`, `Wal`, etc.
- [x] Remove dead `clap` dependency ✅

### Crate Metadata (`Cargo.toml`)
- [x] `repository`, `keywords`, `categories`, `readme`, `documentation` fields set
- [ ] Verify `description` is accurate and compelling (Phase 7.8)

### Documentation (Phase 7.8)
- [ ] All public API items have rustdoc comments with examples
- [ ] `README.md` quick-start example compiles and runs
- [ ] `CHANGELOG.md` up to date

### Quality Gates (Phase 7.8)
- [ ] `cargo test` passes on Linux, macOS, Windows (CI matrix)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo doc --no-deps` builds without warnings
- [ ] No `unwrap()`/`expect()` in library code paths (tests and binary are fine)

### Versioning
- [ ] Publish as `0.x` — no backwards-compat promise until v1.0.0 (Phase 7.8)
- [ ] Stable API target is v1.0.0 — after Phase 8 cross-platform work

## Code of Conduct

This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you agree to uphold it.
