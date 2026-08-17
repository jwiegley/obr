# Integrity, Concurrency, and Crash-Safety Analysis — `obr` (beads_rust)

Every claim is marked **CONFIRMED** (traced in code *and*, where marked "reproduced", demonstrated
empirically against the installed `obr 0.1.14`, which matches `Cargo.toml:3` and current `main` @
`fcbbce2`) or **SUSPECTED** (reasoned from code, not executed).

Experiments ran in `.../scratchpad/exp/` with `HOME` redirected into scratch. `/Users/johnw/src/obr`
was treated read-only; only reads plus one `cp` of `.beads/beads.db` into scratch.

---

## Executive summary

The 4-step mutation protocol is genuinely atomic, `BEGIN IMMEDIATE` is used correctly, and
`busy_timeout` works. **SQLite is not where this system loses data.** Every integrity failure I could
construct lives in the layer *above* SQLite — the export file — because the DB write and the file
write are protected by two different (and in one case, zero) concurrency mechanisms.

Headline: **two concurrent `obr` processes silently lose committed writes from the git-tracked export
file at ~8% per round in a two-issue database**, with exit 0, no warning, no dirty flag left behind,
and `obr doctor` reporting `OK`. Since `.beads/*.db` is gitignored (`.beads/.gitignore:2`), the
export file *is* the durable artifact — a write that never reaches it is a write the team never sees.

A second independent path (content-hash dedup at merge time) permanently wedges the exporter, after
which every subsequent mutation is silently withheld from the file, forever, at exit 0.

| # | Finding | Severity | Status |
|---|---------|----------|--------|
| I-1 | Concurrent auto-flush silently drops committed writes from the export file | Critical | CONFIRMED, 5/60 |
| I-2 | `run_auto_flush` swallows every error at `debug!`; exit 0 | Critical | CONFIRMED, reproduced |
| I-3 | Content-hash dedup annihilates issues at merge, then wedges the exporter | Critical | CONFIRMED, reproduced |
| I-4 | Cycle check outside the transaction -> real deadlock cycles | High | CONFIRMED, 11/25 |
| I-5 | `waits-for` cycles reachable with ZERO concurrency (logic bug) | High | CONFIRMED, reproduced |
| I-6 | `foreign_keys` OFF on every open after the first (PRAGMA gap) | Medium-High | CONFIRMED, reproduced |
| I-7 | Import fully non-transactional; crash leaves partial DB | Medium | CONFIRMED, reproduced |
| I-8 | `update_issue` computes `content_hash` from a pre-transaction read | Medium | CONFIRMED, 10/15 |
| I-9 | `doctor` is content-blind, no cycle check | Medium | CONFIRMED |
| I-10 | Export reads four unsynchronized snapshots | Low-Medium | CONFIRMED in code |
| I-11 | `SqliteStorage::open()` skips `busy_timeout` | Low | CONFIRMED |
| I-12 | No `PRAGMA optimize`, no `application_id`, no process lock | Low | CONFIRMED |
| I-13 | `tests/e2e_concurrency.rs` never inspects the export file | Test gap | CONFIRMED |

---

## 0. What the mutation protocol actually guarantees

### 0.1 `mutate()` is atomic — CONFIRMED
`src/storage/sqlite.rs:148-195`. All four steps inside one IMMEDIATE transaction, single `commit()`:
txn opened IMMEDIATE `:152-154`; (1) closure `:157`; (2) events flushed `:159-174`; (3)
`INSERT OR REPLACE INTO dirty_issues` `:176-182`; (4) blocked-cache rebuild if flagged `:184-190`;
`tx.commit()` `:192`. IMMEDIATE is correct and matches sqlite-rust guidance: takes the write lock at
BEGIN so the read-then-write upgrade (which bypasses the busy handler) never happens. **Preserve this.**

14 call sites, all in sqlite.rs: create_issue:204, update_issue:400, delete_issue:657,
add_dependency:1800, remove_dependency:1852, remove_all_dependencies:1884, remove_parent:1940,
add_label:1972, remove_label:2011, remove_all_labels:2042, set_labels:2073, rename_label:2246,
add_comment:2332.

### 0.2 Writes that bypass `mutate()` — CONFIRMED
Autocommit writes on the bare connection: sqlite.rs:2656 (`set_config`), :2841
(`clear_dirty_issues`), :2851 (`clear_all_dirty_issues`), :2886 (`set_export_hash`, dead), :2924
(`clear_all_export_hashes`), :2993 (`set_metadata`), :3452/:3463 (duplicate dirty clearers), **:3687
(`upsert_issue_for_import`)**, :3754/:3779/:3813 (label/dep/comment import sync).

The import bypass is deliberate and documented at `sqlite.rs:3669-3671` and is the right call — it
prevents import->auto-flush ping-pong. The problem is that **nothing replaces the transaction** (I-7).

`rebuild_blocked_cache` uses a **DEFERRED** txn (`self.conn.transaction()`, `sqlite.rs:1335`) unlike
mutate's IMMEDIATE. Its first statement is a write (`DELETE FROM blocked_issues_cache`, `:1345`) so
the lock is taken immediately in practice — but it is an inconsistency a refactor could weaponize.

### 0.3 The atomic claim guard is correct — CONFIRMED
`sqlite.rs:400-426` re-reads `assignee` *inside* the IMMEDIATE transaction to close the two-agent
claim race. The one place the TOCTOU was recognised and closed properly — and the exception, not the
rule (see I-4, I-8).

---

## I-1 — CRITICAL: concurrent auto-flush silently drops committed writes

**CONFIRMED, reproduced.**

`auto_flush` (`src/sync/mod.rs:1918-1965`) runs after every mutating command (`src/main.rs:133-135`)
and calls `export_to_jsonl_with_policy` (`src/sync/mod.rs:1259`), which:
1. reads the entire issue set — `:1289` -> `sqlite.rs:2690-2709`
2. runs data-loss guards against the current on-disk file — `:1292-1340`
3. hydrates children with three more queries — `:1351`, `:1362`, `:1373`
4. writes temp + `sync_all()` — `:1508-1512`
5. `fs::rename` over the target — `:1530`

then `finalize_export` (`:1861-1886`) clears the dirty flag for **every** id in `exported_ids`
(`:1462` pushes every exported issue) and stamps `jsonl_content_hash`/`last_export_time`.

Crucially `export_to_jsonl_with_policy` takes `storage: &SqliteStorage` (`:1259`) and opens **no
transaction**. Steps 1-5 are unsynchronized across processes. SQLite serializes DB writes; **nothing
serializes the file write.** No lock file exists anywhere (grep for flock/LockFile/.lock in src/
yields only `stdout.lock()` and `lock_timeout` plumbing).

### Losing interleaving
```
A: mutate(a) commits           -> a dirty
A: get_all_issues_for_export() -> S0 (has a, NOT b)
A: guard reads file            -> passes
B: mutate(b) commits           -> b dirty
B: get_all_issues_for_export() -> S1 (has a AND b)
B: rename(temp,file)           -> file = S1   [correct]
B: finalize_export             -> clears dirty for ALL of S1 (a AND b)
A: rename(temp,file)           -> file = S0   [b GONE from file]
A: finalize_export             -> clears dirty for S0
```
End state: DB has `b`, file does not, `dirty_issues` **empty**, both exited 0. Nothing re-exports `b`.

The stale-DB guard cannot catch it: it only checks ids in the *file* absent from the *DB*
(`:1306-1340`, `jsonl_ids.difference(&db_ids)`). Loss runs the other way; and for updates both ids
are present on both sides — it is *content* that is lost, which the guard never inspects.

### Reproduction — creates (4,000 issues, ~0.8s export)
```
round 1: db=4003 org=4002 dirty=1  DIVERGENCE=1
```

### Reproduction — updates (dangerous variant), TWO issues, 60 rounds
```
round 41 DIVERGED: DB=[B-41] FILE=[B-40] dirty=0
round 47 DIVERGED: DB=[B-47] FILE=[B-46] dirty=0
round 49 DIVERGED: DB=[B-49] FILE=[B-48] dirty=0
round 51 DIVERGED: DB=[B-51] FILE=[B-50] dirty=0
round 55 DIVERGED: DB=[B-55] FILE=[B-54] dirty=0
total diverged rounds: 5 / 60 (2 issues only)
```
**~8% per round with a two-issue DB.** Not exotic — the ordinary two-agent case. `dirty=0` every
time, so the divergence is *permanent*.

At 4,000 issues, persisted divergence:
```
>>> DIVERGED at round 20: DB=[BBB-20] FILE=[BBB-updated-8] dirty=0
```
Twelve consecutive rounds of updates lost. Detection surfaces:
- `obr sync --status` -> `Status: JSONL is newer (import recommended)` — **backwards**; the file is stale.
- `obr doctor` -> **all OK**, incl. `OK counts.db_vs_jsonl: Both have 4011 records`.

### Does it corrupt the DB? No.
```
BEFORE: DB=[BBB-20|2026-08-06T18:08:32.920088+00:00]
FILE  : [* TODO [#C] BBB-updated-8]
AFTER : DB=[BBB-20|2026-08-06T18:08:32.920088+00:00]
```
LWW on `updated_at` (`src/sync/mod.rs:2124-2132`) correctly skips the stale file record. Damage is
confined to the export file — the artifact that goes into git and that every fresh clone reads.

### Fixes
1. **Serialize the flush** — run the export inside `BEGIN IMMEDIATE` on the same connection. Costs
   nothing at obr scale, makes the snapshot consistent (fixes I-10 free), makes rename exclusive.
2. Or make `finalize_export` compare-and-clear (only clear if `updated_at` unchanged since snapshot).
3. Or re-read `dirty_issues` immediately before `fs::rename` and abort if it grew.
(1) is right and subsumes the others.

---

## I-2 — CRITICAL: auto-flush swallows every error; exit 0

**CONFIRMED, reproduced.** `src/main.rs:283-296`:
```rust
Err(e) => {
    // Log but don't fail - auto-flush errors shouldn't break the command
    debug!(?e, "Auto-flush failed (non-fatal)");
}
```
plus `debug!`-and-return for beads-dir discovery (`:262-267`) and storage open (`:276-280`).

At default verbosity `debug!` is not emitted (`src/logging.rs:20-86`; `-v0` -> `beads_rust=info` in
release). So **a mutating command whose export failed prints `Created ...` and exits 0.**

Reproduced where the guard legitimately refuses:
```
exit code of plain run:
  exit=0
  db=509 file=6000 dirty=3
```
With `-vv` the only trace is `DEBUG Auto-flush: exporting dirty issues dirty_count=2`. The underlying
error is well-written and never shown:
```json
"error": { "code": "CONFIG_ERROR",
  "message": "Refusing to export stale database that would lose issues.\nDatabase has 7 issues, JSONL has 2 unique issues.\nExport would lose 1 issue(s): dd-zzz99\nHint: Run import first, or use --force to override.",
  "retryable": false }
```
The intent ("shouldn't break the command") is defensible for *transient* failures, but the guards are
sticky, and a sticky invisible refusal becomes unbounded silent divergence. At minimum use `warn!`.

**`--allow-stale` amplifies it**: auto-import skipped (visible WARN), then auto-flush refused
silently. `dirty` climbed 3 -> 4 -> 5 -> 6 across three `obr create` calls, each exit 0 with a
success message, file unchanged. Those issues exist only in a gitignored SQLite file.

---

## I-3 — CRITICAL: content-hash dedup annihilates issues, then wedges the exporter

**CONFIRMED, reproduced.**

### Part A — annihilation
`detect_collision` (`src/sync/mod.rs:2050-2087`): `external_ref` -> **`content_hash`** -> `id` -> new.
The hash (`src/util/hash.rs:62-96`) covers 15 fields and **excludes `id` and all timestamps**
(`:31-38`). So two issues with identical title/description/status/priority/type/assignee/owner/
created_by are by definition the same issue, regardless of id.

Routine scenario: two clones, same actor, both file "Fix the login bug"/"Users cannot log in";
different ids; git merges both lines; then import:
```
file: dd-2nd Fix the login bug / dd-zzz99 Fix the login bug
INFO Import complete created_or_updated=1 skipped=1 tombstone_skipped=0
  Processed: 1 issues
  Skipped: 1 issues (up-to-date)
=== RESULT ===  dd-2nd|Fix the login bug
```
**`dd-zzz99` no longer exists.** Its content was written into `dd-2nd` via
`CollisionAction::Update{existing_id}` (`src/sync/mod.rs:2500-2513`); its own id was never created.
Reported as `Skipped: 1 issues (up-to-date)` — the most reassuring possible phrasing for a
destructive op. Any external reference to `dd-zzz99` now dangles.

Mitigation verified: `created_by` **is** hashed (`src/util/hash.rs:52`), so distinct `--actor` values
differ:
```
dd-149|johnw|37d26c7785f5
dd-28v|agent-two|577fa9f49947
```
But the default actor is the OS username, so **all agents on one machine share it** — obr's primary
deployment shape.

Aggravating: `content_hash` is indexed but **not unique** (`schema.rs:67`) and `find_by_content_hash`
(`sqlite.rs:3629`) has no `ORDER BY`, so with several matching rows the dedup target is unstable.

Good property verified: a *fresh* import does NOT collapse duplicates — Phase 1 detects collisions
against pre-import state (`:2390-2447`) before Phase 3 writes (`:2466-2477`). Rebuilding from a file
with two identical issues preserves both. Loss only occurs when one is already in the DB.

### Part B — permanent wedge
The file now holds an id the DB will never have, so the stale-DB guard (`:1306-1340`) refuses **every**
subsequent export. With I-2 that refusal is invisible:
```
DB ids:   dd-149 dd-28v dd-2nd dd-3ph
FILE ids: dd-2nd dd-zzz99
dirty: 3
  after create 1: exit=0 file=2 db=5 dirty=4
  after create 2: exit=0 file=2 db=6 dirty=5
  after create 3: exit=0 file=2 db=7 dirty=6
```
Permanently wedged: unbounded divergence, every command succeeds, recovery needs
`obr sync --flush-only --force` which the user has no way to know they need.

`obr doctor` does flag this one (`WARN counts.db_vs_jsonl`) but exits 0 — the best existing detection
signal, on a command nobody runs on a schedule.

### Fixes
- Make content-hash dedup opt-in, or restrict to `external_ref`-absent + same `source_repo`.
- If kept: report `deduplicated`, not `skipped (up-to-date)`; record the absorbed id into the
  survivor's `external_ref` (machinery exists — `--rename-prefix` does this at `:2352-2354`).
- Surface export refusals (I-2). The wedge is only catastrophic because it is silent.

---

## I-4 — HIGH: cycle check runs outside the transaction

**CONFIRMED, reproduced.** `add_dependency` (`sqlite.rs:1780-1839`) calls `would_create_cycle` at
`:1791` — **before** `self.mutate(...)` at `:1800`. Two processes each check a pre-transaction
snapshot, both see no cycle, both commit. Contrast `create_issue`, which correctly runs
`Self::check_cycle(tx, ...)` inside the closure (`:281`).

25 rounds of concurrent `obr dep add A B` / `obr dep add B A`:
```
round 2,4,6,8,10,11,16,17,18,21,24: CYCLE CREATED (2 edges)
```
**11/25 ~= 44%.** Result:
```
cy-3d0|cy-2fj|blocks
cy-2fj|cy-3d0|blocks
```
Consequences verified on the corrupted workspace:
- `obr ready` -> `No open issues` — both agents permanently starved, no explanation.
- `obr blocked` -> shows mutual block, presented as normal.
- `obr doctor` -> **all OK, exit 0.**

The codebase *has* a correct detector — `detect_all_cycles` (`sqlite.rs:3522`), explicit-stack DFS,
tested (`tests/storage_deps.rs:497,530,751`) — reachable only from `obr dep cycles`
(`src/cli/commands/dep.rs:787`), not wired into `doctor`. Run manually it finds it:
```
Warning: Found 1 dependency cycle(s):  1. wf-1xa -> wf-1yt -> wf-1xa
```
**Fix:** move the check inside the `mutate` closure on `tx` — `check_cycle(conn, ...)`
(`sqlite.rs:345`) takes `&Connection` and `&Transaction` derefs to it, so ~a one-line change. Wire
`detect_all_cycles` into `doctor`.

---

## I-5 — HIGH: `waits-for` cycles reachable with zero concurrency

**CONFIRMED, reproduced.** Pure logic bug. `check_cycle`'s filter (`sqlite.rs:352-356`):
```rust
"AND type IN ('blocks', 'parent-child', 'conditional-blocks')"
```
omits `'waits-for'`. But `DependencyType::is_blocking()` (`src/model/mod.rs:245-250`) **includes**
`WaitsFor`, and `add_dependency` gates the check on `is_blocking()` (`:1791`). The check runs but
traverses a graph excluding the edge type being added.

Two sequential commands, no concurrency:
```
$ obr dep add wf-1yt wf-1xa --type waits-for   -> Added
$ obr dep add wf-1xa wf-1yt --type waits-for   -> Added
wf-1yt|wf-1xa|waits-for
wf-1xa|wf-1yt|waits-for
```
Effect verified: `obr ready` -> `No open issues`; `obr blocked` shows mutual block; `obr dep cycles`
identifies it. The cycle is honoured by blocking logic — `rebuild_blocked_cache_impl` includes
`waits-for` (`sqlite.rs:1341-1466`), as does `idx_dependencies_blocking` (`schema.rs:107-109`).
**Only the cycle checker disagrees.**

**Fix:** add `'waits-for'` at `sqlite.rs:353`; better, derive the list from `is_blocking()` — four
places currently hand-enumerate these types.

Minor: the `dependencies` PK is `(issue_id, depends_on_id)` with no `type` (`schema.rs:97`), so a
`waits-for` edge blocks a later `blocks` edge between the same pair.

---

## I-6 — MEDIUM-HIGH: `foreign_keys` OFF on every open after the first

**CONFIRMED, reproduced.** Corroborates the sqlite-rust research exactly.

All per-connection PRAGMAs are set only inside `apply_schema` (`schema.rs:216-229`): `journal_mode=WAL`,
`foreign_keys=ON`, `synchronous=NORMAL`, `temp_store=MEMORY`, `cache_size=-8000`, `user_version=1`.
But `open_with_timeout` (`sqlite.rs:105-109`) calls `apply_schema` only when `user_version < 1`, and
`CURRENT_SCHEMA_VERSION = 1` (`schema.rs:5`) has never been bumped. Only `journal_mode` is persistent.

Verified on a copy of this repo's own DB:
```
$ sqlite3 repo-copy.db "PRAGMA user_version; PRAGMA journal_mode; PRAGMA foreign_keys; PRAGMA synchronous;"
1
wal
0      <- foreign_keys OFF
2      <- synchronous FULL, not NORMAL
```
Live scratch workspace after init+create: `PRAGMA foreign_keys` -> `0`.

### What this actually breaks — narrower than it first looks
**There is no `DELETE FROM issues` anywhere in `src/`** (grep empty); `delete_issue`
(`sqlite.rs:657-686`) is a soft tombstone UPDATE. So the nine `ON DELETE CASCADE` clauses have almost
no parent-delete to fire from. Real losses:

1. **Referential integrity on INSERT is unenforced.** Verified:
```
$ sqlite3 ... "INSERT INTO labels VALUES('fk-ghost','orphan'); ..."   -> orphan rows accepted
$ sqlite3 ... "PRAGMA foreign_keys=ON; INSERT INTO labels VALUES('fk-ghost2','orphan');"
Error: FOREIGN KEY constraint failed
```
Constraints are correct but inert. Rust validation currently holds the line — `obr dep add <real>
<nonexistent>` is correctly rejected via `crate::validation::DependencyStore` (`sqlite.rs:3840+`).
The invariant is enforced in **Rust only**, SQL as unrealised backup. Any future path inserting a
child row outside the Rust validator gets no net.

2. **`INSERT OR REPLACE` differs between fresh and existing DBs.** `upsert_issue_for_import`
(`sqlite.rs:3687-3698`) uses `INSERT OR REPLACE`; SQLite cascades on REPLACE *when FKs are enabled*.
First open of a new/legacy DB (FK ON, in-process) silently cascades away that issue's `events`,
`dirty_issues`, `export_hashes`, `blocked_issues_cache`; every later open (FK OFF) does not. Same
input, different side-effect state. Low blast radius (all derived/queue data) but behaviour depends
on invisible history.

3. `synchronous=FULL` instead of `NORMAL` — perf regression, safe direction. cache_size/temp_store
   likewise revert.

### What still works — CONFIRMED
`CHECK` and `UNIQUE`/`PK` are **not** gated by the `foreign_keys` pragma and do fire:
```
CHECK constraint failed: priority >= 0 AND priority <= 4
CHECK constraint failed: (status = 'closed' AND closed_at IS NOT NULL) OR ...
UNIQUE constraint failed: issues.external_ref
```
So closed-at invariant (`schema.rs:50-55`), priority range (`:22`), title length (`:16`) and
external-ref uniqueness (`:69`) are genuinely enforced in SQL. Only FK is dead.

### Why tests miss it
`test_pragmas_are_set_correctly` (`sqlite.rs:4637-4657`) uses `open_memory()`, which always calls
`apply_schema` (`:118-122`); `schema.rs:544` asserts right after `apply_schema`. Both are structurally
incapable of catching a skipped-configuration bug.

**Fix:** extract `configure_connection(&Connection)`, call unconditionally after `Connection::open`;
leave only DDL + `user_version` in `apply_schema`. Regression test: create on disk, drop, reopen,
assert `PRAGMA foreign_keys == 1`. Note `PRAGMA foreign_keys` is a no-op inside a transaction, so
open-time is the correct place.

Related loaded footgun: `CURRENT_SCHEMA_VERSION` pinned at 1 while `run_pre_schema_migrations` probes
columns via `pragma_table_info` (`schema.rs:329-354`). Since the gate is `user_version <
CURRENT_SCHEMA_VERSION`, **adding a column to `ISSUE_COLUMNS` without bumping the constant is a
silent no-op on every existing database.**

---

## I-7 — MEDIUM: import is fully non-transactional

**CONFIRMED, reproduced.** `grep -n "transaction\|BEGIN" src/sync/mod.rs` returns **nothing**.
`import_from_jsonl` (`:2201-2490`) clears all export hashes at `:2380` then runs Phase 3 as a bare
loop of autocommit statements (`:2466-2477` -> `sqlite.rs:3676`).

SIGKILL 0.35s into a 6,000-issue import:
```
killed. DB state: 614      <- of 6000
metadata:                  <- EMPTY
export_hashes: 0
dirty: 0
```

### Recovery is better than expected — CONFIRMED
Metadata (`last_import_time`, `jsonl_content_hash`) is written **last** (`:2481-2486`), so staleness
still says "file newer than DB" and the next command's auto-import re-runs the whole import:
```
file lines before: 6000  db: 614
$ obr create "post-crash issue"   -> Created cr-1ejifu
  db=6001 file=6001
```
A genuinely good property, apparently deliberate. Worth documenting so it is not refactored away.

### Where recovery fails
`--allow-stale` and `--no-auto-import` disable the repair:
```
  before: db=506 file=6000
WARN JSONL is newer than DB; skipping auto-import due to --allow-stale
  AFTER:  db=507 file=6000
```
Crash resilience depends entirely on the export guard firing *and* the user never passing
`--allow-stale`. The guard does fire; it just never says so (I-2).

**Fix:** wrap Phase 3 in one IMMEDIATE transaction. Imports are already fully parsed and validated in
memory before any write (`:2224-2275`), so nothing prevents it; also closes the
`clear_all_export_hashes`-then-crash window.

---

## I-8 — MEDIUM: `update_issue` computes `content_hash` from a pre-transaction read

**CONFIRMED, reproduced.** The issue is fetched at `sqlite.rs:392-394`, **before** `self.mutate(...)`
at `:400`. Inside the txn only supplied fields get SET clauses, but `content_hash` is computed from
the whole in-memory struct (`:616-619`). A concurrent change to a *different* field makes the stored
hash describe (our new values + the other process's **old** values) while the columns hold (our new +
their **new**).

Reproduced by recomputing the hash externally (re-implementing `src/util/hash.rs:62-96` field order):
```
round 1: MISMATCH stored= b82d2bc4d0fa recomputed= cff8322fb29f | title= T-1 desc= D-1
round 2: MISMATCH stored= 30fe7aaa5043 recomputed= a4ed3958fa55 | title= T-2 desc= D-2
... 10 of 15 rounds
final: MATCH stored= 1d9b64361d72 recomputed= 1d9b64361d72
```
**~67% mismatch.** Columns are correct — both writes land; only the derived hash is wrong.

**Blast radius is genuinely small**: `content_hash` is `#[serde(skip)]` (`src/model/mod.rs:374-375`)
so it never reaches the file, and import recomputes it in `normalize_issue` (`src/sync/mod.rs:2151`).
The consequence is a *missed* dedup in `detect_collision` phase 2 — given I-3, arguably the safer
failure. `get_issues_needing_export`'s hash filter is dead code.

Still worth fixing: the invariant "`content_hash` describes the row" is false, and future features
(cross-repo dedup, incremental export) would silently rely on it.
**Fix:** re-read inside the txn, or compute from the post-UPDATE row via `RETURNING`.

---

## I-9 — MEDIUM: `doctor` cannot see the failures that matter

**CONFIRMED.** Checks (`src/cli/commands/doctor.rs`): `check_integrity:283`, `check_merge_artifacts:305`,
`check_export_file:351`, `check_db_count:444`, `check_sync_jsonl_path:502`,
`check_sync_conflict_markers:639`, `check_sync_metadata:706`.

Measured on my three corrupted workspaces:

| Corruption | verdict |
|---|---|
| I-1 content divergence (same counts) | **all OK** incl. `OK counts.db_vs_jsonl: Both have 4011 records` |
| I-4 dependency cycle (permanent deadlock) | **all OK** |
| I-3 export wedge (count mismatch) | `WARN counts.db_vs_jsonl` — caught, exit 0 |

Gaps: (1) no content comparison — the whole I-1 class is invisible; a per-issue hash diff would catch
it cheaply. (2) No cycle check, despite `detect_all_cycles` (`sqlite.rs:3522`) existing, being correct
and tested — one call site away. (3) No orphan check, though FK-off (I-6) permits orphans.
(4) Always exits 0 — `std::process::exit(1)` at `doctor.rs:853,870,947` covers only *operational*
failures, so `obr doctor && deploy` cannot gate on it.

---

## I-10 — LOW-MEDIUM: export reads four unsynchronized snapshots

**CONFIRMED in code; low practical impact.** Four independent statements on `&self.conn`, each its own
implicit read transaction: `get_all_issues_for_export()` (`src/sync/mod.rs:1289` -> `sqlite.rs:2690`),
`get_all_dependency_records()` (`:1351` -> `:2719`), `get_all_labels()` (`:1362` -> `:2197`),
`get_all_comments()` (`:1373` -> `:2764`). In WAL each takes a fresh snapshot.

Deliberately not overstated: the three child queries return **maps keyed by issue id**, so a torn read
mostly means an issue carries slightly newer children than its own columns — not cross-linked or
corrupted data. I could not construct a case where this alone loses information. A correctness smell;
fixing I-1 by wrapping the export in a transaction eliminates it for free.

---

## I-11 — LOW: `busy_timeout` works, but one open path skips it

**CONFIRMED, reproduced.** `open_with_timeout` sets it when supplied (`sqlite.rs:102-104`), and the CLI
always supplies one — `.or(Some(30000))` at `src/config/mod.rs:313` and `:382`. Verified by holding a
write lock 6s from a separate process:
```
-- starting obr create while locked --
LOCK RELEASED
Created sm-3eb: lock contention probe
obr create  ...  5.107 total
```
`obr` waited 5.1s and succeeded. IMMEDIATE + 30s busy timeout is the correct multi-process config and
matches sqlite-rust guidance.

Gap: `SqliteStorage::open()` (`sqlite.rs:91-93`) passes `None`, falling back to rusqlite's default
(documented as "currently 5000ms, but this may be subject to change"). Production caller:
`src/cli/commands/init.rs:46`. Minimal impact today; one crate bump from mattering. Set it
unconditionally at open.

---

## I-12 — LOW: missing SQLite hygiene

**CONFIRMED.**
- **No `PRAGMA optimize`** (`grep -rn "optimize" src/storage/` empty). sqlite.org recommends it just
  before closing each connection for exactly obr's many-short-lived-connections shape; "usually a
  no-op or nearly so". Natural home: `Drop for SqliteStorage`.
- **No `PRAGMA application_id`** (grep empty). Would let `doctor` cheaply reject a wrong-type `.db`.
- **No process-level lock** — fine for the DB (SQLite handles it), exactly the problem for the export
  file (I-1). git-bug uses a PID lock for this reason.
- **Three SQLite opens per mutating command** (auto-import, body, auto-flush); acknowledged by
  `tests/repro_auto_flush_inefficiency.rs`.
- **SUSPECTED (not independently verified):** rusqlite 0.38 / libsqlite3-sys 0.36 bundles SQLite
  3.51.1, inside the WAL-reset corruption window (3.7.0-3.51.2, fixed 3.51.3) whose documented trigger
  is multi-process WAL writers — obr's exact shape. Negligible real-world rate; hygiene, not emergency.

---

## I-13 — Test gap: concurrency tests never look at the export file

**CONFIRMED.** `tests/e2e_concurrency.rs` has 7 tests:
`e2e_concurrent_writes_succeed_with_retry:91`, `e2e_lock_timeout_behavior:162`,
`e2e_concurrent_reads_succeed:248`, `e2e_lock_timeout_timing:313`, `e2e_write_serialization:353`,
`e2e_mixed_read_write_concurrency:417`, `e2e_lock_error_reporting:498`.

`grep -n "jsonl\|issues.org\|export" tests/e2e_concurrency.rs` returns **nothing**. Every assertion
goes through `obr list --json`, i.e. the DB. The suite validates precisely the layer that works
(SQLite serialization) and never touches the layer that fails (the export file).
`tests/storage_export_atomic.rs` covers temp+rename atomicity for a *single* writer — a different
property.

The missing test is small and would have caught I-1: run two `obr update` concurrently, then assert
every issue's DB content matches its content in the export file. My 60-round loop found 5 failures; a
20-round CI loop would be adequately sensitive.

---

## Cross-cutting: where invariants live, and where they drift

| Invariant | SQL | Rust | Enforced by |
|---|---|---|---|
| priority 0..4 | CHECK `schema.rs:22` | validation/ | **Both** (verified) |
| title <= 500 | CHECK `schema.rs:16` | validation/ | **Both** |
| closed => closed_at | CHECK `schema.rs:50-55` | close cmd | **Both** (verified) |
| external_ref unique | UNIQUE `schema.rs:69` | import phase 1 | **Both** (verified) |
| issue id unique | PK `schema.rs:14` | id generator | **Both** |
| child rows reference a real issue | FK x9 | DependencyStore | **Rust only** — FK inert (I-6) |
| no dependency cycles | — | check_cycle | **Rust only, racy** (I-4) + wrong for waits-for (I-5) |
| content_hash describes the row | — | update_issue | **Rust only, racy** (I-8) |
| DB == export file | — | dirty flags + guards | **Neither** (I-1, I-3) |

Everything expressible as a single-row SQL constraint is enforced twice and is solid. Everything
requiring a *graph* or *cross-store* invariant is enforced in Rust only, outside the transaction
boundary, and is racy. The three highest-severity findings (I-1, I-3, I-4) are all in that category.

---

## ID generation — assessed, largely sound

`IdConfig` defaults (`src/util/id.rs:22-30`): prefix `bd`, min 3, max 8, `max_collision_prob` 0.25.
`optimal_length` (`:78-92`) uses `P ~= 1 - e^(-n^2/2*36^len)`. Computed thresholds:

| len | space | holds up to n |
|---|---|---|
| 3 | 46,656 | 163 |
| 4 | 1,679,616 | 983 |
| 5 | 60,466,176 | 5,898 |
| 6 | 2,176,782,336 | 35,389 |
| 7 | 78,364,164,096 | 212,339 |
| 8 | 2,821,109,907,456 | 1,274,035 |

Verified: a 6,001-issue workspace minted `cr-1ejifu` — 6 chars, matching `optimal_length(6000)`. The
adaptive ladder works.

`max_collision_prob = 0.25` is not alarming: it is the probability the *retry ladder gets exercised*,
not a corruption probability. The ladder (`:113-163`): nonces 0..10 at current length -> increment
length to max -> fixed 12-char hash with unbounded nonce -> desperate `{prefix}-{hash}{nonce}` after
nonce > 1000.

**Residual TOCTOU — SUSPECTED, low risk.** The `exists` closure (`src/cli/commands/create.rs:175`,
`:450`) queries outside the eventual insert transaction, so two processes could theoretically settle
on the same id. The seed includes `created_at` at nanosecond resolution (`generate_id_seed`), so it
needs identical title+description+creator+nanosecond — vanishingly unlikely. Critically **the failure
mode is safe**: `id TEXT PRIMARY KEY` (`schema.rs:14`) turns a genuine collision into a loud
constraint violation inside `create_issue`'s transaction, not silent corruption. Could not reproduce;
acceptable as-is.

Against the sync-merge research: obr's ids are content-derived-plus-timestamp, not random, so two
agents filing the *same title at the same instant* is the collision case — which is also the I-3 dedup
case. The two mechanisms interact: distinct ids that collide on *content* get merged (I-3), while
identical ids are rejected by the PK. The dedup path is the dangerous one.

---

## Recommendations, ranked by (severity x cheapness)

1. **Wrap the export in an IMMEDIATE transaction** (I-1, I-10). Highest-value change; fixes the only
   unguarded seam and makes the export snapshot consistent for free.
2. **Change `debug!` to `warn!` in `run_auto_flush`** (`src/main.rs:283-296`) (I-2). One word; turns
   three silent-corruption classes visible.
3. **Move the cycle check inside the transaction** (`sqlite.rs:1791`) (I-4) and **add `'waits-for'` at
   `sqlite.rs:353`** (I-5). Near-one-liners; ideally derive the SQL list from `is_blocking()`.
4. **Extract `configure_connection()` and call it on every open** (I-6). Restores FK enforcement,
   `synchronous=NORMAL`, cache size. Add the reopen regression test.
5. **Wrap import Phase 3 in a transaction** (I-7).
6. **Teach `doctor` to compare content, detect cycles, find orphans; exit non-zero** (I-9). All three
   detectors already exist or are trivial.
7. **Reclassify content-hash dedup** (I-3): report `deduplicated`, preserve absorbed id in
   `external_ref`, consider opt-in.
8. **Add the two-writer export-divergence test** (I-13).
9. Housekeeping: `busy_timeout` unconditionally (I-11); `PRAGMA optimize` on drop + `application_id`
   at creation (I-12); a `CURRENT_SCHEMA_VERSION` bump policy so a new `ISSUE_COLUMNS` entry is not a
   silent no-op (I-6).

## Corrections to prior agents' notes

- The storage map's "ON DELETE CASCADE does not fire in normal CLI use" is true but understates *why
  it barely matters*: there is no `DELETE FROM issues` at all, so the practical loss is INSERT-side
  referential integrity and the REPLACE nondeterminism, not cascade.
- The sqlite-rust note implies broad orphan risk from FK-off. Verified: Rust-level validation
  currently prevents orphans through the CLI. Exposure is latent, not active.
- The sync map lists content-hash duplicate-target instability as a risk. Confirmed, and worse than
  listed: not merely unstable but **destructive**, and it wedges the exporter permanently (I-3). I
  could NOT reproduce the "two content-identical NEW issues in one import both resolve to NewIssue"
  duplicate-creation path as harmful — that direction is actually safe.
- The storage map's `update_issue` TOCTOU note is correct; quantified at ~67%, with a small blast
  radius (columns fine, only the derived hash stale).
