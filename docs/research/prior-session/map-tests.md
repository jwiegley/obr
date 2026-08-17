# Subsystem map: `tests` — test & build infrastructure of `/Users/johnw/src/obr`

Repo: crate `beads_rust` v0.1.14, binary target **`obr`** (`Cargo.toml:13`), edition 2024,
nightly toolchain. Rust port of the Go `bd` (beads) issue tracker.

All paths below are relative to `/Users/johnw/src/obr` unless absolute.

---

## 0. Headline numbers

| Metric | Value | Source |
|---|---|---|
| Cargo integration-test targets | **97** | `cargo metadata --no-deps` (one per `tests/*.rs`; `ls tests/*.rs \| wc -l` = 97) |
| `#[test]` attributes under `tests/` | **1541** | `grep -rc '#\[test\]' tests/` |
| `#[test]` attributes under `src/` (unit tests) | **788** in **66** files | `grep -rc '#\[test\]' src/`. `CLAUDE.md` claims "781 tests" for `cargo test --lib --bins` — close, mildly stale. |
| `#[ignore]` attributes under `tests/` | **52** | see §2.7 for the taxonomy |
| Lines of test code under `tests/` | **~148,800** | `wc -l tests/**/*.rs` |
| Largest single file | `tests/conformance.rs` — **13,491** lines / 447 KB / 222 `#[test]` | |
| `insta` snapshot files | **65** `.snap` in `tests/snapshots/snapshots/` | |
| JSON baseline fixtures | **16** in `tests/fixtures/json_baseline/` | |
| Cargo bench targets | **1** (`benchmarks` → `benches/storage_perf.rs`) | `cargo metadata` |
| Criterion benchmark fns | **20**, in 3 groups | `benches/storage_perf.rs:1000-1032` |
| Fuzz targets | **3** | `fuzz/fuzz_targets/` |
| Shell test/CI scripts | 12 in `scripts/`, 3 in `tests/e2e/` | |
| GitHub workflows | 7 | `.github/workflows/` |
| Locked crates in dep graph | **480** | `grep -c '^\[\[package\]\]' Cargo.lock` |

---

## 1. Layering overview — what each layer protects

```
src/**  #[cfg(test)] mod tests        788 unit tests   pure functions, parsers, SQL helpers, formatters
   |
tests/storage_*.rs                    ~200 tests       library-level storage invariants (no subprocess)
tests/jsonl_import_export.rs                           sync engine roundtrip at library level
tests/proptest_*.rs                    52 tests        randomized properties: id, hash, time, validation
   |
tests/snapshots/**  (insta)            76 tests        golden text / JSON / JSONL output, normalized
tests/e2e_*.rs                        ~700 tests       spawn the real `obr` binary in a temp workspace
tests/repro_*.rs                       21 tests        one bug each, permanent regression pins
   |
tests/conformance*.rs                 ~315 tests       run BOTH `obr` and Go `bd`, diff the outputs
tests/bench*.rs, benchmark_*.rs        ~33 tests       mostly #[ignore]d perf harnesses (obr vs bd)
benches/storage_perf.rs                20 benches      criterion micro-benchmarks, CI regression gate
fuzz/fuzz_targets/                      3 targets      libFuzzer on the three untrusted input boundaries
```

---

## 2. `tests/` — full enumeration and classification

### 2.1 Shared harness: `tests/common/` (12 modules, ~7,900 lines)

`tests/common/mod.rs:9-19` declares **all** submodules; `tests/common/mod.rs:21-40` re-exports the
public surface. `tests/common/mod.rs:44-48` `init_test_logging()` (once-guarded),
`:50-78` `TestLogGuard`/`test_log()` (logs "starting" / elapsed on drop),
`:80-83` `test_db()` (in-memory SQLite), `:85-92` `test_db_with_dir()` (temp `.beads/beads.db`).

| Module | Lines | Role |
|---|---|---|
| `cli.rs` | 147 | Minimal `BrWorkspace` (`:17-35`) + `run_br`/`run_br_with_env`/`run_br_with_stdin` (`:37-136`). Spawns `assert_cmd::cargo::cargo_bin!("obr")` (`:95`) with `NO_COLOR=1`, `RUST_LOG=beads_rust=debug`, `RUST_BACKTRACE=1`, `HOME=<workspace>` (`:99-102`), and writes a per-command `.log` (`:115-127`). `extract_json_payload()` (`:138-146`) skips banner lines until the first `[`/`{`. |
| `harness.rs` | 1783 | The "real" harness. `CommandResult` (`:41-87`) with `assert_success`/`assert_failure`/`json::<T>()`. `ParallelismMode` (`:148`), `ResourceGuardrails` (`:158-195`), `RunnerPolicy` (`:202-348`) with `ci()`/`local()`/`benchmark()` presets. `ArtifactLogger` (`:396-607`) writes JSONL event logs, tree snapshots and `summary.json`. `TestWorkspace` (`:609-957`) — temp dir, optional `init_git()` (`:635`), `run_br*`/`run_bd*`. `ConformanceWorkspace` (`:959-1447`) — paired obr/bd roots with env+stdin variants. Helpers `extract_json_payload` (`:1449`), `parse_created_id` (`:1466`). |
| `scenarios.rs` | 3438 | **Scenario DSL** unifying E2E / Conformance / Benchmark (`:1-10` doc). `ExecutionMode{E2E,Conformance,Benchmark}` (`:30-37`); `CompareMode` (`:41-57`); `NormalizationRules` (`:62-261`) with `conformance_default()` (`:85-124`, masks 7 timestamp fields, 8 path fields, 5s timestamp tolerance, sorts arrays, normalizes IDs, paths and CRLF), `strict()` (`:127`), `cross_platform()` (`:136`). `Invariants` (`:263-311`), `ScenarioSetup` (`:312`), `ScenarioCommand` (`:324-372`), `BenchmarkConfig`/`BenchmarkRunner` (`:425-1008`) incl. `measure_peak_rss` (Linux `:544`, stub elsewhere `:565`) and `measure_io_sizes` (`:570`). `Scenario` builder (`:1011-1123`), `ScenarioFilter` with `from_env()` reading `HARNESS_TAGS`/`HARNESS_EXCLUDE_TAGS`/`HARNESS_TAG_MATCH`/`HARNESS_SKIP_SLOW` (`:1140-1302`), `ScenarioRunner` (`:1304-2328`), canned scenarios at `:2338-2389`. |
| `dataset_registry.rs` | 1258 | Real `.beads` dirs as fixtures, copied to isolated temp dirs; **source datasets are never mutated** (`:3-4`). `KnownDataset` enum (`:58-63`): `BeadsRust` (uses `CARGO_MANIFEST_DIR`, `:78`), `BeadsViewer` → `/data/projects/beads_viewer`, `CodingAgentSessionSearch`, `BrennerBot` — the last three are **hard-coded absolute paths on the original author's machine** (`:79-84`) and simply won't resolve here. SHA-256 content hashing + `DatasetIntegrityGuard`. |
| `baseline.rs` | 764 | Benchmark baselines + regression detection. Env knobs documented at `:10-14`: `BENCH_DURATION_THRESHOLD` (default 1.20), `BENCH_RSS_THRESHOLD` (1.30), `BENCH_BASELINE_FILE`, `BENCH_STRICT_MODE`. |
| `binary_discovery.rs` | 378 | Locates/validates `obr` and `bd`; `MIN_BD_VERSION = "0.5.0"` (`:13`); records `BinaryVersion{version,commit,build_date}` into conformance logs (`:16-38`). |
| `report_indexer.rs` | 953 | Turns artifact dirs into HTML/Markdown triage reports (`generate_html_report`, `generate_markdown_report`, `write_reports`). Task `beads_rust-x7on`. |
| `artifact_validator.rs` | 514 | Validates emitted JSONL event logs / snapshots / summaries against `docs/ARTIFACT_LOG_SCHEMA.md` (`:1-6`). Task `beads_rust-r23m`. |
| `json_baseline.rs` | 136 | Loads `tests/fixtures/json_baseline/*.json` (`:28`), `load_baseline`, `load_baseline_raw`, `compare_json_output` — guards that JSON output stayed byte-identical after the rich-terminal integration. |
| `fixtures.rs` | 132 | Deterministic `Issue` builder. `base_time()` is a **fixed** epoch `1_735_689_600` = 2025-01-01Z (`:8-11`) so snapshot IDs are stable; id = `test-<sha256(title)[..8]>` (`:16`, `:58-63`). `IssueBuilder` (`:65-120`) — note `with_status(Closed)` auto-sets `closed_at` to satisfy a DB CHECK constraint (`:82-87`). `dependency()` (`:122`). |
| `assertions.rs` | 26 | `assert_issue_exists`, `assert_status`. |
| `mod.rs` | 92 | see above. |

### 2.2 Conformance vs the Go `bd` binary (6 files, ~19,700 lines, ~315 tests)

| File | Lines | Tests | Focus |
|---|---|---|---|
| `tests/conformance.rs` | 13491 | 222 (26 ignored) | The master harness + the bulk of the parity suite |
| `tests/conformance_labels_comments.rs` | 1729 | 23 | label/comment command parity |
| `tests/conformance_edge_cases.rs` | 1464 | 28 (1 ignored) | input validation, concurrency/stress, error recovery, cross-platform (`:1-10`) |
| `tests/conformance_workflows.rs` | 1309 | 8 | multi-step mutating workflows + JSONL export diffing (`:1-15`, bead `beads_rust-4vzm`) |
| `tests/conformance_schema.rs` | 1248 | 9 | SQLite schema parity via `PRAGMA` introspection, `metadata.json`, JSONL field presence (`:1-9`) |
| `tests/conformance_text_output.rs` | 873 | 25 (3 ignored) | human-readable text parity for list/show/ready/blocked/stats/orphans with ANSI+ID+timestamp normalization (`:1-7`, bead `beads_rust-g1ig`) |

#### How conformance actually works (`tests/conformance.rs`)

1. **Locating `bd`.** `get_bd_binary()` (`:28-30`) reads `$BD_BINARY`, else `"bd"` on `PATH`.
   `bd_available()` (`:35-57`) runs `bd version` and inspects the **first token** of stdout: accepts
   `bd`/`beads`, explicitly rejects `obr` (`:53`) so a symlink/alias of the Rust binary can't
   masquerade as the Go reference. `skip_if_no_bd!()` (`:60-70`) makes every conformance test a
   **silent no-op skip** when `bd` is missing — which is the normal state in this repo's CI
   (`ci.yml` never installs Go; only the dedicated `conformance.yml` does).
2. **Paired workspaces.** `ConformanceWorkspace::new()` (`:89-107`) creates one `TempDir` with
   `br_workspace/`, `bd_workspace/`, `logs/`. `init_both()` (`:110-114`) runs `init` in both.
   `run_br` (`:117-124`) / `run_bd` (`:140-146`) / `run_br_in_bd_env` (`:126-137`, used to set up
   bd-side state with the Rust binary when bd can't produce it).
3. **Process launch.** `run_br_cmd` (`:149-163`) uses `assert_cmd::cargo::cargo_bin!("obr")` and
   pins `NO_COLOR=1`, `RUST_LOG=beads_rust=debug`, `RUST_BACKTRACE=1`, `HOME=<cwd>`.
   `run_bd_cmd` → `run_cmd_system` (`:174-240`) additionally sets `BEADS_DIR=<cwd>/.beads`
   "to avoid contributor routing to planning repos" (`:190-191`) — bd has global routing logic that
   would otherwise escape the sandbox.
4. **Instrumentation.** Every run writes `<label>.log` with args/status/duration/stdout/stderr
   (`:200-212`) and a `RunLogEntry` (`:330-347`) carrying SHA-256 of stdout/stderr (`:228-229`,
   hashing helper `:391-395`). `record_run` (`:566-585`) is a no-op unless one of four env flags is
   on (`LogConfig::from_env`, `:319-328`; `env_flag` accepts `1|true|yes|on`, `:381-389`):
   - `CONFORMANCE_JSON_LOGS` → append to `logs/conformance_runs.jsonl` (`:417-426`)
   - `CONFORMANCE_SUMMARY` → rebuild `conformance_summary.json` with per-binary/per-label run
     counts, failures, total ms and a `speedup_bd_over_br` ratio (`:439-492`)
   - `CONFORMANCE_JUNIT_XML` → `conformance_junit.xml` (`:503-534`)
   - `CONFORMANCE_FAILURE_CONTEXT` → `<label>.failure.json` with stdout/stderr previews (2000
     chars), a `.beads` directory listing and the last 5 runs (`:536-564`)
   A global `LOG_MUTEX` (`:375-379`) serializes these writes across the parallel test threads.
5. **Normalization.** `normalize_json` / `normalize_value` (`:708-760`): 7 timestamp fields
   (`created_at`, `updated_at`, `closed_at`, `deleted_at`, `due_at`, `defer_until`, `compacted_at`)
   → `"NORMALIZED_TIMESTAMP"`; `id`/`issue_id`/`depends_on_id` keep the prefix before the first `-`
   and become `<prefix>-NORMALIZED`; `content_hash` → `"NORMALIZED_HASH"`.
   `normalize_path_fields` (`:762-810`) rewrites `\`→`/` and replaces the canonicalized workspace
   root with `<WORKSPACE>` for the keys `path|database_path|beads_dir|jsonl_path|redirected_from|socket_path`.
6. **Comparison modes.** `CompareMode` (`:292-309`): `ExactJson`, `NormalizedJson`,
   `ContainsFields(Vec<String>)` (dotted paths, resolved by `extract_field` `:923-941` which also
   indexes into `arr[0]`), `ExitCodeOnly`, `ArrayUnordered` (`json_equal_unordered` `:943-979`),
   `FieldsExcluded` (`filter_fields` `:982-995`), `StructureOnly` (`structure_matches` `:998-1033`,
   type-shape-only, uses only the first array element). Dispatcher `compare_json` (`:813-912`).
7. **Diagnostics.** `diff_json`/`collect_diffs` (`:1040-1123`) produce a path-keyed
   `br=… , bd=…` report, capped at 20 differences (`:1050`, `:1058`).
8. **Timing.** `log_timings` (`:914-921`) emits a `speedup` per test. A full mini-benchmark harness
   lives inside the same file: `BenchmarkConfig{warmup_runs:2, timed_runs:5, outlier_threshold:2.0}`
   (`:593-610`), `TimingStats::from_durations` (mean/median/p95/stddev/min/max, `:626-663`),
   `filter_outliers` (`:666-683`), `run_benchmark` (`:687-705`).
9. **Declarative scenarios.** `TestScenario` (`:1157-1247`) with `setup_commands` run on *both*
   binaries then a `test_command` compared under a `CompareMode`; canned ones in
   `mod scenarios` (`:1251-1294`): `empty_list`, `create_basic`, `create_with_type_and_priority`,
   `stats_after_create`. Most real tests are hand-written rather than using this.
10. **Suite layout** (section banners in `conformance.rs`): CONFORMANCE TESTS `:1297`,
    SYNC COMMAND EXPANSION `:4073`, CRUD COMMAND EXPANSION `:4996`, DEPENDENCY COMMANDS `:7551`,
    UTILITY COMMANDS `:8941`, REMAINING CRUD `:10206`, EPIC `:10668`, GRAPH `:11239`, AUDIT `:11489`,
    Q/QUICK-CAPTURE `:11842`, LINT `:12033`, DEFER/UNDEFER `:12180`, HISTORY (br-only) `:12327`,
    ORPHANS `:12411`, CHANGELOG (br-only) `:12494`, QUERY (br-only) `:12565`, COMPLETIONS `:12683`,
    BASE SNAPSHOT `:12882`, CONFLICT MARKER `:13131`, PREFIX MISMATCH `:13321`.

**Harness fragmentation.** The five satellite conformance files do *not* share one abstraction:
- `conformance_edge_cases.rs:23-26` pulls the master file in with `#[path = "conformance.rs"] mod conformance;`
- `conformance_labels_comments.rs:24-29` re-implements its own weaker `bd_available()` (no
  anti-alias check, no `BD_BINARY`)
- `conformance_text_output.rs:11` uses `common::harness::ConformanceWorkspace` instead
- `conformance_schema.rs:33-...` defines a third, separate `SchemaWorkspace`
- `conformance_workflows.rs:35-40` re-implements `get_bd_binary` + the alias check
So there are four distinct `bd_available()` implementations with different strictness.

### 2.3 E2E — subprocess-level (`tests/e2e_*.rs`, 53 files, ~700 tests)

Grouped by what they protect:

- **Lifecycle / CRUD:** `e2e_basic_lifecycle` (18), `e2e_create_output` (3), `e2e_comments` (13),
  `e2e_comments_stdin` (1), `e2e_labels` (22), `e2e_relations` (6), `e2e_epic` (12),
  `e2e_quick_capture` (19), `e2e_claim_atomic` (9 — atomic claim/lease semantics).
- **Query surface:** `e2e_list_comprehensive` (37), `e2e_list_scenarios` (33), `e2e_list_priority`
  (3), `e2e_ready` (20), `e2e_ready_limit` (1), `e2e_queries` (7), `e2e_search_scenarios` (19),
  `e2e_graph` (9), `e2e_graph_ordering` (1), `e2e_dep_tree_mermaid` (13), `e2e_orphans` (13),
  `e2e_stale` (3), `e2e_defer` (22), `e2e_undefer` (1), `e2e_audit` (20), `e2e_lint` (21),
  `e2e_changelog` (15).
- **CLI contract:** `e2e_errors` (36 — exit codes + error envelopes), `e2e_global_flags` (29),
  `e2e_env_overrides` (18), `e2e_config_precedence` (4), `e2e_completions` (23),
  `e2e_version` (4), `e2e_schema` (2), `e2e_wrap` (16 — terminal wrapping/width),
  `e2e_routing` (14 — workspace routing/redirection).
- **Sync / git safety (the crown jewels):**
  - `e2e_sync_git_safety.rs` (1463 lines, 7 tests) — asserts `br sync` **never** runs git, commits,
    stages, or mutates `.git` (`:1-11`, beads `beads_rust-0v1.2.4`, `-0v1.3.3`).
  - `e2e_git_safety_full_cli.rs` (1138 lines, 3 tests) — same guarantee across the whole CLI.
  - `e2e_sync_failure_injection.rs` (944 lines, 15) — read-only dirs, permission denied, partial
    writes; asserts export/import atomicity and no corruption (`:1-9`).
  - `e2e_sync_fuzz_edge_cases.rs` (802 lines, 12), `e2e_sync_artifacts.rs` (971 lines, 9),
    `e2e_sync_preflight_integration.rs` (603 lines, 9).
- **History / workspace:** `e2e_history` (22), `e2e_history_restore_prune` (17),
  `e2e_history_custom_path` (1), `e2e_workspace_commands` (24), `e2e_workspace_scenarios` (19).
- **Concurrency:** `e2e_concurrency.rs` (521 lines, 7) — SQLite lock contention, `--lock-timeout`
  error codes, concurrent readers (`:1-9`, bead `beads_rust-uahy`). Uses `Arc<Barrier>` + threads.
- **Distribution:** `e2e_installer.rs` (717 lines, 17, 2 ignored) — exercises `install.sh`
  (platform detection, version resolution via GitHub API + redirect fallback, checksum verify,
  idempotency, proxy). Network-dependent tests are `#[ignore]`d (`:16`).
  `e2e_upgrade.rs` (638 lines, 22) — `upgrade --check/--dry-run`, cannot actually self-update
  because that would rewrite the binary under test (`:9-14`).
- **Meta:** `e2e_harness_demo` (4 — validates the harness itself),
  `e2e_report_generation` (6, 1 ignored — invoked by `scripts/generate-report.sh`),
  `e2e_cold_warm_benchmarks` (5, 1 ignored).

Two spawn styles coexist: the lightweight `common::cli::{BrWorkspace, run_br}` (e.g.
`e2e_basic_lifecycle.rs:1-9`) and the heavyweight `common::harness::TestWorkspace` with artifact
logging. Some files (`e2e_concurrency.rs:11-18`) bypass both and drive `assert_cmd::Command` directly.

### 2.4 Storage / library-level invariants (9 files, ~5,900 lines, ~200 tests)

| File | Lines | Tests | Protects |
|---|---|---|---|
| `storage_invariants.rs` | 1183 | 31 | see below |
| `storage_list_filters.rs` | 939 | 33 | list filter matrix |
| `storage_deps.rs` | 1066 | 29 | dependency graph, cycles |
| `storage_crud.rs` | 811 | 33 | create/read/update/delete |
| `storage_blocked_cache.rs` | 713 | 15 | `blocked_issues_cache` maintenance |
| `storage_ready.rs` | 651 | 20 | ready-query semantics |
| `storage_export_atomic.rs` | 520 | 12 | atomic export pipeline (`:1-13`, bead `beads_rust-3hls`): concurrent exports, runtime path-traversal rejection, export outside `beads_dir` rejected, no orphaned `.tmp`, clean re-export, every line valid JSONL, idempotent hash |
| `storage_history.rs` | 453 | 15 | history/backup files |
| `storage_id_hash_parity.rs` | 751 | 16 | deterministic ID generation + content-hash parity "against legacy bd outputs" (`:1-10`) |
| `jsonl_import_export.rs` | 504 | 19 | `export_to_jsonl`/`finalize_export`/`import_from_jsonl`/`read_issues_from_jsonl` roundtrips |
| `markdown_import.rs` | 179 | 5 | markdown → issues import via the CLI |

**Invariants asserted by `tests/storage_invariants.rs`** (helpers: `table_names` `:10-18`,
`column_names` via `pragma_table_info` `:20-28`, `issue_ids` `:30-32`):

1. `schema_tables_and_columns_exist` (`:34-80`) — the 11 tables `issues`, `dependencies`, `labels`,
   `comments`, `events`, `config`, `metadata`, `dirty_issues`, `export_hashes`,
   `blocked_issues_cache`, `child_counters` must exist (`:43-57`); `issues` must have
   `id, content_hash, title, status, priority, issue_type, created_at, updated_at` (`:60-71`);
   `blocked_issues_cache` must have `issue_id, blocked_by, blocked_at` (`:73-79`).
2. `label_crud_roundtrip` (`:82-109`) — `add_label` returns `true` first time / `false` on duplicate
   (idempotency, `:88-89`); `set_labels` replaces wholesale; `remove_label` returns `true`.
3. `dependency_crud_updates_blocked_cache` (`:111-154`) — adding a `blocks` dep makes the dependent
   appear in `get_blocked_ids()` and `get_blocked_issues()` with a `"<blocker_id>:"`-prefixed
   reason (`:130-144`); after `remove_dependency` + `rebuild_blocked_cache(true)` it disappears
   (`:146-153`). Note the cache is *not* asserted to self-heal without the explicit rebuild.
4. `ready_filters_exclude_blocked_and_deferred` (`:156-200`) — a blocked issue and an issue with
   `defer_until` in the future are both excluded from `get_ready_issues` while an equally-labelled,
   equally-prioritized free issue is included.
5. `list_filters_*` — a 30-test matrix (`:202-1128`, banner `:245`) covering single/multiple status,
   single/range priority, single/multiple type, assignee, unassigned, include-closed,
   include-templates, title-contains, limit, empty result, no-filter default (open only), and a
   dedicated **combined-filter** block (banner `:701`) with ≥15 combinations up to five simultaneous
   filters (`:1015`).
6. `list_issues_with_counts_accurate_dependencies` (`:1133-1168`, banner `:1130`) — on a
   parent→2 children→grandchild graph, `count_dependents`/`count_dependencies` are exact.
7. `find_by_content_hash_roundtrip` (`:1170-1183`) — `upsert_issue_for_import` then
   `find_by_content_hash` returns the same issue: the content-addressed dedup contract.

### 2.5 Property-based (`tests/proptest_*.rs`, 4 files, 1002 lines, 52 tests)

All use `proptest` with `ProptestConfig { cases: 100 }` and a `try_init` tracing subscriber.

- `proptest_id.rs` (264) — IDs always match `<prefix>-<base36hash>` (`:36`), determinism, no
  collisions in realistic batches (`id_no_collisions_batch` `:221`), `optimal_length_monotonic`
  (`:247`). Exercises `IdConfig/IdGenerator/compute_id_hash/generate_id/generate_id_seed/parse_id`.
- `proptest_hash.rs` (294) — hash is 64 hex chars (SHA-256), deterministic, changes with content;
  `hash_low_collision_rate` (`:248`), `hash_changes_with_issue_type` (`:277`).
- `proptest_time.rs` (354) — RFC3339 roundtrip over generated Y/M/D/H/M/S (`:31-38`), relative-time
  expressions, rejection of invalid formats; `keyword_tomorrow_is_future` (`:259`),
  `keyword_next_week_is_week_away` (`:284`), `rfc3339_with_offset_parses` (`:307`),
  `whitespace_is_trimmed` (`:331`).
- `proptest_validation.rs` (374) — valid issues pass, bad priorities/empty titles fail, timestamp
  invariants; `all_standard_statuses_valid` (`:324`), `all_standard_types_valid` (`:351`).

### 2.6 Snapshot tests (`insta`)

`tests/snapshots.rs` is a 2-line shim: `#[path = "snapshots/mod.rs"] mod snapshots;`.
`tests/snapshots/mod.rs` (881 lines, 23 tests) is the golden-text engine (bead `beads_rust-hdc0`,
`:37-43`): `init_workspace()` forces `init --prefix bd` (`:13-18`) for stable IDs; `parse_created_id`
tolerates both `Created …` and `✓ Created …` (`:26-35`); pre-compiled `LazyLock<Regex>` normalizers
strip ANSI (`:46`), IDs (`:48`), full timestamps (`:50`), dates (`:54`), the `(main@sha)` version
suffix (`:56`), `Owner: …` (`:58`), version numbers (`:60`).
Sub-modules: `json_output.rs` (753 lines, 30 tests), `cli_output.rs` (231, 15),
`error_messages.rs` (93, 7), `jsonl_format.rs` (20, 1).
65 committed `.snap` files under `tests/snapshots/snapshots/` covering list/show/ready/blocked/
stats/count/graph/orphans/stale/search/label/comments/info/where/version/help, six error messages,
and one JSONL export. `.gitignore` ignores `*.snap.new` so accepted-vs-pending is enforced.

### 2.7 Regression pins (`tests/repro_*.rs`, 16 files, 21 tests)

One historical bug each — the cheapest, most durable layer:

| File | The bug it pins |
|---|---|
| `repro_time_panic.rs` (2 tests) | `parse_flexible_timestamp` / `parse_relative_time` panicked on multi-byte input; must return `Err` |
| `repro_truncate_width.rs` | `"🚀🚀🚀"` is 3 chars but 6 columns — truncation must be width-aware |
| `repro_cache_crash.rs` | `rebuild_blocked_cache` crashed with multiple parents |
| `repro_pinned_blocker.rs` | `Status::Pinned` **does** block dependents |
| `repro_epic_blocking.rs` | an epic parent does **not** block its child task |
| `repro_dep_tree.rs` | diamond dependency visibility in `dep tree` |
| `repro_id_collision.rs` | ID generator fallback on collision |
| `repro_collision_labels.rs` | identical content + different labels must not collide |
| `repro_history_collision.rs` | backup filename stem collision (`issues_archive.jsonl`) |
| `repro_import_collision_remap.rs` | import collision must remap dependencies too |
| `repro_sync_relations.rs` (2) | relation/dependency updates bump `updated_at` and get synced |
| `repro_auto_flush_inefficiency.rs` (2) | auto-flush must no-op when content didn't change |
| `repro_list_sort.rs` | `list --sort title` is case-insensitive |
| `repro_list_sort_alias.rs` | sort aliases accepted |
| `repro_create_output.rs` | `create --json` emits a single object, not an array |
| `repro_create_path_traversal_check.rs` | **`#[ignore]`d**: "Path traversal restriction still active; feature to allow `..` in CLI input not yet implemented" |

### 2.8 Benchmark-shaped tests (7 files, ~6,800 lines, ~33 tests, mostly `#[ignore]`)

These are `cargo test` targets, not criterion benches — they compare `obr` against `bd` end-to-end.

- `bench_cold_warm.rs` (943, 5, 2 ign) and `bench_cold_warm_start.rs` (966, 7, 5 ign) — cold
  (first process invocation) vs warm (cached) start for both binaries.
- `bench_real_datasets.rs` (783, 4, 2 ign) — read/write workloads on real `.beads` datasets,
  always on isolated copies.
- `bench_synthetic_scale.rs` (935, 8, 5 ign) — generates 100k+ issue synthetic datasets; opt-in via
  `BR_E2E_STRESS=1 cargo test --test bench_synthetic_scale -- --ignored --nocapture` (`:9-11`).
- `benchmark_comparison.rs` (1639, 6, 2 ign) — latency/throughput/scaling obr vs bd.
- `benchmark_datasets.rs` (1306, 3, 1 ign) — per-dataset comparison tables with time + RSS.
- `e2e_cold_warm_benchmarks.rs` (848, 5, 1 ign).

### 2.9 Packaging validation

`tests/package_manifests.rs` (313 lines, 5 tests) — validates the Homebrew formula, Scoop manifest,
AUR PKGBUILD and `Cargo.toml` metadata. **Currently broken/no-op**, see §7.

### 2.10 Shell E2E (`tests/e2e/`, 3 scripts, not wired into `cargo test`)

- `test_json_compat.sh` — JSON output validity/consistency
- `test_mode_detection.sh` — output-mode (rich/plain/json/quiet) detection, checks for ANSI presence
- `test_rich_output.sh` — rich terminal integration
All three log to `/tmp/br_e2e_logs/<name>_<timestamp>.log` via `exec > >(tee …)`. **Nothing in
`scripts/` or `.github/workflows/` invokes them** — they are manual-only.

---

## 3. Fuzzing — `fuzz/`

Separate cargo workspace (`fuzz/Cargo.toml:32-33 [workspace] members = ["."]`), `cargo-fuzz` +
`libfuzzer-sys 0.4`, depends on the parent by path (`:14-15`). Three targets, all taking `&str`
(so libFuzzer only produces valid UTF-8; byte-level malformed input is *not* explored):

| Target | Entry point | Boundary protected |
|---|---|---|
| `fuzz_jsonl_parse.rs` | `serde_json::from_str::<beads_rust::model::Issue>` | JSONL line parsing on import (the git-sync boundary) |
| `fuzz_org_parse.rs` | `beads_rust::sync::org_bridge::org_text_to_issues` (`src/sync/org_bridge.rs:335`) | Org-mode import |
| `fuzz_validation.rs` | `validation::is_valid_id_format` (`src/validation/mod.rs:257`), `validation::LabelValidator::validate` (`:181`), and `FromStr` for `Status`/`Priority`/`IssueType` | all string-based validators/parsers |

`.gitignore` excludes `fuzz/artifacts/` and `fuzz/corpus/`; no seed corpus is committed.
**No CI workflow runs the fuzzers** — they are developer-invoked only.
Added recently: commit `5312cb5 "Add cargo-fuzz targets for input boundaries"`.

---

## 4. Benchmarks — `benches/`

- `Cargo.toml:85-88` declares exactly one bench target: `name = "benchmarks"`,
  `path = "benches/storage_perf.rs"`, `harness = false`.
- `benches/storage_perf.rs` (34,766 bytes) — 20 criterion benchmark functions with documented
  targets in the header table (`:5-14`): create <1 ms, list(1k) <10 ms, list(10k) <100 ms,
  ready(1k/2k) <5 ms, ready(10k/20k) <50 ms, export(10k) <500 ms, import(10k) <1 s.
  Env-tunable criterion config: `BENCH_NOISE_THRESHOLD` (`:159`), `BENCH_SIGNIFICANCE_LEVEL`
  (`:164`), `BENCH_SAMPLE_SIZE` (`:169`). Groups (`:1000-1032`):
  - `storage_benches` — create_single, create_batch, update_issue, close_issue_with_reason,
    delete_issue, list_issues, list_issues_filtered, ready_query, blocked_query, add_dependency,
    cycle_detection, search
  - `sync_benches` — export, import, dirty_tracking_mark, dirty_tracking_query
  - `id_benches` — generate_id, resolve_id_prefix, id_hash, content_hash
- `benches/benchmarks.rs` is an 8-line shim that `include!("storage_perf.rs")`. **It is never
  compiled**: `cargo metadata` reports only one bench target and its `src_path` is
  `benches/storage_perf.rs`. The manifest `[[bench]] name = "benchmarks"` shadows the auto-inferred
  `benches/benchmarks.rs` by name, and `benches/storage_perf.rs` is excluded from auto-inference by
  path. So `benchmarks.rs` is dead weight.

---

## 5. Build & toolchain

### `Cargo.toml`
- Package `beads_rust` 0.1.14, `rust-version = "1.85"`, edition 2024, BSD-3-Clause.
  **Binary is `obr`** (`:12-14`) — CLAUDE.md and most doc comments still say `br`.
- `[profile.release]` (`:90-95`): `opt-level = "z"`, `lto = true`, `codegen-units = 1`,
  `panic = "abort"`, `strip = true` — size-optimized. `[profile.dev] opt-level = 1` (`:97-98`).
- `[features]` (`:100-102`): `default = ["self_update"]`; `self_update = ["dep:self_update"]`.
- `[lints.rust] unsafe_code = "forbid"` (`:104-105`), reinforced by `#![forbid(unsafe_code)]` in
  `src/lib.rs:19`.
- `[lints.clippy]` (`:107-116`): `pedantic` and `nursery` both at **deny** with `priority = -1`,
  then six escape hatches at `priority = 1`: `cast_precision_loss`, `doc_markdown`,
  `missing_const_for_fn`, `uninlined_format_args`, `useless_let_if_seq`, `format_push_string`
  (comment at `:110` says these exist "to unblock CI").
  Note: most `tests/conformance*.rs` open with `#![allow(clippy::all, clippy::pedantic, clippy::nursery)]`
  — the strict lint policy does not apply to the conformance suite.

### Dependency tree highlights (480 locked crates)
| Crate | Why |
|---|---|
| `clap` 4.5 + `clap_complete` 4.5 (`unstable-ext`, `unstable-dynamic`) | 35+ subcommand CLI, dynamic shell completions |
| `rusqlite` 0.38 with `bundled`, `modern_sqlite`, `fallible_uint` | **bundled** SQLite (`libsqlite3-sys` 0.36 compiles C in-tree) → no system SQLite dependency, reproducible schema across platforms; `fallible_uint` avoids silent unsigned truncation |
| `serde`/`serde_json`/`serde_yaml` 0.9/`schemars` 0.8 | JSONL is the sync format; `schemars` powers `br schema`/`info --schema` output |
| `chrono` 0.4 (serde) | all timestamps |
| `sha2` 0.10 | content-addressed dedup + ID hashes + conformance stdout hashing |
| `anyhow` 1.0 + `thiserror` 2.0 | the 3-layer error system (`BeadsError → StructuredError → ErrorCode`) |
| `tracing` + `tracing-subscriber` (env-filter, fmt, json) | structured logs; tests set `RUST_LOG=beads_rust=debug` |
| `indicatif` 0.18, `crossterm` 0.28 | progress + terminal control |
| `rich_rust` 0.2.0 (crates.io, checksum in `Cargo.lock:3201-3204`) | sibling project — rich TTY output. Cargo.toml comment `:49` says "pinned to commit with let_chains fix" but it is actually a plain registry version, not a git pin |
| `toon_rust` **git rev `788589d`** (`Cargo.toml:53`, `Cargo.lock:3957-3959`) | TOON output format. Requires a Nix `outputHashes` entry (`flake.nix:66`, `:109`) |
| `org2jsonl` **path `../org2jsonl`** (`Cargo.toml:64`) | Org-mode import. Out-of-tree path dependency — the repo does not build standalone; `flake.nix` works around this with an `org2jsonl` flake input pointing at `git+file:///Users/johnw/src/org2jsonl` (`:21-24`) and a `postUnpack` that copies it into place (`:58-61`) |
| `dunce` 1.0 | strip `\\?\` on Windows |
| `once_cell`, `regex` 1.11, `unicode-width` 0.2, `semver` 1.0 | misc; `unicode-width` backs the truncation fix pinned by `repro_truncate_width.rs` |
| `self_update` 0.42 (rustls, signatures; optional) | `br upgrade` |
| **build-dep** `vergen-gix` 9.1 | build/cargo/rustc instructions |
| **dev-deps** | `tempfile` 3.10, `assert_cmd` 2.0, `predicates` 3.1, `criterion` 0.8 (html_reports), `walkdir` 2.4, `insta` 1.38 (json, yaml), `proptest` 1.6, `rand` 0.9.2 |

### `build.rs` (26 lines)
`vergen-gix` emits: build timestamp (`:8`), cargo target triple (`:9`), git branch/sha/commit
timestamp/**dirty** flag (`:10-15`), rustc semver (`:16`). This is what `obr --version` prints and
what the snapshot normalizer at `tests/snapshots/mod.rs:56-57` has to scrub (`(main@abc123)`).
Consequence: a dirty tree changes the version string, hence the normalization.

### `rust-toolchain.toml`
`channel = "nightly"`, components `rustfmt`, `clippy`. Nightly is required for edition 2024 at the
time this was written; `rust-version = "1.85"` in Cargo.toml is inconsistent with that (§7).

### `lefthook.yml` (pre-commit, `parallel: true`)
| Command | Glob | Runs |
|---|---|---|
| `fmt-check` | `*.rs` | `cargo fmt --all -- --check` |
| `clippy` | `*.rs` | `cargo clippy --all-targets -- --deny warnings` |
| `test` | `*.{rs,toml}` | `cargo test --lib --bins` (unit tests only — the 97 integration targets are **not** gated pre-commit) |
| `shellcheck` | `*.sh` | `shellcheck {staged_files}` |
| `shfmt-check` | `*.sh` | `shfmt -d {staged_files}` |
| `nix-check` | `*.{rs,nix,toml,lock}` | `nix flake check --no-warn-dirty` |
Since `parallel: true`, `clippy` and `nix flake check` can contend for the same cargo target lock.

### `flake.nix` (6 KB)
Inputs: `nixpkgs` (unstable), `rust-overlay`, `flake-utils`, and `org2jsonl` from
`git+file:///Users/johnw/src/org2jsonl` (`:21-24` — a machine-local absolute path; the flake is not
portable).
- `rustToolchain = pkgs.rust-bin.nightly.latest.default` + rust-src/rust-analyzer/clippy/rustfmt
  (`:36-43`); custom `rustPlatform` built on it (`:46-49`).
- Package `beads_rust` (`:52-93`): `postUnpack` copies the `org2jsonl` input into place (`:58-61`);
  `cargoLock.outputHashes` pins the `toon_rust` git dep (`:65-68`); `nativeBuildInputs` =
  pkg-config + `rustPlatform.bindgenHook`; `buildInputs` = sqlite (+ libiconv, apple-sdk_15 on
  Darwin); **`doCheck = false`** (`:84`) — `nix build` runs no tests. `mainProgram = "obr"` (`:90`).
- `devShells.default` (`:132-165`): toolchain + pkg-config, cargo-watch, cargo-edit, cargo-expand,
  **cargo-nextest**, lefthook, cargo-audit, shellcheck, shfmt, sqlite. `shellHook` unsets
  `DEVELOPER_DIR` to avoid an SDK clash on Darwin (`:157-159`) and exports `RUST_BACKTRACE=1`,
  `RUST_LOG=info`.
- `checks` (`:180-222`) — three derivations sharing `checkCommon` (`:96-127`): `build`,
  `formatting` (`cargo fmt --all -- --check`), `clippy` (`cargo clippy --all-targets -- --deny warnings`),
  `tests` (`cargo test --lib --bins`). Again: only unit tests.

### `tarpaulin.toml`
`timeout 600s`, `verbose`, `ignore-tests`, `follow-exec`; outputs Html+Lcov+Json to `coverage/`;
`jobs = 4`; `exclude-files = ["tests/*", "benches/*"]`; `skip-clean`; debug mode;
**`fail-under = 50.0`** (`:32`); `run-types = ["Tests", "Doctests"]`.
Note the CI coverage job uses **`cargo-llvm-cov`**, not tarpaulin (`ci.yml:115-121`) — tarpaulin is
local-only via `scripts/coverage.sh`, and the 50% gate is therefore never enforced in CI.

---

## 6. CI/CD — `.github/workflows/` (7 files) and `scripts/` (12 files)

### `ci.yml` (push/PR to `main` + dispatch; concurrency-cancelling)
| Job | Timeout | What |
|---|---|---|
| `check` | 15 m | `cargo fmt --check`; clippy `--all-targets --all-features -D warnings`; clippy `--no-default-features`; `cargo check --all-targets --all-features` |
| `security` | 10 m | `cargo audit --deny warnings` (**`continue-on-error: true`**, `:66` — advisory only) then `cargo audit --deny yanked` (hard gate) |
| `test` (needs check) | 40 m | `cargo test --all-features -- --nocapture` with `RUST_LOG=beads_rust=debug`; `cargo test --no-default-features`; `cargo test --doc` |
| `coverage` (needs test) | 30 m | `cargo llvm-cov --all-features --workspace --lcov` (**`continue-on-error`**), upload to Codecov with `fail_ci_if_error: false` — coverage is informational only |
| `build` (needs check) | 30 m | 5-way matrix: linux x64/arm64 (native arm runner), macOS arm64/x64, windows x64; `--version` smoke test; uploads `obr-<platform>` artifacts |
| `bench` (needs check, PR or main) | 20 m | restores a criterion cache, snapshots it to `target/criterion_prev`, runs `cargo bench --bench benchmarks -- --baseline main --save-baseline main` (**`continue-on-error`**), then an **inline Python heredoc** (`:244-288`) that fails the job if any `mean.point_estimate` regressed >5% (`threshold = 1.05`) |
| `version-audit` (needs build) | 10 m | downloads the linux artifact and greps `--version` output for `obr\|beads` (warn-only) |
| `e2e-quick` (needs check) | 10 m | `scripts/e2e.sh` with `HARNESS_ARTIFACTS=1`, uploads `e2e_quick_summary.json` |

All third-party actions are **SHA-pinned**. `permissions: contents: read` at the top.

### `conformance.yml` — the only place Go `bd` exists
`workflow_dispatch` (inputs `strict_mode`, `verbose`) + **weekly cron Mon 06:00 UTC** (`:14-15`).
Installs Go 1.23, `git clone --depth 1 https://github.com/Dicklesworthstone/beads.git ../beads-go`,
`go build -o bd ./cmd/beads`, exports `BD_BINARY` (`:53-58`), then runs `scripts/conformance.sh`
with `CONFORMANCE_STRICT`, `CONFORMANCE_TIMEOUT=180`, `NO_COLOR=1`. Uploads
`conformance_summary.json` always and `target/test-artifacts/conformance/` on failure.
**Consequence: on every PR the ~315 conformance tests silently skip via `skip_if_no_bd!()`.**

### `e2e-full.yml` — dispatch + weekly cron Sun 04:00 UTC
Job `e2e-full` (45 m) runs `scripts/e2e_full.sh` with `E2E_FULL_CONFIRM=1`, `E2E_TIMEOUT=180`,
`HARNESS_ARTIFACTS=1`. Job `benchmarks-full` (60 m) runs `scripts/bench.sh` with `BENCH_CONFIRM=1`,
`BENCH_TIMEOUT=600`, restoring/saving a criterion baseline cache and uploading criterion reports.

### `audit.yml` — nightly cron + on any `Cargo.toml`/`Cargo.lock` push
Same two-step as the `security` job in `ci.yml` (advisories advisory-only, yanked = hard fail).

### `release.yml` (14 KB, on `v*` tags)
`preflight` verifies the tag matches `Cargo.toml` version (`:41-49`); a comment at `:32-34` notes
fmt/clippy/test gates were **deliberately removed** from release because "duplicating them here
caused release failures for cosmetic issues". `build-release` is a 5-target matrix incl.
musl; produces archives, SHA-256 checksums, **minisign Ed25519 signatures**; `create-release`
verifies all checksums, generates a changelog and a **CycloneDX SBOM**, creates the GitHub Release,
and emits **SLSA provenance attestations** (`permissions: id-token: write, attestations: write`,
`:25-27`). `update-crates-io` runs `cargo publish`.

### `update-package-manifests.yml`
Dispatch/post-release: downloads release checksums, rewrites `packaging/homebrew/obr.rb`,
`packaging/scoop/br.json`, `packaging/aur/PKGBUILD`, opens a PR on branch `packaging/v<version>`.

### `notify-acfs.yml`
Watches `install.sh`, computes SHA-256, compares to the previous checksum, notifies a downstream
project (dry-run and force inputs available).

### `scripts/`
| Script | Purpose |
|---|---|
| `ci-local.sh` (50) | Mirrors `ci.yml`'s `check`+`test` jobs locally, in order |
| `e2e.sh` (190) | **Quick E2E**: hard-coded list of 6 targets (`:29-36`) — `e2e_basic_lifecycle`, `e2e_ready`, `e2e_create_output`, `e2e_list_priority`, `e2e_errors`, `e2e_harness_demo`. Builds `--release`, runs each under `timeout $E2E_TIMEOUT` (default 180), writes `target/test-artifacts/e2e_quick_summary.json` |
| `e2e_full.sh` (~180) | All `tests/e2e_*.rs`; requires `E2E_FULL_CONFIRM=1`; `--parallel`, `--filter`, `--dataset`; `E2E_TIMEOUT` default 120 |
| `conformance.sh` (262) | Discovers `bd` from `$BD_BINARY`, `/data/projects/beads/.bin/beads`, `~/go/bin/bd`, `~/.local/bin/bd`, `PATH` (`:77-99`); **exits 2** if absent; runs 4 targets: `conformance`, `conformance_edge_cases`, `conformance_labels_comments`, `conformance_schema` (`:35-40`) — **omits `conformance_workflows` and `conformance_text_output`**; writes `conformance_summary.json` with both binary versions |
| `bench.sh` (7.7 K) | `--quick`/`--criterion`/`--compare`/`--save NAME`/`--baseline NAME`; `BENCH_TIMEOUT` 300; writes `benchmark_summary.json` |
| `bench_regression.sh` (24) | `cargo bench --bench benchmarks` with save/compare baseline, then `python3 scripts/check_regression.py target` |
| `check_regression.py` (74) | Reads `target/criterion/*/*/change/estimates.json`, prints a table, fails if any `mean.point_estimate > BENCH_REGRESSION_THRESHOLD` (default **0.10**) — note this is a *different* threshold from `ci.yml`'s 5% |
| `coverage.sh` (91) | `cargo tarpaulin` in `quick`/`full`/`html`/`lib` modes |
| `generate-report.sh` (117) | Runs `cargo test --test e2e_report_generation -- --nocapture generate_and_save_report` to emit `target/reports/report.{html,md}`; has a bizarre fallback that `rustc`-compiles a stub to `/tmp` and prints a note (`:82-103`) |
| `generate_json_baseline.sh` (7.5 K) | Regenerates `tests/fixtures/json_baseline/*.json` from a live binary; deliberately does **not** delete its temp workspace (`:58-60`, an explicit "no `rm -rf` without approval" policy note) |
| `build-release.sh` (6.5 K) | Local cross-build of 4 targets (Windows commented out, `:25`), tarballs + SHA-256 |
| `agent_smoke_test.sh` (80) | Agent-facing smoke test: requires `jq` and `tru`; checks `--format json` and `--format toon` decode, `TOON_DEFAULT_FORMAT` vs `BR_OUTPUT_FORMAT` precedence, and that a not-found error emits `{"error":{"code":"ISSUE_NOT_FOUND"}}` on **stderr** (`:72-79`) |

All shell scripts are `shfmt`-formatted (commit `5eb13fb`) and shellcheck-clean (lefthook gates both).

### Environment-variable surface (test harness)
`HARNESS_ARTIFACTS`, `HARNESS_PRESERVE_SUCCESS`, `HARNESS_PARALLEL`, `HARNESS_PARALLEL_WORKERS`,
`HARNESS_FAIL_FAST`, `HARNESS_RETRY_COUNT`, `HARNESS_SKIP_SLOW`, `HARNESS_TAGS`,
`HARNESS_EXCLUDE_TAGS`, `HARNESS_TAG_MATCH`, `HARNESS_COMMAND_TIMEOUT_SECS`,
`HARNESS_SCENARIO_TIMEOUT_SECS`, `HARNESS_MAX_STDOUT_BYTES`, `HARNESS_MAX_STDERR_BYTES`,
`HARNESS_MAX_ARTIFACT_BYTES`, `HARNESS_MAX_ARTIFACT_DIR_BYTES`, `HARNESS_ARTIFACT_RETENTION_DAYS`,
`HARNESS_MAX_LOG_LINES` (`tests/common/harness.rs:177-246`);
`BD_BINARY`, `BR_BINARY`; `CONFORMANCE_{JSON_LOGS,JUNIT_XML,SUMMARY,FAILURE_CONTEXT,STRICT,TIMEOUT}`;
`BENCH_{DURATION,RSS}_THRESHOLD`, `BENCH_BASELINE_FILE`, `BENCH_STRICT_MODE`,
`BENCH_UPDATE_BASELINE`, `BENCH_INCLUDE_BD`, `BENCH_MEASURE_RSS`, `BENCH_MEASURE_IO`,
`BENCH_ARTIFACTS_DIR`, `BENCH_NOISE_THRESHOLD`, `BENCH_SIGNIFICANCE_LEVEL`, `BENCH_SAMPLE_SIZE`,
`BENCH_REGRESSION_THRESHOLD`; `BR_E2E_STRESS`; `E2E_{TIMEOUT,PARALLEL,DATASET,FULL_CONFIRM}`.

### Docs
`docs/TEST_HARNESS.md` (478) is the authoritative runbook (script table at `:31-39`);
`docs/E2E_COVERAGE_MATRIX.md` (354), `docs/E2E_SYNC_TESTS.md` (362),
`docs/ARTIFACT_LOG_SCHEMA.md` (245, the contract `artifact_validator.rs` enforces).

---

## 7. Risks, oddities, and outright bugs

**Confirmed test failures in the current tree**

1. `tests/package_manifests.rs:264-267` asserts `Cargo.toml` contains `name = "br"`, but the binary
   was renamed to `obr` (`Cargo.toml:13`). `test_cargo_metadata` **fails**.
2. `tests/package_manifests.rs:292-301` asserts the Scoop manifest version equals the Cargo version.
   `Cargo.toml` is `0.1.14`; `packaging/scoop/br.json:3` is `"0.1.13"`; `packaging/aur/PKGBUILD:8`
   is `pkgver=0.1.13`. `test_version_consistency` **fails**.
3. `tests/package_manifests.rs:13` and `:282` look for `packaging/homebrew/br.rb`, but the file was
   renamed to `packaging/homebrew/obr.rb` (and the class to `class Obr < Formula`,
   `packaging/homebrew/obr.rb:13`). Both Homebrew checks silently `return` — the formula is now
   **untested**, including by `test_version_consistency`.

**Structural duplication (large, silent cost)**

4. Every one of the 73 `tests/*.rs` files that says `mod common;` compiles *all* of
   `tests/common/` — and integration test crates are built with `cfg(test)`, so the
   **124 `#[cfg(test)]` unit tests inside `tests/common/`** (scenarios 60, dataset_registry 20,
   harness 16, baseline 8, artifact_validator 7, report_indexer 5, binary_discovery 5,
   json_baseline 3) run **once per test binary** ≈ **9,000+ redundant test executions** per
   `cargo test`, plus 73× the compile cost of ~7,900 lines of helper code.
5. `tests/conformance_edge_cases.rs:23-24` does `#[path = "conformance.rs"] mod conformance;`,
   which pulls the master file's **222 `#[test]` functions** into that binary as well — they are
   compiled and executed twice per run, and the 13,491-line file is compiled twice.

**Coverage gaps / gates that don't gate**

6. Conformance against Go `bd` runs **only** in the weekly `conformance.yml` cron or manual
   dispatch. On PRs `bd_available()` is false and all ~315 conformance tests are skipped with an
   `eprintln!` — a green CI says nothing about parity.
7. `scripts/conformance.sh:35-40` runs only 4 of the 6 conformance targets;
   `conformance_workflows` (multi-step mutating workflows) and `conformance_text_output` (text
   parity) are never run by the CI path.
8. Pre-commit (`lefthook.yml`) and `nix flake check` (`flake.nix:211-221`) both run only
   `cargo test --lib --bins` — none of the 97 integration targets.
9. `nix build` sets `doCheck = false` (`flake.nix:84`).
10. Coverage is decorative: the tarpaulin `fail-under = 50.0` gate (`tarpaulin.toml:32`) is never
    invoked by CI (CI uses `cargo-llvm-cov` with `continue-on-error: true` and
    `fail_ci_if_error: false`, `ci.yml:120-128`).
11. `cargo audit --deny warnings` is `continue-on-error: true` in both `ci.yml:66` and
    `audit.yml:43` — only *yanked* crates hard-fail.
12. The fuzz targets are never run by CI, ship no seed corpus, and all take `&str`, so libFuzzer
    only ever feeds them valid UTF-8 — the byte-level decoding path of JSONL import is unfuzzed.
13. `tests/e2e/*.sh` (3 scripts) are referenced by nothing.

**Inconsistency / fragility**

14. Two different benchmark-regression thresholds: `ci.yml:250` uses 5% (`threshold = 1.05`);
    `scripts/check_regression.py:71` defaults to 10% (`BENCH_REGRESSION_THRESHOLD=0.10`). The CI
    check reimplements the Python script inline as a heredoc rather than calling it.
15. Four independent `bd_available()` implementations with different strictness:
    `conformance.rs:35-57` (checks `BD_BINARY`, rejects `obr` alias),
    `conformance_workflows.rs:35-40+` (same idea, duplicated),
    `conformance_labels_comments.rs:24-29` (just `bd version` succeeds — **would accept an `obr`
    aliased as `bd`**), `conformance_text_output.rs:16-21` (same weak check).
16. `benches/benchmarks.rs` is dead: the manifest `[[bench]] name="benchmarks"` points at
    `storage_perf.rs`, shadowing the same-named auto-inferred target. Verified via `cargo metadata`
    — only one bench target exists and its `src_path` is `benches/storage_perf.rs`.
17. `tests/common/dataset_registry.rs:79-84` hard-codes `/data/projects/{beads_viewer,
    coding_agent_session_search,brenner_bot}` — 3 of the 4 "known datasets" are unavailable
    anywhere but the original author's machine; `bench_real_datasets`/`benchmark_datasets` degrade
    to no-ops. Similarly `scripts/conformance.sh:85` probes `/data/projects/beads/.bin/beads`.
18. `flake.nix:22` pins the `org2jsonl` input to `git+file:///Users/johnw/src/org2jsonl`, and
    `Cargo.toml:64` uses `path = "../org2jsonl"`. The crate cannot be built by anyone who does not
    have that sibling checkout — this also means `cargo publish` in `release.yml:387` would fail.
19. `Cargo.toml:5` says `rust-version = "1.85"` while `rust-toolchain.toml` pins nightly and the
    code uses edition 2024 — the stated MSRV is not real.
20. `Cargo.toml:49-50` comments claim `rich_rust` is "pinned to commit with let_chains fix", but it
    resolves to the crates.io registry version 0.2.0 (`Cargo.lock:3201-3204`), not a git rev.
21. `Cargo.toml:110` openly documents six clippy lints allowed "to unblock CI"; separately the
    entire conformance suite opts out of clippy with `#![allow(clippy::all, …)]` at file scope.
22. Naming drift throughout: the binary is `obr`, but `CLAUDE.md`, `README`, most doc comments,
    `scripts/*.sh` variables (`BR_BIN`, `find_br`), `packaging/scoop/br.json` (`"bin": "br.exe"`,
    release URLs `br-v$version-…`) and the log labels (`run_br`, `br_workspace`) still say `br`.
    `scripts/agent_smoke_test.sh:33-35` even falls back to `BR="obr"` when `command -v br` succeeds
    — i.e. it detects `br` and then runs `obr`.
23. 14 of the 52 `#[ignore]`s carry **no reason string**; the 21 that do are valuable documentation
    of real obr↔bd divergences: bd v0.46.0's `list` returns empty or invalid JSON for
    `--json/--status/--type/--label/--assignee/--limit/--sort created/--priority range/multiple
    filters` (11 separate ignores), bd's `sync` runs `git commit` and fails in non-git dirs
    ("br is intentionally non-invasive"), bd exposes extra `prefix` and `config` fields, and
    "br parent-child dependency blocks children; bd does not" — that last one is a **semantic
    divergence, not a formatting one**.
24. `ci.yml`'s `bench` job is `continue-on-error: true` for the actual benchmark run (`:240`), so a
    benchmark that fails to *run* is invisible; only the regression comparison can fail the job,
    and it skips silently when no baseline exists (`:254-261`).
25. `lefthook.yml` runs `clippy`, `test` and `nix flake check` in parallel, all of which want the
    cargo target-dir lock.
26. `scripts/generate-report.sh:82-103` has a fallback that writes a stub `.rs` to `/tmp`,
    `rustc`-compiles it and runs it just to print a message — dead code with a `/tmp` write.
27. `custom.db` (176 KB SQLite), `temp_test/`, `temp_test_2/` are committed at the repo root and
    look like stray test residue (`temp_test*/` *is* in `.gitignore` but the directories exist on
    disk).
