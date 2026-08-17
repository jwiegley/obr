# Phase 0 — Upstream baseline (no-NEW-failures reference)

**Recorded:** 2026-08-06. **Tree:** `upstream/main` = `d1fb0d3a` (beads_rust v0.2.22), worktree `/tmp/obr-baseline` (regenerable: `git worktree add /tmp/obr-baseline upstream/main`).

Everything below is **pre-existing upstream breakage**. Per plan §1/§7.3, later gates compare against these inventories; do not chase these failures, do not hide behind them.

## Canonical gate toolchain

```
nix shell 'github:nix-community/fenix/a7c3ef79859bf024cdc865a0ed8f14a4e3f31099#complete.toolchain' --command <cmd>
# rustc 1.99.0-nightly (7608eb7b0 2026-08-05), clippy 0.1.99
```

Use this exact pin for every phase gate. Rationale (all VERIFIED on this host):

1. Fork's nix dev shell pins nightly 1.95.0 (2026-02-17) — cannot build upstream deps (`sysinfo 0.39.6` needs then-unstable `cfg_select`).
2. Host profile stable 1.97.1 builds the tree, but **`cargo clippy`/`cargo fmt` are unusable machine-wide**: `$CARGO_HOME/bin/{cargo-clippy,cargo-fmt}` are rustup shims resolved *before* PATH, dispatching to rustup's stale Feb-2026 nightly. **Always invoke `cargo-clippy` (the binary) directly inside the nix shell**, never `cargo clippy`.
3. Upstream's own flake devShell is broken on Darwin, twice: references removed `darwin.apple_sdk.frameworks.*` stubs (`flake.nix:110-112`), and crane's eval-time `vendorCargoDeps` cannot find `Cargo.lock` because `combinedSrc` nests sources under `beads_rust/` while `src` points at the root. Both feed R3/Phase 4 (fork's flake rework carries forward).

## Gate results at baseline

| Gate | Command | Result |
|---|---|---|
| G-build | `cargo build` | **PASS** (~145s cold) |
| G-build-locked | `cargo build --locked` | **PASS** — upstream's committed `Cargo.lock` is valid; org2jsonl must be added without breaking it |
| G-lint | `cargo-clippy --all-targets -- -D warnings` | **FAIL: 85 errors** in `beads_rust` lib (pedantic/nursery violations under clippy 0.1.99). Gate is therefore **no-NEW-warnings** vs `baseline/v3-clippy-lints-baseline.txt` |
| G-unit | `cargo test --lib --bins` | **FAIL: 266 failed / 2440 passed / 9 ignored**, plus **4 aborting tests** (below). Parallel runs die early (exit 5) and hide most failures — see procedure |
| G-e2e `e2e_basic_lifecycle` | `cargo test --test e2e_basic_lifecycle` | 193 passed, **3 failed** |
| G-e2e `e2e_sync_git_safety` | `cargo test --test e2e_sync_git_safety` | 166 passed, **4 failed** — incl. upstream's own file-allowlist tests (`integration_sync_only_touches_allowed_files` etc.), the exact class plan-landmine §7.5 flags for Org awareness |
| G-e2e `jsonl_import_export` | `cargo test --test jsonl_import_export` | 158 passed, **23 failed** — core import/export area unhealthy at baseline |

E2e failure name-lists are identical under stable 1.97.1 and nightly 1.99.0 (VERIFIED by diff) — deterministic, not toolchain-dependent.

## The four process-aborting unit tests

All in `cli::commands::doctor::tests`, all `execute_repair_indexes_*`; each kills the harness with exit status 5, silently truncating the run:

```
execute_repair_indexes_clears_stale_sidecar_snapshots_from_previous_run
execute_repair_indexes_dry_run_skips_mutation
execute_repair_indexes_quotes_names_that_need_quoting
execute_repair_indexes_succeeds_against_healthy_db_and_retains_snapshot
```

## G-unit comparison procedure (use at every gate)

1. Fast check: `cargo test --lib --bins` — compare visible FAILED names against `baseline/v3-unit-failures-complete.txt`; any name not in the baseline list = NEW failure = red gate.
2. Full check (Phase 7 at minimum): serial run with the four aborters skipped:
   `cargo test --lib --bins -- --test-threads=1 --skip <each aborter>` → diff FAILED set against `baseline/v3-unit-failures-complete.txt` (266 names). A new aborter (harness dies elsewhere) is also a red gate.

## Inventory files (this directory, `baseline/`)

- `v3-unit-failures-complete.txt` — 266 unit failures (complete, serial, aborters skipped)
- `v3-unit-aborters.txt` — the 4 aborting tests
- `v3-e2e_basic_lifecycle-failures-baseline.txt`, `v3-e2e_sync_git_safety-failures-baseline.txt`, `v3-jsonl_import_export-failures-baseline.txt`
- `v3-clippy-lints-baseline.txt` — lint kind/count inventory (full log in session scratchpad `baseline/v3-clippy.log`)

Raw logs: `/private/tmp/claude-501/-Users-johnw-src-obr/880cafc9-25ec-4128-bebf-087c9b355d72/scratchpad/baseline/` (ephemeral; inventories above are the durable reference).

## Deviations made to the baseline worktree (throwaway only)

- `flake.nix`: removed the three `darwin.apple_sdk.frameworks.*` lines (eval fix; documents the Phase-4 work).
- `git add -N Cargo.lock` + generated `flake.lock` during diagnosis. None of this affects gate results (gates bypass the flake via `nix shell`).
