# obr — Deep Research Dossier

**Subject:** `/Users/johnw/src/obr` — crate `beads_rust`, binary `obr` (v0.1.14), a Rust port of
Steve Yegge's Go issue tracker *beads* (`bd`).
**Date:** 2026-08-06. **Synthesized from:** 7 subsystem maps, 5 web-research briefs,
5 adversarial analyses, 1 completeness critique, 4 gap-fill investigations (index in §8).
**Method:** every nontrivial claim below is cited `path:line` against the repo, which was treated
as strictly read-only. Contradictions between contributing agents were re-verified by the
synthesizer against source and against the installed `obr 0.1.14` binary in throwaway
workspaces; those verifications are marked **[SYNTH-VERIFIED]** and detailed in §9.
**Revision 2 (gap-fill round):** a completeness critic found four material gaps; four gap-fill
agents investigated them first-hand. Their results are integrated into the relevant sections
below and marked **[ADDENDUM]**; new risks appear as R28–R32; the round itself is summarized in
§10. Where an addendum contradicts an earlier claim, the earlier claim is struck in place rather
than deleted, so the correction is auditable.

---

## 1. Executive summary

obr is a single-binary, daemon-less, agent-first issue tracker. Issues live in SQLite for query
speed and in a git-tracked flat file for sync. It is a deliberate *fork-in-amber* of the
"classic" beads architecture — SQLite + flat-file hybrid, content-addressed SHA-256 dedup, a
4-step mutation protocol, and a hard rule that the tool never runs git on your behalf — captured
at the moment upstream abandoned that architecture for Dolt (upstream v0.50.0 made Dolt default;
v0.58.0 deleted the SQLite layer entirely; `web-beads-upstream.md`). The project was
specification-first: 7,937 lines of reverse-engineered Go spec were written before any Rust, and
the first eleven commits are documentation (`map-docs.md`).

**The engineering is genuinely good in the places its author thought hardest about, and
systematically weak in the places between subsystems.** SQLite usage is correct — `BEGIN
IMMEDIATE` on every write, a single-commit 4-step protocol, a version-gated open that keeps read
commands lock-free, an atomic claim guard that closes a real two-agent TOCTOU. The sync *safety*
layer — data-loss guards, a never-bypassable conflict-marker scan, a `.git` path rejection that
runs before every other check and that no flag can open — is the best-designed part of the
project, and it exists because `bd sync` destroyed this very repository's source tree on
2026-01-19 (`.beads/SYNC_THREAT_MODEL.md:5`; commits `b8020fa`, `a326295`).

**But almost nothing that matters is protected end-to-end.** The defects cluster in three
places:

1. **The seam between SQLite and the file.** The DB write is transactional; the file write is
   not, and nothing serializes it. Concurrent auto-flush silently drops committed writes from
   the git-tracked file (reproduced 5/60 rounds with *two* issues). The export temp file has a
   fixed name (`src/sync/mod.rs:1421-1426`), so 12 concurrent `obr create` calls produced a
   truncated JSON fragment plus duplicate ids and bricked the workspace. Import is selective
   (may Skip) while export is total, and import always stamps the content hash of the file it
   read — so any skip becomes permanent divergence the staleness check cannot see.

2. **Whole-record replace as the merge primitive.** The wire unit is a full issue snapshot, so
   `upsert_issue_for_import` is `INSERT OR REPLACE` (`src/storage/sqlite.rs:3687-3698`), which
   is a row delete + insert, which cascades. **[SYNTH-VERIFIED]** a plain `obr list` — a *read*
   command, via auto-import — destroyed an issue's entire event/audit history (3 → 0 events),
   unrecoverably, because events are never exported. Conflict resolution is wall-clock
   last-write-wins with no logical clock anywhere, which is documented data loss in the
   distributed-systems literature and which here silently reverts a human's hand-resolved git
   conflict whenever the kept version's `updated_at` is older.

3. **Failures are invisible.** `run_auto_flush` swallows every error at `debug!` with an explicit
   "shouldn't break the command" comment (`src/main.rs:283-296`). At default verbosity a
   mutating command whose export failed prints success and exits 0. Since the export guards are
   *sticky*, an invisible refusal becomes unbounded silent divergence. This single line is what
   converts three separate recoverable defects into catastrophic ones.

Two structural facts amplify everything. First, **the test estate is enormous and not being
executed**: ~2,330 tests across 97 integration targets, but the only command any gate runs is
`cargo test --lib --bins`, and CI cannot build the tree at all because `org2jsonl` is a local
path dependency no workflow provisions. Four of 97 targets were executed by the assurance agent;
all four failed, including the three crown-jewel git-safety tests, which now fire the project's
own `SAFETY VIOLATION` alarm because the test keeps a private copy of an allowlist that never
learned about Org. Second, **the docs describe a different program**: the storage format migrated
JSONL → Org-mode on 2026-02-18 and the binary was renamed `br` → `obr` on 2026-02-19, and
*not one document mentions Org-mode*. The README was rewritten a month after the migration and
still describes JSONL throughout, with three reproducibly wrong claims.

**[ADDENDUM] A fourth cluster, found in the gap-fill round: the safety layer's *anchor* is
unguarded, and the one command outside it has none.** `src/sync/path.rs` validates paths
*relative to `beads_dir`* — and `beads_dir` is produced by `routing::follow_redirects`, which
reads a git-committable `.beads/redirect` file, accepts any absolute path, `..`, or symlink, and
applies no validation whatsoever (`src/config/routing.rs:200-232`; R28). A plain `obr create`
from a directory containing nothing but `.beads/redirect` was observed writing a SQLite database
**inside a `.git` directory**, exit 0. Separately, `obr agents` (1,054 lines, zero citations
before this round) walks up to three parent directories, routinely leaving the repository,
follows symlinks, and whole-file-rewrites `AGENTS.md`/`CLAUDE.md` with none of the sync layer's
guards (R29) — and injects a 2,076-byte blurb in which **19 of 19 command examples name a binary
that does not exist**. And the recovery mechanism the dossier's first revision did not know about
is **inert on the path every agent mutation takes**: `run_auto_flush` discovers a *relative*
`./.beads`, and the backup call site is gated on an absolute-path `starts_with`, so
`.br_history` is never written by auto-flush at all (R32). 244 concurrent `obr create` calls
produced zero backups.

The highest-leverage repairs are small and mostly one-line: change one `debug!` to `warn!`; wrap
the export read in the transaction it already has a connection for; give the temp file a unique
name; move the cycle check inside the transaction it sits three lines above; add `'waits-for'`
to one SQL string; **canonicalize `beads_dir` in `run_auto_flush` so backups exist at all**. The
deeper questions — whether whole-record snapshots can ever merge correctly, whether wall-clock
LWW is salvageable, whether obr should track upstream's returning SQLite seam — are laid out in
§7.

---

## 2. Architecture — how the system actually works

### 2.1 Shape

```
      CLI (clap derive, 39 subcommands, flat dispatch)     src/main.rs, src/cli/mod.rs
                     │
     ┌───────────────┼──────────────────┐
     │               │                  │
  auto-import    command body       auto-flush          (3 separate SQLite opens per mutation)
     │               │                  │
     └──────► SqliteStorage ◄───────────┘               src/storage/sqlite.rs (5157 lines)
                     │                                   11 tables, 26 indices, 0 triggers
                     │  mutate() = 4-step protocol
                     ▼
              .beads/beads.db  (gitignored — NOT the durable artifact)
                     ▲
                     │  export (total) / import (selective)   src/sync/mod.rs (5139 lines)
                     ▼
              .beads/issues.org  (git-tracked — THE durable artifact)
```

Total `src/` is ~53,500 lines against `docs/ARCHITECTURE.md:38`'s claim of "~33k". Two god
modules dominate: `src/storage/sqlite.rs` (5,157 lines) and `src/sync/mod.rs` (5,139 lines).

### 2.2 Storage

**Schema.** One `SCHEMA_SQL` const (`src/storage/schema.rs:9-194`) applied with `execute_batch`;
every statement is `IF NOT EXISTS`, so it is idempotent. Eleven tables: `issues` (36 columns),
`dependencies`, `labels`, `comments`, `events`, `config`, `metadata`, `dirty_issues`,
`export_hashes`, `blocked_issues_cache`, `child_counters`. Three CHECK constraints only —
`length(title) <= 500` (`:16`), `priority 0..4` (`:22`), and a bidirectional closed-at invariant
(`:50-55`). All child tables FK to `issues(id) ON DELETE CASCADE` *except*
`dependencies.depends_on_id`, whose FK is deliberately omitted so `external:proj:cap` pseudo-IDs
can be referenced (`:99`). **There are zero SQL triggers in the repo** — all bookkeeping is done
in Rust inside `mutate()`.

`child_counters` is created, FK'd, and reported by `doctor` and `info`, but is never inserted
into or read; `next_child_number` does a `LIKE '<parent>.%'` scan instead
(`src/storage/sqlite.rs:2504-2534`).

**Migrations.** `CURRENT_SCHEMA_VERSION = 1` (`schema.rs:5`) and has never been bumped.
`apply_schema` runs pre-migrations (ALTER TABLE ADD COLUMN from four declarative column tables,
plus an unconditional `DROP INDEX idx_issues_ready` so a partial index can evolve) → the DDL
batch → post-migrations → pragmas → `user_version = 1`. **`open_with_timeout` calls
`apply_schema` only when `user_version < 1`** (`src/storage/sqlite.rs:105-109`). This is
deliberate and good: read commands never take a write lock, with a dedicated regression test at
`sqlite.rs:4618-4635`. It has one bad consequence, dissected in §5 and §9: the per-connection
performance pragmas live inside `apply_schema` and therefore apply exactly once per database
lifetime.

Because the gate is `user_version < CURRENT_SCHEMA_VERSION` and the constant is frozen at 1,
adding a column to `ISSUE_COLUMNS` without bumping it is a **silent no-op on every existing
database**.

**The 4-step mutation protocol** (`src/storage/sqlite.rs:148-195`) is the project's flagship
invariant:

```rust
pub fn mutate<F, R>(&mut self, op: &str, actor: &str, f: F) -> Result<R>
where F: FnOnce(&Transaction, &mut MutationContext) -> Result<R>
```

1. **Apply the change** — the caller's closure runs against the `Transaction` (`:157`).
2. **Write an audit event** — buffered via `ctx.record_event` / `record_field_change`, flushed
   at `:159-174`.
3. **Mark issues dirty** — `ctx.mark_dirty`, flushed as `INSERT OR REPLACE INTO dirty_issues`
   at `:176-182`.
4. **Invalidate the blocked cache** — if `ctx.invalidate_cache()` was called, the cache is
   fully rebuilt in-transaction at `:184-190`.

All four steps plus the caller's work land in **one** `TransactionBehavior::Immediate`
transaction with a single `commit()` at `:192`. `IMMEDIATE` is the correct choice for
multi-process writers: it takes the write lock at `BEGIN`, so the read-then-write lock upgrade —
which bypasses the busy handler and yields instant `SQLITE_BUSY` — never occurs.

There are exactly **14 `mutate()` call sites**, all inside `sqlite.rs`. The protocol is enforced
only by the privacy of the `conn` field (`sqlite.rs:17-19`) — i.e. by the module boundary — and
that module is one 5,157-line file containing **12 direct `self.conn.execute(...)` write paths
that bypass it** (`:2656, :2841, :2851, :2886, :2924, :2993, :3452, :3463, :3687, :3754, :3779,
:3813`).

The import path bypasses `mutate()` **deliberately** — `upsert_issue_for_import`'s doc says
"This does NOT trigger dirty tracking or events" (`sqlite.rs:3670`) — because otherwise every
import would re-dirty every row and cause export ping-pong. That reasoning is sound; the
consequences are not (§5.2).

**Dirty tracking is two-level:** a `dirty_issues` queue (written only by mutate step 3) plus an
`export_hashes` content-hash filter in `get_issues_needing_export` (`sqlite.rs:2936-2967`). The
second level — the incremental-export machinery — **has no callers anywhere**. Every export is a
full export.

**Blocked cache.** `rebuild_blocked_cache_impl` (`sqlite.rs:1341-1466`) deletes the whole cache,
computes direct blockers over `('blocks','conditional-blocks','waits-for')` excluding
closed/tombstone, treats a missing non-external blocker as blocking, stores JSON arrays of
`"<id>:<status>"`, then propagates `parent-child` blocking to a fixpoint capped at
`MAX_DEPTH = 50`. This full rebuild inside every mutation looks alarming but is exactly what
upstream bd does, and it was measured at 33/102/312 ms at 1k/10k/50k issues — linear in |E| and
not a bottleneck. **Keep it.**

Note there are **two different definitions of "blocked"** in the codebase.
`get_blocked_by_blocks_deps_only` (`sqlite.rs:1254-1262`) writes LEFT JOINs whose WHERE predicate
is false for NULL, making them effectively INNER JOINs, so orphan and external blockers are
excluded — the opposite of `rebuild_blocked_cache_impl`, which counts orphans as blocking.

**IDs.** Format `<prefix>-<base36 hash>[.N.M]`. `IdConfig` defaults prefix `bd`, min 3, max 8,
`max_collision_prob` 0.25 (`src/util/id.rs:11-31`). `optimal_length` uses the birthday
approximation `1 - e^(-n²/2·36^len)` and returns the first length under the threshold
(`:78-92`); `compute_id_hash` SHA-256s a seed (which includes `created_at` nanos, so IDs are
*not* content-derived), folds the first 8 bytes to a u64, base36-encodes. The collision ladder
(`:113-163`) tries nonces 0..10, then increments length to max, then a fixed 12-char hash with
unbounded nonce, then a desperate `{prefix}-{hash}{nonce}` fallback. The residual check-then-
insert TOCTOU has a **safe** failure mode: `id TEXT PRIMARY KEY` turns a genuine collision into a
loud constraint violation inside the transaction.

`IdResolver::resolve` (`:601-676`) is four-step — lowercase-normalize, exact match, prepend
default prefix if no dash, substring match on the base hash — with `AmbiguousId` on ≥2 matches.

### 2.3 Sync

**Format.** The default export file in *this fork* is **Org-mode, not JSONL**:
`DEFAULT_JSONL_FILENAME = "issues.org"` (`src/config/mod.rs:33`). Format is chosen per-call by
`path.extension() == "org"` (`src/sync/mod.rs:1139, 1414, 1976, 2214`). JSONL lines are literally
`serde_json::to_string(&Issue)`, so **the `#[serde]` attributes on `Issue` *are* the git-tracked
wire format** — any field rename is a breaking change. Encoding is sparse
(`skip_serializing_if`), with one exception: `compaction_level` uses a custom serializer that
always emits an i32, coercing `None` → 0, because bd's Go SQL scanner cannot handle NULL for
integer columns (`src/model/mod.rs:22-29, 473-475`). `content_hash` is `#[serde(skip)]`
(`:374-375`) and is **never on disk** — a fact with large downstream consequences (§5.4).

Org emission/parse is `src/sync/org_bridge.rs` (~900 lines): status ↔ TODO keyword
(`TODO|DOING|WAIT|DEFER|DONE|CANCELED|NOTE`, `:262-273`), priority ↔ `[#A]`..`[#E]` (`:290-298`),
labels ↔ Org tags, dependencies and comments as JSON src blocks. It hardens against
property-drawer injection (`sanitize_property_value :241` strips `\n\r` and neuters `:END:`;
`sanitize_org_text :248` comma-escapes leading `*`). `:ID:` is the only required property.

**[ADDENDUM] The Org *parse* half is not obr's code.** `org_text_to_issues`
(`org_bridge.rs:335-355`) is ~20 lines of dispatch over
`org2jsonl::org_to_json::org_to_entries_with_keywords` — a 2,274-line converter in an
unpublished sibling crate (`Cargo.toml:64`, `org2jsonl = { path = "../org2jsonl" }`) which is
itself a CST wrapper over **`orgize 0.10.0-alpha.10`**, a pre-1.0 alpha. So the durable artifact
of this project is parsed by an alpha crate, wrapped by an unversioned path dependency, behind a
`flake.nix` input pinned to `git+file:///Users/johnw/src/org2jsonl` — a URL that resolves on
exactly one machine, at a rev **15 commits behind that machine's own worktree** (R30). obr uses
**one function and five types** from the crate; it never calls the writer (`json_to_org.rs`,
2,231 lines), because obr emits Org itself in `emit_issue_heading` (`org_bridge.rs:45-237`).
Crucially, `org_to_entries_with_keywords` hard-codes `file: None`
(`org2jsonl/src/org_to_json.rs:74`), and location metadata is only populated when `file` is
`Some` — so obr **structurally cannot** report a line number for a malformed heading (R31).

**Export protocol** (`export_to_jsonl_with_policy`, `src/sync/mod.rs:1259-1560`), in order:
path validation → history backup (`:1285`) → read all issues (`:1289`) → empty-DB guard
(`:1296`) → stale-DB guard (`:1306`) → batch-hydrate deps/labels/comments (`:1348-1381`) →
temp file in the same directory + PC-4 validation (`:1414-1427`) → expired-tombstone filter
(`:1444`) → write → `flush + sync_all` (`:1508-1512`) → re-validate overwrite path (`:1515`) →
atomic `fs::rename` (`:1530`) → `chmod 0600` (`:1533`) → count verification (`:1545`).
`finalize_export` (`:1861-1889`) is a *separate* call that clears dirty flags, sets export
hashes, and stamps `jsonl_content_hash` + `last_export_time`.

Note what is *not* there: **the function takes `&SqliteStorage` and opens no transaction.**

**Import protocol** (`import_from_jsonl`, `src/sync/mod.rs:2201-2492`): path validation →
conflict-marker scan (never bypassable, even by `--force`) → parse the whole file into memory →
normalize (recompute `content_hash`, wisp detection, `closed_at` repair) → validate → prefix
guard / renumber → 4-phase collision detection → remap ids and both ends of every dependency →
upsert (bypassing `mutate()`) → rebuild blocked cache → write metadata **last**.

**Dedup and conflict rules.** The content hash is SHA-256 over 15 NUL-separated fields in fixed
order (`src/util/hash.rs:62-96`): title, description, design, acceptance_criteria, notes, status,
`P{priority}`, issue_type, assignee, owner, created_by, external_ref, source_system, pinned,
is_template. It **excludes** id, all timestamps, labels, dependencies, comments, and tombstone
fields. Collision detection (`detect_collision`, `:2050-2087`) has strict precedence:

1. `external_ref` match → 2. `content_hash` match → 3. `id` match → 4. NewIssue.

`determine_action` (`:2089-2139`) then decides: **tombstone veto first** (`:2098-2103`, which
outranks even `--force`), then `force_upsert`, then last-write-wins on `updated_at` with
`Ordering::Equal → Skip`. Ties go to the local DB.

**Three-way merge** (`three_way_merge :2868`, `merge_issue :2691-2853`) is an eight-case decision
table over `(base, left, right)`. Case 6 (all three present) branches on `content_hash`
equality. It is structurally non-functional — see §5.4.

**Auto-hooks.** Auto-import runs before ~26 of 39 commands (`src/main.rs:32-36`, gate
`:160-210`); auto-flush runs after every mutating command (`:133-135`, gate `:139-158`). A
mutating command therefore opens SQLite **three times**.

**Git safety** is layered:
- *NGI-1/NGI-2* (no git commands / no git libraries in sync) are enforced only by **tests** —
  a `grep -r "Command::new.*git" src/sync/` meta-test that **silently passes if grep is missing**
  (`src/validation/mod.rs:846-871`), and a `Cargo.toml` `[dependencies]` scan.
  `assert_no_git_in_sync` (`:411-417`) is an empty `const fn`.
- *NGI-3* is enforced at **runtime**: `validate_no_git_path` (`src/sync/path.rs:137-175`) rejects
  any `.git` path component, its string forms, and the *canonicalized* path and parent (catching
  symlinks). It is called **first** in `validate_sync_path` (`:211`), and — critically — in
  `validate_sync_path_with_external` **before** the `allow_external` early return (`:450-458`),
  so `--allow-external-jsonl` cannot reach `.git/`.
- The write allowlist (`path.rs:47-59`) is exhaustive: extensions `db, db-wal, db-shm, jsonl,
  jsonl.tmp, org, org.tmp`; exact names `.manifest.json, metadata.json`.

**[SYNTH-VERIFIED]** five commands *outside* sync do shell out to git, all read-only:
`comments.rs:352` (`git config --get user.name`), `orphans.rs:234,245`, `changelog.rs:290`,
`stats.rs:381`. The enforced invariant is narrower than the prose claim (§6).

**[ADDENDUM] The layer is anchored on an unvalidated input, and three write paths sit outside
it entirely.** Everything above is expressed *relative to `beads_dir`*: `validate_sync_path`
canonicalizes `beads_dir` (`path.rs:238`) and asks whether the target is under it (`:326`).
`beads_dir` itself comes from `config::discover_beads_dir` → `routing::follow_redirects`, which
applies no policy at all (R28). Three further write paths never consult `src/sync/path.rs`:
- **SQLite open.** `config::open_storage` → `SqliteStorage::open_with_timeout(&paths.db_path, …)`
  (`config/mod.rs:315`, `:412`) performs **no path validation whatsoever**. A redirect into a
  `.git` directory produced a 229,376-byte `beads.db` there while the *Org* export was correctly
  rejected — split verdict, and the rejection was swallowed by R2 (R28).
- **`obr agents`.** Imports nothing from `crate::sync` (`agents.rs:6-12`); writes `AGENTS.md`,
  `CLAUDE.md` and `*.md.bak` at arbitrary ancestor paths, including inside `.git/` (R29).
- **`obr history restore` / `history diff`.** `history_dir.join(filename)` with an unsanitized
  user string (`history.rs:240`, `:137`) — absolute paths replace, `../` escapes; reproduced
  copying `/etc/passwd` and `.git/config` over `.beads/issues.org` (R32).

### 2.4 CLI, config, output

**CLI.** clap 4.5 derive, 39 `Commands` variants, one flat 39-arm match in `main.rs`. Twelve
`global = true` flags (`--db --actor --json --no-daemon --no-auto-flush --no-auto-import
--allow-stale --lock-timeout --no-db -v -q --no-color`). **`--robot` is NOT global** — it is a
per-command bool on 10 arg structs, and `--format` on 7. `should_auto_import` is an exhaustive
match with no wildcard arm (`main.rs:160-210`), so a new command forces a compile error at the
policy decision point — a nice touch.

**[ADDENDUM] Workspace discovery.** Before any of the above, every command must answer "which
`.beads`?". `discover_beads_dir(start)` (`src/config/mod.rs:204-242`) checks `BEADS_DIR`, then
walks up from the start directory looking for `.beads`, and — at all three exits (`:214`, `:220`,
`:233`) — returns `routing::follow_redirects(&candidate, 10)`. **33 of the 39 commands go
through this** (the exceptions are `Init`, `Version`, `Completions`, `Schema`, `Agents`,
`Upgrade`), as do both auto-hooks (`main.rs:219`, `:261`). `discover_beads_dir_with_cli`
(`:255-262`) *bypasses* routing when `--db` is given, calling `derive_beads_dir_from_db_path`
instead — which is why the hooks and the command body can disagree (R8, R28). The discovered
path is **relative** (`./.beads`) on the ordinary hook path, because `main.rs:219/261` pass
`Path::new(".")` — a detail that silently disables the entire backup mechanism (R32).

**Config** merges seven layers low→high (`src/config/mod.rs:815-840`): defaults → DB `config`
table → legacy user (`~/.beads`) → user (`~/.config/beads`, fallback `~/.config/bd`) → project
(`.beads/config.yaml`) → env (`BD_*` in three spellings plus four `BEADS_*` vars) → CLI. A
`ConfigLayer` is **two `HashMap<String,String>`s** (`:593-597`) partitioned into `startup` (never
readable from the DB) and `runtime`. There is **no typed Config struct anywhere**; every consumer
re-enumerates accepted spellings at its own call site. Unknown keys are accepted, echoed back by
`config list`, and ignored.

**Output.** There are **two parallel output subsystems with colliding type names**. `src/output/`
is live (used by all 38 command modules); `src/format/{context,theme,rich,syntax,markdown}.rs`
(~1,927 lines) is an abandoned earlier generation defining a second `OutputContext`/`OutputMode`/
`Theme` that nothing calls — which is why issue descriptions are never markdown-rendered or
syntax-highlighted despite full implementations existing.

The live `OutputMode` has five variants — `Rich, Plain, Json, Toon, Quiet` — conflating two
orthogonal axes (encoding vs presentation intensity), with no `Csv` variant. Selection happens
**twice**: an outer context in `main.rs:19`, then each command re-derives a local one from
`resolve_output_format*`. Precedence is `--json`/`--robot` > `--format` > `BR_OUTPUT_FORMAT` >
`TOON_DEFAULT_FORMAT` > text.

**Errors** are three layers: `BeadsError` (thiserror, 31 variants) → `StructuredError`
`{code, message, hint, retryable, context}` → `ErrorCode` (33 SCREAMING_SNAKE codes) with
`exit_code()` grouping 1–8. The layering is *justified* — a wire-stable code decoupled from Rust
variant names lets variants be renamed without breaking agents. Delivery is a single funnel,
`main::handle_error` (`src/main.rs:304-323`), which emits JSON on stderr whenever
`json_mode || !stdout().is_terminal()` — genuinely ahead of clig.dev.

Below that funnel, though, the layer is largely decorative: **all seven "smart"
`StructuredError` constructors have zero production callers**, so the advertised Levenshtein
"did you mean" never runs; four of seven `generate_hint` arms are shadowed by `err.suggestion()`
and are unreachable; five `ErrorCode`s (including `CONFLICT_MARKERS` and `PATH_TRAVERSAL`) are
unreachable because 59 `BeadsError::Config(String)` sites collapse the entire sync-safety layer
into one opaque code.

---

## 3. The design in context

### 3.1 Comparison table

| Dimension | **obr** | **upstream beads ≥ v0.58** | **git-bug** | **Fossil** | **CRDT ideal (git-appraise / Radicle / jj)** |
|---|---|---|---|---|---|
| Source of truth | Flat file in working tree (`.beads/issues.org`) | Dolt DB over `refs/dolt/data`; JSONL demoted to "an export" | Git objects under `refs/bugs/<id>` | Ticket-change *artifacts* in the Fossil repo | Git objects under side refs (`refs/cobs/*`, git-notes) |
| Query index | SQLite, gitignored, rebuilt by import | Dolt (is the store) | Disposable `.gob` excerpts + Bleve, rebuilt on `formatVersion` mismatch | SQL `TICKET` table, rebuilt by replay on schema change | Materialized view, `from_history` on ref change |
| Wire unit | **Whole-issue snapshot** | Row-level Dolt diffs | **Operation** (SetTitle, AddComment) in an OperationPack | **Amending artifact** omitting unchanged fields | One record/op per semantic change |
| Merge granularity | **Record-level replace** | Cell-level | Op-level, DAG-ordered | Field-level replay | Field-level / set-union |
| Ordering | **Wall-clock `updated_at`, LWW, ties→local** | Dolt merge + gates | **Lamport clocks** in tree entry names; explicit "you can't rely on other people's time" | Wall clock (documented as a known weakness) | Lamport/vector + deterministic tiebreak |
| Concurrent-edit outcome | Textual git conflict (see below), then LWW on resolve | Dolt conflict rows | DAG merge; causality-violating commits **discarded** | Replay in timestamp order | Conflict *representable*, queryable, idempotent under re-sync |
| Identity | SHA-256 of `created_at`-seeded blob → base36 short id | UUID-derived adaptive ids | `hash(json(op))` + **mandatory random nonce** | Random 40-hex | Hash of creating record |
| Dedup | **SHA-256 over 15 content fields**, non-unique index | `external_ref` + id | none (nonce prevents it) | none | Textual dedup via canonical serialization |
| Daemon / hooks | **None, by design** | Daemon + auto-sync (upstream's most-complained-about feature) | None | Server process | None |
| Runs git itself | **Never in sync**; 5 read-only commands elsewhere | Yes (`bd sync` does `git commit`) | Yes (it *is* git) | N/A | Yes |
| Human web view | None | None | `git bug webui` | **Built-in, permission-separated** | Radicle web UI |
| Status | Fork-in-amber, one maintainer + one prior author | 26.1k★, ~10,340 commits, v1.1.2 | 9,964★, alive | Alive, in production by its host project | Mixed; git-appraise dormant, SIT and ticgit-2014 dead |

### 3.2 Where obr sits relative to best practice

**Convergent and correct.** The SQLite-as-derived-index pattern is independently validated four
times over: Radicle materializes SQLite from `refs/cobs/*`; ticgit's 2026 Rust revival uses
`refs/meta/*` + `.git/git-meta.sqlite`; Fossil rebuilds its `TICKET` table by replay; git-bug
rebuilds excerpts and its Bleve index on version mismatch. obr differs only in using a
working-tree file rather than side refs as truth — which buys diffability and PR review at the
cost of colliding with source-code merges.

`BEGIN IMMEDIATE` is exactly right, and obr adopted it *before* the norms research that
recommends it. So is refusing a daemon: upstream's daemon, auto-sync, and global
`~/.beads/default.db` fallback are the three most-cited field complaints, and obr omits all
three. Upstream's own docs supply the strongest available justification for obr's existence:
gates exist in beads *because* Dolt lost what JSONL gave free — "With file-based storage (JSONL),
issue updates land atomically with code in the same commit. With Dolt, they don't."

**Divergent and questionable.**

*Whole-record snapshots.* Every surviving git-native tracker decomposes below record level, and
each converged there independently: Fossil omits unchanged fields from amending artifacts;
ticgit-2026 uses typed key paths per field; git-issue puts each attribute in its own file;
git-bug emits one operation per semantic change; Kubernetes has `patchMergeKey`/`patchStrategy`;
Dolt merges cells with a `CollisionFn`; SQLite's session rebaser combines resolutions "on a
per-field basis rather than per-row". obr is the outlier, and §5.2 shows the concrete price
(cascade-destroyed audit history) as well as the abstract one (concurrent edits to *different*
fields of one issue cannot both survive).

*Wall-clock LWW.* Riak's documentation is blunt — there is "no reliable definition of 'last
write'", and "any correct behavior it exhibits is more a matter of good luck than of good
design". git-bug rejected wall clocks explicitly. Fossil is the control group proving the
alternative fails: its own docs admit a timestamp off "by months or years… can seriously confuse
the replay algorithm". obr has no logical clock of any kind (`grep` for lamport/vector/hlc over
`src/` returns zero), and no upper bound on `updated_at` — one year-2030 timestamp pins an issue
permanently across every clone.

*Time-based tombstone TTL* (`src/model/mod.rs:566-586`) is Cassandra's `gc_grace_seconds` footgun
without a repair protocol or membership list. ScyllaDB moved to repair-based GC precisely to stop
resurrection. Mitigating today: `deletions_retention_days` defaults to `None`, and auto-flush
never reaps while `sync --flush-only` does — itself a determinism bug.

*Content-hash dedup* is unusual and, as implemented, actively dangerous (§5.3). git-bug had to
add a random nonce to every operation precisely *because* content-hashed ids would otherwise
collide. obr made the opposite choice — collision *is* the dedup — without deciding whether
issues, comments, and events want the same answer. They almost certainly do not.

**The strongest published objection.** SIT — a dead Rust tracker in exactly this problem space —
explicitly rejected "plain text append-only files" in favour of directory-per-record because the
latter is "far more merge-friendly (one wouldn't incur merge conflicts)". obr has no written
rebuttal. The honest rebuttal exists — canonically serialized, append-only, union-merged,
hash-deduplicated flat text is equally conflict-free *while* preserving diffability,
greppability, and reviewable PRs that hash-named directories destroy — but obr's file is not
append-only, is not field-decomposed, and is not canonically ordered per record, so the rebuttal
is currently aspirational rather than true.

**A hazard obr has so far avoided.** obr installs **no merge driver anywhere** — no
`.gitattributes`, no `merge=union` (verified by grep across `src/`, `tests/`, `docs/`,
`.beads/*.md`, `README.md`). This is correct and should stay that way. Measured behaviour of
`merge=union` on JSONL (git 2.55.0, 14 experiments): it exits 0 while producing **duplicate-id**
records when two branches edit the same record; it silently resurrects deletes; it *duplicates*
rather than dedupes identical lines; a missing trailing newline turns every append into a
last-line edit; and **one raw NUL byte anywhere** trips git's binary heuristic so union degrades
to keep-ours with a `UU` state and **no conflict markers** — a plausible mechanism for the
upstream "lost bd-100 through bd-179" incident. And it would not help anyway: GitHub explicitly
ignores user `.gitattributes` on PR merges ("we use our own `.gitattributes` file which you
can't change"), GitLab skips custom drivers in MRs, and Kubernetes removed their union driver for
exactly this reason. **The importer is the only real enforcement point**, which makes §5.6
(import accepts duplicate ids by last-line-wins) load-bearing.

---

## 4. Strengths worth defending

These are the parts a refactor must not damage. Each is verified.

1. **The sync safety layer.** The empty-DB guard (`src/sync/mod.rs:1296`), the stale-DB guard
   (`:1306`), the never-bypassable conflict-marker scan (`:1091`, called `:2221`), the runtime
   `.git` rejection that runs *first* and that `--allow-external-jsonl` cannot open
   (`src/sync/path.rs:137-175, 211, 450, 598`), the exhaustive write allowlist (`:47-59`), and
   the atomic temp+fsync+rename+chmod-0600 export. Born from a real incident; mechanically
   enforced. In the design agent's `--db` experiment the stale-DB guard **caught a data-loss
   bug from a completely different subsystem it was never written for**.

   **[ADDENDUM] — scope correction.** Everything in this item is true *of `obr sync`, and
   relative to whatever directory `beads_dir` resolves to*. It is **not** true of obr as a whole.
   The layer has an unguarded upstream input (`follow_redirects`, R28), does not cover the SQLite
   open at all (R28), and three commands write outside it (`agents`, `history restore`,
   `history diff` — R29, R32). The correct sentence is in §10.1. The engineering *inside* the
   layer remains the best in the project; the perimeter is smaller than the prose claims.

2. **`BEGIN IMMEDIATE` + one-commit `mutate()`.** `src/storage/sqlite.rs:148-195`. Correct for
   multi-process writers, and correct for the right reason. `busy_timeout` works end-to-end
   (verified: `obr create` waited 5.107 s on a held lock, then succeeded), defaulting to 30 s
   via `.or(Some(30000))` (`src/config/mod.rs:313, 382`).

3. **The atomic claim guard** (`sqlite.rs:400-426`) re-reads `assignee` *inside* the IMMEDIATE
   transaction to close a genuine two-agent TOCTOU. This is the one place the race was
   recognized and closed properly, and it is the pattern the cycle check and the content-hash
   computation should copy. Upstream converged on the same primitive (`bd update --claim`).

4. **Version-gated open** (`sqlite.rs:105-109`) makes read commands lock-free against a
   concurrent writer, with a dedicated regression test at `:4618-4635` and the reasoning
   written down. Startup floor measured at ~4–6 ms over process spawn — better than any
   daemon-less Go equivalent.

5. **Import writes metadata LAST** (`src/sync/mod.rs:2481-2486`), so a crashed import self-heals:
   SIGKILL 0.35 s into a 6,000-issue import left the DB at 614 issues, and the *next* command's
   auto-import re-ran the whole import and converged. This is a genuine crash-safety property
   and it is not written down anywhere — document it before someone "optimizes" the ordering.

6. **`empty_to_none` at exactly one boundary** (`sqlite.rs:3066-3072`) reconciles bd's
   `NOT NULL DEFAULT ''` convention with Rust `Option` in a single place, with the reason
   recorded. The right way to absorb a foreign schema.

7. **Recursive CTEs use `UNION`, not `UNION ALL`**, as an explicit cycle-termination guarantee
   with the reasoning at `sqlite.rs:1166-1168`; `detect_all_cycles` (`:3517-3595`) uses an
   explicit-stack iterative DFS to avoid stack overflow on deep graphs. Both are correct, and
   `detect_all_cycles` is *already tested and already right* — it is one call site away from
   fixing §5.5.

8. **`should_auto_import`'s exhaustive match with no wildcard** (`main.rs:160-210`): adding a
   command forces a compile error at the policy decision point.

9. **`obr schema` is generated from live types by schemars**, so it cannot drift — unlike the
   hand-mirrored `ErrorEnvelope` beside it (`src/cli/commands/schema.rs:21-38`).

10. **Strict stdout/stderr separation** (`src/logging.rs:23-24`): `obr ready --json 2>/dev/null`
    is clean parseable JSON. The JSON error envelope on stderr whenever stdout is not a TTY
    (`main.rs:309-310`) is ahead of clig.dev.

11. ~~**Destructive commands never prompt**~~ — **[ADDENDUM] withdrawn as a general claim; it
    holds only for `obr delete`.** `obr delete` does have `--dry-run`, `--force`, `--cascade`,
    is a tombstone rather than a hard delete, and there is **no `DELETE FROM issues` anywhere in
    `src/`** — all of that stands. But `grep -rn '\[y/N\]' src/` returns **four** prompt sites:
    `agents.rs:525` (`--add`), `:611` (`--remove`), `:696` (`--update`), and
    `orphans.rs:203` (`--fix`, which drives a *database* mutation per orphan). The first three
    are bypassable with `--force`; **`OrphansArgs` (`cli/mod.rs:2267-2279`) has no `--force` and
    no `--yes` at all, so `obr orphans --fix` cannot be automated.** With stdin at EOF the prompt
    prints "Aborted." and **exits 0** — indistinguishable from success; with stdin an open pipe
    (the normal agent harness shape) it **hangs indefinitely** (measured: `exit 124` under
    `timeout 5`). The prompt is written to **stdout**, unterminated and explicitly flushed, so it
    also corrupts anything the caller is parsing. See R29.

12. **Non-invasiveness is the product.** No daemon, no hooks, no automatic git, no global
    fallback DB. Given the field reports, the exclusions are the value proposition.

13. **`tests/repro_*.rs`** — 21 tests, one historical bug each. Cheap, permanent, the
    best-value layer in the repo. And **`FileTreeSnapshot`** (`tests/e2e_sync_git_safety.rs:684-721`)
    asserts on the *complement* — that only allowlisted paths changed — which fails closed on
    unknown behaviour. It is failing right now precisely because it works.
    **[ADDENDUM]** All five of its uses target `sync` (`:1000/1016`, `:1095/1115`, `:1186/1207`,
    `:1412/1427`). **The repo's strongest safety assertion has never been pointed at the one
    command that writes outside the sandbox** — pointing it at `obr agents --add --force` would
    fail immediately, since `md`/`md.bak` are absent from both the test's private allowlist and
    `src/sync/path.rs:48-56` (R29).

14. **CSV output is RFC 4180-correct** (verified by round-trip through Python's `csv`), and the
    exit-code taxonomy is genuinely asserted in `tests/e2e_errors.rs`.

15. **[ADDENDUM] Org emission is byte-deterministic.** Verified: 11 consecutive exports of a
    25-issue DB (including across separate processes) produced one MD5; a second workspace with
    10 labels, 3 dependencies and 5 comments produced one MD5 over 10 exports. Issues are
    ordered lexicographically by id, labels are explicitly sorted
    (`org_bridge.rs:65-66`), dependencies and comments are serialized from already-ordered
    vectors, properties are emitted by a hand-written `push_str` sequence (`:79-168`) rather
    than by iterating a map, and every timestamp is `to_rfc3339()`. **No `HashMap`/`HashSet`
    iteration reaches the output.** This matters more than it looks: it is the prerequisite for
    the git-tracked file not churning, and for the "derived materialized view" answer in Q1 to
    be viable at all. The file *does* churn — but from content degradation (R23), not from
    emission nondeterminism, and those have different fixes.

---

## 5. Confirmed risks and weaknesses, ranked

Ranked by (damage × reachability) / cost-to-fix. Every item is CONFIRMED unless labelled.

**[ADDENDUM] Numbering note.** R28–R32 come from the gap-fill round and are appended at the end
of this section so that existing R-numbers keep their meaning in anything that already cites
them. Their *rank* is not their number: **R29 (`obr agents` data loss) and R32 (the backup
mechanism is inert) belong between R3 and R4**, and **R28 (unvalidated redirect steers all
I/O) belongs beside R8**. Read §5 as R1, R2, R3, R29, R32, R28, R4, R5, …

---

### R1 — Concurrent auto-flush silently drops committed writes from the git-tracked file
**Severity: critical. Reproduced 5/60 rounds (~8%) with a two-issue database.**

`export_to_jsonl_with_policy` takes `storage: &SqliteStorage` (`src/sync/mod.rs:1259`) and opens
**no transaction** — verified by reading the function. It reads a full snapshot (`:1289`), runs
the guards, writes a temp file, `fs::rename`s it (`:1530`), and then `finalize_export`
(`:1861-1886`) clears the dirty flag for **every** id in `exported_ids`. SQLite serializes DB
writes; **nothing serializes the file write**, and no lock file exists anywhere in `src/`.

Losing interleaving: A reads snapshot S0 (lacks issue *b*) → B commits *b*, reads S1, renames S1,
`finalize` clears dirty for *a* **and** *b* → A renames S0 (*b* now gone from the file) → A
finalizes. End state: DB has *b*, file does not, `dirty_issues` is empty, both processes exit 0,
**permanent**.

The stale-DB guard cannot catch it: it computes `jsonl_ids − db_ids` (`:1306-1340`) and never
compares content. At 4,000 issues the integrity agent persisted 12 rounds of loss;
`obr doctor` reported all OK including "Both have 4011 records", and `obr sync --status` said
"JSONL is newer (import recommended)" — backwards.

**Why it is critical rather than annoying:** `.beads/*.db` is gitignored. The file **is** the
durable artifact. This is team-visible data loss with a green health check.

**Fix:** wrap the export read in a `BEGIN IMMEDIATE` transaction on the same connection. Cheap
at obr's scale, makes the snapshot consistent (fixing the torn-read problem for free), and makes
the rename mutually exclusive.

---

### R2 — `run_auto_flush` swallows every error at `debug!`; the command exits 0
**Severity: critical (as an amplifier). Reproduced.**

`src/main.rs:283-296`:

```rust
Err(e) => {
    // Log but don't fail - auto-flush errors shouldn't break the command
    debug!(?e, "Auto-flush failed (non-fatal)");
}
```

plus `debug!`-and-return for beads-dir discovery (`:262-267`) and storage open (`:276-280`). At
default verbosity `debug!` is not emitted, so a mutating command whose export failed prints its
success line and exits 0. Reproduced: three successive `obr create` calls each exited 0 with a
success message while `dirty_issues` climbed 3 → 4 → 5 → 6 and the file never changed.

The underlying error is well-written and never shown:
`"Refusing to export stale database that would lose issues… Hint: Run import first, or use
--force to override."`

The intent is defensible for transient failures. But **the export guards are sticky**, so an
invisible sticky refusal becomes unbounded silent divergence. This one line is what converts R1,
R3, R7, R8 and R9 from recoverable into catastrophic.

**Fix:** change `debug!` to `warn!`. One word. It makes five failure classes visible.

---

### R3 — Routine import destroys an issue's entire audit history
**Severity: critical. [SYNTH-VERIFIED] — reproduced independently by the synthesizer.**

Verified end-to-end in a throwaway workspace with the shipped binary:

```
obr init --prefix fk; obr create "FK probe issue"      → fk-1bj
obr update fk-1bj --status in_progress
obr update fk-1bj --assignee bob
sqlite3 … "select count(*) from events where issue_id='fk-1bj'"   → 3
# edit .beads/issues.org externally, bump :UPDATED_AT:
obr list                    # a READ command; auto-import fires
sqlite3 … "select count(*) from events"                           → 0
```

Three deliberate decisions stack:

1. Import bypasses `mutate()` so importing does not re-dirty rows (`sqlite.rs:3669-3671`) —
   sound reasoning.
2. `upsert_issue_for_import` implements upsert as **`INSERT OR REPLACE`**
   (`sqlite.rs:3687-3698`) — a row *replace* (DELETE + INSERT), not a field-level UPDATE.
3. `events.issue_id REFERENCES issues(id) ON DELETE CASCADE` (`schema.rs:143`), and **foreign
   keys are enforced** (§9.1).

REPLACE's conflict resolution performs the DELETE; CASCADE fires. `dirty_issues`,
`export_hashes` and `blocked_issues_cache` rows go the same way. Labels, dependencies and
comments self-heal only because `sync_{labels,dependencies,comments}_for_import` rewrite them
immediately after (`sqlite.rs:3747, 3768, 3802`). **Nothing rewrites events**, and events are
local-only and never exported (`src/storage/events.rs:8`), so the loss is unrecoverable while
`obr audit log` presents them as durable history.

Blast radius: auto-import runs before ~26 of 39 commands, so any read command executed after a
colleague's commit lands wipes the local audit trail of every touched issue, silently, exit 0.

**Fix:** `INSERT INTO issues(...) VALUES(...) ON CONFLICT(id) DO UPDATE SET ...` — a field-level
upsert. Two lines. But note the general lesson: this is the price of record-level replace as the
merge primitive (§3.2).

---

### R4 — Import is selective, export is total: the engine of silent divergence
**Severity: critical (structural).**

`import_from_jsonl` may `Skip` any subset of incoming issues via `determine_action`
(`src/sync/mod.rs:2089-2139`) but **always** finishes by stamping `jsonl_content_hash` with the
hash of the file it just read (`:2488-2489`). `export_to_jsonl_with_policy` writes **all** rows.

After any skip, DB ≠ file — but `compute_staleness` (`:1740`, hash gate `:1760-1766`) now reports
not-newer, so nothing re-imports, and the next mutating command's auto-flush overwrites the file
from the DB. Skips are logged only at `debug!` (`:2522`); `AutoImportResult` does not even carry
`skipped_count` (`:1780-1787`); and the CLI reports skips as **"Skipped: n issues (up-to-date)"**
(`src/cli/commands/sync.rs:990`) — the most reassuring possible phrasing for a discard.

Concrete consequence, the most damaging behaviour on the ordinary non-`--merge` path:

> **A human's git conflict resolution is silently reverted.** Same-issue concurrent edits always
> produce conflict markers (§5, R11), so someone resolves the file by hand. The resolved line
> carries whatever `updated_at` was kept. On the machine whose version *lost*, `determine_action`
> sees `incoming.updated_at < existing.updated_at` and returns `Skip{"Existing is newer"}`. The
> DB keeps its losing version, the hash is stamped, and the next auto-flush destroys the
> resolution. Nothing is printed.

The stale-DB guard cannot catch it: `missing = jsonl_ids − db_ids` (`:1310`) has no notion of
"the file's version of this id is newer than mine."

**Fix:** add a *DL-2b* export guard beside `:1306` refusing to overwrite a file whose version of
any id is newer than the DB's. Converts silent loss into an error.

---

### R5 — Concurrent mutations corrupt the export file and brick the workspace
**Severity: critical. Reproduced.**

`src/sync/mod.rs:1414-1426` (verified by reading):

```rust
let temp_ext = if is_org_format { "org.tmp" } else { "jsonl.tmp" };
let temp_path = output_path.with_extension(temp_ext);   // .beads/issues.jsonl.tmp
```

No pid, no nonce, no `mkstemp`, no lock. (PC-4 `validate_temp_file_path` only requires the temp
file to live beside the target — a *unique name* in that directory would satisfy it.) Combined
with auto-flush-after-every-mutation, N concurrent `obr` processes all `File::create` the same
path, interleave writes into one inode, and each renames it over the real file.

Reproduced on a 10,027-issue workspace with 12 concurrent `obr create`: a truncated JSON fragment
at line 10034 (`:"syn-p899tu","title":…` — missing its leading `{"id`), plus 5 duplicate ids.
Every subsequent command then hard-fails, because `analyze_jsonl` errors on the first bad line
(`:1165-1166`) and is called from the export guard, the count verification, **and** import:

```
$ obr list --limit 1
{"error":{"code":"CONFIG_ERROR","message":"…Invalid JSON at line 10034…"}}
```

~~The workspace is unusable until a human hand-edits the file.~~ For a tracker whose premise is
multiple concurrent coding agents, this is the most serious operational finding.

**[ADDENDUM] — the "unusable until hand-edited" claim is wrong in the common case and worse than
stated in the rare one.** The SQLite DB is untouched by this corruption; the file is derived.
Recovery on an intact workspace is **one command**, verified on a deliberately corrupted
2,245-issue file: `obr sync --flush-only --force` re-exported all 2,245 issues losslessly.
(`--force` is *required* — without it the command is a no-op, "Nothing to export (no dirty
issues)", because the dirty flags were already cleared — and nothing tells the user that.) So R5
is, on an intact workspace, **a discoverability defect**: `obr doctor` does exit 1 here and names
the parse error, but no output anywhere names the fix, and `hint` is `null`.
**But in a fresh clone** — where `.beads/*.db` is gitignored and therefore absent — the identical
error message accompanies a genuinely unrecoverable state, and *the same playbook destroys the
remaining data*: `obr sync --flush-only --force` exported **0 issues over a 5-issue file, leaving
78 bytes**, because `--force` bypasses the empty-DB guard (`src/sync/mod.rs:1298-1306`) that
exists for exactly this. Nothing in obr's output distinguishes "DB intact, file corrupt" from
"DB gone, file corrupt". The only real recovery in the clone case is
`git checkout .beads/issues.org`, which obr never suggests and, by charter, will never run.
Also verified: the *same* fixed-temp-name race is loud rather than silent on the explicit path —
10–30 concurrent `obr sync --flush-only --force` produced 21/30 failures with
`Export verification failed: expected 10 issues, JSONL has 11 lines` and bare
`I/O error: No such file or directory (os error 2)` (process B renaming a temp file A already
consumed). **The loud failure and the silent data loss are the same bug wearing different
clothes** — auto-flush swallows this identical error at `debug!` and exits 0.

**Fix:** `issues.org.<pid>.<nonce>.tmp`. One line. Plus: attach the situation-dependent
remediation to the parse error (R32 item 3).

---

### R6 — Content-hash dedup annihilates independently created issues, then wedges the exporter
**Severity: critical. Reproduced.**

The hash excludes id and all timestamps, so two issues with identical title/description/status/
priority/type/assignee/owner/**created_by** are the same issue *by definition*. `created_by` is
hashed, so distinct `--actor` protects you — but the default actor is the OS username, shared by
all agents on one machine, which is obr's primary deployment shape.

Reproduced: two clones, same actor, both file "Fix the login bug"; git merges both lines cleanly;
import reports `created_or_updated=1 skipped=1` and prints **"Skipped: 1 issues (up-to-date)"**
while `dd-zzz99` is **destroyed** (absorbed via `CollisionAction::Update{existing_id}`,
`:2500-2513`). Any external reference to it now dangles.

Worse, it is **non-confluent**: clone A converges on `bd-aaa`, clone B on `bd-bbb`, and a fresh
clone C — importing the identical file into an empty DB — keeps **both**. Three clones, three
states, one input.

Then the wedge: A's file now holds an id A's DB will never have, so the stale-DB guard refuses
**every** subsequent export. Combined with R2 this is invisible: DB grew 5 → 6 → 7 while the file
stayed at 2 and `dirty_issues` climbed 4 → 5 → 6, all exit 0. Recovery needs
`obr sync --flush-only --force`, which the user has no way to know they need.

Aggravating: `content_hash` is indexed but **not unique** (`schema.rs:67`) and
`find_by_content_hash` has **no `ORDER BY`** (`sqlite.rs:3629`), so with several matches the
target is arbitrary.

*Good news the sync map understated:* a **fresh** import does not collapse duplicates, because
Phase 1 detects collisions against pre-import state before Phase 3 writes.

**Fix, in order of ambition:** (a) report "deduplicated", not "skipped (up-to-date)", and
preserve the absorbed id into the survivor's `external_ref` — the machinery already exists at
`:2352-2354`; (b) make `find_by_content_hash` deterministic (`ORDER BY id LIMIT 1`) and never
dedup when `id_exists(incoming.id)`; (c) restrict content-hash dedup to the `--rename-prefix`
flow it was built for.

---

### R7 — Hard failure ceiling at exactly 32,767 exportable issues
**Severity: high. Bisected empirically; mechanism [SYNTH-VERIFIED].**

`finalize_export` (`src/sync/mod.rs:1866-1877`) passes **all** exported ids to
`clear_dirty_issues`, which builds one `?` per id with **no chunking** (verified by reading
`sqlite.rs:2825-2843`). Its duplicate `clear_dirty_flags` (`:3442-3460`) has the identical
defect. **Five other call sites in the same file correctly chunk at `SQLITE_VAR_LIMIT = 900`**
(`:725, 2157, 2545, 2586, 3311`).

**[SYNTH-VERIFIED]** the bundled SQLite's compile options (dumped from the shipped binary)
include `MAX_VARIABLE_NUMBER=32766`. Bisection: 32,766 exported OK; 32,767 → `too many SQL
variables in DELETE FROM dirty_issues WHERE issue_id IN (?,?,…)`.

Above the ceiling the failure is **silent** (R2) and causes a death spiral: the file *is* written
(steps 1–9 complete) but `clear_dirty_issues` fails first inside `finalize_export`, so
`set_export_hashes`, `jsonl_content_hash` and `last_export_time` are all skipped. The stale hash
makes every subsequent read command run a full auto-import. Measured at 50k: `create` 1.0 s, next
`list --limit 1` **8.3–9.0 s** (`imported_count=0`), next 25 ms. Forever.

Note `get_all_issues_for_export` includes tombstones, so deletions count toward the ceiling.

**Fix:** chunk. Or better, delete the duplicate pair and chunk the survivor.

---

### R8 — `--db` plus auto-flush writes one workspace's issues into another's git-tracked file
**Severity: high. [SYNTH-VERIFIED] — reproduced by the synthesizer.**

```
cd wsC && obr init --prefix ccc          # empty workspace
grep -c ':ID:' .beads/issues.org         → 0
obr create "Created into A from empty C" --db ../wsA/.beads/beads.db
                                         → ✓ Created aaa-1tc
grep ':ID:' .beads/issues.org            → aaa-1tc aaa-1zw aaa-21g aaa-5sp
sqlite3 .beads/beads.db 'select count(*) from issues'   → 0
```

Workspace C's git-tracked `.beads/issues.org` now contains all four of workspace A's issues under
A's prefix while C's own DB is empty. Exit 0, no warning. `git add .` commits A's tracker into
C's history.

Mechanism — four independent resolutions of "where is the export file":
`run_auto_flush` discovers the beads dir with `config::discover_beads_dir(Some(Path::new(".")))`
(`src/main.rs:261`), **ignoring `--db`**, although `discover_beads_dir_with_cli` exists for
exactly this and is used by ~26 command modules. It then calls `config::open_storage(&beads_dir,
overrides.db, …)`, which **does** honour `--db` and correctly computes `paths.jsonl_path` — and
those correctly-resolved paths are **explicitly discarded**: `let (mut storage, _paths) = …`
(`main.rs:273`). `auto_flush` then re-derives the path a *fourth* time from `Metadata::load`
(`src/sync/mod.rs:1929-1938`). Net: DB from `--db`, destination file from `$PWD`.

In the non-empty case the DL-2 guard catches it — but the refusal is swallowed (R2), so
mutations silently never persist at exit 0.

**Correction to `map-cli.md`:** command-level `--db` handling is actually coherent (running from
A with `--db ../B` correctly used B's prefix and ignored A's `default_priority`). The defect is
exclusively in `main.rs`'s auto-hooks.

---

### R9 — Org round-trip destroys any label containing a colon, including `provides:` capabilities
**Severity: high. [SYNTH-VERIFIED] — reproduced by the synthesizer.**

```
obr label add px-1sk "provides:auth"
obr sync --flush-only
grep '^\*' .beads/issues.org   → * TODO [#C] Capability probe    :provides:auth:
# clear labels, bump :UPDATED_AT:, re-import
obr sync --import-only
sqlite3 … "select label from labels"   → auth
                                          provides
```

`LabelValidator` permits `:` (`src/validation/mod.rs:198-206`) and `label.rs:618-619` explicitly
asserts `validate_label("provides:auth").is_ok()`. But `provides:<cap>` is the **wire format of
the cross-repo capability dependency feature** — `sqlite.rs:3320` builds `format!("provides:{cap}")`
and `:3343` does `label.strip_prefix("provides:")` to decide whether a peer repo satisfies a
capability.

Org emits labels as heading tags (`org_bridge.rs:62-76`), and Org tag syntax is colon-delimited.
One flush plus one import converts `provides:auth` into two labels, and the capability dependency
stops resolving. No error; `obr doctor`'s only consistency check is a record *count* comparison,
which still matches.

Every assurance layer misses it: `org_bridge.rs` has 7 unit tests and none is a label test; no
org proptest exists; there is no `.org` insta golden; the only org-aware label e2e check does a
substring assertion on a single-word label and never re-imports; `fuzz_org_parse` has no oracle.

**Fix:** escape or reject colons in the Org tag emission. And add the ~10-line round-trip
proptest that would have caught it: `issues → issues_to_org_text → org_text_to_issues == issues`.

---

### R10 — `obr sync --merge` is structurally non-functional and force-overwrites on top of it
**Severity: high (bounded by low usage). Confirmed by reading; zero e2e coverage.**

`Issue.content_hash` is `#[serde(skip)]` (verified: `src/model/mod.rs:374-375`), so it is never
on disk. In `execute_merge` (`src/cli/commands/sync.rs:1101-1215`), *left* comes from
`get_all_issues_for_export` → always `Some`; *right* from `read_issues_from_jsonl` /
`org_text_to_issues` → always `None`; *base* from `load_base_snapshot` → always `None`.
`execute_merge` never calls `normalize_issue`.

`merge_issue` case 6 (`src/sync/mod.rs:2764-2771`): `left_changed = (l.content_hash !=
b.content_hash)` is always `Some != None` = true; `right_changed` is always `None != None` =
false; then `(_, false) => Keep(l)`. **Every issue present in the base resolves in favour of the
local DB, unconditionally, regardless of timestamp or strategy** — followed by a `force: true`
export (`sync.rs:1211`) that bypasses both data-loss guards. `save_base_snapshot` writes JSONL
even when the working file is Org (`:2937`), so the base is self-reinforcingly `None` forever.

Compounding: `tombstones` is hardcoded `None` (`sync.rs:1157`), so invariant DL-3 ("import never
resurrects tombstones") does **not** hold on this path; `strategy` is hardcoded `PreferNewer`,
making three arms and all of `MergeConfig` dead; `ensure_no_conflict_markers` is never called, so
in Org the markers are absorbed as body text and the force export erases the evidence; and
`MergeResult::Delete` can hit `IssueNotFound` mid-loop with no transaction wrapping.

`grep -rn '\-\-merge' tests/*.rs` returns **nothing**. The only coverage is 32 unit tests that
call `make_issue_with_hash(…, Some("hash1"))` — **manufacturing the exact precondition
production can never satisfy.**

Secondary damage: when `Keep(right)` *does* fire (cases 3 and 7), `content_hash = NULL` is
written straight into the issues table, permanently disabling content-hash dedup for that row.

**Fix:** normalize base and right before `MergeContext::new`; call `ensure_no_conflict_markers`;
pass the real tombstone set; wrap the apply loop in a transaction. Or gate the command behind a
loud experimental warning until it is fixed.

---

### R11 — Wall-clock LWW destroys work with no merge involved; a future timestamp pins an issue
**Severity: high (structural).**

There is **no logical clock anywhere** (`grep` for lamport/vector/hlc/monotonic over `src/`
returns zero); `Issue` has no version or writer field in any comparison. Every mutator writes
`Utc::now()` unconditionally with no monotonicity guard (`sqlite.rs:613-614, 1823-1826,
1858-1861, 1995-1998, 2019-2022, 657`).

*No-merge loss:* A's clock runs 10 min fast. A edits `bd-1` at `T_A`. B fast-forwards, imports.
B edits later in real time but its honest clock gives `T_B < T_A`, so B's own row's `updated_at`
goes **backwards**. B pushes; A fast-forwards and imports: `T_B < T_A` → `Skip`. A's next
auto-flush reverts the file. B pulls, imports A's version — **B's edit is destroyed on B's own
machine.** No conflict, no marker, no warning.

*Unbounded future:* `IssueValidator` checks only `updated_at >= created_at` (`validation/mod.rs:73-78`)
and never bounds `updated_at` above. One year-2030 timestamp makes an issue permanently
un-updatable across every clone.

*Equal timestamps discard hand edits:* `Ordering::Equal → Skip{"Equal timestamps"}` (`:2125-2127`).
A human who edits a title in `.beads/issues.org` without touching `:UPDATED_AT:` has the edit
**silently discarded**, the file hash stamped so it is never re-examined, and the edit overwritten
by the next auto-flush — **precisely the workflow the Org migration exists to enable**, and no
document warns about it because no document mentions Org at all.

*Empirically*, `:UPDATED_AT:` is also an accidental optimistic-concurrency token. Measured (git
2.55.0, no attributes file): Org edits to two semantically *disjoint fields* of one issue
(assignee vs priority) **conflict**, because both bump `:UPDATED_AT:` and it is a single line. So
Org's per-field lines could give field-level git merge for free, and `:UPDATED_AT:` throws it
away. (Also measured: JSONL edits to two *different* consecutive issues conflict at distance 1
and merge clean at distance ≥2. In this repo's own 549-issue tracker, any two agents touching
consecutive issues in id order block the sync.)

**Fix:** monotonize (`max(now, existing + 1ns)`) and bound (`reject updated_at > now + skew`)
first — cheap, no format change. Then add additive `lamport: u64` (default 0) and `writer_id`
fields ordered as `(lamport, writer_id, updated_at)`, backward compatible with existing files,
and in Org emit them **after** `:UPDATED_AT:` — or move `:UPDATED_AT:` to the end of the drawer —
so the accidental-conflict property does not worsen.

---

### R12 — Dependency cycle check runs outside the transaction (44% failure rate)
**Severity: high. Reproduced 11/25 rounds.**

Verified by reading `src/storage/sqlite.rs:1789-1800`: `would_create_cycle` is called **before**
`self.mutate(...)`. Two processes each evaluate a pre-transaction snapshot, both see no cycle,
both commit. `create_issue` does it **correctly**, running `check_cycle` on `tx` inside the
closure (`:281`).

Reproduced with concurrent `obr dep add A B` / `obr dep add B A`: real mutual cycles in 11 of 25
rounds. Consequences: `obr ready` prints "No open issues" so both agents are permanently starved
with no explanation; `obr blocked` shows the mutual block as normal; `obr doctor` reports all OK,
exit 0. The correct detector — `detect_all_cycles`, an explicit-stack DFS — already exists, is
tested, and is reachable only from `obr dep cycles`.

**Fix:** move the check inside the closure operating on `tx`. `check_cycle` already takes
`&Connection` and `&Transaction` derefs to it — close to a one-line change. Also wire
`detect_all_cycles` into `doctor`.

---

### R13 — `waits-for` cycles are creatable with zero concurrency (pure logic bug)
**Severity: high. [SYNTH-VERIFIED] — reproduced by the synthesizer with two sequential commands.**

```
obr dep add wf-v1e wf-3q8 --type waits-for   → ✓ Added
obr dep add wf-3q8 wf-v1e --type waits-for   → ✓ Added      ← should be rejected
obr ready                                    → ✨ No open issues
obr dep cycles                               → Warning: Found 1 dependency cycle(s):
                                                 1. wf-3q8 -> wf-v1e -> wf-3q8
obr doctor                                   → exit 0
# control: the same shape with --type blocks IS correctly rejected
```

`check_cycle`'s filter (verified at `sqlite.rs:352-354`) is
`AND type IN ('blocks', 'parent-child', 'conditional-blocks')` — it **omits `'waits-for'`**. But
`DependencyType::is_blocking()` (`src/model/mod.rs:245-250`) *includes* `WaitsFor`, and
`add_dependency` gates the cycle check on `is_blocking()`. So the check runs, over a graph that
excludes the very edge type being added. `waits-for` is honoured everywhere else — the blocked-
cache rebuild includes it, as does the `idx_dependencies_blocking` partial index. **Only the
cycle checker disagrees.**

**Fix:** add `'waits-for'` at `sqlite.rs:353`. Better: derive the SQL type list from
`DependencyType::is_blocking()`, since **four** separate places hand-enumerate these types.

**Trap:** `src/cli/commands/graph.rs:1116-1127` deliberately constructs exactly this cycle (all
`waits-for`, each `.unwrap()`ed) to prove `graph_all` does not hang. **Fixing the bug breaks the
test.** The suite has encoded the defect as a fixture; fix and test must change together.

---

### R14 — The test pyramid is not executed, and CI cannot build the tree
**Severity: high (meta).**

The only test command any automated gate runs is `cargo test --lib --bins` (`lefthook.yml:9-12`;
`flake.nix:210-221`; `flake.nix:84` sets `doCheck = false`). The only job running the integration
suite is `ci.yml:88-96`, and **CI cannot build at all**: `Cargo.toml:64` declares
`org2jsonl = { path = "../org2jsonl" }` and `grep -rn org2jsonl .github/` is empty, so
fmt/clippy/check/test all fail at dependency resolution on a clean runner — inoperative since the
Org migration on 2026-02-18. Both local gates *do* run `cargo clippy --all-targets`, which
**compiles** the 97 integration targets, so they type-check every commit and never run.

The assurance agent built a copy outside the repo and executed 4 of 97 targets. **All four
failed**, with 11 distinct real failures:

| # | Test | Cause |
|---|---|---|
| 1 | `e2e_init_creates_jsonl` | asserts `.beads/issues.jsonl` after sync; init writes `.org` |
| 2 | `snapshot_jsonl_export` | panics on `.expect(read jsonl)` |
| 3–6 | 4 `cli_output` goldens | `Usage: br` → `Usage: obr`; "Parsed 0 records" → "Parsed 0 issues from Org format" |
| 7–9 | 3 `error_messages` goldens | capture `path=./.beads/issues.jsonl` |
| 10 | `test_cargo_metadata` | asserts `name = "br"`; it is `"obr"` |
| 11 | `test_version_consistency` | Cargo 0.1.14 vs Scoop/AUR 0.1.13 |

And the three **crown-jewel git-safety tests are red**:
`regression_full_sync_cycle_does_not_touch_git` panics at `:408`; both
`integration_sync_*_only_touches_allowed_files` fire the project's own alarm —
`SAFETY VIOLATION: sync export modified files outside allowed list! CREATED
.beads/.br_history/issues.*.org, MODIFIED .beads/issues.org` — because the test keeps a **private
copy** of the allowlist (`tests/e2e_sync_git_safety.rs:637-676`) listing only
db/db-wal/db-shm/jsonl/jsonl.tmp, which never received `org`/`org.tmp`.

Worse than failing, **two tests are silent no-ops**: `e2e_sync_git_safety.rs:1086-1092` and
`e2e_git_safety_full_cli.rs:1099-1104` wrap the "simulate a git pull" mutation in
`if jsonl_path.exists()`, which never fires — so the import half of the safety test imports an
unchanged file and proves nothing.

**[ADDENDUM] — the suite is not uniformly broken, and one common-module test poisons every
target.** A gap-fill agent built an out-of-tree copy (with `org2jsonl` copied alongside, which
resolves the `../org2jsonl` path dep by position) and ran `cargo test --test e2e_routing` under
`nix develop`: **all 14 routing tests pass.** The only failure in that target was
`common::dataset_registry::tests::test_metadata_includes_source_commit`
(`tests/common/dataset_registry.rs:1226`) — which fails purely because the copy has no `.git`.
Since `tests/common/` compiles its unit tests into *every* integration target, **that single
assertion turns `cargo test --test <anything>` red outside a git checkout** — worth knowing for
anyone re-enabling these gates, and a second reason (besides the `org2jsonl` path dep) that CI
could not go green even if it could build. Note also that the system `rustc` here is stable
1.97.1 and `rust-toolchain.toml`'s `channel = "nightly"` is not honoured without rustup, so the
build fails on `rich_rust`'s `#![feature(let_chains)]` (E0554) outside `nix develop`.
Passing ≠ meaningful, though: §10.2 shows 9 of the 14 routing assertions are vacuous, and the one
comment in the file that explains the redirect mechanism (`tests/e2e_routing.rs:231`) asserts the
opposite of the truth.

Also: `packaging/homebrew/br.rb` is now `obr.rb`, so both Homebrew checks silently `return` and
the formula is untested. `tests/common/`'s 124 helper unit tests execute ~73 times per
`cargo test` (~9,000 redundant executions), and `conformance_edge_cases.rs` re-includes the
13,491-line `conformance.rs` so its 222 tests run twice. Coverage and audit gates are decorative:
`tarpaulin.toml`'s `fail-under = 50.0` is invoked by nothing; `cargo audit --deny warnings` is
`continue-on-error`; `release.yml:32-34` documents that fmt/clippy/test were **deliberately
removed** from the release pipeline.

---

### R15 — Conformance-to-Go: skipped equals passed, no version pin, divergence silenced
**Severity: medium-high (meta).**

`skip_if_no_bd!()` is `eprintln!(…); return;` — a skipped conformance test is a *passing* test.
On every PR all ~315 conformance tests skip silently; bd is built only by a **weekly cron**
workflow. That workflow does `git clone --depth 1 …/beads.git` — an **unpinned HEAD clone of a
fork**. `MIN_BD_VERSION = "0.5.0"` exists but `check_bd_version` is unreachable from the 222-test
master suite, and `compare_versions` parses `0.46.0 → [0,46,0] > [0,5,0]` so it passes, as does
`1.1.2`; there is no upper bound. Meanwhile **all 13 divergence notes are written against bd
v0.46.0**, and upstream ≥0.50 defaults to Dolt while ≥0.58 has no SQLite at all. A modern binary
passes the gate and then diverges on everything.

All 13 `INTENTIONAL DIVERGENCE` sites are `#[ignore = "…"]`, which asserts nothing; there is no
`CompareMode::MustDiffer`. **Eleven of thirteen are `list`** — the suite has disabled essentially
all JSON parity coverage for the most-used command. Two are genuine design positions
("br parent-child dependency blocks children; bd does not"; "bd sync does git commit… br is
intentionally non-invasive") living only as attribute strings; `grep -rn divergence docs/
.beads/*.md` returns nothing.

Four independent `bd_available()` implementations exist; two only check that `bd version` exits
0, so **an `obr` aliased as `bd` would be accepted as the reference implementation** and ~48
tests would compare obr to itself and pass. (The master suite's anti-spoof check —
rejecting a bd whose version starts with `obr` — is right and just needs to exist in all four.)

---

### R16 — Performance: auto-flush rewrites the whole file after every mutation
**Severity: medium-high.**

Every mutating command triggers a **full** export. Verified with `RUST_LOG=debug`:
`dirty_count=9` → `exported=10021`. Each flush performs: one full file copy (`.br_history`
backup), **two** full file re-parses (`analyze_jsonl` safety scan + `count_issues_in_jsonl`
verification), one full write + `fsync`, four full table scans, a deep clone of every `Issue`,
and N autocommit INSERTs into `export_hashes`.

Measured `obr create`: 60 ms @44 issues, 0.45–0.75 s @1k, 1.2–1.7 s @10k. With
`--no-auto-flush`: 9.8 / 47 / 47 ms. **All growth is auto-flush.**

Upstream Go bd avoided exactly this with a daemon + 5 s debounce + **incremental** export. obr
dropped the daemon (correctly) but never replaced the debounce — and its incremental machinery
(`get_issues_needing_export`) has **zero callers**.

Related measurements:
- **Org export peaks at 810 MB RSS for a 38 MB file at 50k** (21×) vs 235 MB for JSONL, because
  `issues_to_org_text` builds the entire document as one `String` before a byte is written. Org
  is the **default** format and `grep -rn org benches/` returns nothing.
- Import is fully autocommit — ~7 fsync'd transactions per issue, **40 s for 50k**. Only ~57%
  CPU with `sys == user`: the signature of syscall domination. One `BEGIN IMMEDIATE` around
  Phase 3 would be a large multiple *and* fix the half-imported-DB hazard.
- `--no-db` mode re-imports the whole file into an in-memory DB on **every** invocation:
  2.34 s/command at 10k vs 125 ms normal (19×).
- `obr count` materializes every `Issue` in Rust to call `.len()` (213 ms and ~100 MB at 50k) —
  `count_issues` (`SELECT count(*)`) exists and is unused.
- **`idx_issues_ready`, commented "most important for performance", is never chosen by any
  query plan.** `EXPLAIN QUERY PLAN` on a real 50k obr DB shows `idx_issues_status` + a TEMP
  B-TREE. Three independent causes: `(pinned = 0 OR pinned IS NULL)` cannot imply the partial
  index's `pinned = 0`; the `ORDER BY CASE WHEN priority <= 1 …` is unindexable; and there is no
  `ANALYZE` or `PRAGMA optimize` anywhere so the DB has no `sqlite_stat1`. Pure write
  amplification.

**The benchmarks are structurally blind.** `bench_export` measures `export_to_writer` — which
production never calls, so it excludes by construction the history copy, both re-parses, the
fsync, the rename and `finalize_export`. All sizes stop at 5,000 (the ceiling is 6.5× higher).
Benches open brand-new DBs, so they run with pragmas the shipped CLI never gets.
`tests/bench_synthetic_scale.rs` has the right idea and fails five ways — `#[ignore]`d and gated
on an env var no workflow sets, **zero assertions**, filters failures out of its own summary,
reads `/proc/self/status` (Linux-only, and of the *harness* not the child, so the one metric that
would have caught the 810 MB Org export watches the wrong process), and benchmarks
`obr export`, **a subcommand that does not exist**.

---

### R17 — Per-connection *performance* pragmas revert on every reopen
**Severity: medium. [SYNTH-VERIFIED — and the widely-repeated FK version of this finding is WRONG; see §9.1.]**

All per-connection pragmas live inside `apply_schema` (`schema.rs:216-227`), which
`open_with_timeout` calls only when `user_version < 1`. `journal_mode = WAL` is persistent and
survives. **`synchronous`, `cache_size` and `temp_store` do not.** Confirmed from the shipped
binary's compile-option table: `DEFAULT_SYNCHRONOUS=2` (FULL, not the intended NORMAL) and
`DEFAULT_CACHE_SIZE=-2000` (2 MB, not the intended 8 MB); `temp_store` reverts to FILE.

So the tuning applies **exactly once per database lifetime** — during `init` — and never again.
Every WAL commit in production does a full fsync (multiplying R16's autocommit import cost), the
page cache is 2 MB, and the TEMP B-TREE sorts that both `list` and `ready` always build spill to
disk.

`foreign_keys` is the exception and is fine (§9.1) — but only because rusqlite's *vendored*
SQLite is compiled with `SQLITE_DEFAULT_FOREIGN_KEYS`. **That is an invisible dependency on a
C build flag that appears nowhere in obr's source and is asserted nowhere.** Build against a
system SQLite (default FK off) and import silently stops cascading and starts orphaning rows
instead — the import path's *behaviour* changes with a build flag.

Both existing pragma tests are structurally incapable of catching any of this:
`sqlite.rs:4637-4657` uses `open_memory()` and `schema.rs:544` asserts immediately after an
explicit `apply_schema`. **Missing test:** create on disk, drop, reopen, assert
`PRAGMA foreign_keys == 1 AND synchronous == 1`.

**Fix:** extract `configure_connection(&Connection)`, call it unconditionally after
`Connection::open`, leave only DDL + `user_version` in `apply_schema`. Also set `busy_timeout`
unconditionally (`sqlite.rs:102-104` sets it only when supplied; `SqliteStorage::open()` passes
`None` and falls back to rusqlite's 5000 ms default, which the crate documents as "subject to
change"). Consider `PRAGMA optimize` before close and an `application_id`.

---

### R18 — `obr list | head` panics and exits 134
**Severity: medium. [SYNTH-VERIFIED].**

```
$ (set -o pipefail; obr schema all 2>/dev/null | head -c 5 >/dev/null); echo $?
134
```

Text and TOON paths use bare `println!`; `panic = "abort"` (`Cargo.toml:94`) converts the
`BrokenPipe` panic into SIGABRT, so the status is 134 — **outside obr's documented 1–8
families** — and stderr carries a Rust panic message instead of the JSON envelope an agent's
parser expects. `--json` survives only because `json`/`json_pretty` write through a `BufWriter`
and tolerate the IO error; `ctx.toon` and `toon_with_stats` use `println!` and need the same
treatment. The toolchain is already nightly, so `-Zon-broken-pipe=kill` is a one-line fix.

**[ADDENDUM] — the *other* `panic = "abort"` hazard (a parser panic on hostile file content) is
real in kind but unrealized in practice.** A gap-fill agent ran a 100,000-iteration byte-level
mutation fuzz through `org_to_entries_with_keywords` → write → reparse → rewrite inside
`catch_unwind`, plus a 32-file adversarial `.org` corpus through obr's actual auto-import path
(`obr list`, 30 s timeout, `ulimit -v 4 GB`): **zero panics, zero SIGABRT, zero hangs, zero
OOM.** Every failure produced obr's JSON envelope with a clean exit code. Invalid UTF-8 and lone
surrogates are rejected at `fs::read_to_string` (`sync/mod.rs:2230`) before the parser sees them.
The robustness defects that *do* bite are different ones: an O(n²) parse blowup and an
all-or-nothing import (R31).

---

### R19 — Import is non-transactional; comment ids collide across issues
**Severity: medium.**

`grep -n transaction src/sync/mod.rs` returns **nothing**. `import_from_jsonl` calls
`clear_all_export_hashes()` at `:2380` — **before any write** — then Phase 3 (`:2466-2477`) is a
bare loop of autocommit `conn.execute` calls. Input-side failure is safe by construction (the
whole file is parsed and validated before any write), and the crash case self-heals because
metadata is written last (§4.5) — **unless** `--allow-stale` or `--no-auto-import` disables the
repair, in which case the DB stays permanently behind and every mutation is silently withheld
from the file.

**All five `import_failure_*_no_db_changes` tests cover only input-validation aborts** — they
verify a property that holds by construction. Nothing injects a DB-side failure mid-import.

Separately: `comments.id` is a **global** `INTEGER PRIMARY KEY AUTOINCREMENT` rowid
(`schema.rs:122-129`) that is serialized to the file and re-inserted verbatim.
`sync_comments_for_import` deletes only the *target* issue's comments then does
`INSERT OR REPLACE INTO comments (id, issue_id, …)` (`sqlite.rs:3812-3822`). **A file-supplied
comment id colliding with a comment on a different issue silently overwrites it.** Cross-repo
comment merging is not id-safe.

---

### R20 — Import bypasses all dependency validation; remapping can mint self-blocking edges
**Severity: medium.**

`IssueValidator` does not inspect dependencies at all. `sync_dependencies_for_import`
(`sqlite.rs:3768-3794`) is a bare `DELETE` + `INSERT OR IGNORE` with **no cycle check and no
self-edge check** — unlike `add_dependency`. If two file issues X and Y both dedup to the same
existing Z, Phase 2 rewrites both ids to Z, so a dependency `X → Y` becomes **`Z → Z`**;
`rebuild_blocked_cache_impl` then marks Z permanently blocked and it never appears in
`obr ready`. A hand-edited or merged file can introduce arbitrary cycles directly.

---

### R21 — Duplicate ids in the import file resolve by last-LINE-wins, not last-write-wins
**Severity: medium (but load-bearing for §3.2).**

There is **no duplicate-id detection anywhere** in import; `analyze_jsonl` collapses ids into a
`HashSet` so it cannot report them either. Phase 1 evaluates *both* duplicate lines against the
pre-import state, so both get `Update`; Phase 3 applies them in file order. **The last line wins
regardless of timestamp.** This is exactly the shape a `merge=union` driver, a careless conflict
resolution, or a hand-edited Org file produces — and it is the concrete reason obr must never
configure `merge=union`.

---

### R22 — Tombstone/deletion semantics do not converge
**Severity: medium.**

`determine_action` checks `is_tombstone(existing_id)` **before** `force_upsert` and before any
timestamp comparison (`:2097-2103`). Once clone A has tombstoned `bd-1`, **no** incoming version
can ever be applied to A — not a newer edit, not `--force`, not a deliberate human resolution.
If `T_delete < T_edit`, clone B keeps the issue open and re-exports it; A imports it,
tombstone-vetoes, re-exports the tombstone. **The working file flips on every sync round trip
forever.**

There is also no un-delete path: import never deletes, and `ImportConfig.orphan_mode` /
`OrphanMode::Resurrect` is parsed from `--orphans`, stored, logged, and **never read** — step 7
of `import_from_jsonl`'s own docstring is unimplemented.

Reaping is path-dependent: auto-flush builds an `ExportConfig` whose `retention_days` is `None`
so it **never reaps**, while `obr sync --flush-only` does. Whether a tombstone line survives
depends on which code path flushed.

---

### R23 — Org round-trip is lossy and is not a fixpoint
**Severity: medium.**

`extract_body_text` (`org_bridge.rs:540-554`) handles **only** `Element::Paragraph` — lists,
tables, src blocks, quote blocks and sub-structure inside description/design/acceptance_criteria/
notes are **dropped**. It also indexes the `\n\n` separator on the *element* index rather than the
emitted-paragraph index, so a body of `[List, Paragraph]` gains a spurious leading blank line.

In the originating clone this is invisible (import Skips on equal timestamps), but a **different**
clone receiving the file for the first time inserts the lossy version, computes a different
content hash from it, and its next export writes the truncated description back. **Content
degrades exactly at clone boundaries and the degradation propagates as a phantom change.**

Related: `#[serde(untagged)] Custom(String)` on `Status` and `IssueType` (`model/mod.rs:45, 148`)
means a typo in a hand-edited file becomes a new *custom* status rather than an error — and the
asymmetry is worse than that. `Status::from_str` **rejects** an unknown status from the CLI while
the file path accepts one. Reproduced: editing `* TODO [#C] Status validation probe` to
`* FROBNICATE [#C] …` yields title `FROBNICATE [#C] Status validation probe`, status silently
reset to `open`, priority silently reset to P2, exit 0. Since the file is hand-editable and
git-merged — the entire point of the Org migration — **the looser boundary is the one that
matters**, and a merge that mangles one keyword rewrites the issue.

(Credit: a 10-title adversarial round-trip — leading `*`, leading `[#A]` cookie, `:PROPERTIES:`
text, org markup, embedded tab, emoji, backslashes — found only one other infidelity: trailing
whitespace in a title is trimmed. The Org bridge is otherwise solid.)

**[ADDENDUM] — attribution confirmed, magnitude quantified, and three further defects in the
same 100 lines.**

*Attribution.* The loss is **obr's, not the parser's**. Running org2jsonl's own oracle
(`roundtrip.sh` = `diff "$1" <(org2jsonl "$1" | jsonl2org)`) over obr-generated `issues.org`
files shows the parser reproduces lists, tables, src blocks, quote blocks, drawers, keywords,
comments, fixed-width lines and footnote definitions **byte for byte**, modulo blank-run collapse
and trailing-whitespace stripping. obr then throws them away: `extract_body_text`
(`org_bridge.rs:540-554`) consumes **1 of the parser's 23 `Element` variants** (a second,
`SrcBlock`, is consumed conditionally by `extract_json_from_body` for the Dependencies/Comments
children); `flatten_inline_to_text` (`:451-537`) handles **14 of 22 `InlineContent` variants**,
ending in `_ => {}`.

*Three defects beyond "only Paragraph is handled", all in the same neighbourhood:*
1. The `"\n\n"` separator is keyed on `i > 0` — the index in `body`, not on whether a paragraph
   was emitted. Because orgize's paragraph token keeps its own trailing `\n`, a dropped element
   between two paragraphs yields the signature `Before.\n\n\nAfter.` This is why convergence
   takes **two** cycles rather than one.
2. `flatten_inline_to_text`'s `Entity` arm (`:519-523`) emits `\NAME{}` — **adding braces the
   source did not have**. Content *growth*, the opposite direction from every other loss, and it
   is a genuine model-level lossiness: `org2jsonl`'s `Entity { name }` (`model.rs:282-283`) does
   not record whether the source used `{}`.
3. `unsanitize_org_text` (`:569-580`) is **not the inverse** of `sanitize_org_text` (`:248-259`):
   sanitize prefixes a comma only when a line `starts_with('*')`, unsanitize strips a comma from
   *any* line starting with `,*`. A user's literal `,*` silently loses its comma on first import.
   (`sanitize_org_text` also normalizes CRLF→LF and drops a trailing newline via
   `lines()`/`join("\n")` — stable, but lossy on the first write.)

*Magnitude, measured.* A 30,000-sample driver over Org-hostile random descriptions, calling
obr's own `issues_to_org_text` / `org_text_to_issues`:
**2,807/30,000 (9.4%) lose or change content on the first pass; 318/30,000 (1.06%) change
*again* on the second; nothing moves at generation 3 or 4.** So obr's pipeline is
eventually-convergent (unlike org2jsonl's writer, R30) but **is not a fixpoint after one cycle**
— a clone that imports a healthy file and flushes it back produces a *different* file.
Hand-built confirmation over 47 adversarial payloads: `PlainList`, `Table`, `SrcBlock`,
`QuoteBlock`, `Drawer`, `Comment`, `FixedWidth`, `HorizontalRule`, footnotes,
`#+TITLE:`/`#+SEQ_TODO:`/`CLOCK:`/example/verse bodies (these become `None` — **whole description
erased**), macros, targets and statistics cookies all vanish. Also confirmed: a label containing
`:` splits into two labels, and a *title* ending in `:something:` is truncated with a **phantom
label injected** — labels are emitted raw (`org_bridge.rs:63-74`) while every property value is
sanitized.

*The phantom change, end to end.* Verbatim: drop a healthy `issues.org` into a fresh workspace →
`obr list` (auto-import; the file's MD5 is **unchanged**, so nothing is visible) → `obr create`
one unrelated issue → the file's MD5 changes and **twelve unrelated issues' descriptions are
destroyed in the same write**. The read command that caused the damage leaves no trace on disk;
the destruction lands in the commit of whoever next mutates anything. This is R23's propagation
mechanism, reproduced.

*Assurance.* `grep -rn 'org_text_to_issues\|issues_to_org_text' tests/` returns **nothing**, and
none of the seven `#[test]`s in `org_bridge.rs:753-900` parses an Org file. **The 15-line
function responsible for the entire loss surface of the durable artifact has no test at all**, in
a codebase with 788 unit tests.

---

### R24 — Agent-facing surface: `--robot` is broken, exit codes collide, no envelope
**Severity: medium (this is the target audience).**

- **`--robot` is a clap usage error on 29 of 39 commands.** **[SYNTH-VERIFIED]**
  `obr list --robot` → `error: unexpected argument '--robot' found`, **exit 2** — which collides
  head-on with obr's own `ErrorCode` family 2 = *Database errors*. An agent seeing exit 2 cannot
  distinguish "that flag does not exist here" from "database locked / not initialized", and those
  demand opposite responses. (**Correction to `map-cli.md` and `web-rust-cli-norms.md`, which
  both call this a "silent no-op".**)
- **`--robot` is inconsistent within one subcommand.** `sync.rs:97` computes
  `use_json = ctx.is_json() || args.robot`, then passes `use_json` to `execute_status` and
  `execute_import` but the raw `json` to `execute_flush` and `execute_merge`. So
  `obr sync --status --robot` prints JSON and `obr sync --flush-only --robot` prints human text —
  and `--flush-only` is the mode agents call most.
- **`--robot` never reaches error handling**: `handle_error(&e, cli.json)` (`main.rs:129`) sees
  only the global `--json`, so on a TTY `obr ready --robot` emits JSON on stdout and a
  human-readable ANSI error on stderr.
- **Exit codes collide twice** (clap 2 vs Database 2; `NOTHING_TO_DO` 3 vs `ISSUE_NOT_FOUND` 3)
  and are **undocumented** — `obr --help | grep -ci exit` → **0** [SYNTH-VERIFIED]. Closing an
  already-closed issue exits 3 [SYNTH-VERIFIED], so an agent cannot distinguish "already done"
  from "does not exist" by exit code.
- **Idempotency is inconsistent**: `label add` twice → 0; `dep add` twice → 0; `close` twice → 3.
  Three behaviours, none documented.
- **JSON is heterogeneous, unversioned and unenveloped.** Bare arrays (list/ready/search/
  blocked/stale/orphans/dep list/label list/comments); **a bare array of length 1 for
  `show <id>`** [SYNTH-VERIFIED]; ad-hoc objects for stats/count/lint/where/info/changelog;
  compact for doctor/version and pretty for 70 other sites. Entity shape differs per command for
  the same entity (`ready` items have 8 keys, `list` items 13). `compaction_level` and
  `original_size` are forced into every record and are always 0 — pure token waste in the
  highest-volume output. `--fields` exists but **only for CSV**: the single highest-leverage
  agent token lever is wired to the least-used encoding.
- `obr version --json` ships build placeholders: `"commit":"VERGEN_IDEMPOTENT_OUTPUT"`.
- **ANSI leaks into "Plain" mode with `NO_COLOR=1`.** `config::should_use_color`
  (`src/config/mod.rs:892-907`) ranks config `display.color` **above** the `NO_COLOR` env var
  while `OutputContext::from_output_format` checks `NO_COLOR` itself, so with `display.color:
  true` you get `mode() == Plain` and `use_color == true`. Violates both `AGENTS.md:418` and the
  NO_COLOR spec.
- **`--format csv --quiet` emits nothing** — `OutputContext::from_output_format` folds `Csv` into
  the `Text | Csv` arm, so `--quiet` maps CSV to `OutputMode::Quiet` and `list.rs:77-79` early-
  returns before reaching the CSV branch. `--quiet` on a *data* encoding should suppress chatter,
  not data.
- `--quiet` is honoured per-call-site, not by the abstraction: 382 raw `println!` and 49
  `eprintln!` under `src/cli/commands/` versus ~82 `ctx.print/success/info` calls. The
  `OutputContext` abstraction governs about a fifth of the tool's output.

---

### R25 — `obr doctor` cannot see the failures that matter, and exits 0 on all of them
**Severity: medium.**

Measured against three corrupted workspaces: (a) content divergence with matching counts → **all
OK**, including the actively misleading "OK counts.db_vs_jsonl: Both have 4011 records"; (b) a
genuine permanent dependency deadlock → **all OK**; (c) the export wedge with mismatched counts →
one WARN, **still exit 0**.

Gaps: no content comparison (a per-issue hash diff between DB rows and file records would catch
R1 and R4 cheaply); no cycle check despite `detect_all_cycles` existing, being correct, and being
one call site away; no orphan check. `std::process::exit(1)` at three points covers only
operational failures, so `obr doctor && deploy` cannot gate on anything.

**[ADDENDUM] — one nuance in doctor's favour, and one against.** *For:* when a check reports
ERROR rather than WARN, doctor **does** exit 1 — verified on a corrupted file
(`ERROR jsonl.parse: Failed to parse Org file: … Missing required :ID: property`, exit 1). The
"always exits 0" phrasing above is too strong; the accurate statement is that `has_error`
(`doctor.rs:58`) escalates only on ERROR, and **the single most diagnostic signal —
`counts.db_vs_jsonl` — is a WARN**, so the export wedge (R6) yields exit 0. *Against:* in the
fresh-clone case doctor prints `OK counts.db_vs_jsonl: Both have 0 records` beside
`ERROR jsonl.parse`, i.e. it is **reassuring and wrong at the same moment** — and that is
precisely the case where following the R5 playbook destroys the remaining data. Promoting
`counts.db_vs_jsonl` to ERROR when the divergence is one-directional with unexported ids, and
attaching remediation text, is the cheapest useful change here.

---

### R26 — Structure: a god object whose file size defeats its own invariant
**Severity: medium (maintainability).**

`SqliteStorage` has ~89 public methods spanning twelve unrelated concerns. **14 have no caller
outside `sqlite.rs`**, and there are **three exact-duplicate public pairs**:
`clear_dirty_issues`/`clear_dirty_flags`, `clear_all_dirty_issues`/`clear_all_dirty_flags`,
`count_issues`/`count_all_issues`. (R7's ceiling exists in *both* halves of the first pair.)

"Add a trait" is not the right criticism — YAGNI applies with one backend, and SQLite's
multi-process story is the moat. The sharper argument: **the 4-step protocol is enforced only by
the privacy of `conn`, i.e. by the module boundary, and the module is one 5,157-line file
containing 12 direct write paths that bypass it.** Splitting into `storage/{issues,deps,labels,
comments,config,sync_support}.rs` with `conn` private to a small `storage/conn.rs` would make the
protocol **compiler-enforced**.

Two layering inversions: `config ↔ sync` is a **dependency cycle** (config imports
export/import/finalize from sync; sync imports `config::Metadata`), and `open_storage_with_cli`
performs a full JSONL import in `--no-db` mode — the config layer doing bulk I/O. **R8 lives in
this cycle.** And `storage` imports `crate::format`, so `get_issue_details` is a SQL-layer method
returning a presentation DTO carrying `JsonSchema`; a JSON output contract change forces a SQL
layer change.

The **36-column positional row mapping** is the project's own open bead `beads_rust-2ckc`:
`issue_from_row` reads `row.get(0)..row.get(35)` and the identical column list is hand-duplicated
at **11 SELECT sites**. A reorder in any one silently corrupts data with no compile error.
rusqlite supports `row.get("column_name")`. **This is not forced by bd parity** — bd's schema
constrains the columns, not how Rust reads them.

---

### R27 — Large dead surface
**Severity: low-medium.**

~1,927 lines across `src/format/{context,theme,rich,syntax,markdown}.rs` — an abandoned output
generation with colliding type names, carrying **58 unit tests** (~7.4% of the one enforced test
budget spent on unreachable code). ~~About 400 lines of `config/routing.rs` (the Gastown
mayor/town routing, explicitly excluded by the porting plan, ported anyway and abandoned): 8 of
10 public items have zero external references.~~ **[ADDENDUM] — CORRECTION, and this one is
dangerous if acted on.** `config/routing.rs` has **12** public items, not 10. Ten of them are genuinely dead
(`resolve_route`, `resolve_route_entry`, `load_routes`, `find_route`, `find_town_root`,
`extract_prefix`, `is_external_id`, `RouteEntry`, `RoutingResult`, `RoutingResult::local/
::external`) — verified by exhaustive grep, and empirically: a valid `routes.jsonl` pointing at a
workspace containing the requested issue is **ignored**, and a syntactically invalid one produces
**no error**, because `load_routes` is never called. But the remaining two —
**`follow_redirects` (`:200`) and its helper `read_redirect` (`:162`) — are the first thing that
runs in 33 of 39 commands and in both auto-hooks.** Deleting the module on this item's original
advice deletes the workspace resolver for the entire CLI. See R28 for what that code does and
does not check. `find_town_root`'s unbounded upward walk (`:85-101`, one `is_file()` stat per
ancestor with no depth cap, terminating only at `/`) is dead today but would be a real cost if
`resolve_route` were ever wired up. Note that abandoning `routes.jsonl` / `mayor/town.json` is a
**product** decision, not a cleanup: they are part of the `bd` compatibility surface
(`routing.rs:6-19`), and obr silently ignoring a `routes.jsonl` that `bd` would honour is a
divergence nobody has recorded. ~530 lines of `preflight_export`/`preflight_import` with no
production callers (the real guards are re-implemented inline, and a **third** time in
`execute_flush`). `DependencyValidator` and `CommentValidator`, both never called
(`CommentValidator`'s `id > 0` rule is unusable pre-insert anyway). All 7 `StructuredError` smart
constructors plus `find_similar_ids` and `levenshtein_distance`. `ResultExt`/`OptionExt`. Three
`BeadsError` helper methods, one of which (`exit_code()`) **actively contradicts live behaviour**
in its doc comment. `IssueTableColumns::compact()/full()`, `DependencyTree`, `StatsPanel`,
`ctx.section()`, `ctx.error_panel()`, `export_to_writer*` (benches only).

Because it is all `pub`, there are no dead-code warnings. The most user-visible consequence:
issue descriptions and comments are **never markdown-rendered or syntax-highlighted** despite
complete implementations existing.

---

### R28 — A git-committable `.beads/redirect` file steers every read and write, unvalidated
**Severity: critical. [GAP-FILL] — reproduced end-to-end. True rank: beside R8.**

`routing::follow_redirects` (`src/config/routing.rs:200-232`) reads `.beads/redirect`, treats its
trimmed contents as a path (absolute verbatim at `:177-178`; relative resolved against
`beads_dir.parent()`, i.e. **CWD-relative** in the ordinary case, at `:181`), and loops up to
`max_depth = 10`. What it does **not** do: reject `..`, resolve symlinks, canonicalize, require
containment in the repo, or call anything from `src/sync/path.rs` — the module imports only
`error`, `serde`, `fs`, `io`, `path`, `tracing` (`:21-26`). No caller validates the result
either.

Reproduced, in a directory whose `.beads/` contains **nothing but a `redirect` file**:

```
$ printf '%s\n' "$PWD/rt/OUTSIDE" > rt/B/.beads/redirect
$ cd rt/B && obr create "Written to OUTSIDE" --type task --priority 3
✓ Created bd-w96: Written to OUTSIDE
INFO beads_rust::sync: Auto-flush complete exported=1                exit=0
$ ls -la rt/OUTSIDE
-rw-r--r--  229376  beads.db          # a SQLite database, in a directory the user never named
-rw-------     335  issues.org        # and the durable artifact
```

And into `.git`:

```
$ printf '%s\n' "$PWD/rt/REPO/.git" > rt/B/.beads/redirect
$ cd rt/B && obr create "Written into dot-git" --type task           exit=0
$ ls rt/REPO/.git
beads.db   hooks/                     # beads.db INSIDE .git
```

**Split verdict, and the split is the finding.** The Org export *was* correctly refused
(`Refusing to use JSONL path inside .git directory`, `sync.rs:232-241`) — but on the auto-flush
path that refusal is swallowed at `debug!` (R2), so `obr create` printed success and exited 0
with no `Auto-flush complete` line. Meanwhile **`SqliteStorage::open_with_timeout` performs no
path validation at all** (`config/mod.rs:315`, `:412`), so the database landed inside `.git`
regardless. The `.git` string check survives because it inspects the path *text*; every
boundary-relative check (`validate_sync_path`'s `starts_with(canonical_beads)` at `path.rs:326`,
`sync.rs:212`'s `is_external`) is satisfied by construction once `beads_dir` *is* the redirected
directory.

**Why a checked-in redirect is realistic:** `obr init` writes a `.beads/.gitignore`
(`init.rs:84-95`) listing `*.db`, `*.db-shm`, `*.db-wal`, `*.lock`, `last-touched`, `*.tmp` —
**and not `redirect`**. This repo's own `.beads/.gitignore`, inherited from the Go tool, *does*
exclude it, with the comment *"Must not be committed as paths would be wrong in other clones"*.
Nothing in the Rust tree ever writes a `redirect` file; it is a pure, unvalidated, git-committable
input.

Seven further behaviours, all reproduced:
- **`max_depth` exhaustion is not an error** (`:204-221`); hop 10 is silently adopted. Worse,
  `obr where` follows the chain a *second* time (`where.rs:37`) and therefore reports hop 14
  while every other command reads and writes hop 9. `where` is the documented way to answer
  "which `.beads` am I using?" (`docs/CLI_REFERENCE.md:740`) and it is wrong in exactly the case
  where it matters.
- **Loop detection is exact `PathBuf` string equality** (`:208`), so `/x/.beads` vs `/x/./.beads`
  vs a symlink alias are not the same node — those degrade into the silent truncation above. A
  plain self-redirect *is* caught (exit 7).
- **Symlinks are followed both ways** — `is_dir()` at `:224` and `is_file()` at `:164` follow
  links, so both the target directory and the `redirect` file itself may be symlinks.
  `src/sync/path.rs`'s explicit `SymlinkEscape` rejection (`:300-316`) protects files *inside*
  `.beads`; the escape here happens one level up.
- **A relative redirect containing `..`** yields a `beads_dir` that literally contains a
  `ParentDir` component, which `validate_sync_path` then rejects (`path.rs:223-234`) — so the DB
  write succeeds, the export is dropped with a WARN nobody sees, exit 0, and DB↔Org diverge
  permanently.
- **`--db` + redirect cross-wires the hooks.** `run_auto_flush` discovers *with* redirects
  (`main.rs:261`) but opens `overrides.db`, so `obr --db M/.beads/beads.db create …` run from a
  redirected directory exported **M's entire database into A2's git-tracked `issues.org`** while
  leaving M's own file stale. This is R8's mechanism with a second steering input.
- **`obr init` ignores redirects** (`init.rs:21`), happily creating a second, permanently
  shadowed database next to the redirect file.
- **Redirect errors are masked** by `let Ok(..) else` in `where.rs:33` ("No beads directory
  found. Run `br init`"), `doctor.rs:840`, `orphans.rs:49`, and `config.rs:92/105/418/585`
  (`obr config list` prints defaults, exit 0). Only `list`/`create` surface the real
  `CONFIG_ERROR`.

**Fix:** validate the output of `follow_redirects` — `validate_no_git_path`, canonicalize, reject
`..`, and either require containment or print a one-line notice naming the redirected directory
on every mutating command. Make `max_depth` exhaustion an error. Add `redirect` (and
`.br_history/`) to `init`'s `.gitignore`. Validate `db_path` before `SqliteStorage::open`.

---

### R29 — `obr agents` rewrites files outside the repo, and destroys them in five distinct ways
**Severity: critical. [GAP-FILL] — 14 live experiments, all reproduced. True rank: beside R3.**

`src/cli/commands/agents.rs` (1,054 lines) is the **only** subcommand that reads, creates,
overwrites or truncates files outside `.beads/`. It imports nothing from `crate::sync`
(`:6-12`): no allowlist, no `validate_no_git_path`, no traversal check, no canonicalization, no
temp+rename, no fsync. It resolves its target with `detect_agent_file_in_parents(work_dir, 3)`
(`:366`) over an **inclusive** `0..=max_levels` (`:228`) — so `$PWD` plus **three** ancestors —
matching `AGENTS.md`, `CLAUDE.md`, `agents.md`, `claude.md` (`:25`), then whole-file
`fs::write`s (`:549`, `:633`, `:718`). It needs no beads project at all. The global `--db` flag
is ignored; there is no `--path`.

*Confirmed escapes.* A single `obr agents --add` run from `work/myrepo/src/deep` rewrote
`work/AGENTS.md` — a **sibling project's parent directory**, outside
`git rev-parse --show-toplevel` — and dropped a stray `.bak` there. From inside `repo/.git/hooks`
it created `repo/.git/hooks/AGENTS.md`, exit 0, in direct contradiction of NGI-3. With
`proj/AGENTS.md` symlinked to `secret/other.md`, the **link target** was rewritten while the
`.bak` was written next to the *link* — so recovery requires the operator to notice the
indirection. Practical reach: **`~/AGENTS.md` and `~/CLAUDE.md` are in range from any repo ≤3
levels below `$HOME`**, which is the normal `~/src/foo` layout.

*Five data-loss classes, all exit 0, all printing success:*

1. **Non-UTF-8 file → entire content replaced by the blurb.** `check_agent_file:201-208` returns
   a detection with `content: None` when `fs::read_to_string` fails; `execute_add:489` does
   `detection.content.clone().unwrap_or_default()`. So an undecodable file is treated as *found
   but empty* and overwritten. Measured: a 114-byte file with one `\xe9` byte became 2,076 bytes
   of pure blurb, no warning. A follow-up `--remove` then overwrote the `.bak` with the blurb and
   left `AGENTS.md` at **0 bytes** — original unrecoverable, two commands, both "successful".
2. **`remove_blurb` deletes everything between an unrelated marker mention and the first end
   marker.** `:263` finds the start marker, `:267` finds the end marker **with a fresh
   `content.find` over the whole string** — no ordering check, no `start < end` assertion, no
   scoping. Measured: an `AGENTS.md` that merely *documents* the beads integration went 292 → 64
   bytes, silently destroying two whole sections and truncating a sentence mid-clause.
3. **Exponential growth / non-idempotence.** When the end marker appears *above* the real blurb,
   `&content[..start] + &content[end..]` re-emits the region twice and the blurb survives:
   308 → 356 → 452 → 644 → 1028 bytes over four identical runs, doubling forever. The `.bak`
   captures the corrupted state from run 2 onward.
4. **False success.** With the end marker hand-deleted, `--remove` returns early and changes
   nothing while printing "Removed beads workflow instructions from: …", exit 0. With two blurbs
   (the post-merge shape) only the first is removed. With a legacy `bv` blurb present,
   `execute_remove:587-591` takes the legacy branch and the current blurb survives. Several of
   these also weld two lines together (`# Header` + the next line), destroying a heading.
5. **Backup failure is non-fatal.** `fs::copy` failure → `eprintln!("Warning: …")` → the
   destructive write proceeds anyway (`:535-547`). The backup name is fixed (`.md.bak`), so the
   second run clobbers the only pristine copy; and `*.md.bak` is gitignored nowhere, so
   `git add -A` sweeps it up.

Even the happy path is lossy: `--add` then `--remove` on a clean file eats the trailing newline
(`:277-279` consumes all newlines after the marker, `:283-286` restores at most two before it).

*Agent-facing behaviour is broken in both directions.* Without `--force`, `--remove` and
`--update` **always** prompt (`:611`, `:696`) — stdin at EOF gives "Aborted." and **exit 0**;
stdin as an open pipe **hangs** (`exit 124`). With `--json`, `execute_json` takes `_args`
(`:397`) and never reads it, so `obr --json agents --add` reports `"needs_blurb": true` and
exits 0 **having done nothing**. There is no flag combination under which an agent gets a
machine-readable, non-blocking, mutating `obr agents`. Flags also silently swallow each other:
`--add --check` performs no add; `--add --remove` performs the add and never mentions the drop
(no `conflicts_with` anywhere in `AgentsArgs`, `cli/mod.rs:2367-2391`).

*What it injects.* 2,076 bytes into a file whose entire purpose is to be obeyed by an AI agent:
**19 `br …` invocations (all `exit 127` — `br` is not on PATH) plus one `bd …`**, the wrong
upstream URL (`github.com/Dicklesworthstone/beads_rust`, `:34`), **"Export DB to JSONL" twice**
for a store that produces only `issues.org`, and a mandatory end-of-session checklist of
`git status` / `git add` / `git commit` / `git push` — handing to the *agent* precisely the
automation obr's headline property disclaims at the process level. `BLURB_VERSION` is frozen at 1
(`:16`) through both migrations, so `needs_upgrade()` (`:127-132`) returns false forever and **no
deployed copy will ever be told it is stale**. Nine more `br agents …` strings appear in the
command's own help output (`:432/443/449/452/459`, `:757/766/776/786`).

*Assurance:* **zero.** `grep -rln agents tests/` matches one help snapshot. `grep -c execute`
inside the `#[cfg(test)]` block: **0** — `fs::write`, `fs::copy` and `stdin().read_line` are
never exercised at any level. The nine unit tests cover pure functions only, and
`test_remove_blurb` asserts three `contains` predicates with no byte-equality, which is why every
corruption class above passes unnoticed. `FileTreeSnapshot` has never been aimed here (§4.13).

**Fix (smallest set that retires the top eight risks):** error out when `found() &&
content.is_none()`; search for the end marker *from* `start_idx` and assert ordering; set
`max_levels = 0` or bound at the repo root; treat non-TTY stdin as an explicit refusal with a
non-zero exit (`main.rs:309` already knows how); honour `_args` under `--json`; route the target
through `validate_no_git_path` + canonicalize + temp-rename; make backup failure fatal and give
backups unique names; bump `BLURB_VERSION` and rewrite the blurb for `obr` + Org.

---

### R30 — The parser of the durable artifact is unversioned, unbuildable by anyone else, and the pinned build differs from the local one
**Severity: high (meta / supply chain). [GAP-FILL].**

`Cargo.toml:64`: `org2jsonl = { path = "../org2jsonl" }`. A path dependency records **no version
requirement** — `Cargo.lock:2673-2683` has `name = "org2jsonl", version = "0.1.0"` with no
`source` and no `checksum`. Consequences, each verified:

- **`cargo build` fails for anyone without a sibling `../org2jsonl`.** `.github/` never mentions
  it (`grep -rn org2jsonl .github/` is empty), so every compiling CI job is inoperative — this is
  R14's build failure, with the prior question answered: it is not a missing CI step, it is that
  *nobody but the author can build obr at all*.
- **`cargo audit` / `cargo deny` / SBOM tooling cannot see it**, nor its transitive
  `orgize 0.10.0-alpha.10`. `cargo publish` is impossible while it exists.
- **The committed `Cargo.lock` is already stale.** org2jsonl's worktree added `walkdir` (commit
  `b8fc44d`); the lock does not list it, so `cargo build --offline --locked` **errors**, and
  building without `--locked` silently rewrites the committed lockfile.
- **The flake pins a different parser than cargo compiles.** `flake.nix:21-23` pins
  `git+file:///Users/johnw/src/org2jsonl` — a URL resolvable on exactly one host — at rev
  `5ea7586` / `revCount 15`, while the worktree HEAD is `59521f9`, **commit 30 of 30**. So
  `nix build` and `cargo build` produce **different parsers, both labelled `org2jsonl 0.1.0`**,
  and nothing detects it. The flake "works" only because its pin happens to predate the `walkdir`
  addition — a coincidence, not a mechanism.
- **Zero version compatibility is enforced between a file and the parser reading it.**
  `issues_to_org_text` emits `:BEADS_SCHEMA_VERSION: 1` (`org_bridge.rs:80`) and
  `parse_properties` **explicitly discards it** (`:735-738`, comment claims it is "validated
  elsewhere" — it is not; grep finds no reader). `org2jsonl::SCHEMA_VERSION` is stamped into
  every `OrgEntry` and never read by obr. There is no writer marker, no parser-identity keyword,
  and `content_hash` cannot help because it is computed *from* the parsed issue — a lossy parse
  simply yields a different, self-consistent hash.

*The crate itself is not clean either*, which matters because it is obr's only fidelity oracle:
5 of its 11 own fixtures fail its own `roundtrip.sh`; `Element::Clock` round-trips with a
**duplicated `CLOCK: ` prefix** (the parser stores `clock.raw()` including the prefix at
`org_to_json.rs:680-685`; the writer prepends it again at `json_to_org.rs:465-470`) — and the
unit test that "covers" it hand-constructs a value the parser can never produce; and a **6-byte
input (`-\n* h\n`) grows by one blank line per round-trip, without bound**, falsifying the
crate's README claim and tripping its own `fuzz_roundtrip.rs` assertion, which has never been
run. None of this reaches obr today *only because obr never calls the writer* — but `jsonl2org`
on `.beads/issues.org` is the advertised workflow for the format.

**Fix (recommended minimum):** git-pin with a rev, exactly as obr already does for its other
sibling — `Cargo.toml:54` is
`toon_rust = { git = "https://github.com/Dicklesworthstone/toon_rust.git", rev = "788589d" }`
with a matching `flake.nix:65-67` `outputHashes` entry. One line each; immediately fixes CI,
audit visibility, third-party builds, and the flake/cargo divergence. Then regenerate
`Cargo.lock`, add a CI job that actually builds, and record the parser identity in the file. The
correct long-term answer may be to inline: obr uses 1 of 23 `Element` variants and 14 of 22
`InlineContent` variants, so a purpose-built ~300-line `orgize`→`Issue` reader would cover it and
make the loss surface explicit instead of accidental.

---

### R31 — Import is all-or-nothing with no location information, and has an O(n²) blowup
**Severity: high. [GAP-FILL] — both reproduced.**

*One bad heading bricks the workspace.* `parse_properties` returns `Err` when `:ID:` is absent
(`org_bridge.rs:742-748`) and `org_text_to_issues` propagates with `?` (`:347`). There is no
per-heading recovery. Appending `* TODO [#C] orphan heading with no ID` to a healthy file makes
**every command in the workspace fail**, including `obr show <unrelated-healthy-id>`:

```
{"error":{"code":"VALIDATION_FAILED","message":"Validation failed: id: Missing required :ID: property","hint":null,…}}
```

No line number, no heading title, no file offset — and as established in §2.3 obr **cannot**
supply one, because `org_to_entries_with_keywords` hard-codes `file: None` and location metadata
is only populated when `file` is `Some`. For a format whose selling point is that humans edit it
in Emacs, adding a `* Notes to self` heading is a workspace-wide outage with an unactionable
error.

*Quadratic parse.* A description containing repeated `[[` (unbalanced link openers — the shape a
large paste or a bad merge produces) drives `orgize`'s link scanning quadratically. Measured
through obr's ordinary auto-import path (`obr list`, release binary): 25 KB → 0.83 s;
50 KB → 3.22 s; 100 KB → **12.73 s**; 200 KB → 50.96 s. Doubling the input quadruples the time.
Extrapolated, a 1 MB `issues.org` of this shape costs ~20 minutes **per obr invocation**, and
`.beads/issues.org` is git-tracked and agent-editable.

*Two more accepted-but-wrong inputs:* 100 level-1 headings sharing one `:ID:` import without
complaint (last wins — the R21 shape, confirmed for Org); and a literal NUL byte inside a heading
title is accepted, stored, and re-emitted. Also, obr writes `#+SEQ_TODO:` (`org_bridge.rs:33`)
and then **ignores it on read** — the parse always uses the hardcoded `BEADS_TODO_KEYWORDS`
(`:312-324`), so `* REVIEWING [#B] hand written` parses as title `"REVIEWING [#B] hand written"`,
status `open`, priority reset to 2. `status_to_keyword`'s `Status::Custom(s) => s.to_uppercase()`
(`:271`) is by construction not in that list, so any custom status that reaches the exporter
round-trips into the title and silently becomes `Open`.

**Fix:** collect per-heading errors instead of `?`-ing the first; add
`org_to_entries_with_keywords_and_source` upstream (or thread the existing `file: Some(...)`
path) so errors carry a line number; bound or reject pathological link inputs; honour
`#+SEQ_TODO:` or stop emitting it.

---

### R32 — The backup mechanism is inert on the path every agent mutation takes, and `history restore` is a path-traversal primitive
**Severity: high. [GAP-FILL] — reproduced at DEBUG level, by A/B, and at scale. True rank: beside R3.**

`.beads/.br_history` + `obr history {list,diff,restore,prune}` is obr's only snapshot-and-restore
mechanism, and **auto-flush never writes to it.**

*Mechanism.* `run_auto_flush` calls `discover_beads_dir(Some(Path::new(".")))` (`main.rs:261`),
which returns the **relative** `./.beads`. The backup call site is gated on
`output_abs.starts_with(beads_dir)` (`sync/mod.rs:1276-1286`) where `output_abs` has been forced
absolute. `starts_with` is component-wise, so **an absolute path can never start with a relative
one — the branch is dead.** Three proofs: (a) `obr -vv create` logs `beads_dir=./.beads` and
never emits `history.rs:94`'s "Created backup"; (b) the same workspace with an absolute
`BEADS_DIR` *does* produce a backup, the same command without it does not; (c) **244 concurrent
`obr create` calls across 8 bursts produced exactly zero new backups.**

So `.br_history` exists only if the user runs `obr sync` explicitly. Coverage by operation:
`sync --flush-only` yes · `sync --merge` yes · a *refused* export yes (the backup at `:1285`
precedes the guards at `:1293-1340` — so the snapshot is of the already-bad file) ·
auto-flush **no** · `sync --import-only` **no** · `history restore` **no** · the SQLite DB
**never**.

*Even when it fires, three limits compound.* Filenames are `{stem}.{%Y%m%d_%H%M%S}.{ext}`
(`history.rs:72-78`) written with an overwriting `fs::copy` (`:93`): measured, **40 export cycles
in 3.4 s retained 5 files — 87.5% of snapshots silently destroyed**, with a 17-mutation gap
between the first two survivors. `HistoryConfig::default()` is `max_count = 100` /
`max_age_days = 30` (`:22-29`), enforced (151 planted → 100 after one rotate) — but 100 files
means 100 *distinct seconds containing an export*, not 100 mutations. And
`files_are_identical` (`:83-91`) skips the backup when the file equals the newest one, which
fires **precisely when a losing rename has moved the file backwards** — reproduced: the one state
worth keeping was never captured. Structurally, `backup_before_export` snapshots the file *about
to be replaced*, so **the current good state is never in history**.

*The restore path is unsafe and misdirected.* `restore_backup` is four operative lines
(`history.rs:240-257`): `history_dir.join(filename)` with an **unsanitized user string**, an
existence check, and `fs::copy`. `Path::join` with an absolute path replaces; with `../` it
escapes. All three reproduced — `obr history restore /…/evil.txt --force`,
`obr history restore ../../../evil.txt --force`, and
**`obr history restore ../../.git/config --force`**, each copying an arbitrary readable file over
the git-tracked `.beads/issues.org`. It is an arbitrary-read → durable-artifact-overwrite
primitive (the write target is fixed), and the `.git/config` case directly violates the invariant
`src/sync/path.rs:31-35` declares as hard. `diff_backup` has the identical unvalidated join at
`:137`, giving arbitrary-file-read-to-stdout (`obr history diff /etc/hosts` prints it). No test
covers this across 2,158 lines of history tests. Restore also takes **no snapshot of the file it
destroys**, so restoring the wrong backup is unrecoverable from within obr; and the follow-up it
prescribes — `obr sync --import-only --force`, which sets `force_upsert` (`sync.rs:937`) — rolls
back field edits on issues present in both but **never deletes DB rows absent from the file**, so
the result is a *union*, not a rollback. Its own next-step string names `br`
(`history.rs:265, 278, 286`), a binary that does not exist.

*Why 2,158 lines of tests missed all of it.* Every history e2e test creates issues with
`--no-auto-flush` and then calls explicit sync, with the comment at `tests/e2e_history.rs:39-40`
explaining that this is to stop auto-flush from clearing dirty flags. **The suite routes around
the only code path a real user takes.** Eleven `thread::sleep(1100 ms)` calls across the history
tests normalize the one-second granularity limit as a workaround rather than treating it as a
defect.

*Two more:* `HistoryConfig` is never configured — all three production constructions are
`::default()` (`sync.rs:584`, `:1219`, `sync/mod.rs:1942-1946`) — yet **`obr config set
history.enabled false` is accepted, persisted to `config.yaml`, echoed back by `config get`, and
ignored** (a flush after setting it still backed up). And `obr init`'s `.gitignore` omits
`.br_history/`, so up to 100 near-complete copies of the issue database get committed
(~50 MB in one test workspace), leaking closed and deleted issue content into git history — while
obr's *own* `.beads/.gitignore` excludes it, and `PLAN_TO_PORT…md:892` promised init would too.

**Fix, in order of impact ÷ cost:** (1) canonicalize `beads_dir` in `run_auto_flush`, or compare
canonical paths at `sync/mod.rs:1284` — **one line, and it turns the whole mechanism on**; ship
the test the suite avoids. (2) unique backup filenames (PID + counter, or sub-second precision)
with an `O_EXCL` copy so collisions error rather than overwrite. (3) put the *situation-specific*
remediation into the error text (see R5 and R25). (4) snapshot before `history restore`, before
`import --force` (a `beads.db` copy — the only durable protection for the audit trail the file
format cannot carry), and before `delete --hard`. (5) validate the `file` argument in
`restore_backup` and `diff_backup`. (6) add `.br_history/` to init's `.gitignore`.
(7) implement or reject the `history.*` config keys.

---

## 6. Divergences between intended design and implementation

The port was specification-first and the specifications are good. They have simply not been
maintained through two migrations (JSONL → Org, `br` → `obr`), neither of which was propagated.

| Claim | Where | Reality |
|---|---|---|
| Storage format is JSONL | README, `AGENTS.md`, `CLAUDE.md`, all of `docs/`, all of `.beads/*.md`, `skills/` | Default is **Org-mode**. `rg 'issues\.org\|org.mode'` across all prose returns **zero hits**. `src/sync/org_bridge.rs` (900 lines) is entirely undocumented. |
| Binary is `br` | `AGENTS.md` (47×), `CLAUDE.md` (10×), `VCS_INTEGRATION.md`, `docs/agent/*`, `CLI_SCHEMA.json` (`"binary":"br"`), `ROBOT_MODE_EXAMPLES.jsonl` (all 3 `cmd` fields), and **error hints** (`structured.rs:624`: "Run 'br list'") | Binary is `obr`. `scripts/agent_smoke_test.sh` exposes the confusion: `elif command -v br …; then BR="obr"`. **An agent following the machine-readable examples verbatim gets command-not-found.** |
| "Never executes git commands" | `README.md:140`, `CLAUDE.md:52` | **[SYNTH-VERIFIED]** false as written: `comments.rs:352`, `orphans.rs:234,245`, `changelog.rs:290`, `stats.rs:381`. All read-only, so the *spirit* holds; the enforced check greps only `src/sync/`. |
| `obr list --status open --priority 0-1` | `README.md:57` | Fails: `INVALID_PRIORITY … got: -1`. Ranges need `--priority-min/--priority-max`. |
| Nested YAML config (`id.prefix`, `defaults.priority`, `sync.auto_flush`) | `README.md:79-89` | Not recognised. Real keys are flat (`issue_prefix`, `default_priority`). **Nasty trap:** `obr config get id.prefix` *returns* the value (YAML dot-flattening) while ID generation ignores it — the wrong example appears to verify. |
| Four output modes | `CLAUDE.md`, `AGENTS.md:386-406`, `src/output/mod.rs:6-14` | **Five** (`Rich, Plain, Json, Toon, Quiet`) since TOON landed. |
| `ALLOWED_EXTENSIONS = [.jsonl,.json,.db,.yaml]` + `config.yaml` | `docs/ARCHITECTURE.md:339-340` | Actual: `db, db-wal, db-shm, jsonl, jsonl.tmp, org, org.tmp` + `.manifest.json, metadata.json` only. |
| Table `blocked_cache` | `docs/ARCHITECTURE.md:249,270` | `blocked_issues_cache`. Doc also omits `metadata`, `export_hashes`, `child_counters`. |
| `rayon` is a dependency | `docs/ARCHITECTURE.md:666` | Not in `Cargo.toml` nor anywhere in `src/`. Same for `parking_lot` and TOML config prescribed by `PROPOSED_ARCHITECTURE`. |
| Per-type module split; `storage/queries/`; `src/context.rs` | `PROPOSED_ARCHITECTURE:80-176`, `AGENTS.md:265` | Two 5k-line god modules; no `storage/queries/`; no `src/context.rs`. `docs/ARCHITECTURE.md:38` says ~33k lines; actual ~53.5k. |
| `config` is flag-style (`-l/--list`, `-g/--get`) | `docs/CLI_REFERENCE.md:666-702` | Subcommand-style since `da715c5` (2026-01-17). `obr config --list` errors. **Seven months of an agent-facing reference documenting an interface that errors on invocation.** |
| Merge base is `.beads/base_snapshot.jsonl` | `src/cli/mod.rs:2058` (the `--merge` help itself) | Code uses `beads.base.jsonl`. |
| `graph`, `agents`, `schema`, `status`, `sync --merge`, `sync --rename-prefix` | — | Undocumented in `docs/CLI_REFERENCE.md`. |
| No compatibility shims ("no users, no tech debt") | `AGENTS.md:116-121` | `PROPOSED_ARCHITECTURE:3447` prescribes them; the Org migration shipped exactly one (`config/mod.rs:100-118`). Neither doc reconciled. |
| NGI-2: "no git libraries in Cargo.toml" | `.beads/SYNC_SAFETY_INVARIANTS.md:93` | `vergen-gix` (gitoxide) is a **build**-dependency; `UPGRADE_LOG.md:45` acknowledges pulling in gix-* crates. Build-time only, so defensible — but the invariant as phrased is violated. |
| 781 unit tests | `CLAUDE.md:30` | 788 in `src/`; 1,541 more in `tests/`. |
| `--priority`, `no-auto-flush`, `no-auto-import`, `json` config keys | `config/mod.rs:1046-1049` classifies them as startup keys and round-trips them | `main` reads only `cli.no_auto_flush` / `cli.no_auto_import` / `cli.json` — setting them in YAML or `BD_*` has **no effect**. Same for `claim-exclusive`, which is unreachable because it is absent from `is_startup_key`'s list. |

**[ADDENDUM] — eight more, found in the gap-fill round:**

| Claim | Where | Reality |
|---|---|---|
| `.beads/.gitignore` excludes `redirect` and `.br_history/` | this repo's own `.beads/.gitignore` (both present, with explanatory comments); `PLAN_TO_PORT…md:892` promises `.br_history/` | `obr init`'s template (`init.rs:84-95`) lists neither. Every workspace obr creates commits its own backups and its own redirect file; the maintainers' workspace does not. (R28, R32) |
| Backups are created on every export | `docs/SYNC_SAFETY.md:95-97`, `docs/ARCHITECTURE.md:179` ("2. Create history backup") | Never on auto-flush (R32). `docs/CLI_REFERENCE.md:781` happens to be right by accident. |
| `history.enabled` / `max_count` / `max_age_days` are configurable | `PLAN_TO_PORT…md:895-897` | Not implemented. `obr config set` accepts, persists and echoes them, then ignores them (R32). |
| Corrupt file → `br history list` / `br history restore <backup>` | `docs/TROUBLESHOOTING.md:391-393` | Wrong tool *and* wrong binary: the fix is `obr sync --flush-only --force` when the DB is intact, and `git checkout` when it is not (R5, R32). |
| `history restore` / `history prune` have no e2e coverage | `docs/E2E_COVERAGE_MATRIX.md:175-179, 239-240` | Stale in the *other* direction: `tests/e2e_history_restore_prune.rs` has 18 tests. Also `docs/CLI_REFERENCE.md:770-783` documents only `list` and `restore`, omitting `diff`, `prune` and `--force`. |
| `tests/e2e_routing.rs` covers "External DB reference safety", "Redirect loop detection", "Prefix-based route lookup" | `tests/e2e_routing.rs:1-8` | The three "safety" tests are all about `--db`; there is **no** loop-detection test; the route tests are vacuous because routing is dead. The comment at `:231` — "The redirect is used during route resolution, not BEADS_DIR discovery" — is **factually wrong** (`config/mod.rs:233`), and is precisely the belief that hid R28. |
| `obr where` shows the active `.beads` after redirects | `docs/CLI_REFERENCE.md:740` | It follows the chain a *second* time (`where.rs:37`), so beyond `max_depth` it reports a directory no other command uses (R28). |
| The `agents` command exists | — | Undocumented everywhere: `grep -rn "obr agents\|br agents" --include=*.md .` matches nothing outside `target/`. Its only doc-corpus appearance is one clap-generated help line in a snapshot. |

**Two more that are actively hazardous, not merely stale:**

- **`.beads/README.md` is unmodified upstream Go `bd init` boilerplate** instructing users to run
  `bd sync`, with "Auto-syncs with your commits" and "Automatic sync with git commits" — the
  exact behaviour that destroyed this repository's source tree on 2026-01-19 and the reason the
  entire sync threat model exists. These are explicit non-goals (NG-1..NG-8).
- **`obr upgrade` is a live footgun for the fork**: `src/cli/commands/upgrade.rs:15,18` hard-code
  `REPO_OWNER="Dicklesworthstone"` / `REPO_NAME="beads_rust"`, so running it silently replaces
  John's `obr` with Jeffrey's `br` — **a different tool with a different storage format.**
  (The critic checked and confirmed the self-updater is *not* an unsigned-download hole:
  `upgrade.rs:249,268` pass `.verifying_keys(...)`. The wrong-repo target is the whole defect.)
- **[ADDENDUM] `AGENT_BLURB` is drift that obr actively propagates into other people's
  repositories** (R29). Every doc-drift item above is passive — a stale file someone might read.
  This one is *generated at runtime and written into third-party `AGENTS.md`/`CLAUDE.md` files*,
  which agent harnesses load as system-level instruction. It contains 19 `br` invocations that
  exit 127, one `bd` invocation, the wrong upstream URL, "JSONL" named twice for an Org-mode
  store, and a mandatory `git add`/`commit`/`push` checklist. `BLURB_VERSION` is frozen at 1, so
  the drift is self-perpetuating: no deployed copy will ever be flagged stale. The dossier's
  model of "stale docs" does not cover this, because this document does not exist until obr
  writes it.

**Meta-observations.** The newest document is among the least accurate: README.md was rewritten
2026-03-16, a month *after* the Org migration, and was clearly rewritten from the old README's
prose rather than from the code. `skills/bd-to-br-migration/` exists solely to mechanically
migrate *other* repos' docs from `bd` → `br`, and was never run against this repo's own
`br` → `obr` rename. And **the project stopped dogfooding its own tracker exactly when the Org
migration landed**: `.beads/` is still JSONL (549 issues, 545 closed, 3 open) and untouched since
2026-02-18 — so the new Org code path gets no real-world exercise from the project itself.

Git-tracked cruft never mentioned anywhere: `custom.db` (176 KB SQLite), `temp_test/.beads/` and
`temp_test_2/.beads/` — including a **`daemon.log`**, from a tool whose entire design premise is
"no daemon".

---

## 7. Open design questions

These are the questions a designer should be able to answer before the next significant change.
Each sketches the tradeoff landscape rather than prescribing an answer.

### Q1 — Should the wire unit stay a whole-issue snapshot, or become field-level deltas?

This is the root question; R3, R11, R21 and most of §3.2 descend from it.

*Keep snapshots.* Cheapest. The file stays human-readable and diffable in the obvious way (one
heading = one issue). Import stays a simple upsert. Compatible with bd's JSONL for conformance.
Costs: record-level replace forces cascade loss (R3); concurrent edits to *different fields* of
one issue cannot both survive; every survivor in the field decomposed below record level for
exactly this reason.

*Field-level deltas.* Correct by construction for concurrent disjoint edits; set-valued fields
(labels, deps) become OR-sets with explicit add/remove rather than replace-the-list, which is the
only way concurrent label additions stop clobbering each other. Costs: the file stops being a
readable snapshot and becomes a log; a human can no longer edit an issue by editing its heading —
which is precisely the workflow the Org migration exists to enable; conformance with bd's JSONL
is lost.

*Hybrid (probably the real answer).* An append-only `.beads/events.jsonl` (comments, status
transitions, dependency edges) plus a **derived** `issues.org` materialized view. This is the
Fossil / git-bug / Radicle shape. It makes "append-only" actually true for the log, at which
point concurrent appends genuinely commute; the view file can stay hand-editable with edits
converted to appended events at import. Costs: two files, two merge stories, and the view must be
regenerable byte-identically or diffs churn.

**[ADDENDUM] — the hybrid's hard prerequisite is already satisfied, and the escape-hatch question
is now answered empirically.** "The view must be regenerable byte-identically" holds today:
emission is deterministic across runs and processes, with no map iteration reaching the output
(§4.15). What does *not* hold is the round trip: **9.4% of descriptions change on the first
flush→import pass and 1.06% change again on the second** (R23), so the view is not currently a
fixpoint of its own pipeline. Any hybrid design must fix the *reader*, not the writer.
And on hand-editing: it is currently **advertised and broken three ways** — R11's equal-timestamp
discard, R23's silent structure destruction, and R31's "one heading without `:ID:` bricks every
command in the workspace, with no line number". A human who adds `* Notes to self` to
`issues.org` takes the workspace down. Answer this question before anything else in §7; it
determines whether R23 and R31 are P0 or cosmetic.

### Q2 — Wall-clock LWW, monotone clocks, or logical clocks?

*Status quo.* Zero cost, silently destroys work under clock skew (R11), and cannot express
"these two edits are concurrent."

*Monotonize + bound.* Write `max(now, existing + 1ns)`; reject `updated_at > now + skew`. No
format change, no new fields, removes the backwards-clock and pinned-future failure modes.
Does **not** make the ordering meaningful across machines — it only stops the pathologies.

*Additive Lamport.* Add `lamport: u64` (default 0) and `writer_id`; order by
`(lamport, writer_id, updated_at)`. Backward compatible: old records default to 0 and fall back
to current behaviour. This is git-bug's rule exactly. Costs: two more fields on the wire; every
mutator must bump the counter; and in Org they must be emitted *after* `:UPDATED_AT:` (or
`:UPDATED_AT:` moved to the end of the drawer) or the accidental-conflict property worsens.

*Full vector clocks / CRDT.* Correct concurrency detection, but a large step: needs per-replica
identity, garbage collection, and a conflict *representation* (Q3).

*Question to answer first:* how many writers realistically share one issue? If it is "one agent
per issue, occasionally two", monotonize+bound may be sufficient forever and Lamport is
over-engineering. If it is "a swarm", Lamport is the floor.

### Q3 — Should conflicts be fatal, or representable?

Today an unresolvable state either aborts the import (conflict markers) or is silently resolved
by LWW. jujutsu's model — record a *conflicted* state, succeed, keep both sides queryable
(`obr list --conflicted`), resolve later as an ordinary mutation — is strictly more informative.
The critical property to copy is **idempotence**: repeated syncs of a conflicted issue must not
accumulate nested conflict records (jj achieves this because its conflict algebra cancels
redundant terms). Cost: a new state in the model, in the schema, in the Org format, and in every
query predicate — plus a decision about whether a conflicted issue is "ready".

### Q4 — Is content-hash dedup a feature or an accident?

R6 shows it destroys independently created issues, is non-confluent across clones, and wedges the
exporter. But it exists for a reason: `--rename-prefix` cross-repo import needs to recognise
"this is the same issue under a new id", and the machinery to preserve the absorbed id into
`external_ref` already exists (`:2352-2354`).

*Options:* (a) restrict phase 2 to the `--rename-prefix` flow; (b) keep it globally but make it
deterministic, non-destructive (preserve the id), and honestly reported ("deduplicated", not
"skipped (up-to-date)"); (c) delete it and rely on `external_ref` alone.

*Note the asymmetry git-bug discovered:* content-hash collision is a **bug** for issues (two
agents legitimately filing the same title) and **exactly the desired dedup** for comments and
events. obr applies one policy to all three.

### Q5 — Should obr track upstream's returning SQLite seam, or stay a fork-in-amber?

Upstream's unreleased changelog states "embedded Dolt, Dolt server, and SQLite are the supported
storage paths", behind a capability-gated seam (six core traits, 33 data methods, capability
traits defaulting to absent, Dolt-only commands stubbed) — designed off a **Rust spike that hit
299/299 byte-parity**.

*Stay frozen.* Zero cost; obr is already a coherent product; the conformance suite is already
pinned to v0.46 semantics in practice. But then the conformance suite must be honestly retargeted
(R15): pin a tag, add a max-version bound naming the Dolt pivot, and move the 13 divergences into
docs as design positions rather than `#[ignore]` strings.

*Track the seam.* Mirroring the seam shape and the differential-conformance harness is the
cheapest path to relevance if obr ever wants to be more than a fork. Cost: a trait boundary obr
currently does not need (one backend, YAGNI), and ongoing chase.

*Minimum either way:* make the importer **forward-tolerant**. Modern `bd export` emits an
optional `{"_schema":"beads-jsonl/1"}` header, `_type`-discriminated lines, and a `wisp_plane`
marker; upstream now has **10 dependency types** and custom issue types from config. Skipping an
unknown `_schema` header, ignoring `_type:"memory"` lines, and passing an unknown dep type
through as non-blocking metadata buys one-way readability of any modern bd export at near-zero
cost — and prevents any repo that ever saw a newer bd from becoming permanently unimportable.

### Q6 — Where should the export/flush policy live?

Auto-flush-after-every-mutation (R16) is the cause of the concurrency corruption (R5), the
32,767 ceiling exposure (R7), the cross-workspace contamination (R8), and most of the write-path
cost. The daemon+debounce that upstream used to solve it is a stated non-goal.

*Options:* (a) incremental export — the machinery exists and is unwired (`get_issues_needing_export`),
but rewriting a *sorted* file incrementally means either a read-modify-write of the whole file
(same cost) or an append-only log (Q1); (b) a debounce implemented as a lock file + timestamp,
with no daemon — flush only if the last flush was >N ms ago, deferring otherwise; (c) explicit
flush only, with a loud dirty-count indicator; (d) keep it but make it safe (transaction + unique
temp name), accepting the O(N) cost.

*Question to answer first:* what is obr's target scale? 549 issues (this repo) makes (d) fine
forever. The bench targets say 10k, at which (d) costs 1.2–1.7 s per mutation. Nothing in the
repo states the answer.

### Q7 — What is the enforcement boundary for the mutation protocol?

The protocol is the flagship invariant and is enforced only by a 5,157-line module boundary
containing 12 bypasses (R26). Splitting the file makes it compiler-enforced — but the deliberate
bypass (`upsert_issue_for_import`) then needs an explicit, named, reviewable escape hatch rather
than "it's in the same file". What should that look like? A `#[doc(hidden)] pub(super) fn
raw_write` on a `ConnHandle`? A second `import_mutate` that does steps 1 and 4 but not 2 and 3
(which would, incidentally, have prevented R3)?

### Q8 — Should the CLI contract be frozen, and at what version?

The agent-CLI literature converges on treating the CLI as the semver public API: additive-only
after 1.0, breaking changes only behind `--format-version 2`, exit codes as a stability
commitment. obr is at 0.1.14 with an unpublished exit-code table, a `--robot` flag that is a
usage error on 29 of 39 commands, no output envelope, and no version field. The window to make
breaking corrections (global `--robot`; non-TTY JSON for *success* output, not just errors; a
`{data, metadata, errors}` envelope; `--fields` on read commands; NDJSON for lists) is now,
before there are users. Each of those is individually cheap and collectively a 1.0 boundary.

*Note the tension with agent expectations:* Arcjet's position is that after 1.0 the contract must
be **additive-only forever**, because agents replay cached command shapes. That argues for making
the breaking changes *now* and then freezing hard, rather than evolving gradually.

### Q9 — What is the recovery story? **[ADDENDUM — this question was answered wrongly; rewritten]**

The original answer below asked the wrong question, because obr already ships a
snapshot-and-restore mechanism that nobody had evaluated. Having evaluated it, the conclusion
**inverts**: the highest-value work is not a new feature, it is three small fixes to the
mechanism that exists.

*The original framing, retained for the record:* "Several confirmed defects leave a workspace in
a state a user cannot diagnose or exit: the export wedge needing `--force` the user cannot know
to use (R6); a corrupted file needing hand-editing (R5); a permanently-blocked dependency cycle
(R12/R13); destroyed audit history with no second copy (R3). `obr doctor` certifies all of them
as healthy (R25). A `doctor --repair` that (a) diffs per-issue content hashes between DB and
file, (b) runs `detect_all_cycles`, (c) checks for orphan rows, (d) verifies the file parses, and
(e) **exits nonzero** would turn most of §5 from silent corruption into a diagnosable state.
That is probably the single highest-value *new* feature in this dossier."

**What is actually true (R32, R5):**

1. **The remediation for the two worst file-side defects already exists and is one command:**
   `obr sync --flush-only --force`. The DB survives R5's corruption and R6's wedge; the file is
   derived; re-deriving it is exact and lossless. Verified on a 2,245-issue corrupted workspace
   and on a deliberately wedged one. Nothing in obr's output names this command, and `--force` —
   which is *mandatory*, since the dirty flags are already clear — is nowhere suggested.
2. **`obr history restore` is the wrong tool for all three defects, and for the wedge it is
   actively harmful:** the only backup available was taken *of* the wedged file (backups run
   before the guards), so restoring reinstalls the wedge, and the prescribed
   `sync --import-only --force` then cannot roll back creations, leaving a union.
3. **The mechanism is inert where it matters.** Auto-flush — 100% of agent mutations — never
   creates a backup at all (R32). Where it does fire it loses ~88% of states to same-second
   filename collisions, can skip the exact snapshot you need via dedup-against-latest, never
   captures the current good state, never snapshots the DB, and never captures events.
4. **The one case that is genuinely unrecoverable is the fresh clone** (DB gitignored, file
   corrupt) — and there the R5 playbook *destroys the remaining data*, because `--force` bypasses
   the empty-DB guard. The correct action is `git checkout .beads/issues.org`, which obr will
   never say. **obr's output does not distinguish this case from case 1, and they demand
   opposite commands.**

*Revised answer.* Ranked by impact ÷ cost:

- **(a) One line:** canonicalize `beads_dir` in `run_auto_flush` (`main.rs:261`) or compare
  canonical paths at `sync/mod.rs:1284`. Backups begin to exist. Ship the test the suite avoids.
- **(b) Unique backup filenames** + `O_EXCL`. Recovers the 87.5% of snapshots currently
  destroyed, and deletes 11 `thread::sleep(1100)` calls from the test suite.
- **(c) Three error strings**, each attached to a condition already detected: the org parse
  failure gets a hint that *branches on whether the DB is intact*; the stale-DB guard's message
  (already the best in the codebase — it names the offending id and gives a hint) simply stops
  being swallowed at `debug!`; `doctor` promotes one-directional `counts.db_vs_jsonl` divergence
  to ERROR and attaches the same text.
- **(d) Validate `history restore` / `history diff` arguments** (five lines reusing
  `src/sync/path.rs`), and snapshot before every destructive operation that currently does not —
  including a `beads.db` copy before `import --force`, which is the **only** possible durable
  protection for the audit trail, since the file format cannot carry events.

*And on `doctor --repair`:* the detection half (a)–(e) above is still right and still worth
building. But the hard part was never the repair — it is deciding **which side to trust**, and
that judgement cannot be automated safely: §R5's fresh-clone case shows the wrong choice is
unrecoverable. `doctor --repair` should be a convenience wrapper over a mechanism that works,
not a replacement for one that never did. Items (a)–(d) are together well under 100 lines.

### Q10 — Should the `.beads/` file be readable from a bare repository?

git-bug's inability to run in a bare repo blocks all server-side use, and essentially all git
hosting is bare server-side. obr's file-in-working-tree design means a bare clone has no
`.beads/issues.org` at a path — but `git show HEAD:.beads/issues.org` does. Preserving "obr can
read the file from a stream / a given path, with no `.beads` discovery" is far cheaper than
adding it back, and it is the prerequisite for a CI validation check, a server-side render, or
the zero-install human view that the 2013 field survey identifies as the single most-cited cause
of death in this category.

### Q11 — **[ADDENDUM]** What is obr's write perimeter, and who is allowed to move it?

The gap-fill round turned a stylistic question into a design one. obr has *four* distinct
write-path policies and no stated boundary between them:

| Path | Policy |
|---|---|
| `obr sync` export/import | Full `src/sync/path.rs`: allowlist, `.git` rejection, traversal + symlink checks, temp+fsync+rename+chmod |
| SQLite open | **None** (`config/mod.rs:315`, `:412`) |
| `obr agents` | **None** — 3-ancestor walk, symlink-following, whole-file `fs::write` |
| `obr history restore` / `diff` | **None** — unsanitized `join` |

…and the *anchor* for the first row is chosen by an untrusted, git-committable file
(`.beads/redirect`). The questions a designer must answer: Is `.beads/redirect` a trusted input
or an untrusted one? (The Go tool's answer was "never commit it"; obr's `init` dropped that
line.) Should `beads_dir` be canonicalized and containment-checked once, at the single point
where it is produced, so every downstream consumer inherits the guarantee? Should `obr agents`
exist at all in a tool whose value proposition is non-invasiveness — and if it must, should it
take an explicit `--path` rather than searching an ancestor window? Cheapest coherent answer: one
`resolve_workspace()` that validates and canonicalizes, one `write_file()` that every writer in
the tree must go through, and `FileTreeSnapshot` pointed at **every** command rather than at
`sync` alone.

### Q12 — **[ADDENDUM]** Who owns the parser of the durable artifact?

`org2jsonl` is a general-purpose Org⇄JSONL converter with one obr-specific concession
(`org_to_entries_with_keywords`), 30 commits old, unpublished, unversioned, and pinned two
different ways that resolve to two different revisions (R30). obr uses ~52% of it and **1 of its
23 `Element` variants**. Three futures: *pin it* (one line each in `Cargo.toml` and `flake.nix`,
matching what obr already does for `toon_rust` — the recommended minimum); *vendor it* (drops
the 2,231-line writer obr never calls); or *inline it* (a ~300-line `orgize`→`Issue` reader,
which would make the loss surface explicit instead of accidental, at the cost of taking a direct
dependency on an alpha crate). The question underneath: is obr's Org subset going to grow? If it
stays "headings + a property drawer + two JSON src blocks", the parser is over-general and
inlining is correct. If obr wants to read Org that humans actually write, the parser is
under-used and the loss surface is the bug.

---

## 8. Index of notes files

All under
`/private/tmp/claude-501/-Users-johnw-src-obr/f80d1967-0fc0-44fa-a53f-3054d39727e2/scratchpad/research/`.

| File | Lines | Contents |
|---|---:|---|
| `map-storage.md` | 738 | Schema, 11 tables, migrations, 4-step protocol, dirty tracking, blocked cache, ID generation, query construction |
| `map-sync.md` | 730 | Export/import/merge protocols, Org bridge, content hashing, collision detection, git-safety layers, history backups, auto-hooks |
| `map-model.md` | 677 | `Issue` and enums, serde wire format, three error layers, `ErrorCode` taxonomy, all five validators, hint generation |
| `map-cli.md` | 575 | 39 commands, main lifecycle, global flags, config's 7 layers, `--no-db`, multi-repo, routing (dead), logging |
| `map-output.md` | 818 | Live `src/output/` vs dead `src/format/`, five output modes, JSON payload types, `br schema`, error envelope, agent artifacts |
| `map-tests.md` | 623 | 97 targets / ~2,330 tests, 7 layers, conformance harness, build config, CI workflows, dependency inventory |
| `map-docs.md` | 638 | 21k lines of prose, intent vs trajectory, authorship, the full drift catalogue, roadmap and open beads |
| `web-beads-upstream.md` | 798 | Upstream's Dolt pivot and SQLite revival, v0.46 pinned semantics, reception, 13 recommendations |
| `web-related-trackers.md` | 1031 | 11 trackers surveyed, architectural convergence, git-appraise/git-bug/Fossil/SIT in detail, what killed the category |
| `web-sqlite-rust.md` | 1005 | PRAGMA persistence, `BEGIN IMMEDIATE`, busy-handler semantics, WAL constraints, 3.51.x corruption window, 19 recommendations |
| `web-sync-merge.md` | 957 | 14 reproduced git merge experiments on JSONL, prior-art convergence on field-level merge, LWW as documented data loss |
| `web-rust-cli-norms.md` | 1234 | clig.dev vs agent-CLI discourse, clap/thiserror/anstream norms, exit-code taxonomies, testing tooling, MSRV/nightly, lints |
| `analysis-integrity.md` | 646 | R1, R2, R6, R12, R13; FK investigation (superseded — see §9.1); crash resilience; doctor blindness; concurrency test gap |
| `analysis-sync-correctness.md` | 782 | R4, R10, R11, R19–R23; empirical git merge behaviour on Org and JSONL; guarantees that hold vs fail; ranked remediation |
| `analysis-design.md` | 894 | §9.1 and §9.2 corrections; R3, R8, R24, R26, R27; error-system critique; port-fidelity analysis; 12 confirmed strengths |
| `analysis-performance.md` | 740 | R7, R5, R16, R17; the dead `idx_issues_ready`; Org memory profile; benchmark blindness; read-path strengths |
| `analysis-assurance.md` | 727 | R14, R15, R9; 11 executed test failures; fuzz-target gaps; coverage/audit non-gates; test-budget-vs-risk inversion |
| **`critique-completeness.md`** | 258 | **[ADDENDUM]** Mechanical coverage diff (which `src/*.rs` files no agent cited) + claim spot-check; the four gaps commissioned below; two non-gaps cleared (SQL injection, `upgrade` signature verification); lower-priority observations not commissioned |
| **`gap-1.md`** = `gap-routing-discovery.md` | 786 | **[ADDENDUM]** R28. Complete `routing.rs` call graph; what `follow_redirects` accepts (13-case table); 14 reproduced redirect experiments incl. writes into `.git`, `..`, symlinks, `max_depth` truncation, `--db` cross-wiring; `routes.jsonl` proven inert; item-by-item live/dead verdict on all 12 public items; `tests/e2e_routing.rs` audited test-by-test (and executed: 14/14 pass) |
| **`gap-2.md`** = `gap-agents-command.md` | 1155 | **[ADDENDUM]** R29. Full read of `agents.rs`; clap wiring and flag-precedence losses; complete write-surface enumeration; 5 adversarial `AGENTS.md` corruption classes with byte-level before/after; non-UTF-8 total-loss case; the three prompt sites and the pipe hang; verbatim `AGENT_BLURB` with drift quantified; zero-coverage proof; §9 incident disclosure |
| **`gap-3.md`** = `gap-org2jsonl-parser.md` | 1145 | **[ADDENDUM]** R30, R31, and R23's quantification. What `org2jsonl` is; the exact obr⇄crate seam; 23 `Element` / 22 `InlineContent` variant tables with obr's treatment; parser-vs-obr loss attribution; 30k-sample fixpoint measurement; determinism proof; 32-file hostile corpus + 100k mutation fuzz; O(n²) blowup; supply-chain/reproducibility verdict |
| **`gap-4.md`** = `gap-recovery-history.md` | 836 | **[ADDENDUM]** R32 and Q9's rewrite. `history restore` read line-by-line; path-traversal reproductions; the auto-flush backup gate proven dead three ways; per-defect recovery classification; quantified granularity/rotation/dedup limits; what does and does not snapshot; config-key no-op proof; minimum hardening list |
| `DOSSIER.md` | — | This document |

*(The four gap files exist under two names each — `gap-N.md` and a descriptive alias — with
identical contents.)*

---

## 9. Contradiction resolutions

Five direct contradictions arose between contributing agents. Four are resolved here with
first-hand evidence; one is left open.

### 9.1 — `foreign_keys`: **ON, not OFF.** The most widely repeated finding in the corpus is wrong.

`map-storage.md`, `web-sqlite-rust.md`, `analysis-integrity.md` and `analysis-assurance.md` all
lead with: `apply_schema` is skipped when `user_version >= 1`, all pragmas live in
`apply_schema`, therefore `foreign_keys` is OFF on every real invocation and all nine
`ON DELETE CASCADE` clauses are inert. `analysis-design.md` disputes this. **The design agent is
right.**

*Evidence 1 — compile options of the shipped binary:*

```
$ strings $(which obr) | grep -E '^(DEFAULT_FOREIGN_KEYS|DEFAULT_SYNCHRONOUS|DEFAULT_CACHE_SIZE|MAX_VARIABLE_NUMBER)'
DEFAULT_CACHE_SIZE=-2000
DEFAULT_FOREIGN_KEYS          ← present as a bare name = defined truthy
DEFAULT_SYNCHRONOUS=2
MAX_VARIABLE_NUMBER=32766
```

rusqlite's bundled SQLite (3.51.1) is compiled with `SQLITE_DEFAULT_FOREIGN_KEYS=1`, so
`conn.pragma_update(None,"foreign_keys","ON")` at `schema.rs:220` is redundant, not load-bearing.

*Evidence 2 — behavioural, on a database where `apply_schema` was provably skipped:*
3 events existed for `fk-1bj`; a plain `obr list` (auto-import) left 0. Cascade fired.

*Source of the error:* the integrity agent read `PRAGMA foreign_keys` from a **copy of the DB via
the system `sqlite3` CLI**, which is a different SQLite build. Confirmed:

```
$ sqlite3 .beads/beads.db 'PRAGMA foreign_keys;'    → 0
$ sqlite3 :memory: 'pragma compile_options;' | grep -i foreign   → (nothing)
```

The observation was real; it just described the wrong process's connection.

**What survives, and is R17:** the *performance* pragmas genuinely do revert
(`DEFAULT_SYNCHRONOUS=2` = FULL vs intended NORMAL; `DEFAULT_CACHE_SIZE=-2000` = 2 MB vs intended
8 MB; `temp_store` → FILE). **And the deeper point is worse than the original claim:** FK
correctness now depends on a compile-time flag of a vendored C library that appears nowhere in
obr's source and is asserted nowhere. Build against system SQLite and R3 stops being a data-loss
bug and starts being an orphaned-rows bug — the import path's behaviour changes with a build flag.

*Downstream corrections:* `map-storage.md`'s "ON DELETE CASCADE does not fire in normal CLI use"
and `web-sqlite-rust.md`'s "9 CASCADEs are silently inert" are both wrong.
`analysis-sync-correctness.md`'s CROSS-CUTTING TRAP — "fixing the pragma gap would *newly* enable
cascade during import" — is also wrong; cascade is already enabled, which is exactly why R3 is
reproducible today.

*The cascade angle is narrower than feared in one respect the integrity agent got right:* there is
**no `DELETE FROM issues` anywhere in `src/`** (delete is a soft tombstone UPDATE), so the only
cascade trigger in practice is `INSERT OR REPLACE` during import. That is precisely R3.

### 9.2 — `obr list --robot`: **a clap usage error, exit 2 — not a silent no-op.**

`map-cli.md` and `web-rust-cli-norms.md` describe it as silently ignored.
**[SYNTH-VERIFIED]** `obr list --robot` → `error: unexpected argument '--robot' found`, exit 2.
`--robot` is declared on only 10 of 39 arg structs, so on the other 29 it is a *parse* failure.
This is materially worse than a no-op, because exit 2 collides with obr's own Database error
family (R24).

### 9.3 — `--db` handling: **command bodies are coherent; `main.rs`'s auto-hooks are not.**

`map-cli.md` reports `--db` "honored inconsistently for discovery" across ~9 command modules.
`analysis-design.md` disputes this and locates the real defect in `run_auto_flush`.
**[SYNTH-VERIFIED]** the design agent is right about severity: the reproduced failure (R8) is
entirely in `main.rs:261/273` + `sync/mod.rs:1929`. The map's observation about `blocked`,
`doctor`, `orphans`, `config` and `where` using `discover_beads_dir(None|".")` is factually
correct but has a much smaller blast radius (wrong *config*, right *DB*).

### 9.4 — Broken pipe exit code: **134, but only when output exceeds the pipe buffer.**

`analysis-design.md` reports exit 134; a naive re-test on a 4-issue workspace showed exit 0.
Resolved: **[SYNTH-VERIFIED]** `obr schema all | head -c 5` → **exit 134**. Small outputs fit the
64 KB pipe buffer and never raise SIGPIPE. The defect is real and conditional on output size.

### 9.5 — Unresolved: the state of upstream conformance in CI.

`analysis-assurance.md` marks "the weekly conformance cron is already red" as SUSPECTED and could
not verify without network access and a Go toolchain. This synthesizer could not verify it
either. The *mechanism* is confirmed (unpinned HEAD clone of a fork; no upper version bound; all
13 divergence notes written against v0.46.0; upstream ≥0.50 defaults to Dolt and ≥0.58 has no
SQLite), but whether the job currently fails, silently skips, or has not run is unknown.

*One further item flagged but not independently reproduced by the synthesizer:* the 32,767 export
ceiling (R7) was bisected by the performance agent and its mechanism is confirmed here
(`MAX_VARIABLE_NUMBER=32766` from the shipped binary; unchunked placeholder construction read at
`sqlite.rs:2825-2843`), but the synthesizer did not rebuild a 32k-issue workspace.

---

## 10. Addendum: the gap-fill round

A completeness critic re-read this dossier against the repository and against the 17 contributing
notes files, using two passes: a **mechanical coverage diff** (for every `.rs` file under `src/`,
count citations of its path and basename across all notes) and a **claim spot-check** (re-derive
the most load-bearing *negative* claims from source). The diff surfaced ten command modules with
**zero** path citations anywhere in the corpus; the spot-check found two dossier claims wrong and
two unexamined. Four gap-fill agents were commissioned. Everything they found is integrated
above; this section records what was caught and why it mattered.

### 10.1 What the critic caught

| # | Gap | Why the dossier was wrong or blind | Where it now lives |
|---|---|---|---|
| 1 | **`config/routing.rs` is on the hot path, and R27 called it dead** | R27 said "8 of 10 public items have zero external references" and listed the module as deletable. `follow_redirects` is called from `config/mod.rs:214/220/233` — i.e. from the terminating branch of *ordinary* workspace discovery — plus `where.rs:37`. A reader acting on R27 would delete the workspace resolver. Worse, the module is the **unguarded upstream input** to the safety layer §4.1 calls the best-designed part of the project. | R28, §2.3, §2.4, §4.1, R27 (corrected), §6, Q11 |
| 2 | **`obr agents` (1,054 lines) had zero citations** | The third-largest command module and the only one that mutates files in the user's source tree. Its existence falsifies §4.11 ("destructive commands never prompt") and narrows §4.1's write-perimeter claim in exactly the way §6 had already had to narrow "never executes git commands". It is also a *runtime-generated* doc-drift vector the dossier's model did not cover. | R29, §4.11 (withdrawn), §4.13, §6, Q11 |
| 3 | **`org2jsonl` — the parser of the durable artifact — was never opened** | The dossier mentioned it twice, both times as "CI cannot build". Nobody asked the prior question: what is it, what does it drop, does it round-trip, and is the Org story sound? R23's loss was attributed entirely to obr's `extract_body_text` without checking the other half of the pipeline. The crate ships an unrun round-trip oracle and an unrun fuzz target. | R30, R31, R23 (quantified), §2.3, §4.15, Q1, Q12 |
| 4 | **A recovery mechanism already existed and Q9 answered the wrong question** | `grep "history restore\|execute_restore"` returned **zero** across all 18 notes files including the dossier. Q9 proposed `doctor --repair` as "the single highest-value new feature" without evaluating the remediation that ships today — which changes conclusions in *both* directions: R5's severity is lower than stated in the common case and higher in the clone case. | R32, R5 (corrected), R25 (corrected), Q9 (rewritten) |

The critic also cleared two things explicitly, so they should not be re-investigated:
**SQL injection** (every dynamically assembled `WHERE`/`ORDER BY` in `sqlite.rs` appends literal
fragments with `?` placeholders — `:812-902`, `:988-1031`, `:1082-1204`; the one `format!`-built
UPDATE at `:622` interpolates from a fixed internal list; `:3448` interpolates only a `?,?,…`
string; `query.rs`'s `format!` calls build config *keys*, not SQL) and **`obr upgrade` signature
verification** (`upgrade.rs:249,268` pass `.verifying_keys(...)`; the wrong-repo target is the
real defect and was already recorded).

### 10.2 What the round changed, in one line each

- **Three "the tool cannot do X" claims were falsified.** obr *can* write outside `.beads/`
  (R29), *can* write inside `.git/` (R28, R29, R32), and *does* prompt on stdin (R29).
- **One "dead code" recommendation was reversed** before anyone acted on it (R27 → R28).
- **One severity was split rather than moved.** R5 is a discoverability defect when the DB is
  intact and an unrecoverable-loss defect when it is not — with the *same error message* and the
  *same recommended command*, which is destructive in the second case.
- **One proposed feature was demoted.** Q9's `doctor --repair` gave way to a one-line path fix,
  unique backup filenames, and three error strings.
- **One strength was added.** Org emission is byte-deterministic (§4.15) — the prerequisite for
  Q1's hybrid answer, and the only part of the Org story that is unambiguously sound.
- **Two "never executed" claims were nuanced.** `tests/e2e_routing.rs` builds and passes 14/14
  under `nix develop`; a single `tests/common/dataset_registry.rs:1226` assertion reddens every
  integration target outside a git checkout. Passing is not the same as asserting: 9 of those 14
  tests are vacuous, and one of their comments states the opposite of the truth.

### 10.3 The corrected write-perimeter sentence

Replacing §4.1's and §6's over-broad phrasing, and superseding the `obr sync --help` banner:

> **`obr sync` never writes outside the resolved beads directory:** its paths are checked against
> an explicit extension allowlist (`src/sync/path.rs:47-59`), rejected if they contain a `.git`
> component even under `--allow-external-jsonl` (`:137-175`, `:211`, `:450-458`), and written
> atomically via temp-file-then-rename with fsync and chmod 0600. **That guarantee is scoped to
> the sync layer, and it is relative to a directory chosen by an unvalidated, git-committable
> `.beads/redirect` file** (`src/config/routing.rs:200-232`). It does not cover the SQLite open
> (`src/config/mod.rs:315`, `:412`), which is unvalidated; `obr agents`, which locates
> `AGENTS.md`/`CLAUDE.md` in `$PWD` plus three ancestors, follows symlinks, and whole-file
> rewrites with no validation and a single-generation backup whose failure is only a warning
> (`agents.rs:366`, `:225-244`, `:535-549`); or `obr history restore` / `history diff`, whose
> `file` argument is joined unsanitized (`history.rs:240`, `:137`). Separately, `obr agents` and
> `obr orphans --fix` read from stdin; the former is bypassable with `--force`, **the latter is
> not bypassable at all.** The narrower claim — *obr never runs git itself* — survives at the
> process level (five read-only exceptions outside sync, §6), but is undercut in spirit by the
> blurb `obr agents` writes into third-party repositories, which instructs the *agent* to run
> `git add` / `git commit` / `git push` at the end of every session.

### 10.4 Still open after this round

- **Not commissioned, from the critic's lower-priority list:** the epic/hierarchy feature end to
  end (`epic.rs`, the `.N.M` child-id grammar, missing parent on import, `--rename-prefix` over
  hierarchical ids, cascade close, 51-deep trees against `MAX_DEPTH = 50`);
  `src/util/markdown_import.rs` and `obr q` quick-capture as bulk-ingestion paths for the agent
  audience; `src/cli/commands/graph.rs` beyond the R13 fixture trap;
  `tests/e2e_workspace_commands.rs` and the `where`/`info` "which workspace am I in" surface
  (Gap 1's user-facing half); and `audit.rs`, `blocked.rs`, `close.rs`, `defer.rs`, `reopen.rs`,
  `stale.rs`, `version.rs` — still zero path citations.
- **Unresolved from §9.5:** the state of the weekly upstream-conformance cron.
- **Not reproduced:** the 32,767 export ceiling (mechanism confirmed, workspace not rebuilt);
  interleaved-write corruption on the *auto-flush* path specifically (gap-4 could not reproduce
  it on that machine — SQLite lock contention appears to serialize the flushes — while
  reproducing the identical race loudly on the explicit-sync path at 21/30 failures; R5's
  original 12-concurrent-`create` reproduction on a 10k workspace stands).
- **Method note for a future round:** the mechanical coverage diff (count citations of each
  source file across all notes) found every one of these gaps and cost minutes. It should run
  *before* synthesis, not after.
