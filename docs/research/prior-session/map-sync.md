# Subsystem map: `sync` (obr / beads_rust)

Scope: `src/sync/mod.rs` (5139 lines), `src/sync/path.rs` (1070), `src/sync/history.rs` (441),
`src/sync/org_bridge.rs` (900), `src/cli/commands/sync.rs` (1425), plus the git-safety contract in
`tests/e2e_sync_git_safety.rs` (1463) and the invariants doc `.beads/SYNC_SAFETY_INVARIANTS.md`.

All paths below are relative to `/Users/johnw/src/obr`. Line citations are `path:line`.

---

## 0. Ten-second orientation

The sync engine is the bridge between the SQLite database (fast, local, authoritative for queries)
and a **flat text file in the working tree** (git-friendly, authoritative for cross-machine transfer).
There are exactly two directions and one experimental reconciler:

| Direction | Entry point | Notes |
|---|---|---|
| DB → file ("flush"/export) | `export_to_jsonl_with_policy` `src/sync/mod.rs:1259` | atomic temp+rename, safety guards, history backup |
| file → DB ("import") | `import_from_jsonl` `src/sync/mod.rs:2201` | conflict-marker scan, normalize, 4-phase collision detect, upsert |
| 3-way merge | `three_way_merge` `src/sync/mod.rs:2868` driven by `execute_merge` `src/cli/commands/sync.rs:1101` | base snapshot + DB + file |

**Surprise #1 (the big one for this fork):** the *default* on-disk format is **Org-mode**, not JSONL.
`DEFAULT_JSONL_FILENAME = "issues.org"` (`src/config/mod.rs:33`), and `obr init` writes
`"jsonl_export": "issues.org"` into `metadata.json` (`src/cli/commands/init.rs:63`, and it creates an
empty `.beads/issues.org` at `init.rs:99-101`). Everything in the sync module still *calls* it JSONL;
format selection is a per-call `path.extension() == "org"` test scattered through the module
(`mod.rs:1139`, `1414-1424`, `1976`, `2214`). So "JSONL" in this codebase means "the flat export file",
which may be `.jsonl` or `.org`.

**Surprise #2:** the binary is now `obr` (`Cargo.toml:12-14`), not `br` as `CLAUDE.md` claims.

---

## 1. The flat-file format

### 1.1 JSONL: one `Issue` per line, serde struct order

There is no hand-written serializer. A line is `serde_json::to_string(&issue)`
(`src/sync/mod.rs:1479`) followed by `\n` (`mod.rs:1489`). Therefore **field ordering is exactly the
declaration order of `struct Issue`** at `src/model/mod.rs:369-503`:

```
id, title, description?, design?, acceptance_criteria?, notes?,
status, priority, issue_type, assignee?, owner?, estimated_minutes?,
created_at, created_by?, updated_at, closed_at?, close_reason?, closed_by_session?,
due_at?, defer_until?, external_ref?, source_system?, source_repo?,
deleted_at?, deleted_by?, delete_reason?, original_type?,
compaction_level, compacted_at?, compacted_at_commit?, original_size?,
sender?, ephemeral?, pinned?, is_template?,
labels[], dependencies[], comments[]
```

Canonicalization rules baked into the serde attributes:

- **`content_hash` is `#[serde(skip)]`** (`model/mod.rs:374-375`). It is *never* written to and never
  read from the file. This is load-bearing — see §6.3. Asserted by a unit test:
  `assert!(!json.contains("content_hash"))` at `model/mod.rs:743`.
- Every optional scalar uses `skip_serializing_if = "Option::is_none"`; `ephemeral`/`pinned`/
  `is_template` use `skip_serializing_if = "is_false"` (`model/mod.rs:487-494`); the three vectors use
  `skip_serializing_if = "Vec::is_empty"` (`model/mod.rs:497-502`). So the encoding is *sparse*: absent
  key == default.
- `compaction_level` is the one exception — a custom `serialize_with = "serialize_compaction_level"`
  (`model/mod.rs:475`) that always emits an `i32`, coercing `None` to `0` (`model/mod.rs:24-29`). So
  `"compaction_level":0` appears on every line.
- `Status`/`IssueType` are `rename_all = "snake_case"` with an `#[serde(untagged)] Custom(String)`
  escape hatch (`model/mod.rs:33-46`, `138-149`); tombstone status serializes as the literal
  `"tombstone"` (`model/mod.rs:41-42`).
- `Priority` is `#[serde(transparent)]` over an integer (`model/mod.rs:104`) — P0 critical … P4 backlog.
- `Dependency.dep_type` is renamed to `"type"` (`model/mod.rs:608`); `Comment.body` is renamed to
  `"text"` (`model/mod.rs:633-634`).
- Labels are **sorted and deduped** immediately before export (`mod.rs:1393-1396`) and again on import
  inside `normalize_issue` (`mod.rs:2145-2148`), so the round trip is stable.
- Row order is **sorted by `id ASC`** — enforced in SQL, not in Rust:
  `ORDER BY id ASC` in `get_all_issues_for_export` (`src/storage/sqlite.rs:2701`). Dependencies are
  ordered `ORDER BY issue_id, depends_on_id` (`sqlite.rs:2727`). This is what makes export
  byte-deterministic (test `test_export_deterministic_order` `mod.rs:3879`,
  `export_produces_deterministic_content_hash` `tests/jsonl_import_export.rs:381`).

Rows excluded from export at the SQL level: `ephemeral = 1` and any id matching `%-wisp-%`
(`sqlite.rs:2699-2700`). Tombstones **are** exported (they are the deletion-propagation mechanism).

### 1.2 Org-mode: the `org_bridge`

`issues_to_org_text` (`src/sync/org_bridge.rs:28`) emits, per issue, one level-1 heading:

```
#+TITLE: Beads Issues
#+SEQ_TODO: TODO DOING WAIT DEFER NOTE | DONE CANCELED     (org_bridge.rs:31-33)

* <KEYWORD> [#<P>] <title>    :label1:label2:
:PROPERTIES:
:BEADS_SCHEMA_VERSION: 1
:ID:       <id>
:ISSUE_TYPE: ... :CREATED_AT: ... :UPDATED_AT: ...  (+ ~25 optional props)
:END:
<description paragraphs>
** Design / ** Acceptance Criteria / ** Notes
** Dependencies    #+begin_src json ... #+end_src
** Comments        #+begin_src json ... #+end_src
```

Mappings (both directions, so they must stay in sync):

- status ↔ keyword: `status_to_keyword` `org_bridge.rs:262-273`, `keyword_to_status` `org_bridge.rs:276-287`.
  `Open→TODO, InProgress→DOING, Blocked→WAIT, Deferred→DEFER, Closed→DONE, Tombstone→CANCELED,
  Pinned→NOTE, Custom(s)→s.to_uppercase()`. The reverse accepts both the Org keyword and the raw
  internal name (`"OPEN"`, `"TOMBSTONE"`, …). Round-tripping a `Custom("foo")` status turns it into
  `Custom("FOO")` — case is not preserved.
- priority ↔ cookie: `priority_to_org` `org_bridge.rs:290-299` / `org_to_priority` `org_bridge.rs:301-310`.
  `0→[#A] 1→[#B] 2→[#C] 3→[#D] 4→[#E]`; anything unrecognised collapses to MEDIUM.
- labels ↔ Org tags, sorted on the way out (`org_bridge.rs:65-73`) and on the way in
  (`org_bridge.rs:376`).
- `Status::Pinned` also sets the `pinned` bool on parse (`org_bridge.rs:369-371`) — asymmetric with
  export, which emits `:PINNED: true` separately (`org_bridge.rs:158-160`).

Injection hardening: `sanitize_property_value` strips `\n`/`\r` and neuters `:END:` →`:END `
(`org_bridge.rs:241-246`); `sanitize_org_text` comma-escapes body lines starting with `*`
(`org_bridge.rs:248-259`), reversed by `unsanitize_org_text` (`org_bridge.rs:569-583`).

Parsing goes through the external `org2jsonl` crate with beads-specific keyword lists
`BEADS_TODO_KEYWORDS` / `BEADS_DONE_KEYWORDS` (`org_bridge.rs:312-323`), then
`parse_heading_to_issue` (`org_bridge.rs:358`) and `parse_properties` (`org_bridge.rs:585`).
`:ID:` is the only required property — missing it is a hard `Validation` error (`org_bridge.rs:741-746`).
Unknown properties are silently dropped (`org_bridge.rs:735-739`), as are unknown level-2 child
headings (`org_bridge.rs:412-415`).

Lossy fields on Org round trip: `content_hash` (as in JSONL), and any issue whose description
contains structure the org2jsonl `Element::Paragraph` extractor doesn't model — `extract_body_text`
only handles `Element::Paragraph` and joins with `\n\n` (`org_bridge.rs:540-554`), so lists, tables,
and nested blocks inside a description are dropped.

### 1.3 The file hash (`jsonl_content_hash`)

Two independent implementations that must agree:

- Export-side: `Sha256` fed `json.as_bytes()` then `b"\n"` per line (`mod.rs:1488-1489`); for Org it
  feeds the whole text at once (`mod.rs:1470`).
- File-side: `compute_jsonl_hash` (`mod.rs:2551`) reads lines, strips trailing `\n`/`\r`, and feeds
  `trimmed + b"\n"`.

They agree for JSONL by construction. They agree for Org only because org export is `\r`-free —
`sanitize_property_value` removes `\r` (`org_bridge.rs:243`) and `str::lines()` inside
`sanitize_org_text` (`org_bridge.rs:249`) also drops trailing `\r`. Any future path that lets a `\r`
into org output would silently desynchronize the stored hash from the recomputed hash and make the DB
look permanently stale.

---

## 2. Export / flush protocol

`export_to_jsonl_with_policy` `src/sync/mod.rs:1259-1560`, in order:

1. **Path validation** — `validate_sync_path_with_external(output, beads_dir, allow_external)`
   (`mod.rs:1266`). Skipped entirely if `config.beads_dir` is `None` (back-compat escape hatch,
   `mod.rs:1265`).
2. **History backup** — `history::backup_before_export` (`mod.rs:1285`), only when the resolved
   absolute output path is inside `.beads/` (`mod.rs:1284`). Note this happens **before** the safety
   guards, so a refused export still leaves a backup copy.
3. **Read** — `get_all_issues_for_export()` (`mod.rs:1289`).
4. **Safety guards** (skipped if `config.force`, `mod.rs:1292`):
   - *Empty-DB guard* (DL-1): 0 rows in DB but >0 in file → refuse (`mod.rs:1296-1304`).
   - *Stale-DB guard* (DL-2): any id present in the file but missing from the DB → refuse, listing up
     to 10 (`mod.rs:1306-1337`).
5. **Batch hydrate** deps/labels/comments via three whole-table queries to avoid N+1
   (`mod.rs:1348-1400`); labels sorted+deduped (`mod.rs:1393-1396`).
6. **Temp path** `<target>.jsonl.tmp` or `<target>.org.tmp` (`mod.rs:1414-1424`), validated by
   `validate_temp_file_path` (PC-4, same-directory + `.tmp` suffix, `mod.rs:1427`).
7. **Filter + hash + write**: expired tombstones are dropped here via
   `issue.is_expired_tombstone(retention_days)` (`mod.rs:1444`) and recorded in
   `skipped_tombstone_ids` so their dirty flags still get cleared.
8. **Durability**: `flush()` then `into_inner()?.sync_all()` (`mod.rs:1508-1512`) — AW-2.
9. **Re-validate both paths for destructive use** via `require_safe_sync_overwrite_path`
   (`mod.rs:1515-1527`), then **atomic `fs::rename`** (`mod.rs:1530`) — AW-1/AW-4.
10. **chmod 0600** on unix, best-effort (`mod.rs:1533-1539`).
11. **Verification**: re-count issues in the written file and fail if it differs from
    `exported_ids.len()` (`mod.rs:1545-1552`).

`finalize_export` (`mod.rs:1861`) is a *separate* call the caller must make: it clears dirty flags for
exported + skipped-tombstone ids (`mod.rs:1874-1880`), stores per-issue `export_hashes`
(`mod.rs:1883`), and writes `jsonl_content_hash` + `last_export_time` metadata (`mod.rs:1886-1887`).

### 2.1 Error policy

`ExportErrorPolicy` (`mod.rs:61-70`) = `Strict` (default, abort) | `BestEffort` | `Partial` (both
collect and continue) | `RequiredCore` (abort only on `Issue`-typed errors, tolerate
dependency/label/comment failures). Dispatched in `ExportContext::handle_error` `mod.rs:202-227`.
`ExportReport` carries counts + errors + `success_rate()` (`mod.rs:145-180`).

### 2.2 Auto-flush

`auto_flush` (`mod.rs:1918`) is a no-op when `dirty_issues` is empty (`mod.rs:1920-1924`); otherwise it
re-resolves the export path from `metadata.json` (`mod.rs:1928-1939`), exports with default config
(force = false), and finalizes. It is invoked from `main.rs:133-135` after any *mutating* command
unless `--no-auto-flush`/`--no-db`; failures are swallowed to `debug!` and never fail the command
(`main.rs:293-296`). `is_mutating_command` at `main.rs:139-158` is the gate list.

### 2.3 Stdout export is a different code path (and diverges)

`export_to_writer_with_policy` (`mod.rs:1598`) duplicates the hydrate+serialize loop but **does not**
sort/dedup labels and **does not** filter expired tombstones (`skipped_tombstone_ids` is a `let`
binding that is never pushed to, `mod.rs:1662`). It is only used by `benches/storage_perf.rs:534,557`.

---

## 3. Import / merge-into-DB protocol

`import_from_jsonl` `src/sync/mod.rs:2201-2492`. The docstring at `mod.rs:2178-2196` lists 11 steps;
the code actually does:

0. Path validation (`mod.rs:2211-2219`).
1. **`ensure_no_conflict_markers`** (`mod.rs:2221`) — this runs before *any* file parse or DB write,
   satisfying DL-4/IV-1.
2. Parse the whole file into memory (`mod.rs:2224-2253`) — Org via `org_text_to_issues`, JSONL via a
   2 MiB `BufReader`. Blank lines skipped; any bad line aborts with a line number
   (`mod.rs:2246-2249`). **Because parsing is complete before any write, malformed input can never
   partially mutate the DB** (this is what `tests/e2e_sync_failure_injection.rs:293` asserts).
3. `normalize_issue` on every issue (`mod.rs:2257-2259`, impl `mod.rs:2141-2175`):
   sort+dedup labels; **recompute `content_hash`**; force `ephemeral = true` if the id contains
   `-wisp-`; repair the `closed_at` invariant in both directions (set it for closed/tombstone, clear
   it otherwise).
4. `IssueValidator::validate` on every issue; first failure aborts the whole import
   (`mod.rs:2262-2275`).
5. **Prefix guard** (`mod.rs:2277-2374`) — see §5.
6. `clear_all_export_hashes()` (`mod.rs:2377`).
7. **Phase 1 — scan & resolve** (`mod.rs:2380-2444`): skip ephemerals; handle duplicate
   `external_ref` (error, or blank it out when `clear_duplicate_external_refs`); compute content hash;
   `detect_collision`; `determine_action`; record `incoming_id → target_id` in `renames`.
8. **Phase 2 — remap** (`mod.rs:2446-2464`): rewrite `issue.id` and both ends of every
   `Dependency` through the `renames` map.
9. **Phase 3 — execute** (`mod.rs:2466-2477`) via `process_import_action` (`mod.rs:2494`).
10. Restore `export_hashes` (`mod.rs:2479-2482`), `rebuild_blocked_cache(true)` (`mod.rs:2485`),
    write `last_import_time` and `jsonl_content_hash` metadata (`mod.rs:2487-2490`).

Relation sync is destructive-replace per issue (`sync_issue_relations` `mod.rs:2533`):
labels `DELETE WHERE issue_id` then re-insert (`sqlite.rs:3747-3761`); dependencies likewise
(`sqlite.rs:3768-3794`); comments likewise (`sqlite.rs:3802-3825`).

Import writes go through `upsert_issue_for_import` (`sqlite.rs:3676`), a bare
`INSERT OR REPLACE` that deliberately **bypasses the 4-step `mutate()` protocol**: no events, no dirty
marking, no cache invalidation (documented at `sqlite.rs:3669-3671`). Not marking imported issues
dirty is what prevents an import→auto-flush→import ping-pong.

---

## 4. Content-addressed dedup and the 4-phase collision algorithm

### 4.1 What the hash covers

`crate::util::content_hash` (`src/util/hash.rs:39-57`) → `content_hash_from_parts`
(`hash.rs:62-96`): SHA-256 over 15 fields, each NUL-terminated, in this fixed order:

`title, description, design, acceptance_criteria, notes, status, "P{priority}", issue_type,
assignee, owner, created_by, external_ref, source_system, pinned, is_template`

`None` is encoded as the empty string (`hash.rs:118-120`); bools as `"true"`/`"false"`
(`hash.rs:122-124`); an embedded NUL in a value is replaced with a space before hashing so the
separator can't be forged (`hash.rs:110-113`).

Deliberately **excluded** (documented `hash.rs:31-37`): `id`, `content_hash` itself, labels,
dependencies, comments, events, all timestamps, all tombstone fields, `estimated_minutes`, `due_at`,
`defer_until`, `close_reason`, `closed_by_session`. Consequence: two issues that differ only in labels
or dependencies are *content-identical* and will be deduped into one.

### 4.2 `detect_collision` — `mod.rs:2050-2087`

| Phase | Predicate | Storage call |
|---|---|---|
| 1 | `incoming.external_ref` matches an existing row | `find_by_external_ref` `sqlite.rs:3604` |
| 2 | computed content hash matches an existing row | `find_by_content_hash` `sqlite.rs:3629` |
| 3 | `incoming.id` already exists | `id_exists` `sqlite.rs:1689` |
| 4 | none of the above → `NewIssue` | — |

The ordering is the point: **`external_ref` beats `content_hash` beats `id`**. Unit tests pin all
three orderings: `test_detect_collision_external_ref_priority` `mod.rs:3650`,
`test_detect_collision_content_hash_before_id` `mod.rs:3686`, `test_detect_collision_id_match`
`mod.rs:3718`.

`content_hash` in SQLite is only *indexed*, not unique (`src/storage/schema.rs:67`), so
`find_by_content_hash` uses `query_row` with no `ORDER BY` (`sqlite.rs:3629-3645`) — with duplicate
hashes the dedup target is whichever row SQLite returns first.

### 4.3 `determine_action` — `mod.rs:2089-2139` (**this is merge decision point #1**)

```
NewIssue                                  -> Insert
Match && storage.is_tombstone(existing)   -> Skip "Tombstone protection"   (mod.rs:2098-2103)
Match && force_upsert                     -> Update                        (mod.rs:2106-2110)
Match && incoming.updated_at >  existing  -> Update                        (mod.rs:2124-2126)
Match && incoming.updated_at == existing  -> Skip "Equal timestamps"       (mod.rs:2127-2129)
Match && incoming.updated_at <  existing  -> Skip "Existing is newer"      (mod.rs:2130-2132)
```

So the import conflict rule is plain **last-write-wins on `updated_at`, with ties going to the local
DB**, and **tombstone protection outranks even `--force`** (the tombstone check is *before* the
`force_upsert` branch — DL-3). Tests: `test_determine_action_tombstone_skip` `mod.rs:3746`,
`test_determine_action_timestamp_comparison` `mod.rs:3769`, `test_import_tombstone_protection`
`mod.rs:3510`.

Also note `determine_action` is queried against the **pre-import DB state** for every issue in Phase 1,
before any Phase-3 write. Two content-identical *new* issues in the same file therefore both come back
`NewIssue` and both get inserted, producing duplicate `content_hash` rows.

---

## 5. ID collision and renumbering across repos

Three distinct mechanisms, easy to confuse:

**(a) Prefix guard.** `import_from_jsonl` `mod.rs:2277-2374`. If `skip_prefix_validation` is off and an
`expected_prefix` is supplied, ids not starting with the prefix are collected
(`mod.rs:2281-2291`); **tombstones with a foreign prefix are exempt** (`mod.rs:2285-2288`). Without
`rename_on_import` the import aborts with a `Prefix mismatch` error listing 5 offenders
(`mod.rs:2294-2302`); afterwards, surviving foreign-prefix tombstones are silently dropped by
`issues.retain(|i| i.id.starts_with(prefix))` (`mod.rs:2369-2371`). The same check exists read-only in
`preflight_import` `mod.rs:934-1004`.

**(b) Prefix renumbering (`--rename-prefix`).** `mod.rs:2305-2367`. For each mismatched issue it mints
a fresh id with `IdGenerator` (`src/util/id.rs:44`) seeded on `title|description|creator|created_at_ns|nonce`
(`id.rs:166-184`), base-36 encoded, with adaptive length from a birthday-bound
(`optimal_length` `id.rs:78-88`), and a uniqueness predicate that checks the existing DB ids, the ids
in the current file, and previously assigned renames (`mod.rs:2341-2346`). Crucially:

- the old id is **preserved into `external_ref`** if that field was empty (`mod.rs:2352-2354`), which
  makes the *next* import of the same upstream file dedup via collision phase 1;
- the content hash is recomputed because `external_ref` is part of the hash (`mod.rs:2357`);
- dependency edges on both ends are rewritten through the rename map (`mod.rs:2360-2366`).

**(c) Implicit renumbering by collision.** When phase 1 or 2 matches an existing issue whose id
differs from the incoming id, `renames.insert(incoming_id, existing_id)` (`mod.rs:2429-2431`), and
Phase 2 rewrites the incoming issue's id *and every dependency edge pointing at it*
(`mod.rs:2446-2464`). `process_import_action`'s `Update{existing_id}` branch handles the residual case
where they still differ (`mod.rs:2508-2521`). Regression test:
`tests/repro_import_collision_remap.rs:52` — DB has `bd-1` with `external_ref=EXT-1`; the file has
`bd-2` (same ext-ref, newer) and `bd-3` depending on `bd-2`; after import `bd-2` does not exist, `bd-1`
is updated, and `bd-3` depends on `bd-1`.

Prefix auto-detection for migration: if no `issue_prefix` config exists, `execute_import` sniffs the
first non-tombstone id's text before the first `-` and *persists* it
(`src/cli/commands/sync.rs:1055-1099`, called at `sync.rs:944-955`).

---

## 6. Conflict detection and resolution

### 6.1 Git merge-conflict markers (textual)

`scan_conflict_markers` `mod.rs:1053-1071` + `detect_conflict_marker` `mod.rs:1073-1089`. Prefixes are
`<<<<<<<`, `=======`, `>>>>>>>` (`mod.rs:1044-1046`); the branch name after `<<<<<<<`/`>>>>>>>` is
captured. `ensure_no_conflict_markers` `mod.rs:1091` turns any hit into a hard `Config` error listing
the first five, and is called as import step 1 (`mod.rs:2221`). **Not bypassable by `--force`** —
`--force` only feeds `skip_prefix_validation` and `force_upsert`, never the marker scan.
Also surfaced read-only by `obr doctor` (`src/cli/commands/doctor.rs:646`).

### 6.2 Import-time conflicts

Covered in §4.3: last-write-wins on `updated_at`, ties to local, tombstones win over everything.

### 6.3 3-way merge — `merge_issue` `mod.rs:2691-2853` (**merge decision point #2**)

Signature `(base, left=local/DB, right=external/file, strategy) -> MergeResult`. The eight cases:

| # | (base,left,right) | Result |
|---|---|---|
| 1 | `(S,N,N)` | `Delete` `mod.rs:2699` |
| 2 | `(N,S,N)` | `Keep(local)` `mod.rs:2702` |
| 3 | `(N,N,S)` | `Keep(external)` `mod.rs:2705` |
| 4 | `(S,S,N)` deleted externally | if `left.updated_at > base.updated_at` → strategy (PreferLocal/PreferNewer keep local, PreferExternal delete, Manual → `Conflict(DeleteVsModify)`); else `Delete`. `mod.rs:2707-2733` |
| 5 | `(S,N,S)` deleted locally | mirror image, `mod.rs:2735-2762` |
| 6 | `(S,S,S)` | `left_changed = l.content_hash != b.content_hash`, `right_changed = r.content_hash != b.content_hash`; `(_,false)→Keep(left)`, `(false,true)→Keep(right)`, `(true,true)→strategy` (PreferNewer/Manual compare `updated_at`, **ties to local**). `mod.rs:2764-2814` |
| 7 | `(N,S,S)` convergent creation | equal `content_hash` → `Keep(left)`; else strategy, PreferNewer/Manual tie-to-local. `mod.rs:2817-2848` |
| 8 | `(N,N,N)` | `NoAction` `mod.rs:2851` |

`three_way_merge` `mod.rs:2868-2915` iterates all ids in the union (`all_issue_ids` `mod.rs:2624`) and
applies an *optional* tombstone-protection pre-filter: if the id is in the `tombstones` set and it is
absent from `left` but present in `right`, it is refused as a resurrection attempt
(`mod.rs:2884-2892`). `MergeReport` (`mod.rs:2635-2646`) accumulates `kept`/`deleted`/`conflicts`/
`tombstone_protected`/`notes`.

Base state persistence: `save_base_snapshot`/`load_base_snapshot` write and read
`<beads_dir>/beads.base.jsonl` (`mod.rs:2937`, `mod.rs:2957`) — **always JSONL, even when the working
file is Org**. `.beads/.gitignore` already ignores `beads.base.jsonl` and friends
(`.beads/.gitignore`, "Merge artifacts" section), so the base is deliberately per-clone.

### 6.4 The CLI merge driver — `execute_merge` `src/cli/commands/sync.rs:1101-1246`

Loads base from the snapshot (`sync.rs:1116`), left from the DB with deps/labels/comments hydrated
(`sync.rs:1119-1138`), right from the working file via `read_issues_from_jsonl` (`sync.rs:1144-1148`),
then:

- strategy is **hardcoded** to `ConflictResolution::PreferNewer` with a TODO
  (`sync.rs:1154-1156`);
- `tombstones` is **hardcoded to `None`** (`sync.rs:1157`) — i.e. the merge path does *not* apply
  tombstone protection at all;
- any `Conflict` aborts the whole command (`sync.rs:1170-1181`) — since `PreferNewer` never returns
  `Conflict`, this branch is effectively unreachable today;
- deletions call `storage.delete_issue(id, "system", "merge deletion", now)` (`sync.rs:1185`), which is
  the ordinary tombstoning mutation (`sqlite.rs:643-689`);
- kept issues are upserted plus relation-synced (`sync.rs:1190-1195`);
- the new base snapshot is `report.kept` only (`sync.rs:1203-1207`);
- finally a **`force: true`** export is performed (`sync.rs:1209-1223`), deliberately bypassing both
  data-loss guards on the grounds that the DB is now authoritative.

---

## 7. Deletion / tombstone semantics

- Deleting an issue never removes the row: `delete_issue` sets `status='tombstone'` plus
  `deleted_at`/`deleted_by`/`delete_reason`/`original_type` and bumps `updated_at`
  (`sqlite.rs:657-686`). Because `get_all_issues_for_export` only filters ephemerals/wisps, tombstones
  are exported and thus propagate through git.
- **Import never resurrects a tombstone.** `determine_action` short-circuits on
  `storage.is_tombstone(existing_id)` before consulting `force_upsert` or timestamps
  (`mod.rs:2098-2103`). `is_tombstone` is a plain `status == "tombstone"` probe (`sqlite.rs:3654-3665`).
  Skips are counted separately as `tombstone_skipped` by sniffing the reason string prefix
  (`mod.rs:2524-2528`) — a string-matching coupling that will break if the message is reworded.
- **Retention / reaping.** `Issue::is_expired_tombstone(retention_days)` (`model/mod.rs:566-586`)
  returns true only when status is Tombstone, retention is `Some(days)` with `days != 0`, `deleted_at`
  is set, and `now > deleted_at + days`. Export drops those lines (`mod.rs:1443-1448`) but still clears
  their dirty flags via `skipped_tombstone_ids` (`mod.rs:1875-1880`). Retention comes from
  `metadata.deletions_retention_days` (`src/config/mod.rs:57`, threaded at `sync.rs:92`,
  `sync.rs:577`). The row stays in SQLite forever; only the exported text drops it.
- **Foreign-prefix tombstones are dropped silently** on import (`mod.rs:2285-2288` then
  `mod.rs:2369-2371`), so a deletion made in another project's prefix will not propagate.

---

## 8. "Never runs git" — what git safety actually means here

### 8.1 The stated contract

`.beads/SYNC_SAFETY_INVARIANTS.md` enumerates 8 non-goals (NG-1…NG-8) and 5 invariant families:
path confinement PC-1…PC-4, atomic write AW-1…AW-4, data loss DL-1…DL-5, input validation
IV-1…IV-4, and no-git NGI-1…NGI-3. The three git ones:

- **NGI-1** — sync never spawns a `git` subprocess.
- **NGI-2** — sync never links libgit2/gitoxide.
- **NGI-3** — sync never reads or writes anything under `.git/`.

### 8.2 How each is enforced

**NGI-1/NGI-2 — static, not runtime.** `SyncSafetyValidator::assert_no_git_in_sync` is a `const fn`
with an empty body — pure documentation (`src/validation/mod.rs:401-417`). The real enforcement is two
tests: `sync_safety_no_git_commands_in_sync_module` shells out to `grep -r "Command::new.*git"
src/sync/` and asserts no match (`validation/mod.rs:846-871`), and
`sync_safety_no_git_library_dependencies` parses the `[dependencies]` section of `Cargo.toml` and
rejects `git2`/`gitoxide`/`gix`/`libgit2` (`validation/mod.rs:877-905`). Note the grep test silently
*passes* if `grep` is unavailable (`validation/mod.rs:864-869`).

Independently verified: no `Command::new` at all appears anywhere under `src/sync/`. Other commands
*do* run git — `changelog.rs:290`, `stats.rs:381`, `orphans.rs:234,245`, `comments.rs:352` — so the
invariant is scoped to the sync module, not the binary.

**NGI-3 — runtime, and it is the strictest rule in the module.** `validate_no_git_path`
(`src/sync/path.rs:137-175`) rejects a path if any `Component::Normal` equals `.git`, or the string
contains `/.git/` or `\.git\` or ends with `/.git`; it then re-checks the *canonicalized* path (and,
for non-existent files, the canonicalized parent) so a symlink into `.git/` is also caught. It is
called first in `validate_sync_path` (`path.rs:211-220`), first in
`validate_sync_path_with_external` **before** the `allow_external` early-return (`path.rs:450-458`),
and first in `validate_temp_file_path` (`path.rs:598-606`). `--allow-external-jsonl` therefore cannot
open a hole into `.git/`. A redundant belt-and-braces check also lives in the CLI:
`contains_git_dir` (`src/cli/commands/sync.rs:261-267`) invoked at `sync.rs:230`.

**Path confinement (PC-1/PC-2/PC-3).** The allowlist is exhaustive and lives in
`src/sync/path.rs:47-59`:

- extensions: `db`, `db-wal`, `db-shm`, `jsonl`, `jsonl.tmp`, `org`, `org.tmp`
- exact names: `.manifest.json`, `metadata.json`

`validate_sync_path` (`path.rs:207-348`) does: git check → reject any `..` component
(`path.rs:222-235`) → canonicalize `beads_dir` → canonicalize the path (or its parent for files that
don't exist yet, `path.rs:253-270`) → explicit symlink-escape check via `read_link` +
canonicalize target (`path.rs:296-312`) → containment under canonical `.beads` (`path.rs:314-333`) →
`validate_extension_and_name` (`path.rs:351-378`, compound extensions handled by
`ends_with(".jsonl.tmp")`).

`validate_sync_path_with_external` (`path.rs:441-500`) is the front door: git always rejected; when
`allow_external` is on it logs at INFO (PC-2) and downgrades to "must be a `.jsonl`/`.org`(`.tmp`) file
with no `..` component", dropping the containment requirement entirely.

`require_safe_sync_overwrite_path` (`path.rs:518-580`) is the extra gate for *destructive* operations
(rename-over, manifest write) and is called at `mod.rs:1516,1522` and `sync.rs:617`.

`validate_temp_file_path` (`path.rs:592-648`) enforces PC-4: temp parent == target parent, `.tmp`
extension, and containment when not external.

### 8.3 The test contract in `tests/e2e_sync_git_safety.rs`

Five regression tests and two integration tests:

- `regression_sync_export_does_not_create_commits` (`:146`), `..._import_...` (`:269`),
  `regression_full_sync_cycle_does_not_touch_git` (`:370`),
  `regression_sync_manifest_does_not_touch_git` (`:488`),
  `regression_sync_never_touches_source_files` (`:536`).
  Method: `init_git_repo` (`:103`) makes a real repo with a commit; the test records
  `git rev-parse HEAD`, `git rev-list --count HEAD`, and a recursive SHA-256 map of everything under
  `.git/` (`hash_directory_contents` `:54`); runs a full export→edit→import→export→status cycle; then
  asserts commit hash, commit count, and the `.git` content map are all unchanged. Transient
  `*.lock`, `index`, and `*HEAD*` entries are filtered out before comparison (`:444-455`) because
  merely *reading* the repo can touch them.
- `integration_sync_only_touches_allowed_files` (`:931`) and
  `integration_sync_manifest_only_touches_allowed_files` (`:1395`): take a full
  `FileTreeSnapshot` (`:679`) of the whole workspace before and after each sync operation, diff it
  (`:723`), and assert **every created/modified/deleted path satisfies `is_allowed_sync_file`**
  (`:637-670`) — i.e. is under `.beads/` and matches the allowlist. Any other change is a "SAFETY
  VIOLATION" and a detailed postmortem log is written to the workspace log dir.

The complementary suites are `tests/e2e_sync_failure_injection.rs` (read-only dirs, temp-file failure,
malformed input, temp cleanup, 0600 permissions), `tests/e2e_sync_preflight_integration.rs`, and
`tests/e2e_sync_fuzz_edge_cases.rs`.

### 8.4 What git safety does *not* cover

`br`/`obr` never stages, commits, installs hooks, or reads `.git/`. It also never *writes* a
`.gitignore` during sync. The user is expected to `git add .beads/issues.org` themselves. The only
git-adjacent artifact the sync engine produces is the merge-conflict-marker *detector* — the design
assumes git will produce conflicted text files and that a human/agent resolves them before import.

---

## 9. History backups (`src/sync/history.rs`)

`backup_before_export` (`history.rs:43-97`) copies the current export file to
`<beads_dir>/.br_history/<stem>.<YYYYMMDD_HHMMSS>.<ext>` before overwriting it. It is skipped when
`config.history.enabled` is false or the target doesn't exist yet. Content-identical consecutive
backups are suppressed by a full byte comparison against the newest backup with the same stem
(`files_are_identical` `history.rs:226-267`, called at `history.rs:80-88`).

`rotate_history` (`history.rs:104-133`) prunes per-stem by *both* count (`max_count`, default 100) and
age (`max_age_days`, default 30) — `HistoryConfig::default()` at `history.rs:22-28`.
`list_backups` (`history.rs:150-213`) parses the timestamp out of the second-to-last dot-component,
requires exactly 15 chars matching `%Y%m%d_%H%M%S`, and accepts only `.jsonl`/`.org` extensions; sorted
newest-first. `prune_backups` (`history.rs:272-301`) is the CLI-facing variant used by
`src/cli/commands/history.rs:299`.

`.br_history/` is gitignored (`.beads/.gitignore`, "Local history backups"), so it is a purely local
undo buffer. Its files still satisfy the sync allowlist (they end in `.jsonl`/`.org` under `.beads/`).

---

## 10. CLI surface (`src/cli/commands/sync.rs`)

`execute` (`sync.rs:83-157`): discover `.beads`, open storage, resolve jsonl path from
`ConfigPaths`, run `validate_sync_paths`, then dispatch. `--status` short-circuits first
(`sync.rs:105-107`); `--flush-only` / `--import-only` / `--merge` are mutually exclusive
(`sync.rs:110-118`); **the default with no mode flag is import** (`sync.rs:141-148`).

`validate_sync_paths` (`sync.rs:159-259`) is a *second*, CLI-level path policy independent of
`src/sync/path.rs`: it canonicalizes the parent, requires a `.jsonl` or `.org` extension
(`sync.rs:190-197`), requires containment in `.beads` unless `--allow-external-jsonl`
(`sync.rs:199-211`), derives the temp and `.manifest.json` paths, and re-checks for `.git`
(`sync.rs:229-239`).

`execute_status` (`sync.rs:269-378`): dirty count + `last_export_time` / `last_import_time` /
`jsonl_content_hash` metadata + staleness. Uses `symlink_metadata` (lstat) deliberately, so touching a
*symlink* to the file counts (`sync.rs:295`), and guards against bare `touch` by re-hashing the file
and comparing to the stored hash (`sync.rs:306-334`).

`execute_flush` (`sync.rs:469-711`): short-circuits with "Nothing to export (no dirty issues)" when
clean and not forced (`sync.rs:493-572`) — but **runs the empty-DB and stale-DB guards inside that
short-circuit too**, so a stale DB is refused even when there is nothing to flush
(`sync.rs:497-551`). Then builds `ExportConfig`, exports, finalizes, and optionally writes
`.beads/.manifest.json` (`sync.rs:606-624`).

`execute_import` (`sync.rs:846-1001`): missing file → success no-op; unless `--force`, it skips
entirely when the recomputed file hash equals the stored `jsonl_content_hash`
(`sync.rs:882-908`); parses `--orphans`; builds `ImportConfig` where
`skip_prefix_validation = force && !rename_prefix` and `force_upsert = force`
(`sync.rs:929-940`); resolves or auto-detects the prefix; imports; then **redundantly re-writes**
`jsonl_content_hash` that `import_from_jsonl` already wrote (`sync.rs:965-966` vs `mod.rs:2488-2490`).

`execute_merge` — see §6.4.

Other entry points into the same engine: `main.rs:133-135` (auto-flush after mutations),
`main.rs:236-243` (auto-import before most read and write commands, gated by `should_auto_import`
`main.rs:161-209`), and `config/mod.rs:398-403` (`--no-db` mode imports the whole file into an
in-memory SQLite on every invocation, `config/mod.rs:388-408`, flushed back by
`flush_no_db_if_dirty` `config/mod.rs:330-359`).

Auto-import policy: `auto_import_if_stale` (`mod.rs:1795-1859`) does nothing unless
`compute_staleness` says the file is newer; `--allow-stale` downgrades to a warning
(`mod.rs:1806-1813`); `--no-auto-import` turns it into a hard error telling you to run
`obr sync --import-only` (`mod.rs:1815-1822`); the auto-import is deliberately strict about prefixes
so a stray foreign file cannot silently merge in (`mod.rs:1824-1834`).

---

## 11. Where every merge decision is made — index

| Decision | Location |
|---|---|
| Which existing row an incoming issue corresponds to (4-phase) | `detect_collision` `src/sync/mod.rs:2050-2087` |
| Insert vs Update vs Skip, tombstone veto, last-write-wins | `determine_action` `src/sync/mod.rs:2089-2139` |
| Executing that decision against SQLite | `process_import_action` `src/sync/mod.rs:2494-2531` |
| Which id an incoming issue is remapped to | `src/sync/mod.rs:2426-2431` (record) + `2446-2464` (apply) |
| Prefix-driven renumbering | `src/sync/mod.rs:2305-2367` |
| Per-issue 3-way merge outcome (8 cases) | `merge_issue` `src/sync/mod.rs:2691-2853` |
| 3-way tombstone resurrection veto | `three_way_merge` `src/sync/mod.rs:2884-2892` |
| Strategy actually used by `obr sync --merge` | `src/cli/commands/sync.rs:1154-1157` (hardcoded `PreferNewer`, `tombstones = None`) |
| Refusing an export that would lose data | `src/sync/mod.rs:1292-1338` and duplicated at `src/cli/commands/sync.rs:497-551` |
| Whether the file is newer than the DB | `compute_staleness` `src/sync/mod.rs:1740-1779`, duplicated at `src/cli/commands/sync.rs:291-342` |
| Whether a path may be touched at all | `validate_sync_path_with_external` `src/sync/path.rs:441-500` |

---

## 12. Risks, oddities, and dead code

### 12.1 CONFIRMED-BY-READING: the 3-way merge always keeps local for pre-existing issues

`merge_issue` case 6 branches on `content_hash` equality (`mod.rs:2766-2767`). But:

- `Issue.content_hash` is `#[serde(skip)]` (`model/mod.rs:374-375`), so anything deserialized from a
  file has `content_hash == None`. `load_base_snapshot` (`mod.rs:2956-2985`) and
  `read_issues_from_jsonl` (`mod.rs:1972-2005`) both go through plain `serde_json::from_str::<Issue>`;
  the Org path starts from `Issue::default()` (`org_bridge.rs:359`) and never sets it either.
- `left` comes from `get_all_issues_for_export()` (`sync.rs:1120`), which selects `content_hash` from
  SQLite (`sqlite.rs:2691`) → `Some(...)`.
- `execute_merge` never calls `normalize_issue`, so nothing recomputes the hashes.

Therefore in every real invocation: `left_changed = Some(h) != None = true` and
`right_changed = None != None = false`, which lands on `(_, false) => Keep(l)` at `mod.rs:2771`.
**Every issue present in the base snapshot silently resolves to the local version, discarding all
external edits**, regardless of timestamps or strategy. Case 7 (convergent creation) is also skewed:
`l.content_hash == r.content_hash` is `Some(h) == None` → false, so identical-content pairs never take
the fast path and always go through the `updated_at` tiebreak.

The unit tests do not catch this because they set hashes by hand — `set_content_hash` `mod.rs:3083`
and `make_issue_with_hash` `mod.rs:4609`.

Secondary effect: when `Keep(right)` *does* fire (cases 3 and 7), the kept issue has
`content_hash = None`, and `upsert_issue_for_import` writes that NULL straight into the `issues` table
(`sqlite.rs:3676-3735`), permanently disabling content-hash dedup for that row until something
recomputes it.

### 12.2 CONFIRMED: git-safety test allowlist has drifted from the real allowlist

`tests/e2e_sync_git_safety.rs:656-663` keeps its own copy of `ALLOWED_EXTENSIONS` containing only
`db, db-wal, db-shm, jsonl, jsonl.tmp` — it is missing `org` and `org.tmp`, which
`src/sync/path.rs:47-55` has had since the Org migration. Combined with the fact that `obr init` now
writes `.beads/issues.org` (`src/cli/commands/init.rs:63,99-101`) and the tests still reference
`.beads/issues.jsonl` (`e2e_sync_git_safety.rs:407,1087`),
`integration_sync_only_touches_allowed_files` should now classify the modified `.beads/issues.org` as a
SAFETY VIOLATION. Timeline supports the drift: the Org default landed 2026-02-18 (`afe1351`) while the
test file was last touched 2026-01-17 (`a6f128a`). Not executed to confirm (no build tree present), but
the code paths are unambiguous.

### 12.3 `--merge` disables tombstone protection

`three_way_merge` accepts a tombstone set, but the only production caller passes `None`
(`src/cli/commands/sync.rs:1157`). DL-3 ("import never resurrects tombstoned issues") therefore holds
for `--import-only` but **not** for `--merge`. The `ConflictResolution` enum has four arms; only
`PreferNewer` is reachable (`sync.rs:1156`), and `MergeConfig` (`mod.rs:2917-2925`, including its
`respect_tombstones` field) is defined and never used anywhere in the repo.

### 12.4 `--merge` can fail on a legitimate deletion

`MergeResult::Delete` pushes the id into `report.deleted` (`mod.rs:2903-2905`) and `execute_merge` calls
`storage.delete_issue(id, ...)` (`sync.rs:1185`), which returns `IssueNotFound` if the row is absent
(`sqlite.rs:651-652`). Cases 1 and 5 produce `Delete` precisely for ids that may not exist locally, so
the merge aborts mid-way — after some upserts have already been applied, since nothing wraps the merge
in a transaction.

### 12.5 Import is not transactional

`import_from_jsonl` performs Phase 3 as a loop of independent `conn.execute` calls
(`mod.rs:2466-2477` → `sqlite.rs:3676`, `3747`, `3768`, `3802`). Parsing/validation happen first, so
malformed *input* cannot half-apply — but a mid-loop database error (disk full, FK violation, lock
timeout) leaves the DB in a partial state with metadata not yet updated. The failure-injection suite
only exercises input-side failures (`tests/e2e_sync_failure_injection.rs:253-432`).

### 12.6 Comment ids are global rowids but are imported verbatim

`comments.id` is `INTEGER PRIMARY KEY AUTOINCREMENT` (`src/storage/schema.rs:122-129`) and
`Comment.id: i64` is serialized to the file (`model/mod.rs:630`). `sync_comments_for_import` deletes
only the *target issue's* comments and then does
`INSERT OR REPLACE INTO comments (id, issue_id, ...)` with the file-supplied id
(`sqlite.rs:3812-3822`). A comment id that collides with a comment belonging to a *different* issue
will silently overwrite it. Cross-repo comment merging is not id-safe.

### 12.7 Preflight is implemented, exported, tested — and not wired in

`preflight_export` (`mod.rs:489-706`) and `preflight_import` (`mod.rs:719-1026`) are ~530 lines of
careful, structured, remediation-carrying checks. Grepping the whole `src/` tree, the only callers are
`tests/e2e_sync_preflight_integration.rs`. The production guards are re-implemented inline in
`export_to_jsonl_with_policy` and `import_from_jsonl`, and a *third* time in
`execute_flush` (`sync.rs:497-551`). Three copies of the DL-1/DL-2 logic.

### 12.8 Two divergent staleness implementations

`compute_staleness` (`mod.rs:1750-1754`) treats an unparseable `last_import_time` as "file is newer"
(`.map_or(true, ...)`), whereas `execute_status` (`sync.rs:299-304`) treats it as "not newer"
(`.is_ok_and(...)`). `obr sync --status` can therefore report "in sync" while the next command's
auto-import decides the file is stale.

### 12.9 Dead / vestigial configuration

- `ImportConfig.orphan_mode` and the whole `OrphanMode` enum (`mod.rs:255`, `285-296`) are parsed from
  `--orphans` (`sync.rs:914-928`), stored, logged — and **never read** inside `import_from_jsonl`.
  Steps 7 ("Orphan handling") in the function's own docstring (`mod.rs:2186`) is not implemented.
- `ExportConfig.is_default_path` (`mod.rs:41`) is set at `sync.rs:578`, `sync.rs:1213`,
  `config/mod.rs:344` and never read.
- `ImportResult.conflict_markers` (`mod.rs:306`) is always empty — markers are a hard error, so the
  field can never be populated.
- `SyncPathPolicy.jsonl_temp_path` is computed (`sync.rs:226-230`) and unused; the struct carries
  `#[allow(dead_code)]` (`sync.rs:67`). The export path computes its own temp name independently
  (`mod.rs:1414-1424`).
- `export_to_writer` / `export_to_writer_with_policy` are only reachable from benchmarks.

### 12.10 Smaller sharp edges

- History backup runs **before** the data-loss guards (`mod.rs:1285` vs `mod.rs:1292`), so a refused
  export still consumes a backup slot.
- Tombstone-skip accounting depends on `reason.starts_with("Tombstone")` (`mod.rs:2525`) matching the
  message minted at `mod.rs:2100`.
- `find_by_content_hash` has no `ORDER BY` and `content_hash` is non-unique
  (`schema.rs:67`, `sqlite.rs:3629`) — with duplicate hashes the dedup target is arbitrary.
- The `--merge` docstring says the ancestor lives in `.beads/base_snapshot.jsonl`
  (`src/cli/mod.rs:2058`) but the code uses `beads.base.jsonl` (`mod.rs:2937`).
- `--merge` finishes with a `force: true` export (`sync.rs:1211`), so both data-loss guards are
  bypassed on the merge path by design.
- `save_base_snapshot` writes with a bare `File::create` (`mod.rs:2938`) rather than going through
  `require_safe_sync_overwrite_path`; it is inside `.beads/` so it is safe today, but it is the one
  sync write that skips the guard.
- Two independent path-policy implementations exist (`src/sync/path.rs` and
  `validate_sync_paths` in `src/cli/commands/sync.rs:159`), plus a third partial one in
  `SyncSafetyValidator::validate_path_containment` (`src/validation/mod.rs:367-399`).
- `.br_history` and much of the documentation still say `br`, while the binary is `obr`
  (`Cargo.toml:12-14`, `history.rs:54`).
- `#[serde(untagged)] Custom(String)` on `Status` and `IssueType` means a typo'd status in a
  hand-edited Org/JSONL file is accepted silently as a new custom status rather than rejected.
