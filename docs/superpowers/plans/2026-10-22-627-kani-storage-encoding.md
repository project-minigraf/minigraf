# Kani proofs of the storage encodings (#627) — Plan

Spec: `docs/superpowers/specs/2026-10-22-kani-storage-encoding-design.md` (PR #689).
Test-only: no library, `.graph` format or WAL change.

| PR | Steps | Branch | Base |
|---|---|---|---|
| 1 | 1–8: harnesses, lint, workflow, docs, this plan | `test/627-kani` | `v3` |
| 2 | 9–13: `encode_value` harnesses, lint, workflow, docs | `test/627-kani-v2` | `main` |

PR 1 says "Closes #627" (a `v3` test-only change closes on merge). PR 2 says "Refs #627".
Each PR is reviewed with Codex, CI is owned until green, and neither merges without the
user's go-ahead (`v3`: merge commit; `main`: squash).

Kani runs only on the bench machine (the laptop OOMs). Harness names are fully qualified
with `--exact`; `--verbose` crashes the Kani compiler on this crate, use
`-Z unstable-options --cbmc-args --verbosity 9` instead.

## Deviations from the spec

1. **`--output-format terse` is added to the Kani arguments** (spec §3.3, §4). Kani 0.68.0
   refuses `-j` without it (`Conflicting options: --jobs requires
   --output-format=terse`); found on the first bench-machine run of step 4.

Clarifications:

1. **Test counts do not change.** Kani harnesses are not `#[test]`s, so the 1321 count in
   `CLAUDE.md` and `docs/TEST_COVERAGE.md` stays. `TEST_COVERAGE.md` gets a separate Kani
   section with the 12 (`v3`) or 5 (`main`) harnesses.
2. **The lint table is `[workspace.lints.rust]`.** The package already opts in with
   `[lints] workspace = true` on both branches, so the check-cfg entry goes next to
   `[workspace.lints.clippy]`. `fuzz/` and the golden generators are outside the
   workspace and never see `cfg(kani)`.

## PR 1 — `v3`

1. **Rebase.** `test/627-kani` onto `origin/v3` (done 2026-10-22: 75a3faca; `keys.rs`,
   `index.rs`, `graph/types.rs` unchanged). Harness files as prototyped:
   `src/storage/keys/verification.rs` (7), `src/storage/index/verification.rs` (5),
   `#[cfg(kani)] mod verification;` in `keys.rs` (before `mod tests`) and `index.rs`
   (before `mod tests`).
2. **Lint.** In `Cargo.toml`, before `[workspace.lints.clippy]`:

   ```toml
   [workspace.lints.rust]
   # Kani proof harnesses (#627): `cargo kani` sets cfg(kani).
   unexpected_cfgs = { level = "warn", check-cfg = ['cfg(kani)'] }
   ```

   Check: `cargo build` and `cargo clippy --all-targets` print no
   `unexpected cfg condition name: kani`; `cargo test --lib` passes.
3. **Workflow** `.github/workflows/kani.yml`, modelled on `grammar-verifier.yml`:
   - `run-name` and `KANI_REF` from `workflow_dispatch` `inputs.ref`, else from the cron
     string (`"0 6 * * *"` main, `"0 18 * * *"` v3; offset from the grammar verifier's
     05:30/17:30), else for `pull_request` the PR head (checkout default).
   - `pull_request` `paths:` `src/storage/keys.rs`, `src/storage/keys/**`,
     `src/storage/index.rs`, `src/storage/index/**`, `src/graph/types.rs`, `Cargo.toml`,
     `.github/workflows/kani.yml`.
   - `permissions: contents: read, issues: write`; `timeout-minutes: 30`.
   - Steps: `actions/checkout@v4` (with `ref: KANI_REF` only when not a PR);
     `model-checking/kani-github-action@v1.1` with `kani-version: 0.68.0`,
     `args: --lib -j --output-format terse -Z unstable-options --harness-timeout 10m`. The action has no
     output capture, so the failure report links the run instead of tailing a log.
   - `Record Kani failure` step, `if: failure() && github.event_name == 'schedule'`,
     opens or comments on `Nightly Kani failure (<branch>)` with run URL, commit and the
     reproduce command `cargo kani --lib -j`.
   - Check: `actionlint` clean; after push, the PR run is green with 12 harnesses
     reported `VERIFICATION:- SUCCESSFUL`. Dispatch and schedule need the file on the
     default branch, so the nightly path is first exercised after PR 2 merges (step 13).
4. **Bench-machine run.** Copy the rebased tree to the bench machine and run
   `cargo kani --lib -j --output-format terse -Z unstable-options --harness-timeout 10m`.
   Result 2026-10-22: 12 successes, 40 s wall, 1.1 GB peak RSS. Record the summary line in the PR body.
5. **Mutation check (not committed).** On the bench machine, break each encoder once
   (`put_int` drops the sign flip; `float_bits` skips the negative transform;
   `put_valid_to` encodes `i64::MAX` like any other value; `encode_value` Ref writes
   the UUID bytes reversed) and confirm the matching harness fails. Revert. Result 2026-10-22: one mutation per
   harness, all 12 fail.
6. **Docs.**
   - `docs/TEST_COVERAGE.md`: a `### Kani proofs (#627)` section under Covered Areas,
     the two harness tables from spec §3.1/§3.2 (names and properties, no timings),
     what is left out (spec §5, one paragraph), and the run command; reproduce line in
     Reproducing.
   - `CLAUDE.md`: module list — `index.rs` and the `keys.rs` entry note
     `verification.rs` (`#[cfg(kani)]`, encoder proofs, #627); Build and Run gets
     `cargo kani --lib -j` with "bench machine or CI only: needs ~27 GB if a decoder
     harness is added".
   - `CHANGELOG.md` `[Unreleased]`: a Testing entry — Kani proves the key and value
     encoders order-preserving (#627); test-only.
   - `ROADMAP.md`: tick #627 if listed. No docs-site change (no user-facing behaviour).
7. **Commit** the plan with PR 1 (`docs(plan): ...`), then `test(storage): Kani proofs
   of the key and value encoders (#627)`, `ci: nightly and PR Kani workflow (#627)`,
   `docs: ...`.
8. **PR, Codex review, CI.** `codex exec -s read-only ... < /dev/null`; verify findings
   against the code; fix; wait for green; ask to merge.

## PR 2 — `main` backport

9. Branch `test/627-kani-v2` from `origin/main` in `.claude/worktrees/627-kani-v2`.
   Copy `src/storage/index/verification.rs` unchanged and add `#[cfg(kani)] mod
   verification;` before `#[cfg(test)] mod tests` in `src/storage/index.rs`. Confirm
   `encode_value`'s type bytes match (`Null 0x00`, `Boolean 0x01`, `Integer 0x02`,
   `Float 0x03`, `Ref 0x06`) and that `index.rs` imports `Value` so `use super::*`
   resolves.
10. Same `Cargo.toml` lint (step 2), same `kani.yml` (step 3) byte-for-byte, so the
    copy on `main` that drives both crons matches `v3`.
11. Bench-machine run: 5 successes. Mutation check on the Ref harness.
12. Docs as step 6 with 5 harnesses; `CHANGELOG.md` `[Unreleased]` on `main`. The PR body
    says the 7 `keys.rs` harnesses are left out because `main` has no `keys.rs`, and that
    the NaN position is #688, not asserted.
13. Codex review, CI green, ask to merge. After merge, dispatch `kani.yml` once with
    `ref: main` and once with `ref: v3` to exercise the nightly path; both green.

## After both merge

- Delete the bench-machine prototype dirs: `~/Work/AMC/Minigraf/kani-627`, `kani-627b`,
  `kani-627-logs`, `kani-627b-logs`, `kani-627-run.sh`, `kani-627-par.sh`,
  `kani-627b-par.sh`.
- Ask whether to uninstall Kani from the laptop (`~/.kani`, 485 MB, `cargo-kani`).
- Remove the worktrees; update memory (`project_627_kani_next_session.md`, issue roadmap).
