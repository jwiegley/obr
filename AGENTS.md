# AGENTS.md — obr

> Guidelines for AI coding agents working in this Rust codebase.

---

## RULE 0 - THE FUNDAMENTAL OVERRIDE PREROGATIVE

If I tell you to do something, even if it goes against what follows below, YOU MUST LISTEN TO ME. I AM IN CHARGE, NOT YOU.

---

## RULE NUMBER 1: NO FILE DELETION

**YOU ARE NEVER ALLOWED TO DELETE A FILE WITHOUT EXPRESS PERMISSION.** Even a new file that you yourself created, such as a test code file. You have a horrible track record of deleting critically important files or otherwise throwing away tons of expensive work. As a result, you have permanently lost any and all rights to determine that a file or folder should be deleted.

**YOU MUST ALWAYS ASK AND RECEIVE CLEAR, WRITTEN PERMISSION BEFORE EVER DELETING A FILE OR FOLDER OF ANY KIND.**

---

## Irreversible Git & Filesystem Actions — DO NOT EVER BREAK GLASS

1. **Absolutely forbidden commands:** `git reset --hard`, `git clean -fd`, `rm -rf`, or any command that can delete or overwrite code/data must never be run unless the user explicitly provides the exact command and states, in the same message, that they understand and want the irreversible consequences.
2. **No guessing:** If there is any uncertainty about what a command might delete or overwrite, stop immediately and ask the user for specific approval. "I think it's safe" is never acceptable.
3. **Safer alternatives first:** When cleanup or rollbacks are needed, request permission to use non-destructive options (`git status`, `git diff`, `git stash`, copying to backups) before ever considering a destructive command.
4. **Mandatory explicit plan:** Even after explicit user authorization, restate the command verbatim, list exactly what will be affected, and wait for a confirmation that your understanding is correct. Only then may you execute it—if anything remains ambiguous, refuse and escalate.
5. **Document the confirmation:** When running any approved destructive command, record (in the session notes / final response) the exact user text that authorized it, the command actually run, and the execution time. If that record is absent, the operation did not happen.

---

## Git Branch

The default branch is `main`, and it is the only branch on `origin`. A `master`
branch exists on the `upstream` remote only; nothing in this fork mirrors to it.

---

## CI/Release Workflow Supply-Chain Policy

For any `.github/workflows/` edit, use
[`docs/CI_SUPPLY_CHAIN.md`](docs/CI_SUPPLY_CHAIN.md) as the canonical policy.
It defines the immutable external GitHub Action pin inventory, upstream update
audit, workflow-fragment harnesses, branch-trigger expectations, and proof
commands for workflow changes.

Important boundaries:

- `obr` never performs workflow git operations, releases, pull requests, network
  dispatches, or upstream lookups automatically.
- Whole-crate `cargo check --all-targets` and
  `cargo clippy --all-targets -- -D warnings` are required when Rust code
  changes.
- Run `git diff --check`, `actionlint` when available, the relevant workflow
  harnesses, and `ubs` on changed workflow-related files before committing.

---

## Toolchain: Rust & Cargo

We only use **Cargo** in this project, NEVER any other package manager.

- **Edition:** Rust 2024 (nightly required — see `rust-toolchain.toml`)
- **Dependency versions:** Explicit versions for stability
- **Configuration:** Cargo.toml only (single crate, not a workspace)
- **Unsafe code:** Forbidden (`#![forbid(unsafe_code)]` via crate lints)

### Key Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` | CLI parsing with derive macros + shell completions |
| `fsqlite` + `fsqlite-types` + `fsqlite-error` | SQLite engine facade plus shared storage types/errors (path dependencies) |
| `serde` + `serde_json` | Issue serialization and JSONL export |
| `schemars` | JSON Schema generation for robot output |
| `chrono` | Timestamp parsing and RFC3339 formatting |
| `rich_rust` | Rich terminal output (panels, tables, colors) |
| `toon_rust` | TOON format support for token-efficient schema viewing |
| `crossterm` + `indicatif` | Terminal control and progress spinners |
| `anyhow` + `thiserror` | Error handling (anyhow for CLI, thiserror for typed errors) |
| `sha2` | Content hashing for deduplication |
| `regex` | Pattern matching for search and validation |
| `semver` | Semantic version parsing |
| `tracing` | Structured logging and diagnostics |

### Release Profile

The release build optimizes for binary size (this is a CLI tool for distribution):

```toml
[profile.release]
opt-level = "z"     # Optimize for size (lean binary for distribution)
lto = true          # Link-time optimization
codegen-units = 1   # Single codegen unit for better optimization
panic = "abort"     # Smaller binary, no unwinding overhead
strip = true        # Remove debug symbols
```

---

## Code Editing Discipline

### No Script-Based Changes

**NEVER** run a script that processes/changes code files in this repo. Brittle regex-based transformations create far more problems than they solve.

- **Always make code changes manually**, even when there are many instances
- For many simple changes: use parallel subagents
- For subtle/complex changes: do them methodically yourself

### No File Proliferation

If you want to change something or add a feature, **revise existing code files in place**.

**NEVER** create variations like:
- `mainV2.rs`
- `main_improved.rs`
- `main_enhanced.rs`

New files are reserved for **genuinely new functionality** that makes zero sense to include in any existing file. The bar for creating new files is **incredibly high**.

---

## Backwards Compatibility

Default to none. This is a personal fork with a single user, so prefer fixing
code directly over adding wrappers for deprecated APIs, and do not add
compatibility layers speculatively.

There is exactly one deliberate exception, and it is deliberately thin:
`src/legacy_compat.rs` keeps reading pre-rename ON-DISK WORKSPACE artifacts,
because the user's fork-era repositories may still hold them — the
`.beads`/`_beads` directories, `beads.db`, in-dir `beads.jsonl` / `issues.jsonl`
exports, merge/history/recovery/snapshot filenames, the Org drawer's
`BEADS_SCHEMA_VERSION`, `br.*.v1` schema ids, and the agents marker. Each read
warns once per process, and nothing is ever written under a legacy name.

Environment variables and user-config locations are explicitly NOT in that
surface: `OBR_*` and `~/.config/obr/config.yaml` are the only spellings obr
reads. The `BEADS_*`/`BD_*`/`BR_*` names and the `~/.config/{beads,bd}`,
`~/.beads` locations were removed outright rather than deprecated.

---

## Compiler Checks (CRITICAL)

**After any substantive code changes, you MUST verify no errors were introduced:**

```bash
# Check for compiler errors and warnings
cargo check --all-targets

# Check for clippy lints (pedantic + nursery are enabled)
cargo clippy --all-targets -- -D warnings

# Verify formatting
cargo fmt --check
```

If you see errors, **carefully understand and resolve each issue**. Read sufficient context to fix them the RIGHT way.

---

## Testing

### Testing Policy

Every module includes inline `#[cfg(test)]` unit tests alongside the implementation. Tests must cover:
- Happy path
- Edge cases (empty input, max values, boundary conditions)
- Error conditions

Integration and end-to-end tests live in the `tests/` directory.

### Unit Tests

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run tests for a specific module
cargo test storage
cargo test cli
cargo test sync
cargo test format
cargo test model
cargo test validation

# Run tests with all features enabled
cargo test --all-features
```

### Test Categories

| Directory / Pattern | Focus Areas |
|---------------------|-------------|
| `src/` (inline `#[cfg(test)]`) | Unit tests for each module: model, storage, sync, config, error, format, util, validation |
| `tests/e2e_*.rs` | End-to-end CLI tests: lifecycle, labels, deps, sync, history, search, comments, epics, workspaces, errors, completions |
| `tests/conformance*.rs` | Go/Rust parity: schema compatibility, text output matching, edge cases, labels+comments, workflows |
| `tests/storage_*.rs` | Storage layer: CRUD, list filters, ready queries, deps, history, blocked cache, export atomicity, invariants, ID/hash parity |
| `tests/proptest_*.rs` | Property-based tests: ID generation, hash determinism, time parsing, validation rules |
| `tests/repro_*.rs` | Regression tests: specific bugs reproduced and prevented |
| `tests/jsonl_import_export.rs` | JSONL round-trip fidelity |
| `tests/markdown_import.rs` | Markdown import parsing |
| `benches/storage_perf.rs` | Storage operation benchmarks (criterion) |

### Test Fixtures

Shared test fixtures live in `tests/fixtures/` and `tests/common/` for reusable test harness helpers (temp DB creation, test data builders).

---

## Third-Party Library Usage

If you aren't 100% sure how to use a third-party library, **SEARCH ONLINE** to find the latest documentation and current best practices.

---

## obr — This Project

**This is the project you're working on.** obr is a dependency-aware issue tracker CLI (`obr`) that keeps a SQLite cache in `.obr/` and exports a tracked Org surface (`PLAN.org`) for git-based sync. It is a fork of beads_rust, itself a Rust port of the classic Go beads issue tracker (`bd`), and is deliberately non-invasive (no automatic git operations, no daemons, no hooks).

### What It Does

Provides lightweight issue tracking with dependency graphs, priority-based triage, content-addressed deduplication, and multiple output modes (rich terminal, plain text, JSON, TOON). Designed specifically for AI coding agents to select "ready work," manage task dependencies, and coordinate via structured robot output.

### Architecture

```
CLI (clap derive)
    │
    ├── Commands ────── 35+ subcommands (create, list, show, close, dep, sync, ...)
    │                       │
    │                       ▼
    ├── Storage ─────── SQLite (fsqlite stack)
    │                       │
    │                       ├── Schema (migrations, JSONL ↔ SQLite sync)
    │                       ├── Events (append-only audit log)
    │                       └── Queries (filtered list, ready, search, graph)
    │
    ├── Sync ───────── JSONL import/export (git-friendly, no auto-git)
    │                       │
    │                       ├── Path resolution (.obr/ discovery)
    │                       └── History (snapshot restore, prune)
    │
    ├── Model ──────── Issue, Dependency, Comment, Event, Label
    │                       │
    │                       └── Content hashing (SHA-256 dedup)
    │
    ├── Config ─────── Layered config (file + env + CLI flags)
    │                       │
    │                       └── Routing (project-aware config resolution)
    │
    ├── Format ─────── Rich (panels, tables, colors), Plain, CSV, Markdown, Syntax
    │
    ├── Output ─────── Mode detection (TTY → Rich, pipe → Plain, --json → JSON)
    │                       │
    │                       └── Components (reusable output widgets)
    │
    ├── Validation ─── Input validation (titles, IDs, priorities, dates)
    │
    └── Error ──────── Structured errors with exit codes (BeadsError + ErrorCode)
```

### Project Structure

```
obr/
├── Cargo.toml                     # Single crate (not a workspace)
├── src/
│   ├── main.rs                    # CLI entry point, clap dispatch
│   ├── lib.rs                     # Library root, module declarations
│   ├── cli/
│   │   ├── mod.rs                 # CLI argument parsing, output mode detection
│   │   └── commands/              # 35+ subcommand implementations
│   ├── model/
│   │   └── mod.rs                 # Issue, Dependency, Comment, Event, Label types
│   ├── storage/
│   │   ├── mod.rs                 # Storage trait
│   │   ├── sqlite.rs              # SQLite backend (181KB — the core engine)
│   │   ├── schema.rs              # DDL migrations
│   │   ├── events.rs              # Append-only audit log
│   │   └── queries/               # Reusable query fragments
│   ├── sync/
│   │   ├── mod.rs                 # JSONL import/export (176KB)
│   │   ├── path.rs                # .obr/ directory discovery
│   │   └── history.rs             # Snapshot restore and prune
│   ├── config/
│   │   ├── mod.rs                 # Layered configuration
│   │   └── routing.rs             # Project-aware config resolution
│   ├── error/
│   │   ├── mod.rs                 # BeadsError enum
│   │   ├── structured.rs          # StructuredError with ErrorCode + exit codes
│   │   └── context.rs             # Error context helpers
│   ├── format/
│   │   ├── mod.rs                 # Format module root
│   │   ├── rich.rs                # Rich terminal output (panels, tables)
│   │   ├── text.rs                # Plain text formatting
│   │   ├── csv.rs                 # CSV export
│   │   ├── markdown.rs            # Markdown formatting
│   │   ├── syntax.rs              # Syntax highlighting
│   │   ├── theme.rs               # Color themes
│   │   ├── context.rs             # Format context (width, mode)
│   │   └── output.rs              # Output helpers
│   ├── output/
│   │   ├── mod.rs                 # Output mode detection (Rich/Plain/JSON/Quiet)
│   │   ├── context.rs             # Output context
│   │   ├── theme.rs               # Output theming
│   │   └── components/            # Reusable output widgets
│   ├── validation/
│   │   └── mod.rs                 # Input validation rules
│   ├── util/
│   │   ├── mod.rs                 # Utility module root
│   │   ├── id.rs                  # Hash-based short ID generation
│   │   ├── hash.rs                # SHA-256 content hashing
│   │   ├── time.rs                # Timestamp parsing/formatting
│   │   ├── progress.rs            # Progress spinners
│   │   └── markdown_import.rs     # Markdown file import
│   └── logging.rs                 # tracing-subscriber setup
├── tests/                         # Integration, conformance, property, regression tests
├── benches/                       # Criterion benchmarks
└── docs/                          # Architecture, CLI reference, troubleshooting
```

### Key Files by Module

| Module | Key Files | Purpose |
|--------|-----------|---------|
| `cli` | `cli/mod.rs` | Clap argument parsing, output mode detection, 66KB dispatch logic |
| `cli/commands` | `commands/*.rs` | 35+ subcommands: create, list, show, close, update, dep, sync, search, query, ready, graph, audit, etc. |
| `model` | `model/mod.rs` | `Issue`, `Dependency`, `Comment`, `Event`, `Label` types, content hashing, serde derives |
| `storage` | `storage/sqlite.rs` | Core SQLite engine (181KB): CRUD, filtered queries, dependency graph, search, events |
| `storage` | `storage/schema.rs` | DDL migrations, table creation, index management |
| `storage` | `storage/events.rs` | Append-only audit log for all issue mutations |
| `sync` | `sync/mod.rs` | JSONL import/export engine (176KB): merge, dedup, conflict resolution |
| `sync` | `sync/path.rs` | `.obr/` directory discovery and path resolution |
| `sync` | `sync/history.rs` | Snapshot-based history: restore, prune, diff |
| `config` | `config/mod.rs` | Layered config: file + env vars + CLI flags, project-aware resolution |
| `error` | `error/structured.rs` | `StructuredError` with `ErrorCode` enum and deterministic exit codes |
| `validation` | `validation/mod.rs` | Input validation: titles, IDs, priorities, dates, labels |
| `util` | `util/id.rs` | Hash-based short ID generation (e.g., `proj-abc12`) |
| `util` | `util/hash.rs` | SHA-256 content hashing for deduplication |
| `format` | `format/rich.rs` | Rich terminal output via `rich_rust` (panels, tables, colors) |

### Feature Flags

```toml
[features]
default = []
mcp = ["dep:fastmcp-rust"]          # MCP server surface
```

### Core Types Quick Reference

| Type | Purpose |
|------|---------|
| `Issue` | Core data type: title, description, status, priority, type, labels, timestamps, content hash |
| `Dependency` | Directed edge: `from` blocks `to`, with optional label |
| `Comment` | Timestamped comment attached to an issue |
| `Event` | Append-only audit entry (created, updated, closed, reopened, etc.) |
| `Label` | Categorization tag with optional color |
| `BeadsError` | Unified error enum (thiserror-derived) with structured variants |
| `ErrorCode` | Deterministic exit code mapping (e.g., `IssueNotFound` = exit 3) |
| `StructuredError` | JSON-serializable error with code, message, context |
| `OutputMode` | Enum: `Rich`, `Plain`, `Json`, `Toon`, `Quiet` — auto-detected from flags, env, and terminal state |

### Key Design Decisions

- **Non-invasive by design** — `obr` NEVER executes git commands automatically; all git operations are explicit user actions
- **SQLite + JSONL hybrid** — Primary storage is SQLite for speed; JSONL export for git-based sync and human readability
- **Content-addressed deduplication** — SHA-256 content hashes prevent duplicate issues across sync boundaries
- **Hash-based short IDs** — e.g., `proj-abc12` (not auto-increment integers) for stable cross-repo references
- **Go parity** — Rust `obr` produces identical output to Go `bd` for equivalent inputs; conformance tests validate this
- **Schema compatibility** — Database schema matches Go beads for potential cross-tool usage
- **Multiple output modes** — Rich (TTY), Plain (pipe/NO_COLOR), JSON (--json/--robot), Quiet (--quiet) — auto-detected
- **Append-only audit log** — Every mutation recorded in events table for full traceability
- **Layered configuration** — File + env vars + CLI flags with project-aware routing
- **`unsafe_code = "forbid"`** — Zero unsafe code via crate-level lint
- **`clippy::pedantic` + `clippy::nursery`** — Maximum lint strictness enabled

---

## Sync Safety Maintenance

When modifying sync-related code (`src/sync/`, `src/cli/commands/sync.rs`), you MUST follow the maintenance checklist:

**See: [`docs/SYNC_MAINTENANCE_CHECKLIST.md`](docs/SYNC_MAINTENANCE_CHECKLIST.md)**

Quick summary:
1. **No git operations** — Static check: `grep -rn 'Command::new.*git' src/sync/`
2. **Path allowlist** — Verify only `.obr/` files and the tracked `PLAN.org` surface are touched
3. **Run safety tests** — `cargo test e2e_sync --release`
4. **Review logs** — Check for unexpected safety events
5. **Update docs** — If behavior changed

Related documentation:
- [SYNC_SAFETY.md](docs/SYNC_SAFETY.md) — User-facing safety model
- [E2E_SYNC_TESTS.md](docs/E2E_SYNC_TESTS.md) — Test execution guide
- [SYNC_SAFETY_INVARIANTS.md](docs/SYNC_SAFETY_INVARIANTS.md) — Technical invariants

---

## Output Modes

obr supports multiple output modes for different use cases:

| Mode | When Active | Description |
|------|-------------|-------------|
| **Rich** | TTY with colors | Colored panels, tables, styled text |
| **Plain** | `NO_COLOR` env or `--no-color` | Text output without ANSI codes |
| **JSON** | `--json` or `--robot` | Machine-readable structured output |
| **Toon** | `--format toon`, `OBR_OUTPUT_FORMAT=toon`, or `TOON_DEFAULT_FORMAT=toon` | Token-efficient structured output |
| **Quiet** | `--quiet` or `-q` | Minimal output |

### Mode Detection

The output mode is automatically detected:

1. `--json` or `--robot` flags → **JSON mode**
2. `--quiet` flag → **Quiet mode**
3. `OBR_OUTPUT_FORMAT` env var or `TOON_DEFAULT_FORMAT` fallback env var can force **JSON** or **Toon** mode
4. `NO_COLOR` env var or `--no-color` → **Plain mode**
5. Non-TTY stdout (piped output) → **Plain mode**
6. Otherwise → **Rich mode** (default for interactive terminals)

See [docs/AGENT_INTEGRATION.md](docs/AGENT_INTEGRATION.md) for agent-oriented
format defaults and `TOON_DEFAULT_FORMAT` examples.

### For Coding Agents

**CRITICAL:** Always use `--json` or `--robot` flags when parsing obr output programmatically.

```bash
# CORRECT - stable, parseable output
obr list --json | jq '.issues[0]'
obr ready --robot

# WRONG - output format may vary based on terminal state
obr list | head -1
```

JSON mode guarantees:
- Stable schema (changes are versioned and documented)
- No ANSI escape codes
- Clean stdout (diagnostics go to stderr)
- Exit codes for success/failure

Schema discovery:
- `obr schema all --format json` emits JSON Schema documents for the main robot outputs
- `obr schema issue-details --format toon` for token-efficient schema viewing

### MCP Serve for Agents

`obr serve` exposes the same issue tracker as an MCP server for agents that can
use MCP tools/resources/prompts instead of shelling out. It is optional and only
exists in binaries built with the `mcp` feature:

```bash
cargo build --release --features mcp
RUST_LOG=error ./target/release/obr serve --actor "${AGENT_NAME:-mcp}"
```

Transport is stdio. Configure the MCP client to launch `obr serve`; do not expect
a TCP port or background daemon. Available tools are `list_issues`, `show_issue`,
`create_issue`, `update_issue`, `close_issue`, `manage_dependencies`, and
`project_overview`. Resources include `obr://project/info`,
`obr://issues/{id}`, `obr://schema`, `obr://labels`,
`obr://issues/ready`, `obr://issues/blocked`,
`obr://issues/in_progress`, `obr://issues/deferred`,
`obr://issues/bottlenecks`, `obr://graph/health`, and
`obr://events/recent`. Guided prompts are `triage`, `status_report`,
`plan_next_work`, and `polish_backlog`.

Safety model: MCP serve uses the same local SQLite/JSONL workspace as the CLI,
never runs git, and does not listen on the network. Mutating tools acquire the
workspace `.write.lock`, record audit events with `--actor`, and attempt the
normal JSONL auto-flush after successful writes.

---

## Issue Tracking Policy for This Repository

This repository does NOT use an issue tracker in-repo. Do not create an
`.obr/` (or legacy `.beads/`) workspace here, do not seed tracker data, and do not
adopt tracker-driven workflows from older revisions of this file. Use the
harness's built-in TODO/task functionality for session-level work tracking.

The product this repository builds is an issue tracker; its behavior in
*users'* repositories is documented in `docs/` and `README.md`. The product
never runs git commands and never uses a secondary tracking branch.
