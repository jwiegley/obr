# R2 — Org-mode port specification (fork semantics → upstream v0.2.22 substrate)

**Brief:** Phase 1 / R2 of `UPSTREAM_SYNC_AND_RENAME_PLAN.md`.
**Produced:** 2026-08-06, read-only reconnaissance.
**Fork tree read:** `/Users/johnw/src/obr` @ `main` (fork point `e805fe98`).
**Upstream tree read:** `/Users/johnw/src/obr-org-forward-port` (worktree at `upstream/main` = `d1fb0d3a`, v0.2.22). Nothing was written, built, or modified there.
**org2jsonl read:** `/Users/johnw/src/org2jsonl` @ `59521f9` (clean tree).

Every claim below is marked **[V]** (VERIFIED — I read the file/line or ran the command myself) or **[I]** (INFERRED — deduced from verified facts). Fork facts and upstream facts are labelled separately. Line numbers for the upstream worktree are as of `d1fb0d3a`; re-verify with the greps given before editing.

---

## Summary table — integration points and their new homes

| # | Feature / touchpoint | Fork source of truth | Upstream new home (`/Users/johnw/src/obr-org-forward-port`) | Action | Risk |
|---|---|---|---|---|---|
| 1 | `org2jsonl` dependency | `Cargo.toml:64` (path dep) | `Cargo.toml:38` `[dependencies]` — upstream has **zero** git deps | Add `git`+`rev` dep, rev `59521f9`; regen `Cargo.lock` | **High** (CI/build reproducibility) |
| 2 | `org_bridge` module | `src/sync/org_bridge.rs` (900 lines) | new file `src/sync/org_bridge.rs`; register in `src/sync/mod.rs:~30` (`pub mod history; pub mod path;`) | Port + extend | Med |
| 3 | Default export filename | `src/config/mod.rs:33,35,37` | `src/config/mod.rs:62` `DEFAULT_JSONL_FILENAME = "issues.jsonl"`, `:64` `LEGACY_JSONL_FILENAME` | 3-tier: `issues.org` → `issues.jsonl` → `beads.jsonl` | Med |
| 4 | Discovery | fork `src/config/mod.rs:106-127` | `src/config/mod.rs:147-160` `discover_jsonl` + `:4219-4261` `resolve_jsonl_path` | Add `.org` tier | Med |
| 5 | Path allowlist | fork `src/sync/path.rs:49-56` | `src/sync/path.rs:85-94` `ALLOWED_EXTENSIONS`; `:526-558` `is_allowed_jsonl_temp_name`; `:679-692` `validate_external_jsonl_path` | Add `org`, `org.tmp`, pid-scoped `org.<pid>.tmp` | Med |
| 6 | Temp file naming | fork `src/sync/mod.rs:1416-1426` | `src/sync/mod.rs:1814-1828` `export_temp_path[_for_attempt]` (now **pid-scoped**) | Make extension-aware | Med |
| 7 | Export emission | fork `src/sync/mod.rs:1443-1508` (whole-file) | `src/sync/mod.rs:9386-9418` `write_export_issue_jsonl`, `:9463-9490` `prepare_export_issue_jsonl`, `:9555-9598` `write_prepared_export_entries`, driver `:9704-10113` | Per-issue record writer + file header; keep streaming/batched/parallel shape | **High** |
| 8 | **Export/file content hash mismatch** | n/a (fork's hash normalizer is laxer) | `src/sync/mod.rs:12769-12790` `compute_jsonl_hash_from_reader`; `src/sync/path.rs:2274-2301` `compute_snapshot_content_sha256` | Make canonical hash format-aware (`.org` ⇒ raw bytes) or **export always fails** | **Critical** |
| 9 | Import parse | fork `src/sync/mod.rs:2223-2252` | `src/sync/mod.rs:12077-12095` `for_each_jsonl_import_issue` (4 call sites) | Single format-aware chokepoint | **High** |
| 10 | Incremental (line-splice) auto-flush | **did not exist at fork point** | `src/sync/mod.rs:11073-11354`, `:11356-11585`, `:11620` `auto_flush` | **Must be disabled for `.org`** | **Critical** |
| 11 | Export verification | fork: none | `src/sync/mod.rs:9076-9172` `verify_exported_jsonl_snapshot_integrity` | Org-aware verifier | High |
| 12 | Safety-guard analysis | fork `src/sync/mod.rs:1133-1202` | `src/sync/mod.rs:9012-9066` `analyze_jsonl[_from_reader/_snapshot]` | Org counter + dup-ID detection | High |
| 13 | Record validation (doctor) | fork `doctor.rs` inline | `src/sync/mod.rs:8246-8296` `validate_jsonl_issue_records*` | Org branch | Med |
| 14 | Base merge anchor | fork: unhandled | `src/sync/mod.rs:13918-13969` save/refresh, `:14018-14022` `load_base_snapshot` (`beads.base.jsonl`) | Byte-copy of Org into a `.jsonl` anchor **breaks merge** — re-serialize instead | **High** |
| 15 | Merkle witness | **did not exist at fork point** | `src/sync/witness.rs` (1,894 lines, line-chunked); used by `src/cli/commands/sync.rs:1994,2045,2086` | Gate to JSONL or document Org semantics | Med |
| 16 | Reviewed-additive reconcile | **did not exist** | `src/sync/mod.rs:4565-4682` `reject_unknown_additive_fields`, `:4683` `parse_strict_additive_issue`, `:3694`/`:3759` plan/apply | Reject `.org` with a clear error (strict JSON-field semantics have no Org analogue) | Med |
| 17 | History backups | fork `src/sync/history.rs:70-85,168-185` | `src/sync/history.rs:171-184` `parse_backup_filename`, `:192-223` `create_backup_file_for_timestamp`, `:227` `backup_metadata_path`, `:644-648` listing, `:384` `legacy_backup_target_path` | Extension-aware (now also `.meta.json` sidecars) | Med |
| 18 | `doctor` | fork `a0c54428` (`check_export_file`) | `src/cli/commands/doctor.rs` (24,752 lines): `:10439` `discover_jsonl`, `:10451` `should_fallback_to_workspace_jsonl`, `:10580` `check_jsonl`, `:4672/4885/4939/5143/5201/5371` byte checks, `:4012` hash-cache, `:7959/10202` base anchor | ~10 sites | Med |
| 19 | `stats` | fork `a0c54428` | `src/cli/commands/stats.rs:92` (only hardcoded site); `:644-656` `compute_recent_activity` **already takes a path + computes repo-relative pathspec** | 1-line change — fork's fix is *superseded* | Low |
| 20 | `init` | fork `afe1351e` | `src/cli/commands/init.rs:173` metadata JSON, `:252-257` empty-file seed, `:360` step label, `:406` tree art | Constant-ise | Low |
| 21 | `sync` CLI path policy | fork `src/cli/commands/sync.rs:202-230` | `src/cli/commands/sync.rs:1548-1557` extension gate; `:1571` `export_temp_path` | Accept `.org` | Low |
| 22 | `vcs` diagnostics | did not exist | `src/cli/commands/vcs.rs:259-263` hard `.jsonl`-only gate | Accept `.org` or `br vcs` dies on every Org workspace | Med |
| 23 | `--no-db` | fork: implicit | `src/config/mod.rs:3728-3796` (seeds via `import_from_jsonl_snapshot`), `:3324-3389` `flush_no_db_if_dirty` (`:3352` hardcoded default path) | Free once #9 lands; fix `:3352` | Low |
| 24 | Bootstrap prefix inference | fork `src/config/mod.rs:456-476` | `src/config/mod.rs:4011-4035` + `:4146-4186` `first_prefix_from_jsonl_{snapshot,reader}` | Org branch | Med |
| 25 | e2e estate | fork `c046b35e` (5 files) | 143 test targets; 59 mention `issues.jsonl` | Per-file rule in §6 | **High** |

---

## 0. Method, and what this document is

This is the *normative* specification for the Org feature. It is written so an implementer can reconstruct the behaviour without reading the fork at all, then map each behaviour onto a named upstream function.

Sources read in full: the six Org commits (`afe1351e a0c54428 c046b35e 03c5cd2c 33bb2e82 56e4a0fc`); the fork's `src/sync/org_bridge.rs` in its entirety; `docs/research/map-sync.md` §1.2–1.3; `docs/research/gap-org2jsonl.md` §§0–7; and the relevant upstream files listed in the summary table. **[V]**

Two framings the implementer must hold throughout:

1. **Silent drops are this codebase's #1 historical failure mode** (plan §7.2, dossier R23/R9). Every field, every element type, every code path that could discard data gets an explicit disposition here: *carried*, *derived*, or *explicitly rejected with an error*. "Not mentioned" is not a disposition.
2. **Upstream's sync engine grew a durability/authority layer that the fork never saw** — immutable source snapshots, family write locks, staged-publication receipts, per-issue content-hash mappings, Merkle witnesses, line-splice incremental flush. The Org port is no longer "swap the serializer"; it is "add a second record format to a hardened streaming pipeline". Items 8, 10, 11 and 14 in the summary table are places where naive porting produces a *silently or loudly broken* build.

---

## 1. The Org feature, specified

### 1.1 Format selection

**Rule:** the format is chosen per-file, by the target path's **extension**, case-sensitively lowercase, at every read and write site. `.org` → Org bridge; anything else → JSONL. **[V]** fork `src/sync/mod.rs` afe1351e diff: three independent `path.extension().and_then(|e| e.to_str()) == Some("org")` probes in `analyze_jsonl`, `export_to_jsonl_with_policy`, `read_issues_from_jsonl`, `import_from_jsonl`.

There is **no** config key, no CLI flag, and no in-file magic that selects the format. This is deliberate and should be preserved: `br sync --flush-only --jsonl /tmp/x.jsonl` exports JSONL from an Org workspace, and vice versa.

**Porting instruction:** replace the four ad-hoc probes with one helper in `src/sync/org_bridge.rs`:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExportFormat { Jsonl, Org }
impl ExportFormat {
    #[must_use] pub fn for_path(path: &Path) -> Self { … extension == Some("org") … }
    #[must_use] pub const fn temp_extension(self) -> &'static str { … }
}
```

Every site in §3 dispatches on this one function. Do not re-derive the extension inline — the fork already had three subtly different spellings of the same probe, and upstream has ~15 more places that need it.

### 1.2 Filenames and discovery

Fork constants **[V]** (`src/config/mod.rs:33,35,37`):

```rust
pub const DEFAULT_JSONL_FILENAME: &str = "issues.org";
const LEGACY_JSONL_FILENAME: &str = "issues.jsonl";
const OLDER_LEGACY_JSONL_FILENAME: &str = "beads.jsonl";
```

Note the fork made `DEFAULT_JSONL_FILENAME` **`pub`** (it was private upstream) because `history.rs`, `stats.rs` and `sync/mod.rs` all needed it.

Discovery order (fork `src/config/mod.rs:106-127`) **[V]**: `issues.org` → `issues.jsonl` → `beads.jsonl` → `None`. Merge artifacts and log files are still excluded via the pre-existing `EXCLUDED_JSONL_FILES` list.

`metadata.json` written by `init` sets `"jsonl_export": "issues.org"` **[V]** (afe1351e, `src/cli/commands/init.rs`).

**Upstream state [V]:** `src/config/mod.rs:62` still `"issues.jsonl"`; `discover_jsonl` (`:147-160`) is 2-tier; `resolve_jsonl_path` (`:4219-4261`) has 5 priorities — `BEADS_JSONL` env, explicit non-default `metadata.jsonl_export`, DB-override sibling (which itself calls `discover_jsonl`), discovery, default.

**Porting instruction:**
- `src/config/mod.rs:62-64`: `DEFAULT_JSONL_FILENAME = "issues.org"` (make it `pub`), `LEGACY_JSONL_FILENAME = "issues.jsonl"`, add `OLDER_LEGACY_JSONL_FILENAME = "beads.jsonl"`.
- `discover_jsonl` (`:147-160`): three tiers in that order.
- `resolve_jsonl_path` priority 2 (`:4232-4234`): `metadata_jsonl != DEFAULT_JSONL_FILENAME` now means "not `issues.org`" — a workspace whose `metadata.json` still says `issues.jsonl` will therefore be treated as an *explicit override* and keep using JSONL. **This is the correct and desirable migration behaviour** (existing workspaces do not silently switch format), and it must be stated in the changelog. Do not "fix" it.
- Every literal `beads_dir.join("issues.jsonl")` in non-test code becomes `beads_dir.join(config::DEFAULT_JSONL_FILENAME)`. Non-test sites found **[V]**: `src/config/mod.rs:3352`, `src/cli/commands/stats.rs:92`, `src/cli/commands/doctor.rs:7959,10202,10440,10456`.

### 1.3 File-level layout

Emitted by `issues_to_org_text` (fork `org_bridge.rs:28-41`) **[V]**:

```
#+TITLE: Beads Issues
#+SEQ_TODO: TODO DOING WAIT DEFER NOTE | DONE CANCELED
<blank>
<issue heading block>*
```

An empty issue set still emits the two header lines and a blank line.

The `#+SEQ_TODO:` line is **written but never read** — parsing always uses the hard-coded keyword arrays (§1.13). **[V]** gap-org2jsonl §5.5. Keep writing it (Emacs needs it to render the file), but the port must not pretend it is authoritative.

`#+TITLE: Beads Issues` and the `BEADS_*` identifiers are **Phase-6 rename targets** (§8, plan §6 note). Recommended new spellings and their compatibility policy are in §2.4.

### 1.4 The heading line

Fork `emit_issue_heading` (`org_bridge.rs:45-76`) **[V]**, exactly:

```
"* " ++ <KEYWORD> ++ " " ++ <PRIORITY-COOKIE> ++ " " ++ sanitize_property_value(title)
     ++ [ "    :" ++ label1 ++ ":" ++ label2 ++ … ++ ":" ]   (only when labels non-empty)
     ++ "\n"
```

- Always level 1. Nested issues are not a thing; parent/child is a dependency, emitted in the deps block.
- The priority cookie is **always** emitted, including `[#C]` for the default MEDIUM.
- Tag separator from the title is exactly **four spaces**; tags are `:`-joined and `:`-terminated.
  - Superseded (4afe14c2): four spaces remains the default, but a surface
    declaring `# -*- org-tags-column: N -*-` on its first line is aligned by
    Org's own rule instead. See `push_tag_gap` in `src/sync/org_bridge.rs`.
- Labels are sorted ASCII-ascending before emission (`org_bridge.rs:65-66`). **[V]**
- Labels are emitted **raw, unsanitized** — this is bug **R9** (§4).
- The title is passed through `sanitize_property_value`, which only removes `\n`/`\r` and defuses `:END:`. It does **not** protect a title that ends in something Org will read back as tags (`gap-org2jsonl` §3.5). **[V]**

### 1.5 Status ↔ Org keyword — the exact map

This is the map the plan §6.3 explicitly told me not to guess. Read from `git show 03c5cd2c` and `git show 56e4a0fc` and confirmed against the current file. **[V]**

**Export** (`org_bridge.rs:262-273`):

| `Status` | Keyword |
|---|---|
| `Open` | `TODO` |
| `InProgress` | `DOING` |
| `Blocked` | `WAIT` |
| `Deferred` | `DEFER` |
| `Closed` | `DONE` |
| `Tombstone` | `CANCELED` |
| `Pinned` | `NOTE` |
| `Custom(s)` | `s.to_uppercase()` |

**Import** (`org_bridge.rs:276-287`, post-`56e4a0fc` merged arms):

| Accepted keyword(s) | `Status` |
|---|---|
| `TODO`, `OPEN` | `Open` |
| `DOING`, `IN_PROGRESS`, `INPROGRESS` | `InProgress` |
| `WAIT`, `BLOCKED` | `Blocked` |
| `DEFER`, `DEFERRED` | `Deferred` |
| `DONE`, `CLOSED` | `Closed` |
| `CANCELED`, `TOMBSTONE` | `Tombstone` |
| `NOTE`, `PINNED` | `Pinned` |
| anything else | `Status::from_str(keyword)` |

The comparison is on `keyword.to_uppercase()`. The legacy (internal-name) spellings exist because `afe1351e` shipped `#+SEQ_TODO: OPEN IN_PROGRESS BLOCKED DEFERRED PINNED | CLOSED TOMBSTONE` and `03c5cd2c` changed it two days later; files written in that window must still import. **[V]**

Keyword arrays (`org_bridge.rs:312-324`) **[V]**:

```rust
pub const BEADS_TODO_KEYWORDS: &[&str] = &["TODO", "DOING", "WAIT", "DEFER", "NOTE"];
pub const BEADS_DONE_KEYWORDS: &[&str] = &["DONE", "CANCELED"];
```

Only keywords in these arrays are recognised by the parser as keywords at all; anything else is absorbed into the title (§4, bug B5).

**Upstream delta [V]:** `Status` gained a `Draft` variant (`src/model/mod.rs:58-70`; `as_str()` → `"draft"` at `:104`; `known_value` at `:88`). The fork's map has no entry for it. **Required addition:**

| `Status::Draft` | `DRAFT` (new keyword, active side) |

and `BEADS_TODO_KEYWORDS` becomes `["TODO", "DOING", "DRAFT", "WAIT", "DEFER", "NOTE"]`, with the `#+SEQ_TODO:` line updated to match. Import accepts `DRAFT` → `Draft`. Rationale for `DRAFT` over any Emacs-idiomatic alternative: the whole map is already "internal name, uppercased, shortened only where a standard Org keyword exists"; `DRAFT` is unambiguous, is not a standard keyword with conflicting meaning, and keeps the legacy-alias table trivial (`DRAFT` is both the Org keyword and the internal name).

**Version-skew note [I]:** a file containing `* DRAFT …` read by a binary built before this change parses the keyword into the *title* and defaults the status to `Open` — silently. That is the pre-existing §5.5 failure mode, not a new one, but it argues for the parser-identity marker in §2.4.

**Custom-status hazard [V]** (gap-org2jsonl §5.5): `Custom(s) => s.to_uppercase()` produces a keyword that is by construction absent from `BEADS_TODO_KEYWORDS`, so it round-trips into the title and the status silently becomes `Open`. Upstream still admits `Status::Custom` (`src/model/mod.rs:69`, `FromStr` at `:137-143` never errors) and `normalize_issue` (`src/sync/mod.rs:11983-11996`) maps a few Go-`bd` aliases into `Closed` but leaves the rest. **Required:** the exporter must **refuse** to write an issue whose status is `Custom(_)` to an Org file, with an actionable error naming the issue id and the offending status, rather than emitting an unparseable keyword. This is the "explicitly reject rather than silently drop" rule.

### 1.6 Priority ↔ cookie

Fork `org_bridge.rs:290-309` **[V]**:

| `Priority.0` | cookie | reverse |
|---|---|---|
| 0 `CRITICAL` | `[#A]` | `A` → 0 |
| 1 `HIGH` | `[#B]` | `B` → 1 |
| 2 `MEDIUM` | `[#C]` | `C` → 2 |
| 3 `LOW` | `[#D]` | `D` → 3 |
| 4 `BACKLOG` | `[#E]` | `E` → 4 |
| anything else | `[#C]` | anything else → 2 |

Both directions collapse out-of-range values to MEDIUM. `Priority` is `pub struct Priority(pub i32)` **[V]** — nothing constrains it to 0..=4 at the type level, so a hand-set `Priority(7)` is silently rewritten to 2 on export. Upstream's `Priority` is unchanged (`src/model/mod.rs:150-164`). **[V]** Recommendation: keep the collapse (matching JSONL would require inventing cookies), but log a `tracing::warn!` naming the issue id when it fires — currently it is silent.

### 1.7 The `:PROPERTIES:` drawer — exact contents and order

From `org_bridge.rs:79-168`, in emission order. **[V]** All `String` values go through `sanitize_property_value`; all `DateTime<Utc>` through `.to_rfc3339()`; all integers through `{}`.

| # | Key | Source field | Emitted when | Notes |
|---|---|---|---|---|
| 1 | `BEADS_SCHEMA_VERSION` | literal `1` | always | **write-only** — the parser explicitly discards it (`:735-738`). Phase-6 rename target. |
| 2 | `ID` | `issue.id` | always | format string is `":ID:       {}\n"` — **seven** spaces. Only *required* property on read. |
| 3 | `ISSUE_TYPE` | `issue_type.as_str()` | always | **not** sanitized (`:82`) — see hazard below |
| 4 | `CREATED_AT` | `created_at` | always | RFC3339 |
| 5 | `UPDATED_AT` | `updated_at` | always | RFC3339 |
| 6 | `ASSIGNEE` | `assignee` | `Some` | |
| 7 | `OWNER` | `owner` | `Some` | |
| 8 | `ESTIMATED_MINUTES` | `estimated_minutes` | `Some` | i32 |
| 9 | `CREATED_BY` | `created_by` | `Some` | |
| 10 | `CLOSED_AT` | `closed_at` | `Some` | RFC3339 |
| 11 | `CLOSE_REASON` | `close_reason` | `Some` | |
| 12 | `CLOSED_BY_SESSION` | `closed_by_session` | `Some` | |
| 13 | `DUE_AT` | `due_at` | `Some` | RFC3339 |
| 14 | `DEFER_UNTIL` | `defer_until` | `Some` | RFC3339 |
| 15 | `EXTERNAL_REF` | `external_ref` | `Some` | |
| 16 | `SOURCE_SYSTEM` | `source_system` | `Some` | |
| 17 | `SOURCE_REPO` | `source_repo` | `Some` | |
| 18 | `DELETED_AT` | `deleted_at` | `Some` | RFC3339 |
| 19 | `DELETED_BY` | `deleted_by` | `Some` | |
| 20 | `DELETE_REASON` | `delete_reason` | `Some` | |
| 21 | `ORIGINAL_TYPE` | `original_type` | `Some` | |
| 22 | `COMPACTION_LEVEL` | `compaction_level` | `Some(l)` **and `l > 0`** | asymmetric: `Some(0)` reads back as `None` |
| 23 | `COMPACTED_AT` | `compacted_at` | `Some` | RFC3339 |
| 24 | `COMPACTED_AT_COMMIT` | `compacted_at_commit` | `Some` | |
| 25 | `ORIGINAL_SIZE` | `original_size` | `Some` | i32 |
| 26 | `SENDER` | `sender` | `Some` | |
| 27 | `EPHEMERAL` | `ephemeral` | `true` only | value literal `true` |
| 28 | `PINNED` | `pinned` | `true` only | value literal `true` |
| 29 | `IS_TEMPLATE` | `is_template` | `true` only | value literal `true` |

Then `:END:`.

Read side (`parse_properties`, `org_bridge.rs:585-751`) **[V]**: a `match` on the key with a catch-all `"BEADS_SCHEMA_VERSION" | _ => {}` that **silently ignores unknown properties**. Timestamp and integer parses produce `BeadsError::Validation { field, reason }`. After the loop, an empty `issue.id` is a hard `Validation` error (`:742-748`).

**Consequences of "only `:ID:` is required" [I], worth a regression test:** an issue whose drawer omits `CREATED_AT`/`UPDATED_AT` inherits `Issue::default()`, whose `created_at`/`updated_at` are `Utc::now()` (upstream `src/model/mod.rs:646,648` **[V]**). A hand-edited heading therefore silently acquires today's timestamps and wins every last-write-wins comparison. Recommendation: warn (not error) when `CREATED_AT`/`UPDATED_AT` are absent, and name the issue id.

**`ISSUE_TYPE` un-sanitized (`org_bridge.rs:82`) [V]:** `IssueType::Custom(s)` reaches the drawer verbatim. Upstream's `IssueType` is unchanged and still has an untagged `Custom(String)` (`src/model/mod.rs:194-205` **[V]**). A custom type containing a newline would break the drawer. **Required:** route it through `sanitize_property_value` like every other string, or reject non-conforming custom types at export with a named error.

### 1.8 Body and child sections

Fork `org_bridge.rs:171-233` **[V]**:

| Content | Emission | Guard |
|---|---|---|
| `description` | blank line, `sanitize_org_text(desc)`, `\n` | `Some(_)` — **no `is_empty()` check** |
| `design` | blank line, `** Design\n`, sanitized body, `\n` | `Some(s)` and `!s.is_empty()` |
| `acceptance_criteria` | `** Acceptance Criteria` | same |
| `notes` | `** Notes` | same |
| `dependencies` | `** Dependencies` + JSON src block | `!is_empty()` |
| `comments` | `** Comments` + JSON src block | `!is_empty()` |

then a trailing blank line per issue (`:236`).

Read side (`parse_heading_to_issue`, `:391-431`): iterate `heading.children`, take only `child.level == 2`, dispatch on the **flattened plain-text child title string** — `"Design"`, `"Acceptance Criteria"`, `"Notes"`, `"Dependencies"`, `"Comments"`. **Any other level-2 child, and every level ≥ 3 child, is silently discarded** (`:426-428`, `:392`). **[V]**

`description` is set only when the extracted body text is non-empty (`:386-388`), so `Some("")` → `None`. **[V]** Asymmetric with the emission guard above.

### 1.9 The JSON src blocks

```
** Dependencies
#+begin_src json
<serde_json::to_string_pretty(&issue.dependencies)>
#+end_src
```

identically for `** Comments`. On serialization failure the fork writes the literal `"[]"` (`org_bridge.rs:216-217`, `:228-229` `unwrap_or_else`) — a **silent total loss of the relation set**. **[V]** **Required change:** propagate the error (this is exactly the class the fork's own `aa129a2c` commit was about).

Read side: `extract_json_from_body` (`:557-566`) returns the `value` of the **first** `Element::SrcBlock` in the child's body whose `language == "json"`, then `serde_json::from_str::<Vec<Dependency>>` / `<Vec<Comment>>` with the error mapped to `BeadsError::Json`. **[V]**

**Safety-by-construction note [I], worth stating so nobody "hardens" it wrongly:** `to_string_pretty` escapes every newline inside string values, and every line it emits begins with `[`, `]`, `{`, `}`, or indentation-then-`"`. Therefore no line of a dependencies/comments block can begin with `*` or `#+`, and the block cannot be terminated early by user content. This holds only as long as the block content is machine-serialized JSON — see §2.2 for `agent_context`, where it does *not* hold automatically.

**Determinism [V]** (gap-org2jsonl §4, confirmed by reading upstream): the fork sorted labels in the emitter; upstream additionally normalizes `labels`/`dependencies`/`comments` ordering during hydration in `normalize_issue_for_export` (`src/sync/mod.rs:10821-10850`), called from `populate_export_issue_relations` (`:9382`). Keep the emitter's own label sort as belt-and-braces, and rely on upstream's total order for deps/comments.

### 1.10 Sanitization

Two functions, and they are **not inverses** — this is bug B4 (§4).

```rust
// org_bridge.rs:241-245
fn sanitize_property_value(v: &str) -> String {
    v.replace(['\n', '\r'], " ").replace(":END:", ":END ")
}
// org_bridge.rs:248-259
fn sanitize_org_text(t: &str) -> String {           // lines starting with '*' get a leading comma
    t.lines().map(|l| if l.starts_with('*') { format!(",{l}") } else { l.to_string() })
     .collect::<Vec<_>>().join("\n")
}
// org_bridge.rs:569-580
fn unsanitize_org_text(t: &str) -> String {          // ANY line starting ",*" loses the comma
    t.lines().map(|l| if l.starts_with(",*") { l[1..].to_string() } else { l.to_string() })
     .collect::<Vec<_>>().join("\n")
}
```

`sanitize_org_text` additionally normalizes CRLF → LF and drops a trailing newline, because `str::lines()` does (gap-org2jsonl §4 caveat). **[V]** That is stable but lossy on first write.

### 1.11 Determinism guarantees to preserve

Measured in the fork **[V]** (gap-org2jsonl §4): byte-identical output across 11 runs and across processes. Sources of determinism: issues iterated in `ORDER BY id ASC` (upstream `export_issue_ids`, `src/sync/mod.rs:9222-9235` **[V]**); labels/deps/comments totally ordered; no `HashMap` iteration reaches output; properties emitted by an explicit `push_str` sequence; timestamps via `to_rfc3339()`.

**New upstream hazard [V]:** `prepare_export_issues_jsonl_parallel` (`src/sync/mod.rs:9506-9553`) shards issues across threads and reassembles by `start_index` before writing, so output order is preserved. Per-issue Org emission is a pure function of the issue, so parallelism is safe for Org too — **provided** the Org record producer is pure (no shared hasher, no shared counter). Do not introduce cross-issue state into the emitter (e.g. a comment-id renumbering pass).

### 1.12 Parse pipeline and error semantics

```rust
// org_bridge.rs:335-355
pub fn org_text_to_issues(org_text: &str) -> Result<Vec<Issue>> {
    let entries = org2jsonl::org_to_json::org_to_entries_with_keywords(
        org_text, BEADS_TODO_KEYWORDS, BEADS_DONE_KEYWORDS);
    for entry in entries {
        if let EntryContent::Heading(h) = entry.content {
            if h.level == 1 { issues.push(parse_heading_to_issue(&h)?); }
        }
    }  // Section entries (the file header) skipped
}
```
**[V]** Note the `?`: **one bad heading aborts the entire parse** and therefore every command in the workspace (bug B2, §4).

### 1.13 The `org2jsonl` seam

Complete surface **[V]** (gap-org2jsonl §2.1, re-verified against the current fork file):

| Symbol | Kind | Used at |
|---|---|---|
| `org2jsonl::org_to_json::org_to_entries_with_keywords(&str, &[&str], &[&str]) -> Vec<OrgEntry>` | the **only** function | `org_bridge.rs:336-340`, `src/sync/mod.rs:1178-1182` |
| `org2jsonl::model::EntryContent` | enum (`Section` \| `Heading(Box<Heading>)`) | `org_bridge.rs:12,344`; `src/sync/mod.rs:1188` |
| `org2jsonl::model::Heading` | struct (`level, keyword, priority, title, tags, planning, properties, body, children, …`) | `org_bridge.rs:358` |
| `org2jsonl::model::Element` | enum, 23 variants | `org_bridge.rs:540,557` |
| `org2jsonl::model::InlineContent` | enum, 22 variants | `org_bridge.rs:451` |
| `org2jsonl::model::Property` | `{ key, value }` | `org_bridge.rs:585` |

The writer half of the crate (`json_to_org`, 2,231 lines) is **never called** — obr writes Org by hand. Its bugs (CLOCK duplication, unbounded-growth non-idempotency) therefore do not reach obr, but they mean the crate's own fidelity oracle is unreliable. **[V]**

Element coverage: **1 of 23** handled (`Paragraph`), 1 conditionally consumed (`SrcBlock` under a recognised child), 21 dropped. InlineContent coverage: 14 of 22 handled, 1 mangled (`Entity` gains `{}`), 7 dropped. **[V]** gap-org2jsonl §§2.3–2.4. This is the loss surface and it is *obr's*, not the parser's (§2.5 there): the parser reproduces lists, tables, src blocks, drawers, comments and fixed-width lines byte-for-byte.

**Porting instruction — do not silently keep the drop.** The port has three defensible options; pick one and write it down:

- **(a) Preserve-by-fallback (recommended, ~40 lines).** Keep `extract_body_text` as the semantic extractor, but when the body contains **any** element other than `Element::Paragraph`, fall back to a *raw slice* of the heading body. With the pinned rev this is now possible: `char_begin`/`char_end` exist on `OrgEntry` (org2jsonl `src/model.rs:10-25` at `59521f9` **[V]**) — see §5 for the tiny upstream-crate addition needed to get them together with custom keywords. This converts 21 silent drops into 0.
- **(b) Explicit reject.** Error on export if a description contains a line that would parse as a non-paragraph element. Safe but hostile to the human-editing workflow that motivates the format.
- **(c) Status quo + documented lossiness.** Keep the drops, but make `G-org-roundtrip` (§7) assert them as *known* rather than discovering them.

Option (a) also fixes the `i > 0` separator artifact (`Before.\n\n\nAfter.`) that makes the pipeline take two cycles to converge (gap-org2jsonl §§2.3, 3.6). **[V]**

---

## 2. Upstream model delta → Org representation

Diffed the `Issue` struct field lists between the two trees. **[V]**

```
$ diff <(fork Issue fields) <(upstream Issue fields)
> pub agent_context:
> pub source_repo_path:
```

Plus the `Status::Draft` variant (§1.5). `Priority`, `IssueType`, `Dependency`, `Comment`, `DependencyType` are **unchanged**. **[V]** (`Dependency` gained only a `deserialize_optional_metadata` attribute at `src/model/mod.rs:897-902`, which is a serde behaviour change, not a field.)

### 2.1 `source_repo_path: Option<String>`

Upstream `src/model/mod.rs:557-566` **[V]**. An absolute canonical filesystem path; distinct from `source_repo` (a basename); used by multi-repo fleet routing (beads_rust#289).

**Representation: property drawer, `:SOURCE_REPO_PATH:`**, inserted immediately after `:SOURCE_REPO:` (position 17.5 in the §1.7 table), sanitized like every other string, `Some`-guarded. A path may contain `:` — harmless, since only the leading `:KEY:` is structural in a drawer. It may contain `\n` only pathologically; sanitization covers it.

### 2.2 `agent_context: Option<String>`

Upstream `src/model/mod.rs:568-585` **[V]**. A **canonical-JSON document** (open schema: `skills`, `constraints`, `references`, `workflow`, `metadata`), inherited by descendant beads, surfaced by `br show` and `br update --claim`. Potentially multi-line and long.

**A property drawer entry is wrong for this field.** `sanitize_property_value` replaces newlines with spaces, which for pretty-printed JSON is *lossy in a way that still parses* — the worst possible failure mode. **[I]**

**Representation: a level-2 child `** Agent Context` containing a `#+begin_src json` block**, symmetric with Dependencies/Comments:

```
** Agent Context
#+begin_src json
{ …verbatim stored text… }
#+end_src
```

Read side: add `"Agent Context" => issue.agent_context = extract_json_from_body(&child.body)` to the dispatch at `org_bridge.rs:396-429`, storing the block value **verbatim** (not re-serialized) so the DB text is unchanged.

Three requirements that make this safe:
1. **Emit exactly one trailing `\n` after the value** and strip exactly one on read, matching the deps/comments convention.
2. **Guard the block terminator.** Unlike the machine-serialized deps/comments blocks, `agent_context` is arbitrary stored text. Before emitting, apply an Org block escape: prefix a comma to any line whose first non-blank characters are `*`, `#+`, or `,`; reverse it on read. Implement this as one symmetric pair (`escape_block_lines` / `unescape_block_lines`), unlike the existing asymmetric `sanitize_org_text`/`unsanitize_org_text` (bug B4). Valid JSON cannot in fact produce such a line **[I]** (a raw `#` outside a string is not JSON, and JSON strings cannot contain raw newlines), so the guard is cheap insurance rather than a hot path.
3. **Do not canonicalize.** Re-serializing through `serde_json::Value` would rewrite the DB's stored text on the next import and change `content_hash` for issues nobody touched — the §3.7 phantom-change mechanism.

**Alternative if the implementer wants the smaller diff:** explicitly reject — refuse to export an Org file when any issue has `agent_context.is_some()`, with an error naming the issue id and pointing at `--jsonl`. That is acceptable *only* if `inherited_context.enabled` is off by default upstream and the reject is loud. Silent drop is not acceptable.

### 2.3 Field-coverage audit rule (make this a test, not a review item)

The fork covered every `Issue` field except the `#[serde(skip)]` `content_hash`, but nothing enforced it, which is why two new upstream fields would have vanished without this document. **Required:** add a compile-time-ish guard — a unit test that destructures `Issue` exhaustively:

```rust
#[test]
fn org_emission_covers_every_issue_field() {
    let Issue { id, content_hash, title, description, /* … every field … */ } = Issue::default();
    // touching each binding forces a compile error when a field is added
}
```

A new upstream field then breaks the build during the *next* sync instead of silently disappearing. This is the single highest-leverage line in this document.

### 2.4 Phase-6 rename of Org-visible identifiers

These strings are baked into every `issues.org` ever written and are therefore a **data-format compatibility** question, not a cosmetic one.

| Current | Recommended | Compatibility policy |
|---|---|---|
| `#+TITLE: Beads Issues` | `#+TITLE: Issues` | write-only; not read. Free to change. |
| `:BEADS_SCHEMA_VERSION:` | `:OBR_SCHEMA_VERSION:` | write new; **accept both on read** (both currently ignored, so the only cost is one match arm) |
| `BEADS_TODO_KEYWORDS` / `BEADS_DONE_KEYWORDS` (Rust consts) | `ORG_TODO_KEYWORDS` / `ORG_DONE_KEYWORDS` | internal only |
| — | **new** `:ORG2JSONL_REV:` or `#+OBR_PARSER:` | see below |

**Recommended addition (fixes gap-org2jsonl §6.4):** stamp the parser identity into the file — a single `#+OBR_PARSER: org2jsonl@<short-rev>` header line — and have `doctor` warn when the file's stamp differs from the binary's compiled-in rev. Today an `issues.org` written by one build and read by another silently produces different issues, and the degradation lands as a legitimate-looking git diff. One header line plus one doctor check closes it. **[V]** for the problem; **[I]** for the fix's sufficiency.

---

## 3. Per-touchpoint porting instructions against upstream

Anchors are `path:line` in `/Users/johnw/src/obr-org-forward-port` at `d1fb0d3a`. **[V]** for all anchors.

### 3.1 `Cargo.toml` — the dependency

Upstream has **no git dependencies at all**: `toon_rust` is now the crates.io package `tru` (`Cargo.toml:92`) and `rich_rust` is crates.io `0.2.2` (`:89`). **[V]** The plan's instruction to "mirror fork `Cargo.toml:54`'s `toon_rust` git-dep pattern" refers to a pattern upstream has *retired* — see §8 correction C4. The pattern is still correct Cargo; there is just no in-tree exemplar to copy.

Add under `[dependencies]` (upstream `Cargo.toml:38`):

```toml
# Org-mode parsing (sibling project; see docs/ORG_FORMAT.md)
org2jsonl = { git = "https://github.com/jwiegley/org2jsonl", rev = "59521f99a490703d4d02f9b0f312a92ec9135ba8" }
```

Then `cargo update -p org2jsonl` (or a plain `cargo build`) and commit `Cargo.lock`; gate with `cargo build --locked`. Rationale and rev choice in §5.

### 3.2 `src/config/mod.rs`

| Site | Change |
|---|---|
| `:62-64` | constants per §1.2; make `DEFAULT_JSONL_FILENAME` `pub` |
| `:147-160` `discover_jsonl` | 3-tier |
| `:4219-4261` `resolve_jsonl_path` | no logic change needed; verify the priority-2 semantics note in §1.2 |
| `:3352` | `join("issues.jsonl")` → `join(DEFAULT_JSONL_FILENAME)` |
| `:3728-3796` (`no_db` branch) | no change — it goes through `import_from_jsonl_snapshot`, fixed by §3.6 |
| `:4011-4035` `resolve_bootstrap_issue_prefix_snapshot` → `:4146-4186` `first_prefix_from_jsonl_snapshot` | **needs an Org branch**: parse the snapshot with `org_text_to_issues` (or a cheap `:ID:`-scanner) and return the first non-tombstone id's prefix. Without it, a fresh `--no-db` open of an Org workspace infers the prefix from the directory name and every subsequently created issue gets the wrong prefix. |

The fork's version of the prefix probe (`src/config/mod.rs:456-476` in `common_prefix_from_jsonl`) required *all* ids to share one prefix; upstream takes the *first* non-tombstone id. Port upstream's semantics, not the fork's. **[V]**

### 3.3 `src/sync/path.rs`

| Site | Change |
|---|---|
| `:85-94` `ALLOWED_EXTENSIONS` | add `"org"`, `"org.tmp"` (with the same "plus pid-scoped" caveat in the error text at `:138-141`) |
| `:526-558` `is_allowed_jsonl_temp_name` | generalize to accept `.org.tmp` and `.org.<pid>.tmp`; rename to `is_allowed_export_temp_name` |
| `:679-692` `validate_external_jsonl_path` | accept `.org` alongside `.jsonl` (this is the fork's `afe1351e` change to the same function) |
| `:949-979` `validate_pinned_jsonl_leaf` | **no change** — extension-agnostic **[V]** |
| `:2274-2301` `compute_snapshot_content_sha256` | **see §3.5 — critical** |
| `:2548-2550` `validate_jsonl_fd_metadata` | no change (regular-file check only) |

### 3.4 Export emission — `src/sync/mod.rs`

The fork's approach (build one `String` for the whole file, `write_all` it, hash it whole) **cannot** be ported literally: upstream streams, batches at 1,024 issues (`:54`), full-scans below 20,000 (`:55`), and prepares records on up to 64 threads (`:56-57`). **[V]**

The good news: the upstream shape is *record-oriented*, and Org emission is per-issue pure, so the seam is clean.

1. **`:9422-9428` `struct PreparedExportIssue`** — rename `jsonl_line: Vec<u8>` to `record_bytes: Vec<u8>`. Nothing else in the struct is format-specific.
2. **`:9463-9490` `prepare_export_issue_jsonl`** — take an `ExportFormat` parameter; for `Org`, produce the bytes of `emit_issue_heading` instead of `serde_json::to_writer` + `b'\n'`. Thread the parameter through `prepare_export_issue_chunk` (`:9493-9503`) and `prepare_export_issues_jsonl_parallel` (`:9506-9553`).
3. **`:9386-9418` `write_export_issue_jsonl`** — same treatment for the serial path.
4. **`:9555-9598` `write_prepared_export_entries`** — no change beyond the field rename.
5. **File header.** In `export_to_jsonl_with_policy_expected_authority`, immediately after `let mut writer = BufWriter::new(temp_file);` (`:9887`) and before the batching branch, write the Org header and feed it to `hasher` when `format == Org`. This is the *only* cross-record state in Org emission.
6. **Tombstone skipping, progress, `issue_hashes`, `report.*` counters** — unchanged; they are format-independent.
7. **`:9946-9948`** `let content_hash = hex_encode(&hasher.finalize());` — unchanged, but read §3.5 before assuming it works.

**Memory note [I]:** the fork's `issues_for_export.push(issue.clone())` (afe1351e) materialized every issue twice. Do not reproduce that; the per-record shape above never holds more than one batch.

### 3.5 The content-hash landmine — **fix this first or nothing works**

Upstream computes two digests per file: `raw_sha256` (bytes as-is) and `content_sha256` (canonical). The canonical one is:

```rust
// src/sync/mod.rs:12769-12790  and  src/sync/path.rs:2274-2301 (identical logic)
let trimmed = line.trim_ascii();
if !trimmed.is_empty() { hasher.update(trimmed); hasher.update(b"\n"); }
```

i.e. **blank lines are skipped and every line is trimmed on both sides**. **[V]**

For JSONL this is equal to a straight byte hash: no blank lines, no leading whitespace. For **Org it is not**: the format has blank lines by design (§1.3, §1.8) and `to_string_pretty` indents (§1.9).

The exporter hashes the bytes it writes and then asserts equality against the staged file's canonical hash:

```rust
// src/sync/mod.rs:9951-9957
let staged_source = pinned_temp.capture()?;
verify_exported_jsonl_snapshot_integrity(&staged_source, &exported_ids)?;
if staged_source.content_sha256() != content_hash {
    return Err(BeadsError::SyncConflict { message: "Staged JSONL bytes do not match the export content hash" });
}
```
**[V]** With an Org file this comparison fails on the very first export of any issue set. **Every `br sync --flush-only`, every auto-flush, in every Org workspace, would error.** This is the single most likely way the port "compiles and passes unit tests but is dead on arrival".

The fork never hit it because its `compute_jsonl_hash` (`/Users/johnw/src/obr/src/sync/mod.rs:2551-2569`) only strips trailing `\n`/`\r` and does not skip blank lines or trim — so whole-text hashing agreed. **[V]** Upstream tightened it; the fork's assumption silently expires.

**Required fix (recommended form):** make the canonical hash format-aware, defining Org's canonical form as *raw bytes*.

- `src/sync/path.rs:2274` `compute_snapshot_content_sha256` — take the target path (it is already in scope at the call site `:2407`); if `ExportFormat::for_path(path) == Org`, return the already-computed `raw_sha256` instead of the line-canonical digest.
- `src/sync/mod.rs:12801-12805` `compute_jsonl_hash` — same branch, so the recompute path agrees.
- `src/sync/mod.rs:12769` `compute_jsonl_hash_from_reader` — leave as the JSONL implementation; add `compute_org_hash_from_reader` (raw) and dispatch in the public wrappers.

Consequences, all in the safe direction **[I]**: Org staleness detection becomes byte-exact rather than whitespace-insensitive, so an edit that only changes indentation inside a src block is *detected* rather than ignored (the ignore case is a data-loss path — the DB would believe it was in sync and overwrite the edit on the next flush).

**Verification for the implementer:** after this change, `metadata.jsonl_content_hash` written on import (`src/sync/mod.rs:12641`) and on export (`export_result.content_hash`) must be the same function; add a unit test that round-trips an Org file with blank lines, indented JSON, and a trailing newline through both paths and asserts digest equality.

### 3.6 Import — `src/sync/mod.rs`

Upstream import is **three streaming passes over an immutable snapshot**, all funnelled through one helper:

```rust
// :12077-12095
fn for_each_jsonl_import_issue(source: &JsonlSourceSnapshot,
                               handle_issue: impl FnMut(usize, Issue) -> Result<()>) -> Result<()>
```

Call sites **[V]**: `:12106` `collect_import_validation_plan`, `:12284` `scan_import_collision_renames`, `:12434` `stream_import_actions_in_tx`, `:13008` `for_each_reconcile_classified_row`.

**This is the chokepoint. Make it format-aware and import, `--no-db` seeding, and `sync --reconcile` all become Org-capable in one edit.**

```rust
fn for_each_import_issue(source: &JsonlSourceSnapshot,
                         mut handle: impl FnMut(usize, Issue) -> Result<()>) -> Result<()> {
    match ExportFormat::for_path(source.display_path()) {
        ExportFormat::Jsonl => { /* existing line loop */ }
        ExportFormat::Org => {
            let text = read_snapshot_to_string(source)?;          // UTF-8 error → BeadsError::Config
            for (idx, mut issue) in org_bridge::org_text_to_issues(&text)?.into_iter().enumerate() {
                normalize_issue(&mut issue);
                validate_or_err(&issue, idx + 1)?;                 // same message shape as :12061-12072
                handle(idx + 1, issue)?;                           // "line_num" = heading ordinal…
            }
        }
    }
}
```

Two details that matter:

- **`parse_normalized_import_issue` (`:12056-12075`) applies `normalize_issue` and `IssueValidator::validate` per record.** The Org branch must do the same, or Org imports skip validation that JSONL imports get — a correctness asymmetry that would show up as inconsistent doctor results.
- **The `line_num` argument feeds user-facing error text** ("Prefix mismatch at line N", "Duplicate issue id … at line N"). With the pinned org2jsonl rev, real line numbers are obtainable (§5); until then, pass the heading ordinal and *say so* in the message ("heading #N") rather than lying about a line number.

Three passes means the Org text is parsed three times. At the O(n²) parse cost of §4/B3 that is a 3× multiplier on an already-bad worst case. **Recommendation:** parse once into a `Vec<Issue>` cached on the snapshot (or hoisted into `import_from_jsonl_snapshot` and passed down), and keep the closure interface. **[I]**

Also update:
- `:11744-11750` `read_issues_from_jsonl` / `:11752` `..._snapshot` — Org branch (used by merge and base-snapshot loading).
- `:8962-9010` `ensure_no_conflict_markers*` — **no change**; conflict-marker scanning is textual and works on Org (the fork's `c046b35e` migrated exactly this test to Org and it passed **[V]**).

### 3.7 Incremental auto-flush — **must be disabled for Org**

`try_incremental_auto_flush` (`:11492-11585`) is a fast path that, when the DB is dirty but not `needs_flush`, **splices individual JSONL lines** into the existing file: `read_jsonl_lines_by_id` (`:9183-9220`) builds a `BTreeMap<id, line>`, `apply_incremental_auto_flush_changes` (`:11475-11490`) replaces entries, `write_jsonl_lines_atomically` (`:11320-…`) rewrites. There is also an even faster in-place variant, `try_write_existing_jsonl_replacements_atomically` (`:11194-…`), which replaces byte ranges of existing lines. **[V]**

None of this has any meaning for Org, where one issue is a multi-line block. Running it against an Org file would either error in `read_jsonl_lines_by_id` (`serde_json::from_str` on `#+TITLE: …`) or, worse, produce a garbage file.

**Porting instruction:** at the top of `try_incremental_auto_flush` (`:11492`), immediately after the `!jsonl_path.exists()` guard:

```rust
if ExportFormat::for_path(jsonl_path) != ExportFormat::Jsonl {
    return Ok(None);   // fall through to the full re-export path
}
```

`Ok(None)` is exactly the "declined, use the full exporter" contract already used at `:11562` (`ExistingJsonlReplacementWrite::Declined`). **[V]** Do not attempt an Org "incremental" path in this port; a block-splicing equivalent is a separate project and would have to interact with `emit_issue_heading`'s blank-line accounting.

**Performance consequence [I]:** every mutating command in an Org workspace does a full re-export. At the fork's scale that is fine; at 20,000 issues it is not. Record it as a known regression against upstream's JSONL performance, and note that `tests/repro_auto_flush_inefficiency.rs` exists upstream precisely to guard this (§6, Class A).

### 3.8 Export verification, analysis, and record validation

| Function | Anchor | Org behaviour |
|---|---|---|
| `verify_exported_jsonl_snapshot_integrity` | `:9081-9172` | parse the staged Org text; assert the id **set** equals `expected_ids` and the count matches. Keep the "Export verification failed: …" error prefix so `doctor`'s string matching still works. |
| `analyze_jsonl_from_reader` | `:9022-9060` | returns `(count, ids)` and **errors on duplicate ids** — new since the fork. The Org counter must reproduce the duplicate check (the fork's `count_issues_in_org` did not, and gap-org2jsonl §5.2 confirms 100 headings sharing one `:ID:` import silently, last-one-wins **[V]**). Prefer counting `:ID:` properties on level-1 headings without a full `Issue` construction, as the fork did (`/Users/johnw/src/obr/src/sync/mod.rs:1175-1202`). |
| `validate_jsonl_issue_records_from_reader` | `:8246-8283` | Org branch: parse, `normalize_issue`, validate, collect failures with heading ordinals. Feeds `doctor`'s `jsonl.parse` check. |
| `get_issue_ids_from_jsonl[_snapshot]` | `:9173-9181` | free (delegates to `analyze_jsonl`) |
| `count_issues_in_jsonl` | `:9072-9074` | free |

### 3.9 The 3-way-merge base anchor — a silent breakage

`refresh_base_snapshot_from_flushed_jsonl_snapshot` (`:13961-13969`) does an explicit **byte copy** of the live export into `.beads/beads.base.jsonl`:

```rust
write_base_snapshot_atomically(jsonl_dir, |writer| { std::io::copy(&mut source.reader(), writer) … })
```
**[V]** — and its doc comment at `:13936-13940` states the byte-copy is deliberate.

With an Org live file this writes Org text into a `.jsonl`-named anchor. `load_base_snapshot` (`:14018-14022`) then calls `read_issues_from_jsonl_snapshot` on it. If §3.6's format dispatch keys on the *extension*, the anchor is parsed as JSONL and every merge silently gets an empty or erroring base. **[V]** for the mechanism; **[I]** for the exact symptom.

**Recommended fix (smallest surface):** for Org, replace the byte copy with `save_base_snapshot_from_jsonl_snapshot` (`:13923-13933`), which parses and re-serializes into canonical JSONL. The anchor stays machine-only, stays `.jsonl`, stays parseable, and no new filename enters `ALLOWED_EXACT_NAMES` or the doctor's `base_jsonl` checks (`:7959`, `:10202`). The property lost — "the anchor is byte-identical to the export" — is one the Org pipeline cannot honour anyway (§4/B6).

**Alternative:** write `beads.base.org` and make the loader extension-aware. More faithful, but it touches path validation, the doctor's two hardcoded `beads.base.jsonl` sites, `legacy_backup_target_path`, and the Phase-6 rename map. Not worth it.

### 3.10 Merkle witness (`src/sync/witness.rs`)

1,894 lines of **line-chunked** Merkle hashing (`build_jsonl_merkle_witness(reader, chunk_size_lines)`, `:218`), with a reuse planner for incremental re-hashing. Domain-separated under `br:jsonl-witness:*` (`:18-21`). Consumed only from `src/cli/commands/sync.rs:1994, 2045, 2086`. **[V]**

Line chunking is not *wrong* for Org — it is content-agnostic — but the reuse plan's value proposition (unchanged line ranges → reuse chunk hashes) collapses when a single issue edit rewrites a multi-line block and shifts nothing else. It still produces correct digests.

**Recommendation:** leave `witness.rs` untouched; it operates on bytes. Verify the CLI surface that exposes it (`br sync` witness flags, `src/cli/mod.rs:2915` "Reads the resolved issues.jsonl bytes and emits chunk/root hashes") does not gate on the `.jsonl` extension, and update its help text. Add one Org e2e asserting determinism of the root hash. **[I]** — I did not read the full witness CLI path.

### 3.11 Reviewed-additive reconcile

`plan_reviewed_additive_reconcile` (`:3694`) / `apply_reviewed_additive_reconcile` (`:3759`) parse with `parse_strict_additive_issue` (`:4683`) and `reject_unknown_additive_fields` (`:4565`) — semantics defined in terms of *unknown JSON object keys*. **[V]** There is no Org analogue: the Org drawer already ignores unknown properties by design (§1.7), which is the exact opposite of "strict".

**Porting instruction:** at the entry points, reject a non-`.jsonl` source with a clear, actionable error (`"reviewed-additive reconcile requires a JSONL source; pass --jsonl <path.jsonl>"`). Do not attempt a lenient Org path — a strict mode that silently isn't strict is worse than an absent feature.

### 3.12 History backups (`src/sync/history.rs`)

The fork made backups extension-aware (`afe1351e`): backup name `{stem}.{ts}.{ext}` and listing accepts `jsonl|org`. **[V]** Upstream has since added collision indices, nanosecond timestamps, and per-backup `.meta.json` sidecars. **[V]**

| Site | Change |
|---|---|
| `:171-184` `parse_backup_filename` | `strip_suffix(".jsonl")` → try `.org` then `.jsonl`; return the extension alongside `(stem, timestamp)` |
| `:192-223` `create_backup_file_for_timestamp` | take the target extension; `format!("{stem}.{timestamp}.{ext}")` and `format!("{stem}.{timestamp}.{collision_idx}.{ext}")` |
| `:227` `backup_metadata_path` | `with_extension("jsonl.meta.json")` → `with_extension(format!("{ext}.meta.json"))` — **this one is easy to miss and silently orphans every sidecar** |
| `:384` `legacy_backup_target_path` | `beads_dir.join(format!("{stem}.jsonl"))` → use the parsed extension |
| `:644-648` listing filter | accept `org` as well as `jsonl` |
| `:166-169` `BACKUP_FILENAME_REGEX` | unchanged (operates on the extension-stripped name) |

### 3.13 `doctor` (`src/cli/commands/doctor.rs`, 24,752 lines)

The fork's `a0c54428` renamed `check_jsonl` → `check_export_file` and added an Org branch. Upstream's doctor is an order of magnitude larger; the equivalent sites are:

| Anchor | Function | Change |
|---|---|---|
| `:10439-10449` | `discover_jsonl` (doctor's own copy) | 3-tier, mirroring `config::discover_jsonl` — or better, delete it and call `config::discover_jsonl` |
| `:10451-10457` | `should_fallback_to_workspace_jsonl` | compares `metadata.jsonl_export == "issues.jsonl"` and `paths.jsonl_path == beads_dir.join("issues.jsonl")` — must accept the Org default **and** the legacy JSONL default |
| `:10580-10626` | `check_jsonl` | delegates to `validate_jsonl_issue_records` (§3.8) — free once that is format-aware; update the `"Parsed {n} records"` message to say "issues" for Org, matching the fork's `"Parsed {total} issues from Org format"` and adding `"format": "org"` to the JSON details, as `a0c54428` did **[V]** |
| `:4012-4059` | `check_export_hash_cache_divergence` | uses `compute_jsonl_hash` — free once §3.5 lands |
| `:4672` | `check_jsonl_utf8_bom` | no change (byte-level; a BOM breaks Org too) |
| `:4885` | `check_jsonl_crlf_endings` | no change — and it is *more* important for Org, since `sanitize_org_text` silently normalizes CRLF (§1.10) |
| `:4939` | `check_jsonl_trailing_newline` | no change (Org emission ends with `\n`) |
| `:5143` | `check_jsonl_oversized` | no change |
| `:5201` | `check_jsonl_duplicate_ids` | **needs an Org branch** — this is the check that would have caught the `dup_ids.org` acceptance in gap-org2jsonl §5.2 |
| `:5371` | `check_jsonl_world_writable` | no change |
| `:7959`, `:10202` | base-anchor checks | hardcode `issues.jsonl`; use the resolved path |
| `:2904`, `:3234`, `:3250` | `doctor.jsonl_rebuild` fixer | rebuilds the DB from the export file — must go through the format-aware reader |
| `:3202` | error text "Fix the offending records in `.beads/issues.jsonl`" | use the resolved path in the message |

The BOM-strip / CRLF→LF / trailing-newline fixers are byte-level and safe on Org. **[I]**

### 3.14 `stats` — the fork's fix is superseded

`a0c54428` changed `compute_recent_activity` to take an explicit path and compute a repo-relative git pathspec. Upstream already does exactly that: `:644-656` takes `jsonl_path: &Path`, calls `git_repo_context` and `repo_relative_git_path`, and caches the result. **[V]**

**Only one line needs changing:** `:92`, the `map_or_else` default `|| beads_dir.join("issues.jsonl")` → `DEFAULT_JSONL_FILENAME`. The `debug!("No issues.jsonl found …")` string at `:650` should be de-hardcoded for clarity.

### 3.15 `init`

| Anchor | Change |
|---|---|
| `:171-174` | metadata template `"jsonl_export": "issues.jsonl"` → `"issues.org"` (build it with `format!` from the constant rather than a literal) |
| `:252-257` | the empty seed file `issues.jsonl` → `issues.org`. **Note:** the comment says this exists "for compatibility with bv (beads_viewer)", which reads `issues.jsonl`. Seeding an empty `issues.org` breaks that contract; the fork accepted this (`afe1351e` **[V]**). Decide explicitly: seed `issues.org` only (fork behaviour, recommended), or seed both. Do not seed a `.jsonl` and a `.org` with the same content — discovery would then always pick `.org` and the `.jsonl` would rot. |
| `:360` | step label `"issues.jsonl (for bv compatibility)"` → `"issues.org"` |
| `:406` | the ASCII tree art |
| `:464`, `:638` | the two unit tests |

An **empty** `issues.org` is a valid Org file that parses to zero issues **[I]** — the fork wrote `""`, not the two header lines. Keep writing `""`: writing the header would make a pristine `init` produce a file whose content hash differs from an empty export.

### 3.16 `sync` CLI path policy

`src/cli/commands/sync.rs:1548-1557` **[V]**:

```rust
if extension.as_deref() != Some("jsonl") {
    return Err(BeadsError::Config(format!("JSONL path must end with .jsonl: {}", …)));
}
```
→ `if !matches!(extension.as_deref(), Some("jsonl" | "org"))` with the message `"Sync path must end with .jsonl or .org"` — verbatim the fork's `afe1351e` change to the same function. **[V]**

`:1571` already calls `export_temp_path(&jsonl_path)`, so fixing §3.4/item 6 covers the temp path here.

### 3.17 `vcs` diagnostics

`src/cli/commands/vcs.rs:259-263` **[V]**:

```rust
if anchored.extension() != Some(OsStr::new("jsonl")) {
    return Err(unsafe_target_error("the diagnostic target must have a .jsonl extension"));
}
```

This command did not exist at the fork point. Left alone, `br vcs` fails on every Org workspace because the *default resolved path* (`:239-240`) is now `.org`. Accept both extensions.

### 3.18 `--no-db`

Seeding: `src/config/mod.rs:3745-3771` calls `import_from_jsonl_snapshot` on the captured source — free once §3.6 lands. **[V]**
Flushing: `flush_no_db_if_dirty` (`:3324-3389`) calls `export_to_jsonl_with_policy_expected_under_authority` — free once §3.4 and §3.5 land; fix the hardcoded `is_default_path` at `:3352`.
Prefix inference: **not** free — see §3.2's last row.

Per plan §6.6 this is the user's preferred lightweight mode, so give it a dedicated e2e: `--no-db list` on a hand-written `issues.org`, then `--no-db create` and assert the file grows by exactly one heading block and no other heading changes byte-wise. That last clause is the direct regression test for the §3.7 phantom-change mechanism.

### 3.19 Preflight

`preflight_export` (`:8321`) and `preflight_import` (`:8548`, impl `:8566`) run path validation, existence and record checks. They consume the same snapshot/validation helpers, so they follow §3.6/§3.8 automatically; audit their user-facing strings for "JSONL". **[I]** — I read the first ~80 lines of `preflight_import_impl` only.

### 3.20 Display-only surfaces

`src/cli/commands/info.rs`, `where.rs`, `src/mcp/{mod,tools,resources}.rs`, `src/cli/mod.rs:962,2865,2915`, `src/health.rs` reference `issues.jsonl` in help text, JSON output keys, and MCP resource descriptions. **[V]** None are load-bearing for the format, but all are user-visible and all are also Phase-6 rename targets. Sweep them once, in Phase 6, not twice.

---

## 4. Known bugs: do not worsen; fix where cheap

| ID | Bug | Evidence | Reachable upstream? | Cheap to fix in the port? |
|---|---|---|---|---|
| **B1 (R9)** | A label containing `:` becomes two labels on round-trip | fork `org_bridge.rs:63-74` emits labels raw into the Org tag list; dossier §R9 reproduced it **[V]** | **Yes, and worse.** `LabelValidator` explicitly permits `:` (upstream `src/validation/mod.rs:392-400`) and `provides:<cap>` is the *wire format* of the capability-dependency feature (`src/storage/sqlite.rs:16187` builds it, `:16214` strips it) **[V]** | **Yes — fix it. ~25 lines.** See below. |
| **B2 (R31)** | One heading without `:ID:` aborts the whole parse, bricking every command in the workspace, with no line number | `org_bridge.rs:742-748` errors; `:347` propagates with `?`; gap §5.4 shows `obr show <healthy-id>` failing **[V]** | Yes — unchanged by upstream | **Partly, cheaply.** Line numbers become available via §5's org2jsonl addition. Per-heading recovery is a design decision (§9, Q3). |
| **B3** | O(n²) parse: 100 KB of `[[` costs 12.7 s per invocation; ~20 min for 1 MB | gap §5.3, measured **[V]**; the cost is inside `orgize`'s link scanning | Yes | **No** — it is in the third-party alpha parser. **Mitigate**: cap `issues.org` size before parsing (upstream already has `check_jsonl_oversized`, `doctor.rs:5143`), and avoid the 3× multiplier from re-parsing (§3.6). |
| **B4** | `sanitize_org_text` / `unsanitize_org_text` are not inverses: a user's literal `,*` loses its comma, one way, forever | `org_bridge.rs:248-259` vs `:569-580`; gap §3.4 **[V]** | Yes | **Yes, trivially.** Escape `,`-prefixed lines symmetrically: on write, prefix a comma to any line starting with `*` **or** `,`; on read, strip exactly one leading comma from any line starting `,*` **or** `,,`. Add a proptest. |
| **B5** | `#+SEQ_TODO:` in the file is ignored; a foreign/hand-written keyword is absorbed into the title and the status silently becomes `Open`. `Status::Custom` round-trips the same way | gap §5.5 **[V]** | Yes; upstream still admits `Status::Custom` | **Half.** Refusing to *export* `Custom` statuses is ~5 lines (§1.5) and closes the round-trip hole. Honouring an in-file `#+SEQ_TODO:` is larger and should be deferred. |
| **B6 (R23)** | Round-trip is not a fixpoint: 9.4 % of descriptions lose content on pass 1, 1.06 % change again on pass 2; convergence at generation 3 | gap §3.6, 30,000-sample measurement **[V]** | Yes | **Yes, if §1.13 option (a) is adopted** (raw-slice fallback + drop the `i > 0` separator artifact). That is the single change that most reduces this. |
| **B7** | `Entity` inline gains braces: `\alpha` → `\alpha{}` — content *growth* | `org_bridge.rs:519-523`; gap §2.4 **[V]** | Yes | **Yes, cheaply**, if the model carries the information — it does not (`org2jsonl/src/model.rs` `Entity { name }` only). Subsumed by §1.13 option (a). |
| **B8** | Title ending in `:foo:` is truncated and injects a phantom label | gap §3.5 **[V]** | Yes | **Yes, cheaply**: when a title's trailing text would parse as a tag list, append a zero-width-safe guard — simplest is to always emit at least one space after the title *and* refuse to emit a title matching `:\w+(:\w+)*:$` without escaping. Needs a round-trip proptest to confirm. |
| **B9** | Duplicate `:ID:` headings import silently, last one wins | gap §5.2 **[V]** | **Worse upstream**: JSONL now *errors* on duplicate ids (`:9036-9043`, `:11784-11791`, `:12119-12126`) **[V]** | **Yes — required for parity.** §3.8. |
| **B10** | Deps/comments serialization failure writes `"[]"`, silently discarding all relations | `org_bridge.rs:216-217, 228-229` **[V]** | Yes | **Yes, trivially** — propagate the error. |
| **B11** | Zero tests parse an Org file: `grep -rn 'org_text_to_issues\|issues_to_org_text' tests/` returns nothing | gap §7 **[V]** | n/a | **Yes** — §7 defines the gate. |

**B1 (R9) recommended fix.** Emit a canonical `:LABELS:` drawer property carrying the exact label vector as compact JSON, and *additionally* emit tag-safe labels as Org tags for human/agenda ergonomics. On read: if `:LABELS:` is present it is authoritative; otherwise fall back to `heading.tags` (so files written before the fix still import). Cost: ~25 lines, one drawer property, two match arms. Benefit: colon labels round-trip, the capability feature keeps working, and the emission stays human-legible.

A secondary hazard worth recording: **Emacs Org's tag regexp is `[[:alnum:]_@#%]+` — hyphen is not a legal tag character**, yet hyphenated labels (`bug-fix`, `needs-review`) are the norm and `LabelValidator` allows `-`. **[I]** — I did not test what `orgize` accepts; the fork's files evidently round-trip through `orgize`, but real Emacs will not render such tags as tags. The `:LABELS:` property makes this a display nit rather than a data problem, which is another argument for it.

---

## 5. `org2jsonl` pinning

### 5.1 Facts

| Fact | Value | Evidence |
|---|---|---|
| Fork `Cargo.toml:64` | `org2jsonl = { path = "../org2jsonl" }` — no version, no source, no checksum | **[V]** |
| Fork `flake.nix:21-23` | `url = "git+file:///Users/johnw/src/org2jsonl"`, locked to rev `5ea7586`, `revCount 15` | **[V]** (`flake.lock`) |
| Worktree HEAD | `59521f99a490703d4d02f9b0f312a92ec9135ba8`, 2026-03-18, **clean tree** | **[V]** `git -C /Users/johnw/src/org2jsonl status --porcelain` → empty |
| Delta `5ea7586..59521f9` | 15 commits, 25 files, +1,524 / −240 | **[V]** |
| Is HEAD pushed? | **Yes** — `refs/heads/main` on `origin` = `59521f9…` | **[V]** `git ls-remote` |
| Is the repo public? | **Yes** — `GET api.github.com/repos/jwiegley/org2jsonl` → `200`, `"private": false`; anonymous `git ls-remote https://…` succeeds | **[V]** |
| License at `5ea7586` | MIT | **[V]** |
| License at `59521f9` | **BSD-3-Clause** (commit `3cde81c`) | **[V]** |

### 5.2 What changed in those 15 commits

Grouped from `git log --oneline 5ea7586..59521f9` and `git diff --stat`: **[V]**

- **Infrastructure, no runtime effect on obr:** `d804c86` clippy/rustfmt, `3cde81c` BSD-3 license, `5a31542` criterion benches + cargo-fuzz targets, `ec0f85a` flake test/doc checks, `fccdc6b` lefthook, `01af2fc` CI + Makefile, `bf44e27` README.
- **New dependency:** `b8fc44d` adds `walkdir = "2"` (`Cargo.toml:15`). **This is the commit that stales obr's `Cargo.lock`** (gap §6.2) — the lock records org2jsonl's deps as `[anyhow, clap, orgize, serde, serde_json]` with no `walkdir`, so `cargo build --locked` fails today. **[V]**
- **Additive API, obr-relevant:** `9b91599` adds `file`/`char_begin`/`char_end`/`line_begin`/`line_end` to `OrgEntry`, all `Option` + `skip_serializing_if` (`src/model.rs:10-25`); `bb99c7b` adds `pub fn org_to_entries_with_source(input, file)`.
- **Multi-file CLI features:** `8401de3`, `8fefe2d`, `360b573`, `7202b96`, `59521f9` — all in the binaries and `json_to_org`, which obr never calls.

**Critical compatibility check [V]:** `pub fn org_to_entries_with_keywords(input: &str, todo_keywords: &[&str], done_keywords: &[&str]) -> Vec<OrgEntry>` is **byte-for-byte the same signature** at `5ea7586:src/org_to_json.rs:40` and at `59521f9:src/org_to_json.rs:61`. The body changed only in that it now calls `org_to_entries_from_parsed(org, input, None)` instead of `org_to_entries_from_parsed(org)` — i.e. it passes `file: None`, which leaves the new location fields `None`. `orgize` is pinned to the same `0.10.0-alpha.10` on both revs. The `Element`/`InlineContent`/`Heading`/`Property` types are unchanged apart from two rustfmt-only reflows. **[V]**

**Verdict: `5ea7586` → `59521f9` is a no-behaviour-change upgrade for obr's five-symbol surface.**

### 5.3 Recommendation

**Pin `59521f9` in both `Cargo.toml` and `flake.nix`, by git URL + rev.**

```toml
# Cargo.toml, under [dependencies]
org2jsonl = { git = "https://github.com/jwiegley/org2jsonl", rev = "59521f99a490703d4d02f9b0f312a92ec9135ba8" }
```

Why `59521f9` rather than `5ea7586`:
1. It is what `cargo build` has actually been compiling in the fork all along (the path dep points at the worktree), so it is the *tested* parser; `5ea7586` is only what `nix build` produced. Pinning the newer rev collapses the two-different-parsers-both-called-0.1.0 divergence in the direction of the one people have used. **[V]** gap §6.3.
2. It is public and reachable anonymously, which fixes CI, `cargo audit` visibility, and third-party builds in one move. **[V]**
3. Its license is BSD-3-Clause — consistent with the fork's own relicense and one fewer complication for the §8 licence decision. **[V]**
4. It carries the location metadata that makes B2's "no line number" fixable. **[V]**

Use the **full 40-hex rev**, not the short form, so it is unambiguous and `cargo` records it exactly. (The fork's `toon_rust` used a 7-char rev; that is legal but weaker.)

**Nix side.** The upstream flake uses `crane` + `fenix` (`flake.nix:23-27,40,60`) and has **no `outputHashes`** anywhere — crane vendors git dependencies from `Cargo.lock` without needing them. **[V]** So unlike the fork's `rustPlatform.buildRustPackage`, no `outputHashes` entry is required. What *is* required, if the fork's flake wins the R3 disposition (`rustPlatform.buildRustPackage`, fork `flake.nix:65-67`), is one `outputHashes` line:

```nix
outputHashes = {
  "toon_rust-0.1.1" = "sha256-…";                 # only if the git toon dep is kept
  "org2jsonl-0.1.0" = "sha256-<fill from the first build failure>";
};
```

Either way, **delete the fork's `org2jsonl` flake input and the `postUnpack: cp -r ${inputs.org2jsonl} org2jsonl` hack** (fork `flake.nix:21-23, 59-60, 102-103` **[V]**). It exists solely to satisfy the relative path dep, which the git dep removes. Note that upstream's flake has an analogous vestigial `toon_rust` flake input plus a combined-source-tree `postUnpack` (`flake.nix:32-35, 69-85`) that its own `Cargo.toml` no longer needs, since `toon_rust` is the crates.io package `tru`. **[V]** That is R3's problem, but the same cleanup pattern applies.

**Also required regardless:** regenerate and commit `Cargo.lock`; gate on `cargo build --locked` (plan §7.6). Today that command fails in the fork. **[V]**

### 5.4 One recommended upstream-crate addition

To fix B2's missing line numbers, `org2jsonl` needs one ~8-line public function:

```rust
pub fn org_to_entries_with_keywords_and_source(
    input: &str, todo_keywords: &[&str], done_keywords: &[&str], file: Option<&str>,
) -> Vec<OrgEntry> { /* same as _with_keywords but passes `file` through */ }
```

`org_to_entries_from_parsed(org, input, file)` is already private-but-parameterized for exactly this (`59521f9:src/org_to_json.rs:76`), and location fields are only populated when `file.is_some()` (`:~208-224` region). **[V]** With it, `parse_heading_to_issue` can report `line_begin` in its `Validation` error, and per-heading recovery becomes implementable. This is a change to a repo the user owns; make it a separate commit there, push, and pin obr to the resulting rev.

---

## 6. e2e test policy — a rule the implementer can apply file by file

**Scale [V]:** 143 test targets under `tests/`; 59 files mention `issues.jsonl`; heaviest are `conformance.rs` (65 hits), `e2e_basic_lifecycle.rs` (40), `jsonl_import_export.rs` (22), `e2e_sync_failure_injection.rs` (22), `e2e_history.rs` (22). The fork migrated **5** files. The shared harness (`tests/common/cli.rs`) has **no** export-path helper. **[V]**

### 6.1 Prerequisite: two harness helpers

Add to `tests/common/cli.rs` (next to `BrWorkspace`, `:95`):

```rust
/// The workspace's default export artifact (Org, post-port).
pub fn export_path(ws: &BrWorkspace) -> PathBuf { ws.root.join(".beads").join("issues.org") }

/// Pin this workspace to the JSONL export format. Call immediately after init,
/// before the first mutating command. Returns the pinned path.
pub fn pin_jsonl(ws: &BrWorkspace) -> PathBuf { /* rewrite .beads/metadata.json jsonl_export */ }
```

`pin_jsonl` works because `resolve_jsonl_path` priority 2 treats a non-default `metadata.jsonl_export` as an explicit override (§1.2). **[V]** This is the whole mechanism; no new CLI flag is needed.

### 6.2 The rule

> **Default is Org. JSONL requires an explicit, one-line pin.**
>
> For each test **function** (not file — several files mix both kinds):
>
> **Class A — pin to JSONL** (`pin_jsonl(&ws)` as the line after workspace setup; leave every assertion untouched) if **any** of:
> - the test writes raw JSON into the export path, or reads it and calls `serde_json::from_str` / `serde_json::from_value` / `.lines()` on its content;
> - the test asserts a *line* property (line count, "line N", one-record-per-line);
> - the test exercises machinery that is JSONL-only by construction: reviewed-additive reconcile (§3.11), incremental line auto-flush (§3.7), Merkle witness chunk reuse (§3.10), `beads.base.jsonl` anchors (§3.9), `br vcs` before §3.17 lands;
> - the test is a **conformance** test (compares against the Go `bd` binary — `bd` has no Org format, and plan §7.3 puts these out of scope anyway);
> - the test is a **benchmark/dataset** test whose numbers must stay comparable to upstream's.
>
> **Class B — convert to Org** (swap the literal path for `export_path(&ws)`; translate content assertions) otherwise. Class B is everything that treats the export as *the durable artifact* rather than *a JSONL file*: existence, size, mtime/hash staleness, backup/restore/prune, conflict-marker rejection, path-traversal and git-safety allowlists, doctor counts, determinism of repeated export, import round-trip of issue **fields**.
>
> **Class C — duplicate** when a single test asserts both a format-agnostic behaviour and a JSON-shaped payload: split into `..._org()` and `..._jsonl()` (the latter calling `pin_jsonl`). Prefer this over weakening an assertion.

**Mechanical triage command** (run per file, then read the hits):

```
rg -n 'issues\.jsonl' tests/<file>.rs
rg -n 'from_str|from_value|\.lines\(\)|line \{|"line"' tests/<file>.rs
```

If the second grep hits *within* a block that reads the export file → Class A or C. If it does not → Class B.

**Anti-pattern to avoid — the fork did this and it is why `c046b35e` proves nothing.** The fork's migration frequently wrote `if org_path.exists() { …weak assertion… } else { …original JSONL assertion… }` (e.g. `tests/e2e_labels.rs` in `c046b35e`: the Org branch degenerates to `org_content.contains(":persisted:")`, a substring check that cannot distinguish a label from a title). **[V]** A branch on `exists()` means the test asserts whichever thing happens to be true, and the Org assertion is strictly weaker than the JSONL one it replaced. **Never branch on file existence in a test.** Pin the format, then assert unconditionally.

### 6.3 Pre-classified inventory (starting point, verify each)

| File(s) | Class | Why |
|---|---|---|
| `conformance.rs`, `conformance_{edge_cases,labels_comments,schema,text_output,workflows}.rs` | **A** | Go-`bd` parity; also out of scope per plan §7.3 |
| `jsonl_import_export.rs` | **A** | the JSONL path *is* the subject |
| `storage_export_atomic.rs` | **C** | atomicity is format-agnostic; the content assertions are JSONL |
| `repro_auto_flush_inefficiency.rs` | **A** | guards the line-splice fast path, which Org disables |
| `e2e_sync_reconcile.rs`, `e2e_sync_preflight_integration.rs` | **A** | additive/strict semantics |
| `e2e_vcs_status.rs` | **A** until §3.17 lands, then **C** | |
| `bench_*.rs`, `benchmark_*.rs`, `tests/common/dataset_registry.rs` | **A** | comparability |
| `e2e_basic_lifecycle.rs`, `e2e_comments.rs`, `e2e_labels.rs`, `e2e_relations.rs` | **B** | field round-trip through the default format — exactly what must work |
| `e2e_history.rs`, `e2e_history_restore_prune.rs`, `e2e_history_custom_path.rs`, `storage_history.rs`, `repro_history_collision.rs` | **B** | backup naming/listing is the §3.12 change under test |
| `e2e_sync_artifacts.rs`, `e2e_sync_fuzz_edge_cases.rs`, `e2e_sync_failure_injection.rs` | **B** | the fork already migrated these; port the *intent*, not the weakened assertions |
| `e2e_sync_git_safety.rs`, `proptest_sync_path.rs`, `repro_create_path_traversal_check.rs` | **B** | allowlists must now admit `.org` — plan §7.5 flags exactly this class |
| `e2e_errors.rs`, `e2e_global_flags.rs`, `e2e_env_overrides.rs`, `e2e_workspace_commands.rs`, `e2e_routing.rs`, `e2e_orphans.rs`, `e2e_ready.rs`, `e2e_stats.rs` | **B** | |
| `golden_beads_init.rs`, `snapshots.rs`, `tests/snapshots/*` | **B** | goldens must be regenerated for `issues.org`; review the diff by hand |
| `e2e_sync_status_health.rs`, `e2e_sync_needs_flush_guard.rs` | **B** | staleness/hash behaviour — the §3.5 change is directly under test here |

### 6.4 New Org-only tests the port must add

1. `tests/e2e_org_roundtrip.rs` — the §7 gate, as a real test.
2. Unit proptest in `org_bridge.rs`: `issues → issues_to_org_text → org_text_to_issues == issues` over a generator covering colon labels, unicode, `,*` bodies, empty strings, all statuses including `Draft`, all priorities, deps, comments, and `agent_context`. This is the ~10-line test dossier R9 says would have caught R9. **[V]**
3. `Status::Custom` export refusal (§1.5).
4. Duplicate-`:ID:` rejection parity with JSONL (§3.8 / B9).
5. Content-hash agreement between export-side and file-side for an Org file with blank lines and indented JSON (§3.5).
6. `--no-db` create-one-issue-changes-one-block (§3.18).
7. Field-coverage destructuring test (§2.3).

---

## 7. `G-org-roundtrip` — the gate, made concrete

Plan §7.8 defines it loosely. Concretely:

**Setup.** Throwaway dir; `obr init`; create ≥ 12 issues covering: every `Status` (including `Draft`), every `Priority` 0–4, a colon label (`provides:auth`) and a hyphen label, unicode + emoji + ZWJ in title and description, a description containing `,*`, a `*`-leading line, a list, a table, and a `#+begin_src` block, a multi-paragraph description, `agent_context` JSON, ≥ 3 dependencies of ≥ 2 types, ≥ 5 comments, an `external_ref`, a tombstone, and one issue with only the required fields.

**Cycle.** `obr sync --flush-only` → capture `issues.org` as G1. Copy G1 into a *fresh* workspace, `obr sync --import-only`, `obr sync --flush-only` → G2. Repeat → G3.

**Assertions.**
1. G1 parses; `obr doctor` reports `jsonl.parse: OK` with the right count.
2. **Field-level equality** of `obr list --json` / `obr show --json` between the source DB and the re-imported DB, for every field, with an explicit allow-list of known-lossy fields — and that allow-list must be *empty* for: id, title, status, priority, issue_type, all timestamps, assignee, owner, all `source_*`, all tombstone fields, all compaction fields, `labels` (post-B1 fix), `dependencies`, `comments`, `agent_context`.
3. **`G1 == G2` byte-for-byte.** This is the fixpoint property the fork does *not* have (B6). If §1.13 option (a) is adopted, it should hold; if option (c) is chosen, downgrade to `G2 == G3` and record the one-cycle churn as an accepted defect **with a named issue**.
4. Repeated `sync --flush-only` in the same workspace is byte-identical (determinism, §1.11).
5. `metadata.jsonl_content_hash` after export equals `compute_jsonl_hash(issues.org)` (§3.5).
6. A JSONL-pinned workspace with the same issues produces a `.jsonl` whose re-import is field-identical (the JSONL path must not regress).

---

## 8. Corrections to the plan and the prior research

- **C1 — the status map in plan §6.3 is wrong.** The plan's example reads "open→TODO, in_progress→STRT/NEXT, closed→DONE, deferred→WAIT". The actual map (`03c5cd2c`) is **`InProgress→DOING`** (never STRT or NEXT) and **`Deferred→DEFER`**; `WAIT` is **`Blocked`**, not `Deferred`. The plan told me to read the diff rather than trust the line, and it was right to. The full table is §1.5. **[V]**
- **C2 — plan §6.2 lists the drawer as "including `:ID:`, timestamps as RFC3339, `:BEADS_SCHEMA_VERSION:`". It is 29 properties, enumerated in §1.7**, three of which have non-obvious guards (`COMPACTION_LEVEL` only when `> 0`; `EPHEMERAL`/`PINNED`/`IS_TEMPLATE` only when true) and one of which (`ISSUE_TYPE`) is not sanitized. **[V]**
- **C3 — plan §6.5's "doctor/stats must understand the Org file" understates doctor and overstates stats.** Upstream's `stats` needs **one line** changed (`:92`) because `compute_recent_activity` already takes a path and computes a repo-relative pathspec — the fork's `a0c54428` fix is superseded. Upstream's `doctor` needs ~12 sites (§3.13). **[V]**
- **C4 — plan §7.1 and §5-Phase-3 say to "mirror how the fork pins `toon_rust`" — but upstream has retired that pattern.** Upstream `Cargo.toml` has **no git dependencies**: `toon_rust` is the crates.io package `tru` `0.2.3` (`:92`) and `rich_rust` is `0.2.2` (`:89`). There is no in-tree exemplar to copy, and upstream's `crane`-based flake needs **no `outputHashes`** (unlike the fork's `rustPlatform.buildRustPackage`, which does). The advice is still correct in substance; the reference is stale. **[V]**
- **C5 — the plan's Phase-3 commit list omits four upstream subsystems that did not exist at the fork point and that the Org port must handle**: the line-splice incremental auto-flush (§3.7 — *breaks loudly*), the canonical content-hash normalizer (§3.5 — *breaks loudly*), the byte-copy base-merge anchor (§3.9 — *breaks quietly*), and the reviewed-additive reconcile (§3.11). Each needs its own step or its own line in a step's done-criteria. **[V]**
- **C6 — plan §3 table says "Upstream does NOT have `src/sync/org_bridge.rs`" — correct — but the corollary matters: upstream also has no format-abstraction seam of any kind.** There is no `trait ExportFormat`, no `enum Format`, no dispatch point. Every one of the ~25 touchpoints in the summary table is a fresh `if`. Budget accordingly; the fork's 4-probe version is not a guide to the size.
- **C7 — plan §6.7's "upstream's e2e suite is much larger" is true but the number is worth having: 143 test targets, 59 touching `issues.jsonl`,** versus the fork's 5-file migration. And the fork's migration pattern (branch on `org_path.exists()`) produces assertions strictly weaker than the ones it replaced — do not copy it (§6.2). **[V]**
- **C8 — `gap-org2jsonl.md` §6.3 says the flake pin is "15 commits behind the worktree"; that is exact and still true, and the worktree is clean.** But it does not mention that `59521f9` **is pushed to a public GitHub remote** — which is what makes the recommended fix a one-line change rather than a publish-first project. **[V]**
- **C9 — `map-sync.md` §1.3's claim that the export-side and file-side hashes "agree for Org because org export is `\r`-free" was true of the fork and is false of upstream.** Upstream's canonical hash also skips blank lines and trims leading whitespace (§3.5). The Org file has both. **[V]**
- **C10 — dossier R9's fix suggestion ("escape or reject colons in the Org tag emission") is incomplete.** Rejecting colons would break the `provides:<cap>` capability wire format, which upstream still builds and parses (`src/storage/sqlite.rs:16187,16214`). The fix must be *lossless*, not a rejection — hence the `:LABELS:` property in §4/B1. **[V]**

---

## 9. Open questions and user decisions

**User decisions (do not guess — plan §8 discipline):**

- **U1 — `agent_context` in Org: carry (as a `** Agent Context` src block, §2.2) or explicitly reject?** Carrying costs ~30 lines and one more child-section title in the namespace; rejecting means Org workspaces cannot use inherited context at all. Recommend: carry.
- **U2 — Description fidelity: §1.13 option (a) preserve-by-raw-slice, (b) explicit reject, or (c) documented lossiness?** This is *the* decision that determines whether R23/B6 is fixed or inherited. It changes the round-trip gate (§7, assertion 3) and it is the difference between "Org is a durable format" and "Org is a lossy view". Recommend: (a).
- **U3 — Fix R9/B1 in this port, or defer?** It is ~25 lines and it currently breaks a shipped upstream feature (capability dependencies). Recommend: fix, in the same commit as the emitter.
- **U4 — `init` seeding: `issues.org` only (fork behaviour, breaks the `bv` viewer contract), or both files?** §3.15. Recommend: `issues.org` only, and say so in the README.
- **U5 — Property-key rename timing.** `:BEADS_SCHEMA_VERSION:` → `:OBR_SCHEMA_VERSION:` is a *file-format* change. Do it in Phase 3 (one format, one migration) or Phase 6 (with the rest of the rename, requiring every Org file written in between to be re-read under a dual-key reader)? Recommend: Phase 3, since nothing reads the key today and the accept-both reader is two match arms either way.

**Open questions I could not close read-only:**

- **Q1 — does `orgize 0.10.0-alpha.10` accept `-` in tags?** The fork's hyphenated labels apparently survive, but Emacs' `org-tag-re` excludes `-`. Needs a 5-line test against the pinned rev. Affects whether `:LABELS:` is a nicety or a necessity. **[I]**
- **Q2 — what exactly does the `br sync` witness CLI surface gate on?** I confirmed `witness.rs` itself is byte-oriented and format-agnostic, but read only the three call sites in `src/cli/commands/sync.rs` (`:1994, 2045, 2086`), not the surrounding flag handling or `src/cli/mod.rs:2915`'s contract.
- **Q3 — per-heading import recovery (B2): error-and-abort, or skip-with-warning?** Abort is the current behaviour and is safest for a sync format (a partial import that then flushes would delete the skipped issues). Skip-with-warning makes hand-editing survivable. A third option — abort, but with a line number and the offending heading title, now that §5.4 makes that possible — may be sufficient. Needs a call from whoever owns the human-editing workflow.
- **Q4 — how large is the doctor fixture suite's exposure?** `tests/doctor_fixtures/` and `tests/e2e_doctor_fixture_suite.rs` exist; I did not enumerate whether the fixtures embed `issues.jsonl` filenames or JSONL content. Cheap to check: `rg -l 'issues\.jsonl|\{"id"' tests/doctor_fixtures/`.
- **Q5 — does anything in upstream write to the export file outside the paths audited here?** I traced export, auto-flush (both variants), no-db flush, reconcile apply, base-snapshot refresh, history backup, and the doctor fixers. A `rg -n 'durable_rename|write_all|fs::write' src/ | rg -i jsonl` sweep before the port declares done would close it.
