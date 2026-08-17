# Subsystem Map: `storage` (SQLite engine) — obr / beads_rust

Scope: `src/storage/mod.rs`, `src/storage/schema.rs`, `src/storage/events.rs`,
`src/storage/sqlite.rs`, plus `src/util/hash.rs` and `src/util/id.rs`.

File sizes (lines): `sqlite.rs` 5157, `schema.rs` 1075, `events.rs` 792, `mod.rs` 19,
`util/id.rs` 1104, `util/hash.rs` 276.

---

## 1. Module layout

`src/storage/mod.rs:15-19` — three submodules (`events`, `schema`, `sqlite`), and a
re-export surface of exactly five names:

```rust
pub use sqlite::{IssueUpdate, ListFilters, ReadyFilters, ReadySortPolicy, SqliteStorage};
```

`MutationContext` is **not** re-exported from `storage`; callers reach it as
`crate::storage::sqlite::MutationContext` (the only external `mutate()` caller in
`src/` is a test at `src/cli/commands/delete.rs:591`).

The module doc (`src/storage/mod.rs:1-13`) advertises four properties: WAL mode for
concurrent reads, transaction discipline for atomic writes, dirty tracking for JSONL
export, blocked cache for ready/blocked queries. All four are real; see caveats in §11.

---

## 2. Full SQLite schema

All DDL lives in one const string `SCHEMA_SQL` (`src/storage/schema.rs:9-194`), applied
with `execute_batch`. Every statement is `IF NOT EXISTS`, so the batch is idempotent.
Header comment (`schema.rs:8`): "Schema matches classic bd (Go) for interoperability."

### 2.1 Tables (11 total)

| Table | PK | Purpose | Def |
|---|---|---|---|
| `issues` | `id TEXT` | core entity, 36 columns | `schema.rs:13-56` |
| `dependencies` | `(issue_id, depends_on_id)` | typed edges | `schema.rs:89-100` |
| `labels` | `(issue_id, label)` | tags | `schema.rs:112-117` |
| `comments` | `id INTEGER AUTOINCREMENT` | comment stream | `schema.rs:122-129` |
| `events` | `id INTEGER AUTOINCREMENT` | local-only audit log | `schema.rs:134-144` |
| `config` | `key TEXT` | runtime config (e.g. `issue_prefix`) | `schema.rs:151-154` |
| `metadata` | `key TEXT` | sync bookkeeping | `schema.rs:157-160` |
| `dirty_issues` | `issue_id TEXT` | export queue | `schema.rs:163-167` |
| `export_hashes` | `issue_id TEXT` | last-exported content hash | `schema.rs:171-176` |
| `blocked_issues_cache` | `issue_id TEXT` | materialized blocked set | `schema.rs:180-185` |
| `child_counters` | `parent_id TEXT` | hierarchical-ID counter — **never read or written** | `schema.rs:189-193` |

`issues` columns (`schema.rs:14-49`): `id, content_hash, title, description, design,
acceptance_criteria, notes, status, priority, issue_type, assignee, owner,
estimated_minutes, created_at, created_by, updated_at, closed_at, close_reason,
closed_by_session, due_at, defer_until, external_ref, source_system, source_repo,
deleted_at, deleted_by, delete_reason, original_type, compaction_level, compacted_at,
compacted_at_commit, original_size, sender, ephemeral, pinned, is_template`.

Note (`schema.rs:11-12`): TEXT fields use `DEFAULT ''` rather than NULL specifically
because "bd's sql.Scan doesn't handle NULL well when scanning into string fields." The
Rust side undoes this on read via `SqliteStorage::empty_to_none` (`sqlite.rs:3070-3072`),
so `''` in the DB becomes `None` in the `Issue` model.

### 2.2 CHECK constraints (there are exactly three)

1. `length(title) <= 500` (`schema.rs:16`).
2. `priority >= 0 AND priority <= 4` (`schema.rs:22`) — P0..P4.
3. The **closed-at invariant** (`schema.rs:50-55`):

```sql
CHECK (
    (status = 'closed' AND closed_at IS NOT NULL) OR
    (status = 'tombstone') OR
    (status NOT IN ('closed','tombstone') AND closed_at IS NULL)
)
```

This is bidirectional: closed rows must have `closed_at`, and *non*-closed,
non-tombstone rows must have `closed_at IS NULL`. That is why `update_issue` explicitly
clears `closed_at` on reopen (`sqlite.rs:499-503`) and auto-sets it on close
(`sqlite.rs:494-498`). Tombstones are exempt. Verified by the conformance test at
`schema.rs:798-828`.

### 2.3 Indices (26 named indices)

`issues` (`schema.rs:59-86`):
- `idx_issues_status`, `idx_issues_priority`, `idx_issues_issue_type`,
  `idx_issues_created_at`, `idx_issues_updated_at`
- `idx_issues_assignee` (partial: `WHERE assignee IS NOT NULL`)
- `idx_issues_content_hash`
- `idx_issues_external_ref` (partial) **and** `idx_issues_external_ref_unique`
  (UNIQUE partial) — the unique one is what enforces external-ref uniqueness
- `idx_issues_ephemeral` / `idx_issues_pinned` / `idx_issues_tombstone` (partial, on
  the "interesting" value only)
- `idx_issues_due_at` / `idx_issues_defer_until` (partial `IS NOT NULL`)
- `idx_issues_ready` — the composite hot path (`schema.rs:81-86`):
  `ON issues(status, priority, created_at) WHERE status IN ('open','in_progress') AND
  ephemeral = 0 AND pinned = 0 AND (is_template = 0 OR is_template IS NULL)`

`dependencies` (`schema.rs:101-109`): `idx_dependencies_issue`,
`_depends_on`, `_type`, `_depends_on_type` (composite), `_thread` (partial
`thread_id != ''`), and `idx_dependencies_blocking` — partial composite
`(depends_on_id, issue_id) WHERE type IN ('blocks','parent-child','conditional-blocks','waits-for')`.

`labels`: `idx_labels_label`, `idx_labels_issue` (`schema.rs:118-119`).
`comments`: `idx_comments_issue`, `idx_comments_created_at` (`schema.rs:130-131`).
`events`: `idx_events_issue`, `idx_events_type`, `idx_events_created_at`,
`idx_events_actor` (partial `actor != ''`) (`schema.rs:145-148`).
`dirty_issues`: `idx_dirty_issues_marked_at` (`schema.rs:168`).
`blocked_issues_cache`: `idx_blocked_cache_blocked_at` (`schema.rs:186`).

### 2.4 Foreign keys

Every child table FKs `issue_id -> issues(id) ON DELETE CASCADE`: `dependencies`
(`schema.rs:98`), `labels` (`:116`), `comments` (`:128`), `events` (`:143`),
`dirty_issues` (`:166`), `export_hashes` (`:175`), `blocked_issues_cache` (`:184`),
`child_counters.parent_id` (`:192`).

**Deliberate omission** (`schema.rs:99`): `dependencies.depends_on_id` has *no* FK —
"intentionally removed to allow external issue references". This is what lets
`external:<project>:<capability>` pseudo-IDs and cross-repo IDs live in the edge table.

### 2.5 Triggers

**There are none.** `grep -rn TRIGGER src/` returns nothing. Everything the Go version
might do with triggers (dirty marking, cache invalidation, audit) is done in Rust inside
`mutate()`.

---

## 3. Migration strategy and versioning

`CURRENT_SCHEMA_VERSION: i32 = 1` (`schema.rs:5`). It has never been bumped.

`apply_schema(conn)` (`schema.rs:204-232`) runs, in order:

1. `run_pre_schema_migrations(conn)` (`schema.rs:329-354`)
2. `conn.execute_batch(SCHEMA_SQL)` (`schema.rs:210`)
3. `run_migrations(conn)` (`schema.rs:360-517`)
4. pragmas: `journal_mode=WAL`, `foreign_keys=ON`, `synchronous=NORMAL`,
   `temp_store=MEMORY`, `cache_size=-8000` (8 MB) (`schema.rs:216-227`)
5. `user_version = CURRENT_SCHEMA_VERSION` (`schema.rs:229`)

### 3.1 Pre-schema migrations (`schema.rs:329-354`)

Must run *before* the DDL batch, because `CREATE INDEX` on a legacy table missing a
column would fail (`schema.rs:326-328`).

- Drops `blocked_issues_cache` outright if it lacks `blocked_at`/`blocked_by`/`issue_id`
  (it is a cache; data is regenerated) — `schema.rs:332-340`.
- `ensure_columns` (`schema.rs:310-323`) issues `ALTER TABLE ... ADD COLUMN` for each
  missing column in four declarative tables:
  `ISSUE_COLUMNS` (34 entries, `schema.rs:250-285`), `DEPENDENCY_COLUMNS`
  (`:287-293`), `COMMENT_COLUMNS` (`:295-299`), `EVENT_COLUMNS` (`:301-308`).
- **Unconditionally** `DROP INDEX IF EXISTS idx_issues_ready` (`schema.rs:351`) so the
  batch recreates it with the current `is_template` predicate. Comment justifies it as
  O(1) drop + fast recreate.

`table_exists` (`schema.rs:234-238`) and `column_exists` (`schema.rs:240-248`) both
swallow errors via `.unwrap_or(false)`. `column_exists` string-interpolates the table
name into `pragma_table_info('{table}')` (can't be parameterized) but parameterizes the
column name; callers only pass hardcoded table names.

### 3.2 Post-schema migrations (`schema.rs:360-517`)

Idempotent, all guarded by `IF NOT EXISTS` / existence probes:
- Recreate `blocked_issues_cache` if it still has the legacy `blocked_by_json` shape
  (`schema.rs:363-394`).
- `UPDATE issues SET compaction_level = 0 WHERE compaction_level IS NULL` — bd
  compatibility (`schema.rs:396-407`).
- Add `source_repo` (`:409-420`) and `is_template` (`:422-428`) if absent.
- Recreate the bd-parity index set (`:432-457`).
- Drop legacy index names: `idx_dependencies_issue_id`, `idx_dependencies_depends_on_id`,
  `idx_dependencies_composite`, `idx_labels_issue_id` (`:460-467`).
- Conditionally recreate dependency/label/comment/event indices (`:469-514`).

### 3.3 Version gate on open

`SqliteStorage::open_with_timeout` (`sqlite.rs:100-111`):

```rust
let conn = Connection::open(path)?;
if let Some(timeout) = lock_timeout_ms { conn.busy_timeout(...)?; }
let user_version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
if user_version < CURRENT_SCHEMA_VERSION { apply_schema(&conn)?; }
```

Rationale (test at `sqlite.rs:4618-4635`): opening a current-schema DB must not need a
write lock, so a concurrent `BEGIN IMMEDIATE` holder doesn't block read commands.

Consequences of this design (see §11 for risk framing):
- A DB created by Go `bd` has `user_version = 0`, so `br` migrates it on first open.
- Once `br` has opened a DB, `user_version = 1` forever, so **any future schema change
  requires bumping `CURRENT_SCHEMA_VERSION`** or existing DBs silently skip all
  migrations. There is no per-migration ledger table.
- The pragma block in `apply_schema` is also skipped on the fast path. `journal_mode=WAL`
  is persistent in the DB file so WAL survives; `foreign_keys`, `synchronous`,
  `temp_store`, `cache_size` are **per-connection** and revert to SQLite defaults
  (`foreign_keys=OFF`, `synchronous=FULL`, `temp_store=default`, ~2 MB cache).

`SqliteStorage::open_memory` (`sqlite.rs:118-122`) always calls `apply_schema`, so every
in-memory test has FKs on — which is exactly why `test_pragmas_are_set_correctly`
(`sqlite.rs:4637-4657`) passes while the on-disk fast path does not set them.

---

## 4. The 4-step mutation protocol

Struct: `MutationContext` (`sqlite.rs:22-28`):

```rust
pub struct MutationContext {
    pub op_name: String,
    pub actor: String,
    pub events: Vec<Event>,
    pub dirty_ids: HashSet<String>,
    pub invalidate_blocked_cache: bool,
}
```

API: `record_event` (`:42-53`), `record_field_change` (`:56-74`), `mark_dirty`
(`:76-78`), `invalidate_cache` (`:80-82`). `op_name` is stored but never used
downstream (no logging reads it).

Driver: `SqliteStorage::mutate` (`sqlite.rs:148-195`):

```rust
pub fn mutate<F, R>(&mut self, op: &str, actor: &str, f: F) -> Result<R>
where F: FnOnce(&Transaction, &mut MutationContext) -> Result<R>
```

Body sequence:
1. `transaction_with_behavior(TransactionBehavior::Immediate)` (`:152-154`)
2. `let result = f(&tx, &mut ctx)?;` (`:157`) — `?` drops `tx` on error → rollback
3. insert every `ctx.events` row into `events` (`:159-174`), `created_at` as RFC3339
4. `INSERT OR REPLACE INTO dirty_issues` for every `ctx.dirty_ids` (`:176-182`)
5. if `ctx.invalidate_blocked_cache`, call `rebuild_blocked_cache_impl(&tx)` **inside**
   the transaction (`:184-190`)
6. `tx.commit()` (`:192`)

### The four steps and their invariants

The named "4 steps" are the responsibilities the closure + driver jointly discharge
(documented at `docs/ARCHITECTURE.md:213-232` and
`PROPOSED_ARCHITECTURE_FOR_BR_USING_RUST_BEST_PRACTICES.md:1456-1520`):

| # | Step | Who does it | Invariant enforced | Consequence if skipped |
|---|---|---|---|---|
| 1 | Apply the change (INSERT/UPDATE/DELETE) | the closure, via `tx` | mutation is atomic with its metadata | data inconsistency |
| 2 | Write an audit event | closure calls `ctx.record_event` / `record_field_change`; driver flushes at `:159-174` | every state transition has a durable, actor-attributed row in `events` | lost history |
| 3 | Mark affected issues dirty | closure calls `ctx.mark_dirty`; driver flushes at `:176-182` | JSONL export cannot miss a change (dirty set is the export queue) | export silently drops the change |
| 4 | Invalidate the blocked cache | closure calls `ctx.invalidate_cache`; driver rebuilds at `:184-190` | `ready` / `blocked` queries stay correct after dependency or status edits | stale `blocked_issues_cache`, wrong `br ready` output |

The key structural property is that all four land in **one** transaction with
`Immediate` behavior, so a reader never sees an issue mutated without its event, dirty
mark, or a consistent cache. Rollback coverage is tested at `sqlite.rs:4044-4070`
(closure returns `Err` → the recorded event never appears).

Note that step 4 is a *full* cache rebuild (`DELETE FROM blocked_issues_cache` then
recompute everything, `sqlite.rs:1341-1466`), not a targeted invalidation.

### Which operations use `mutate()`

14 call sites, all in `sqlite.rs`: `create_issue` (:204), `update_issue` (:400),
`delete_issue` (:657), `add_dependency` (:1800), `remove_dependency` (:1852),
`remove_all_dependencies` (:1884), `remove_parent` (:1940), `add_label` (:1972),
`remove_label` (:2011), `remove_all_labels` (:2042), `set_labels` (:2073),
`rename_label` (:2246), `add_comment` (:2332).

Which ones set which flags:
- `invalidate_cache()` — dependency ops, `delete_issue`, and `update_issue` when status
  changed and `!skip_cache_rebuild` (`sqlite.rs:505-507`).
- Every mutating op calls `mark_dirty` on the touched issue; `remove_all_dependencies`
  also marks all counterparties dirty (`sqlite.rs:1923-1926`); `rename_label` marks every
  affected issue (`:2268, :2285`).
- `updated_at` is bumped by hand in each op (e.g. `:1824-1827`, `:1996-1999`,
  `:2019-2022`) — there is no trigger for it.

### Operations that deliberately bypass `mutate()`

- Import path: `upsert_issue_for_import` (`sqlite.rs:3676-3740`) plus
  `sync_labels_for_import` (`:3747`), `sync_dependencies_for_import` (`:3768`),
  `sync_comments_for_import` (`:3802`). Doc comment at `:3670`: "This does NOT trigger
  dirty tracking or events." Correct — imported state came *from* JSONL, so re-marking
  it dirty would cause export ping-pong.
- Key/value writers: `set_config` (`:2655`), `delete_config` (`:2671`), `set_metadata`
  (`:2992`), `delete_metadata` (`:3005`), `set_export_hash(es)` (`:2884`, `:2900`),
  `clear_dirty_issues` (`:2825`), `clear_all_*` (`:2850`, `:2923`, `:3462`). All run in
  autocommit.
- `rebuild_blocked_cache` public entry point (`:1331-1339`) uses a plain (DEFERRED)
  `self.conn.transaction()`.

---

## 5. Transaction, locking, WAL configuration

- `journal_mode = WAL` (`schema.rs:216`) — persistent in the DB file, enables concurrent
  readers during a write.
- `synchronous = NORMAL` (`schema.rs:223`), justified in-comment as safe under WAL.
- `temp_store = MEMORY` (`:225`), `cache_size = -8000` i.e. 8 MB (`:227`).
- `foreign_keys = ON` (`:219`) — per-connection.
- Busy timeout: `conn.busy_timeout(...)` only when a timeout is supplied
  (`sqlite.rs:102-104`). The resolved default is **30 000 ms**, applied in
  `config::open_storage` (`src/config/mod.rs:311-315`) and
  `src/config/mod.rs:379-382` via `.or(Some(30000))`. Overridable by `--lock-timeout`
  (`src/cli/mod.rs:688`) or config key `lock-timeout`/`lock_timeout`
  (`src/config/mod.rs:1136-1137`).
- Write transactions use `TransactionBehavior::Immediate` (`sqlite.rs:154`), taking the
  RESERVED lock up front so two writers serialize on `busy_timeout` instead of failing
  mid-transaction on upgrade.
- `SqliteStorage` holds a single bare `Connection` (`sqlite.rs:17-19`); no mutex, no
  pool, no `Send`/`Sync` sharing. `br` is a one-shot CLI, so concurrency is
  inter-process, mediated entirely by SQLite locks.
- Read-only external access: `query_external_project_capabilities` opens other projects'
  DBs with `OpenFlags::SQLITE_OPEN_READ_ONLY` (`sqlite.rs:3317`).
- `no-db` mode (`src/config/mod.rs:386-405`) opens an **in-memory** DB, sets
  `issue_prefix`, and imports JSONL into it — so `apply_schema` always runs there.

---

## 6. Dirty tracking

Two-level design:

1. **`dirty_issues`** — the coarse queue. Written only by `mutate` step 3
   (`sqlite.rs:176-182`, `INSERT OR REPLACE` with an RFC3339 `marked_at`). Read via
   `get_dirty_issue_count` (`:2796`), `get_dirty_issue_ids` (`:2808`, ordered by
   `marked_at`). Cleared by `clear_dirty_issues` (`:2825`) / `clear_all_dirty_issues`
   (`:2850`) — and by the byte-identical duplicates `clear_dirty_flags` (`:3442`) and
   `clear_all_dirty_flags` (`:3462`).

2. **`export_hashes`** — the fine filter. Stores the content hash at last successful
   export (`:2884-2914`). `get_issues_needing_export(dirty_ids)` (`:2936-2967`) narrows
   the dirty set to rows whose current `content_hash` differs from the stored export
   hash (or that were never exported), so a no-op edit that flipped `updated_at` doesn't
   force a rewrite. **This method has no callers outside its own definition** — the
   incremental-export filter is currently dead code.

Consumers in sync: `compute_staleness` (`src/sync/mod.rs:1740-1743`) combines dirty
count with `metadata` keys; `finalize_export` (`src/sync/mod.rs:1862-1886`) clears dirty
flags for exported *and* skipped-tombstone IDs, records export hashes, and stamps
`jsonl_content_hash` + `last_export_time`; `auto_flush` (`src/sync/mod.rs:1918+`)
short-circuits when `get_dirty_issue_count() == 0`.

Metadata keys (`src/sync/mod.rs:1719-1723`): `jsonl_content_hash`, `last_export_time`,
`last_import_time`.

---

## 7. Content hashing (`src/util/hash.rs`)

SHA-256 over a fixed field order with `\0` separators (`hash.rs:1-4`, `:101-132`).

Fields **included** (`hash.rs:82-96`, 15 in this order): `title`, `description`,
`design`, `acceptance_criteria`, `notes`, `status`, `"P{priority}"`, `issue_type`,
`assignee`, `owner`, `created_by`, `external_ref`, `source_system`, `pinned`,
`is_template`. `None` is written as `""` (`hash.rs:121-123`); booleans as the literals
`"true"`/`"false"` (`:125-127`).

Fields **excluded** (documented `hash.rs:31-38`): `id`, `content_hash` itself (circular),
labels/dependencies/comments/events (separate entities), all timestamps, tombstone
fields, `estimated_minutes`, `due_at`, `defer_until`, `close_reason`,
`closed_by_session`.

Embedded NULs in a field value are replaced with a space before hashing
(`hash.rs:112-119`) so the separator can never be forged from field content.

Output is 64 lowercase hex chars (`hash.rs:129-131`; asserted at `hash.rs:190-196`).

Wiring: `Issue::compute_content_hash` (`src/model/mod.rs:558-562`) delegates to
`util::content_hash`. The trait `ContentHashable` (`hash.rs:11-20`) is implemented for
`Issue`.

Where the hash is set:
- On create, by the **CLI**, not storage: `src/cli/commands/create.rs:254` and `:519`
  (`issue.content_hash = Some(issue.compute_content_hash())`). `create_issue`
  (`sqlite.rs:226`) just persists whatever the caller supplied — pass `None` and the
  column stays NULL.
- On update, by **storage**: `sqlite.rs:616-619` recomputes from the in-memory
  post-update `Issue` and writes it in the same `UPDATE`. Tested at `sqlite.rs:4549-4583`.
- On import, by **sync**: `content_hash(&effective_issue)` at `src/sync/mod.rs:2419`,
  used for collision detection and then stored in `export_hashes`.

Dedup on import uses it: `find_by_content_hash` (`sqlite.rs:3629-3647`) backs
`detect_collision` (`src/sync/mod.rs:2052-2080`), which checks `external_ref` first,
then content hash, then ID.

---

## 8. Query construction

There is no query builder abstraction. Two shapes are used:

**(a) Fixed SQL with `prepare_cached`** for hot single-purpose reads: `get_issue`
(`sqlite.rs:698-717`), `get_blocked_ids` (`:1232`), `id_exists` (`:1693`),
`get_labels` (`:2141`), `get_all_issues_for_export` (`:2703`), etc.

**(b) String-built SQL with a parallel `Vec<Box<dyn ToSql>>`** for filtered lists —
`list_issues` (`:766-919`), `search_issues` (`:927-1047`), `get_ready_issues`
(`:1062-1224`). Pattern: start `WHERE 1=1`, append ` AND ...` per active filter, push
one boxed param per `?`, then
`params.iter().map(AsRef::as_ref).collect::<Vec<&dyn ToSql>>()`. All *values* are bound;
only *identifiers* (sort columns) are interpolated, and those go through a closed
`match` (`:874-898`) with a safe default arm.

Supporting helpers:
- `escape_like_pattern` (`:3371-3375`) escapes `\`, `%`, `_`; always paired with
  `LIKE ? ESCAPE '\'`. Used by title filter (`:854`), search (`:948`), `find_ids_by_hash`
  (`:1706`), `next_child_number` (`:2511`).
- `SQLITE_VAR_LIMIT = 900` chunking to stay under SQLite's 999-variable default:
  `get_issues_by_ids` (`:725`), `get_labels_for_issues` (`:2157`),
  `count_dependencies_for_issues` (`:2545`), `count_dependents_for_issues` (`:2586`),
  `query_external_project_capabilities` (`:3311`).
- `issue_from_row` (`:3075-3137`) — the single 36-column row mapper. **Every issue-select
  query must project exactly those 36 columns in that order.** `get_blocked_issues`
  (`:1473-1505`) appends `bc.blocked_by` as column index 36 and reads it explicitly at
  `:1492`.
- `parse_datetime` (`:3352-3366`) accepts RFC3339 first, then SQLite's
  `%Y-%m-%d %H:%M:%S`, else a `FromSqlConversionFailure`.
- `parse_status` (`:3282-3287`) never fails — unknown strings become `Status::Custom`.
  `parse_issue_type` (`:3289-3291`) falls back to the default instead.

Recursive CTEs are used in three places:
- `check_cycle` (`:345-383`) — forward transitive closure from `depends_on_id`, with an
  optional `type IN ('blocks','parent-child','conditional-blocks')` filter (note:
  **`waits-for` is absent here** even though `DependencyType::is_blocking` includes it,
  `src/model/mod.rs:245-250`). `UNION` (not `UNION ALL`) provides cycle safety.
- `get_ready_issues --parent --recursive` (`:1169-1181`) — descendants over
  `parent-child`; the comment at `:1166-1168` explains `UNION` is required for
  termination under cyclic parent edges.
- `detect_all_cycles` (`:3522-3595`) is *not* a CTE — it loads the whole edge set into a
  `HashMap` and runs an **iterative** DFS with an explicit `(node, neighbor_index)`
  stack, deliberately to avoid stack overflow on deep graphs (`:3517`). Keys are sorted
  for deterministic output (`:3549-3550`).

### Ready-work definition

Documented at `sqlite.rs:1049-1057` and implemented at `:1080-1106`: status in
`('open','in_progress')` (plus `'deferred'` with `--include-deferred`), `NOT EXISTS` in
`blocked_issues_cache`, `defer_until IS NULL OR datetime(defer_until) <= datetime('now')`,
not pinned, not ephemeral, `id NOT LIKE '%-wisp-%'`, not a template. Sorting is
`ReadySortPolicy::{Hybrid, Priority, Oldest}` (`:3271-3280`), Hybrid = P0/P1 bucket first
then `created_at ASC` (`:1198`).

### Blocked-cache rebuild

`rebuild_blocked_cache_impl` (`sqlite.rs:1341-1466`), `MAX_DEPTH = 50` (`:1342`):

1. `DELETE FROM blocked_issues_cache` (`:1345`).
2. Direct blockers: types `('blocks','conditional-blocks','waits-for')` where the blocker
   is not `closed`/`tombstone`, **or** the blocker row is missing and the ID isn't
   `external:%` (orphan edge = blocking) (`:1359-1371`). `parent-child` is deliberately
   excluded here — comment at `:1350-1352`: a child is not blocked merely because its
   parent epic is open.
3. Blockers stored as a JSON array of `"<id>:<status>"` strings, serialized with
   `serde_json` "for safety" (`:1396-1398`).
4. Transitive propagation loop (`:1406-1462`): children of anything already in the cache
   get an entry `"<parent>:parent-blocked"`, iterating until fixpoint or depth 50 (which
   logs a `tracing::warn!`).

`get_blockers` (`:1292-1316`) strips the `:status` annotation and, on malformed JSON,
prints a warning to **stderr** via `eprintln!` (`:1304`) rather than `tracing` — the only
such call in the file — and returns an empty vec.

External dependencies are resolved *outside* the cache, at query time:
`resolve_external_dependency_statuses` (`:1515-1574`) parses `external:<project>:<cap>`
(`parse_external_dependency`, `:3293-3305`), opens each configured peer DB read-only, and
treats a capability as satisfied iff some **closed/tombstone** issue there carries the
label `provides:<cap>` (`:3327-3332`). Unconfigured project or query failure ⇒
unsatisfied, with a `tracing::warn!`. `external_blockers` (`:1584-1660`) then computes
direct external blocks and propagates them down `parent-child` edges with an explicit
worklist + `seen` set. Covered end-to-end by `sqlite.rs:4072-4116`.

---

## 9. Event recording (`src/storage/events.rs`)

Header (`events.rs:8`): "Events are local DB only - never exported to JSONL."

Two parallel event-writing paths exist:

- **The live path** — `MutationContext::record_event` / `record_field_change` buffer
  `Event` structs, and `mutate` writes them (`sqlite.rs:159-174`). This is what every
  storage mutation actually uses.
- **`events.rs`'s typed helpers** — `insert_event` (`events.rs:57-84`) plus 11
  specialized wrappers (`insert_created_event` `:91`, `insert_updated_event` `:100`,
  `insert_status_changed_event` `:125`, `insert_closed_event` `:148`,
  `insert_reopened_event` `:170`, `insert_commented_event` `:192`,
  `insert_dependency_added_event` `:214`, `insert_dependency_removed_event` `:238`,
  `insert_label_added_event` `:261`, `insert_label_removed_event` `:283`,
  `insert_deleted_event` `:305`, `insert_restored_event` `:327`). These are exercised
  only by the module's own tests (`events.rs:506-792`) — production code goes through
  `MutationContext`.

Read path (used in production): `get_events` (`events.rs:355-383`) and `get_all_events`
(`:432-458`), both `ORDER BY created_at DESC, id DESC` with `limit == 0` meaning
unlimited. `count_events` (`:465-472`). Re-exposed on the storage type at
`sqlite.rs:129-140` and consumed by `get_issue_details` (`sqlite.rs:3048-3052`).

`parse_event_type` (`events.rs:475-494`) maps 15 known strings, everything else becomes
`EventType::Custom`. `parse_event_timestamp` (`:413-423`) tries RFC3339 then SQLite
format and **falls back to `Utc::now()`** rather than erroring.

`EVENTS_TABLE_SCHEMA` (`events.rs:19-36`) is a second, slightly divergent copy of the
events DDL: `actor TEXT NOT NULL` (no `DEFAULT ''`), and it names two indices
`idx_events_event_type` / `idx_events_actor` (non-partial) where `SCHEMA_SQL` uses
`idx_events_type` and a partial `idx_events_actor`. `init_events_table`
(`events.rs:501-504`) is only called from `events.rs` tests.

Event types (`src/model/mod.rs:282-323`): `created, updated, status_changed,
priority_changed, assignee_changed, commented, closed, reopened, dependency_added,
dependency_removed, label_added, label_removed, compacted, deleted, restored`, plus
`Custom(String)`.

---

## 10. Storage ↔ sync interaction

Export (`src/sync/mod.rs`):
- `get_all_issues_for_export` (`sqlite.rs:2690-2709`) — includes tombstones (for sync
  propagation), excludes `ephemeral = 1` and `id LIKE '%-wisp-%'`, `ORDER BY id ASC` for
  deterministic output. Called at `src/sync/mod.rs:1290` and `:1603`.
- N+1 avoidance: `get_all_dependency_records` (`sqlite.rs:2719-2754`),
  `get_all_labels` (`:2197-2211`), `get_all_comments` (`:2764-2789`) — each returns a
  `HashMap<issue_id, Vec<_>>`; called at `src/sync/mod.rs:1351-1381`.
- Single-issue variant: `get_issue_for_export` (`sqlite.rs:3389-3400`) populates labels
  + full dependency structs + comments.
- `count_exportable_issues` (`:3472-3481`) is the safety guard against writing an empty
  DB over a populated JSONL.
- `finalize_export` (`src/sync/mod.rs:1862-1886`) → `clear_dirty_issues`,
  `set_export_hashes`, `set_metadata(jsonl_content_hash)`, `set_metadata(last_export_time)`.

Import (`src/sync/mod.rs:2380-2490`):
1. `clear_all_export_hashes` (`:2380`)
2. Phase 1 scan: per issue compute `content_hash`, `detect_collision` →
   `find_by_external_ref` / `find_by_content_hash` / `id_exists`
   (`sqlite.rs:3604`, `:3629`, `:1689`), `determine_action`, record rename mapping,
   accumulate `(target_id, hash)` pairs
3. Phase 2 remap dependency endpoints through the rename map (`:2444-2465`)
4. Phase 3 execute: `upsert_issue_for_import` (`INSERT OR REPLACE`) then
   `sync_issue_relations` → `sync_labels_for_import` / `sync_dependencies_for_import` /
   `sync_comments_for_import` (`src/sync/mod.rs:2533-2542`)
5. `set_export_hashes` (`:2480`), `rebuild_blocked_cache(true)` (`:2484`),
   `set_metadata(last_import_time / jsonl_content_hash)` (`:2487-2489`)

Note: the entire import runs in **autocommit** — `grep -n "transaction" src/sync/mod.rs`
returns nothing.

`sync_dependencies_for_import` uses `INSERT OR IGNORE` (`sqlite.rs:3780`) with defaults
`created_by = "import"`, `metadata = "{}"`, `thread_id = ""`.
`sync_comments_for_import` uses `INSERT OR REPLACE` preserving the JSONL comment `id`
(`sqlite.rs:3814`), whereas live `add_comment` lets SQLite autoincrement
(`insert_comment_row`, `:3852-3864`).

Storage also implements `crate::validation::DependencyStore` (`sqlite.rs:3830-3850`),
forwarding `issue_exists` / `dependency_exists` / `would_create_cycle` — this is how the
validation layer stays storage-agnostic.

---

## 11. Risks, oddities, and clever bits

### Risks

1. **Pragmas are skipped on the fast open path.** `open_with_timeout`
   (`sqlite.rs:105-109`) only calls `apply_schema` when `user_version < 1`. Since
   `foreign_keys`, `synchronous`, `temp_store`, `cache_size` are per-connection pragmas
   set only inside `apply_schema` (`schema.rs:216-227`), every open of an already-migrated
   on-disk DB runs with `foreign_keys = OFF` (SQLite's default), `synchronous = FULL`, no
   memory temp store, and ~2 MB cache. `ON DELETE CASCADE` therefore does not fire in
   normal CLI operation. `test_pragmas_are_set_correctly` (`sqlite.rs:4637-4657`) uses
   `open_memory()`, which always applies the schema, so it cannot catch this.

2. **`INSERT OR REPLACE` + FK cascade hazard in import.** `upsert_issue_for_import`
   (`sqlite.rs:3687-3698`) replaces the issues row. SQLite performs `ON DELETE CASCADE`
   actions for REPLACE conflict resolution *when FKs are enabled*. Labels, deps and
   comments are re-synced immediately afterwards, but `events`, `dirty_issues`,
   `export_hashes` and `blocked_issues_cache` rows for that issue would be silently
   dropped. Today this is masked by risk #1 (FKs are usually off) — meaning behavior
   differs between a freshly-migrated DB and a subsequently-opened one.

3. **No migration ledger; version pinned at 1.** `CURRENT_SCHEMA_VERSION` (`schema.rs:5`)
   has never been bumped, and the gate is `user_version < CURRENT_SCHEMA_VERSION`. Adding
   a column to `ISSUE_COLUMNS` without bumping the constant would be a no-op on every
   existing DB.

4. **Import is not transactional.** `import_from_jsonl` (`src/sync/mod.rs:2380-2490`)
   issues hundreds of autocommit statements; a mid-import failure leaves the DB half
   updated with export hashes already cleared (`:2380`).

5. **Cycle-check TOCTOU.** `add_dependency` runs `would_create_cycle` *before* opening the
   transaction (`sqlite.rs:1791`) and only then enters `mutate` (`:1800`). Two concurrent
   processes can each see "no cycle" and jointly create one. `create_issue` does it right
   — `check_cycle` runs on `tx` inside the closure (`:281`).

6. **`update_issue` read-modify-write TOCTOU.** The issue is fetched before the
   transaction (`sqlite.rs:392-394`), mutated in memory, and the recomputed
   `content_hash` (`:617`) is written inside the transaction. A concurrent writer between
   the read and the `BEGIN IMMEDIATE` produces a `content_hash` that does not describe the
   persisted columns. The one field that *is* handled correctly is the claim guard, which
   re-reads `assignee` inside the transaction specifically to close this race
   (`sqlite.rs:400-426`, doc at `:401-402`).

7. **`rebuild_blocked_cache(false)` is a silent no-op.** `sqlite.rs:1331-1334` returns
   `Ok(0)` when `force_rebuild` is false. All five production call sites pass `true`, so
   the parameter only exists to make the no-op reachable.

8. **`rebuild_blocked_cache` public entry uses a DEFERRED transaction**
   (`sqlite.rs:1335`, plain `conn.transaction()`), unlike `mutate`'s IMMEDIATE. Under
   concurrency this can fail on lock upgrade rather than waiting on `busy_timeout`.

9. **Mixed timestamp encodings in one column.** `insert_comment_row` writes
   `CURRENT_TIMESTAMP` → `'YYYY-MM-DD HH:MM:SS'` (`sqlite.rs:3858-3862`), while
   `create_issue` (`:322`) and `sync_comments_for_import` (`:3820`) write RFC3339.
   `comments` is then sorted `ORDER BY created_at ASC` as text (`:2308`, `:2768`). At
   index 10 RFC3339 has `'T'` (0x54) and SQLite format has `' '` (0x20), so same-second
   rows from the two paths sort in encoding order, not time order. `parse_datetime`
   (`:3352-3366`) absorbs both on read, hiding the inconsistency.

10. **`get_blocked_by_blocks_deps_only` has LEFT JOINs that behave as INNER JOINs.**
    `sqlite.rs:1254-1262`: the `WHERE blocker.status NOT IN (...)` predicate is false for
    NULL, so orphan and external blockers are excluded — the opposite of
    `rebuild_blocked_cache_impl`, which counts orphans as blocking (`:1367-1369`). Two
    different "blocked" definitions coexist (this one is documented as bd-parity for
    `stats`, `:1241-1245`; used at `src/cli/commands/stats.rs:131`).

11. **`waits-for` is missing from `check_cycle`'s blocking filter** (`sqlite.rs:353`)
    even though `DependencyType::is_blocking()` includes it
    (`src/model/mod.rs:245-250`), and `add_dependency` gates the cycle check on
    `is_blocking()` (`sqlite.rs:1791`). A `waits-for` cycle is therefore reachable.

12. **`search_issues` silently drops filters that `list_issues` honors.** No
    `include_deferred` handling (`sqlite.rs:996-998` vs `:820-826`), no
    `updated_before`/`updated_after`, no `sort`/`reverse` (fixed
    `ORDER BY priority ASC, created_at DESC` at `:1031`). Callers passing a populated
    `ListFilters` to `search_issues` get different semantics than with `list_issues`.

13. **`get_issues_needing_export` excludes deleted rows** (`AND i.deleted_at IS NULL`,
    `sqlite.rs:2948`) while `get_all_issues_for_export` deliberately includes tombstones
    (`:2684`). The method is currently unused, so this is latent rather than live.

### Dead / duplicated surface

- `child_counters` (`schema.rs:189-193`) is created, FK'd and listed by `doctor`
  (`src/cli/commands/doctor.rs:199`) and `info` (`src/cli/commands/info.rs:26`), but
  **never inserted into or read**. `next_child_number` (`sqlite.rs:2504-2534`) instead
  does a `LIKE '<parent>.%'` scan and parses the max first segment. The conformance
  test even records that br's column is `last_child` while bd's is `next_child_number`
  (`tests/conformance_schema.rs:1125-1127`).
- Exact duplicate pairs: `clear_dirty_issues` (`:2825`) ≡ `clear_dirty_flags` (`:3442`);
  `clear_all_dirty_issues` (`:2850`) ≡ `clear_all_dirty_flags` (`:3462`);
  `count_issues` (`:1720`) ≡ `count_all_issues` (`:3017`).
- Unused outside their definitions: `set_export_hash` (`:2884`),
  `clear_all_export_hashes_in_tx` (`:3157`), `set_metadata_in_tx` (`:3144`),
  `count_all_issues` (`:3017`), `get_issues_needing_export` (`:2936`).
- All 12 typed helpers in `events.rs` (`:91-342`) plus `init_events_table` (`:501`) and
  `EVENTS_TABLE_SCHEMA` (`:19`) are test-only; production uses `MutationContext`.
- `MutationContext::op_name` (`sqlite.rs:24`) is set but never read.

### Clever bits worth preserving

- Version-gated open (`sqlite.rs:105-109`) makes read-only commands lock-free against a
  concurrent writer, with a dedicated regression test (`:4618-4635`).
- `DROP INDEX IF EXISTS idx_issues_ready` before every DDL batch (`schema.rs:348-351`) —
  a cheap way to evolve a *partial* index definition without a version bump, since
  `CREATE INDEX IF NOT EXISTS` would otherwise keep the stale predicate.
- `UNION` (not `UNION ALL`) in both recursive CTEs as an explicit cycle-termination
  guarantee, with the reasoning written down (`sqlite.rs:1166-1168`).
- `empty_to_none` (`:3070-3072`) cleanly reconciles bd's `DEFAULT ''` storage convention
  with Rust's `Option` model at exactly one boundary.
- Content hash writes `\0` separators and scrubs embedded NULs (`hash.rs:112-119`), so
  field boundaries are unforgeable.
- Atomic claim guard inside the IMMEDIATE transaction with idempotent same-actor re-claim
  (`sqlite.rs:400-426`) — the only place the codebase explicitly closes a TOCTOU window.

---

## 12. ID generation (`src/util/id.rs`)

Format: `<prefix>-<base36 hash>` with optional `.N.M` child path (`id.rs:1-4`).

`IdConfig` (`id.rs:11-31`) defaults: `prefix = "bd"`, `min_hash_length = 3`,
`max_hash_length = 8`, `max_collision_prob = 0.25`. Overridable from config keys
`issue_prefix`/`issue-prefix`/`prefix`, `min_hash_length`, `max_hash_length`,
`max_collision_prob` (`src/config/mod.rs:844-860`).

**Adaptive length** — `optimal_length` (`id.rs:78-92`) walks `min..=max` and returns the
first length where the birthday-problem estimate `1 - e^(-n²/2·36^len)` drops below
`max_collision_prob`.

**Hash** — `generate_id_seed` (`id.rs:170-185`) builds
`title|description|creator|created_at_nanos|nonce`; `compute_id_hash` (`id.rs:192-213`)
SHA-256s it, folds the first 8 bytes into a `u64`, base36-encodes (`base36_encode`,
`:215-226`), left-pads with `'0'` and truncates to the requested length.

**Collision handling** — `IdGenerator::generate` (`id.rs:113-163`), a three-tier ladder:
1. nonces `0..10` at the current length;
2. all collided ⇒ `length += 1`, repeat, up to `max_hash_length`;
3. at max length ⇒ switch to a fixed 12-char hash and increment the nonce without bound
   until free; after nonce > 1000, the "desperate fallback" returns
   `format!("{prefix}-{hash}{nonce}")` (`:156-159`) — note this concatenates the nonce
   onto the hash with no separator, which still parses (`test_desperate_fallback_id_format`,
   `id.rs:1090-1103`).

`exists` is injected as `F: Fn(&str) -> bool`; the CLI passes
`|id| storage.id_exists(id).unwrap_or(false)` (`src/cli/commands/create.rs:167-176`) —
so a DB error is indistinguishable from "free", and the check is outside any transaction.

**Child IDs** — `child_id(parent, n)` = `"{parent}.{n}"` (`id.rs:236-238`). The CLI
(`src/cli/commands/create.rs:133-163`) verifies the parent exists, calls
`storage.next_child_number`, then re-checks the candidate and linearly probes up to
+100 before erroring. There is **no renumbering machinery anywhere in the repo**
(`grep -rn "renumber" src/` is empty); IDs are immutable once assigned. The only ID
rewriting is import-time *renaming* on collision (`src/sync/mod.rs:2431-2465`), which
remaps both `issue.id` and every dependency endpoint.

**Prefix parsing** — `issue_id_separator` (`id.rs:264-278`) splits at the **last** `-`,
which is what makes hyphenated prefixes work (`bead-me-up-3e9` → prefix `bead-me-up`,
hash `3e9`; `document-intelligence-0sa.2` → child path `[2]`; tests `id.rs:931-951`).
`is_likely_hash_segment` (`:244-262`) requires ≥3 chars and, for 4+ chars, at least one
digit — but the function's verdict is ultimately ignored, since both branches of
`issue_id_separator` return the same `last_dash` (`:272-277`); the "fallback" comment at
`:275-276` documents this as intentional for word-like hashes.

`parse_id` (`id.rs:411-447`) rejects: no dash, empty hash, non-`[a-z0-9]` hash (so
uppercase fails — `id.rs:980-983`), and non-numeric child segments. `ParsedId` supports
`is_root`, `depth`, `parent()`, `to_id_string()`, `is_child_of()` (`:343-395`).
`validate_prefix` (`:460-475`) accepts the expected prefix or any in `allowed_prefixes`,
else `PrefixMismatch`.

**Resolution** — `IdResolver::resolve` (`id.rs:601-676`), four steps: lowercase-normalize
→ exact match → if the input has no `-`, prepend `default_prefix-` and retry → substring
match on the hash portion via `find_matching_ids` (`:708-720`), which compares only the
pre-`.` base hash. 0 matches ⇒ `IssueNotFound`, ≥2 ⇒ `AmbiguousId { partial, matches }`.
Storage backs the substring step with `find_ids_by_hash` (`sqlite.rs:1703-1713`), which
searches `id LIKE '%-<escaped>%' ESCAPE '\'` — note the pattern anchors on `-`, so it can
also match inside a hyphenated *prefix*, not just the hash. Used by `show`, `update`,
`close`, `dep`, `reopen`, `graph`, `lint`, `defer`.
