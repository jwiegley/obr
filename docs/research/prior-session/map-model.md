# Subsystem map: `model` + `error` + `validation`

Repo: `/Users/johnw/src/obr` (crate `beads_rust`, binary `br`/`obr`). All paths below are
absolute-relative to that root; citations are `path:line`.

Files in scope:

| File | Lines | Role |
|---|---|---|
| `src/model/mod.rs` | 1494 (654 are tests) | Domain types + enums + serde wire format |
| `src/error/mod.rs` | 332 | Layer 1: `BeadsError` (thiserror) |
| `src/error/structured.rs` | 1066 | Layers 2+3: `StructuredError`, `ErrorCode`, intent detection |
| `src/error/context.rs` | 114 | `ResultExt` / `OptionExt` context adapters (**unused**) |
| `src/validation/mod.rs` | 901 (481 are tests) | Field/invariant validators + sync-safety guards |

---

## 1. The domain model (`src/model/mod.rs`)

### 1.1 Entities

Five serializable entities + one derived view type:

| Type | Decl | Notes |
|---|---|---|
| `Issue` | `src/model/mod.rs:369-503` | 38 fields; the JSONL wire record |
| `EpicStatus` | `src/model/mod.rs:591-596` | Derived view: `{epic, total_children, closed_children, eligible_for_close}` |
| `Dependency` | `src/model/mod.rs:600-625` | Edge `issue_id -> depends_on_id` |
| `Comment` | `src/model/mod.rs:629-636` | `id: i64` (SQLite rowid), `body` serialized as `"text"` |
| `Event` | `src/model/mod.rs:640-652` | Audit log row; **never exported to JSONL** |

`Issue` derives `Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema`
(`src/model/mod.rs:368`). Every entity is `JsonSchema` because `br schema` emits JSON Schema
for agents (`src/cli/commands/schema.rs:96-131`).

### 1.2 `Issue` field groups and serde behavior

The `Issue` struct has **no `rename_all`** — JSON keys are the Rust field names verbatim
(`issue_type`, `acceptance_criteria`, `estimated_minutes`, …). Groups:

- **Identity**: `id` (`:371`), `content_hash: Option<String>` marked `#[serde(skip)]` (`:374-375`).
  The content hash is *never* written to or read from JSONL; import recomputes it
  (`src/sync/mod.rs:2151`).
- **Content**: `title` (required, always serialized), `description`, `design`,
  `acceptance_criteria`, `notes` — all `Option<String>` with
  `#[serde(default, skip_serializing_if = "Option::is_none")]` (`:378-394`).
- **Classification**: `status: Status`, `priority: Priority`, `issue_type: IssueType`, each
  `#[serde(default)]` (`:397-406`). Always emitted (no skip).
- **People**: `assignee`, `owner`, `created_by`, `sender` (`:409-414`, `:425`, `:486`).
- **Time**: `created_at`, `updated_at` (required, non-Option, chrono RFC3339)
  (`:421`, `:428`); `closed_at`, `due_at`, `defer_until`, `deleted_at`, `compacted_at`
  optional.
- **Closure**: `close_reason`, `closed_by_session` (`:436`, `:440`).
- **Provenance**: `external_ref`, `source_system`, `source_repo` (`:452-460`).
- **Tombstone**: `deleted_at`, `deleted_by`, `delete_reason`, `original_type` (`:463-470`).
- **Compaction (legacy)**: `compaction_level`, `compacted_at`, `compacted_at_commit`,
  `original_size` (`:475-482`).
- **Flags**: `ephemeral`, `pinned`, `is_template` — `bool` with
  `skip_serializing_if = "is_false"` (`is_false` at `src/model/mod.rs:17-20`), so `false` is
  omitted from JSONL (`:487-494`).
- **Relations**: `labels: Vec<String>`, `dependencies: Vec<Dependency>`,
  `comments: Vec<Comment>` with `skip_serializing_if = "Vec::is_empty"` (`:497-502`). These
  live in separate SQLite tables and are hydrated only for export/display
  (`src/storage/sqlite.rs:3134-3136` sets them empty on row read; `src/sync/mod.rs:1381-1400`
  batch-populates them before export).

**Special-cased serializer**: `compaction_level: Option<i32>` uses
`serialize_with = "serialize_compaction_level"` (`src/model/mod.rs:475`, fn at `:22-29`) which
emits `0` when `None`. Comment at `:473-474` explains why: *bd's Go sql scanner cannot handle
NULL for integer columns*. Consequence: `compaction_level` is the one optional numeric that is
**always present** in every JSONL line.

`Default for Issue` is hand-written (`:505-549`) and stamps `Utc::now()` into both
`created_at` and `updated_at` (two separate calls at `:521` and `:523`, so they can differ by
nanoseconds).

### 1.3 `Status` (`src/model/mod.rs:32-98`)

```
Open | InProgress | Blocked | Deferred | Closed | Tombstone | Pinned | Custom(String)
```

- `#[serde(rename_all = "snake_case")]` (`:33`) plus redundant explicit
  `#[serde(rename = "tombstone")]` / `#[serde(rename = "pinned")]` (`:41-44`) — these are
  no-ops given snake_case.
- `Custom(String)` is `#[serde(untagged)]` (`:45`), making unknown status strings round-trip
  losslessly through JSONL.
- `as_str()` (`:51-62`), `is_terminal()` = `Closed | Tombstone` (`:65-67`),
  `is_active()` = `Open | InProgress` (`:70-72`). Note `Blocked` and `Deferred` are neither
  active nor terminal.
- `FromStr` (`:81-98`) lowercases, accepts `in_progress` **and** `inprogress` (not
  `in-progress`), and **errors** on anything unknown with `BeadsError::InvalidStatus`. This is
  the only site in the whole crate that constructs `InvalidStatus`.
- Asymmetry: the storage reader `parse_status` (`src/storage/sqlite.rs:3282-3287`) falls back
  to `Status::Custom(val)` on parse failure, and serde deserialization also yields `Custom`.
  So custom statuses can enter via JSONL/DB but **cannot be set from the CLI**.

### 1.4 `Priority` (`src/model/mod.rs:101-134`)

Newtype `pub struct Priority(pub i32)` with `#[serde(transparent)]` (`:104`), so JSON is a bare
integer. Constants `CRITICAL=0, HIGH=1, MEDIUM=2, LOW=3, BACKLOG=4` (`:108-112`).
`Default` = `Priority(0)` (derived, i.e. **critical**) — but the CLI default is
`Priority::MEDIUM` via config (`src/config/mod.rs:870`), and the DB column defaults to `2`
(`src/storage/schema.rs:22`). Display is `P{n}` (`:116-118`).
`Ord` is derived so lower number sorts first (`P0 < P1`), matching "most urgent first".

`FromStr` (`:121-134`): trims, uppercases, strips a leading `P`, parses i32, range-checks 0..=4.
**Lossy failure**: any non-numeric input produces `InvalidPriority { priority: -1 }` (`:131`),
discarding the original string. See §4.4 for the downstream consequence.

Because serde is `transparent`, **deserialization does no range check** — a JSONL line with
`"priority": 99` parses fine and is caught later by `IssueValidator` (§5.1) or the SQLite
`CHECK(priority >= 0 AND priority <= 4)` (`src/storage/schema.rs:22`).

### 1.5 `IssueType` (`src/model/mod.rs:137-196`)

```
Task(default) | Bug | Feature | Epic | Chore | Docs | Question | Custom(String)
```
snake_case serde with untagged `Custom` (`:148`). `is_standard()` returns
`!matches!(self, Custom(_))` (`:170-172`).

**`FromStr` never fails** — unknown input becomes `Custom(other)` (`:193`). So
`br create --type=nonsense` succeeds and stores a custom type; this is asserted by
`tests/e2e_errors.rs:861-864` ("custom type should be preserved"). The only place that rejects
non-standard types is the `br lint --type` filter (`src/cli/commands/lint.rs:239-244`), which is
the sole producer of `BeadsError::InvalidType` in the codebase.

### 1.6 `DependencyType` (`src/model/mod.rs:199-278`)

kebab-case serde (`:200`), untagged `Custom` (`:213`). Eleven standard variants:
`blocks, parent-child, conditional-blocks, waits-for, related, discovered-from, replies-to,
relates-to, duplicates, supersedes, caused-by`.

Two predicates, `affects_ready_work()` (`:237-242`) and `is_blocking()` (`:245-250`), have
**byte-identical bodies** — both are `Blocks | ParentChild | ConditionalBlocks | WaitsFor`.
`is_blocking()` gates cycle detection (`src/storage/sqlite.rs:1791`,
`src/validation/mod.rs:156`); `affects_ready_work()` is the "ready work" query predicate.
`FromStr` never fails (`:275`).

SQL mirrors the blocking set in a partial index
(`src/storage/schema.rs:107-109`: `WHERE type IN ('blocks','parent-child','conditional-blocks','waits-for')`).

### 1.7 `EventType` (`src/model/mod.rs:281-365`)

15 variants + `Custom(String)`: `created, updated, status_changed, priority_changed,
assignee_changed, commented, closed, reopened, dependency_added, dependency_removed,
label_added, label_removed, compacted, deleted, restored`.

Unlike the other enums, `EventType` has **hand-written** `Serialize` (`:325-329`),
`Deserialize` (`:331-354`), and `JsonSchema` (`:356-365`, which just delegates to
`String`'s schema). It serializes as a plain string; unknown strings deserialize to `Custom`.

### 1.8 Status transitions — there is no state machine

There is **no** `can_transition`/`valid_transition` function anywhere in `src/`
(grep confirms only unrelated hits: `src/util/time.rs:19`, `src/cli/commands/update.rs:71-79`).
Transition rules are enforced ad hoc at three layers:

1. **CLI guard**: `br update --claim` / `--status in_progress` refuses if the issue is blocked,
   unless `--force` (`src/cli/commands/update.rs:71-79`).
2. **Derived-field coupling**: setting status Closed/Tombstone stamps `closed_at`
   (`src/cli/commands/create.rs:202-206`, `:423-427`; `src/cli/commands/update.rs:242`).
3. **DB CHECK constraint** (`src/storage/schema.rs:50-55`): closed ⇒ `closed_at IS NOT NULL`;
   tombstone ⇒ unconstrained; anything else ⇒ `closed_at IS NULL`.
4. **Import repair**: `normalize_issue` (`src/sync/mod.rs:2141-2174`) back-fills
   `closed_at = updated_at` for closed/tombstone issues and *clears* `closed_at` otherwise,
   so imported data always satisfies the CHECK.

Any status→any status is otherwise legal.

### 1.9 Content hashing / dedup

`Issue::compute_content_hash()` (`src/model/mod.rs:560-562`) delegates to
`crate::util::content_hash` (`src/util/hash.rs:40-58`). SHA-256 over NUL-separated fields in a
fixed order (`src/util/hash.rs:81-95`):

> title, description, design, acceptance_criteria, notes, status.as_str(),
> `format!("P{}", priority.0)`, issue_type.as_str(), assignee, owner, created_by,
> external_ref, source_system, pinned, is_template

**Excluded**: id, content_hash, all timestamps, labels/dependencies/comments/events,
tombstone fields, `estimated_minutes`, `due_at`, `defer_until`, `close_reason`,
`closed_by_session`, `source_repo`, `sender`, compaction fields
(`src/util/hash.rs:29-38`). Embedded NULs in values are replaced with spaces before hashing
(`src/util/hash.rs:112-114`).

Tests pin the invariants: hash is stable across differing timestamps
(`src/model/mod.rs:1265-1275`) and differing IDs (`:1278-1287`), and changes on title,
description, status, priority (`:1205-1262`).

### 1.10 Tombstone TTL

`Issue::is_expired_tombstone(retention_days: Option<u64>)` (`src/model/mod.rs:566-586`).
Returns `false` (i.e. keep) unless: status is `Tombstone`, `retention_days` is `Some(d)`,
`d != 0`, `deleted_at` is `Some`, and `now > deleted_at + d days`. The `days == 0` arm returns
false with the comment "Keep forever if 0 (though usually means disabled/immediate, assume safe
default)" (`:575-577`). Used by export to drop expired tombstones
(`src/sync/mod.rs:1453-1458`).

### 1.11 Serialized forms — the three representations

**(a) JSONL wire format** is literally `serde_json::to_string(&issue)` of the `Issue` struct,
one per line (`src/sync/mod.rs:1489-1500`), and import is
`serde_json::from_str::<Issue>(&line)` (`src/sync/mod.rs:2245`). There is *no* separate DTO —
the `#[serde]` attributes on `Issue` **are** the git-tracked file format, so any field rename
is a breaking wire change. (An alternate Org-mode format exists for `.org` outputs,
`src/sync/mod.rs:1477-1487`, via `src/sync/org_bridge.rs`.)

Field renames that matter on the wire:
- `Dependency::dep_type` → `"type"` (`src/model/mod.rs:608`)
- `Comment::body` → `"text"` (`src/model/mod.rs:633`)

**(b) SQLite rows** (`src/storage/schema.rs:13-56` for `issues`, `:89-109` dependencies,
`:112-118` labels, `:122-131` comments, `:134-148` events). Notable impedance mismatch: text
columns are `NOT NULL DEFAULT ''` rather than nullable, so the reader maps `""` back to `None`
via `SqliteStorage::empty_to_none` (`src/storage/sqlite.rs:3065-3071`) at
`src/storage/sqlite.rs:3079-3131`. `source_repo` defaults to `'.'` in SQL
(`src/storage/schema.rs:37`) but is `Option<String>` in the model, so `.` survives as
`Some(".")` while `''` becomes `None`.

**(c) JSON Schema** emitted by `br schema` (`src/cli/commands/schema.rs:88-133`), which
publishes `Issue` plus the `format::*` view types and an `ErrorEnvelope`
(`src/cli/commands/schema.rs:21-38`) describing the error JSON contract.

---

## 2. Error layer 1 — `BeadsError` (`src/error/mod.rs`)

A `thiserror` enum with **30 variants** (`src/error/mod.rs:27-164`), grouped by comment banner:

| Group | Variants |
|---|---|
| Storage | `DatabaseNotFound{path}`, `DatabaseLocked{path}`, `SchemaMismatch{expected,found}`, `Database(#[from] rusqlite::Error)` |
| Issue | `IssueNotFound{id}`, `IdCollision{id}`, `AmbiguousId{partial,matches}`, `InvalidId{id}` |
| Validation | `Validation{field,reason}`, `ValidationErrors{errors}`, `InvalidStatus{status}`, `InvalidType{issue_type}`, `InvalidPriority{priority: i32}` |
| JSONL | `JsonlParse{line,reason}`, `PrefixMismatch{expected,found}`, `ImportCollision{count}` |
| Dependency | `DependencyCycle{path}`, `HasDependents{id,count}`, `SelfDependency{id}`, `DependencyNotFound{id}`, `DuplicateDependency{from,to}` |
| Config | `Config(String)`, `NotInitialized`, `AlreadyInitialized{path}` |
| I/O | `Io(#[from] io::Error)`, `Json(#[from] serde_json::Error)`, `Yaml(#[from] serde_yaml::Error)` |
| Wrapping | `WithContext{context, #[source] source}`, `Other(#[from] anyhow::Error)` |
| Operational | `NothingToDo{reason}` |

What this layer adds: a **typed, structured payload** (no stringly-typed errors), `Display`
messages via `#[error(...)]`, and `From` conversions for the three foreign error types so `?`
works across rusqlite/io/serde boundaries. `pub type Result<T> = std::result::Result<T, BeadsError>`
(`:283`).

`ValidationError { field, message }` (`:167-192`) is a separate lightweight struct (not a
`BeadsError` variant) used to accumulate multiple field errors before collapsing them:
`BeadsError::from_validation_errors` (`:269-279`) yields `Validation` for exactly one error and
`ValidationErrors` for 2+.

Helper methods on `BeadsError`:
- `suggestion() -> Option<&'static str>` (`:225-244`) — static remediation strings for 11
  variants. **This is the hint source in practice** (see §4.3).
- `is_user_recoverable()` (`:197-210`) — **never called** anywhere in `src/` or `tests/`.
- `suggests_force()` (`:214-221`) — **never called**.
- `exit_code()` (`:251-256`) — returns 3 for `NothingToDo`, else 1. **Never called**; the real
  exit code comes from `ErrorCode::exit_code()` (§3). These two functions disagree and the dead
  one is the more bd-legacy-looking of the pair.

### 2.1 `src/error/context.rs` — entirely unused

`ResultExt::context` / `with_context` (`:12-52`) wrap any `E: Error + Send + Sync` into
`BeadsError::WithContext`; `OptionExt::ok_or_context` (`:55-68`) turns `None` into
`BeadsError::Other(anyhow!(msg))`. Both are re-exported at `src/error/mod.rs:17` and reachable
as `beads_rust::error::{ResultExt, OptionExt}`, but **grep finds zero call sites** outside the
module's own tests. The `WithContext` variant is therefore effectively unreachable in
production (nothing else constructs it), which also makes the `InternalError` mapping at
`src/error/structured.rs:607-609` dead.

---

## 3. Error layer 3 — `ErrorCode` taxonomy (`src/error/structured.rs:31-240`)

33 codes, `SCREAMING_SNAKE_CASE` on the wire via `as_str()` (`:121-166`). Two orthogonal
attributes hang off the code: `is_retryable()` (`:174-185`) and `exit_code()` (`:199-239`).

### 3.1 Full taxonomy

| Exit | Category | Codes |
|---:|---|---|
| 2 | Database | `DATABASE_NOT_FOUND`, `DATABASE_LOCKED`, `SCHEMA_MISMATCH`, `DATABASE_ERROR`, `NOT_INITIALIZED`, `ALREADY_INITIALIZED` |
| 3 | Issue + operational | `ISSUE_NOT_FOUND`, `AMBIGUOUS_ID`, `ID_COLLISION`, `INVALID_ID`, `NOTHING_TO_DO` |
| 4 | Validation | `VALIDATION_FAILED`, `INVALID_STATUS`, `INVALID_TYPE`, `INVALID_PRIORITY`, `REQUIRED_FIELD` |
| 5 | Dependency | `CYCLE_DETECTED`, `DEPENDENCY_NOT_FOUND`, `HAS_DEPENDENTS`, `SELF_DEPENDENCY`, `DUPLICATE_DEPENDENCY` |
| 6 | Sync/JSONL | `JSONL_PARSE_ERROR`, `PREFIX_MISMATCH`, `IMPORT_COLLISION`, `CONFLICT_MARKERS`, `PATH_TRAVERSAL` |
| 7 | Config | `CONFIG_ERROR`, `CONFIG_NOT_FOUND`, `CONFIG_PARSE_ERROR` |
| 8 | I/O | `IO_ERROR`, `JSON_ERROR`, `YAML_ERROR` |
| 1 | Internal | `INTERNAL_ERROR` |

Note that `NOT_INITIALIZED` and `ALREADY_INITIALIZED` are filed under *database* (exit 2), not
config. `NOTHING_TO_DO` is filed under *issue* (exit 3) — the one place where
`BeadsError::exit_code()`'s dead value agrees with reality.

`is_retryable()` = `{DatabaseLocked, ValidationFailed, InvalidStatus, InvalidType,
InvalidPriority, RequiredField, AmbiguousId}` (`:176-184`). Semantics per the doc comment
(`:168-173`): "retryable" conflates *wait and retry* (locked DB) with *fix your input and
retry* (validation). Notably `ISSUE_NOT_FOUND` and `CYCLE_DETECTED` are **not** retryable.

### 3.2 Five codes are unreachable

`extract_code_and_context` (`src/error/structured.rs:492-612`) is an exhaustive match over all
30 `BeadsError` variants, but it never yields `RequiredField`, `ConflictMarkers`,
`PathTraversal`, `ConfigNotFound`, or `ConfigParseError`. Since no other code path constructs a
`StructuredError` in production (§4.1), those five codes — and with them the entire exit-code-6
"sync" band except `JSONL_PARSE_ERROR`/`PREFIX_MISMATCH`/`IMPORT_COLLISION` — can never be
emitted. This is acknowledged in the e2e test at `tests/e2e_errors.rs:787-791`, which accepts
"exit code 6 (sync errors) **or 7 (config)**" because sync failures are reported as
`BeadsError::Config(String)` (e.g. `src/sync/mod.rs:2246`, `:2263-2273`, `:1298-1304`).

---

## 4. Error layer 2 — `StructuredError` and how errors reach the user

### 4.1 The struct

```rust
pub struct StructuredError {
    pub code: ErrorCode,
    pub message: String,
    pub hint: Option<String>,      // skip_serializing_if none
    pub retryable: bool,
    pub context: Option<Value>,    // skip_serializing_if none
}
```
(`src/error/structured.rs:250-264`)

What this layer adds over `BeadsError`: (a) a stable machine code, (b) a *hint* aimed at
self-correcting AI agents, (c) a retryability flag, (d) a free-form JSON `context` blob with
the error's structured fields re-exposed for programmatic consumption.

`StructuredError::from_error(&BeadsError)` (`:269-280`) is the **only** constructor used in
production (`src/main.rs:305`, plus a unit test at `src/lib.rs:65`). The seven convenience
constructors — `issue_not_found` (`:284-307`), `ambiguous_id` (`:311-334`), `cycle_detected`
(`:338-353`), `not_initialized` (`:357-365`), `invalid_priority` (`:369-400`), `invalid_status`
(`:404-423`), `invalid_type` (`:427-446`) — **are never called outside `#[cfg(test)]`**. The
richer behavior they encode (Levenshtein "did you mean" suggestions, valid-value lists in
context) is therefore never seen by users. Likewise `find_similar_ids` (`:898-917`) is `pub`
but lives in a private `mod structured` that only re-exports `ErrorCode` and `StructuredError`
(`src/error/mod.rs:15,18`), so it is unreachable from outside the module.

### 4.2 The delivery path

Single funnel: `main::handle_error(&BeadsError, json_mode) -> !` (`src/main.rs:304-323`),
called from exactly two sites — after auto-import (`src/main.rs:34`) and after the command
dispatch match (`src/main.rs:129`).

```rust
let structured = StructuredError::from_error(err);
let exit_code = structured.code.exit_code();
let use_json = json_mode || !io::stdout().is_terminal();   // src/main.rs:307
if use_json { eprintln!("{}", to_string_pretty(&structured.to_json())) }
else        { eprintln!("{}", structured.to_human(io::stderr().is_terminal())) }
std::process::exit(exit_code);
```

Per output mode:

| Mode | Trigger | Error rendering |
|---|---|---|
| **JSON** | `--json` / `--robot` | pretty `{"error":{code,message,hint,retryable,context}}` on **stderr** (`to_json` at `:450-460`) |
| **Piped (non-TTY stdout)** | any command with stdout redirected | **also JSON** — `!io::stdout().is_terminal()` at `src/main.rs:307` |
| **Rich (TTY)** | interactive | `to_human(true)`: `\x1b[31mError:\x1b[0m msg` + `\x1b[33mHint:\x1b[0m …` (`:464-488`) |
| **Plain / `--no-color`** | `NO_COLOR`, `--no-color` **with a TTY stdout** | `to_human(false)`: `Error: msg\nHint: …` |
| **Quiet** | `--quiet` | unchanged — errors still print in full; `--quiet` only suppresses success/info |

Two consequences worth flagging:

1. The **error** output-mode decision (`src/main.rs:307`) is *not* the same as the normal
   output-mode decision (`OutputContext::detect_mode`, `src/output/context.rs:107-119`).
   `detect_mode` honours `--quiet` and `NO_COLOR` and yields `Plain` for a pipe;
   `handle_error` ignores both and yields **JSON** for a pipe. So `br show bogus | cat` writes
   a JSON error envelope, while `br show bogus` in a terminal writes a colored line.
   `OutputMode::Toon` has no error path at all.
2. `OutputContext::error` (`src/output/context.rs:306-316`) and `error_panel` (`:356-378`) are
   **never called** from any command — grep for `.error(` / `.error_panel(` outside
   `src/output/` returns nothing. They are dead. Non-fatal/partial failures instead use raw
   `eprintln!` (49 occurrences under `src/cli/`), e.g. per-issue failures during bulk create
   (`src/cli/commands/create.rs:441`, `:456`, `:468`, `:519`, `:536-540`), which bypass the
   structured envelope entirely — including in `--json` mode.

### 4.3 Hint generation — and why intent detection never surfaces

`generate_hint(err, context)` (`src/error/structured.rs:615-665`) **short-circuits on
`err.suggestion()` first** (`:617-619`). `BeadsError::suggestion()` (`src/error/mod.rs:225-244`)
already returns `Some` for `NotInitialized, DatabaseNotFound, AmbiguousId, HasDependents,
ImportCollision, DependencyCycle, SelfDependency, AlreadyInitialized, InvalidPriority,
InvalidStatus, InvalidType`.

Therefore the `match` arms in `generate_hint` for `InvalidPriority` (`:626-632`),
`InvalidStatus` (`:633-639`), `InvalidType` (`:640-646`) and `HasDependents` (`:647-656`) are
**dead** — the static string always wins. Only the `IssueNotFound` (`:623-625`), `NothingToDo`
(`:657-659`) and `JsonlParse` (`:660-662`) arms are reachable.

Net effect: `--status done` produces the generic
`"Valid statuses: open, in_progress, blocked, deferred, closed"`, never
`"Did you mean --status closed?"`. The e2e test at `tests/e2e_errors.rs:626-634` asserts only
that the hint *contains* the substring `"closed"` — which the generic string does — so it passes
without exercising the synonym logic at all. Same false-positive shape at
`tests/e2e_errors.rs:890-894` for priority ("hint should mention valid priority range").

The smarter hint is not entirely lost: `extract_code_and_context` computes
`detect_*_intent(...)` and stuffs it into the **context** blob as a nested `"hint"` key for
`InvalidStatus` (`:533-544`), `InvalidType` (`:545-556`) and `InvalidPriority` (`:557-570`). So
JSON consumers see both `error.hint` (generic) and `error.context.hint` (smart, or `null`).

### 4.4 Intent detection machinery (mostly unexercised)

Precomputed `LazyLock` sets/maps (`src/error/structured.rs:668-766`):

- `VALID_STATUSES` = open, in_progress, blocked, deferred, closed, tombstone (`:671-682`) —
  **omits `pinned`**, which `Status::from_str` accepts.
- `VALID_TYPES` = task, bug, feature, epic, chore (`:685-689`) — **omits `docs` and
  `question`**, which `IssueType::from_str` accepts and which the type-synonym table maps *to*
  (`"documentation"→"docs"`, `"ask"→"question"`, `:730-739`). So the suggester can propose a
  type its own "valid values" list rejects.
- `STATUS_SYNONYMS` (17 entries, `:692-715`): done/complete/completed/finished/resolved/wontfix
  →closed, wip/working/active/started→in_progress, new/todo/pending→open, waiting→blocked,
  hold/later/postponed→deferred.
- `TYPE_SYNONYMS` (18 entries, `:718-743`), `PRIORITY_SYNONYMS` (13 entries, `:746-766`:
  critical/crit/urgent/highest→0, high/important→1, medium/normal/default→2, low/minor→3,
  backlog/lowest/trivial→4).

`detect_status_intent` / `detect_type_intent` (`:771-816`) do exact → synonym → **prefix**
match, iterating a `HashSet` for the prefix pass (`:785`, `:809`) — non-deterministic order if
two valid values ever shared a prefix (today all first letters are distinct, so it happens to
be stable). `detect_priority_intent` (`:819-851`) handles bare digits, `P0`–`P4`, then synonyms.

**The priority path is doubly broken**: `Priority::from_str` collapses `"high"` to
`InvalidPriority { priority: -1 }` (`src/model/mod.rs:131`), so
`detect_priority_intent(&priority.to_string())` at `src/error/structured.rs:558` and `:627`
runs against the string `"-1"`, not `"high"`. Even if `suggestion()` did not short-circuit,
the suggester could not recover the user's word. The message the user sees for
`br create --priority high` is `Priority must be 0-4, got: -1`.

`levenshtein_distance` (`:858-893`) is a textbook full-matrix implementation, char-based;
`find_similar_ids` (`:898-917`) filters distance ≤ 3, sorts by (distance, id), takes N. Both
only ever run from `StructuredError::issue_not_found`, which is never called (§4.1).

---

## 5. Validation (`src/validation/mod.rs`)

Five independent validators. None of them mutate storage; the module doc calls this out
(`:1-13`).

### 5.1 `IssueValidator::validate(&Issue) -> Result<(), Vec<ValidationError>>` (`:32-101`)

Accumulates *all* failures rather than short-circuiting. Rules:

| Field | Rule | Line |
|---|---|---|
| `id` | non-empty after trim | `:36-38` |
| `id` | `len() <= 105` (`MAX_ID_PREFIX_LEN 64` + 1 + `MAX_ID_HASH_LEN 40`, `:19-21`) | `:39-44` |
| `id` | matches `is_valid_id_format` when non-empty | `:45-50` |
| `title` | non-empty after trim | `:53-55` |
| `title` | `len() <= 500` (**bytes**, not chars) | `:56-58` |
| `description` | `len() <= 102_400` (100 KiB, bytes) | `:61-65` |
| `priority` | `0 <= p <= 4` | `:68-70` |
| `updated_at` | `>= created_at` | `:73-78` |
| `external_ref` | `len() <= 200` and contains no whitespace | `:81-94` |

Call sites (only three in production):
- `br create` single-issue path, `src/cli/commands/create.rs:257` — hard error.
- `br create` bulk/file path, `src/cli/commands/create.rs:519-524` — logs and **skips** that
  issue, continues the batch.
- JSONL/Org import step 3.5, `src/sync/mod.rs:2262-2274` — first failure aborts the whole
  import with `BeadsError::Config(...)` (hence exit 7, not 4).

`br update` does **not** run `IssueValidator` — updates are validated only by the SQLite CHECK
constraints and by `Status::from_str`/`Priority::from_str` at the argument-parsing boundary.

### 5.2 Deliberately *not* validated

This is the interesting half. `IssueValidator` says nothing about:

- **`status`** — any `Status` including `Custom(_)` passes; test
  `tests/proptest_validation.rs:320-350` asserts all seven standard statuses are valid, and
  nothing rejects custom ones.
- **`issue_type`** — same; `Custom(_)` is legal at the model layer (`tests/proptest_validation.rs:352-380`).
- **`design`, `acceptance_criteria`, `notes`** — no length cap at all, unlike `description`.
- **`assignee`, `owner`, `created_by`, `sender`, `close_reason`, `delete_reason`,
  `source_system`, `source_repo`** — no length or charset rules.
- **`estimated_minutes`, `original_size`, `compaction_level`** — no sign or range checks.
- **Date coherence** — `closed_at`, `due_at`, `defer_until`, `deleted_at` are never compared to
  `created_at` or to each other; only `updated_at >= created_at` is enforced. The
  status↔`closed_at` coupling lives in SQL (`src/storage/schema.rs:50-55`) and in
  `normalize_issue` (`src/sync/mod.rs:2158-2173`), not here.
- **Embedded relations** — `issue.labels`, `issue.dependencies`, `issue.comments` are ignored
  by `IssueValidator`; labels are validated only at the CLI boundary (§5.4), comments only by
  the unused `CommentValidator` (§5.5), dependencies only by storage (§5.3).
- **Uniqueness / referential integrity** — entirely SQLite's job.

Byte-vs-char is a systematic choice (`String::len()` everywhere): a 500-emoji title is rejected
at ~125 characters. Same for the 50-byte label cap and 200-byte author/external_ref caps.

### 5.3 `DependencyValidator` + `DependencyStore` (`:105-178`)

`DependencyStore` is a 3-method trait (`issue_exists`, `dependency_exists`,
`would_create_cycle`, `:105-124`) implemented for `SqliteStorage`
(`src/storage/sqlite.rs:3829-3849`). `DependencyValidator::validate` (`:135-177`) accumulates:
self-dependency (`:138-143`), missing source issue (`:145-147`), missing target (`:149-154`),
cycle **only when `dep_type.is_blocking()`** (`:156-163`), duplicate edge (`:165-170`), then
collapses via `BeadsError::from_validation_errors`.

**`DependencyValidator::validate` is never called from production code** — grep finds only its
definition and its five unit tests (`:582-666`). The real path is
`SqliteStorage::add_dependency` (`src/storage/sqlite.rs:1782-1830`), which does its own
blocking-only cycle check raising `BeadsError::DependencyCycle` (`:1791-1797`) and silently
returns `Ok(false)` for a duplicate edge (`:1806-1808`) instead of erroring. So the trait impl
exists solely to satisfy a validator nobody calls; the divergence means duplicate-dependency is
a no-op in practice and `DUPLICATE_DEPENDENCY` / `DEPENDENCY_NOT_FOUND` codes are effectively
unreachable through that route (e2e observes `ISSUE_NOT_FOUND` instead —
`tests/e2e_errors.rs:1118-1136`).

### 5.4 `LabelValidator::validate(&str)` (`:183-209`)

Single-error (not accumulating). Non-empty (`:190-192`), `len() <= 50` (`:194-196`), charset
`[A-Za-z0-9] | '-' | '_' | ':'` (`:198-206`) — the colon enables namespaced labels like
`team:backend` (`:519-521`).

Call sites: `br create` (`src/cli/commands/create.rs:281-282` hard error;
`:535-541` warn-and-skip in the bulk path), `br update` (`src/cli/commands/update.rs:99`,
`:114` for add/remove), `br q` (`src/cli/commands/q.rs:117`).

### 5.5 `CommentValidator::validate(&Comment)` (`:215-253`)

Accumulating: `id > 0` (`:224-226`), non-empty `issue_id` (`:228-230`), non-empty trimmed body
reported under field name **`"content"`** not `"body"` (`:232-234`), body `<= 51_200` bytes
(`:236-238`), non-empty author (`:240-242`), author `<= 200` bytes (`:244-246`).

**Never called in production** — only its own unit test (`:524-535`). The `id > 0` rule makes it
unusable pre-insert anyway, since `Comment::id` is a SQLite AUTOINCREMENT rowid assigned by
`insert_comment_row` (`src/storage/sqlite.rs:3852-3863`).

### 5.6 `is_valid_id_format` — two divergent implementations

`src/validation/mod.rs:257-300` splits via `crate::util::id::split_prefix_remainder` and then
requires: prefix non-empty and ≤ 64 (`:263-265`); prefix charset lowercase/digit/`_`/`-`/`.`
(`:267-272`); remainder non-empty and ≤ 40 (`:275-277`); base hash (before first `.`) non-empty
and lowercase-alnum only (`:281-291`); every dot segment non-empty and all-digits (`:293-297`).

`src/util/id.rs:485-487` is a *different* function of the same name — `parse_id(id).is_ok()`.
`parse_id` (`:406-447`) applies **no length limits**, **no prefix charset check**, and requires
child segments to parse as `u32` (so `"bd-a.99999999999"` fails there but passes the validation
version, while `"BD-abc"` fails validation but its prefix would pass `parse_id`). Both are
re-exported (`src/util/mod.rs:20` exports the util one; `validation` shadows it locally at
`:45`). Tests for the two live separately and encode different expectations
(`src/util/id.rs:1034-1038` vs `src/validation/mod.rs:694-724`).

Note also the prefix/hash split heuristic: `issue_id_separator` (`src/util/id.rs:264-278`) takes
the **last** `-` and tries `is_likely_hash_segment` (3-char base36, or 4+ containing a digit,
`:244-262`), but falls through to the last dash regardless — so the "likely hash" test is
decorative.

### 5.7 `SyncSafetyValidator` (`:317-418`)

Encodes the "br never touches git" invariant as runtime guards + documentation.

- `validate_no_git_path(&Path)` (`:327-354`): rejects any `Component::Normal(".git")`
  (`:329-338`), then belt-and-braces string checks for `/.git/`, `\.git\`, trailing `/.git`
  (`:341-351`). `.gitignore` is explicitly allowed (`:756-763`).
- `validate_path_containment(path, beads_dir, allow_external)` (`:367-399`): `.git` check
  first (so `--allow-external-jsonl` cannot re-enable it, `:829-840`), then, unless
  `allow_external`, requires `dunce::canonicalize(path).starts_with(canonicalize(beads_dir))`
  (`:380-396`). Canonicalization **falls back to the raw path** if the file doesn't exist
  (`:381-383`), which weakens the guard for not-yet-created outputs (symlink/`..` cases).
- `assert_no_git_in_sync()` (`:411-417`) is an empty `const fn` — pure documentation.

Two meta-tests enforce the invariant statically: `sync_safety_no_git_commands_in_sync_module`
(`:846-871`) shells out to `grep -r "Command::new.*git" src/sync/` and, notably, **silently
passes if `grep` is unavailable** (`:865-869`); `sync_safety_no_git_library_dependencies`
(`:878-900`) parses `Cargo.toml`'s `[dependencies]` section for `git2`/`gitoxide`/`gix`/`libgit2`
(build-deps like `vergen-gix` are deliberately allowed, `:873-876`).

Callers of the path guards are in sync (`src/sync/mod.rs:1264`, `:1428-1440`, `:1533-1546` via
`validate_sync_path_with_external` / `validate_temp_file_path` /
`require_safe_sync_overwrite_path`).

### 5.8 `br lint` — corpus-level "validation" that isn't in this module

Worth noting because it's the user-facing "is my data good" command: it checks only that a
description contains required markdown headings by issue type
(`src/cli/commands/lint.rs:289-335`): Bug ⇒ `## Steps to Reproduce` + `## Acceptance Criteria`
(`:55-64`); Task/Feature ⇒ `## Acceptance Criteria` (`:66-69`); Epic ⇒ `## Success Criteria`
(`:71-74`); everything else ⇒ no checks. Matching is a case-insensitive `contains` of the
heading text with the `##` stripped (`:337-356`). Exit code is 1 on warnings in text mode, 0 in
JSON mode (`:43-46`).

---

## 6. Test coverage map

- `src/model/mod.rs:654-1494` — 45 unit tests: enum round-trips, FromStr matrices, Display,
  content-hash determinism/sensitivity, tombstone expiry, serde field renames.
- `src/error/mod.rs:285-332` — 5 tests (Display, suggestion, validation collapse).
- `src/error/structured.rs:919-1066` — 13 tests, including the only exercise of the unused
  convenience constructors and intent detectors.
- `src/validation/mod.rs:420-901` — 27 tests, including the sync-safety meta-tests.
- `tests/proptest_validation.rs` — property tests: valid issues pass, priority out of range
  fails, empty title fails, timestamp ordering, label charset/length, external_ref whitespace.
- `tests/e2e_errors.rs` — end-to-end assertions on the JSON envelope shape
  (`verify_error_structure`, `:556`) and per-code exit codes (2/3/4/5, `:567`, `:592`, `:622`,
  `:664`, `:693`, `:882`, `:1061`, `:1096`, `:1254`).

---

## 7. Consolidated risk / oddity list

1. **`generate_hint` short-circuit kills intent detection.** `err.suggestion()` wins before the
   `detect_*_intent` arms (`src/error/structured.rs:617-619` vs `:626-656`), so "Did you mean
   --status closed?" never reaches the user. Existing e2e assertions are satisfied by substring
   coincidence (`tests/e2e_errors.rs:630-634`).
2. **`Priority::from_str` destroys the input.** Non-numeric input becomes
   `InvalidPriority { priority: -1 }` (`src/model/mod.rs:131`), producing the misleading message
   "Priority must be 0-4, got: -1" and making priority-synonym detection impossible.
3. **Seven `StructuredError` constructors + `find_similar_ids` are dead**
   (`src/error/structured.rs:284-446`, `:898-917`). Levenshtein "did you mean bd-abc123?" is
   implemented, tested, and never shipped.
4. **Two `exit_code()` functions disagree.** `BeadsError::exit_code` (`src/error/mod.rs:251-256`,
   returns 1 or 3) is unused; `ErrorCode::exit_code` (`src/error/structured.rs:199-239`,
   returns 1–8) is authoritative via `src/main.rs:306`.
5. **Five `ErrorCode`s are unreachable**: `RequiredField`, `ConflictMarkers`, `PathTraversal`,
   `ConfigNotFound`, `ConfigParseError` — never produced by `extract_code_and_context`
   (`:492-612`). Sync failures land on `CONFIG_ERROR` (exit 7) instead of the intended exit-6
   band; `tests/e2e_errors.rs:787-791` codifies the ambiguity.
6. **Piping changes the error format.** `!io::stdout().is_terminal()` at `src/main.rs:307`
   forces JSON errors for any redirected command, diverging from `OutputContext::detect_mode`
   (`src/output/context.rs:107-119`) which would say `Plain`. `OutputMode::Toon` has no error
   representation at all.
7. **`OutputContext::error` / `error_panel` are dead** (`src/output/context.rs:306-316`,
   `:356-378`); 49 raw `eprintln!` sites under `src/cli/` emit non-structured text even under
   `--json` (e.g. bulk-create per-issue failures, `src/cli/commands/create.rs:441-540`).
8. **`DependencyValidator` and `CommentValidator` are dead** (`src/validation/mod.rs:129-178`,
   `:215-253`). Real dependency rules live in `SqliteStorage::add_dependency`
   (`src/storage/sqlite.rs:1782-1830`) and differ: duplicates silently succeed rather than
   erroring. `CommentValidator`'s `id > 0` rule is unusable pre-insert.
9. **Two `is_valid_id_format` implementations disagree** (`src/validation/mod.rs:257-300` vs
   `src/util/id.rs:485-487`) on prefix charset, length caps, and child-segment numeric range.
10. **`ResultExt`/`OptionExt` are unused** (`src/error/context.rs`), leaving
    `BeadsError::WithContext` unconstructed and its `INTERNAL_ERROR` mapping dead.
11. **Untagged `Custom` variants create a round-trip trap.** `Status::Custom("open")`
    serializes to `"open"` and deserializes back as `Status::Open`
    (`src/model/mod.rs:45-46`) — the custom/standard distinction is not preserved through JSONL
    for strings that happen to be canonical. Same shape for `IssueType` and `DependencyType`.
12. **Valid-value tables drift from the enums.** `VALID_STATUSES` omits `pinned`
    (`src/error/structured.rs:671-682`); `VALID_TYPES` omits `docs`/`question`
    (`:685-689`) even though `TYPE_SYNONYMS` maps onto them (`:730-739`) and `IssueType`
    supports them (`src/model/mod.rs:145-147`).
13. **`is_blocking()` and `affects_ready_work()` are literal duplicates**
    (`src/model/mod.rs:237-250`) — two names, one predicate, plus a third copy of the same set
    hard-coded in SQL (`src/storage/schema.rs:107-109`).
14. **Custom issue types are creatable but un-filterable.** `IssueType::from_str` never fails
    (`src/model/mod.rs:193`), so `br create --type=nonsense` succeeds
    (`tests/e2e_errors.rs:861-864`), but `br lint --type=nonsense` rejects it
    (`src/cli/commands/lint.rs:239-244`). Custom statuses are the mirror image: importable, not
    settable (`src/model/mod.rs:93-96` vs `src/storage/sqlite.rs:3282-3287`).
15. **Byte-length caps on Unicode fields.** `title.len() > 500`, `description.len() > 102_400`,
    `label.len() > 50`, `body.len() > 51_200`, `author.len() > 200`, `external_ref.len() > 200`
    all measure UTF-8 bytes (`src/validation/mod.rs:56,62,194,236,245,83`).
16. **`br update` skips `IssueValidator` entirely** — only DB CHECK constraints and arg parsing
    stand between an update and the database.
17. **`validate_path_containment` degrades open.** `dunce::canonicalize(...).unwrap_or_else(|_|
    path.to_path_buf())` (`src/validation/mod.rs:381-383`) means a non-existent target is
    compared uncanonicalized, so `..` traversal in a not-yet-created output path is not
    normalized away.
18. **The "no git in sync" static test can no-op.** If `grep` is missing it prints a warning and
    passes (`src/validation/mod.rs:865-869`).
19. **`compaction_level` is the sole always-serialized optional** (`src/model/mod.rs:475`,
    serializer at `:22-29`) purely for Go-side NULL-scanner compatibility — a wire-format wart
    that must not be "cleaned up".
20. **`Priority::default()` is P0 (critical)** by derive (`src/model/mod.rs:102`), contradicting
    the CLI/DB default of P2 (`src/config/mod.rs:870`, `src/storage/schema.rs:22`). Any code
    path that relies on `Priority::default()` or `#[serde(default)]` for a missing JSONL
    `priority` field silently gets *critical*.
