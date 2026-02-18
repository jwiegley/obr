# CLAUDE.md — beads_rust development guide

> See `AGENTS.md` for full agent workflow rules, safety guidelines, and tool references.

## Quick reference

- **Binary name:** `br` (beads_rust)
- **Edition:** Rust 2024 (requires nightly toolchain)
- **Unsafe code:** Forbidden (`#![forbid(unsafe_code)]` via crate lints)
- **Clippy:** pedantic + nursery, deny warnings

## Build environment

Use Nix for a reproducible development shell:

```bash
nix develop                        # Enter dev shell with nightly Rust
nix develop --command cargo build  # Build without entering shell
nix build                          # Build the br binary via Nix
```

The flake provides nightly Rust via `rust-overlay`. No manual toolchain setup needed.

If not using Nix, ensure `rust-toolchain.toml` is respected (channel = nightly).

## Essential commands

```bash
cargo build                                    # Build
cargo test --lib --bins                        # Unit tests (781 tests)
cargo test                                     # All tests (conformance tests need Go bd binary)
cargo clippy --all-targets -- --deny warnings  # Lint check
cargo fmt --check                              # Format check
```

## Project structure

| Path | Purpose |
|------|---------|
| `src/main.rs` | CLI entry point, 35+ subcommand dispatch |
| `src/storage/sqlite.rs` | Core SQLite engine (~5K lines) |
| `src/sync/mod.rs` | JSONL import/export engine (~5K lines) |
| `src/model/mod.rs` | Issue, Dependency, Comment, Event types |
| `src/error/` | 3-layer error system: BeadsError -> StructuredError -> ErrorCode |
| `src/cli/commands/` | Individual subcommand implementations |
| `src/validation/mod.rs` | Input validation rules |
| `tests/` | Integration, conformance, property-based, regression tests |

## Architecture notes

- **SQLite + JSONL hybrid:** SQLite for speed, JSONL for git-based sync
- **Non-invasive:** `br` never executes git commands automatically
- **Content-addressed dedup:** SHA-256 hashes prevent duplicate issues across sync
- **4-step mutation protocol:** All writes go through `mutate()` with `FnOnce(&Transaction, &mut MutationContext)`
- **Output modes:** Rich (TTY), Plain (pipe), JSON (`--json`/`--robot`), Quiet (`--quiet`)

## Issue tracking with beads

This project tracks its own issues via beads (`.beads/` directory):

```bash
br ready                    # Show unblocked work
br list --status=open       # All open issues
br show <id>                # Issue details
br create --title="..." --type=task --priority=2
br close <id>               # Mark complete
br sync --flush-only        # Export to JSONL (then git add/commit manually)
```

## Key conventions

- Priority uses integers: P0=critical, P1=high, P2=medium, P3=low, P4=backlog
- Default branch is `main` (never `master`)
- After code changes, always run `cargo clippy --all-targets -- --deny warnings`
- Conformance tests (`tests/conformance.rs`) compare `br` against the Go `bd` binary and require `bd` to be installed
