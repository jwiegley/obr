# obr — Performance and Scaling Analysis

Analyst notes. Every nontrivial claim carries a `path:line` citation or a reproducible
measurement. Claims are tagged **CONFIRMED** (traced in code and/or reproduced empirically),
**SUSPECTED** (reasoned but not proven), or **MEASURED** (empirical numbers only).

Repo treated as read-only. All experiments ran in the session scratchpad using the
already-installed `obr 0.1.14` binary
(`/nix/store/3ks37asmlzi6368g26zi9m6436l52077-beads_rust-0.1.14/bin/obr`), whose version matches
`Cargo.toml:3` (`version = "0.1.14"`). Host: darwin 25.5.0 (Apple silicon, APFS).

---

## 0. Executive summary — the first bottleneck a heavy user hits

There are four, in the order a growing repository meets them:

| # | Threshold | Symptom | Severity |
|---|-----------|---------|----------|
| 1 | **any N, any concurrency >= 2** | concurrent `obr` mutations corrupt `.beads/issues.jsonl` (fixed temp filename) and brick the workspace | **critical, silent** |
| 2 | **~1,000 issues** | every mutating command costs ~0.5-1.7 s because auto-flush re-exports the *entire* file | severe, visible as slowness |
| 3 | **32,767 exportable issues** | `finalize_export` dies with "too many SQL variables"; auto-flush silently half-fails, dirty flags never clear, and *every subsequent read command runs a full auto-import* | **critical, hard ceiling** |
| 4 | **~50k issues (Org format)** | export peaks at **810 MB RSS** for a 38 MB file | OOM risk |

The headline answer to "what does a heavy user hit first" is **#2 in practice and #1 in an agent
swarm**: the design decision to auto-flush a *complete* rewrite of the export file after every
single mutating command. Everything else is downstream of it.

Notably, upstream Go `bd` solved exactly this with a **daemon + 5-second debounce + incremental
export** (`web-beads-upstream.md:188-200`: "auto-sync, 5s debounce ... incremental"). obr
deliberately removed the daemon (a stated non-negotiable) but did **not** replace the debounce
with anything — it made the flush synchronous and full. The machinery for incremental export
exists (`dirty_issues`, `export_hashes`, `get_issues_needing_export` at
`src/storage/sqlite.rs:2936`) but `get_issues_needing_export` **has no callers anywhere**. The
optimisation was built and never wired up.

---

## 1. Startup cost per invocation

### 1.1 Floor

MEASURED (20 iterations each, tiny 44-issue workspace):

| command | wall/invocation |
|---|---|
| `/usr/bin/true` (process-spawn floor) | 2.8 ms |
| `obr version` | 6.8 ms |
| `obr list` | 8.4 ms |
| `obr create --no-auto-flush` | 9.8 ms |
| `obr create` (auto-flush on, 44 issues) | 60 ms |
| `obr show <exact-id>` (50k workspace) | 9-19 ms |

obr's own fixed startup is roughly **4-6 ms** on top of process spawn — genuinely good. The
problem is not startup; it is what runs *after* startup.

### 1.2 Migration check on open — cheap, but it skips the PRAGMAs

CONFIRMED. `SqliteStorage::open_with_timeout` (`src/storage/sqlite.rs:100-111`) reads
`PRAGMA user_version` and only calls `apply_schema` when `user_version < CURRENT_SCHEMA_VERSION`.
`CURRENT_SCHEMA_VERSION = 1` (`src/storage/schema.rs:5`) and has never been bumped, so on every
open of an existing DB the whole DDL/migration path is skipped. **That part is good design** —
read commands take no write lock and pay one pragma read, with a dedicated regression test
(`src/storage/sqlite.rs:4618-4635`).

The bug is that all the **per-connection** pragmas live *inside* `apply_schema`
(`src/storage/schema.rs:216-227`):

    conn.pragma_update(None, "journal_mode", "WAL")     // persistent — survives
    conn.pragma_update(None, "foreign_keys", "ON")      // per-connection — LOST
    conn.pragma_update(None, "synchronous", "NORMAL")   // per-connection — LOST
    conn.pragma_update(None, "temp_store", "MEMORY")    // per-connection — LOST
    conn.pragma_update(None, "cache_size", "-8000")     // per-connection — LOST

Consequences on every real CLI invocation against an existing `.beads/beads.db`:

* `foreign_keys = OFF` (SQLite default). All 9 `ON DELETE CASCADE` clauses are inert.
* `synchronous = FULL` instead of NORMAL — every WAL commit does a full fsync barrier. This
  directly multiplies the cost of the autocommit-per-statement import loop (section 4).
* `cache_size` back to ~2 MB and `temp_store` back to disk — the latter matters because *every*
  `ready` and `list` query builds a TEMP B-TREE for ORDER BY (section 3).

Both existing pragma tests are structurally incapable of catching this:
`src/storage/sqlite.rs:4637-4657` uses `open_memory()` (which always calls `apply_schema`), and
`src/storage/schema.rs:544` asserts immediately after `apply_schema`.

I attempted an A/B measurement (import 10k into a DB whose schema was applied in-process vs. in a
prior process). Results were **inconclusive and highly variable** (9.1 s / 12.7 s one trial,
10.4 s / 37.2 s the next — the "faster" configuration lost both times; variance swamps the
effect). So: the *correctness* consequence (FKs off) is CONFIRMED by code reading; the
*performance* consequence is SUSPECTED but I could not isolate it on this host. Note that
`temp_store=MEMORY` and `cache_size=-8000` are exactly the pragmas whose absence would show up in
the ORDER BY temp-B-tree path, so this is worth measuring on a quiet machine.

Also CONFIRMED absent: there is **no `ANALYZE` and no `PRAGMA optimize` anywhere in `src/`**
(`grep -rn ANALYZE src/` -> empty). The database has no `sqlite_stat1`; the planner runs on
default row-count guesses forever. (Adding `ANALYZE` does not fix the dead ready-index; see 3.1.)

### 1.3 Three SQLite opens per mutating command

CONFIRMED (also flagged in `map-cli.md`, with its own regression test
`tests/repro_auto_flush_inefficiency.rs`). `src/main.rs` opens storage for auto-import
(`run_auto_import`, `main.rs:212-252`), the command body opens its own, and `run_auto_flush`
(`main.rs:259-296`) opens a third. Each re-runs `discover_beads_dir` (upward filesystem walk),
`load_config` (up to 5 YAML layers) and a fresh `Connection::open`. At ~1-2 ms per open this is
a ~5 ms tax — negligible next to section 2, but pure waste, and it triples the window in which
two processes can interleave.

### 1.4 The auto-import staleness probe

`compute_staleness` (`src/sync/mod.rs:1739-1776`) is correctly two-tiered: it compares the export
file's mtime against `last_import_time` metadata, and only if the file looks newer does it
SHA-256 the whole file (`compute_jsonl_hash`, `src/sync/mod.rs:2551-2569`).

MEASURED: hashing a 35 MB `issues.jsonl` line-by-line costs roughly 15-20 ms warm — acceptable.
**But** `finalize_export` (`src/sync/mod.rs:1861-1889`) writes `jsonl_content_hash` and
`last_export_time` and *never touches `last_import_time`*. So after every successful auto-flush
the file's mtime is permanently newer than `last_import_time` and **every** subsequent command
re-hashes the entire file until someone runs an import. At 10k ~5 ms; at 50k ~20 ms; at 250k
~100 ms. Small, but a per-invocation O(file) cost that exists only because one metadata key is
not written. That is the benign case. The malignant case is section 5.

---

## 2. The dominant cost: auto-flush re-exports everything, every time

### 2.1 What actually happens on `obr create`

CONFIRMED by code and by `RUST_LOG=debug`. `src/main.rs:133-135` calls `run_auto_flush` after
every mutating command (`is_mutating_command`, `main.rs:139-158`) unless `--no-auto-flush`.
`auto_flush` (`src/sync/mod.rs:1918-1965`) checks `dirty_count` and if nonzero calls
**`export_to_jsonl_with_policy`** — the full-file exporter, not an incremental one.

Direct evidence from a 10,020-issue workspace, one `obr create`:

    DEBUG beads_rust::sync: Auto-flush: exporting dirty issues dirty_count=9
    INFO  beads_rust::sync: Auto-flush complete exported=10021

Nine dirty issues -> 10,021 issues written.

Per flush, `export_to_jsonl_with_policy` (`src/sync/mod.rs:1259-1560`) performs:

1. `history::backup_before_export` (`mod.rs:1285`) -> `files_are_identical` byte-compare against
   the newest backup (`src/sync/history.rs:226`), then `fs::copy` of the **whole file**
   (`history.rs:93`), then `rotate_history` readdir+stat (`history.rs:107-138`, defaults
   `max_count = 100`, `max_age_days = 30`, `history.rs:22-30`).
2. `storage.get_all_issues_for_export()` (`mod.rs:1290`) — full table scan, N `Issue` structs.
3. `analyze_jsonl(output_path)` (`mod.rs:1294`) — **full re-parse of the existing file**
   (`serde_json::from_str::<PartialId>` per line, `mod.rs:1165`), building a `HashSet` of all N
   ids, then a `HashSet` of all N DB ids and a set difference (`mod.rs:1307-1310`).
4. Three more full-table hydrations: `get_all_dependency_records`, `get_all_labels`,
   `get_all_comments` (`mod.rs:1348-1381`) into HashMaps.
5. `issues_for_export.push(issue.clone())` (`mod.rs:1474`) — a **full deep clone** of every
   issue, doubling peak memory.
6. Serialize everything; for Org, build the entire document as one `String`
   (`org_bridge::issues_to_org_text`, `mod.rs:1479`).
7. `writer.flush()` + `sync_all()` (`mod.rs:1522-1526`) — a real fsync barrier (F_FULLFSYNC on
   macOS).
8. `fs::rename` + `chmod 0600` (`mod.rs:1544-1552`).
9. `count_issues_in_jsonl(output_path)` (`mod.rs:1558`) — **a third full parse**, of the file
   just written. For Org this is a full org-mode parse via `count_issues_in_org` ->
   `org2jsonl::org_to_json::org_to_entries_with_keywords` (`mod.rs:1176-1183`).
10. `finalize_export` (`mod.rs:1861`) -> `clear_dirty_issues` (one giant DELETE, section 5) +
    `set_export_hashes` for **all N issues** as N separate autocommit `INSERT OR REPLACE`
    statements (`src/storage/sqlite.rs:2900-2913` — `prepare_cached`, but **no transaction**).

So: **1 full file copy + 2 full file parses + 1 full file write + 4 full table scans + N
autocommit DB writes + 1 fsync — to record a single-field edit.**

### 2.2 MEASURED cost

Synthetic workspaces, ~680 B/issue JSONL, 2 labels/issue, 0.9 deps/issue. Min-of-3 or mean-of-5;
host was noisy, treat +/-30% as the error bar.

| N issues | `create --no-auto-flush` | `create` (auto-flush) | `sync --flush-only` | export file |
|---|---|---|---|---|
| 44 | 9.8 ms | 60 ms | — | 30 KB |
| 1,016 | 47 ms | 450-745 ms | 162 ms | 0.68 MB |
| 10,030 | 47 ms | 1.2-1.7 s | 997 ms (JSONL) / 1.58 s (**Org**) | 6.8 / 7.6 MB |
| 50,021 | 63 ms | 0.85-8.3 s (see section 5) | fails (section 5) | 35.8 / 38.0 MB |

A mutation itself is ~50 ms flat regardless of N. **All the growth is auto-flush.**

Org is consistently ~1.6x slower than JSONL for the same content, and much worse on memory
(section 7). Org is the *default* format (`src/config/mod.rs:33`, `DEFAULT_JSONL_FILENAME =
"issues.org"`) and **is not benchmarked anywhere** (`grep -rn org benches/` -> empty).

### 2.3 The `.br_history` amplification

CONFIRMED. Every flush that changes the file copies the whole file into `.beads/.br_history/`
(`history.rs:93`), retaining up to 100 (`history.rs:26`). A heavy user at 10k issues writes
**~14 MB per mutation** (7 MB export + 7 MB backup) and accumulates up to **700 MB** of history;
at 50k it is 76 MB/mutation and up to 3.8 GB. The backup also runs *before* the data-loss guards
(`mod.rs:1285` vs `mod.rs:1292`), so a refused export still burns a rotation slot.

---

## 3. Query patterns in hot commands

Good news first: there are **no classic N+1s in the hot read paths**. `list` batches labels via
`storage.get_labels_for_issues(&issue_ids)` (`src/cli/commands/list.rs:86`, chunked at 900 vars),
and export batches deps/labels/comments (`mod.rs:1348-1381`). `show` issues a constant number of
queries. The problems are different in kind.

### 3.1 CONFIRMED: the "most important" index is never used

`src/storage/schema.rs:80-86`:

    -- Ready work composite index (most important for performance)
    CREATE INDEX IF NOT EXISTS idx_issues_ready
        ON issues(status, priority, created_at)
        WHERE status IN ('open','in_progress') AND ephemeral = 0 AND pinned = 0
          AND (is_template = 0 OR is_template IS NULL);

`get_ready_issues` (`src/storage/sqlite.rs:1062-1220`) emits:

    ... AND (pinned = 0 OR pinned IS NULL)
        AND (ephemeral = 0 OR ephemeral IS NULL)
        AND id NOT LIKE '%-wisp-%'
    ORDER BY CASE WHEN priority <= 1 THEN 0 ELSE 1 END, created_at ASC

`EXPLAIN QUERY PLAN` on the real 50k database produced by `obr` itself:

    SEARCH issues USING INDEX idx_issues_status (status=?)
    CORRELATED SCALAR SUBQUERY 1
      SEARCH blocked_issues_cache USING COVERING INDEX ... (issue_id=?)
    USE TEMP B-TREE FOR ORDER BY

`idx_issues_ready` does not appear. Three independent reasons, each sufficient:

1. `(pinned = 0 OR pinned IS NULL)` does not *imply* the index's `pinned = 0`, so SQLite cannot
   prove the partial index covers the query. Same for `ephemeral`. (The DDL declares these
   `INTEGER DEFAULT 0` with no NOT NULL — confirmed in `sqlite_master` — so the `OR ... IS NULL`
   is defensive, but it is exactly what kills the index.)
2. Rewriting to strict `pinned = 0 AND ephemeral = 0` with `ORDER BY priority, created_at`
   **still** picks `idx_issues_status` (tested).
3. The `ORDER BY CASE WHEN priority <= 1 ...` expression can never be served by any index, so a
   TEMP B-TREE sort of *every* ready row is unavoidable — even for `--limit 1`.

Running `ANALYZE` on a copy of the DB changed nothing (25.6 ms vs 27.3 ms, identical plan).

**Net effect:** the index is pure write-amplification. It costs an index update on every
insert/status change and serves nothing. `ready --limit 20` at 50k sorts ~14k rows in a temp
B-tree — with `temp_store` back at its disk default because of section 1.2.

Bonus: `AND id NOT LIKE '%-wisp-%'` is a leading-wildcard LIKE evaluated per candidate row; and
`datetime(defer_until) <= datetime('now')` wraps the column in a function, defeating
`idx_issues_defer_until` (`schema.rs:78`).

MEASURED `obr ready --limit 20`: 16.6 ms (1k) -> 20 ms (10k) -> 81 ms (50k).

### 3.2 CONFIRMED: `obr count` materialises every issue to produce an integer

`src/cli/commands/count.rs:67-68`:

    let issues = storage.list_issues(&filters)?;
    let total = issues.len();

`list_issues` (`sqlite.rs:766`) projects all 36 columns and builds a full `Issue` (every `String`)
per row. `SqliteStorage::count_issues` (`sqlite.rs:1720`) and `count_all_issues` (`:3017`) exist
and do `SELECT count(*)`; neither is called by the `count` command.

MEASURED `obr count`: 13 ms (1k) -> 51 ms (10k) -> 213 ms (50k), allocating ~100 MB at 50k.

### 3.3 `list` and `stats`

`list_issues` correctly pushes `LIMIT` into SQL (`sqlite.rs:903-908`), so `list --limit 20` stays
cheap-ish: 9.5 / 17.7 / 32 ms at 1k / 10k / 50k. The residual growth is the
`USE TEMP B-TREE FOR LAST TERM OF ORDER BY` (confirmed via EXPLAIN) on
`ORDER BY priority ASC, created_at DESC` — `idx_issues_priority` serves the first term only. A
composite `(priority, created_at)` index would remove it.

`obr list --json` with no limit: 18 ms / 98 ms / 558 ms, 108 MB peak RSS at 50k.

`stats` (`src/cli/commands/stats.rs:51`) does `storage.list_issues(&all_filters)` — full
materialisation — then `get_blocked_by_blocks_deps_only()` and `get_blocked_ids()`.
MEASURED: 14 / 68 / 336 ms. `blocked` MEASURED: 18 / 51 / 259 ms.

`search` uses `LIKE` scans (`sqlite.rs:927`); no FTS anywhere. MEASURED 44 / 77 / 58 ms with
`--limit 20`; a query matching everything is bounded by the limit, so this is fine in practice —
but it is O(N) LIKE evaluation and will not survive a 250k repo with a low-selectivity term.

### 3.4 The blocked-cache full rebuild

`mutate()` step 4 (`sqlite.rs:184-190`) calls `rebuild_blocked_cache_impl`
(`sqlite.rs:1341-1466`), which does `DELETE FROM blocked_issues_cache` and recomputes from
scratch: one scan of `dependencies` with a LEFT JOIN, one INSERT per blocked issue, then a
parent-child propagation loop that runs at least twice and up to `MAX_DEPTH = 50`
(`sqlite.rs:1342`, `:1403-1414`).

Fires on: any status change (`sqlite.rs:506`), delete (`:683`), dependency add/remove
(`:1835, :1871, :1927, :1959`), and per-dependency during create (`:311`).

MEASURED `obr close --no-auto-flush`: 33 ms (1k, 900 deps) -> 102 ms (10k, 9k deps) ->
312 ms (50k, 45k deps). Linear in |E|, as expected.

This is **parity with upstream**, not a regression: bd also fully rebuilds inside the triggering
transaction (`web-beads-upstream.md:255-259`, which records that the cache replaced a recursive
CTE, 752 ms -> 29 ms at 10k). Keep it. It is not the bottleneck.

One inefficiency: the public `rebuild_blocked_cache` uses a **DEFERRED** transaction
(`self.conn.transaction()`, `sqlite.rs:1335`) unlike `mutate`'s IMMEDIATE, so under concurrency
it can hit `SQLITE_BUSY_SNAPSHOT` on lock upgrade instead of waiting out `busy_timeout`.

---

## 4. Import cost

CONFIRMED: import is entirely non-transactional. `grep -n transaction src/sync/mod.rs` returns
**nothing**. Phase 3 (`src/sync/mod.rs:2466-2477`) is a bare loop:

    for (issue, action) in import_ops {
        process_import_action(storage, &action, &issue, &mut result)?;
    }

`process_import_action` -> `upsert_issue_for_import` (`sqlite.rs:3676`) uses
`self.conn.execute(...)` — **uncached, autocommit**. Then `sync_labels_for_import` (`:3747`),
`sync_dependencies_for_import` (`:3768`) and `sync_comments_for_import` (`:3802`) each do a
`DELETE` plus one `INSERT` per row, all separately autocommitted.

For a typical issue with 2 labels and 1 dependency that is **~7 autocommit transactions per
issue**. With `synchronous = FULL` (because of section 1.2) each is a WAL fsync. At 50k issues,
~350,000 fsyncs.

MEASURED full import into an empty DB:

| N | wall | user | sys |
|---|---|---|---|
| 1,000 | 0.71 s | 0.22 s | 0.27 s |
| 10,000 | 6.70 s | 2.23 s | 2.23 s |
| 50,000 | 40.4 s | 11.5 s | 11.5 s |

Only ~57% of time is CPU and `sys` equals `user` — the classic signature of syscall/fsync
domination. A single `BEGIN IMMEDIATE ... COMMIT` around Phase 3 plus `prepare_cached` would
plausibly be a 5-20x win and would simultaneously fix the "mid-import failure leaves a
half-updated DB" hazard already flagged in `map-sync.md`.

A no-op re-import (hash matches, everything Skips) is 0.09-0.11 s at 10k — the parse and
comparison path is cheap; the writes are everything.

Memory: import peak RSS 10.5 MB at 10k, **208 MB at 50k** — the whole file is parsed into
`Vec<Issue>` before any write (`mod.rs:2224-2275`), deliberate (it makes malformed input safe)
but O(N) resident.

---

## 5. The hard ceiling: 32,767 issues (CONFIRMED, reproduced)

`finalize_export` (`src/sync/mod.rs:1866-1877`):

    let mut clear_ids = result.exported_ids.clone();   // ALL exported issues, not just dirty ones
    ...
    storage.clear_dirty_issues(&clear_ids)?;

`clear_dirty_issues` (`src/storage/sqlite.rs:2825-2843`) builds **one placeholder per id** with
**no chunking**:

    let placeholders: Vec<&str> = issue_ids.iter().map(|_| "?").collect();
    let sql = format!("DELETE FROM dirty_issues WHERE issue_id IN ({})", placeholders.join(","));

Its duplicate `clear_dirty_flags` (`sqlite.rs:3442-3460`) has the same defect. Five *other* call
sites in the same file correctly chunk at `SQLITE_VAR_LIMIT = 900`
(`sqlite.rs:725, 2157, 2545, 2586, 3311`). These two were missed.

SQLite's `SQLITE_MAX_VARIABLE_NUMBER` defaults to **32766** since 3.32.

**Empirically bisected** (fresh workspaces, seed N issues + 1 created issue, then
`obr sync --flush-only`):

| exported issues | result |
|---|---|
| 32,001 | OK |
| 32,766 | OK |
| 32,767 | `Database error: too many SQL variables in DELETE FROM dirty_issues WHERE issue_id IN (?,?,...)` |
| 32,768 | same failure |
| 50,021 | same failure |

### 5.1 Why this is worse than a clean error

`run_auto_flush` swallows every failure at `debug!` level (`src/main.rs:264-296`, ending
`debug!(?e, "Auto-flush failed (non-fatal)")`). So above the ceiling:

* `obr create` **prints success and exits 0** (verified: exit code 0 at 50k).
* The export file *is* written (steps 1-9 complete before `finalize_export`).
* But `clear_dirty_issues` fails **first** in `finalize_export`, so `set_export_hashes`,
  `jsonl_content_hash` and `last_export_time` are all skipped.
* Therefore `dirty_issues` grows monotonically forever (observed 16 -> 20 -> 24 ...), and
  `last_export_time` stays `None` while the on-disk hash no longer matches the stored one.

### 5.2 The death spiral

Because `jsonl_content_hash` is now stale, `compute_staleness` reports `jsonl_newer = true`, so
the **next** command runs a full auto-import. MEASURED on the 50k workspace:

    obr create "spiral 1"     ->  1.19 s
      then obr list --limit 1 ->  8.86 s      <-- full auto-import, imported_count=0
      then obr list --limit 1 ->  0.024 s
    obr create "spiral 2"     ->  0.84 s
      then obr list --limit 1 ->  8.34 s
      then obr list --limit 1 ->  0.065 s
    obr create "spiral 3"     ->  0.98 s
      then obr list --limit 1 ->  9.04 s

Confirmed with `-vv`:

    DEBUG beads_rust::sync: Import path validated input_path=./.beads/issues.jsonl
    DEBUG beads_rust::sync: Auto-import completed imported_count=0

Eight seconds of pure waste, per mutate/read pair, forever. Direct verification that the stored
hash is stale:

    stored jsonl_content_hash == file hash : False
    last_export_time                       : None

Practical answer to "how big can an obr repo get": **32,766 non-ephemeral issues (tombstones
included, since `get_all_issues_for_export` keeps them, `sqlite.rs:2690-2709`)**. Past that the
tool degrades into a state where every command pays a full import and dirty tracking is
permanently broken.

### 5.3 Why no test caught it

`tests/bench_synthetic_scale.rs` has a Medium tier at exactly **50,000** issues
(`bench_synthetic_scale.rs:64-81`) which would hit this. But:

* **Never runs.** `#[ignore]` plus a `BR_E2E_STRESS` gate
  (`bench_synthetic_scale.rs:54-57, 700-701`), and `grep -rn BR_E2E_STRESS .github/ scripts/`
  returns **nothing**.
* **Zero assertions.** `benchmark_synthetic` records `success: bool` per operation
  (`:463`) then *filters failures out* of the averages (`successful_ops`, `:566`). A wholly
  failing command shows up as an omission, not a failure.
* **Generator is O(N^2).** It drives `obr create` **without `--no-auto-flush`** (`:218-231`), so
  generating the 50k tier would take hours and the 250k tier is effectively impossible — which is
  presumably why nobody ever ran it.

---

## 6. Concurrency: CONFIRMED data corruption, caused by the flush design

`export_to_jsonl_with_policy` writes to a **fixed** temp path (`src/sync/mod.rs:1421-1426`):

    let temp_ext = if is_org_format { "org.tmp" } else { "jsonl.tmp" };
    let temp_path = output_path.with_extension(temp_ext);   // .beads/issues.jsonl.tmp — no PID, no nonce

There is no `mkstemp`, no pid suffix, no lock. (PC-4 `validate_temp_file_path`,
`src/sync/path.rs:592`, requires the temp file to sit beside the target, which is why a `TempDir`
was not used — but a unique *name* in the same directory would satisfy it.)

Combined with auto-flush-after-every-mutation, N concurrent `obr` processes all
`File::create(".beads/issues.jsonl.tmp")`, interleave their writes into the same inode, and then
each `fs::rename` it over the real file.

**Reproduction** (10,027-issue workspace, 12 concurrent `obr create`):

    pre : 10027 lines
    post: 10040 lines
    PARSE FAIL line 10034: ':"syn-p899tu","title":"probe nf 1","status":"open",...'
    lines 10039  unique ids 10034  bad 1
    db issues 10039   dirty 12

A truncated JSON fragment (missing its leading `{"id"`) plus 5 duplicate ids. The workspace is
then **bricked** — `analyze_jsonl` hard-errors on the first bad line
(`src/sync/mod.rs:1165-1166`) and it is called from the export safety guard *and* the count
verification *and* import:

    $ obr list --limit 1
    {"error":{"code":"CONFIG_ERROR",
      "message":"Configuration error: Invalid JSON at line 10034: expected value at line 1 column 1", ...}}
    $ obr create "after corruption"
    {"error":{"code":"CONFIG_ERROR", ... same ... }}

Every command fails until a human hand-edits the file. For a tool whose entire premise is
"multiple coding agents share one tracker", this is the most serious finding in this report.

Secondary concurrency observations:

* MEASURED: 6 concurrent creates at 10k -> all six independently exported the *whole* 7 MB file
  (`Auto-flush: exporting dirty issues dirty_count=2/2/4/4/5/6`). ~42 MB written to serve six
  one-line changes.
* Dirty flags are left stranded after concurrent flushes (dirty=6 and dirty=12 in two runs),
  because losers of the rename race silently fail and `run_auto_flush` swallows the error.
* `tests/e2e_concurrency.rs` exists but evidently does not cover concurrent auto-flush against a
  shared export file.

---

## 7. Memory profile

MEASURED peak RSS (`/usr/bin/time -l`):

| operation | 1k | 10k | 50k |
|---|---|---|---|
| `sync --flush-only` (JSONL) | 16.6 MB | 54.1 MB | 234.7 MB |
| `sync --flush-only` (**Org**) | — | **171.2 MB** | **809.8 MB** |
| `sync --import-only` | — | 10.5 MB | 208.6 MB |
| `list --json` (all rows) | — | — | 108.4 MB |

The Org number is the alarming one: **810 MB resident to write a 38 MB file — 21x the output
size**, and 3.2x the JSONL exporter at the same issue count. Causes:

1. `get_all_issues_for_export` materialises all N `Issue`s (`mod.rs:1290`);
2. plus three full HashMaps of deps/labels/comments (`mod.rs:1348-1381`);
3. plus `issues_for_export.push(issue.clone())` — a second full copy (`mod.rs:1474`);
4. plus `org_bridge::issues_to_org_text(&issues_for_export)` returning the **entire document as
   one `String`** (`mod.rs:1479`) before a single byte is written.

Linear extrapolation puts a 100k-issue Org export at ~1.6 GB and 250k at ~4 GB. The JSONL path
streams line by line through the `BufWriter` (`mod.rs:1497-1508`) and only suffers from (1)-(3).

`--no-db` mode rebuilds an in-memory database by importing the whole file on **every** invocation
(`src/config/mod.rs:368-419`). MEASURED at 10k: **2.34 s/command** vs 125 ms normal — 19x slower.
`--no-db` is unusable above a few thousand issues.

---

## 8. Hashing cost

Two hashes, both fine:

* **Content hash** — SHA-256 over 15 NUL-separated fields (`src/util/hash.rs:82-96`). Sub-us per
  issue; ~50 ms for 50k. Never measurable next to the I/O.
* **File hash** — `compute_jsonl_hash` (`src/sync/mod.rs:2551`), whole-file, ~15-20 ms for 35 MB
  warm. Only pathological because it runs on nearly every invocation (section 1.4).

One hashing *fragility* worth recording (also noted in `map-sync.md`; I confirmed byte-level
equality holds today): the export hasher feeds `hasher.update(json)` + `hasher.update(b"\n")` per
line (`mod.rs:1513-1514`), or the whole Org text in one shot (`mod.rs:1486`), while
`compute_jsonl_hash` reconstructs line-by-line with `trim_end_matches(['\n','\r'])` + `"\n"`. I
verified with an independent Python reimplementation that stored and recomputed hashes match for
both formats today. But any future path that emits a `\r`, or an Org document that does not end
in `\n`, permanently desynchronises the two and puts obr into the 5.2 spiral *at any issue
count*.

---

## 9. What the benchmarks measure vs. what they miss

### 9.1 `benches/storage_perf.rs` (the only criterion target)

Header targets (`benches/storage_perf.rs:5-14`): create <1 ms, list(10k) <100 ms,
export(10k) <500 ms, import(10k) <1 s.

Problems, all CONFIRMED:

1. **Sizes stop at 5,000.** Every `for size in [100, 500, 1000, 2000, 5000]`
   (`:397, :525, :553`). The documented 10k target is never exercised, and the 32,767 hard
   ceiling is 6.5x above the largest size benchmarked.
2. **The export benchmark measures the one exporter production never calls.** `bench_export` uses
   `export_to_writer` into an in-memory `Cursor` (`benches/storage_perf.rs:532-536`).
   `grep -rn export_to_writer` shows its only non-test callers are the benches themselves — the
   production path is `export_to_jsonl_with_policy`. So the benchmark excludes, by construction,
   **every single thing that actually costs time**: the history backup copy, the `analyze_jsonl`
   pre-scan, the temp file + `sync_all` fsync + rename, the `count_issues_in_jsonl` re-parse, and
   `finalize_export`'s N autocommit writes. It measures serialization and nothing else. (It also
   benchmarks a *different* code path: `export_to_writer_with_policy` (`mod.rs:1598`) does not
   sort/dedup labels and does not filter expired tombstones.)
3. **Benchmarks run with pragmas production never gets.** `setup_db_with_issues`
   (`benches/storage_perf.rs:112-125`) calls `SqliteStorage::open` on a *brand-new* path, so
   `user_version == 0` and `apply_schema` runs -> `synchronous=NORMAL`, `cache_size=-8000`,
   `temp_store=MEMORY`, `foreign_keys=ON`. The real CLI reopening an existing DB gets none of
   these (section 1.2). All bench numbers are measured on a configuration the product does not
   ship.
4. **No process-level measurement at all.** Nothing benchmarks startup, config discovery, the
   three-opens-per-mutation, the auto-import staleness probe, or auto-flush — i.e. nothing
   benchmarks what users experience.
5. **Org is never benchmarked**, despite being the default format and 1.6x slower / 3.2x fatter.
6. `benches/benchmarks.rs` is dead: `[[bench]] name = "benchmarks"` with an explicit path to
   `storage_perf.rs` shadows it (per `map-tests.md`; `cargo metadata` shows one bench target).

### 9.2 `tests/bench_synthetic_scale.rs` (right idea, broken instrument)

The *right* design — it drives the real binary at 10k/50k/100k/250k (`:60-81`). It fails on five
counts:

1. **Never runs** (`#[ignore]` + `BR_E2E_STRESS`, which appears in no workflow or script).
2. **No assertions**; it prints and writes JSON (`:723-730`), and failures are filtered out of the
   summary (`:566`).
3. **RSS instrumentation is wrong twice over.** `get_peak_rss_bytes` (`:488-511`) reads
   `/proc/self/status` — (a) Linux-only, returning `None` on the maintainer's macOS, and (b) it
   is **the test harness's own RSS**, not the spawned `obr` child's. The one metric that would
   have caught the 810 MB Org export measures the wrong process.
4. **It benchmarks a command that does not exist.** `run_operation(br_path, &["export",
   "--output", ...], ...)` at `:558-563`. There is no `export` subcommand (verified:
   `obr export --output /tmp/x.jsonl` -> `error: unrecognized subcommand 'export'`; the 39
   variants are at `src/cli/mod.rs:707-891`). That operation always fails and is silently dropped
   from the averages.
5. **Generation uses bare `obr create`** (`:218-231`) — O(N^2) bytes written — so the tiers it
   defines are not generatable in reasonable time.

### 9.3 CI gates

Per `map-tests.md` and `.github/workflows/ci.yml:240-288`: the criterion bench job is
`continue-on-error`, and the 5% regression gate is an inline Python heredoc that reimplements
`scripts/check_regression.py` (which defaults to 10%). Coverage (`tarpaulin.toml:32`,
`fail-under = 50.0`) is never invoked. There is no enforced performance gate of any kind.
`docs/superpowers/plans/2026-03-16-productize.md` Chunk 5 (the perf gate) is the one chunk never
completed — exactly consistent with what I found.

### 9.4 Unsourced claim

`README.md:43`: "Listing 10,000 issues takes under 100ms." MEASURED `obr list --json` at 10,030
issues: **98 ms** — technically true, barely, and only for the unlimited-JSON path on a warm
cache. `obr list` (rich mode, default limit) at 10k is 18 ms. The claim survives, but nothing in
the repo ties it to a measurement.

---

## 10. Comparison with upstream Go `bd`

From `web-beads-upstream.md`:

* bd's stated architecture (`:186-200`): "SQLite write (immediate) -> mark dirty -> **5s debounce
  -> incremental** export". obr kept the dirty-marking, dropped the debounce *and* the
  incrementality, and made the flush synchronous. This is the single biggest performance delta,
  and it is a consequence of the (correct) no-daemon decision not being followed through with an
  incremental exporter. **obr already has all the parts**: `dirty_issues`, `export_hashes`,
  `get_issues_needing_export` (`sqlite.rs:2936`, zero callers), and `set_export_hashes`.
* bd's blocked-cache design is identical (full DELETE+INSERT rebuild in the triggering
  transaction, depth-limited to 50) and is documented as having replaced a recursive CTE with a
  752 ms -> 29 ms win at 10k (`web-beads-upstream.md:255-259`). obr's version is parity; 3.4
  confirms it is not the bottleneck.
* obr's read-path latency is genuinely competitive: 9-32 ms for `list --limit 20` across 1k-50k,
  honouring the "local queries complete in milliseconds" claim (`web-beads-upstream.md:195`) that
  upstream makes for itself.
* Upstream's field-reported worst pain points were the daemon, auto-sync and the global
  `~/.beads/default.db` fallback. obr has none of those. Its equivalent pain point is the
  synchronous full flush.
* The conformance suite pins `MIN_BD_VERSION = "0.5.0"` (`tests/common/binary_discovery.rs:13`)
  while all its divergence notes target bd v0.46; bd >= 0.58 deleted SQLite entirely. Any future
  obr-vs-bd *performance* comparison is now against a Dolt-backed tool and is meaningless without
  a version bound.

---

## 11. Ranked recommendations

**P0 — correctness/availability, cheap fixes**

1. **Unique the export temp filename.** `src/sync/mod.rs:1421-1426` — append pid + nonce
   (`issues.jsonl.<pid>.<rand>.tmp`) and widen the PC-4 allowlist (`src/sync/path.rs:47-59`).
   Fixes the section 6 corruption. Add an e2e test running >=8 concurrent `obr create` and
   re-parsing the result.
2. **Chunk `clear_dirty_issues` and `clear_dirty_flags`** at `SQLITE_VAR_LIMIT`
   (`sqlite.rs:2825`, `:3442`) — five sibling call sites already do it. Removes the 32,767
   ceiling. Better still, pass only the ids that were *actually dirty* rather than all exported
   ids (`src/sync/mod.rs:1866`).
3. **Stop swallowing auto-flush errors** (`src/main.rs:295`). At minimum `warn!`; ideally a
   nonzero exit, since the current behaviour reports success while silently corrupting the
   dirty-tracking invariant.
4. **Write `last_import_time` (or a `last_sync_time`) in `finalize_export`**
   (`src/sync/mod.rs:1861-1889`) so the post-flush full-file re-hash stops.

**P1 — the actual scaling fix**

5. **Make auto-flush incremental, or debounce it.** `get_issues_needing_export`
   (`sqlite.rs:2936`) already exists and is unused. Even "if `dirty_count/total < 1%` rewrite only
   the affected records" would collapse the section 2 cost. If incremental rewriting of a sorted
   file is unattractive, a debounce/marker file (flush at most once per T seconds, always on
   `obr sync`) preserves the never-runs-git invariant and matches upstream's proven design.
6. **Wrap import Phase 3 in one IMMEDIATE transaction** (`src/sync/mod.rs:2466-2477`) and use
   `prepare_cached` in `upsert_issue_for_import` / `sync_*_for_import`
   (`sqlite.rs:3676-3830`). Expect a large multiple on the 40 s / 50k import, and it fixes the
   known partial-import hazard.
7. **Move per-connection PRAGMAs out of `apply_schema`** into an unconditional
   `configure_connection` after `Connection::open` (`sqlite.rs:100-111`,
   `schema.rs:216-227`). Add a test that opens on disk, drops, reopens, and asserts
   `PRAGMA foreign_keys == 1`.
8. **Stream the Org export** instead of building one `String` (`mod.rs:1479`), and drop the
   `issues_for_export.push(issue.clone())` copy (`mod.rs:1474`). Together these should cut the
   810 MB peak by well over half.
9. **Skip the redundant parses.** `count_issues_in_jsonl(output_path)` (`mod.rs:1558`) re-parses
   the file just written — the writer already knows the count. For Org it re-runs a full
   org-mode parse.

**P2 — query hygiene**

10. Rewrite `get_ready_issues` predicates to strict equality (`pinned = 0`, `ephemeral = 0`,
    `sqlite.rs:1097-1102`) or add `NOT NULL DEFAULT 0` to those columns, replace the
    `CASE WHEN priority <= 1` ORDER BY with an indexable form, and then either fix or delete
    `idx_issues_ready` (`schema.rs:80-86`) — today it is write-amplification with no reader.
11. Make `obr count` use `SELECT count(*)` (`src/cli/commands/count.rs:67`); `count_issues`
    already exists at `sqlite.rs:1720`.
12. Add a composite `issues(priority, created_at)` index to remove `list`'s temp-B-tree.
13. Replace `id NOT LIKE '%-wisp-%'` (`sqlite.rs:1100`, `:2700`) with a boolean column or a
    prefix-anchored test.
14. Consider `PRAGMA optimize` on close and a periodic `ANALYZE` in `obr doctor` — though note
    ANALYZE did *not* fix the ready-plan, so this is hygiene, not a fix.

**P3 — make the benchmarks tell the truth**

15. Point `bench_export` at `export_to_jsonl_with_policy` against a real temp directory
    (`benches/storage_perf.rs:518-543`), and extend sizes to at least 20k/50k.
16. Fix `get_peak_rss_bytes` (`tests/bench_synthetic_scale.rs:488`) to read the **child's**
    `rusage`/`VmHWM`, and make it work on macOS.
17. Delete or fix the nonexistent `export` operation (`tests/bench_synthetic_scale.rs:558-563`),
    add `--no-auto-flush` to the generator (`:218-231`), and add hard assertions
    (`success == true` for every op; wall-clock ceilings) so a 50k tier failure is a red test
    rather than a JSON footnote.
18. Wire *one* scale tier into CI (40k would have caught the ceiling).
19. Benchmark the Org path, since it is the default.

---

## Appendix A — reproduction recipe

    # gen.py writes N synthetic issues as JSONL (2 labels, ~0.9 blocking deps each)
    mk() { N=$1; W=$2; mkdir -p $W; (cd $W; git init -q .; obr init --prefix syn;
           printf '{"database":"beads.db","jsonl_export":"issues.jsonl"}' > .beads/metadata.json;
           rm -f .beads/issues.org); python3 gen.py $N $W/.beads/issues.jsonl syn; }

    # ceiling
    mk 32766 c1 && (cd c1; obr sync --import-only; obr create --no-auto-flush z; obr sync --flush-only)  # OK
    mk 32767 c2 && (cd c2; obr sync --import-only; obr create --no-auto-flush z; obr sync --flush-only)  # too many SQL variables

    # death spiral
    mk 50000 d && (cd d; obr sync --import-only
      time obr create "x"          # ~1s, exits 0, prints success
      time obr list --limit 1      # ~9s  <- full auto-import, imported_count=0
      time obr list --limit 1)     # ~0.03s

    # concurrency corruption
    mk 10000 r && (cd r; obr sync --import-only; obr sync --flush-only
      for i in $(seq 12); do (obr create "race $i" &); done; sleep 25
      python3 -c "import json;[json.loads(l) for l in open('.beads/issues.jsonl')]")  # raises

    # org memory
    (cd d; printf '{"database":"beads.db","jsonl_export":"issues.org"}' > .beads/metadata.json
     rm -f .beads/issues.jsonl; /usr/bin/time -l obr sync --flush-only)   # ~810 MB maxrss

## Appendix B — measurement caveats

* The host was under variable load; repeated identical runs differed by up to 4x in the worst
  case (import 10k: 9.1 s / 12.7 s / 10.4 s / 37.2 s across four runs). Ratios *within* a single
  interleaved run are trustworthy; absolute numbers are order-of-magnitude.
* Everything was measured against the installed `obr 0.1.14` binary rather than a fresh build,
  because `target/` is absent and building would write into the read-only repo. The binary's
  version matches `Cargo.toml`; the Org export path and the default `issues.org` are both present
  in it, so it postdates the Org migration.
* The 32,766/32,767 bisection is exact and deterministic; it is not a timing measurement.
* The concurrency corruption reproduced on the first attempt with 12 racers and is structurally
  inevitable given the fixed temp path, but it is a race — a lower racer count may not trip it.
