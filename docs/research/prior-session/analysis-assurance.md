# Analysis: the assurance strategy of `/Users/johnw/src/obr`

Scope: what each test layer actually protects; what falls through all of them; whether the
Go-`bd` conformance strategy has a future; whether the fuzz targets point at the real risk
surface; whether coverage tooling gates anything.

Method. I read the earlier subsystem maps, then re-verified every load-bearing claim against
source. Where a claim was falsifiable by execution I executed it: I copied the tree to
`/private/tmp/.../scratchpad/repro/obr` (plus a copy of the sibling `org2jsonl`), built with
`CARGO_TARGET_DIR` pointed outside both, and ran tests under
`nix develop /Users/johnw/src/obr` (nightly 1.95.0). **`/Users/johnw/src/obr` was never written to.**

Labels: **CONFIRMED (executed)** = I ran it and saw the failure. **CONFIRMED (traced)** = I read
the code path end to end. **SUSPECTED** = inference.

---

## 0. The headline: the pyramid is not being executed

The repo holds ~2,330 test functions (788 `#[test]` in `src/` across 66 files; 1,541 in `tests/`
across 97 integration targets, ~148,800 lines), 65 insta goldens, 52 property tests, 3 fuzz
targets, 20 criterion benches, ~315 Go-conformance tests, 7 workflows.

**The only test command any automated gate runs is `cargo test --lib --bins`.**

- `lefthook.yml:9-12` — pre-commit `test` step: `cargo test --lib --bins`
- `flake.nix:210-221` — `checks.tests` buildPhase: `cargo test --lib --bins`
- `flake.nix:84` — the package itself: `doCheck = false`
- `.github/workflows/ci.yml:88-96` — the only job running the integration suite
  (`cargo test --all-features`). **CI cannot build this tree**: `Cargo.toml:64` declares
  `org2jsonl = { path = "../org2jsonl" }` and `grep -rn org2jsonl .github/` returns nothing.
  No workflow provisions the sibling checkout, so fmt/clippy/check/test all fail at dependency
  resolution on a clean runner. Every CI gate has been inoperative since `afe1351` (2026-02-18).

Both local gates *do* run `cargo clippy --all-targets`, which **compiles** all 97 integration
targets. So the tests type-check on every commit and are never executed. That is exactly the
failure signature observed: every broken test below fails at runtime, not compile time.

### 0.1 Executed evidence — 11 real failures in 4 of 97 targets

I ran four targets. Results (`scratchpad/run1.txt`, `run2.txt`, `run3.txt`):

| Target | Result |
|---|---|
| `e2e_workspace_commands` | `test result: FAILED. 144 passed; 4 failed` |
| `package_manifests` | `test result: FAILED. 3 passed; 2 failed` |
| `snapshots` | `test result: FAILED. 188 passed; 11 failed` |
| `e2e_sync_git_safety` | `test result: FAILED. 125 passed; 6 failed` |

Deduplicating the three `common::dataset_registry` failures (see §0.2, an artifact of my copy),
**11 distinct real failures**:

1. **`e2e_init_creates_jsonl`** — `tests/e2e_workspace_commands.rs:116`
   `assert!(jsonl_path.exists(), "issues.jsonl should exist after sync")`. Plain `init` now writes
   `.beads/issues.org` (`src/cli/commands/init.rs:99-104`, `DEFAULT_JSONL_FILENAME = "issues.org"`
   at `src/config/mod.rs:33`). **CONFIRMED (executed).**
2. **`snapshot_jsonl_export`** — panics at `tests/snapshots/jsonl_format.rs:16:52`,
   `fs::read_to_string(".beads/issues.jsonl").expect("read jsonl")`. **CONFIRMED (executed).**
3-6. **`snapshot_help_output`, `snapshot_create_help`, `snapshot_version_output`,
   `snapshot_doctor_output`** — stale insta goldens. Diffing the generated `.snap.new`:
   `Usage: br [OPTIONS]` → `Usage: obr [OPTIONS]` and
   `Emit JSON Schemas for br output types` → `... for obr ...` (the `br`→`obr` rename, `a2760e6`);
   `OK jsonl.parse: Parsed 0 records` → `OK jsonl.parse: Parsed 0 issues from Org format` (the Org
   migration). **CONFIRMED (executed).**
7-9. **`snapshot_error_invalid_priority`, `snapshot_error_issue_not_found`,
   `snapshot_error_update_closed_issue`** — the goldens capture stderr tracing lines containing
   `path=./.beads/issues.jsonl`; actual is `path=./.beads/issues.org`. **CONFIRMED (executed).**
10. **`test_cargo_metadata`** — `tests/package_manifests.rs:264` asserts `Cargo.toml` contains
    `name = "br"`; it is `name = "obr"` (`Cargo.toml:13`). **CONFIRMED (executed).**
11. **`test_version_consistency`** — `tests/package_manifests.rs:298`: Cargo `0.1.14` vs
    `packaging/scoop/br.json:3` `0.1.13` and `packaging/aur/PKGBUILD:8` `0.1.13`.
    **CONFIRMED (executed).**

And three in the crown-jewel safety file:

12. **`regression_full_sync_cycle_does_not_touch_git`** — panics at
    `tests/e2e_sync_git_safety.rs:408:52`, unguarded
    `fs::read_to_string(".beads/issues.jsonl").expect("read jsonl")`. **CONFIRMED (executed).**
13. **`integration_sync_only_touches_allowed_files`** — panics at `:1062` with the project's own
    alarm text:
    ```
    SAFETY VIOLATION: sync export modified files outside allowed list!
      CREATED: .beads/.br_history/issues.20260806_182255.org (0 bytes)
      MODIFIED: .beads/issues.org (0 -> 1131 bytes)
    ```
    Cause: the test keeps a **private copy** of the allowlist,
    `is_allowed_sync_file` (`tests/e2e_sync_git_safety.rs:637-676`), listing only
    `db, db-wal, db-shm, jsonl, jsonl.tmp` — it never received `org`/`org.tmp`, which the real
    allowlist `src/sync/path.rs:47-59` has had since the migration. **CONFIRMED (executed).**
14. **`integration_sync_manifest_only_touches_allowed_files`** — same mechanism, panics at `:1438`.
    **CONFIRMED (executed).**

Two further tests are **silent no-ops** rather than failures, which is worse for confidence:
- `tests/e2e_sync_git_safety.rs:1086-1092` — the import half of test 13 is wrapped in
  `if jsonl_path.exists()`. It never fires, so "simulate a git pull bringing changes" never
  happens and the subsequent import imports an unchanged file.
- `tests/e2e_git_safety_full_cli.rs:1099-1104` — identical pattern in
  `regression_auto_import_does_not_touch_git`: the file is never touched, auto-import never
  triggers, the test proves nothing.

Corroboration that nobody noticed: `git show --stat c046b35` ("Migrate e2e tests from JSONL to
Org-mode format") touched exactly five test files — `e2e_labels`, `e2e_relations`,
`e2e_sync_artifacts`, `e2e_sync_failure_injection`, `e2e_sync_fuzz_edge_cases`. Whoever did it
fixed the failures they happened to run and stopped. `tests/e2e_sync_git_safety.rs` was last
touched by `a6f128a` (2026-01-17), a month earlier.

### 0.2 Caveat on the three `dataset_registry` failures

`tests/common/dataset_registry.rs:1204, 1223, 1235` failed in every target I ran. These use
`KnownDataset::BeadsRust`, which resolves to `CARGO_MANIFEST_DIR` (`:78`) and requires a populated
`.beads/` and a git repo. My copy has neither, so they are **artifacts of my harness, not repo
findings**. They are still informative: those three tests couple the helper crate to the presence
of the repository's own tracker data, and they execute in all 73 targets that declare `mod common;`.

### 0.3 Assessment

This is not a pyramid; it is a large, thoughtfully-designed, unexecuted artifact resting on a small
executed base of pure-function unit tests. Everything below should be read through that lens: the
gaps are not "tests we forgot to write" but "tests we wrote and stopped running".

---

## 1. Layer by layer: what each protects, and what it is blind to by construction

### 1.1 Unit tests — 788 `#[test]` in `src/` (the only layer that runs)

**Protects.** Pure functions and small parsers, genuinely well: `src/model/mod.rs` (61),
`src/config/mod.rs` (68), `src/util/id.rs` (33), `src/sync/path.rs` (30),
`src/validation/mod.rs` (28). The ID generator, the path allowlist, and config precedence are
real assets.

**Blind spot: the budget is inversely correlated with risk. CONFIRMED (traced).**

| Module | Lines | `#[test]` | Status |
|---|---|---|---|
| `src/storage/sqlite.rs` | 5,157 | 34 | core engine |
| `src/sync/mod.rs` | 5,139 | 84 | core engine |
| `src/sync/org_bridge.rs` | 900 | **7** | **the default on-disk format** |
| `src/format/syntax.rs` | 387 | 18 | **dead** |
| `src/format/markdown.rs` | 502 | 17 | **dead** |
| `src/format/context.rs` | 344 | 11 | **dead** |
| `src/format/rich.rs` | 427 | 7 | **dead** |
| `src/format/theme.rs` | 267 | 5 | **dead** |

`grep -rn "format::syntax\|format::markdown\|syntax::\|markdown::" src/`, excluding those files and
`src/format/mod.rs`, returns **zero hits**. 1,927 lines with 58 unit tests can never execute.
That is ~7.4% of the one enforced test budget spent on unreachable code, while the 900-line parser
through which every issue's data passes on every auto-flush/auto-import has seven tests — and none
of them is a round-trip (`:759, :766, :777, :784, :803, :814, :837`; `test_issue_to_org_basic`
tests emission only).

**Blind spot: a unit test that cements a bug. CONFIRMED (traced).**
`src/storage/sqlite.rs:352-354` — `check_cycle`'s blocking filter is
`AND type IN ('blocks', 'parent-child', 'conditional-blocks')`, omitting `'waits-for'`. But
`DependencyType::is_blocking()` (`src/model/mod.rs:245-250`) includes `WaitsFor`, and
`add_dependency` gates the cycle check on `is_blocking()` (`src/storage/sqlite.rs:1790-1798`). So a
`waits-for` cycle is creatable.

`src/cli/commands/graph.rs:1116-1127` constructs exactly that cycle — `bd-1 → root`,
`bd-2 → bd-1`, `bd-1 → bd-2`, all `"waits-for"`, each `.unwrap()`ed — to prove `graph_all` doesn't
hang. **Fixing `check_cycle` makes this test panic.** The suite has encoded the defect as a fixture.

### 1.2 Storage invariants — `tests/storage_*.rs`, ~200 tests

**Protects.** The strongest-designed layer. `tests/storage_invariants.rs:34-80` pins all 11 tables
and key columns; `:82-109` label idempotency; `:111-154` blocked-cache maintenance; `:156-200`
ready excludes blocked and deferred; `:202-1128` a 30-test list-filter matrix up to five
simultaneous filters; `:1170-1183` the content-hash dedup contract.
`tests/storage_export_atomic.rs` covers the atomic export pipeline.

**Blind spot #1: the whole layer tests a connection configuration that never occurs in
production. CONFIRMED (traced).**

`tests/common/mod.rs:80-83` `test_db()` = `SqliteStorage::open_memory()`, which always calls
`apply_schema` (`src/storage/sqlite.rs:118-120`). Occurrence counts across `tests/storage_*.rs`:
`test_db()` **159 times**, `test_db_with_dir()` **twice** — and both of those create a *fresh*
on-disk DB, so `user_version == 0` and `apply_schema` runs there too.

Production never re-runs `apply_schema`. `open_with_timeout` (`src/storage/sqlite.rs:100-111`)
gates it on `PRAGMA user_version < CURRENT_SCHEMA_VERSION`, and `CURRENT_SCHEMA_VERSION = 1`
(`src/storage/schema.rs:5`) has never been bumped. Every per-connection PRAGMA lives *inside*
`apply_schema` (`src/storage/schema.rs:216-227`): `foreign_keys=ON`, `synchronous=NORMAL`,
`temp_store=MEMORY`, `cache_size=-8000`. Only `journal_mode=WAL` persists in the file header.

**Every real `obr` invocation after the one that created the database runs with
`foreign_keys = OFF`, `synchronous = FULL` and the ~2 MB default cache, while every storage test
runs with `foreign_keys = ON`.** The schema's nine `ON DELETE CASCADE` clauses are inert in
production and live in tests.

The two tests that look like they'd catch this cannot: `src/storage/sqlite.rs:4637-4657`
(`test_pragmas_are_set_correctly`) uses `open_memory()`, and `src/storage/schema.rs:544-555`
asserts immediately after an explicit `apply_schema(&conn)`. **No test anywhere opens a DB on
disk, drops it, reopens it, and asserts a PRAGMA.**

Concrete divergence: `upsert_issue_for_import` uses `INSERT OR REPLACE INTO issues`
(`src/storage/sqlite.rs:3688`). SQLite performs `ON DELETE CASCADE` for REPLACE conflict
resolution *when foreign keys are enabled*. So in the tested configuration an import silently
deletes that issue's `events`, `dirty_issues`, `export_hashes` and `blocked_issues_cache` rows;
in the shipped configuration it does not. The suite is currently insensitive to the difference —
but any future assertion of the form "does a pending dirty flag survive an import?" will pass
locally and be wrong in the field.

**Blind spot #2: the matrix covers `list_issues` only.** `src/storage/sqlite.rs:1031` hardcodes
`ORDER BY priority ASC, created_at DESC` in `search_issues`, while `src/cli/commands/search.rs:319-320`
faithfully populates `ListFilters.sort`/`.reverse` from `SearchArgs`, which flattens `ListArgs`
(`src/cli/mod.rs:1409-1415`). `obr search foo --sort title --reverse` is accepted and silently
ignored. `grep -n sort tests/e2e_search_scenarios.rs` returns **nothing**. Similarly
`search_issues` never applies `include_deferred` (`:996-998` vs `list_issues` at `:818-826`);
`src/cli/commands/search.rs:422` compensates in Rust for `Status::Deferred` only, not for a future
`defer_until`.

### 1.3 Property tests — 52 tests in 4 files

**Protects.** ID shape/determinism/collision-freedom, SHA-256 determinism and sensitivity,
RFC3339 and relative-time round-trips, validation accept/reject.

**Blind spot. CONFIRMED (traced).** All four files import only `beads_rust::{model, util,
validation}`. **No property test touches `beads_rust::storage` or `beads_rust::sync`.** The
properties that would matter most for a sync tool are all absent:

- import idempotence: `import(import(f)) == import(f)`
- export→import round-trip identity, in *either* format
- `updated_at` monotonicity under `update_issue`
- "stored `content_hash` always equals `compute_content_hash()` of the stored row"
- ready = list minus blocked minus deferred
- merge decision-table commutativity

Each is a two-dozen-line proptest, and each would have caught a defect listed in §2.

### 1.4 Snapshots — insta, 76 tests, 65 `.snap`

**Protects.** Human text for ~19 commands, JSON payload shape for ~38, 7 error messages.
Normalization is careful (`tests/snapshots/mod.rs:46-60`: ANSI, IDs, timestamps, dates,
`(main@sha)`, `Owner:`, version numbers) and `*.snap.new` is gitignored.

**Blind spot. CONFIRMED (executed).** Exactly one snapshot covers the on-disk sync format —
`snapshots__snapshots__jsonl_format__issues_jsonl_export.snap` — and it is a **JSONL** golden for
a format that is no longer the default and which the test can no longer even produce (it panics,
§0.1 #2). **There is no golden for `.beads/issues.org`.** The format git actually diffs, humans
actually read, and merge conflicts actually land in has zero byte-level regression protection.
Meanwhile 7 of the 65 goldens are provably stale (§0.1 #3-9), i.e. the layer's own contents have
drifted from reality for ~6 months.

### 1.5 E2E — `tests/e2e_*.rs`, 53 files, ~700 tests

**Protects.** The real coverage: exit-code taxonomy is genuinely asserted
(`tests/e2e_errors.rs:567, 592, 622, 664, 693, 788, 882` — codes 2/3/4/5/6-7), global flags, env
overrides, completions, the full CRUD/query surface, history, git safety.

**Blind spot #1: concurrency tests measure liveness, not correctness. CONFIRMED (traced).**
All seven tests in `tests/e2e_concurrency.rs` (`:91, :162, :248, :313, :353, :417, :498`) drive
`create` (append-only) or `list`, and assert "both succeeded" and "both issues appear".
**Not one performs a concurrent read-modify-write.** Consequently both storage races are
unprotected:

- `src/storage/sqlite.rs:392-394` — `update_issue` fetches the issue *before* the transaction, then
  at `:617-619` recomputes `content_hash` from that stale snapshot and writes it inside the
  transaction. Column values survive (the `UPDATE` is field-scoped), but the persisted
  `content_hash` describes a row that never existed. Since `find_by_content_hash` (`:3629`) is
  phase 2 of import collision detection (`src/sync/mod.rs:2050-2087`), that issue becomes
  permanently invisible to content-hash dedup. Notably the atomic **claim** guard at `:400-426`
  *does* re-read inside the transaction — the author knew the shape and fixed only that case.
- `src/storage/sqlite.rs:1791` — `would_create_cycle` runs *before* `mutate` opens its IMMEDIATE
  transaction at `:1800`. Two `obr dep add` processes can each see "no cycle" and jointly commit
  one. `create_issue` does it correctly (`check_cycle` on `tx` inside the closure, `:281`).

**Blind spot #2: `sync --merge` has zero end-to-end coverage. CONFIRMED (traced).**
`grep -rn '\-\-merge' tests/*.rs` returns **nothing**. See §2.2.

**Blind spot #3: the failure-injection suite tests only the safe half. CONFIRMED (traced).**
`tests/e2e_sync_failure_injection.rs` import cases are `import_failure_missing_file_no_db_changes`
(`:253`), `..._malformed_json_...` (`:293`), `..._conflict_markers_...` (`:343`),
`..._prefix_mismatch_...` (`:386`), `cli_import_malformed_preserves_db` (`:508`) — all
*input-validation* failures, and `import_from_jsonl` validates the entire file before any write
(`src/sync/mod.rs:2224-2275`). All five verify a property that holds by construction. Likewise
`tests/e2e_sync_fuzz_edge_cases.rs` — all 12 tests (`:65, :121, :181, :240, :340, :398, :466,
:534, :595, :633, :672, :732`) are input-*rejection* tests. **Nothing in the estate tests fidelity
of valid input, and nothing injects a DB-side failure mid-import.**

**Blind spot #4: the one Org round-trip e2e asserts almost nothing.**
`e2e_sync_full_cycle_with_artifacts` (`tests/e2e_sync_artifacts.rs:458-540`) does
init → 3 creates → export → external edit of `issues.org` → import → re-export, and then asserts
exactly one thing: `list.stdout.contains("critical")` (`:540`). Labels, dependencies, comments,
timestamps, priority, type, assignee — none checked.

### 1.6 Regression pins — `tests/repro_*.rs`, 21 tests

The cheapest, most durable, healthiest layer: one historical bug each (multibyte panic in time
parsing, unicode-width truncation, blocked-cache crash with multiple parents, `Pinned` blocks
dependents, epic parent does not block child, diamond deps, ID collision fallback,
identical-content-different-labels, backup stem collision, import collision remapping deps,
relation updates bumping `updated_at`, auto-flush no-op, case-insensitive sort, sort aliases,
`create --json` shape). One oddity: `tests/repro_create_path_traversal_check.rs:10` is `#[ignore]`d
as "feature to allow `..` in CLI input not yet implemented" — a regression pin for behaviour never
built.

### 1.7 Structural waste that degrades every layer — CONFIRMED (executed)

73 of the 97 integration targets declare `mod common;`, and `tests/common/` holds **124
`#[cfg(test)]` unit tests** (scenarios 60, dataset_registry 20, harness 16, baseline 8,
artifact_validator 7, report_indexer 5, binary_discovery 5, json_baseline 3). Integration crates
build with `cfg(test)`, so those execute 73 times each ≈ **9,052 redundant executions**. Observed
directly: `e2e_workspace_commands` reported `running 148 tests` for a file with **24** `#[test]`;
`e2e_sync_git_safety` reported 131 for a file with **7**; `snapshots` reported 199 for ~75.
Separately, `tests/conformance_edge_cases.rs:23-24` does `#[path = "conformance.rs"] mod
conformance;`, pulling the 13,491-line master file's 222 tests into a second binary — compiled and
run twice.

This is not merely wasteful. It is a large part of why the suite is slow enough that nobody runs
it, which is the proximate cause of §0.

---

## 2. What falls through *every* layer — concrete, ranked

### 2.1 Org round-trip destroys any label containing a colon — including the `provides:` feature

**CONFIRMED (traced).**

`LabelValidator::validate` (`src/validation/mod.rs:196-206`) permits `:`:
```rust
!label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':')
```
and `src/cli/commands/label.rs:618-619` explicitly asserts `validate_label("provides:auth").is_ok()`
and `validate_label("provides:").is_ok()`.

`provides:<cap>` is the wire format of the cross-repo capability-dependency feature:
`src/storage/sqlite.rs:3320` builds `format!("provides:{cap}")`; `:3343` does
`label.strip_prefix("provides:")` to decide whether a peer repo satisfies a capability.

Org emission writes labels as heading tags — `src/sync/org_bridge.rs:62-76` emits
`    :label1:label2:`. Org tag syntax is colon-delimited, so `provides:auth` becomes
`:provides:auth:`. On import, `org_text_to_issues` (`:335-355`) delegates to
`org2jsonl::org_to_json::org_to_entries_with_keywords`; tags come from `hl.tags()`
(`/Users/johnw/src/org2jsonl/src/org_to_json.rs:238`), a colon split; and
`parse_heading_to_issue` (`:377-379`) does `issue.labels.clone_from(&heading.tags)`.

**One auto-flush plus one auto-import silently converts the label `provides:auth` into two labels
`provides` and `auth`, and the cross-repo capability dependency stops resolving.** No error, no
warning. `obr doctor`'s only consistency check is a record *count* comparison
(`src/cli/commands/doctor.rs:444-488`, and only a `Warn`), which still matches.

Why every layer misses it:
- unit: `validation/mod.rs` tests the label in isolation; `org_bridge.rs`'s 7 tests include **no
  label/tag test at all**
- storage: `label_crud_roundtrip` never leaves SQLite
- property: no org proptest exists
- snapshot: no `.org` golden
- e2e: `tests/e2e_labels.rs:653-662` is the only org-aware label check — a *substring* assertion
  `org_content.contains(":persisted:")` on a single-word label, never re-imported
- fuzz: `fuzz_org_parse` has no oracle; `fuzz_validation` calls `LabelValidator` with no org
  awareness
- conformance: skipped, and bd has no org format

A ~10-line proptest (`issues → issues_to_org_text → org_text_to_issues == issues`) catches this
and an unknown number of siblings (titles containing `:tag:`-shaped text; `provides:` with an empty
capability emits the degenerate `:provides::`).

### 2.2 `sync --merge` silently discards *all* external edits

**CONFIRMED (traced).** `Issue.content_hash` is `#[serde(skip)]` (`src/model/mod.rs:373-375`), so
it is never on disk. In `execute_merge` (`src/cli/commands/sync.rs:1101-1215`):

- `left` (local) ← `get_all_issues_for_export` → `content_hash` is `Some(_)`
- `right` (external) ← `read_issues_from_jsonl` (`src/sync/mod.rs:1972+`, which branches to
  `org_text_to_issues` at `:1981` for `.org`) → `None`
- `base` ← `load_base_snapshot` (`src/sync/mod.rs:2950-2975`, `serde_json::from_str`) → `None`

`merge_issue` case 6 (`src/sync/mod.rs:2764-2771`):
`left_changed = l.content_hash != b.content_hash` is always `Some != None` = **true**;
`right_changed = r.content_hash != b.content_hash` is always `None != None` = **false**; then
`(false | true, false) => MergeResult::Keep(l.clone())`.

**Every issue present in the base snapshot resolves in favour of the local DB regardless of
timestamps or the requested strategy**, and the command then does a `force: true` export
(`src/cli/commands/sync.rs:1211`) overwriting the external file with local state.

Secondary damage: when `Keep(right)` does fire (cases 3 and 7 — new-on-the-external-side),
`upsert_issue_for_import` writes `content_hash = NULL` into `issues`, disabling content-hash dedup
for that row forever.

Why every layer misses it: the seven unit tests (`src/sync/mod.rs:4915-5095`) build fixtures via
`make_issue_with_hash(..., Some("hash1"))` (`:4609-4625`) and `set_content_hash` (`:3083`) — they
manufacture the exact precondition production can never satisfy. There is **no e2e test of
`--merge` at all**.

Same command, also untested: `tombstones = None` is hardcoded at `src/cli/commands/sync.rs:1157`,
so "import never resurrects tombstones" (DL-3) does not hold for `--merge`; and
`MergeResult::Delete` feeds IDs to `storage.delete_issue` (`sync.rs:1185`) which returns
`IssueNotFound` for absent rows (`sqlite.rs:651-652`), aborting mid-merge after earlier upserts
have already landed outside any transaction.

### 2.3 The connection-PRAGMA gap (see §1.2)

Stated as an untested behaviour: **no test in any layer opens an existing on-disk database and
inspects its connection configuration.** The shipped configuration
(`foreign_keys=OFF, synchronous=FULL`) has never been executed by a test. The fix (hoist the
PRAGMAs into a `configure_connection` called unconditionally after `Connection::open`) and the
guard (create → drop → reopen → assert `PRAGMA foreign_keys == 1`) are both one-liners.

### 2.4 Import is non-transactional, and the "no DB changes" tests don't cover it

**CONFIRMED (traced).** `grep -n transaction src/sync/mod.rs` returns nothing.
`import_from_jsonl` calls `storage.clear_all_export_hashes()?` at `src/sync/mod.rs:2380` —
**before any write** — then Phase 3 (`:2466-2477`) is a bare loop of `process_import_action(...)?`,
each issuing autocommit `conn.execute` calls (`src/storage/sqlite.rs:3676-3740`). Only after the
loop are export hashes restored (`:2479-2482`), the blocked cache rebuilt (`:2485`), and metadata
stamped (`:2487-2490`).

A mid-loop failure (disk full, `SQLITE_BUSY` past the timeout, a constraint violation on one row)
leaves: some issues upserted, **all export hashes cleared**, the blocked cache stale,
`last_import_time`/`jsonl_content_hash` unset. On the next command `auto_import_if_stale` re-fires
(the file still looks newer) and re-imports over the partial state.

The five `import_failure_*_no_db_changes` tests all abort before the loop. **No test injects a
DB-side failure mid-import.** Their naming actively manufactures confidence in a property they do
not check.

### 2.5 A failed auto-flush is invisible and exits 0

`src/main.rs:264-296` — `run_auto_flush` swallows every failure at `debug!` level. At default
verbosity `obr close bd-x` can succeed, fail to write the export file, print nothing, and exit 0;
the mutation then exists only in the gitignored SQLite file and is invisible to git.
`tests/repro_auto_flush_inefficiency.rs` pins the *no-op* case; nothing pins the *failure* case.
Given that the tool's entire value proposition is "issue state lands in git alongside the code", a
silent export failure is the worst outcome it can produce, and it is untested.

### 2.6 Two definitions of "blocked" coexist

`get_blocked_by_blocks_deps_only` (`src/storage/sqlite.rs:1254-1262`) writes LEFT JOINs whose WHERE
predicate is false for NULL — effectively INNER JOINs — excluding orphan and external blockers;
`rebuild_blocked_cache_impl` (`:1367-1369`) counts a missing non-external blocker as blocking.
`tests/storage_blocked_cache.rs` (15 tests) exercises the cache path only; nothing asserts the two
agree, so `obr blocked` and dep-derived views can disagree about the same issue.

### 2.7 Comment IDs are global rowids re-inserted verbatim from the file

`src/storage/schema.rs:122-129` declares `comments.id` `AUTOINCREMENT`; it is serialized
(`src/model/mod.rs:630`) and `sync_comments_for_import` (`src/storage/sqlite.rs:3812-3822`) deletes
only the target issue's comments, then `INSERT OR REPLACE INTO comments (id, issue_id, ...)`.
A file-supplied comment id colliding with a comment on a *different* issue silently overwrites it.
No test imports a file whose comment ids overlap the local DB's.

### 2.8 The project stopped dogfooding the new format

`ls .beads/` shows `issues.jsonl` and no `issues.org`, last touched 2026-02-18. The Org path gets
no exploratory or manual exercise either — the only thing that ever runs it is the e2e suite,
which isn't running.

---

## 3. The conformance-to-Go strategy

### 3.1 Mechanics

`tests/conformance.rs`: 13,491 lines, 222 tests, plus five satellites (~315 total). Paired
`br_workspace/` + `bd_workspace/` under one `TempDir` (`:89-147`), pinned
`NO_COLOR`/`HOME`/`BEADS_DIR`, seven `CompareMode`s (`:292-309`), heavy JSON normalization (7
timestamp fields, ID hash portions, `content_hash`, 6 path keys → `<WORKSPACE>`; `:708-810`), and
optional env-gated JSONL/JUnit/summary instrumentation. Only ~40 `CompareMode::*` sites exist
across 222 tests (ExactJson 6, NormalizedJson 10, ContainsFields 11, StructureOnly 7,
FieldsExcluded 4, ArrayUnordered 1, ExitCodeOnly 1) — most tests are hand-written assertions
rather than the declarative harness.

### 3.2 It reports green whether or not it ran — CONFIRMED (traced)

`skip_if_no_bd!()` (`tests/conformance.rs:59-70`) is `eprintln!(...); return;`. **A skipped
conformance test is a passing test.** There is no way to distinguish "315 parity tests passed"
from "315 were not run". The only loud guard is `scripts/conformance.sh` (exits 2 when `bd` is
absent), and that only runs in `.github/workflows/conformance.yml`, which is `workflow_dispatch` +
a **weekly Monday 06:00 UTC cron** (`:14-15`). On every PR, all ~315 tests skip silently.

### 3.3 There is no version pin at all — CONFIRMED (traced)

- `.github/workflows/conformance.yml:53-57`:
  `git clone --depth 1 https://github.com/Dicklesworthstone/beads.git ../beads-go` — an
  **unpinned HEAD clone of a fork**. No tag, no SHA.
- `tests/common/binary_discovery.rs:13` defines `MIN_BD_VERSION = "0.5.0"`, and
  `check_bd_version` (`:292-307`) is called from exactly **one** place —
  `tests/common/scenarios.rs:756`, inside the Scenario DSL. The 222-test master suite uses its own
  `bd_available()` (`tests/conformance.rs:35-57`) with **no version check whatsoever**.
- Even where it is called, `compare_versions` (`:310-322`) parses digits into `Vec<u32>` and
  compares lexicographically: `0.46.0` → `[0,46,0]` > `[0,5,0]` passes, and so does `1.1.2`.
  **There is no upper bound.**
- All 13 documented divergences are written against a specific version, e.g.
  `tests/conformance.rs:1478` "INTENTIONAL DIVERGENCE: bd v0.46.0's `list --json` returns empty
  array even when issues exist."

Per the upstream research: bd v0.50.0 made Dolt the default backend, v0.51.0 began removing
SQLite, v0.58.0 deleted the SQLite layer entirely, and modern `bd export` emits `_schema` header
records, `_type` discriminators and `wisp_plane` markers. The repo also still cites
`github.com/steveyegge/beads` (`.beads/README.md`, `tests/common/binary_discovery.rs:57`), an org
that has moved. So the weekly cron is either already failing incomprehensibly (a modern `bd init`
produces a Dolt workspace and every paired comparison diverges) or building a stale fork that no
longer tracks upstream. **SUSPECTED** — I cannot run it without network + Go.

### 3.4 Divergence is silenced, not asserted — CONFIRMED (traced)

All 13 `INTENTIONAL DIVERGENCE` sites are `#[ignore = "..."]`. **An ignored test asserts nothing.**
If `br` accidentally converges, diverges further, or if `bd` fixes its bug, nothing detects it.
`CompareMode` (`:292-309`) has seven variants and **none expresses "these must differ, in this
specific way"**.

What is silenced matters: eleven of thirteen are `list` — `--json`, `--type`, `--status` (×3),
`--assignee`, `--limit`, `--priority` range, `--label`, multiple filters, `--sort created`
(`:1481, :3059, :5740, :5806, :5875, :5927, :5996, :6069, :6141, :6210, :6369`). **The suite has
disabled essentially all JSON parity coverage for the single most-used command**, because the
reference implementation is buggy. Defensible locally, catastrophic globally: `list`-shaped interop
is the whole point.

The two genuinely semantic ones deserve preservation, not suppression: `:10930` "br parent-child
dependency blocks children; bd does not" and `:12908` "bd v0.46.0 sync does git commit, fails in
non-git dirs: br is intentionally non-invasive". Both are *design positions*, and both live only
as attribute strings — `grep -rn divergence docs/ .beads/*.md` returns nothing.

### 3.5 Harness fragmentation weakens the anti-spoof guard — CONFIRMED (traced)

Four independent `bd_available()` implementations: `tests/conformance.rs:35-57` (honours
`BD_BINARY`, rejects a first token of `obr`), `tests/conformance_workflows.rs:35-40` (duplicated),
`tests/conformance_labels_comments.rs:24-29` and `tests/conformance_text_output.rs:16-21` (both
merely check that `bd version` exits 0). **The last two would accept an `obr` aliased as `bd`**,
in which case ~48 tests compare `obr` to itself and pass. And `scripts/conformance.sh:35-40` runs
only 4 of the 6 targets, permanently excluding `conformance_workflows` (multi-step mutating
workflows) and `conformance_text_output` (text parity).

### 3.6 Is there a deprecation path? No.

None of the four things a fork needs exists:

1. **Pin what you conform to.** Clone a tag (`--branch v0.46.0`) or a SHA; make `skip_if_no_bd!`
   *fail* rather than skip when `BD_BINARY` is set but the version falls outside `[0.40, 0.50)`,
   naming the Dolt pivot in the message.
2. **Assert divergence instead of silencing it.** Add `CompareMode::MustDiffer { reason }` and
   convert the 13 `#[ignore]`s. A bd upgrade that fixes `list --json` then fails the build with a
   pointer to the note, instead of continuing to skip forever.
3. **Distinguish "green" from "not run".** Fail the suite if `BD_BINARY` is set and unusable; emit
   a machine-readable skip count into `conformance_summary.json` and assert on it in the workflow.
4. **Move the divergence catalogue into prose.** Those 13 items are the closest thing this project
   has to a compatibility specification and they exist only inside a 13,491-line test file. They
   belong in `docs/` beside the sync invariants.

Strategic note: upstream reportedly restores SQLite as a first-class backend behind a
capability-gated seam, validated by a Rust spike at 299/299 byte-parity. If that lands, conformance
becomes meaningful again — but only against a *pinned, named* upstream version. Conforming to HEAD
of a fork is not a strategy; it is a cron job that will eventually go red for reasons nobody can
diagnose.

---

## 4. Are the fuzz targets aimed at the real risk surface?

Three targets, added in the most recent work (`5312cb5`), all `|data: &str|`:

| Target | Entry point | Verdict |
|---|---|---|
| `fuzz_jsonl_parse` | `serde_json::from_str::<beads_rust::model::Issue>` | **near-zero marginal value** |
| `fuzz_org_parse` | `sync::org_bridge::org_text_to_issues` | right door, **no oracle** |
| `fuzz_validation` | `is_valid_id_format`, `LabelValidator::validate`, `Status`/`Priority`/`IssueType::from_str` | right surface, low yield |

**`fuzz_jsonl_parse` fuzzes a dependency.** Its body is one `serde_json::from_str` into a
`#[derive(Deserialize)]` struct. serde_json is among the most heavily fuzzed crates in the
ecosystem and derived deserializers contain no hand-written parsing. All obr-specific risk is
*downstream* of that call — `normalize_issue` (`src/sync/mod.rs:2151`), `detect_collision`
(`:2050-2087`), `determine_action` (`:2089-2139`), the prefix guard, the ID remapper
(`:2426-2464`) — none of it reached.

**`fuzz_org_parse` can only find panics.** It discards the result. The §2.1 defect is a *silent
semantic* corruption that a panic-only fuzzer cannot see. The high-value target is a round-trip
differential: `Arbitrary` a `Vec<Issue>`, run `issues_to_org_text` → `org_text_to_issues`, assert
field-by-field equality. That one target would cover the default storage format, the sanitizers
(`sanitize_property_value` `:241`, `sanitize_org_text` `:248`, `unsanitize_org_text` `:450`), the
status/priority keyword maps (`:262-298`), and the JSON src blocks for deps/comments.

**Hand-written parsers that are NOT fuzzed:**

- `src/util/markdown_import.rs::parse_markdown_content` (`:141-205`) — reached from
  `obr create --file x.md`, a genuine untrusted-input boundary; does raw byte slicing at `:158`
  (`line[3..]`, safe today only because the `"## "` guard is ASCII) and hand-rolled section state
  machines. 14 unit tests, 5 e2e tests, **no fuzz target** — despite markdown import being named
  as a risk surface.
- `src/util/time.rs::parse_flexible_timestamp` / `parse_relative_time` — the one function family
  with a **documented panic history** (`tests/repro_time_panic.rs`, multibyte input). Covered only
  by `proptest_time.rs`'s structured generators, much weaker than byte-level fuzzing here.
- `src/util/id.rs::split_prefix_remainder` / `IdResolver::resolve` (`:601-676`) — reached from
  every CLI ID argument.
- config YAML via `serde_yaml` 0.9 — an **archived** crate (open bead `beads_rust-ddwo`) parsing
  a file that arrives over git.
- the Org **emit** side, where §2.1's corruption originates.

**And nothing builds them. CONFIRMED (traced).** `fuzz/Cargo.toml:32-33` declares
`[workspace] members = ["."]`, making `fuzz/` a separate workspace; the root `Cargo.toml` has no
`[workspace]` section. So `cargo check --all-targets` — the clippy gate in lefthook and
`nix flake check` — never compiles the fuzz targets.
`grep -rn fuzz .github/ scripts/ lefthook.yml flake.nix` returns nothing; the only hits are
`.gitignore:35-36` excluding `fuzz/artifacts/` and `fuzz/corpus/`. **No corpus is committed, no CI
runs them, and nothing proves they still compile.** They will rot on the next API rename.

Minimum fix, in order: (1) a weekly `cargo +nightly fuzz build` step so they cannot rot;
(2) a committed seed corpus from `tests/fixtures/` and the repo's own `.beads/issues.jsonl`;
(3) replace `fuzz_jsonl_parse` with a full-pipeline `import_from_jsonl`-into-in-memory-DB target;
(4) add the org round-trip differential; (5) add `markdown_import` and `util::time`.

---

## 5. Is coverage tooling wired to anything enforcing?

**No. CONFIRMED (traced).**

- `tarpaulin.toml:32` sets `fail-under = 50.0`.
  `grep -rn "tarpaulin\|coverage" .github/workflows/ lefthook.yml flake.nix` finds tarpaulin
  **nowhere**. Its only consumer is `scripts/coverage.sh`, a manual developer script that never
  passes `--config` (tarpaulin auto-reads the file, so the gate may fire locally — but nothing
  automated ever invokes it).
- CI uses a *different* tool: `.github/workflows/ci.yml:120-122`
  `cargo llvm-cov --all-features --workspace --lcov` with **`continue-on-error: true`**, then
  `codecov-action` with **`fail_ci_if_error: false`** (`:124-128`). There is **no `codecov.yml` /
  `.codecov.yml`** in the repo, so no status targets are configured either.
- The `coverage` job `needs: test`, and `test` cannot pass (org2jsonl), so it never runs at all.

Two coverage systems, one threshold, zero enforcement points. And the number would be misleading
anyway: with 124 helper tests executing 73 times and the entire conformance suite skipping, a
figure from `cargo test --all-features` measures the harness, not the product.

Adjacent gates that also don't gate:
- `cargo audit --deny warnings` is `continue-on-error: true` in both `ci.yml:66` and `audit.yml:43`;
  only `--deny yanked` hard-fails. Meanwhile `serde_yaml` 0.9 is archived and tracked by an open bead.
- `.github/workflows/release.yml:32-34` documents that fmt/clippy/test gates were **deliberately
  removed** from the release pipeline because "duplicating them here caused release failures for
  cosmetic issues". Tagged releases are built with no verification — and `cargo publish` cannot
  succeed anyway while `org2jsonl` is a path dependency.
- `e2e-quick` runs `scripts/e2e.sh`, whose `QUICK_TESTS` (`scripts/e2e.sh:29-36`) is six targets:
  `e2e_basic_lifecycle`, `e2e_ready`, `e2e_create_output`, `e2e_list_priority`, `e2e_errors`,
  `e2e_harness_demo`. **None is a sync or git-safety test.** The project's highest-severity risk —
  the one with a real incident behind it (`a326295` "restore all files destroyed by bd sync virus")
  — is excluded from the fast gate.
- `ci.yml:240` marks the benchmark *run* `continue-on-error: true`, so a bench that fails to run is
  invisible; and there are two regression thresholds (`ci.yml:250` 5% via inline Python heredoc;
  `scripts/check_regression.py:71` 10%), with CI reimplementing the script rather than calling it.

---

## 6. What is genuinely good and worth not breaking

- **`tests/repro_*.rs`** — 21 tests, one historical bug each, cheap and permanent. Exemplary.
- **The git-safety design.** `src/sync/path.rs:137-175` rejects `.git` before every other check
  and *before* the `--allow-external-jsonl` early return at `:450-458`; the allowlist at `:47-59`
  is exhaustive; and `FileTreeSnapshot` (`tests/e2e_sync_git_safety.rs:684-721`) hashes the entire
  workspace before and after and asserts that *only allowlisted paths changed*. Asserting on the
  complement rather than on named files is strictly better — it fails closed on unknown behaviour.
  It is failing right now precisely because it works; the bug is the duplicated constant.
- **The exit-code taxonomy is actually tested** (`tests/e2e_errors.rs:567-882`, codes 2-7) — rare,
  and exactly what an agent-facing CLI needs.
- **Conformance instrumentation** — JSONL run logs with SHA-256 of stdout/stderr, JUnit XML,
  per-label speedup ratios, failure-context dumps — is more infrastructure than most projects build.
- **The anti-spoof check** (`tests/conformance.rs:47-56`, rejecting a `bd` whose version output
  starts with `obr`) shows someone thought about the failure mode of comparing a tool to itself.
  It just needs to exist in all four copies.
- **Deterministic fixtures** — `tests/common/fixtures.rs:8-11` pins a fixed epoch (2025-01-01Z)
  so snapshot IDs are stable. Right instinct, right place.

---

## 7. Recommendations, ranked by value per unit of effort

1. **Make one command the gate and make it green.** Fix the 11 red tests, then move the pre-commit
   `test` step beyond `--lib --bins` to include at least `e2e_sync_git_safety` and `snapshots`.
   Nothing else here matters while the suite is not run.
2. **Delete the duplicated allowlist.** `tests/e2e_sync_git_safety.rs:637-676` must call
   `beads_rust::sync::path`'s real predicate/constants rather than re-declaring them; otherwise the
   same drift recurs on the next format change. Same for the four `bd_available()` copies.
3. **Provision `org2jsonl` in CI, or vendor/publish it.** Every workflow gate is currently theatre;
   a pinned clone step restores the whole CI story in a few lines.
4. **Add the Org round-trip property test** (proptest, ~30 lines) and a matching fuzz target.
   Highest-value new test in the repo: it covers the default storage format, catches §2.1 today,
   and upgrades the fuzz corpus from panic-hunting to semantics.
5. **Add an `.org` insta golden** so the on-disk format has byte-level regression protection, and
   re-accept the seven stale goldens.
6. **Hoist the PRAGMAs out of `apply_schema` and add the reopen test** (create on disk → drop →
   reopen → assert `PRAGMA foreign_keys == 1`). Two small diffs, closes a whole class and aligns
   the test configuration with the shipped one.
7. **Write the first end-to-end `sync --merge` test** — create locally, edit the file externally,
   merge, assert the external edit survived. It fails today; that is the point.
8. **Add concurrency tests that assert correctness, not liveness:** two concurrent `update` of one
   issue, then assert `stored content_hash == compute_content_hash(stored row)`; two concurrent
   `dep add` forming a cycle, then assert `detect_all_cycles()` is empty.
9. **Add a mid-import failure injection** and assert the DB is either fully imported or fully
   unchanged.
10. **Pin the conformance target; convert `#[ignore]` to `MustDiffer`; make skipped ≠ passed.**
11. **Deduplicate `tests/common/`** — 9,052 redundant executions is a large share of why nobody
    runs the suite. Also drop the `#[path = "conformance.rs"]` include in
    `conformance_edge_cases.rs`.
12. **Delete `src/format/{context,theme,rich,syntax,markdown}.rs` and their 58 unit tests**, or
    wire them up. Stop spending the one enforced budget on unreachable code.
13. **Pick one coverage tool and one threshold and make it required** — or delete `tarpaulin.toml`
    and `scripts/coverage.sh` so the repo stops advertising a gate it doesn't have.
14. **Fix `check_cycle`'s type filter and `graph.rs:1116-1127` together** — the test currently
    depends on the bug.

---

## Appendix A: verification commands

```
grep -rc '#\[test\]' src/ tests/
grep -l '^mod common;' tests/*.rs | wc -l              # 73
grep -rc '#\[test\]' tests/common/*.rs                 # 124 total
grep -rn 'join("issues.jsonl")' tests/                 # locates the org-migration breakage
grep -rn 'foreign_keys' src/ tests/                    # only 2 assertions, both post-apply_schema
grep -rn '\-\-merge' tests/*.rs                        # empty
grep -rn 'check_bd_version' tests/                     # only tests/common/scenarios.rs:756
grep -rn 'fuzz' .github/ scripts/ lefthook.yml flake.nix   # only .gitignore
grep -rn 'tarpaulin\|coverage' .github/workflows/      # only the llvm-cov job
grep -rn 'org2jsonl' .github/                          # empty
git show --stat c046b35                                # 5 test files migrated; git-safety not among them
```

## Appendix B: how the failures were executed

```
cp -R {src,tests,benches,Cargo.*,build.rs,rust-toolchain.toml,packaging,scripts,docs} $SCRATCH/repro/obr
cp -R /Users/johnw/src/org2jsonl $SCRATCH/org2jsonl
cd $SCRATCH/repro/obr
CARGO_TARGET_DIR=$SCRATCH/tgt nix develop /Users/johnw/src/obr --command \
  cargo test --no-fail-fast --test e2e_workspace_commands --test package_manifests \
                            --test snapshots --test e2e_sync_git_safety
```
Original repository untouched (verified: no `target/` was created there, no `Cargo.lock` rewrite).
