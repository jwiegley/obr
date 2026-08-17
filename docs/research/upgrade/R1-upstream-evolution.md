# R1 — Upstream Evolution Recon

**Brief:** Phase 1 (R1) of `UPSTREAM_SYNC_AND_RENAME_PLAN.md` §5.
**Date:** 2026-08-06. **Author:** read-only recon subagent.
**Upstream under study:** worktree `/Users/johnw/src/obr-org-forward-port` @ `upstream/main` =
`d1fb0d3ac48c34156bda5ddd78052c20c5138a5c`, "release: bump version to 0.2.22", 2026-08-06.
**Fork baseline:** `/Users/johnw/src/obr` @ `main`; merge-base `e805fe98` (2026-02-15).

**Evidence convention.** Bare `path:line` = the upstream worktree. `fork:path:line` = the fork's
working tree. `e805fe98:path` = the fork point via git. Every claim is tagged **[V]** (verified —
I read, grepped, or ran it myself) or **[I]** (inferred from what I read). No claim here rests on
the dossier or the plan alone.

**Nothing was modified.** No writes outside this file; no `cargo` invocation anywhere.

---

## 0. Summary table — what the Org forward-port must know

| # | Finding | Impact on the Org port | Evidence |
|---|---|---|---|
| 1 | **Upstream has no format abstraction at all.** No `Format` enum, no trait, no extension dispatch, zero occurrences of `org`/`orgize`/`org2jsonl` anywhere in `src/`. JSONL is hard-wired end to end. | The fork's extension-based dispatch has no upstream seam to slot into; it must be *created*, and the crate is 4× larger than at the fork point. | `grep -rn "org_bridge\|is_org_format\|orgize\|org2jsonl" src/` → 0 hits; `grep -rn "enum [A-Za-z]*Format" src/` → only `OutputFormat`, `OutputFormatBasic`, `GitObjectFormat` **[V]** |
| 2 | `export_to_jsonl_with_policy` still exists **and is still the name**, but is now a 3-deep façade over `export_to_jsonl_with_policy_expected_authority`. | Direct analogue found; the real work moved. | `src/sync/mod.rs:9642` → `:9650` → `:9704` **[V]** |
| 3 | `import_from_jsonl` still exists **and is still the name**, but now delegates to `import_from_jsonl_snapshot`, which streams the file **three times** through a pinned-fd `JsonlSourceSnapshot`. | Org parsing is whole-file (orgize), not streaming; the three streaming passes each need an Org path or a cached parse. | `:12539` → `:12561`; passes at `:12580` (`collect_import_validation_plan`), `:12596` (`scan_import_collision_renames`), `:12627` (`stream_import_actions_in_tx`) **[V]** |
| 4 | Per-issue emission (`emit_issue_heading`'s analogue) has **fanned out to four sites**: `write_export_issue_jsonl` (serial), `prepare_export_issue_jsonl` (parallel worker), `export_to_writer_with_policy_and_retention_at` (stdout), and `collect_incremental_auto_flush_changes` (per-line incremental flush). | Four hook points, not one. Missing any one silently writes JSONL into an `.org` file. | `:9386`, `:9463`, `:10150`, `:11377` **[V]** |
| 5 | **A whole line-oriented *incremental* auto-flush path is new** and is the default for ordinary mutations: it rewrites only the changed lines of the existing file in place. | Org is not line-per-issue. This path is structurally incompatible and must be either Org-aware or bypassed when the target is `.org`. | `try_incremental_auto_flush` `:11492`, called from `auto_flush` `:11678`; `scan_existing_jsonl_replacements` `:11073`; `write_existing_jsonl_replacements_atomically` `:11225` **[V]** |
| 6 | **The sync allowlist is real, central, and JSONL-only** — `ALLOWED_EXTENSIONS` has no `org`. A second, *private, hand-copied* allowlist lives in the git-safety test. | This is landmine §7.5, confirmed. Both copies need `org`/`org.tmp`, plus `is_allowed_jsonl_temp_name` and `validate_external_jsonl_path`. | `src/sync/path.rs:85-94`, `:512-544`, `:546-559`, `:679-690`; `tests/e2e_sync_git_safety.rs:1274-1283` **[V]** |
| 7 | The export temp filename is computed as `output_path.with_extension("jsonl.<pid>.tmp")` — it *replaces* the extension, so an `issues.org` target yields `issues.jsonl.<pid>.tmp`. | Silent wrong-name behavior; needs the fork's `is_org_format` branch re-created here. | `export_temp_path_for_attempt` `:1818-1829` **[V]** |
| 8 | `DEFAULT_JSONL_FILENAME = "issues.jsonl"` (private `const`, not `pub`), legacy fallback `beads.jsonl`. Discovery is 5-priority: `BEADS_JSONL` env → non-default `metadata.json` → `--db` sibling → file discovery → default. | Fork changes it to `"issues.org"` and makes it `pub`. Fork's 3-step fallback chain (`issues.org` → `issues.jsonl` → `beads.jsonl`) grafts onto `discover_jsonl` cleanly. | `src/config/mod.rs:62`, `:64`, `:147-162`, `resolve_jsonl_path` `:4219-4262`; `fork:src/config/mod.rs:33`, `:107-125` **[V]** |
| 9 | **The `Issue` model gained exactly two fields since the fork point:** `source_repo_path: Option<String>` and `agent_context: Option<String>`. Everything else is byte-identical. | R2's port surface is far smaller than feared. Two new `:PROPERTIES:` keys. | field-list diff `e805fe98:src/model/mod.rs` vs `src/model/mod.rs:460-624` **[V]** |
| 10 | **`Status` gained one variant: `Draft`.** | The fork's `status_to_keyword`/`keyword_to_status` map (commit `03c5cd2c`) is now *incomplete* — `Draft` would fall through to `Status::Custom` on round-trip. | `src/model/mod.rs:64`, `:91`, `:106`, `:127`; fork map in `git show 03c5cd2c` **[V]** |
| 11 | **JSONL wire format did NOT evolve.** Still exactly one bare `Issue` JSON object per line. No header line, no `_type` records, no schema-version marker, no envelope. | The Org representation does not have to model any new record kind. | `read_issues_from_jsonl_reader` `:11757-11795`; `write_export_issue_jsonl` `:9386-9418`; `parse_normalized_import_issue` `:12056` **[V]** |
| 12 | **Dossier R1 (concurrent flush loss), R2 (`debug!`-swallowed errors), R3 (import destroys audit history), R5 (fixed temp filename), R7 (32 767 ceiling), R8 (`--db` cross-workspace flush) are all FIXED upstream.** | Six of the fork's eight worst defects evaporate by rebasing. Do not re-port fork workarounds for them. | §7 below, each with code citation **[V]** |
| 13 | **Dossier R4 (selective-import / total-export divergence) is *mitigated*, not fixed; R6 (content-hash dedup) is *partially* fixed.** | Carry both forward into the Phase-2 risk register. | §7.4, §7.6 **[V]** |
| 14 | `witness.rs` is a **line-chunk Merkle tree** — inherently JSONL-shaped — but is reachable only from `br sync --witness`. `db_inode_lock.rs` is format-agnostic (database file only). `cache.rs` is **dead code** (no caller anywhere in `src/`). | Only `witness.rs` needs an Org decision, and the cheap answer is "reject `--witness` on `.org`". | `:witness.rs` header + `grep -rn "witness::" src/` → one caller; `src/cache.rs:1-5` + `grep -rn "crate::cache" src/` → 0 hits **[V]** |
| 15 | `Commands` grew 39 → 47. Eight new: `Capabilities`, `Capacity`, `Coordination`, `Gate`, `RobotDocs`, `Scheduler`, `Serve`, `VcsStatus`. | Rename surface (R4) and doc surface grow; none are Org-critical except `VcsStatus` (inspects the export file's git visibility). | enum extraction, §5 **[V]** |
| 16 | 621 hardcoded `issues.jsonl` occurrences across **160 files under `tests/`**; 396 `"issues.jsonl"` string literals in `src/`; `src/cli/commands/doctor.rs` alone has 323 `.jsonl` mentions. | The Phase-3 "decide per test file: pin JSONL or convert to Org" question is a ~160-file decision, not a handful. | `grep -rn "issues\.jsonl" tests/ \| wc -l`; `grep -rc '\.jsonl' src/` **[V]** |
| 17 | Schema version went **1 → 17**. | `:BEADS_SCHEMA_VERSION:` in the Org drawer now emits `17`; any fork assumption about `1` is stale. | `src/storage/schema.rs:11` vs `e805fe98:src/storage/schema.rs:5` **[V]** |
| 18 | The conditional-publication primitive (`renameat2` `RENAME_EXCHANGE`/`RENAME_NOREPLACE`) is **cfg-gated to Linux/Android/Apple**; every other platform hard-errors. | Not an Org concern, but it is the load-bearing atomicity primitive and it constrains where the port can be tested. | `:1464-1530` (`#[cfg(...)]` at the second definition `:1521`) **[V]** |

---

## 1. Release arc — clustering the 1,843 commits

### 1.1 Method and a caveat about `CHANGELOG.md`

The brief asked me to cluster from `CHANGELOG.md`. **The changelog is not a complete record.** It
is 974 lines and covers 40 version sections, but it has large holes precisely where the most
commits landed: there is **no section at all** for v0.1.34–v0.1.45, v0.2.0–v0.2.7,
v0.2.12–v0.2.18, or v0.2.20 **[V]** (`awk '/^## /' CHANGELOG.md` yields 40 headings; the tag list
has 59 tags). Its own preamble says it is "organized by capability rather than diff order"
(`CHANGELOG.md:7`) — it is a curated narrative, not an audit trail.

So I clustered two ways: the changelog for *intent*, and `git log` over tag spans for *mass*.

Commit-message prefix distribution over `e805fe98..upstream/main` **[V]**:

| prefix | count | | prefix | count |
|---|---|---|---|---|
| `fix` | 605 | | `docs` | 57 |
| `chore` | 240 | | `style` | 36 |
| `feat` | 216 | | `ci` | 23 |
| `test` | 171 | | `beads` (tracker data) | 16 |
| `doctor` | 140 | | `release` | 12 |
| `perf` | 125 | | `deps` | 9 |
| `refactor` | 86 | | `fuzz` | 7 |
| *(non-conventional)* | 70 | | everything else | ~28 |

**Read that first row.** A third of all post-fork work is bug fixes, and `doctor` has its own
prefix with 140 commits. This is a codebase that spent five months hardening, not expanding.

### 1.2 Mass per release span **[V]**

```
e805fe98..v0.1.15   32   2026-02-22
v0.1.15 ..v0.1.20   19   2026-02-26
v0.1.20 ..v0.1.25  135   2026-03-11
v0.1.25 ..v0.1.29  106   2026-03-18
v0.1.29 ..v0.1.33   80   2026-03-23
v0.1.33 ..v0.1.35   59   2026-04-01
v0.1.35 ..v0.1.38   20   2026-04-10
v0.1.38 ..v0.1.41   26   2026-04-16
v0.1.41 ..v0.1.45   83   2026-04-20
v0.1.45 ..v0.2.0   323   2026-04-24   <-- largest single span
v0.2.0  ..v0.2.2   111   2026-04-29
v0.2.2  ..v0.2.8   552   2026-05-14   <-- largest overall
v0.2.8  ..v0.2.11   81   2026-05-21
v0.2.11 ..v0.2.15   22   2026-06-02
v0.2.15 ..v0.2.16   45   2026-06-29
v0.2.16 ..v0.2.19   95   2026-07-27
v0.2.19 ..v0.2.20   50   2026-08-04
v0.2.20 ..v0.2.22    4   2026-08-06
```

### 1.3 The narrative, in six acts

**Act I — Feb 2026, v0.1.15–v0.1.20 (~51 commits): trailing edge of the fork point.** MIT license
gains the "OpenAI/Anthropic Rider" here (`CHANGELOG.md:632-668`, the v0.1.15 §Licensing section)
**[V]** — this is the license the plan's §8 decision is about, and it postdates the fork by one
week. Small feature adds.

**Act II — March 2026, v0.1.20–v0.1.33 (~400 commits): cross-project routing, TOON, storage
engine.** Three durable structural additions land:
- **Cross-project routing** (`src/config/routing.rs`): `.beads/redirect`, `routes.jsonl`, town
  roots, `is_external_id`, `external:<project>:<capability>` dependency targets. First landing
  v0.1.26, hardened in v0.1.27 and v0.1.32 (`CHANGELOG.md:454`, `:401`, `:241`) **[V]**.
- **TOON output format** (v0.1.27, `CHANGELOG.md:~410`) — the third output mode alongside
  JSON/rich; upstream depends on the `toon_rust` crate, exactly as the fork does **[V]**.
- **Storage engine swap to frankensqlite/`fsqlite`** — upstream no longer uses `rusqlite`; the
  connection type is `FrankenError`-returning (`src/storage/sqlite.rs`, e.g. `:17066`
  `self.conn.query_row_with_params`) **[V]**. This is a large, invisible-to-Org substrate change,
  but it is the reason `sqlite.rs` went 5,157 → 34,865 lines.

**Act III — late March–April 2026, v0.1.34–v0.2.0 (~500 commits): correctness convergence with Go
`bd`, plus the performance pass.** The single most consequential commit family here realigns
`content_hash` with Go `bd`'s algorithm and adds a **schema v7 migration that recomputes every
stored hash** (`fix(hash,schema): align content_hash algorithm with Go bd + v7 migration
recomputes stored hashes`) **[V]**. Also: read-only fast-open for pure-read commands, "skip JSONL
rewrite when dirty markers don't change content" (the seed of the incremental auto-flush path),
no-op-write elision in `update_issue`, negative-epoch datetime fixes, and text-field bounds
validation. Version rolls to 0.2.0 on 2026-04-24.

**Act IV — late April–mid May 2026, v0.2.0–v0.2.8 (663 commits): the doctor era.** This is the
largest block of work in the whole post-fork history and it is almost entirely
`br doctor`: a multi-pass detector/fixer registry (`doctor(P3)`…`doctor(P9)` commit series), a
`FixerFilter` with `--only`/`--skip` ids, refuse-gates, fingerprinted recovery artifacts,
symlink-quarantine, and a fixture suite. It is why `src/cli/commands/doctor_subsystems/` exists
(8 modules: `capabilities_doctor.rs`, `exit_codes.rs`, `mod.rs`, `mutate.rs`, `refuse_gates.rs`,
`run_dir.rs`, `schema_migration.rs`, `surface.rs`) **[V]** and why `doctor.rs` alone contains 323
`.jsonl` references **[V]**. Alongside it: `.br_history/` snapshot backups with a
`sync.history_enabled` flag (#293), policy/closure gates (#274), and the dependency refresh that
moved `fsqlite`/`fastmcp-rust` to crates.io.

**Act V — May–July 2026, v0.2.8–v0.2.19 (~250 commits): reliability and release engineering.**
Ephemeral `open_memory()` temp-file leak fix (#299, `CHANGELOG.md:114-140`), identifier-allocation
fail-closed rework (#381), Windows portability, installer checksum fail-closed, package-manifest
automation. Then, on 2026-07-27, the **"rescue" merge**: commit `251b501b`
`rescue(sync): write-authority locking, atomic export publication, br vcs-status, receipt-bound
schema migration — stranded workstream snapshot`, merged as `dbdb1296` **[V]**. That one merge is
the origin of essentially all the concurrency machinery this report describes — `JsonlFamilyWriteLock`,
`DatabaseFamilyWriteLock`, `PinnedJsonlName`, conditional publication, `br vcs-status`. The same
day, `a3a1243f` added `br sync --reconcile` lossless additive reconciliation.

**Act VI — Aug 2026, v0.2.20–v0.2.22 (54 commits): the write-authority aftershock.** v0.2.20
shipped the database-family write authority; it locked the database inode with `flock`, which
collides with the SQLite engine's own POSIX record locks on macOS/BSD and with mandatory
`LockFileEx` on Windows, bricking every workspace after `br init` outside Linux (GitHub #412).
v0.2.21 replaced it with a one-byte OFD lock at `i64::MAX - 1`
(`src/sync/db_inode_lock.rs:30-76`) **[V]**; v0.2.22 fixed the Windows identity witness to use
creation time instead of `(length, mtime)` (`CHANGELOG.md:18-34`) **[V]**.

> **Consequence for the port:** the substrate you are integrating into is 10 days old in its most
> load-bearing subsystem, and its last three releases were emergency fixes to that subsystem. Treat
> `src/sync/mod.rs`'s locking layer as *hot* — expect churn, and keep Org changes to it minimal and
> obviously separable.

---

## 2. Sync engine anatomy today

`src/sync/mod.rs` is 21,609 lines. Roughly `:1`–`:14500` is code; the remainder is
`#[cfg(test)] mod tests`. Below are the functions the Org port must touch or know about, with the
fork's hook point named where one exists.

### 2.1 Export

| Upstream function | Line | Role | Fork analogue |
|---|---|---|---|
| `export_to_jsonl` | `9623` | thin wrapper, drops the report | same name in fork |
| **`export_to_jsonl_with_policy`** | **`9642`** | **public entry, name unchanged** | **`fork:export_to_jsonl_with_policy`** |
| `export_to_jsonl_with_policy_expected` | `9650` | adds "expected previous content sha256" CAS | new |
| `export_to_jsonl_with_policy_expected_under_authority` | `9667` | caller supplies the JSONL family lock | new |
| `export_to_jsonl_with_policy_expected_under_authorities` | `9685` | plus the DB family lock | new |
| **`export_to_jsonl_with_policy_expected_authority`** | **`9704`** | **the real ~400-line body** | — |
| `export_issue_ids` | `9222` | sorted id list, drives guards + batching | — |
| `hydrate_export_issue_batch` / `..._full_scan` | `9237` / `9291` | batched vs full-scan hydration (threshold `EXPORT_FULL_SCAN_ISSUE_THRESHOLD`) | new |
| **`write_export_issue_jsonl`** | **`9386`** | **serial per-issue emission** | **`fork:emit_issue_heading` site** |
| **`prepare_export_issue_jsonl`** | **`9463`** | **parallel-worker per-issue emission** | new second emission site |
| `prepare_export_issues_jsonl_parallel` | `9506` | `thread::scope` fan-out, ≥`EXPORT_PARALLEL_PREPARE_MIN_ISSUES` | new |
| `write_prepared_export_entries` | `9555` | drains prepared lines into the writer + hasher | new |
| **`export_to_writer_with_policy_and_retention_at`** | **`10150`** | **stdout export — third emission site** | — |
| `normalize_issue_for_export` | `10821` | canonicalization before serialization | — |
| `finalize_export` / `..._under_authority` | `10656` / `10666` | clears dirty flags, stamps hashes/metadata | `fork:finalize_export` |

**Body shape of `export_to_jsonl_with_policy_expected_authority` (`:9704`–`:10113`) [V]:**

1. `:9713` verify DB authority if supplied.
2. `:9720` `validate_sync_path_with_external(output_path, beads_dir, allow_external_jsonl)` — the allowlist gate.
3. `:9732` `fs::create_dir_all(parent_dir)`.
4. `:9736-9747` acquire (or accept) the **JSONL family write lock**; `pinned_name_for_target`; `verify_jsonl_authority`.
5. `:9749-9790` capture / verify the *expected previous generation* of the target file (CAS input).
6. `:9801` `history::backup_before_export_snapshot` → `.br_history/`.
7. `:9810` `export_issue_ids`; `:9813` dirty metadata; `:9814` intentionally-excluded set.
8. `:9817-9871` **the two data-loss guards** — Check 1 empty-DB-over-non-empty-file (`:9827`), Check 2 stale-DB-would-lose-ids (`:9840`). Both subtract `get_purged_ids_pending_export()` (#405). Both call `analyze_jsonl_snapshot`.
9. `:9889` `create_full_export_temp_file_under_authority` → pinned temp fd.
10. `:9899-10015` the emission loop (serial or parallel; full-scan or batched).
11. `:10018-10023` flush + `sync_all`.
12. `:10025` final content sha256; `:10029` `verify_exported_jsonl_snapshot_integrity` against `exported_ids`; `:10030-10034` staged-bytes-vs-hash check.
13. `:10035-10056` optional `expected_staged_output` exact-match gate (used by `sync --merge` sagas).
14. `:10058-10069` `require_safe_sync_overwrite_path` on both temp and target.
15. `:10075` **`publish_staged_jsonl_conditionally`** — the atomic swap.
16. `:10089-10105` assemble `ExportResult` incl. `issue_hashes` and a `publication` receipt.

**Org integration verdict [I]:** steps 2, 8, 9, 10, 12 are format-sensitive. Step 8 in particular
calls `analyze_jsonl_snapshot` (`:9061`), which JSON-parses every line to harvest ids — for `.org`
that must become an Org id scan. Steps 4, 5, 14, 15 are format-agnostic and should be left alone.

### 2.2 Atomic publication

- `publish_staged_jsonl_conditionally` `:1544` → `..._with_hooks` `:1612` → `..._with` `:1778`.
- `perform_conditional_namespace_change` `:1464` (Linux/Android/Apple) / `:1521` (everything else,
  hard error) — `rustix::fs::renameat_with` with `RenameFlags::EXCHANGE` when a previous generation
  is expected, `RenameFlags::NOREPLACE` when the target is expected absent **[V]**.
- Post-swap it re-captures the target and asserts `state_witness()` + `content_sha256` equality, and
  asserts the *displaced* file matches the expected previous witness (`:1657-1697`) **[V]**.
- Failure modes are named errors: `BeadsError::JsonlPublicationConflict` (staged file retained at a
  recovery path) and `BeadsError::JsonlPublishedButUnwitnessed` **[V]**.

This is entirely path/byte-level and **format-agnostic**. It works for `issues.org` unchanged
provided the allowlist and temp-name logic let the `.org` names through.

### 2.3 Temp files (this is where the port breaks silently)

```rust
// src/sync/mod.rs:1818
fn export_temp_path_for_attempt(output_path: &Path, attempt: u32) -> PathBuf {
    let pid = std::process::id();
    if attempt == 0 {
        return output_path.with_extension(format!("jsonl.{pid}.tmp"));
    }
    let retry_suffix = u64::from(pid).saturating_mul(100).saturating_add(u64::from(attempt));
    output_path.with_extension(format!("jsonl.{retry_suffix}.tmp"))
}
```

`Path::with_extension` **replaces** the final extension. For `…/issues.org` this yields
`…/issues.jsonl.<pid>.tmp` — a JSONL-named temp file holding Org bytes, which then gets
`RENAME_EXCHANGE`d onto `issues.org`. It would *work*, and it would be wrong in a way no test
would catch. The fork solved this at `fork:src/sync/mod.rs:1414-1426` with
`let temp_ext = if is_org_format { "org.tmp" } else { "jsonl.tmp" };`. **[V]**

Callers: `create_jsonl_temp_file` `:1830` (legacy, unpinned),
`create_pinned_jsonl_temp_file_with` `:1877`, `create_full_export_temp_file_under_authority` `:1912`,
`create_base_snapshot_temp_file_under_authority` `:1940`. **[V]**

### 2.4 Import

| Upstream function | Line | Role |
|---|---|---|
| **`import_from_jsonl`** | **`12539`** | public entry, name unchanged; validates path, captures a snapshot |
| `import_from_jsonl_snapshot` | `12561` | the body |
| `for_each_jsonl_import_issue` | `12077` | the **only** line→`Issue` iterator; used by pass 1 |
| `parse_normalized_import_issue` | `12056` | `serde_json::from_str` + `normalize_issue` + `IssueValidator::validate` |
| `collect_import_validation_plan` | `12098` | pass 1: prefix check, duplicate-id check, record count |
| `scan_import_collision_renames` | `12271` | pass 2: id-collision remapping |
| `stream_import_actions_in_tx` | `12415` | pass 3: the actual writes |
| `detect_collision` | `11839` | 4-phase: ext-ref → **id** → content-hash → new |
| `determine_action` | `11880` | tombstone protection, `force_upsert`, then LWW on `updated_at` |
| `normalize_issue` | `11931` | dedupes labels, recomputes `content_hash`, wisp/ephemeral, `closed_at` repair |
| `read_issues_from_jsonl` | `11744` | standalone whole-file read (merge, snapshots) |
| `read_jsonl_lines_by_id` | `9183` | id → raw line map, used by incremental flush |

**Body shape of `import_from_jsonl_snapshot` (`:12561`–`:12658`) [V]:**
path validation (`:12568`) → `ensure_no_conflict_markers_snapshot` (`:12576`) →
`collect_import_validation_plan` (`:12580`) → optional `build_prefix_renames` (`:12587`) →
`load_import_metadata_maps` (`:12593`) → `scan_import_collision_renames` (`:12596`) →
`compute_jsonl_snapshot_content_hash` (`:12605`) → `PRAGMA foreign_keys = OFF` (`:12614`) →
`with_write_transaction { stream_import_actions_in_tx (:12627); set last_import_time (:12638);
set jsonl_content_hash (:12639); record_observed_jsonl_witness (:12640) }` →
`restore_foreign_keys_after_import` (`:12646`, with FK validation).

**Org integration verdict [I]:** the seam is `for_each_jsonl_import_issue` + the two other
streaming readers. The cheapest Org shape is a `for_each_import_issue(source)` that, for `.org`,
parses once via `org2jsonl` and replays the resulting `Vec<Issue>` through the same closure — but
note the closure receives `line_num` and every error message says "at line N" / "Invalid JSON at
line N" (`:12057`, `:12118`, `:11780`, `:9044`). Org needs its own diagnostic vocabulary, and
`gap-org2jsonl.md`'s known defect (an `:ID:`-less heading bricks parsing *with no line info*) makes
this worse, not better, against upstream's now-quite-good line-precise errors.

### 2.5 The incremental auto-flush path — the biggest new hazard

`auto_flush` (`:11620`) now branches **[V]**:

```
auto_flush
  ├─ :11631  inspect_pending_sync_merge — refuse if a merge saga is pending
  ├─ :11643  pending_export_state → (dirty_count, needs_flush, db_newer); bail if !db_newer
  ├─ :11650  validate_sync_path_with_external
  ├─ :11661  scan_conflict_markers → refuse (warn!) if markers present
  ├─ :11678  if !needs_flush  → try_incremental_auto_flush(...)   ← NEW, the common path
  └─ :11712  else / on None  → export_to_jsonl_with_policy_expected + finalize_export
```

`try_incremental_auto_flush` (`:11492`) **[V]**:
1. bail if the file does not exist;
2. acquire the JSONL family lock; compute `source_content_hash`; re-scan conflict markers;
3. `collect_incremental_auto_flush_changes` — serializes each dirty issue to **one JSON line**
   (`:11377`), partitions into `replacement_lines` and `removed_hash_ids`;
4. `try_existing_line_auto_flush` (`:11405`) → `try_write_existing_jsonl_replacements_atomically`
   (`:11194`) → `scan_existing_jsonl_replacements` (`:11073`), which **parses every line as
   `PartialId`** and substitutes matched lines in place. It `Declined`s if any replacement id is not
   already in the file (GitHub #404: a new issue could only be appended at the tail, breaking
   id-sorted order);
5. on decline, falls back to `read_jsonl_lines_by_id` (`:9183`) + `apply_incremental_auto_flush_changes`
   (`:11475`) + `write_jsonl_lines_atomically` (`:11320`) — a full rewrite from a `BTreeMap<id, line>`;
6. `finalize_incremental_auto_flush` (`:10999`) updates metadata inside one write transaction.

**Every step of this assumes "one issue = one line".** For Org there are three options, and Phase 2
must pick one explicitly:
- **(a)** make `try_incremental_auto_flush` return `Ok(None)` when the target is `.org`, so Org
  always takes the full-export path. Cheapest, safest, loses the perf win. **Recommended [I].**
- **(b)** teach the machinery to work on "records" rather than lines (a `Vec<(id, Vec<u8>)>`
  abstraction). Large, invasive, touches the hottest code.
- **(c)** leave it alone and let it corrupt Org files. Not an option.

### 2.6 Format selection today — there is none

**[V]** Exhaustively checked:
- `grep -rn "org_bridge\|is_org_format\|orgize\|org2jsonl" src/` → **zero hits**.
- `grep -rn "enum [A-Za-z]*Format" src/` → `OutputFormat` (`src/cli/mod.rs:1567`),
  `OutputFormatBasic` (`:1611`), `GitObjectFormat` (`src/cli/commands/vcs.rs:85`). All three are
  about *terminal/JSON output*, not the storage file.
- No trait, no `dyn` dispatch, no `match extension`. The word "jsonl" appears **1,310 times** in
  `src/` across 25+ files.
- The only extension-shaped logic anywhere is negative: `is_excluded_jsonl` (`src/config/mod.rs:168`)
  rejecting merge artifacts, and `is_allowed_jsonl_temp_name` (`src/sync/path.rs:546`).

So the fork's design — dispatch on `.org` vs everything-else at the read/write boundary — remains
the right shape, but it must be introduced as new architecture rather than parameterized into an
existing one.

**Complete inventory of `Issue`↔file (de)serialization sites in `src/` [V]:**

| Direction | Site | Line |
|---|---|---|
| read | `validate_jsonl_issue_records_from_reader` | `sync/mod.rs:8260` |
| read | `analyze_jsonl_from_reader` (`PartialId`) | `sync/mod.rs:9044` |
| read | `verify_exported_jsonl_snapshot_integrity` | `sync/mod.rs:9105` |
| read | `get_issue_ids_from_jsonl*` (`PartialId`) | `sync/mod.rs:9203` |
| read | `scan_existing_jsonl_replacements` (`PartialId`) | `sync/mod.rs:11099` |
| read | `write_existing_jsonl_replacements_atomically` (`PartialId`) | `sync/mod.rs:11254` |
| read | `read_issues_from_jsonl_reader` | `sync/mod.rs:11780` |
| read | `parse_normalized_import_issue` | `sync/mod.rs:12057` |
| read | `scan_jsonl_for_tombstone_filter_from_reader` | `sync/mod.rs:14469` |
| read | `first_prefix_from_jsonl_reader` (raw `Value`) | `config/mod.rs:4160` |
| read | `br show --no-db` JSONL dependent scan | `cli/commands/show.rs:684` |
| read | `br stats` git-diff activity parser | `cli/commands/stats.rs:830` |
| read | completions issue index (`CompletionIssue`) | `cli/mod.rs:288` |
| write | `write_export_issue_jsonl` | `sync/mod.rs:9394` |
| write | `prepare_export_issue_jsonl` | `sync/mod.rs:9473` |
| write | `collect_incremental_auto_flush_changes` | `sync/mod.rs:11377` |
| write | doctor JSONL rebuild / sentinel | `cli/commands/doctor.rs:19645`, `:13615` |

That is **17 sites** the port must reason about, versus a handful at the fork point.

---

## 3. The three named modules

### 3.1 `src/sync/witness.rs` (1,894 lines)

**What it is [V].** A pure, filesystem-free Merkle-style witness over a JSONL byte stream. Header:
*"This module is intentionally pure: it reads JSONL bytes and produces a stable witness without
touching files, paths, storage, or git state. The serial import/export path can keep its existing
behavior while future parallel sync work uses these witnesses to prove unchanged chunks."*
(`witness.rs:1-6`).

Public surface: `JSONL_WITNESS_SCHEMA_VERSION = "br.jsonl-witness.v1"` (`:18`), domain-separated
hash prefixes `br:jsonl-witness:{root,chunk}:v1` (`:20-21`), types `JsonlMerkleWitness` (`:26`),
`JsonlChunkWitness` (`:37`), `JsonlWitnessComparison` (`:50`), `JsonlWitnessReusePlan` (`:78`),
`JsonlWitnessParallelWorkPlan` (`:143`), and functions `build_jsonl_merkle_witness` (`:218`),
`..._parallel` (`:281`), `plan_jsonl_witness_reuse` (`:322`), `plan_jsonl_witness_parallel_work`
(`:404`), `materialize_jsonl_witness_reuse_plan` (`:468`), `compare_jsonl_merkle_witnesses` (`:579`).

**Does it assume JSONL? Yes, structurally.** The chunk unit is `chunk_size_lines` — a fixed number
of **lines** (`JsonlMerkleWitness.chunk_size_lines`, `:29`). Chunk boundaries are line boundaries.
Applied to an Org file the chunks would straddle heading boundaries arbitrarily. It would not
*corrupt* anything (it is read-only and pure), but the witness would be semantically meaningless as
a "which issues changed" proof.

**Reachability [V].** `grep -rn "witness::" src/` outside the module itself yields exactly one
importer: `src/cli/commands/sync.rs:13`. It is the implementation of `br sync --witness`
(`SyncArgs.witness`, `SyncArgs.witness_chunk_lines`, `SyncArgs.witness_parallelism`,
`src/cli/mod.rs` `SyncArgs`). It is **not** on the export, import, or auto-flush path.

**Recommendation [I].** Do not port. Have `br sync --witness` return a clear
"chunk witnesses are only defined for line-oriented JSONL exports" error when the resolved export
path is `.org`. Cheap, honest, and testable.

> Note: this is a *different* thing from the several other "witness" concepts in `sync/mod.rs`
> (`JsonlWitness` at `:10491`, `JsonlSourceStateWitness`, `AdditiveTableWitness`,
> `capture_sync_database_witness` `:5318`). Those are file-identity and DB-state fingerprints and
> are format-agnostic. Do not conflate them.

### 3.2 `src/sync/db_inode_lock.rs` (298 lines)

**What it is [V].** The crate's single sanctioned `unsafe` exemption (`:59-62`), added in v0.2.21 to
fix GitHub #412. It acquires an **exclusive one-byte advisory lock at offset
`DATABASE_INODE_LOCK_OFFSET = i64::MAX - 1`** on the database file's inode, using
`fcntl(F_OFD_SETLK)` on Linux/Android/macOS/iOS, a one-byte `LockFileEx` on Windows, and
`File::try_lock` (`flock`) elsewhere (`:30-52`, `:76`).

**Why the offset.** SQLite engines lock `0x4000_0000..0x4000_0200`; `i64::MAX - 1` is astronomically
beyond that and beyond any real database size, so on Windows the *mandatory* byte lock can never
intersect real I/O (`:64-75`). The whole-file `flock` that v0.2.20 used collided with the engine's
own POSIX record locks on macOS/BSD (one kernel lock table, conflicting even intra-process) and with
mandatory `LockFileEx` on Windows — producing "Database error: database is busy" / "database schema
is missing or unreadable" on every command after `br init` (`:11-28`).

**Purpose.** Hard-link-alias exclusion: two paths naming one physical database cannot acquire two
independent write authorities.

**Does it assume JSONL? No [V].** It never sees the export file. Zero `.jsonl` occurrences in the
module. **Format-agnostic; the Org port touches nothing here.**

### 3.3 `src/cache.rs` (641 lines)

**What it is [V].** A pure, storage-independent **S3-FIFO** bounded cache policy kernel:
`S3FifoConfig` (`:17`, `Default` = **disabled**, `:52-56`), `S3FifoSegment`, `S3FifoAdmission`,
`S3FifoStats`, `S3FifoCache<K, V>` with `get`/`put`/`remove`/`ghost_contains`/`segment_of`.
Header: *"Pure cache policies for future high-RAM workspace acceleration… a bounded policy kernel
that can be replayed against traces before any storage read path depends on it."* (`:1-5`).

**Reachability [V].** `grep -rn "crate::cache\|cache::" src/` outside the module → **zero hits**. It
is exported (`src/lib.rs:30 pub mod cache;`) and otherwise unused. **It is dead code.**

**Does it assume JSONL? No.** It is generic over `K: Hash + Eq` and `V`.

**Recommendation [I].** Ignore for the Org port. Flag it for the R3 productization audit as a
deletion candidate (or leave it — it costs nothing and upstream clearly intends to wire it up).

### 3.4 The JSONL-assumption inventory in the safety layer (plan landmine §7.5 — CONFIRMED)

This is the section the plan warned about, and the plan was right.

**(a) The central allowlist — `src/sync/path.rs:85-94` [V]:**

```rust
pub const ALLOWED_EXTENSIONS: &[&str] = &[
    "db", "db-wal", "db-shm", "db-journal",
    "db-fsqlite-ns-gate", "db-fsqlite-ns-use",
    "jsonl",              // JSONL export
    "jsonl.tmp",          // Atomic write temp files (plus pid-scoped .jsonl.<pid>.tmp)
];
pub const ALLOWED_EXACT_NAMES: &[&str] = &[".manifest.json", "metadata.json"];
```
No `org`. The matcher is `validate_extension_and_name` (`src/sync/path.rs:512-544`); anything not
matching is returned as `PathValidation::DisallowedExtension` (`:540`).
The module doc-comment table at `:16-27` repeats the list in prose and must be updated too.

**(b) The temp-name grammar — `is_allowed_jsonl_temp_name` `src/sync/path.rs:546-559` [V]:**
accepts `*.jsonl.tmp` and `<base>.jsonl.<all-digits>.tmp`. Nothing else. Combined with §2.3 this
means an Org port that fixes `export_temp_path_for_attempt` to emit `issues.org.<pid>.tmp` will be
**rejected by the allowlist** unless this function is fixed in the same commit.

**(c) External-path validation — `validate_external_jsonl_path` `src/sync/path.rs:679-717` [V]:**
```rust
if !file_name.ends_with(".jsonl") && !is_allowed_jsonl_temp_name(&file_name) {
    return Err(BeadsError::Config(format!("{} must be a .jsonl file", ...)));
}
```
This is the `BEADS_JSONL` / `--allow-external-jsonl` gate. Hard-rejects `.org`.

**(d) The private hand-copied allowlist in the git-safety test — `tests/e2e_sync_git_safety.rs:1274-1283` [V]:**
```rust
const ALLOWED_EXTENSIONS: &[&str] = &[
    "db", "db-journal", "db-wal", "db-shm",
    "db-fsqlite-ns-gate", "db-fsqlite-ns-use",
    "jsonl", "jsonl.tmp",
];
```
This is a *second, independent* copy — exactly the drift that (per the dossier's assurance leg)
made the fork's git-safety tests fire the project's own `SAFETY VIOLATION` alarm. It is the only
such copy in `tests/` (`grep -rn "ALLOWED_EXTENSIONS" tests/` → one file). It is now also more
elaborate than the source: it additionally allowlists `.br_recovery/` artifacts by suffix
(`.bak`, `.rebuild-failed`, `.truncated-wal`, `:1267-1272`). **Any Org port must patch both
copies in the same commit, and should consider making the test import
`beads_rust::sync::path::ALLOWED_EXTENSIONS` instead of re-declaring it.**

**(e) Conflict-marker scanning — format-agnostic and safe [V].** `scan_conflict_markers` `:8932`,
`detect_conflict_marker` `:8944` just look for `<<<<<<<` / `=======` / `>>>>>>>` at line start.
Works identically on Org text. One caveat: it calls `path::validate_jsonl_fd_metadata(&file, path)`
(`:8934`), which is only a regular-file check (`src/sync/path.rs:2548`) — safe. **No change needed**,
though the `.jsonl`-named helper is a rename-surface item.

**(f) Filename literals — the scale of the problem [V]:**

| Scope | Metric | Count |
|---|---|---|
| `src/` | `"issues.jsonl"` string literals | 396 |
| `src/` | any `.jsonl` occurrence | 1,310 |
| `src/cli/commands/doctor.rs` | `.jsonl` occurrences | 323 |
| `src/sync/mod.rs` | `.jsonl` occurrences | 196 |
| `src/config/mod.rs` | `.jsonl` occurrences | 180 |
| `src/sync/path.rs` | `.jsonl` occurrences | 98 |
| `src/cli/commands/sync.rs` | `.jsonl` occurrences | 88 |
| `tests/` | files mentioning `issues.jsonl` | **160** |
| `tests/` | total `issues.jsonl` occurrences | **621** |

Non-`.rs` hits matter too: `tests/doctor_fixtures/*/corrupt.sh` and `*/assert.sh` hardcode
`issues.jsonl` (e.g. `jsonl_oversized/corrupt.sh`, `jsonl_duplicate_ids/corrupt.sh`,
`base_jsonl_symlink_quarantine/assert.sh`) **[V]**.

**(g) Merge-artifact and excluded filenames — `src/config/mod.rs:71-79` [V]:**
`deletions.jsonl`, `interactions.jsonl`, `beads.base.jsonl`, `beads.left.jsonl`,
`beads.right.jsonl`, `sync_base.jsonl`. The three-way merge anchor is
`.beads/beads.base.jsonl` (`src/cli/commands/sync.rs:2009`, `:4239`; `src/cli/mod.rs:932`) and the
doctor has a whole detector/fixer family around it (`doctor.rs:4061-4090`, `:7820`). **These stay
JSONL even in an Org workspace unless the port says otherwise — decide explicitly.**

**(h) Lock sidecar names [V].** `.write.lock` (`:199`, `:1088`), `.sync.lock` (`:1368`),
`.br-db-write-<24hex>.lock` (`:991`), `.br-jsonl-write-<24hex>.lock` (`:1006`). The JSONL one is
derived from the canonical *path*, so it works for `issues.org` unchanged — but its name says
"jsonl" and it is created inside `.beads/`, where the allowlist does not cover `.lock`… because
these are created outside `validate_sync_path` (via `open_and_lock_regular_file`). Worth confirming
during Phase 3 that adding `.org` to the allowlist does not disturb the lock sidecars.

---

## 4. Config discovery today

### 4.1 Constants **[V]**

| Constant | Value | Line |
|---|---|---|
| `DEFAULT_DB_FILENAME` | `"beads.db"` | `src/config/mod.rs:60` |
| **`DEFAULT_JSONL_FILENAME`** | **`"issues.jsonl"`** (private `const`, **not** `pub`) | **`src/config/mod.rs:62`** |
| `LEGACY_JSONL_FILENAME` | `"beads.jsonl"` | `src/config/mod.rs:64` |
| `RECOVERY_DIR_NAME` | `".br_recovery"` | `src/config/mod.rs:66` |
| `EXCLUDED_JSONL_FILES` | 6 names, §3.4(g) | `src/config/mod.rs:71-79` |
| `CURRENT_SCHEMA_VERSION` | `17` (was `1` at fork point) | `src/storage/schema.rs:11` |

Fork's counterpart: `pub const DEFAULT_JSONL_FILENAME: &str = "issues.org"`,
`LEGACY_JSONL_FILENAME = "issues.jsonl"`, `OLDER_LEGACY_JSONL_FILENAME = "beads.jsonl"`
(`fork:src/config/mod.rs:31-37`) **[V]** — the plan's §6.1 citation of `fork:…:33` is accurate.

### 4.2 Workspace discovery order **[V]**

`discover_beads_dir(start)` (`:241`) → `discover_beads_dir_with_env` (`:246`) →
`discover_beads_dir_with_env_and_ceiling` (`:253`):

1. If `BEADS_DIR` is set → `resolve_explicit_beads_dir(path, "BEADS_DIR")`; **discovery stops**.
2. Otherwise `discover_beads_dir_candidate_with_env_and_ceiling` (`:274`) walks **up** from `start`
   (or CWD), testing `current.join(".beads").is_dir()` then `current.join("_beads").is_dir()` at each
   level, stopping at an optional ceiling or filesystem root (`:281-303`).
3. `Err(BeadsError::NotInitialized)` if nothing found.
4. **Then** `routing::follow_redirects(&candidate, 10)` (`:263`).

`is_beads_dir_name` (`:55-57`) is the canonical `.beads` / `_beads` predicate — one place, easy to
extend for `.obr`/`_obr` in Phase 6.

`discover_beads_dir_with_cli(cli)` (`:324`) → `..._with_cli_from_and_ceiling` (`:394`) adds `--db`
handling **before** the walk:
- if `cli.db` is set and `beads_dir_from_db_path(db)` resolves (i.e. the db lives under a
  `.beads`/`_beads` dir) → use that directory;
- else if the `BD_DB`/`BD_DATABASE` env override resolves the same way → use that;
- else fall through to the CWD walk, and on `NotInitialized` wrap the error with
  *"Cannot resolve the project .beads directory for database override '…'; run from the target
  workspace or set BEADS_DIR"* (`:411-420`).

There are also `discover_optional_beads_dir_with_cli` (`:347`) — returns `Ok(None)` on
`NotInitialized` when no explicit `--db` — and `discover_optional_beads_dir_candidate_with_cli`
(`:362`), the pre-redirect variant.

### 4.3 Export-file path resolution — `resolve_jsonl_path` `:4219-4262` **[V]**

Five priorities, in order:
1. **`BEADS_JSONL` env var** (non-empty, trimmed) — absolute wins, no containment check here
   (the allowlist enforces it later).
2. **`metadata.json`'s `jsonl_export`**, but only if it differs from `DEFAULT_JSONL_FILENAME`
   *and* is not an excluded name — i.e. "explicit override".
3. **`--db` sibling**: `discover_jsonl(db.parent())`, else `db.parent()/issues.jsonl`.
4. **File discovery**: `discover_jsonl(beads_dir)` (`:147-162`) — prefer `issues.jsonl`, fall back
   to `beads.jsonl`, else `None`.
5. **Default for writing**: `beads_dir/issues.jsonl`.

> **Port note [I]:** the fork's `issues.org`-first chain slots into step 4 (`discover_jsonl`) with a
> one-line prepend, and step 5 changes with the constant. But **step 2 is a trap**: making
> `DEFAULT_JSONL_FILENAME = "issues.org"` silently promotes a `metadata.json` that says
> `"jsonl_export": "issues.jsonl"` — which `br init` writes today (`src/cli/commands/init.rs:173`)
> and which every existing workspace on disk contains — from "the default" to "an explicit
> override". That is arguably correct (legacy workspaces keep reading their JSONL) but it must be a
> *deliberate, tested* decision, not an accident.

`resolve_db_path` `:4199-4217`: `--db` wins; else `metadata.database`, joined against
`crate::util::resolve_cache_dir(beads_dir)` — i.e. **`BEADS_CACHE_DIR` can move the database off
`.beads/`** while the export file stays put.

### 4.4 `.beads/redirect` **[V]**

`routing::read_redirect` (`src/config/routing.rs:189`): reads `<beads_dir>/redirect`, size-capped
at `MAX_REDIRECT_BYTES`, must be valid UTF-8, empty → warn + `None`. Relative targets resolve
against the `.beads` directory itself ("so `.` stays within the workspace storage root instead of
escaping to the project root", `:208-209`). `follow_redirects` (`:256`) canonicalizes, tracks
`visited`, and bounds the chain at `max_depth` (10 from the discovery caller).

> Compared with the dossier's R28 (which described `follow_redirects` as accepting "any absolute
> path, `..`, or symlink, and applying no validation whatsoever" at `fork:src/config/routing.rs:200-232`),
> upstream has added a size cap, UTF-8 validation, loop detection, and relative-resolution semantics.
> Whether it now validates the *target* enough to close R28 is outside this brief — flag for R2/Phase 2.

### 4.5 Config file chain **[V]**

- Project: `<beads_dir>/config.yaml` (`ConfigPaths::project_config_path`, `:228-231`).
- User: `~/.config/beads/config.yaml` if it exists, **else** `~/.config/bd/config.yaml`
  (`user_config_path`, `:205-216`).
- Legacy user: `~/.beads/config.yaml` (`legacy_user_config_path`, `:219-225`).

### 4.6 Environment variables **[V]**

Complete set of `BEADS_*` / `BD_*` / `BR_*` string literals in `src/`
(`grep -rhoE '"(BEADS|BD|BR)_[A-Z0-9_]+"' src/ | sort -u`):

```
BD_ACTOR  BD_DATABASE  BD_DB  BD_NO_AUTOFLUSH
BEADS_ACTOR  BEADS_AUTO_START_DAEMON  BEADS_CACHE_DIR  BEADS_DIR
BEADS_FLUSH_DEBOUNCE  BEADS_IDENTITY  BEADS_JSONL  BEADS_REMOTE_SYNC_INTERVAL
BR_AGENT_NAME  BR_DISABLE_PARALLEL_JSONL_EXPORT  BR_DISABLE_READ_ONLY_FAST_OPEN
BR_DOCTOR_RUNS_DIR  BR_DOCTOR_STALE_LOCK_THRESHOLD_SECS  BR_HARNESS
BR_HISTORY_MIN_INTERVAL_SECS  BR_INHERITED_CONTEXT  BR_MCP_READ_SNAPSHOT
BR_MODEL  BR_NO_AUTOFLUSH  BR_OUTPUT_FORMAT  BR_SESSION
BR_STARTUP_CACHE  BR_STARTUP_CACHE_DIR
```

27 variables in three spellings — up from the fork's smaller set. This is R4's problem, but two are
R1's: **`BEADS_JSONL`** (steers the export file, hard-gated to `.jsonl` by
`validate_external_jsonl_path`) and **`BR_DISABLE_PARALLEL_JSONL_EXPORT`**
(`src/sync/mod.rs:9452`, the escape hatch for the parallel emission path — useful during Org
bring-up).

### 4.7 `--no-db` mode still works the same way **[V]**

`open_storage_with_startup_config_impl` (`src/config/mod.rs:3700`), `if no_db` branch at `:3728`:
`SqliteStorage::open_memory()` → `validate_sync_path_with_external(jsonl_path, …)` → optionally
acquire the JSONL family write lock (`cli.no_db_write_intent`) → `capture_optional_jsonl_source` →
infer prefix from the file (`resolve_bootstrap_issue_prefix_snapshot`, which calls
`first_prefix_from_jsonl_reader`, `:4151`) → `import_from_jsonl_snapshot(…, skip_prefix_validation = true)`
→ load workflow policy → retain the exact source witness so a later flush can reject even
whitespace-only drift.

The plan's §6.6 ("`--no-db` mode: seeds in-memory DB by importing the org file; must keep working")
is exactly right, and the seam is unchanged in shape. Note the **extra** Org requirement upstream
introduced: prefix inference reads the file directly with a raw `serde_json::Value` parse, so it
needs its own Org path (`config/mod.rs:4151-4185`).

---

## 5. The `Commands` enum today

47 variants (`src/cli/mod.rs:748`ff). Fork point had 39. **New since `e805fe98` marked ★.**

| Variant | Args type | New? |
|---|---|---|
| `Agents` | `AgentsArgs` | |
| `Audit` | inline `{…}` | |
| `Blocked` | `BlockedArgs` | |
| `Capabilities` | `CapabilitiesArgs` | ★ |
| `Capacity` | inline `{…}` | ★ |
| `Changelog` | `ChangelogArgs` | |
| `Close` | `CloseArgs` | |
| `Comments` | `CommentsArgs` | |
| `Completions` | `CompletionsArgs` | |
| `Config` | inline `{…}` | |
| `Coordination` | inline `{…}` | ★ |
| `Count` | `CountArgs` | |
| `Create` | `CreateArgs` | |
| `Defer` | `DeferArgs` | |
| `Delete` | `DeleteArgs` | |
| `Dep` | inline `{…}` | |
| `Doctor` | `DoctorArgs` | |
| `Epic` | inline `{…}` | |
| `Gate` | inline `{…}` | ★ |
| `Graph` | `GraphArgs` | |
| `History` | `HistoryArgs` | |
| `Info` | `InfoArgs` | |
| `Init` | inline `{…}` | |
| `Label` | inline `{…}` | |
| `Lint` | `LintArgs` | |
| `List` | `ListArgs` | |
| `Orphans` | `OrphansArgs` | |
| `Q` | `QuickArgs` | |
| `Query` | inline `{…}` | |
| `Ready` | `ReadyArgs` | |
| `Reopen` | `ReopenArgs` | |
| `RobotDocs` | inline `{…}` | ★ |
| `Scheduler` | `SchedulerArgs` | ★ |
| `Schema` | `SchemaArgs` | |
| `Search` | `SearchArgs` | |
| `Serve` | `crate::mcp::ServeArgs` | ★ (feature-gated MCP server) |
| `Show` | `ShowArgs` | |
| `Stale` | `StaleArgs` | |
| `Stats` | `StatsArgs` | |
| `Status` | `StatsArgs` (alias for `stats`) | |
| `Sync` | `SyncArgs` | |
| `Undefer` | `UndeferArgs` | |
| `Update` | `UpdateArgs` | |
| `Upgrade` | `UpgradeArgs` | |
| `VcsStatus` | `VcsStatusArgs` (`#[command(name = "vcs-status")]`) | ★ |
| `Version` | `VersionArgs` | |
| `Where` | *(unit)* | |

Verified by extracting the enum body from both trees and diffing the variant name sets **[V]**.
Command module files: 45 under `src/cli/commands/` plus 8 under `doctor_subsystems/` **[V]**.

**`SyncArgs` modes (all boolean unless noted) [V]:** `flush_only`, `import_only`, `merge`,
`reconcile`, `dry_run`, `status`, **`witness`**, **`reconcile_additive`**, `apply`,
`resolve_source_ids: Vec<String>`, `witness_chunk_lines: usize`, `witness_parallelism: Option<usize>`,
`export_parallelism: Option<usize>`, `force`, `force_db`, `force_jsonl`, `allow_external_jsonl`,
`manifest`, `error_policy: Option<String>`, `orphans: Option<String>`, `rename_prefix`, `rebuild`,
`robot`. The fork's sync surface is a strict subset.

---

## 6. JSONL wire format and the complete `Issue` model

### 6.1 Wire format: unchanged since the fork point **[V]**

- **One bare `Issue` JSON object per line.** No header, no trailer, no `_type` discriminator, no
  schema-version record, no envelope. Confirmed by reading every reader and every writer:
  `read_issues_from_jsonl_reader` (`:11757`) does `serde_json::from_str::<Issue>(trimmed)` on each
  non-empty line with no special-casing; `write_export_issue_jsonl` (`:9394`) does
  `serde_json::to_writer(&mut *buffer, issue)` then `b"\n"`. Same for the parallel worker (`:9473`)
  and the incremental path (`:11377`).
- Blank lines are tolerated on read (skipped) and never written.
- **Ordering is canonical: sorted by id.** `export_issue_ids` (`:9222`) returns sorted ids;
  the incremental writer declines rather than append out of order (GitHub #404, `:11051-11057`, `:11202-11209`).
- **Duplicate ids are a hard error on read** in every reader (`:11785`, `:12118`, `:9046`, `:11101`).
- **Content hash** is `#[serde(skip)]` — never on the wire (`src/model/mod.rs:464-466`).
- Sidecar files that are *not* the export: `deletions.jsonl`, `interactions.jsonl`,
  `beads.base.jsonl` (merge anchor), `beads.left/right.jsonl`, `sync_base.jsonl`,
  `.manifest.json`, `metadata.json`, `.br_history/*` backups.

**Net for R2: the Org representation needs to model exactly the `Issue` struct — nothing more.**

### 6.2 Complete `Issue` field list (`src/model/mod.rs:460-624`) **[V]**

`#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]`

| # | Field | Type | serde | Note |
|---|---|---|---|---|
| 1 | `id` | `String` | always | |
| 2 | `content_hash` | `Option<String>` | **`skip`** | never serialized |
| 3 | `title` | `String` | always | 1–500 chars |
| 4 | `description` | `Option<String>` | `default`, skip-if-none | Org body text in the fork |
| 5 | `design` | `Option<String>` | `default`, skip-if-none | |
| 6 | `acceptance_criteria` | `Option<String>` | `default`, skip-if-none | |
| 7 | `notes` | `Option<String>` | `default`, skip-if-none | |
| 8 | `status` | `Status` | `default` | see §6.3 |
| 9 | `priority` | `Priority` | `default` | 0=Critical … 4=Backlog |
| 10 | `issue_type` | `IssueType` | `default` | |
| 11 | `assignee` | `Option<String>` | `default`, skip-if-none | |
| 12 | `owner` | `Option<String>` | `default`, skip-if-none | |
| 13 | `estimated_minutes` | `Option<i32>` | `default`, skip-if-none | |
| 14 | `created_at` | `DateTime<Utc>` | always | |
| 15 | `created_by` | `Option<String>` | `default`, skip-if-none | **hashed** into `content_hash` |
| 16 | `updated_at` | `DateTime<Utc>` | always | LWW key |
| 17 | `closed_at` | `Option<DateTime<Utc>>` | `default`, skip-if-none | |
| 18 | `close_reason` | `Option<String>` | `default`, skip-if-none | |
| 19 | `closed_by_session` | `Option<String>` | `default`, skip-if-none | |
| 20 | `due_at` | `Option<DateTime<Utc>>` | `default`, skip-if-none | |
| 21 | `defer_until` | `Option<DateTime<Utc>>` | `default`, skip-if-none | |
| 22 | `external_ref` | `Option<String>` | `default`, skip-if-none | collision phase 1 key |
| 23 | `source_system` | `Option<String>` | `default`, skip-if-none | |
| 24 | `source_repo` | `Option<String>` | `default`, skip-if-none | basename of `.beads`'s parent |
| 25 | **`source_repo_path`** | `Option<String>` | `default`, skip-if-none | **★ NEW** (beads_rust#289) — absolute canonical workspace path |
| 26 | **`agent_context`** | `Option<String>` | `default`, skip-if-none | **★ NEW** (beads_rust#297) — a **JSON document in a TEXT column**, schema intentionally open |
| 27 | `deleted_at` | `Option<DateTime<Utc>>` | `default`, skip-if-none | tombstone |
| 28 | `deleted_by` | `Option<String>` | `default`, skip-if-none | tombstone |
| 29 | `delete_reason` | `Option<String>` | `default`, skip-if-none | tombstone |
| 30 | `original_type` | `Option<String>` | `default`, skip-if-none | tombstone |
| 31 | `compaction_level` | `Option<i32>` | `default`, **custom `serialize_with`** | **always emitted, `0` when `None`**, for Go-`bd` conformance |
| 32 | `compacted_at` | `Option<DateTime<Utc>>` | `default`, skip-if-none | |
| 33 | `compacted_at_commit` | `Option<String>` | `default`, skip-if-none | |
| 34 | `original_size` | `Option<i32>` | `default`, skip-if-none | |
| 35 | `sender` | `Option<String>` | `default`, skip-if-none | messaging |
| 36 | `ephemeral` | `bool` | `default`, skip-if-false | wisps |
| 37 | `pinned` | `bool` | `default`, skip-if-false | |
| 38 | `is_template` | `bool` | `default`, skip-if-false | |
| 39 | `labels` | `Vec<String>` | `default`, skip-if-empty | Org tags in the fork (colon bug, dossier R9) |
| 40 | `dependencies` | `Vec<Dependency>` | `default`, skip-if-empty | |
| 41 | `comments` | `Vec<Comment>` | `default`, skip-if-empty | |

**Delta vs fork point: `source_repo_path` and `agent_context`. That is all.** Verified by
extracting `pub` field lines from both structs and diffing **[V]**.

> `agent_context` deserves a callout for R2: it holds a **JSON document as a string**. Embedding it
> in an Org `:PROPERTIES:` drawer as a single-line value will be ugly and may exceed practical
> property-line limits. The fork already emits dependencies and comments as JSON `src` blocks
> (plan §6.2); `agent_context` probably wants the same treatment.

### 6.3 `Status` — one new variant **[V]** (`src/model/mod.rs:57-135`)

`#[serde(rename_all = "snake_case")]`, custom `Deserialize` that lowercases unknown values into
`Custom`.

| Variant | wire string | fork Org keyword (`03c5cd2c`) |
|---|---|---|
| `Open` *(default)* | `open` | `TODO` |
| `InProgress` | `in_progress` (also accepts `inprogress`) | `DOING` |
| `Blocked` | `blocked` | `WAIT` |
| `Deferred` | `deferred` | `DEFER` |
| **`Draft`** ★ | `draft` | **— NO MAPPING —** |
| `Closed` | `closed` | `DONE` |
| `Tombstone` | `tombstone` | `CANCELED` |
| `Pinned` | `pinned` | `NOTE` |
| `Custom(String)` | the string, lowercased | `s.to_uppercase()` |

Helpers: `is_terminal()` = `Closed | Tombstone`; `is_active()` = `Open | InProgress`;
`is_draft()` = `Draft` (`:117-130`).

**Action for Phase 3 step 4 [I]:** add `Draft` to `status_to_keyword` / `keyword_to_status` and to
`BEADS_TODO_KEYWORDS`. Without it, a drafted issue round-trips as `Status::Custom("draft")` — which
*happens* to preserve the wire string, but breaks `is_draft()` and every `matches!(Status::Draft)`
in the codebase. Suggested keyword: `DRAFT` (it is already an Org convention in some setups) — but
this is R2's call, and it must be added to the fork's `#+TODO:` keyword line too.

### 6.4 `Dependency`, `Comment`, `DependencyType`, `IssueType` — unchanged **[V]**

`Dependency`: `issue_id: String`, `depends_on_id: String`, `dep_type: DependencyType`
(`#[serde(rename = "type")]`), `created_at: DateTime<Utc>`, `created_by: Option<String>`,
`metadata: Option<String>`, `thread_id: Option<String>`.

`Comment`: `id: i64`, `issue_id: String`, `author: String`, `body: String`
(`#[serde(rename = "text")]`), `created_at: DateTime<Utc>`.

`DependencyType`: `Blocks`, `ParentChild`, `ConditionalBlocks`, `WaitsFor`, `Related`,
`DiscoveredFrom`, `RepliesTo`, `RelatesTo`, `Duplicates`, `Supersedes`, `CausedBy`, `Custom(String)`.

`IssueType`: `Task`, `Bug`, `Feature`, `Epic`, `Chore`, `Docs`, `Question`, `Custom(String)`.

All four are variant-for-variant identical to `e805fe98:src/model/mod.rs` **[V]**. The only serde
change is that upstream now spells the `rename` attributes explicitly in the places the fork left
implicit — no wire difference.

---

## 7. Dossier §5 R1–R8 defect audit against upstream

| Risk | Verdict | One-line reason |
|---|---|---|
| **R1** concurrent auto-flush drops writes | **FIXED** | three-layer locking + CAS publication |
| **R2** `debug!`-swallowed auto-flush errors | **FIXED** | `report_auto_flush_failure` warns on stderr / JSON |
| **R3** import destroys audit history | **FIXED** | in-place `UPDATE`, no `INSERT OR REPLACE`; regression test |
| **R4** selective import / total export divergence | **PARTIALLY FIXED** | divergence now detected and re-flushed, but LWW still overwrites resolutions |
| **R5** fixed temp filename corrupts export | **FIXED** | pid-scoped name + `create_new(true)` + retry loop |
| **R6** content-hash dedup annihilates issues | **PARTIALLY FIXED** | id-match now precedes hash-match; determinism still absent |
| **R7** 32 767 exportable-issue ceiling | **FIXED** | dirty-flag clearing is row-by-row, no `IN (?,?,…)` |
| **R8** `--db` writes into another workspace's file | **FIXED** | one `ConfigPaths` used for both DB and export path |

### 7.1 R1 — concurrent auto-flush silently drops committed writes → **FIXED [V]**

Three independent mechanisms now stand where there was nothing:

1. **Process-level database-family write authority.** `src/main.rs:141-148` acquires
   `blocking_database_family_write_lock_with_timeout` at startup for mutating commands, held across
   the whole invocation. That lock takes `<beads_dir>/.write.lock` **and** a per-database sidecar
   `.br-db-write-<24hex>.lock` **and** the one-byte inode lock (§3.2)
   (`blocking_database_family_write_lock_with_timeout`, `src/sync/mod.rs:1075`; sidecar names at
   `:991`/`:1006`; workspace lock at `:1088`).
2. **A `.sync.lock` flock specifically around auto-flush.** `src/main.rs:833` calls
   `sync::try_sync_lock(&paths.beads_dir)` (`src/sync/mod.rs:1367`); on `WouldBlock` it *reports the
   skip loudly* rather than proceeding (`main.rs:835-846`).
3. **A JSONL-family write authority plus compare-and-swap publication.** The exporter acquires
   `blocking_jsonl_family_write_lock_with_timeout(output_path)` (`src/sync/mod.rs:1010`, called at
   `:9736`), captures the target's exact previous generation witness (`:9749-9782`), and publishes
   with `renameat2` `RENAME_EXCHANGE` — then verifies that the *displaced* file matches the expected
   previous witness (`:1685-1697`). The dossier's losing interleaving (A reads S0, B publishes S1, A
   publishes S0) now aborts at the exchange with
   `"the atomically displaced JSONL generation does not match the exact retained source witness"`.

The dossier's suggested fix ("wrap the export read in `BEGIN IMMEDIATE`") was **not** taken
literally — `export_issue_ids` + `hydrate_export_issue_batch` still run as separate reads
(`:9808`, `:9922`) — but the surrounding write authority makes concurrent mutation impossible while
an export is in flight, which achieves the same end. **[I]** on that last inference; the locking is
**[V]**.

### 7.2 R2 — `run_auto_flush` swallows every error at `debug!` → **FIXED [V]**

`src/main.rs:838`, `:849`, `:871` all call `commands::report_auto_flush_failure`
(`src/cli/commands/mod.rs:86-133`), which:
- emits `tracing::warn!` with `beads_dir`, `jsonl_path`, `error`;
- unless `--quiet`, prints to **stderr**:
  `"Warning: Mutation succeeded, but automatic JSONL export failed. Fix the export problem, run
  \`br sync --flush-only\`, then commit the updated .beads/issues.jsonl. JSONL path: … Error: …"`;
- in `--json`/`--toon` mode emits a structured
  `{"warning":{"code":"AUTO_FLUSH_FAILED", …, "recovery": "…"}}` payload.

The dossier's "change `debug!` to `warn!` — one word, five failure classes made visible" was done,
and then some. **Exit code is still 0** — the mutation genuinely succeeded — which is the defensible
choice. Note the message hardcodes `.beads/issues.jsonl`; both the Org port and the rename must fix
that string.

### 7.3 R3 — routine import destroys an issue's entire audit history → **FIXED [V]**

`upsert_issue_for_import_in_tx` (`src/storage/sqlite.rs:17299-17325`) is now:

```rust
let issue_exists = match self.conn.query_row_with_params(
    "SELECT 1 FROM issues WHERE id = ? LIMIT 1", &[SqliteValue::from(issue.id.as_str())]
) { Ok(_) => true, Err(FrankenError::QueryReturnedNoRows) => false, Err(e) => return Err(e.into()) };

if issue_exists {
    let rows = self.update_issue_row_for_import(issue, &timestamps)?;   // in-place UPDATE
    if rows == 0 { return Err(/* "import update did not find existing issue" */); }
    return Ok(true);
}
Ok(self.insert_issue_row_for_import(issue, &timestamps)? > 0)
```

No `DELETE`, no `INSERT OR REPLACE`, so `ON DELETE CASCADE` never fires on `events`.

There is an explicit regression test with the defect written into its doc comment
(`src/storage/sqlite.rs:28437-28480`):
> *"Regression test for issue #263 (a): import upsert must not cascade-drop child rows. Pre-fix,
> `upsert_issue_for_import` did `DELETE FROM issues WHERE id = ?` then `INSERT`, and the child
> tables' `ON DELETE CASCADE` foreign keys swept events, labels, deps, and comments out from under
> the issue every time an import touched it."*

That is the dossier's R3, upstream's issue #263(a), fixed and pinned. There is also
`has_owned_relation_rows_for_import` (`:17337`) guarding the insert-only relation fast path against
stale rows left behind by FK-disabled bulk import.

### 7.4 R4 — import is selective, export is total → **PARTIALLY FIXED [V]**

**Still true:**
- `determine_action` (`:11880`) still returns `Skip` on `Equal timestamps` and `Existing is newer` —
  pure wall-clock LWW, no logical clock (`:11916-11928`).
- `import_from_jsonl_snapshot` still unconditionally stamps
  `storage.set_metadata_in_tx(METADATA_JSONL_CONTENT_HASH, &jsonl_hash)` **after** the skips
  (`:12639`).
- There is **no DL-2b guard**: the export safety block (`:9815-9869`) still only computes
  `jsonl_ids − db_ids`; it has no notion of "the file's version of this id is newer than mine."

**Fixed:**
- `ImportResult` **does** now carry `skipped_count`, plus `tombstone_skipped`,
  `orphan_cleaned_count`, `export_hashes_recorded` and more (`:2350-2380`) — the dossier's
  "`AutoImportResult` does not even carry `skipped_count`" no longer holds.
- The reassuring-phrasing complaint is partly addressed: `br sync` now prints a *breakdown* —
  `"Skipped: {} equal, {} older-in-JSONL, {} tombstone-protected"` (`src/cli/commands/sync.rs:3776`) —
  though the plain summary line `"  Skipped: {} issues (up-to-date)"` still exists at `:3531`.
- **The real mitigation:** upstream distinguishes a *certified* skip from an *uncertified* one.
  `export_hash_entry_for_import_action` (`:12393`) records an export hash for a skipped record
  **only if** `skipped_import_matches_stored_issue` (`:12375`) confirms the stored row is
  `sync_equals` to the incoming one. Otherwise the skip counts as an `uncertified_local_wins`, and
  at the end of the import transaction:
  ```rust
  if uncertified_local_wins > 0 {
      tracing::debug!(count = uncertified_local_wins,
          "Import preserved local records that differ from JSONL; marking database for flush");
      storage.set_metadata_in_tx("needs_flush", "true")?;   // sync/mod.rs:12496
  }
  ```
  `needs_flush = true` makes `pending_export_state` (`:10272`) report `db_newer`, which forces the
  **full** export path on the next auto-flush (`:11678` takes the `else` branch). So DB↔file
  divergence no longer persists invisibly forever.

**What still bites [I].** The repair direction is always **DB wins**. The dossier's most damaging
scenario — a human resolves a git conflict by hand, and the machine whose version *lost* silently
reverts the resolution — is not fixed; it is now *guaranteed and prompt* rather than *accidental and
eventual*. And the trigger is logged at `debug!`, so at default verbosity nothing is printed.

**Carry forward into the Phase-2 risk register as: "R4-residual — LWW import + DB-wins repair
silently reverts hand-resolved merges; the DL-2b export guard the dossier proposed still does not
exist."**

### 7.5 R5 — fixed temp filename corrupts the export → **FIXED [V]**

Three changes, any one of which would have sufficed:
1. **The name carries the pid**: `output_path.with_extension(format!("jsonl.{pid}.tmp"))`
   (`:1820`) — exactly the dossier's suggested fix.
2. **Creation is exclusive**: `OpenOptions::new().write(true).create_new(true)` with mode `0o600`
   (`:1848-1856`), and on `AlreadyExists` it retries with a different suffix up to
   `MAX_JSONL_TEMP_PATH_ATTEMPTS` (`:1831`, `:1867-1873`). A symlink at the temp path is a hard
   error, not a retry (`:1859-1864`).
3. **Publication is a CAS exchange** (§2.2), so even a hypothetical name collision cannot let
   process B rename a temp file process A already consumed — B's exchange fails with a named
   `SyncConflict` instead.

The allowlist was updated in lockstep: `is_allowed_jsonl_temp_name` (`src/sync/path.rs:546`) accepts
`<base>.jsonl.<digits>.tmp`, and `validate_temp_file_path` still requires the temp file to sit
beside the target.

### 7.6 R6 — content-hash dedup annihilates independently created issues → **PARTIALLY FIXED [V]**

**Fixed — the dossier's fix (b), first half.** The collision phases were **reordered**. At the fork
point (`e805fe98:src/sync/mod.rs:1964-1993`): Phase 1 external-ref, **Phase 2 content hash**,
**Phase 3 id**, Phase 4 new. Upstream (`:11839-11876`): Phase 1 external-ref, **Phase 2 id**,
**Phase 3 content hash**, Phase 4 new. An incoming record whose id already exists locally can no
longer be absorbed into a different id by hash. That closes the dossier's reproduction, in which
`dd-zzz99` was destroyed into `bd-aaa`.

**Also fixed.** Tombstones are excluded from the hash map entirely
(`load_import_metadata_maps:12228`), so a tombstoned row can no longer capture a live incoming one.
And the whole map is now preloaded once (`:12213`) instead of one `find_by_content_hash` query per
record.

**Still broken.**
- `find_by_content_hash` (`src/storage/sqlite.rs:17063-17079`) **still has no `ORDER BY`** — the
  dossier's fix (b), second half, was not applied.
- `idx_issues_content_hash` is **still a non-unique index** (`src/storage/schema.rs:269`,
  `:2132`: `CREATE INDEX IF NOT EXISTS idx_issues_content_hash ON issues(content_hash);`).
- The preloaded map's tie-break is *"preserve the first matching issue to mirror the old
  `query_row` collision path"* (`:12232-12236`), but the source query
  `get_all_issues_metadata` is `SELECT id, external_ref, content_hash, updated_at, status FROM issues`
  with **no `ORDER BY`** (`src/storage/sqlite.rs:7390-7391`). So "first" is scan-order-dependent, i.e.
  still not deterministic across machines.
- Content-hash dedup is **still unconditional on every import** — the dossier's fix (c) (restrict it
  to the `--rename-prefix` flow) was not taken.
- The non-confluence property therefore survives in the narrower window where two *different* ids
  carry identical content and neither id exists locally.

**Carry forward as: "R6-residual — content-hash dedup is still enabled on every import and still
resolves ties non-deterministically; only the id-precedence half was fixed."**

### 7.7 R7 — hard failure ceiling at exactly 32,767 exportable issues → **FIXED [V]**

The offending pattern is gone. `clear_dirty_issues_in_tx` (`src/storage/sqlite.rs:14284-14298`)
issues **one parameterized `DELETE` per issue**, with no `IN (?,?,…)` clause at all:

```rust
for (id, marked_at) in metadata {
    self.conn.execute_with_params(
        "DELETE FROM dirty_issues WHERE issue_id = ? AND marked_at = ?",
        &[SqliteValue::from(id.as_str()), SqliteValue::from(marked_at.as_str())])?;
}
```

The duplicate the dossier flagged, `clear_dirty_flags` (`:16687`), now simply forwards to
`clear_dirty_issue_ids_in_tx` (`:14312`), which chunks at `SQLITE_VAR_LIMIT = 900` *and* deletes
row-by-row inside each chunk, with the comment *"Delete existing entries row-by-row to avoid fsqlite
IN-clause bugs"*. So the duplicate-pair defect is resolved by delegation rather than deletion.

Note also that `clear_dirty_issues` is now **marked_at-aware** — it only clears a dirty flag whose
timestamp matches the one captured at export time, which closes a separate lost-update window the
dossier did not name.

### 7.8 R8 — `--db` plus auto-flush writes into another workspace's file → **FIXED [V]**

The dossier's mechanism was "four independent resolutions of *where is the export file*", with
`main.rs` calling `discover_beads_dir(Some(Path::new(".")))` (ignoring `--db`), then discarding the
correctly-resolved paths with `let (mut storage, _paths) = …`, then letting `auto_flush` re-derive
from `Metadata::load`.

Upstream collapses all of that to **one** resolution **[V]**:
- `StartupContext::init` (`src/main.rs:906`) calls
  `config::discover_beads_dir_with_cli(overrides)` — the `--db`-aware variant — and stores the
  resulting `config::ConfigPaths` on `ctx.paths`.
- The auto-flush block (`src/main.rs:829-876`) reads `ctx.paths.as_ref()` and passes
  `paths.beads_dir`, `paths.jsonl_path`, and `paths.db_path` (via
  `implicit_external_jsonl_allowed`) to `auto_flush`. The storage handle and the export path come
  from the same `ConfigPaths`.
- `auto_flush` (`src/sync/mod.rs:11620`) now **takes `jsonl_path: &Path` as a parameter** — it no
  longer re-derives anything from `Metadata::load`.

The `_paths`-discard antipattern is gone. Cross-workspace flush is structurally impossible on this
path.

### 7.9 Not in scope but worth recording

- **R9 (Org labels containing `:`)** is fork-only; upstream has no Org code, so it neither fixed nor
  worsened it. It carries forward verbatim into the port.
- **R28 (unvalidated redirect)** is materially different upstream — see §4.4. Re-assess in Phase 2.
- **R32 (backup mechanism inert)** — upstream's `.br_history/` is now a real, configurable,
  doctor-monitored subsystem (`src/sync/history.rs`, 1,465 lines; `sync.history_enabled` flag from
  #293; a `br_history-grows-unbounded` doctor detector, commit `71933f06`). Re-assess in Phase 2.

---

## 8. Corrections to `UPSTREAM_SYNC_AND_RENAME_PLAN.md` and the dossier

These are places where I found the plan or dossier wrong or stale. Loudly, as instructed.

1. **§3 "New upstream modules" row overstates the new-command list.**
   The plan says: *"new commands: audit, capabilities, capacity, changelog, coordination, gate,
   delete, defer, …"*. **`audit`, `changelog`, `delete`, and `defer` all existed at the fork point**
   (`e805fe98:src/cli/mod.rs` `Commands` contains `Audit`, `Changelog`, `Delete`, `Defer`) **[V]**.
   The complete and exact new-command set is **eight**: `Capabilities`, `Capacity`, `Coordination`,
   `Gate`, `RobotDocs`, `Scheduler`, `Serve`, `VcsStatus` (§5).

2. **§7.5 half-right about `witness.rs` and `db_inode_lock.rs`.**
   The plan says *"Upstream's new concurrency machinery (`witness.rs`, `db_inode_lock.rs`) may assume
   JSONL in its safety checks"*. **`db_inode_lock.rs` contains zero JSONL assumptions** — it only
   ever locks the database inode **[V]**. `witness.rs` *is* line-oriented, but it is not on any
   safety path; it serves only `br sync --witness` **[V]**. The genuine JSONL-assuming safety layer
   is **`src/sync/path.rs`** (the allowlist), the **temp-name generator** in `sync/mod.rs`, and the
   **incremental line-flush machinery** — none of which the plan names. §3.4 above is the real
   landmine map.

3. **Dossier R5's central claim is obsolete against upstream.** The fixed temp filename
   (`output_path.with_extension("jsonl.tmp")`, no pid, no nonce, no `mkstemp`) is gone; upstream has
   pid + `create_new` + retry + CAS publication (§7.5) **[V]**. Any Phase-3 work that "re-applies the
   fork's temp-file fix" would be re-fixing a fixed bug — but note the *inverse* hazard: upstream's
   fix hardcodes `jsonl` in the temp extension, so the Org port must re-introduce the fork's
   `is_org_format` branch **inside** upstream's new function (§2.3).

4. **Dossier R4's "`AutoImportResult` does not even carry `skipped_count`" is stale.** Upstream's
   `ImportResult` carries `skipped_count`, `tombstone_skipped`, and eight more counters
   (`src/sync/mod.rs:2350-2380`) **[V]**. The structural LWW problem remains; the observability
   complaint does not.

5. **Dossier R6's "make `find_by_content_hash` deterministic … and never dedup when
   `id_exists(incoming.id)`" is half-done, and the *done* half is not where the dossier looked.**
   Upstream did not add `ORDER BY` to `find_by_content_hash` (still absent,
   `src/storage/sqlite.rs:17063`) **[V]**; it instead reordered the collision phases so id-match
   precedes hash-match (`:11839` vs `e805fe98:src/sync/mod.rs:1975-1992`) **[V]**. Anyone auditing
   only `find_by_content_hash` would wrongly conclude R6 is untouched.

6. **§6.1 of the plan says format selection is "chosen per-file by extension".** True of the fork,
   but it is worth stating explicitly for the executor: **upstream has no such mechanism to modify —
   it must be built** (§2.6, zero `Format` abstraction, 17 distinct (de)serialization sites) **[V]**.

7. **The `CHANGELOG.md` is not a substitute for `git log`.** The plan's R1 brief says "read
   CHANGELOG.md, cluster the 1,843 commits". The changelog has **no section** for v0.1.34–v0.1.45,
   v0.2.0–v0.2.7, v0.2.12–v0.2.18, or v0.2.20 — which is where roughly **1,000 of the 1,843 commits
   live** (§1.1, §1.2) **[V]**. Do not treat its silence as "nothing happened."

8. **Plan §3 "Upstream growth: 108 src `.rs` files" — confirmed exactly** (`find src -name '*.rs' | wc -l`
   → 108) **[V]**. `src/sync/mod.rs` 21,609 and `src/storage/sqlite.rs` 34,865 also confirmed **[V]**.
   No correction; recording the verification.

9. **Plan §6.2 mentions `:BEADS_SCHEMA_VERSION:` in the Org drawer.** Whatever value the fork
   emitted, the correct value on the new base is **17**, not 1 (`src/storage/schema.rs:11` vs
   `e805fe98:src/storage/schema.rs:5`) **[V]**. The migration ladder from 1 to 17 also means a fork
   workspace's `beads.db` will be migrated on first open by the new binary — worth a Phase-5 note.

---

## 9. Things the integration planner must decide (surfaced, not decided)

1. **Incremental auto-flush and Org (§2.5).** Bypass for `.org` (recommended), or make the
   record-substitution machinery format-aware? This is the single largest new integration decision
   the fork never faced.

2. **`br sync --witness` on an Org export (§3.1).** Reject with a clear error (recommended), or
   define an Org-heading-boundary chunking? The witness schema string is
   `"br.jsonl-witness.v1"` — an Org variant would need its own version tag.

3. **Merge artifacts stay JSONL? (§3.4g).** `beads.base.jsonl`, `beads.left/right.jsonl`,
   `sync_base.jsonl`, `deletions.jsonl`, `interactions.jsonl` are internal, never hand-edited, and
   `br sync --merge` plus a whole doctor family depend on their names and format. Keeping them JSONL
   inside an Org workspace is defensible and much cheaper — but it means the workspace is
   *mixed-format*, which contradicts a naive reading of "the fork's reason to exist".

4. **`metadata.json`'s `jsonl_export` semantics under a changed default (§4.3).** Flipping
   `DEFAULT_JSONL_FILENAME` to `issues.org` reclassifies every existing `"jsonl_export":
   "issues.jsonl"` as an *explicit override*. Intended (legacy workspaces keep working) or a
   migration hazard? Needs a test either way.

5. **`BEADS_JSONL` / `--allow-external-jsonl` and `.org` (§3.4c).** Should an external Org export
   path be permitted at all? `validate_external_jsonl_path` currently hard-rejects it.

6. **`Draft` status → which Org keyword (§6.3).** `DRAFT`? `SOMEDAY`? It must go into
   `BEADS_TODO_KEYWORDS` and the emitted `#+TODO:` line, and the choice is user-visible.

7. **`agent_context` representation (§6.2).** A JSON document in a property-drawer value is
   unpleasant; a JSON `src` block matches how the fork already handles dependencies and comments.

8. **Test-estate policy at scale (§3.4f).** 160 test files and ~15 doctor fixture shell scripts
   hardcode `issues.jsonl`. Phase 3 needs a *rule*, not per-file judgement: e.g. "every existing
   test pins JSONL explicitly via `metadata.json`; a new parallel Org suite covers the default
   path", or "convert the sync/e2e core to Org and pin the rest".

9. **Whether to fix the two residual defects (R4-residual, R6-residual) during the port.** Both are
   upstream's bugs now, not the fork's. Fixing them in the port branch means carrying local deltas
   in the hottest file; not fixing them means shipping known data-loss paths. This is a genuine
   maintainer decision.

---

## 10. Verification log

Everything below was run by me against the read-only worktree or via `git show` on the fork repo.
No build, no test, no write outside this file.

| What | Command / read | Result |
|---|---|---|
| Upstream HEAD | `git log -1 --format='%H %ad %s'` | `d1fb0d3a … 0.2.22` |
| File sizes | `wc -l src/sync/mod.rs src/storage/sqlite.rs src/config/mod.rs src/model/mod.rs CHANGELOG.md` | 21609 / 34865 / 9513 / 2020 / 974 |
| `.rs` file count | `find src -name '*.rs' \| wc -l` | 108 |
| Org presence | `grep -rn "org_bridge\|is_org_format\|orgize\|org2jsonl" src/` | 0 hits |
| Format enums | `grep -rn "enum [A-Za-z]*Format" src/` | 3 (all output-related) |
| Default filename | `grep -rn "DEFAULT_JSONL_FILENAME" src/` | 14 hits, definition at `config/mod.rs:62` |
| `.jsonl` in `src/` | `grep -rn '\.jsonl' src/ \| wc -l` | 1310 |
| `"issues.jsonl"` in `src/` | `grep -rn '"issues\.jsonl"' src/ \| wc -l` | 396 |
| `issues.jsonl` in `tests/` | `grep -rln … \| wc -l`; `grep -rn … \| wc -l` | 160 files / 621 occurrences |
| Private allowlist copies | `grep -rn "ALLOWED_EXTENSIONS" tests/` | 1 file: `e2e_sync_git_safety.rs:1275` |
| `cache.rs` callers | `grep -rn "crate::cache\|cache::" src/` (excl. self) | 0 |
| `witness.rs` callers | `grep -rn "witness::" src/` (excl. self) | 1: `cli/commands/sync.rs:13` |
| Commands enum diff | `awk` extraction + `sort -u` on both trees | 39 → 47, 8 new |
| Issue field diff | `awk` extraction of `pub` lines, both trees | +2 fields |
| Model sub-type diff | same for `Dependency`, `Comment`, `Status`, `DependencyType`, `IssueType` | +1 `Status::Draft`, else identical |
| Commit clustering | `git log --format=%s e805fe98..upstream/main \| sed -E … \| sort \| uniq -c` | table in §1.1 |
| Per-span counts | `git rev-list --count <tag>..<tag>` over 18 spans | table in §1.2 |
| Env var inventory | `grep -rhoE '"(BEADS\|BD\|BR)_[A-Z0-9_]+"' src/ \| sort -u` | 27 names |
| Schema version | `src/storage/schema.rs:11` vs `e805fe98:src/storage/schema.rs:5` | 17 vs 1 |
| Fork keyword map | `git show 03c5cd2c` | TODO/DOING/WAIT/DEFER/DONE/CANCELED/NOTE |
