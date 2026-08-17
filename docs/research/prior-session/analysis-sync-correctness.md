# Analysis: sync/merge correctness end-to-end (obr / beads_rust)

Repository: `/Users/johnw/src/obr` (READ-ONLY; nothing there was modified).
All `path:line` citations verified by reading the source during this analysis.
Empirical git results were produced in a throwaway repo under the session scratchpad
(`.../scratchpad/gitexp`) with git 2.55.0-class defaults (no merge driver; `diff3` conflict style observed).

Legend:

* **CONFIRMED-CODE** - I read the exact code path; the conclusion follows mechanically.
* **CONFIRMED-EXPERIMENT** - I ran it (git merges only; the `obr` binary could not be built, see 0.1).
* **SUSPECTED** - reasoning is sound but one step is inferred rather than read/run.

---

## 0. Method and limits

### 0.1 No binary

There is no build tree (`target/{debug,release}/obr` absent) and the crate has a sibling path
dependency (`org2jsonl = { path = "../org2jsonl" }`), so a full two-clone end-to-end reproduction was
out of budget. Everything about `obr`'s own behaviour below is CONFIRMED-CODE, traced statement by
statement. Everything about git's behaviour is CONFIRMED-EXPERIMENT.

### 0.2 There is no merge driver

`rg -n "gitattributes|merge=union"` over `src/`, `tests/`, `docs/`, `.beads/*.md`, `README.md` returns
nothing, and no `.gitattributes` exists at the repo root or in `.beads/`. (CONFIRMED-CODE.)

This matters: the sync-merge web research is largely a warning about `merge=union`, and **obr does not
use it**. obr relies on git's default text driver plus a hard refusal to import a file containing
conflict markers (`ensure_no_conflict_markers`, `src/sync/mod.rs:1091`, called as import step 1 at
`src/sync/mod.rs:2221`). That is the safer choice and should be documented as deliberate. obr's real
correctness problems are elsewhere, and they are worse because they are silent.

---

## 1. The sync model, stated precisely

| Concept | Where | Notes |
|---|---|---|
| Authoritative for queries | SQLite `.beads/beads.db` | gitignored |
| Authoritative for transfer | `.beads/issues.org` (default) or `issues.jsonl` | `DEFAULT_JSONL_FILENAME = "issues.org"`, `src/config/mod.rs:33` |
| DB -> file | `export_to_jsonl_with_policy`, `src/sync/mod.rs:1259` | **whole-file, always all rows** |
| file -> DB | `import_from_jsonl`, `src/sync/mod.rs:2201` | **per-issue, selective** |
| 3-way merge | `three_way_merge`, `src/sync/mod.rs:2868`; driver `execute_merge`, `src/cli/commands/sync.rs:1101` | experimental; broken, see 7 |
| Identity resolution | `detect_collision`, `src/sync/mod.rs:2050` | external_ref -> content_hash -> id |
| Conflict rule | `determine_action`, `src/sync/mod.rs:2089` | tombstone veto, then force, then LWW on `updated_at` |
| Ordering signal | `Issue.updated_at: DateTime<Utc>` | wall clock, `Utc::now()` |

**There is no logical clock anywhere.** `rg -n "lamport|vector clock|logical clock|hlc|monotonic"` over
`src/` returns zero hits, and `struct Issue` (`src/model/mod.rs:369-503`) has no version, sequence, or
writer-identity field that participates in any comparison. `source_repo` exists but appears only in
`SELECT` column lists - it is never read by `src/sync/mod.rs` (verified by grep). (CONFIRMED-CODE.)

**Every mutation stamps `updated_at = Utc::now()` unconditionally**, with no monotonicity guard:

* `update_issue` - `src/storage/sqlite.rs:613-614`
* `add_dependency` / `remove_dependency` - `src/storage/sqlite.rs:1823-1826`, `:1858-1861`
* `add_label` / `remove_label` - `src/storage/sqlite.rs:1995-1998`, `:2019-2022`
* `delete_issue` - `src/storage/sqlite.rs:657` onward

Nothing checks that the new `updated_at` exceeds the old one, nor that it is `<= now`.
`IssueValidator::validate` (`src/validation/mod.rs:32-101`) checks only `updated_at >= created_at`
(`:73-78`) and does not look at dependencies at all. (CONFIRMED-CODE.)

### 1.1 The critical asymmetry

> **Import is per-issue and selective. Export is whole-file and total.**

`import_from_jsonl` may `Skip` any subset of incoming issues (`src/sync/mod.rs:2089-2139`) but always
finishes by writing `METADATA_JSONL_CONTENT_HASH` = hash of the file it just read
(`src/sync/mod.rs:2488-2489`). Meanwhile `export_to_jsonl_with_policy` writes **all** rows from
`get_all_issues_for_export()` (`src/sync/mod.rs:1289`, loop at `:1444-1516`).

Consequence: after any import that skipped anything, DB != file, but `compute_staleness`
(`src/sync/mod.rs:1740`) now reports not-newer (the hash gate at `:1760-1766` is false), so nothing
will ever re-import it. The next mutating command's auto-flush overwrites the file from the DB.
**The skipped incoming change is destroyed in the working tree, silently.** This is the engine behind
most failures below.

### 1.2 Skips are invisible in normal operation

`process_import_action` logs skips at `tracing::debug!` (`src/sync/mod.rs:2522`).
`run_auto_import` (`src/main.rs:213-252`) does not surface `skipped_count` at all - `AutoImportResult`
(`src/sync/mod.rs:1780-1787`) carries only `attempted` and `imported_count`. Only an explicit
`obr sync --import-only` prints anything, and it prints `"Skipped: {n} issues (up-to-date)"`
(`src/cli/commands/sync.rs:990`) - a misleading label, because the same counter also holds
`"Existing is newer"` discards (`src/sync/mod.rs:2128-2130`). No ids are printed.

`run_auto_flush` swallows every error at `debug!` (`src/main.rs:294-297`), and `auto_flush` runs with
`force: false` (`src/sync/mod.rs:1941`), so a refused export is also invisible. (CONFIRMED-CODE.)

---

## 2. Empirical: what git actually does to these files

git 2.55.0 defaults, no attributes file.

### 2.1 JSONL - one issue per line, sorted by id (`ORDER BY id ASC`, `src/storage/sqlite.rs:2701`)

| Situation | Result |
|---|---|
| A edits line n, B edits line n+1 (two **different** issues) | **CONFLICT** |
| A edits line n, B edits line n+2 or further | clean |
| A inserts at position p, B inserts at the **same** position p | **CONFLICT** |
| A inserts at p, B inserts at p+1 or further | clean |
| A and B both edit the **same** line | CONFLICT (by construction) |

(CONFIRMED-EXPERIMENT; distance sweep run over 1..5.)

The adjacency result is practically important and undocumented. In this repository's own tracker
(`.beads/issues.jsonl`, 549 issues), two agents touching *any two consecutive issues in id order*
produce a merge conflict, and `obr sync` then hard-fails on markers. Safe, but frequent, and nothing
in the docs prepares a user for it.

### 2.2 Org - one multi-line block per issue

Emission is `emit_issue_heading` (`src/sync/org_bridge.rs:43-200`): the heading line carries status
keyword + priority cookie + title + tags; every scalar is its own `:KEY: value` line in the
`:PROPERTIES:` drawer, including `:UPDATED_AT:` (`org_bridge.rs:82`).

Measured: A adds `:ASSIGNEE: alice` to `bd-03`; B changes `bd-03`'s priority cookie `[#C]`->`[#A]` -
**semantically disjoint field edits to the same issue** - and git produces:

```
`:CREATED_AT: 2026-01-01T00:00:00+00:00
<conflict-start HEAD>
:UPDATED_AT: 2026-01-02T00:00:00+00:00
:ASSIGNEE: alice
<common-ancestor base>
:UPDATED_AT: 2026-01-01T00:00:00+00:00
<conflict-divider>
:UPDATED_AT: 2026-01-03T00:00:00+00:00
<conflict-end B>
:END:
```

(CONFIRMED-EXPERIMENT.)

`:UPDATED_AT:` is therefore an accidental optimistic-concurrency token: because every mutation bumps it
(section 1) and it occupies one line, *any* concurrent modification of the same issue collides
textually even when the modified fields are disjoint. Org's per-field lines would otherwise have given
field-level merge for free; the shared timestamp line throws that away. Moving `:UPDATED_AT:` (and any
future logical clock) to the end of the drawer would let git 3-way-merge disjoint field edits cleanly -
a design opportunity, not merely a bug, though it depends on wanting field-level merge (section 9).

### 2.3 Which merges are clean, and therefore reach obr's own logic

Only these:

1. Edits to different issues >=2 lines apart (JSONL) or in different heading blocks (Org).
2. Independent creation of new issues that sort into different gaps.
3. A tombstone written on one side while the other side touched nothing nearby.

Everything else lands as conflict markers, and `ensure_no_conflict_markers` (`src/sync/mod.rs:1091`,
called at `:2221`) turns that into a hard error that `--force` cannot bypass (`--force` only sets
`skip_prefix_validation` and `force_upsert`, `src/cli/commands/sync.rs:929-940`). One hole:
**`obr sync --merge` never calls `ensure_no_conflict_markers`** - grep yields callers only at
`src/sync/mod.rs:828` (dead `preflight_import`), `:2221` (import), and
`src/cli/commands/doctor.rs:646`. See 7.4.

---

## 3. Scenario (a): two clones edit the same issue field, both flush, git-merge

### 3.1 The common case: git refuses

Both sides rewrite the same JSONL line, or the same `:UPDATED_AT:` line in Org. Git emits markers.
`obr sync` (any mode except `--merge`) errors at `src/sync/mod.rs:2221`. obr cannot proceed until a
human/agent edits the file. **This is correct and safe**, and it means obr has *no automatic
resolution at all* for same-issue concurrent edits over git. That should be stated plainly; today
`--merge` is advertised as if it did.

### 3.2 The dangerous case: a human resolves the conflict, and obr silently reverts it

Resolution is a text edit. The resolved line/block carries whatever `updated_at` the resolver kept:

```
determine_action(...)                                        src/sync/mod.rs:2089
  match incoming.updated_at.cmp(&existing.updated_at)        src/sync/mod.rs:2121
    Greater -> Update
    Equal   -> Skip { "Equal timestamps" }                   src/sync/mod.rs:2125-2127
    Less    -> Skip { "Existing is newer" }                  src/sync/mod.rs:2128-2130
```

If the resolver kept the other side's version - the whole point of resolving - then on the machine
whose version lost, `incoming.updated_at < existing.updated_at`, so import **Skips**. The DB keeps its
own losing version. `jsonl_content_hash` is stamped with the resolved file's hash
(`src/sync/mod.rs:2488-2489`). The next mutating command's auto-flush exports the whole DB
(`src/sync/mod.rs:1918-1963`) and **overwrites the human's resolution in the working tree**. Nothing is
printed at default verbosity. If the user then commits, the resolution is destroyed in git history too.

**CONFIRMED-CODE. This is the most damaging behaviour on the normal (non-`--merge`) path**, because it
silently undoes deliberate conflict resolution.

The DL-2 "stale database" export guard cannot catch it: it checks only for ids present in the file but
**missing from the DB** (`missing = jsonl_ids.difference(&db_ids)`, `src/sync/mod.rs:1310`). It has no
notion of "the file's version of this id is newer than mine".

### 3.3 Determinism of the outcome on both sides

For a **clean** merge of disjoint per-issue edits, convergence holds: on A, issue X (edited by B) has
local `updated_at = T_base < T_B` -> Update; on B, issue Y (edited by A) has `T_base < T_A` -> Update.
Both DBs end up with {A's Y, B's X}, and both re-export byte-identically because export order is
SQL-enforced (`ORDER BY id ASC`, `src/storage/sqlite.rs:2701`) and labels are sorted+deduped on both
export (`src/sync/mod.rs:1393-1396`) and import (`normalize_issue`, `src/sync/mod.rs:2144-2148`).
(CONFIRMED-CODE.)

**Guarantee G1 (holds): disjoint per-issue edits that git merges cleanly converge deterministically,
provided every incoming `updated_at` exceeds the receiving side's stored `updated_at` for that issue.**
That proviso is exactly what clock skew breaks (section 6).

---

## 4. Scenario (b): same-content issues created independently - the deep one

All CONFIRMED-CODE.

### 4.1 Setup

`content_hash` covers 15 fields (`src/util/hash.rs:82-96`): title, description, design,
acceptance_criteria, notes, status, `"P{priority}"`, issue_type, assignee, owner, **created_by**,
external_ref, source_system, pinned, is_template. It excludes id, all timestamps, labels, dependencies,
comments (`src/util/hash.rs:31-38`).

`created_by` = `config::resolve_actor` = config `actor` key, else `$USER`, else `"unknown"`
(`src/config/mod.rs:1004-1013`). A committed `.beads/config.yaml` with `actor: claude` - entirely
normal for an agent-driven tracker - makes `created_by` identical across all clones.

IDs are *not* content-derived: `generate_id_seed` includes `created_at.timestamp_nanos_opt()`
(`src/util/id.rs:166-184`), so two independent creates of the same title get **different ids**.

`create_issue` performs **no content-hash dedup** (`src/storage/sqlite.rs:203-300`) - duplicates are
freely creatable locally. `content_hash` is indexed but **not unique** (`src/storage/schema.rs:67`), and
`find_by_content_hash` is `query_row` with **no `ORDER BY` and no `LIMIT`**
(`src/storage/sqlite.rs:3629-3648`) - with N matching rows it returns whichever the index scan yields
first.

### 4.2 The trace

Clone A creates "Add rate limiting" at T1 -> `bd-aaa`, hash H, `updated_at = T1`.
Clone B creates the same title at T2 > T1 -> `bd-bbb`, hash H, `updated_at = T2`.
Both flush; git merges both new lines cleanly (different gaps - section 2.1, roughly (N-1)/N of the time).
The merged file contains **both** lines.

**Import on clone A** (DB = {bd-aaa@T1}). Phase 1 (`src/sync/mod.rs:2382-2444`) evaluates *every* line
against the **pre-import DB state** - no Phase-3 write has happened yet:

* `bd-aaa`: `find_by_content_hash(H)` -> `bd-aaa`. target == id. T1 == T1 -> **Skip("Equal timestamps")**.
* `bd-bbb`: `find_by_content_hash(H)` -> **`bd-aaa`**. target != id ->
  `renames["bd-bbb"] = "bd-aaa"` (`src/sync/mod.rs:2429-2431`). T2 > T1 -> **Update{bd-aaa}**.

Phase 2 (`:2446-2464`) rewrites `bd-bbb`'s id to `bd-aaa` and remaps both ends of every dependency.
Phase 3 upserts. **Result on A: one row, id `bd-aaa`, carrying B's content. `bd-bbb` does not exist.**

**Import on clone B** (DB = {bd-bbb@T2}):

* `bd-aaa`: `find_by_content_hash(H)` -> `bd-bbb`. rename bd-aaa->bd-bbb. T1 < T2 -> **Skip("Existing is newer")**.
* `bd-bbb`: matches itself, T2 == T2 -> **Skip("Equal timestamps")**.

**Result on B: one row, id `bd-bbb`. `bd-aaa` does not exist.**

**A fresh clone C** (empty DB) importing the same file: both lines evaluate against an empty DB, so
both return `NewIssue` -> both `Insert`. **Result on C: two rows, `bd-aaa` and `bd-bbb`, with identical
`content_hash = H`.**

Three clones, three different states, from one identical input file.

### 4.3 The consequence: both clones are permanently wedged

Clone A's DB has {bd-aaa}; the working file has {bd-aaa, bd-bbb}. Export's DL-2 guard
(`src/sync/mod.rs:1306-1337`) computes `missing = jsonl_ids - db_ids = {bd-bbb}` and **refuses**:

> `Refusing to export stale database that would lose issues. ... Hint: Run import first, or use --force`

But import is now a no-op: `jsonl_content_hash` matches, so `compute_staleness` says not-newer
(`src/sync/mod.rs:1760-1766`) and `execute_import` short-circuits (`src/cli/commands/sync.rs:882-908`).

`auto_flush` runs with `force: false` (`src/sync/mod.rs:1941`) and `run_auto_flush` swallows the error
at `debug!` (`src/main.rs:294-297`). **So every subsequent `obr create/update/close` on clone A
succeeds, marks the issue dirty, and silently fails to export.** The DB drifts from the working tree
indefinitely with zero user-visible signal. Mirrored on clone B.

The only exits are `obr sync --flush-only --force` (which drops the other clone's id from the file for
everyone) or manual surgery. **CONFIRMED-CODE. A self-inflicted deadlock with a silent onset,
reachable from an ordinary workflow.**

### 4.4 Import is not idempotent when duplicate hashes exist

Once clone C has two rows with hash H (4.2), a second import resolves each line's
`find_by_content_hash(H)` to an unordered row. If the two lines' `updated_at` differ, both can resolve
`Update` into each other's ids and **swap their non-hashed fields** (labels, dependencies, comments,
`estimated_minutes`, `due_at`, `close_reason`, timestamps), because `sync_issue_relations`
(`src/sync/mod.rs:2533`) destructively replaces relations per target id (`sync_labels_for_import` /
`sync_dependencies_for_import` / `sync_comments_for_import`, `src/storage/sqlite.rs:3747`, `:3768`,
`:3802`, all `DELETE WHERE issue_id = ?` then re-insert).
(CONFIRMED-CODE for the mechanism; SUSPECTED for the exact swap, which depends on SQLite row order.)

### 4.5 The remapping can create self-dependencies

If two file issues X and Y both dedup to the same existing Z, Phase 2 rewrites both ids to Z, so a
dependency X -> Y becomes Z -> Z. `sync_dependencies_for_import`
(`src/storage/sqlite.rs:3768-3794`) uses bare `INSERT OR IGNORE` with **no cycle check and no self-edge
check** - unlike `add_dependency`, which calls `would_create_cycle`
(`src/storage/sqlite.rs:1790-1798`). `IssueValidator::validate` does not inspect dependencies at all.
A self-blocking edge makes Z permanently non-ready via `rebuild_blocked_cache_impl`. **CONFIRMED-CODE.**

---

## 5. Scenario (c): delete on one side, edit on the other

Deletion is a tombstone, not a row removal: `delete_issue` sets `status='tombstone'` plus
`deleted_at/deleted_by/delete_reason/original_type` and bumps `updated_at`
(`src/storage/sqlite.rs:643-689`). Tombstones **are** exported (`get_all_issues_for_export` filters only
`ephemeral=1` and `id LIKE '%-wisp-%'`, `src/storage/sqlite.rs:2699-2700`).

### 5.1 Delete + edit of the same issue -> git conflict

Both sides rewrite the same line/block. Markers. Hard import error. Safe.

### 5.2 The tombstone veto outranks everything, including a human's resolution

`determine_action` checks `storage.is_tombstone(existing_id)` **before** `force_upsert` and before any
timestamp comparison (`src/sync/mod.rs:2097-2103`):

```rust
if storage.is_tombstone(existing_id)? {
    return Ok(CollisionAction::Skip { reason: format!("Tombstone protection: {existing_id}") });
}
```

Once clone A has tombstoned `bd-1`, **no incoming version of `bd-1` can ever be applied to A** - not a
newer edit, not `--force`, not a deliberate human resolution that chose to keep the issue alive. A's
next flush re-exports the tombstone over the resolved file.

On clone B (which edited, did not delete), importing A's tombstone goes through the normal LWW path, so
the outcome depends entirely on timestamp ordering:

* `T_delete > T_edit` -> B applies the tombstone. Both converge. Correct.
* `T_delete < T_edit` -> B **Skips("Existing is newer")** and keeps the issue open. B re-exports it. A
  imports -> tombstone veto -> Skip. A re-exports the tombstone. **Permanent oscillation**: the working
  file flips between the tombstone line and the open line on every sync round trip, each side's
  auto-flush reverting the other's. **CONFIRMED-CODE.**

There is no `--force` escape on either side: `force_upsert` is checked *after* the tombstone veto.

### 5.3 There is no un-delete and no resurrection path

`import_from_jsonl` never deletes: nothing in `src/sync/mod.rs:2201-2492` iterates ids present in the DB
but absent from the file. Deletion propagates **only** through the tombstone row.

`ImportConfig.orphan_mode` / `OrphanMode::Resurrect` (`src/sync/mod.rs:255`, `:285-296`) is parsed from
`--orphans` (`src/cli/commands/sync.rs:914-928`), stored, logged - and **never read** by
`import_from_jsonl`. Step 7 "Orphan handling" of its own docstring (`src/sync/mod.rs:2186`) is
unimplemented. So there is no supported way to undo a tombstone across clones. (CONFIRMED-CODE.)

### 5.4 Tombstone TTL is a Cassandra gc_grace_seconds footgun (latent - TTL off by default)

`Issue::is_expired_tombstone` (`src/model/mod.rs:565-586`) drops a tombstone from the *export* once
`now > deleted_at + retention_days`; the row stays in SQLite forever. Once the line is gone from the
file, deletion knowledge no longer propagates (5.3).

A clone offline (or on a long-lived branch) longer than the retention window still has the issue
**open**. It imports the reaped file: no `bd-1` line -> nothing happens. It exports: DL-2's
`missing = jsonl_ids - db_ids` is empty (the guard is one-directional), so the export succeeds and
**writes `bd-1` back into the file**. The deleting clone then imports it -> tombstone veto -> Skip ->
re-exports the tombstone. Oscillation again. **CONFIRMED-CODE.**

Mitigating: `Metadata::deletions_retention_days` defaults to `None` (`src/config/mod.rs:58`, `:67`), and
`auto_flush` builds `ExportConfig { force: false, beads_dir, ..Default::default() }`
(`src/sync/mod.rs:1941-1945`) whose `retention_days` is `None` - so **auto-flush never reaps**, while
`obr sync --flush-only` does pass the metadata value (`src/cli/commands/sync.rs:92`, `:577`). Whether a
tombstone line survives therefore depends on *which code path flushed*, itself a determinism bug.

---

## 6. Scenario (d): clock skew

All CONFIRMED-CODE.

### 6.1 The cleanest loss: no git merge required, just fast-forwards

Clone A's clock runs 10 minutes fast.

1. A edits `bd-1`. `updated_at = T_A` (skewed forward). Push.
2. B pulls (fast-forward, no merge, no conflict). B imports: `T_A > T_B0` -> Update. Fine.
3. B edits `bd-1` at real time *later than A's edit*, but its honest clock gives `T_B < T_A`.
   `update_issue` writes `Utc::now()` with no monotonicity check (`src/storage/sqlite.rs:613-614`), so
   **B's row's `updated_at` goes backwards**.
4. B flushes, pushes.
5. A pulls (fast-forward). A imports: `T_B < T_A` -> **Skip("Existing is newer")**. A silently ignores
   B's edit.
6. A's next mutating command auto-flushes the whole DB -> the file reverts to A's version.
7. B pulls, imports: `T_A > T_B` -> Update -> **B's own edit is overwritten on B's machine too.**

B's work is destroyed with no conflict, no marker, no warning, and no git-level evidence. The only
signal is a `debug!` line nobody sees.

### 6.2 A far-future timestamp permanently pins an issue

There is no upper bound on `updated_at` - `IssueValidator::validate` checks only
`updated_at >= created_at` (`src/validation/mod.rs:73-78`). One issue that acquires a year-2030
timestamp (badly skewed VM, hand-edited Org file, bad import) can never again be updated *across
clones*: every honest `Utc::now()` compares `Less` and Skips. Local edits appear to work, then get
reverted on the next sync round trip by exactly the 6.1 loop. There is no way to signal or repair this.

### 6.3 Ties go to local, and that is the hand-edit hazard

`Ordering::Equal -> Skip("Equal timestamps")` (`src/sync/mod.rs:2125-2127`).

This is a normal no-op path for unchanged issues, but it is also the trap for the workflow this fork
exists to enable. **Hand-editing `.beads/issues.org` in Emacs - the entire point of the Org migration -
silently loses the edit** unless the human also hand-bumps `:UPDATED_AT:`:

* Human changes a title in the heading/drawer, does not touch `:UPDATED_AT:`.
* Import: content hash differs -> phase 2 no match; phase 3 id match -> `Match{id}`;
  `incoming.updated_at == existing.updated_at` -> **Skip**.
* `jsonl_content_hash` is stamped with the edited file's hash (`src/sync/mod.rs:2488-2489`), so it will
  never be re-examined.
* Next mutating command -> auto-flush -> the hand edit is overwritten in the working tree.

**CONFIRMED-CODE. Zero documents mention Org-mode at all** (per the docs map), so nobody is warned.

### 6.4 Precision is not the problem

`Utc::now()` gives nanosecond precision; `to_rfc3339()` and chrono's serde impl preserve it;
`parse_datetime` (`src/storage/sqlite.rs:3352-3366`) round-trips RFC3339 exactly. Genuine ties between
distinct writes are essentially impossible. The `Equal` branch fires almost exclusively on unchanged
rows and on hand edits (6.3).

---

## 7. `obr sync --merge` is non-functional and unsafe

All CONFIRMED-CODE.

### 7.1 It always keeps local for anything in the base

`merge_issue` case 6 (`src/sync/mod.rs:2764-2772`):

```rust
let left_changed  = l.content_hash != b.content_hash;   // :2766
let right_changed = r.content_hash != b.content_hash;   // :2767
match (left_changed, right_changed) {
    (false | true, false) => MergeResult::Keep(l.clone()),   // :2771
    ...
}
```

But `Issue.content_hash` is `#[serde(skip)]` (`src/model/mod.rs:374-375`), so:

* `base` from `load_base_snapshot` (`src/sync/mod.rs:2951-2984`, plain `serde_json::from_str`) -> always `None`.
* `right` from `read_issues_from_jsonl` (`src/sync/mod.rs:1972-2000`) or `org_text_to_issues`
  (`src/sync/org_bridge.rs:335`, starts from `Issue::default()`) -> always `None`.
* `left` from `get_all_issues_for_export` (`src/cli/commands/sync.rs:1120` ->
  `src/storage/sqlite.rs:2691`, which SELECTs `content_hash`) -> always `Some`.
* `execute_merge` never calls `normalize_issue`, so nothing recomputes them.

Therefore `left_changed = Some(h) != None = true` and `right_changed = None != None = false` for
**every** issue present in the base, landing on `Keep(l)` unconditionally. **`obr sync --merge`
silently discards all external edits** to any pre-existing issue, regardless of `updated_at` or
strategy - and then performs a `force: true` export (`src/cli/commands/sync.rs:1212`) that bypasses both
data-loss guards and writes that result over the working tree.

Case 7 (convergent creation, `src/sync/mod.rs:2817-2820`) is skewed the same way:
`l.content_hash == r.content_hash` is `Some == None` -> false, so identical-content pairs never take the
fast path.

The base snapshot is self-reinforcing: `save_base_snapshot` (`src/sync/mod.rs:2931-2948`) writes JSONL,
`content_hash` is `#[serde(skip)]`, so the base is guaranteed to come back `None` forever.

The 32 unit tests around `merge_issue` all use `make_issue_with_hash` (`src/sync/mod.rs:4608-4612`),
which sets `content_hash` by hand on all three sides - i.e. they exercise a state that cannot occur in
production.

### 7.2 Tombstone protection is disabled on the merge path

`three_way_merge` accepts a tombstone set (`src/sync/mod.rs:2870`), but the only production caller
hardcodes `let tombstones = None;` (`src/cli/commands/sync.rs:1157`). Invariant DL-3 ("import never
resurrects tombstones") holds for `--import-only` and **not** for `--merge`.

Likewise `let strategy = ConflictResolution::PreferNewer;` (`src/cli/commands/sync.rs:1156`) - the other
three `ConflictResolution` arms and the entire `MergeConfig` struct (`src/sync/mod.rs:2917-2925`) are
unreachable, and since `PreferNewer` never returns `Conflict`, the conflict-abort branch at
`src/cli/commands/sync.rs:1170-1181` is dead.

### 7.3 It can abort mid-way, non-transactionally

`MergeResult::Delete` (cases 1 and 5) yields ids that may not exist locally; `execute_merge` calls
`storage.delete_issue(id, ...)` (`src/cli/commands/sync.rs:1185`), which returns `IssueNotFound`
(`src/storage/sqlite.rs:656-658`). Nothing wraps the merge in a transaction, so earlier upserts are
already committed.

### 7.4 It never scans for conflict markers

`ensure_no_conflict_markers` has exactly three call sites (verified by grep) and `execute_merge` is not
one of them. By format:

* **JSONL**: `serde_json::from_str("<<<<<<< HEAD")` fails -> `read_issues_from_jsonl` errors -> loud
  failure. Acceptable.
* **Org** (the default): `<<<<<<<` / `=======` / `>>>>>>>` are just text to the org parser. They are
  absorbed into whichever heading's drawer or body they fall in, `parse_properties` ignores unknown
  keys (`src/sync/org_bridge.rs:735-739`), the merge then keeps local for everything (7.1), and the
  `force: true` export **silently overwrites the conflicted file with the local DB state**, erasing the
  evidence. (CONFIRMED-CODE for the code path; SUSPECTED for exactly where org2jsonl places the marker
  lines.)

---

## 8. Additional confirmed defects on the sync path

### 8.1 Duplicate ids in the file: last-line-wins, not last-write-wins

There is no duplicate-id detection anywhere in import (`rg -n duplicate src/sync/mod.rs` finds only
`external_ref` handling at `:2401-2415`). `analyze_jsonl` collapses ids into a `HashSet`
(`src/sync/mod.rs:1148`, `:1168`) so it cannot report duplicates either.

Trace: file has two lines both `id = bd-1`, at T1 and T2 (T2 > T1); DB has `bd-1` at T0. Phase 1
evaluates **both** against the pre-import state T0 -> both get `Update`. Phase 3 applies them in file
order. **The last line in the file wins, regardless of timestamp.** If the resolver put the older
version last, the older version wins. LWW is not applied.

This is exactly the shape a `merge=union` driver, a careless conflict resolution, or a hand-edited Org
file produces. **CONFIRMED-CODE.** It is also the reason obr must never adopt `merge=union`.

### 8.2 `external_ref` precedence can write an incoming issue onto the wrong row

`detect_collision` phase 1 (`src/sync/mod.rs:2056-2065`) matches on `external_ref` **before** id.
`external_ref` carries a UNIQUE partial index (`src/storage/schema.rs:69`).

If the file's `bd-1` line has acquired `external_ref = "JIRA-9"` (set on another clone) and the local DB
already has a *different* issue `bd-7` holding `"JIRA-9"`, then phase 1 returns `Match{bd-7}`, Phase 2
rewrites the incoming id `bd-1 -> bd-7`, and Phase 3 writes **`bd-1`'s content onto `bd-7`**. The local
`bd-1` survives untouched with its old content. The next export emits both. The other clone, whose
`external_ref` index differs, produces a different result from the same file. **CONFIRMED-CODE.**

### 8.3 Comment ids are global rowids but are imported verbatim

`comments.id` is `INTEGER PRIMARY KEY AUTOINCREMENT` (`src/storage/schema.rs:122-129`) and is serialized
to the file. `sync_comments_for_import` (`src/storage/sqlite.rs:3796-3823`) deletes only the *target
issue's* comments then does `INSERT OR REPLACE INTO comments (id, issue_id, ...)` with the file-supplied
id. A comment id colliding with a comment on a **different** issue silently overwrites it. Cross-repo
comment merging is not id-safe. **CONFIRMED-CODE.**

### 8.4 Import is non-transactional

Phase 3 is a bare loop of autocommit `conn.execute` calls (`src/sync/mod.rs:2466-2477` ->
`src/storage/sqlite.rs:3676`, `:3747`, `:3768`, `:3802`), and `clear_all_export_hashes()` already ran at
`:2380`. Input-side failures are safe because the whole file is parsed and validated before any write
(`:2224-2275`), but a mid-loop DB failure (disk full, lock timeout, constraint violation) leaves a
partially-imported DB with `last_import_time`/`jsonl_content_hash` **not yet written** - so the next run
re-imports, which is at least recoverable. **CONFIRMED-CODE.**

### 8.5 Org round-trip is not a fixpoint

`extract_body_text` (`src/sync/org_bridge.rs:540-554`) handles **only** `Element::Paragraph`. Lists,
tables, src blocks, quote blocks and sub-structure inside a description/design/AC/notes body are
**dropped**. It also indexes the separator on the *element* index rather than the emitted-paragraph
index:

```rust
for (i, element) in body.iter().enumerate() {
    if let Element::Paragraph { contents } = element {
        if i > 0 { text.push_str("\n\n"); }      // src/sync/org_bridge.rs:545-547
```

so a body of `[List, Paragraph]` yields a description with a spurious leading `"\n\n"`.

Why this matters for convergence: in the originating clone the loss is invisible (import Skips on equal
timestamps). But a **different** clone receiving the file for the first time `Insert`s the *lossy*
version, computes a different `content_hash` from it, and its next export writes the truncated
description back. Content silently degrades exactly at clone boundaries, and the degradation manifests
as a phantom "change" that then propagates. **CONFIRMED-CODE.**

Related lossiness re-verified: `Status::Custom(s)` round-trips uppercased (`status_to_keyword`,
`src/sync/org_bridge.rs:262-273`; `keyword_to_status`, `:276-287`), and `#[serde(untagged)]
Custom(String)` on `Status`/`IssueType` (`src/model/mod.rs:45`, `:148`) means a typo in a hand-edited
file becomes a new custom status rather than an error.

### 8.6 The mtime gate can suppress the hash check

`compute_staleness` (`src/sync/mod.rs:1750-1766`) evaluates `mtime_newer` **first** and only then
compares content hashes. On a filesystem with coarse mtime granularity, or if a tool rewrites the file
preserving mtime (`rsync -t`, `tar -x`, `cp -p`), `jsonl_newer` is false and the hash is never
consulted - auto-import silently skips a genuinely changed file, and the next flush overwrites it.
obr's own restore uses `std::fs::copy` (`src/cli/commands/history.rs:257`, `src/sync/history.rs:93`),
which sets mtime to now, so obr does not trigger this itself.
**CONFIRMED-CODE (mechanism); SUSPECTED (practical reachability).**

### 8.7 Two divergent staleness implementations

`compute_staleness` treats an unparseable `last_import_time` as "file is newer" (`.map_or(true, ...)`,
`src/sync/mod.rs:1754`) while `execute_status` treats it as "not newer" (`.is_ok_and(...)`,
`src/cli/commands/sync.rs:299-304`). `obr sync --status` can report "in sync" while the very next
command's auto-import decides otherwise. **CONFIRMED-CODE.**

---

## 9. Inventory: where correctness depends on what

| Dependency | Where | Verdict |
|---|---|---|
| **Wall clock** as the only ordering signal | `determine_action`, `src/sync/mod.rs:2120-2131`; `merge_issue` PreferNewer, `:2785-2795`, `:2831-2843` | Unsound across machines. Section 6. |
| **Wall clock** written with no monotonicity guard | `src/storage/sqlite.rs:613-614` and every mutator | `updated_at` can go backwards. 6.1. |
| **No upper bound** on `updated_at` | `src/validation/mod.rs:32-101` | One future timestamp pins an issue forever. 6.2. |
| **Ties go to local** | `src/sync/mod.rs:2125-2127` | Silently drops hand edits. 6.3. |
| **Import order** (file line order) | Phase 3 loop, `src/sync/mod.rs:2470-2474` | Decides the winner for duplicate ids. 8.1. |
| **Pre-import DB snapshot** for all collision decisions | Phase 1, `src/sync/mod.rs:2390-2442` | Two content-identical *new* issues both Insert. 4.2. |
| **Unordered SQLite row return** | `find_by_content_hash`, `src/storage/sqlite.rs:3629-3648`; non-unique index `src/storage/schema.rs:67` | Non-deterministic dedup target. 4.4. |
| **Union merge** | *not used* - no `.gitattributes`, no driver anywhere | Not a live risk, and must stay that way (8.1). 0.2. |
| **Conflict markers as the safety net** | `ensure_no_conflict_markers`, `src/sync/mod.rs:1091` @ `:2221` | Works for import; **absent from `--merge`**. 7.4. |
| **Whole-file export vs selective import** | `src/sync/mod.rs:1289` vs `:2089` | Engine of silent revert. 1.1. |
| **`content_hash` never on disk** | `#[serde(skip)]`, `src/model/mod.rs:374-375` | Silently breaks `merge_issue`. 7.1. |
| **Tombstone as the only deletion channel** | `src/storage/sqlite.rs:643-689`; import never deletes | TTL reaping => resurrection. 5.4. |

### 9.1 Convergence guarantees: what holds and what does not

**Holds:**

* **G1** - Disjoint per-issue edits that git merges cleanly converge to identical DB state and identical
  byte output on both clones, provided every incoming `updated_at` strictly exceeds the receiver's
  stored value for that id. (3.3)
* **G2** - Import is idempotent *when* no two rows share a `content_hash`: re-importing the same file
  yields all-Skip. (4.4 is the exception.)
* **G3** - Export is byte-deterministic for a fixed DB: `ORDER BY id ASC`
  (`src/storage/sqlite.rs:2701`), labels sorted+deduped (`src/sync/mod.rs:1393-1396`), dependencies
  `ORDER BY issue_id, depends_on_id` (`src/storage/sqlite.rs:2726`), comments
  `ORDER BY issue_id, created_at ASC` (`:2765`).
* **G4** - Malformed *input* cannot partially mutate the DB: full parse + full validate precede all
  writes (`src/sync/mod.rs:2224-2275`).
* **G5** - Conflict markers can never be imported through `import_from_jsonl`, `--force` included.

**Does not hold:**

* **not-G6 (confluence)** - the same merged file applied to two different DB states yields semantically
  different results, with no subsequent exchange that reconciles them. 4.2 is the proof: clone A
  converges on id `bd-aaa`, clone B on `bd-bbb`, clone C on both, permanently.
* **not-G7 (no silent loss)** - an incoming change with `updated_at <= local` is discarded and then
  overwritten in the working tree, at `debug!` verbosity. 3.2, 6.1, 6.3.
* **not-G8 (commutativity)** - the outcome depends on file line order (8.1), on which rows SQLite
  returns first (4.4), and on which clone imports first (4.2).
* **not-G9 (deletion convergence)** - delete-vs-edit with `T_delete < T_edit` oscillates forever with no
  escape, because the tombstone veto outranks `--force`. 5.2.
* **not-G10 (liveness)** - content-hash dedup can wedge *both* clones' exports permanently behind the
  DL-2 guard, invisibly. 4.3.
* **not-G11 (round-trip fidelity)** - Org descriptions containing non-paragraph elements degrade at
  every clone boundary. 8.5.
* **not-G12** - `obr sync --merge` provides no merge semantics at all; it is `Keep(local)` plus a forced
  overwrite. Section 7.

---

## 10. Comparison against the CRDT / operation-log prior art

The sync-merge and related-trackers research converge on properties every surviving git-native tracker
has. obr has none of them, and each absence maps onto a specific defect traced above.

| Prior-art property | Who does it | obr | Which defect it would have prevented |
|---|---|---|---|
| **Logical time** (Lamport/HLC + writer id, wall clock display-only) | git-bug (Lamport clocks in tree entry names; "you can't rely on the time provided by other people") | Absent; `Utc::now()` only | 6.1 skew loss, 6.2 future-pin, 6.3 tie-loss, 5.2 oscillation |
| **Operation log / field-level deltas** rather than whole-record snapshots | git-bug `OperationPack`; Fossil amending artifacts omit unchanged fields; ticgit-2026 typed key paths; Kubernetes `patchMergeKey`; Dolt cell-level merge; SQLite session rebaser ("per-field basis rather than per-row") | Whole-`Issue` snapshots; whole-record LWW; relations destructive-replace (`src/storage/sqlite.rs:3747`, `:3768`, `:3802`) | Any concurrent same-issue edit loses one side entirely |
| **Set-valued fields as OR-sets** (explicit add/remove) | git-bug ops; git-appraise union log | `labels`/`dependencies`/`comments` are replace-the-whole-list | Concurrent `label add` on two clones: one is destroyed |
| **Conflicts representable, not fatal** | jujutsu conflict algebra (with the simplification property so repeated syncs do not nest); Dolt conflicts-as-rows | Conflicts are either a hard error (markers) or silently resolved by discarding a side | No `obr list --conflicted`; loss is invisible |
| **Entity id = hash of the creating operation, with an entropy nonce** | git-bug (mandatory random nonce, else identical ops collide) | Ids include `created_at` nanos (`src/util/id.rs:166-184`) - good; but the *dedup* key deliberately excludes the nonce | Section 4 - the id-space is safe, the dedup space is not |
| **Derived index provably rebuildable from the log** | git-bug `formatVersion` forced rebuild + doc-count check; Fossil replays on schema change; Radicle `from_history` | File is lossy for Org (8.5) and `content_hash` is never on disk (7.1) | 7.1, 8.5 |

Two sharper observations:

1. **obr's content-hash dedup is a CRDT-shaped mechanism used without CRDT discipline.** Content
   addressing converges only if the address is stable and the merge of two addresses is well-defined.
   Here the address (`content_hash`) is *mutable* (any edit changes it), *non-unique* in storage
   (`src/storage/schema.rs:67`), *not persisted* to the transfer format (`#[serde(skip)]`), and
   *asymmetrically resolved* (`find_by_content_hash` returns an arbitrary row). It is the single
   mechanism responsible for the worst outcome in this analysis (section 4).

2. **SIT's objection is only half-answered.** SIT (Rust, dead) rejected "plain text append-only files"
   for directory-per-record because directories are "far more merge-friendly". obr's file is canonically
   serialized and sorted - which answers the interoperability half - but it is **not append-only**:
   every flush rewrites every line, so the file is a materialized view, not a log. The git-appraise
   contract (one self-contained order-independent record per line, never rewritten, canonically
   serialized) would make union merge a genuine join-semilattice; obr satisfies canonicalization and
   ordering but violates append-only, so union merge would be *unsafe* here even if a driver were
   installed. Worth writing down, because a contributor reading the sync-merge research might reasonably
   conclude "just add merge=union" - and that would be catastrophic given 8.1.

---

## 11. Ranked remediation (each tied to a traced defect)

Ordered by (damage x reachability) / cost.

1. **Make the silent revert loud.** Change `process_import_action`'s Skip from `debug!` to `warn!` with
   the id and reason (`src/sync/mod.rs:2520-2528`), add `skipped_ids` to `ImportResult` and
   `AutoImportResult`, surface a count from `run_auto_import` (`src/main.rs:245-250`), and change
   `run_auto_flush`'s error arm from `debug!` to `warn!` (`src/main.rs:294-297`). Relabel
   "Skipped: n issues (up-to-date)" (`src/cli/commands/sync.rs:990`) to distinguish
   up-to-date / older-than-local / tombstone-protected. *Cheap; makes 3.2, 4.3, 6.1 visible.*
2. **Refuse to overwrite a file whose content differs from what the DB believes.** Add a DL-2b guard
   next to `src/sync/mod.rs:1306`: if any id in the file resolves to a DB row whose `updated_at` is
   older than the file's, refuse the export. Converts 3.2, 6.1 and 6.3 from silent loss into an
   actionable error. *Moderate; `analyze_jsonl` would need to return `(id, updated_at)`.*
3. **Fix or disable `obr sync --merge`.** Minimum: recompute `content_hash` on `base` and `right` in
   `execute_merge` (call `normalize_issue`, `src/sync/mod.rs:2141`, on both maps before
   `MergeContext::new`), call `ensure_no_conflict_markers` first, pass the real tombstone set instead of
   `None` (`src/cli/commands/sync.rs:1157`), and wrap the apply loop in a transaction. If that is more
   than is wanted, hide it behind `--experimental` or remove it. *Currently a loaded gun: 7.1 plus the
   `force: true` export at `src/cli/commands/sync.rs:1212`.*
4. **Bound and monotonize `updated_at`.** In every mutator write `max(now, existing.updated_at + 1ns)`;
   in `IssueValidator::validate` reject `updated_at > now + skew_allowance` on import. *Kills 6.1
   backwards-time and 6.2 future-pin. Cheap and purely local.*
5. **Add a per-record logical clock.** Two additive fields on `Issue` - `lamport: u64` (default 0) and
   `writer_id: String` (the `.beads` install's stable id, not the actor) - with ordering
   `(lamport, writer_id, updated_at)` in `determine_action`. Old records default to 0 and fall back to
   today's behaviour, so it is backward compatible with existing `.jsonl`/`.org` files. In Org, emit it
   *after* `:UPDATED_AT:` so it does not worsen 2.2. *The structural fix for section 6 as a class, and
   the one recommendation every piece of the prior-art research agrees on.*
6. **Make content-hash dedup opt-in and deterministic.** Either (a) restrict phase 2 of
   `detect_collision` to the explicit import-a-foreign-repo flow (it exists precisely to make
   `--rename-prefix` re-imports converge, `src/sync/mod.rs:2352-2357`), or (b) make it deterministic and
   safe: add `ORDER BY id ASC LIMIT 1` to `find_by_content_hash` (`src/storage/sqlite.rs:3629-3648`) and
   never dedup when the incoming id already exists locally (short-circuit on `id_exists(incoming.id)`).
   Option (a) is strongly preferable: it removes section 4 entirely. *Highest-damage defect in the report.*
7. **Detect duplicate ids in the import file and refuse.** A single `HashSet` pass over `issues` after
   parsing (`src/sync/mod.rs:2253`) that errors on a repeat, plus a duplicate-`content_hash` warning and
   a guard for a `renames` map with two sources mapping to one target (4.5). *Cheap; closes 8.1.*
8. **Add self-edge and cycle checks to `sync_dependencies_for_import`**
   (`src/storage/sqlite.rs:3768-3794`), matching `add_dependency`'s `would_create_cycle`. *Closes 4.5.*
9. **Make Org round-trip lossless or refuse the export.** Either extend `extract_body_text`
   (`src/sync/org_bridge.rs:540`) to render all `Element` variants back to Org source, or add an
   export-time round-trip assertion (emit -> parse -> compare `content_hash`) that fails the flush. Fix
   the `i > 0` separator bug at `:545` regardless. Add a property test
   `issues_to_org_text . org_text_to_issues == id`. *Closes 8.5.*
10. **Write the format specification.** The recurring post-mortem across two decades of git-native
    trackers is that nobody wrote one. It should state: canonical serialization, sort order, the exact
    ordering rule per field, tombstone semantics and TTL, id derivation, what `content_hash` covers and
    why, and - critically - **that `merge=union` must never be configured for this file**, with 8.1 as
    the reason. *Cheap, and it forces every invariant above to be real rather than aspirational.*
11. **Add a two-clone convergence test.** There is currently none: `tests/` has `e2e_concurrency.rs`,
    `e2e_sync_*.rs`, `jsonl_import_export.rs`, `repro_import_collision_remap.rs`,
    `repro_sync_relations.rs` - all single-workspace. The 32 `merge_issue` unit tests hand-set
    `content_hash` (`src/sync/mod.rs:4608-4612`) and therefore validate a state that never occurs. A
    harness that inits two workspaces, exports from both, runs a real `git merge`, imports on both sides
    and asserts identical `obr list --json` output would have caught 3.2, 4.2, 5.2 and 7.1 on the first
    run.

---

## 12. Confirmed / suspected index

**CONFIRMED-CODE**
1.1 selective-import vs total-export; 1.2 skip invisibility; 3.2 silent revert of human conflict
resolution; 4.2 three-way divergence from content-hash dedup; 4.3 permanent double-clone export deadlock
behind DL-2; 4.5 remap-induced self-dependency plus no cycle check on import; 5.2 tombstone veto
outranks `--force` => oscillation; 5.3 import never deletes, `orphan_mode` is dead; 5.4 TTL reaping =>
resurrection, and `auto_flush` never reaps while `--flush-only` does; 6.1 skew loss over fast-forwards
alone; 6.2 future timestamp pins an issue permanently; 6.3 hand-edit without bumping `:UPDATED_AT:` is
silently discarded; 7.1 `--merge` always keeps local (`#[serde(skip)]` on `content_hash`); 7.2 tombstones
disabled on `--merge`; 7.3 `--merge` can abort mid-way non-transactionally; 7.4 `--merge` never scans for
markers; 8.1 duplicate ids => last-line-wins; 8.2 `external_ref` precedence writes onto the wrong row;
8.3 comment-id collision across issues; 8.4 import non-transactional; 8.5 Org body loss plus the `i > 0`
separator bug; 8.7 divergent staleness; 0.2 no merge driver exists anywhere; section 9 dependency
inventory.

**CONFIRMED-EXPERIMENT** (git 2.55.0, scratchpad repo)
2.1 JSONL adjacent-line edits conflict at distance 1, clean at >=2; same-gap insertions conflict,
adjacent gaps clean. 2.2 Org disjoint field edits on one issue conflict on `:UPDATED_AT:`.

**SUSPECTED**
4.4 the exact field-swap outcome under duplicate hashes (mechanism confirmed; depends on SQLite row
order). 7.4 exactly where org2jsonl places absorbed conflict-marker lines (that they are not rejected is
confirmed). 8.6 practical reachability of the mtime gate suppressing the hash check (mechanism
confirmed).

**Not investigated**
`--no-db` mode's in-memory import/flush-back round trip (`src/config/mod.rs:368-419`, `:330-359`) as an
additional consistency surface; the `preflight_export`/`preflight_import` dead code paths
(`src/sync/mod.rs:489`, `:719`); interaction between the skipped-PRAGMA bug
(`src/storage/sqlite.rs:105-109` vs `src/storage/schema.rs:216-227`, `foreign_keys` OFF on the common
path) and `INSERT OR REPLACE` in `upsert_issue_for_import` - note that **fixing the PRAGMA bug would
newly enable `ON DELETE CASCADE` during import REPLACE**, silently dropping that issue's `events`,
`dirty_issues` and `export_hashes` rows. That coupling should be resolved before either is touched.
