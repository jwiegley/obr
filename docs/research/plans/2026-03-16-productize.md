# Productize obr Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make obr production-ready with proper licensing, documentation, pre-commit hooks, CI alignment, fuzz testing, coverage enforcement, and performance regression gates.

**Architecture:** Replace the existing MIT license and third-party README with John Wiegley's BSD 3-clause license and voice. Enhance flake.nix with `checks` output so `nix flake check` runs all quality gates. Add lefthook for parallel pre-commit hooks. Set up cargo-fuzz, Miri, and coverage thresholds.

**Tech Stack:** Rust (nightly), Nix flakes, lefthook, cargo-fuzz, cargo-llvm-cov, Miri, criterion, GitHub Actions

---

## Chunk 1: Foundation Files

### Task 1: Replace LICENSE with BSD 3-Clause

**Files:**
- Delete: `LICENSE`
- Create: `LICENSE.md`

- [ ] **Step 1: Create LICENSE.md with BSD 3-clause text**

Match the Ledger project's license format. Year range: 2026 (all commits are from 2026).

- [ ] **Step 2: Remove old LICENSE file**

```bash
git rm LICENSE
```

- [ ] **Step 3: Update Cargo.toml license field**

Change `license = "MIT"` to `license = "BSD-3-Clause"`.

- [ ] **Step 4: Commit**

```bash
git add LICENSE.md Cargo.toml
git commit -m "chore: replace MIT license with BSD 3-Clause"
```

### Task 2: Rewrite README.md in John's Voice

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Rewrite README.md**

Complete rewrite in John Wiegley's voice per the johnw skill. Keep technical content accurate but reframe the narrative. Remove Jeffrey Emanuel references, badges to Dicklesworthstone repo, curl-pipe-bash installer, and AI slop patterns.

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: rewrite README in maintainer voice"
```

---

## Chunk 2: Nix Infrastructure

### Task 3: Enhance flake.nix with Checks and Dev Tools

**Files:**
- Modify: `flake.nix`

- [ ] **Step 1: Add checks output to flake.nix**

Add `checks.<system>` with:
- `formatting` - runs `cargo fmt --check`
- `clippy` - runs `cargo clippy --all-targets -- --deny warnings`
- `tests` - runs `cargo test --lib --bins`
- `build` - the existing package build

- [ ] **Step 2: Add dev tools to devShell**

Add to devShell packages:
- `lefthook` (pre-commit hooks)
- `cargo-llvm-cov` (coverage)
- `cargo-fuzz` (fuzz testing, if available in nixpkgs)
- `cargo-tarpaulin` (coverage, Linux only)
- `cargo-audit` (security)
- `shellcheck` (shell script linting)
- `shfmt` (shell script formatting)

- [ ] **Step 3: Verify `nix flake check` passes**

```bash
nix flake check --no-warn-dirty
```

- [ ] **Step 4: Commit**

```bash
git add flake.nix flake.lock
git commit -m "nix: add flake checks and dev tools"
```

---

## Chunk 3: Pre-commit Hooks

### Task 4: Create lefthook.yml

**Files:**
- Create: `lefthook.yml`

- [ ] **Step 1: Create lefthook.yml with parallel pre-commit hooks**

Hooks (all run in parallel):
- `fmt-check` - `cargo fmt --check`
- `clippy` - `cargo clippy --all-targets -- --deny warnings`
- `test` - `cargo test --lib --bins`
- `shellcheck` - lint shell scripts
- `shfmt` - check shell script formatting
- `nix-check` - `nix flake check --no-warn-dirty`

- [ ] **Step 2: Verify lefthook runs**

```bash
lefthook run pre-commit
```

- [ ] **Step 3: Commit**

```bash
git add lefthook.yml
git commit -m "chore: add lefthook pre-commit hooks"
```

---

## Chunk 4: Fuzz Testing

### Task 5: Set Up Cargo-Fuzz

**Files:**
- Create: `fuzz/Cargo.toml`
- Create: `fuzz/fuzz_targets/fuzz_jsonl_import.rs`
- Create: `fuzz/fuzz_targets/fuzz_issue_create.rs`

- [ ] **Step 1: Initialize cargo-fuzz structure**

Create fuzz directory with targets for JSONL parsing and issue creation (the two main input boundaries).

- [ ] **Step 2: Verify fuzz targets compile**

```bash
cargo +nightly fuzz list
```

- [ ] **Step 3: Commit**

```bash
git add fuzz/
git commit -m "test: add fuzz testing targets"
```

---

## Chunk 5: Coverage and Performance Gates

### Task 6: Add Coverage Threshold Script

**Files:**
- Create: `scripts/coverage-check.sh`

- [ ] **Step 1: Create coverage check script**

Script that runs `cargo llvm-cov` and fails if coverage drops below baseline stored in `coverage-baseline.txt`.

- [ ] **Step 2: Add coverage baseline file**

- [ ] **Step 3: Commit**

### Task 7: Add Performance Regression Gate Script

**Files:**
- Modify: `scripts/bench_regression.sh` (or create new)

- [ ] **Step 1: Create/update bench regression script**

Use existing `scripts/check_regression.py` pattern with 5% threshold.

- [ ] **Step 2: Commit**

---

## Chunk 6: GitHub Actions Alignment

### Task 8: Update CI to Mirror Lefthook

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Fix binary name references**

Replace `br` with `obr` in artifact paths, version checks.

- [ ] **Step 2: Add nix flake check job**

- [ ] **Step 3: Ensure all lefthook checks are represented in CI**

- [ ] **Step 4: Commit**

---

## Chunk 7: Shell Script Quality

### Task 9: Lint and Format Shell Scripts

**Files:**
- Modify: All `.sh` files in `scripts/`

- [ ] **Step 1: Run shellcheck on all scripts, fix issues**

- [ ] **Step 2: Run shfmt to standardize formatting**

- [ ] **Step 3: Commit**
