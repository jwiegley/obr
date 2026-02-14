# AGENTS.md — beads_rust (br)

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

## Git Branch: ONLY Use `main`, NEVER `master`

**The default branch is `main`. The `master` branch exists only for legacy URL compatibility.**

- **All work happens on `main`** — commits, PRs, feature branches all merge to `main`
- **Never reference `master` in code or docs** — if you see `master` anywhere, it's a bug that needs fixing
- **The `master` branch must stay synchronized with `main`** — after pushing to `main`, also push to `master`:
  ```bash
  git push origin main:master
  ```

**If you see `master` referenced anywhere:**
1. Update it to `main`
2. Ensure `master` is synchronized: `git push origin main:master`

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
| `rusqlite` | SQLite storage (bundled, modern_sqlite features) |
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
| `self_update` | Self-update from GitHub releases (optional, feature-gated) |

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

We do not care about backwards compatibility—we're in early development with no users. We want to do things the **RIGHT** way with **NO TECH DEBT**.

- Never create "compatibility shims"
- Never create wrapper functions for deprecated APIs
- Just fix the code directly

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

## beads_rust (br) — This Project

**This is the project you're working on.** beads_rust is an agent-first, dependency-aware issue tracker CLI (`br`) that stores issues in SQLite with JSONL export for git-based sync. It is a Rust port of the classic Go beads issue tracker (`bd`), designed to be non-invasive (no automatic git operations, no daemons, no hooks).

### What It Does

Provides lightweight issue tracking with dependency graphs, priority-based triage, content-addressed deduplication, and multiple output modes (rich terminal, plain text, JSON, TOON). Designed specifically for AI coding agents to select "ready work," manage task dependencies, and coordinate via structured robot output.

### Architecture

```
CLI (clap derive)
    │
    ├── Commands ────── 35+ subcommands (create, list, show, close, dep, sync, ...)
    │                       │
    │                       ▼
    ├── Storage ─────── SQLite (rusqlite, bundled)
    │                       │
    │                       ├── Schema (migrations, JSONL ↔ SQLite sync)
    │                       ├── Events (append-only audit log)
    │                       └── Queries (filtered list, ready, search, graph)
    │
    ├── Sync ───────── JSONL import/export (git-friendly, no auto-git)
    │                       │
    │                       ├── Path resolution (.beads/ discovery)
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
beads_rust/
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
│   │   ├── path.rs                # .beads/ directory discovery
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
├── docs/                          # Architecture, CLI reference, troubleshooting
└── .beads/                        # Self-tracked issues (beads tracking beads)
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
| `sync` | `sync/path.rs` | `.beads/` directory discovery and path resolution |
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
default = ["self_update"]
self_update = ["dep:self_update"]   # Self-update from GitHub releases (rustls TLS, signature verification)
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
| `OutputMode` | Enum: `Rich`, `Plain`, `Json`, `Quiet` — auto-detected from terminal state |

### Key Design Decisions

- **Non-invasive by design** — `br` NEVER executes git commands automatically; all git operations are explicit user actions
- **SQLite + JSONL hybrid** — Primary storage is SQLite for speed; JSONL export for git-based sync and human readability
- **Content-addressed deduplication** — SHA-256 content hashes prevent duplicate issues across sync boundaries
- **Hash-based short IDs** — e.g., `proj-abc12` (not auto-increment integers) for stable cross-repo references
- **Go parity** — Rust `br` produces identical output to Go `bd` for equivalent inputs; conformance tests validate this
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
2. **Path allowlist** — Verify only `.beads/` files are touched
3. **Run safety tests** — `cargo test e2e_sync --release`
4. **Review logs** — Check for unexpected safety events
5. **Update docs** — If behavior changed

Related documentation:
- [SYNC_SAFETY.md](docs/SYNC_SAFETY.md) — User-facing safety model
- [E2E_SYNC_TESTS.md](docs/E2E_SYNC_TESTS.md) — Test execution guide
- [.beads/SYNC_SAFETY_INVARIANTS.md](.beads/SYNC_SAFETY_INVARIANTS.md) — Technical invariants

---

## Output Modes

br supports multiple output modes for different use cases:

| Mode | When Active | Description |
|------|-------------|-------------|
| **Rich** | TTY with colors | Colored panels, tables, styled text |
| **Plain** | `NO_COLOR` env or `--no-color` | Text output without ANSI codes |
| **JSON** | `--json` or `--robot` | Machine-readable structured output |
| **Quiet** | `--quiet` or `-q` | Minimal output |

### Mode Detection

The output mode is automatically detected:

1. `--json` or `--robot` flags → **JSON mode**
2. `--quiet` flag → **Quiet mode**
3. `NO_COLOR` env var or `--no-color` → **Plain mode**
4. Non-TTY stdout (piped output) → **Plain mode**
5. Otherwise → **Rich mode** (default for interactive terminals)

### For Coding Agents

**CRITICAL:** Always use `--json` or `--robot` flags when parsing br output programmatically.

```bash
# CORRECT - stable, parseable output
br list --json | jq '.[0]'
br ready --robot

# WRONG - output format may vary based on terminal state
br list | head -1
```

JSON mode guarantees:
- Stable schema (changes are versioned and documented)
- No ANSI escape codes
- Clean stdout (diagnostics go to stderr)
- Exit codes for success/failure

Schema discovery:
- `br schema all --format json` emits JSON Schema documents for the main robot outputs
- `br schema issue-details --format toon` for token-efficient schema viewing

---

## Beads (br) — Dependency-Aware Issue Tracking

Beads provides a lightweight, dependency-aware issue database and CLI (`br` - beads_rust) for selecting "ready work," setting priorities, and tracking status. It complements MCP Agent Mail's messaging and file reservations.

**Important:** `br` is non-invasive—it NEVER runs git commands automatically. You must manually commit changes after `br sync --flush-only`.

### Conventions

- **Single source of truth:** Beads for task status/priority/dependencies; Agent Mail for conversation and audit
- **Shared identifiers:** Use Beads issue ID (e.g., `br-123`) as Mail `thread_id` and prefix subjects with `[br-123]`
- **Reservations:** When starting a task, call `file_reservation_paths()` with the issue ID in `reason`

### Typical Agent Flow

1. **Pick ready work (Beads):**
   ```bash
   br ready --json  # Choose highest priority, no blockers
   ```

2. **Reserve edit surface (Mail):**
   ```
   file_reservation_paths(project_key, agent_name, ["src/**"], ttl_seconds=3600, exclusive=true, reason="br-123")
   ```

3. **Announce start (Mail):**
   ```
   send_message(..., thread_id="br-123", subject="[br-123] Start: <title>", ack_required=true)
   ```

4. **Work and update:** Reply in-thread with progress

5. **Complete and release:**
   ```bash
   br close 123 --reason "Completed"
   br sync --flush-only  # Export to JSONL (no git operations)
   ```
   ```
   release_file_reservations(project_key, agent_name, paths=["src/**"])
   ```
   Final Mail reply: `[br-123] Completed` with summary

### Mapping Cheat Sheet

| Concept | Value |
|---------|-------|
| Mail `thread_id` | `br-###` |
| Mail subject | `[br-###] ...` |
| File reservation `reason` | `br-###` |
| Commit messages | Include `br-###` for traceability |

---

## bv — Graph-Aware Triage Engine

bv is a graph-aware triage engine for Beads projects (`.beads/beads.jsonl`). It computes PageRank, betweenness, critical path, cycles, HITS, eigenvector, and k-core metrics deterministically.

**Scope boundary:** bv handles *what to work on* (triage, priority, planning). For agent-to-agent coordination (messaging, work claiming, file reservations), use MCP Agent Mail.

**CRITICAL: Use ONLY `--robot-*` flags. Bare `bv` launches an interactive TUI that blocks your session.**

### The Workflow: Start With Triage

**`bv --robot-triage` is your single entry point.** It returns:
- `quick_ref`: at-a-glance counts + top 3 picks
- `recommendations`: ranked actionable items with scores, reasons, unblock info
- `quick_wins`: low-effort high-impact items
- `blockers_to_clear`: items that unblock the most downstream work
- `project_health`: status/type/priority distributions, graph metrics
- `commands`: copy-paste shell commands for next steps

```bash
bv --robot-triage        # THE MEGA-COMMAND: start here
bv --robot-next          # Minimal: just the single top pick + claim command
```

### Command Reference

**Planning:**
| Command | Returns |
|---------|---------|
| `--robot-plan` | Parallel execution tracks with `unblocks` lists |
| `--robot-priority` | Priority misalignment detection with confidence |

**Graph Analysis:**
| Command | Returns |
|---------|---------|
| `--robot-insights` | Full metrics: PageRank, betweenness, HITS, eigenvector, critical path, cycles, k-core, articulation points, slack |
| `--robot-label-health` | Per-label health: `health_level`, `velocity_score`, `staleness`, `blocked_count` |
| `--robot-label-flow` | Cross-label dependency: `flow_matrix`, `dependencies`, `bottleneck_labels` |
| `--robot-label-attention [--attention-limit=N]` | Attention-ranked labels |

**History & Change Tracking:**
| Command | Returns |
|---------|---------|
| `--robot-history` | Bead-to-commit correlations |
| `--robot-diff --diff-since <ref>` | Changes since ref: new/closed/modified issues, cycles |

**Other:**
| Command | Returns |
|---------|---------|
| `--robot-burndown <sprint>` | Sprint burndown, scope changes, at-risk items |
| `--robot-forecast <id\|all>` | ETA predictions with dependency-aware scheduling |
| `--robot-alerts` | Stale issues, blocking cascades, priority mismatches |
| `--robot-suggest` | Hygiene: duplicates, missing deps, label suggestions |
| `--robot-graph [--graph-format=json\|dot\|mermaid]` | Dependency graph export |
| `--export-graph <file.html>` | Interactive HTML visualization |

### Scoping & Filtering

```bash
bv --robot-plan --label backend              # Scope to label's subgraph
bv --robot-insights --as-of HEAD~30          # Historical point-in-time
bv --recipe actionable --robot-plan          # Pre-filter: ready to work
bv --recipe high-impact --robot-triage       # Pre-filter: top PageRank
bv --robot-triage --robot-triage-by-track    # Group by parallel work streams
bv --robot-triage --robot-triage-by-label    # Group by domain
```

### Understanding Robot Output

**All robot JSON includes:**
- `data_hash` — Fingerprint of source beads.jsonl
- `status` — Per-metric state: `computed|approx|timeout|skipped` + elapsed ms
- `as_of` / `as_of_commit` — Present when using `--as-of`

**Two-phase analysis:**
- **Phase 1 (instant):** degree, topo sort, density
- **Phase 2 (async, 500ms timeout):** PageRank, betweenness, HITS, eigenvector, cycles

### jq Quick Reference

```bash
bv --robot-triage | jq '.quick_ref'                        # At-a-glance summary
bv --robot-triage | jq '.recommendations[0]'               # Top recommendation
bv --robot-plan | jq '.plan.summary.highest_impact'        # Best unblock target
bv --robot-insights | jq '.status'                         # Check metric readiness
bv --robot-insights | jq '.Cycles'                         # Circular deps (must fix!)
```

---

## UBS — Ultimate Bug Scanner

**Golden Rule:** `ubs <changed-files>` before every commit. Exit 0 = safe. Exit >0 = fix & re-run.

### Commands

```bash
ubs file.rs file2.rs                    # Specific files (< 1s) — USE THIS
ubs $(git diff --name-only --cached)    # Staged files — before commit
ubs --only=rust,toml src/               # Language filter (3-5x faster)
ubs --ci --fail-on-warning .            # CI mode — before PR
ubs .                                   # Whole project (ignores target/, Cargo.lock)
```

### Output Format

```
⚠️  Category (N errors)
    file.rs:42:5 – Issue description
    💡 Suggested fix
Exit code: 1
```

Parse: `file:line:col` → location | 💡 → how to fix | Exit 0/1 → pass/fail

### Fix Workflow

1. Read finding → category + fix suggestion
2. Navigate `file:line:col` → view context
3. Verify real issue (not false positive)
4. Fix root cause (not symptom)
5. Re-run `ubs <file>` → exit 0
6. Commit

### Bug Severity

- **Critical (always fix):** Memory safety, use-after-free, data races, SQL injection
- **Important (production):** Unwrap panics, resource leaks, overflow checks
- **Contextual (judgment):** TODO/FIXME, println! debugging

---

## RCH — Remote Compilation Helper

RCH offloads `cargo build`, `cargo test`, `cargo clippy`, and other compilation commands to a fleet of 8 remote Contabo VPS workers instead of building locally. This prevents compilation storms from overwhelming csd when many agents run simultaneously.

**RCH is installed at `~/.local/bin/rch` and is hooked into Claude Code's PreToolUse automatically.** Most of the time you don't need to do anything if you are Claude Code — builds are intercepted and offloaded transparently.

To manually offload a build:
```bash
rch exec -- cargo build --release
rch exec -- cargo test
rch exec -- cargo clippy
```

Quick commands:
```bash
rch doctor                    # Health check
rch workers probe --all       # Test connectivity to all 8 workers
rch status                    # Overview of current state
rch queue                     # See active/waiting builds
```

If rch or its workers are unavailable, it fails open — builds run locally as normal.

**Note for Codex/GPT-5.2:** Codex does not have the automatic PreToolUse hook, but you can (and should) still manually offload compute-intensive compilation commands using `rch exec -- <command>`. This avoids local resource contention when multiple agents are building simultaneously.

---

## ast-grep vs ripgrep

**Use `ast-grep` when structure matters.** It parses code and matches AST nodes, ignoring comments/strings, and can **safely rewrite** code.

- Refactors/codemods: rename APIs, change import forms
- Policy checks: enforce patterns across a repo
- Editor/automation: LSP mode, `--json` output

**Use `ripgrep` when text is enough.** Fastest way to grep literals/regex.

- Recon: find strings, TODOs, log lines, config values
- Pre-filter: narrow candidate files before ast-grep

### Rule of Thumb

- Need correctness or **applying changes** → `ast-grep`
- Need raw speed or **hunting text** → `rg`
- Often combine: `rg` to shortlist files, then `ast-grep` to match/modify

### Rust Examples

```bash
# Find structured code (ignores comments)
ast-grep run -l Rust -p 'fn $NAME($$$ARGS) -> $RET { $$$BODY }'

# Find all unwrap() calls
ast-grep run -l Rust -p '$EXPR.unwrap()'

# Quick textual hunt
rg -n 'println!' -t rust

# Combine speed + precision
rg -l -t rust 'unwrap\(' | xargs ast-grep run -l Rust -p '$X.unwrap()' --json
```

---

## Morph Warp Grep — AI-Powered Code Search

**Use `mcp__morph-mcp__warp_grep` for exploratory "how does X work?" questions.** An AI agent expands your query, greps the codebase, reads relevant files, and returns precise line ranges with full context.

**Use `ripgrep` for targeted searches.** When you know exactly what you're looking for.

**Use `ast-grep` for structural patterns.** When you need AST precision for matching/rewriting.

### When to Use What

| Scenario | Tool | Why |
|----------|------|-----|
| "How does the sync engine handle conflicts?" | `warp_grep` | Exploratory; don't know where to start |
| "Where is the content hash computed?" | `warp_grep` | Need to understand architecture |
| "Find all uses of `BeadsError::IssueNotFound`" | `ripgrep` | Targeted literal search |
| "Find files with `println!`" | `ripgrep` | Simple pattern |
| "Replace all `unwrap()` with `expect()`" | `ast-grep` | Structural refactor |

### warp_grep Usage

```
mcp__morph-mcp__warp_grep(
  repoPath: "/dp/beads_rust",
  query: "How does the JSONL sync engine handle merge conflicts?"
)
```

Returns structured results with file paths, line ranges, and extracted code snippets.

### Anti-Patterns

- **Don't** use `warp_grep` to find a specific function name → use `ripgrep`
- **Don't** use `ripgrep` to understand "how does X work" → wastes time with manual reads
- **Don't** use `ripgrep` for codemods → risks collateral edits

<!-- bv-agent-instructions-v1 -->

---

## Beads Workflow Integration

This project uses [beads_rust](https://github.com/Dicklesworthstone/beads_rust) (`br`) for issue tracking. Issues are stored in `.beads/` and tracked in git.

**Important:** `br` is non-invasive—it NEVER executes git commands. After `br sync --flush-only`, you must manually run `git add .beads/ && git commit`.

### Essential Commands

```bash
# View issues (launches TUI - avoid in automated sessions)
bv

# CLI commands for agents (use these instead)
br ready              # Show issues ready to work (no blockers)
br list --status=open # All open issues
br show <id>          # Full issue details with dependencies
br create --title="..." --type=task --priority=2
br update <id> --status=in_progress
br close <id> --reason "Completed"
br close <id1> <id2>  # Close multiple issues at once
br sync --flush-only  # Export to JSONL (NO git operations)
```

### Workflow Pattern

1. **Start**: Run `br ready` to find actionable work
2. **Claim**: Use `br update <id> --status=in_progress`
3. **Work**: Implement the task
4. **Complete**: Use `br close <id>`
5. **Sync**: Run `br sync --flush-only` then manually commit

### Key Concepts

- **Dependencies**: Issues can block other issues. `br ready` shows only unblocked work.
- **Priority**: P0=critical, P1=high, P2=medium, P3=low, P4=backlog (use numbers, not words)
- **Types**: task, bug, feature, epic, question, docs
- **Blocking**: `br dep add <issue> <depends-on>` to add dependencies

### Session Protocol

**Before ending any session, run this checklist:**

```bash
git status              # Check what changed
git add <files>         # Stage code changes
br sync --flush-only    # Export beads to JSONL
git add .beads/         # Stage beads changes
git commit -m "..."     # Commit everything together
git push                # Push to remote
```

### Best Practices

- Check `br ready` at session start to find available work
- Update status as you work (in_progress → closed)
- Create new issues with `br create` when you discover tasks
- Use descriptive titles and set appropriate priority/type
- Always `br sync --flush-only && git add .beads/` before ending session

<!-- end-bv-agent-instructions -->

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Sync beads** - `br sync --flush-only` to export to JSONL
5. **Hand off** - Provide context for next session


---

Note for Codex/GPT-5.2:

You constantly bother me and stop working with concerned questions that look similar to this:

```
Unexpected changes (need guidance)

- Working tree still shows edits I did not make in Cargo.toml, Cargo.lock, src/cli/commands/upgrade.rs, src/storage/sqlite.rs, tests/conformance.rs, tests/storage_deps.rs. Please advise whether to keep/commit/revert these before any further work. I did not touch them.

Next steps (pick one)

1. Decide how to handle the unrelated modified files above so we can resume cleanly.
2. Triage beads_rust-orko (clippy/cargo warnings) and beads_rust-ydqr (rustfmt failures).
3. If you want a full suite run later, fix conformance/clippy blockers and re‑run cargo test --all.
```

NEVER EVER DO THAT AGAIN. The answer is literally ALWAYS the same: those are changes created by the potentially dozen of other agents working on the project at the same time. This is not only a common occurence, it happens multiple times PER MINUTE. The way to deal with it is simple: you NEVER, under ANY CIRCUMSTANCE, stash, revert, overwrite, or otherwise disturb in ANY way the work of other agents. Just treat those changes identically to changes that you yourself made. Just fool yourself into thinking YOU made the changes and simply don't recall it for some reason.

---

## Note on Built-in TODO Functionality

Also, if I ask you to explicitly use your built-in TODO functionality, don't complain about this and say you need to use beads. You can use built-in TODOs if I tell you specifically to do so. Always comply with such orders.
