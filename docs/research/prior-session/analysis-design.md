# Design & API critique — `obr` (crate `beads_rust`)

Analyst pass. Every claim is marked **CONFIRMED** (traced in source, or reproduced against
the installed `obr 0.1.14` binary at
`/nix/store/3ks37asmlzi6368g26zi9m6436l52077-beads_rust-0.1.14/bin/obr`, which matches
`Cargo.toml:13` and contains the current version-gated open path) or **SUSPECTED**.

Experiments ran in throwaway workspaces under `scratchpad/{ws,ws2..ws9,A,B,C}`.
**The repository at `/Users/johnw/src/obr` was not modified.**

Three inherited map/web findings are **wrong** and are corrected in §0. Read that first.

---

## 0. Corrections to earlier agents' findings

### 0.1 CORRECTION — `foreign_keys` is NOT off on the common path (CONFIRMED)

`map-storage.md` and `web-sqlite-rust.md` both lead with: "`apply_schema` is skipped when
`user_version >= 1` (`src/storage/sqlite.rs:105-109`), all PRAGMAs live inside
`apply_schema` (`src/storage/schema.rs:216-227`), therefore `foreign_keys = OFF` on every
real invocation and all 9 `ON DELETE CASCADE` clauses are inert."

The *gating* half is correct (`src/storage/sqlite.rs:105-109`). The *conclusion* is wrong.
rusqlite's bundled SQLite is compiled with `SQLITE_DEFAULT_FOREIGN_KEYS=1`. Compile-option
table dumped from the shipped binary:

```
DEFAULT_CACHE_SIZE=-2000
DEFAULT_FOREIGN_KEYS          <- bare name = defined truthy
DEFAULT_SYNCHRONOUS=2
DEFAULT_WAL_SYNCHRONOUS=2
MAX_VARIABLE_NUMBER=32766
```

Behaviourally confirmed in §1.1: an `INSERT OR REPLACE` during import cascade-deleted child
rows on a *reopened* database (one where `apply_schema` was provably skipped). FK
enforcement is on; `conn.pragma_update(None, "foreign_keys", "ON")`
(`src/storage/schema.rs:220`) is redundant.

**What survives, and is still a real defect:** the *performance* pragmas genuinely do revert
on every reopen. `synchronous` reverts to FULL (compile default 2), `cache_size` to `-2000`
(2 MB, not the intended 8 MB at `schema.rs:227`), `temp_store` to FILE. The tuning at
`schema.rs:222-227` applies exactly once per database lifetime — during `init` — and never
again. Silent, total loss of intent.

**The deeper design point is worse than the original claim:** correctness now depends on a
compile-time flag of a vendored C library that appears nowhere in obr's source, is asserted
nowhere, and flips if anyone builds without rusqlite's `bundled` feature (system SQLite
defaults FK OFF). At that moment import stops cascading and starts silently orphaning rows
instead — the behaviour of the import path changes with a build flag.
`src/storage/sqlite.rs:4637-4657` (`test_pragmas_are_set_correctly`) uses `open_memory()`,
which always runs `apply_schema`, so it cannot detect any of this.

### 0.2 CORRECTION — `obr list --robot` is a hard usage error, not a silent no-op (CONFIRMED)

`map-cli.md` and `web-rust-cli-norms.md` say "`br list --robot` silently does nothing".
Reproduced:

```
$ obr list --robot ; echo $?
error: unexpected argument '--robot' found
2
```

A clap parse failure — *worse* than a silent no-op, because clap's usage exit code is **2**,
colliding head-on with obr's own `ErrorCode::exit_code()` family 2 = "Database errors"
(`src/error/structured.rs:200-207`). An agent seeing exit 2 cannot distinguish "flag doesn't
exist on this subcommand" from "database locked / not initialized", and those demand
opposite responses.

### 0.3 CORRECTION — `--db` in commands is coherent; the breakage is in the auto-hooks (CONFIRMED)

`map-cli.md` claims `--db` is honoured inconsistently across commands. I tested the mutating
path: command-level `--db` handling is actually *correct*. `discover_beads_dir_with_cli`
(`src/config/mod.rs:255-262`) derives the beads dir from `--db`, so project config, prefix
and jsonl path all follow. Running from workspace A with `--db ../B/.beads/beads.db`
correctly used B's prefix (`bbb-`) and correctly ignored A's `default_priority: 0`.

The real defect is one level up, in `main.rs`'s auto-import/auto-flush hooks, and is far
worse than the reported one — see §1.2, demonstrable cross-workspace data contamination.

---

## 1. Two confirmed data-integrity defects that fall out of design choices

### 1.1 CONFIRMED — routine import silently destroys an issue's entire audit history

Reproduction (`ws5`), no explicit sync command involved:

```
$ obr init --prefix ai ; obr create "Auto import audit probe"
$ obr update ai-387 --status in_progress ; obr update ai-387 --assignee bob
$ sqlite3 .beads/beads.db "select count(*) from events where issue_id='ai-387'"
3
# an external editor / git merge changes the title in .beads/issues.org
$ obr list                     # <-- plain READ command; auto-import fires
$ sqlite3 .beads/beads.db "select count(*) from events where issue_id='ai-387'"
0
$ obr audit log ai-387
Audit Log: ai-387
----------------------------------------
                               (empty)
```

Reproduced twice independently (`ws4`, explicit `obr sync --import-only`: 4 -> 0; `ws5`,
bare `obr list`: 3 -> 0).

**Mechanism — three deliberate decisions stacking:**

1. Import bypasses the 4-step `mutate()` protocol so importing does not re-dirty rows and
   cause export ping-pong (`src/storage/sqlite.rs:3669-3671`). Sound reasoning.
2. `upsert_issue_for_import` implements upsert as `INSERT OR REPLACE`
   (`src/storage/sqlite.rs:3687-3698`) — a *row replace* (DELETE+INSERT), not a field-level
   UPDATE.
3. `events.issue_id REFERENCES issues(id) ON DELETE CASCADE`
   (`src/storage/schema.rs:143`), and FK enforcement is on (§0.1).

REPLACE's conflict resolution performs the DELETE, CASCADE fires, the audit trail goes.
`dirty_issues`, `export_hashes` and `blocked_issues_cache` rows go the same way (9
`ON DELETE CASCADE` clauses in `schema.rs`). Labels/deps/comments self-heal only because
`sync_{labels,dependencies,comments}_for_import` (`sqlite.rs:3747/3768/3802`) rewrite them
immediately after. Nothing rewrites events.

**Unrecoverable:** events are local-only, never exported to the sync file
(`src/storage/events.rs:8`). No second copy. `obr audit log` presents them as durable.

**Blast radius:** auto-import runs before ~26 of 39 commands (`src/main.rs:32-36`, gate
`:160-210`), so *any* read command after a colleague's commit lands wipes the local audit
trail of every touched issue, with no output and exit 0.

**Design lesson:** this is the price of row-level replace as the merge primitive. Every
surviving git-native tracker surveyed in `web-related-trackers.md` / `web-sync-merge.md`
(Fossil, git-bug, Radicle, ticgit-2026) decomposes below record level for exactly this
reason. A field-level `ON CONFLICT DO UPDATE` here is a two-line fix, but the general point
stands: obr's wire unit (whole-issue snapshot) forces whole-row writes, which forces
cascade loss.

### 1.2 CONFIRMED — `--db` + auto-flush writes one workspace's issues into another workspace's tracked file

Reproduction (`C`, a freshly-initialised workspace):

```
$ cd C && obr init --prefix ccc
$ grep -c ':ID:' .beads/issues.org      # 0
$ obr create "Created into B from empty C" --db ../B/.beads/beads.db
✓ Created bbb-1yl: Created into B from empty C
$ grep ':ID:' .beads/issues.org
:ID:  bbb-169
:ID:  bbb-1v4
:ID:  bbb-1yl
:ID:  bbb-23m
:ID:  bbb-2d9
:ID:  bbb-2jc
:ID:  bbb-vb5
$ sqlite3 .beads/beads.db 'select count(*) from issues'   # 0
```

Workspace **C**'s git-tracked `issues.org` now contains all seven of workspace **B**'s
issues under B's `bbb-` prefix, while C's own database is still empty. Exit 0, no warning.
The next `git add .` commits B's tracker into C's history.

**Mechanism — two independent resolutions of the same path:**

- `run_auto_flush` (`src/main.rs:259`) discovers the beads dir with
  `config::discover_beads_dir(Some(Path::new(".")))` (`src/main.rs:261`) — **ignoring
  `--db`** — although `discover_beads_dir_with_cli` exists at `src/config/mod.rs:255` for
  exactly this and is used by ~26 command modules.
- It then opens `config::open_storage(&beads_dir, overrides.db.as_ref(), …)`
  (`src/main.rs:273`), which *does* honour `--db` and correctly computes
  `paths.jsonl_path` as the `--db` sibling (`src/config/mod.rs:562-567`)…
- …and the correctly-resolved paths are **explicitly discarded**:
  `let (mut storage, _paths) = …` (`src/main.rs:273`).
- `auto_flush(storage, beads_dir)` (`src/sync/mod.rs:1918`) then re-derives the export path
  a *fourth* time from `Metadata::load(beads_dir)` on the locally-discovered dir
  (`src/sync/mod.rs:1929-1938`).

Net: DB from `--db`, destination file from `$PWD`.

**The near-miss and a second bug.** In the non-empty case the DL-2 stale-database guard
caught it. From `A` (3 issues) with `--db ../B` (6 issues), at `RUST_LOG=trace`:

```
DEBUG obr: Auto-flush failed (non-fatal) e=Config("Refusing to export stale database
  that would lose issues.\nDatabase has 6 issues, JSONL has 3 unique issues.\n
  Export would lose 3 issue(s): aaa-26p, aaa-2lw, aaa-3fz\n
  Hint: Run import first, or use --force to override.")
```

- **Genuine strength.** The data-loss guards (`src/sync/mod.rs:1296`, `:1306`) are
  load-bearing and earned their keep against a bug they were not written for. Given the
  repo's real history (`bd sync` destroying the source tree on 2026-01-19, commits b8020fa /
  a326295), this is the best-justified design in the project.
- **Second confirmed bug.** The refusal is swallowed at `debug!`
  (`src/main.rs:294-296`). At default verbosity the user sees only `✓ Created bbb-2jc` and
  exit 0. Every mutation through `--db` from a foreign directory silently accumulates
  unexported dirty rows: after five creates, B's DB had `dirty_issues = 5` and B's
  `issues.org` was four issues behind, with no signal at any point. For a tool whose entire
  value proposition is "the file in git is the truth", silently failing to write the file
  and exiting 0 is the worst available failure mode.

---

## 2. The three-layer error system: right idea, wrong proportions

Layers: `BeadsError` (thiserror, 31 variants, `src/error/mod.rs:28-164`) ->
`StructuredError` (`src/error/structured.rs:250-264`) -> `ErrorCode` (33 variants,
`structured.rs:32-116`) with `exit_code()` (`:199-239`) and `is_retryable()` (`:174-185`).

**Verdict: the layering is justified; the implementation is simultaneously over-built at the
top and under-used at the bottom, and its most-advertised features are dead.**

### 2.1 Justified

A wire-stable SCREAMING_SNAKE code decoupled from Rust variant names is correct — it lets
variants be renamed/merged without breaking agents, which thiserror alone cannot give.
Exit-code *families* (2-8 by class, `structured.rs:199-239`) and the `retryable` flag are
exactly what `web-rust-cli-norms.md` says agent CLIs need, and obr had them before the
discourse crystallised. Emitting JSON errors on stderr whenever stdout is not a TTY
(`src/main.rs:309-310`), not just under `--json`, is genuinely ahead of clig.dev.

### 2.2 CONFIRMED — every "smart" constructor is dead code

`StructuredError::issue_not_found` (`:284`), `ambiguous_id` (`:311`), `cycle_detected`
(`:338`), `not_initialized` (`:357`), `invalid_priority` (`:369`), `invalid_status` (`:404`),
`invalid_type` (`:427`) have **zero** production call sites. `grep -rn "StructuredError::"`
across `src/` returns only the definitions and four unit tests at `:1019/1026/1034/1042`.
The single live construction path is `StructuredError::from_error(err)` at `src/main.rs:305`.

So `find_similar_ids` (`:898`) and `levenshtein_distance` (`:858`) — the "did-you-mean" ID
suggester that `map-output.md` lists as an agent-friendliness feature and that
`AGENT_FRIENDLINESS_REPORT.md` scores — **never run**. Confirmed:

```
$ obr show demo-zzz
{"error":{"code":"ISSUE_NOT_FOUND","message":"Issue not found: demo-zzz",
 "hint":"Run 'br list' to see available issues.","retryable":false,
 "context":{"searched_id":"demo-zzz"}}}
```

No `context.similar_ids`; the hint is the generic fallback from `generate_hint`
(`:623-625`) — and it names the wrong binary (`br`).

Not a small thing: `web-rust-cli-norms.md` records that Arcjet *deliberately disabled* fuzzy
suggestions for agents because agents treat suggestions as confirmation. obr built the
feature, shipped it in the docs, and ships it disabled. Either way, code and claim must agree.

### 2.3 CONFIRMED — four of seven `generate_hint` arms are structurally unreachable

`generate_hint` (`structured.rs:615`) checks `err.suggestion()` **first** and returns early
if `Some` (`:617-619`). `BeadsError::suggestion()` (`src/error/mod.rs:225-244`) returns
`Some` for `InvalidPriority`, `InvalidStatus`, `InvalidType`, `HasDependents`. So the arms at
`structured.rs:626-632`, `:633-639`, `:640-646`, `:647-656` — the ones calling the intent
detectors and producing the *better* hints — can never execute. Verified end-to-end: an
invalid priority yields the static `"Use a priority between 0 (critical) and 4 (backlog)"`
from `mod.rs:235-237`, never `"Did you mean --priority N?"` from `structured.rs:628`.

### 2.4 CONFIRMED — the same error carries two contradictory hints in one payload

For `InvalidStatus`, `extract_code_and_context` (`structured.rs:533-544`) embeds its *own*
`hint` inside `context`, while `generate_hint` puts a different string in top-level `hint`.
An agent reading `error.hint` gets the generic list; the useful
`"Did you mean --status closed?"` is buried at `error.context.hint`. Same for `InvalidType`
(`:545-556`) and `InvalidPriority` (`:557-570`). Three hint-generation mechanisms
(`BeadsError::suggestion`, `generate_hint`, inline-in-context) for one field.

### 2.5 CONFIRMED — three public `BeadsError` methods are dead, one documents a lie

`BeadsError::exit_code()` (`mod.rs:251`), `is_user_recoverable()` (`:197`),
`suggests_force()` (`:214`) have zero callers in `src/`, `tests/` or `benches/`. The live
exit code comes from `structured.code.exit_code()` (`main.rs:306`). Worse,
`BeadsError::exit_code`'s doc (`mod.rs:248-249`) asserts *"Legacy bd typically uses exit code
1 for most errors"* and returns 1 — directly contradicting the 2-8 taxonomy the binary
emits. Anyone reading `error/mod.rs` to learn obr's exit contract learns the wrong one.

### 2.6 CONFIRMED — the taxonomy is fiction exactly where it matters most

Five `ErrorCode` variants are unreachable via `from_error`: `RequiredField`,
`ConflictMarkers`, `PathTraversal`, `ConfigNotFound`, `ConfigParseError`. Two are the
flagship safety codes.

Reproduced (`ws6`) — conflict markers in `.beads/issues.org`:

```
$ obr sync --import-only ; echo $?
{
  "error": {
    "code": "CONFIG_ERROR",
    "message": "Configuration error: Merge conflict markers detected in …issues.org.\n
                …issues.org:16 Start (HEAD)\n…:18 Separator\n…:20 End (branch)\n
                Resolve conflicts before importing.",
    "hint": null,
    "context": null
  }
}
7
```

Expected per the enum: `CONFLICT_MARKERS`, exit 6. Actual: `CONFIG_ERROR`, exit 7. The
detector at `src/sync/mod.rs:1112-1116` returns `BeadsError::Config(String)`, as does every
path-safety rejection in `src/sync/path.rs` (`:398, :462, :487, :496, :549, :560, :603,
:615, :627, :645`). There are **59** `BeadsError::Config(...)` construction sites across 11
files.

The most safety-critical, most structured error in the system — file, line, marker type,
branch, all available — is flattened into a prose blob with embedded newlines,
`hint: null`, `context: null`, under the most generic code in the taxonomy. The `context`
field that exists for exactly this is unused precisely where it would pay.

**Net verdict.** Three layers is not over-built for an agent CLI. But ~half of layer 2 is
dead, layer 1's public API is 3/4 dead and self-contradictory, and layer 3's taxonomy does
not describe layer 1's behaviour. The honest fix is *subtraction*: delete the dead
constructors and dead `BeadsError` methods, delete `generate_hint`'s unreachable arms,
delete-or-wire Levenshtein — then spend the saved complexity replacing
`BeadsError::Config(String)` with typed variants for the ~6 sync-safety cases that already
have `ErrorCode`s waiting for them.

---

## 3. Output modes: five modes, one confused enum, three parallel dispatch mechanisms

### 3.1 CONFIRMED — the enum conflates encoding with presentation, and it costs correctness

`OutputMode` (`src/output/context.rs:25-37`) has five variants — `Rich, Plain, Json, Toon,
Quiet` — mixing two orthogonal axes: *encoding* (text/json/toon/csv) and *presentation
intensity* (rich/plain/quiet). `Quiet` competes with `Json` rather than modifying it, and
`Csv` has no variant at all.

Reproduced (`ws8`):

```
$ obr list --format csv | head -2
id,title,status,priority,issue_type,assignee,created_at,updated_at
rt-294,emoji 🎯 ok,open,2,task,,…

$ obr list --format csv --quiet
                                       (nothing at all — not even the header)
```

Root cause traced: `OutputContext::from_output_format` (`context.rs:81-102`) folds
`OutputFormat::Csv` into the `Text | Csv` arm, so `--quiet` maps Csv to `OutputMode::Quiet`;
`list.rs:77-79` then early-returns before reaching the `OutputFormat::Csv` branch at
`list.rs:117-121` that would have printed. `--quiet` on a *data* encoding should suppress
chatter, not data.

Three incompatible type-level representations of "what stdout should look like" coexist:
`OutputFormat` (text/json/csv/toon), `OutputFormatBasic` (text/json/toon), `OutputMode` (the
five), plus a bare `String` for `dep tree` accepting `text|mermaid`
(`src/cli/mod.rs:1552-1554`).

### 3.2 CONFIRMED — `--robot` is inconsistent *within a single subcommand*

`src/cli/commands/sync.rs:97` computes `use_json = ctx.is_json() || args.robot`, then passes
`use_json` to `execute_status` (`:110`) and `execute_import` (`:152`) but the raw `json`
parameter to `execute_flush` (`:129`) and `execute_merge` (`:139`). Reproduced:

```
$ obr sync --flush-only --robot
Exported:
  2 issues
  …
$ obr sync --flush-only --json
{"exported_issues":3,"exported_dependencies":0,…}
$ obr sync --status --robot
{"dirty_count":0,"last_export_time":"…",…}
```

`--robot` works for `--status` and `--import-only`, is a no-op for `--flush-only` and
`--merge` — and `--flush-only` is the mode agents call most.

`--robot` reaches its command through four different mechanisms across the 10 arg structs
declaring it: `resolve_output_format_basic(..., args.robot)` (ready.rs:41, blocked.rs:40,
stats.rs:39); `ctx.is_json() || args.robot` (orphans.rs:50, defer.rs:141, sync.rs:97);
`cli.json || args.robot` computed in `main.rs` (`:58, :61, :78, :94, :101, :104, :107,
:110`); and `json || args.robot` (reopen.rs:50). The other 29 subcommands reject it (§0.2).

### 3.3 CONFIRMED — `--robot` never reaches the error path

`handle_error(&e, cli.json)` (`src/main.rs:129`) only sees the *global* `--json`. On a TTY,
`obr ready --robot` emits JSON on stdout and a human-readable, ANSI-coloured error on
stderr — a mixed-mode failure. Rescued only when stdout is redirected, by the
`!io::stdout().is_terminal()` fallback at `main.rs:310`.

### 3.4 CONFIRMED — `--quiet` is honoured per-call-site, not by the abstraction

```
$ obr stale --quiet
Stale issues (0 not updated in 30+ days):
```

`stale.rs` uses the outer context and falls through to `println!`. Across
`src/cli/commands/` there are 382 raw `println!` and 49 `eprintln!` against roughly 82
`ctx.print/success/info` calls — the `OutputContext` abstraction governs about a fifth of
the tool's output, so any mode guarantee it makes is advisory.

### 3.5 CONFIRMED — ANSI leaks into "Plain" mode, contradicting the documented guarantee

Two independent colour decisions: `config::should_use_color` (`src/config/mod.rs:892-907`)
ranks config `display.color` **above** `NO_COLOR`, while `OutputContext::from_output_format`
(`context.rs:89`) checks `NO_COLOR` itself. Reproduced (`ws8`) with `display.color: true`,
`NO_COLOR=1`, stdout piped:

```
$ NO_COLOR=1 obr list | cat -v
^[[mM-bM-^WM-^K^[[m rt-294 [M-bM-^WM-^O ^[[mP2^[[m] [task] - emoji 🎯 ok
```

`ctx.mode() == Plain`, `use_color == true`, ANSI on the wire. AGENTS.md:418 guarantees
"Plain mode: No ANSI escape codes"; NO_COLOR's spec is that presence disables colour
unconditionally.

### 3.6 CONFIRMED — `obr list | head` panics and exits 134

```
$ set -o pipefail
$ obr list --limit 400 | head -c 5 >/dev/null ; echo $?
thread 'main' panicked at library/std/src/io/stdio.rs:1165:9:
failed printing to stdout: Broken pipe (os error 32)
134
$ obr list --limit 400 --format toon | head -c 5 >/dev/null ; echo $?
… panicked … 134
$ obr list --limit 400 --json | head -c 5 >/dev/null ; echo $?
0
```

Text and TOON use bare `println!`; `panic = "abort"` (`Cargo.toml:94`) turns the panic into
SIGABRT, so the status is 134 — outside obr's documented 1-8 taxonomy — and stderr carries a
Rust panic message instead of the JSON error envelope. JSON survives only because
`json`/`json_pretty` (`context.rs:189-225`) write through a `BufWriter` and tolerate the IO
error; `ctx.toon`/`toon_with_stats` (`:237/:247`) use `println!` and need the same treatment.

### 3.7 CONFIRMED — JSON shapes are heterogeneous, unversioned, unenveloped

Sampled across commands (`ws5`/`ws9`):

| command | `--json` top-level shape |
|---|---|
| `list`, `ready`, `search`, `blocked`, `stale`, `orphans`, `dep list`, `label list`, `comments list` | bare array |
| **`show <id>`** | **bare array of length 1** |
| `stats`, `count`, `lint`, `where`, `info`, `changelog`, `query list`, `audit log`, `history list` | object, ad-hoc keys |
| `doctor` | object — and **compact**, not pretty |
| `version` | object — **compact**, not pretty |
| `schema` | `{tool, generated_at, schemas}` |

- `obr show <id> --json` returning `[{…}]` is a schema surprise for the one command whose
  result is definitionally singular.
- Pretty/compact is inconsistent (70 `json_pretty` vs 10 `json` sites), so an agent's byte
  budget varies by which command it calls.
- No `{data, metadata, errors}` envelope and no version field anywhere — self-flagged in
  `AGENT_FRIENDLINESS_REPORT.md:66-71`, while upstream bd v2.0 is adding a default
  `{"schema_version":1,"data":…}` envelope (`web-beads-upstream.md`).
- Entity shape differs per command for the same entity. `ready --json` items have 8 keys
  (`created_at, created_by, id, issue_type, priority, status, title, updated_at`);
  `list --json` items have 13, including `compaction_level` and `original_size`, both always
  `0`. `compaction_level` is *forced* into every record by a custom serializer
  (`src/model/mod.rs:24-29, :475`) for bd parity — pure token waste in the highest-volume
  output.
- `--fields` exists (`src/cli/mod.rs:1404`) but only for CSV. The single highest-leverage
  agent token lever is wired to the least-used encoding.

### 3.8 CONFIRMED — machine-readable output contains build placeholders and the wrong binary name

```
$ obr version --json
{"version":"0.1.14","build":"release","commit":"VERGEN_IDEMPOTENT_OUTPUT",
 "branch":"VERGEN_IDEMPOTENT_OUTPUT","rust_version":"1.95.0-nightly",…}
$ obr version
br version 0.1.14 (release) (VERGEN_IDEMPOTENT_OUTPUT@VERGEN_)
```

The shipped binary reports its own name as `br` and its commit as a vergen sentinel (a
consequence of building in the Nix sandbox without git metadata). `obr --help`'s first line
is still "Agent-first issue tracker (SQLite + JSONL)" although the default export format has
been Org since `src/config/mod.rs:33`.

---

## 4. Agent-friendly CLI claims vs. the norms

### 4.1 What genuinely holds up

- Strict stdout/stderr separation; tracing forced to stderr (`src/logging.rs:23-24`).
  Verified: `obr ready --json 2>/dev/null` is clean parseable JSON.
- JSON error envelope on stderr whenever stdout is not a TTY (`main.rs:309-310`) — ahead of
  clig.dev, matches Arcjet / Agent-Surface.
- Exit-code *families* by error class (2-8) rather than sysexits or a flat 1.
- `retryable` boolean on every error.
- Runtime schema introspection (`obr schema <target>`) generated by schemars from the live
  types, so it cannot drift (unlike the hand-mirrored `ErrorEnvelope` at
  `src/cli/commands/schema.rs:21-38`).
- Destructive commands never prompt. `obr delete` has `--dry-run`, `--force`, `--cascade`,
  and is a tombstone rather than a hard delete. Exactly right for agents.
- `--no-db` mode: a real, non-obvious affordance for read-only/ephemeral contexts.
- CSV output is RFC 4180-correct — verified round-trip through Python's `csv` module for
  titles containing commas, embedded quotes and embedded newlines.
- `obr config list` prints a `Computed values:` section showing what the tool *actually*
  resolved rather than what you typed — the single best defence against §5.2.

### 4.2 CONFIRMED — the exit-code contract has two collisions and is undocumented

- **clap usage errors exit 2**, colliding with the "Database errors" family
  (`structured.rs:200-207`). Verified: `obr list --nosuchflag` -> 2; `NOT_INITIALIZED` -> 2.
- **`NOTHING_TO_DO` shares exit 3 with `ISSUE_NOT_FOUND`** (`structured.rs:208-213`):

```
  $ obr close rt-3fy            # already closed
  Warning: Skipped rt-3fy: already closed
  {"error":{"code":"NOTHING_TO_DO","message":"Nothing to do: all 1 issue(s) skipped",
    "hint":"All specified issues were already closed or not found.","retryable":false,…}}
  $ echo $?
  3
```

  An agent retrying a close cannot distinguish "already done" (benign) from "issue doesn't
  exist" (real failure) without parsing JSON. The hint conflates them ("already closed **or
  not found**"), and `retryable:false` is asserted for a state that is already satisfied.
- Neither `obr --help` nor any subcommand help mentions exit codes
  (`obr --help | grep -ci exit` -> 0). cli-agent-lint's PV-* checks require *documented*
  distinct exit codes; obr has the taxonomy and doesn't publish it.

### 4.3 CONFIRMED — idempotency policy is inconsistent across mutating commands

| repeated operation | exit |
|---|---|
| `obr label add <id> foo` twice | 0 |
| `obr dep add <a> <b>` twice | 0 |
| `obr close <id>` twice | **3** |

Cursor's and Composio's agent-CLI criteria both require a successful command run twice to be
a no-op or an explicit "already done", not a failure. obr has all three behaviours and
documents none.

### 4.4 CONFIRMED — no examples in help, contra clig.dev

`obr --help | grep -ci example` -> 0; `obr create --help | grep -ci example` -> 0.
clig.dev ("Lead with examples") and Cursor ("examples do more than prose for
pattern-matching") both make this a primary rule. Cheapest possible fix, and with `trycmd`
those examples become executable regression tests of the frozen CLI contract.

### 4.5 CONFIRMED — machine-readable artifacts point at a binary that does not exist

The hint on the most common error names the wrong tool: `"Run 'br list' to see available
issues."` (`structured.rs:624`, reproduced live in §2.2). `--no-daemon`'s help ships as
"effectively no-op in **br** v1" (`src/cli/mod.rs:670-672`, visible in `obr delete --help`).
`CLI_SCHEMA.json` says `"binary":"br"`. An agent following the machine-readable examples
verbatim gets command-not-found.

---

## 5. Config discovery & routing: layered where it needn't be, unvalidated where it must be

### 5.1 CONFIRMED — 7 precedence layers over a stringly-typed map, with no schema

`load_config` (`src/config/mod.rs:815-840`) merges seven `ConfigLayer`s. A `ConfigLayer`
(`:593-597`) is two `HashMap<String,String>`s. There is no typed `Config` struct anywhere;
every consumer re-enumerates accepted spellings at its own call site:
`get_value(layer, &["issue_prefix", "issue-prefix", "prefix"])` (`:845`),
`&["display.color", "display-color", "display_color"]` (`:888`),
`&["max_collision_prob", "max-collision-prob"]` (`:857`). Env vars are multiplied into three
spellings each on ingest (`:640-647`).

Nothing owns the set of legal keys. Adding one means touching two places (the reader and
`is_startup_key` at `:1030-1063`); forgetting one is silent.

### 5.2 CONFIRMED — unknown keys are accepted, echoed back, and ignored

Reproduced (`ws2`) with this `.beads/config.yaml`:

```yaml
issue_prefixx: "typo"
id:
  prefix: "nested"
default_priority: 9
totally_unknown_key: yes
```

```
$ obr config list
Runtime settings:
  default_priority: 9
  id.prefix: nested
  issue_prefix: proj
  issue_prefixx: typo
  totally_unknown_key: yes
Computed values:
  prefix: proj          <-- id.prefix ignored
$ obr config get id.prefix
nested                  <-- reads back fine, has no effect
$ obr create "x"
{"error":{"code":"INVALID_PRIORITY","message":"Priority must be 0-4, got: 9", …}}
```

Three failures at once: (a) typo'd and wholly invented keys are accepted silently; (b) YAML
dot-flattening makes `id.prefix` *readable* via `config get` while ID generation ignores it
— exactly the README's documented-but-wrong example, and it survives naive spot-checking;
(c) an invalid `default_priority` is not caught at config-load but at first `create`, so the
config can be broken arbitrarily long before anything says so. A
`#[derive(Deserialize)] #[serde(deny_unknown_fields)]` typed config validated at load
eliminates all three. The `Computed values:` block is a good mitigation and deserves credit
— but it's a diagnostic bolted onto a design that shouldn't need one.

### 5.3 CONFIRMED — 80% of `config/routing.rs` is dead

510 lines. Of 10 public items, **8 have zero references outside the file**: `resolve_route`
(`:249`), `is_external_id` (`:325`), `find_town_root` (`:85`), `load_routes` (`:110`),
`find_route` (`:148`), `extract_prefix` (`:77`), `RouteEntry` (`:30`), `RoutingResult`
(`:39`). Only `read_redirect` (`:162`) and `follow_redirects` (`:200`) are live. The entire
documented "Resolution Order" block at `routing.rs:12-19` describes machinery that never
executes. This is Go bd's Gastown "mayor/town" routing — explicitly listed as an *exclusion*
in `PLAN_TO_PORT…md:399-437` — ported anyway and then abandoned.

The multi-repo mechanism that *does* work is `external_projects` (`config/mod.rs:914-991`),
consumed by `ready`, `blocked`, `dep`. Simple and effective. The dead 400 lines should go.

### 5.4 CONFIRMED — four independent implementations of "where is the export file"

1. `config::resolve_jsonl_path` (`src/config/mod.rs:547-590`) — the real 5-tier one.
2. `sync::auto_flush` (`src/sync/mod.rs:1929-1938`) — re-derives from `Metadata::load`.
3. `sync::path::validate_sync_path{,_with_external}` (`src/sync/path.rs:211`, `:450`).
4. `cli::commands::sync::validate_sync_paths` (`src/cli/commands/sync.rs:159`), alongside
   `validation::SyncSafetyValidator::validate_path_containment`
   (`src/validation/mod.rs:367`) — arguably five.

§1.2 is the direct cost of #1 and #2 disagreeing. Nine `discover_beads_dir(None|".")` call
sites (blocked.rs:33, config.rs:92/105/264/418/585, doctor.rs:840, orphans.rs:49,
where.rs:33) versus ~26 `discover_beads_dir_with_cli` sites is the same problem in the
discovery dimension.

---

## 6. Port fidelity vs. idiomatic Rust: where conformance actually bites

**Genuinely forced by bd parity, handled well:**

- `empty_to_none` (`src/storage/sqlite.rs:3066-3072`). bd's schema uses
  `NOT NULL DEFAULT ''`; Rust wants `Option`. Reconciled at exactly *one* boundary, with the
  reason written down. The right pattern.
- `#[serde(untagged)] Custom(String)` on `Status`/`IssueType` (`src/model/mod.rs:45-46`,
  `:148-149`). bd allows config-defined custom types; a closed Rust enum cannot. Correct
  call — `web-beads-upstream.md` confirms v0.46 bd already had `message`, `molecule`, `gate`
  etc. plus user-defined types, so a closed enum would reject valid classic data.
- `compaction_level`'s always-emit serializer (`model/mod.rs:24-29`). Ugly, forced, isolated.
- The 11-table schema mirroring bd, `IF NOT EXISTS` throughout, no triggers.

**NOT forced by bd parity — avoidable un-idiomatic choices:**

- **36-column positional row mapping.** `issue_from_row` (`sqlite.rs:3075-3137`) reads
  `row.get(0)` … `row.get(35)`. The identical 36-column SELECT list is hand-duplicated at
  **11** sites (`sqlite.rs:217, 699, 736, 768, 934, 1068, 1475, 2691, 3606, 3631, 3689`). A
  reorder in any one silently corrupts data with no compile error. rusqlite supports
  `row.get("column_name")`; a `const ISSUE_COLUMNS: &str` plus named access removes the
  hazard entirely. bd's schema constrains the *columns*, not how Rust reads them. Open as
  the project's own bead `beads_rust-2ckc`.
- **Four-way divergent definition of "valid status".** (1) `Status::from_str`
  (`model/mod.rs:84-97`) accepts 8 spellings including `inprogress`; (2) `VALID_STATUSES`
  (`structured.rs:671-682`) lists 6, omitting `pinned`; (3) the human hint
  (`error/mod.rs:239`) lists 5, omitting `tombstone` and `pinned`; (4) serde accepts anything
  via `Custom`. Same for types: `VALID_TYPES` (`structured.rs:686`) lists 5 while `IssueType`
  has 7 (`model/mod.rs:141-149`), and `TYPE_SYNONYMS` maps `"doc"->"docs"` (`:730`) — a
  target not in `VALID_TYPES`, so `detect_type_intent("doc")` returns `None`. None of this is
  bd's fault; four hand-maintained lists that should be one.
- **Asymmetric validation boundary.** `Status::from_str` *rejects* an unknown status from the
  CLI but serde *accepts* one from the file. Since the file is hand-editable and git-merged —
  the entire point of the Org migration — the looser boundary is the one that matters.
  Reproduced (`ws7`), and it is worse than "accepted as custom": a hand-edited TODO keyword
  the Org parser doesn't recognise is folded into the **title**:

```
  # .beads/issues.org edited: "* TODO [#C] Status validation probe" -> "* FROBNICATE [#C] …"
  $ obr sync --import-only && obr list
  ○ st-2pf [● P2] [task] - FROBNICATE [#C] Status validation probe
```

  Status silently reset to `open`, priority silently reset to P2, the cookie absorbed into
  the title. Exit 0, no warning. A merge that mangles one keyword rewrites the issue.
  (Credit: a 10-title adversarial export->import round-trip found only *one* other
  infidelity — trailing whitespace in a title is trimmed. The Org bridge is otherwise solid.)
- **Exit codes.** obr *diverges* from bd here (bd uses 1) while the code claims parity
  (`error/mod.rs:248-249`). Diverging is the right call for agents; the stale comment is not.

---

## 7. Module boundaries: not god *files* so much as one god *object*

Sizes: `src/storage/sqlite.rs` 5157 lines (≈3880 production + 1270 inline tests),
`src/sync/mod.rs` 5139 (≈2985 + 2150), `src/cli/mod.rs` 2392, `src/config/mod.rs` 2205,
`src/error/structured.rs` 1066, `src/storage/schema.rs` 1075. Total `src/` ≈ 53.5k lines
against `docs/ARCHITECTURE.md:38`'s claim of "~33k".

### 7.1 CONFIRMED — `SqliteStorage` is a god object with ~88 public methods

`grep "^    pub fn" src/storage/sqlite.rs` -> 97 (89 on `SqliteStorage`, 5 on
`MutationContext`, 1 test-only), spanning at least twelve unrelated concerns: connection &
migration, issue CRUD, query/filter builders, blocked-cache maintenance, external cross-repo
dependency resolution, dependency graph ops, labels, comments, a config key/value store, a
metadata key/value store, dirty tracking, export-hash bookkeeping, import upsert, cycle
detection, epic counts.

Exactly **one** trait in the codebase abstracts storage (`validation::DependencyStore`,
`src/validation/mod.rs:105`), used by the validator, not the CLI. The 39 command modules make
100 direct references to the concrete `SqliteStorage`.

"Add a trait" is not the right criticism — YAGNI applies with one backend, and
`web-sqlite-rust.md` is right that SQLite's multi-process story is the moat. The problem is
the surface itself:

- **14 public methods have no caller outside `sqlite.rs`**: `get_issues_by_ids`,
  `get_epic_counts`, `get_parent_id`, `clear_all_dirty_issues`, `set_export_hash`,
  `get_issues_needing_export`, `delete_metadata`, `count_all_issues`, `set_metadata_in_tx`,
  `clear_all_export_hashes_in_tx`, `get_dependencies_full`, `clear_all_dirty_flags`,
  `count_exportable_issues`, `dependency_exists_between`.
- **Three exact-duplicate pairs**: `clear_dirty_issues` (`:2825`) / `clear_dirty_flags`
  (`:3442`); `clear_all_dirty_issues` (`:2850`) / `clear_all_dirty_flags` (`:3462`);
  `count_issues` (`:1720`) / `count_all_issues` (`:3017`). Verbatim-equivalent bodies,
  different names, both public.

### 7.2 CONFIRMED — the file size actively undermines the mutation protocol

The 4-step `mutate()` protocol (`sqlite.rs:148-195`) is the project's flagship invariant,
enforced only by the privacy of the `conn` field (`sqlite.rs:17-19`) — i.e. by the module
boundary. But the module is one 5157-line file, so "inside the boundary" means 5157 lines of
trusted code, containing **12** direct `self.conn.execute(…)` write paths that bypass the
protocol (`:2656, :2841, :2851, :2886, :2924, :2993, :3452, :3463, :3687, :3754, :3779,
:3813`).

Splitting `sqlite.rs` into `storage/{issues,deps,labels,comments,config,sync_support}.rs`
with `conn` private to a small `storage/conn.rs` would make the protocol enforced *by the
compiler* rather than by discipline. That is a stronger argument for the split than "5000
lines is too many".

### 7.3 CONFIRMED — two layering inversions

- **`config` <-> `sync` mutual dependency.** `src/config/mod.rs:17-19` imports
  `export_to_jsonl_with_policy`, `import_from_jsonl`, `finalize_export`, `ExportConfig`,
  `ImportConfig` from `crate::sync`; `src/sync/mod.rs:1929, :1938` imports
  `crate::config::Metadata` and `crate::config::DEFAULT_JSONL_FILENAME`. Concretely,
  `open_storage_with_cli` performs a full JSONL import in `--no-db` mode
  (`config/mod.rs:389-400`) — the configuration layer doing bulk data I/O. §1.2 lives in
  this cycle.
- **`storage` depends on `format`.** `src/storage/sqlite.rs:4` imports
  `crate::format::{IssueDetails, IssueWithDependencyMetadata}`. `get_issue_details`
  (`sqlite.rs:3029`) is a SQL-layer method returning a *presentation* DTO carrying
  `#[derive(JsonSchema)]` for `obr schema`. A change to the JSON output contract forces a
  change in the SQL layer.

### 7.4 CONFIRMED — a hard-coded constant duplicated six times, and one place that forgot it

`const SQLITE_VAR_LIMIT: usize = 900;` is redeclared as a *local* const at `sqlite.rs:725,
2157, 2545, 2586, 3311`. But `clear_dirty_issues` (`:2825-2843`) and `clear_dirty_flags`
(`:3442-3455`) build an unbounded `IN (?,?,…)` list with no chunking, and
`finalize_export` (`src/sync/mod.rs:1874`) calls `clear_dirty_issues` with **every exported
id**.

**SUSPECTED failure** (not reproduced — needs >32 766 issues): bundled SQLite has
`MAX_VARIABLE_NUMBER=32766` (confirmed from the binary's compile options). A first full
export above that size would succeed in writing and atomically renaming the file, then fail
inside `finalize_export` with "too many SQL variables" — *after* the file is on disk but
*before* dirty flags are cleared and before `jsonl_content_hash` / `last_export_time`
metadata is written (`sync/mod.rs:1883-1884`). The DB would then look permanently dirty and
permanently stale, re-exporting everything on every invocation. And because `finalize_export`
runs no transaction, a partial failure leaves dirty flags cleared but export hashes only
partly written. Additionally `set_export_hashes` (`sqlite.rs:2900-2914`) loops N autocommit
`INSERT OR REPLACE`s outside any transaction — one implicit transaction per issue.

---

## 8. Over-engineering and under-engineering, itemised

### Over-engineered (delete or wire up)

| item | evidence |
|---|---|
| 7 `StructuredError` "smart" constructors + Levenshtein suggester | zero production callers; `structured.rs:284-446, 858-916` |
| 4 unreachable arms of `generate_hint` | shadowed by `err.suggestion()`; `structured.rs:617` vs `626-656` |
| 3 dead `BeadsError` methods, one with a misleading doc | `error/mod.rs:197, 214, 251` |
| 5 unreachable `ErrorCode` variants incl. the two safety codes | `structured.rs:67, 89, 91, 97, 99` |
| ~400 lines of town/route resolution | 8/10 public items unused; `config/routing.rs` |
| ~1,927 lines of a second, abandoned output stack | `src/format/{context,theme,rich,syntax,markdown}.rs` |
| dead output components incl. a name-colliding `ProgressTracker` | `src/output/components/{dep_tree,stats,progress}.rs` |
| 14 dead public storage methods + 3 exact-duplicate method pairs | §7.1 |
| ~530 lines of `preflight_export`/`preflight_import` with no production caller | `sync/mod.rs:489, 719`; logic re-implemented twice more |
| `--no-daemon`, `identity`, `flush-debounce`, `remote-sync-interval`, `no-git-ops`, `no-push`, `sync-branch`, `hierarchy.max-depth` | parsed, classified, never consumed |
| 5 output modes over 2 orthogonal axes; 3 `--format` enums + 1 bare String | §3.1 |

### Under-engineered (complexity that should exist and doesn't)

| gap | consequence |
|---|---|
| No typed config with `deny_unknown_fields` | §5.2 — typos accepted, echoed, ignored |
| No single source of truth for the 36-column projection | 11 duplicated SELECT lists; positional indices; bead `2ckc` |
| No typed variants for sync-safety errors | 59 `BeadsError::Config(String)`; `CONFLICT_MARKERS`/`PATH_TRAVERSAL` unreachable |
| No transaction around import Phase 3 or `finalize_export` | partial-write windows; §7.4 |
| No `BrokenPipe` handling | exit 134 on `obr list \| head`; §3.6 |
| `--fields`/NDJSON only for CSV | no token lever for the encodings agents use |
| No output envelope / version field | can't evolve the JSON contract safely; upstream v2.0 is adding one |
| No `--robot` as a global alias | 4 dispatch mechanisms, inconsistent within `sync` |
| No documented exit-code table, no `--help` examples | §4.2, §4.4 |
| Auto-flush failures logged at `debug!` only | §1.2 — silent non-persistence at exit 0 |

---

## 9. Genuine strengths worth preserving

1. **The sync safety layer.** Data-loss guards (`sync/mod.rs:1296, :1306`), the
   never-bypassable conflict-marker scan (`:1091`, called `:2221`), the runtime `.git`
   rejection that runs before every other check and that `--allow-external-jsonl` cannot open
   (`sync/path.rs:137-175`, called first at `:211`, `:450`, `:598`), atomic
   temp-file+rename+`chmod 0600` export, and the `.git`-hash-comparing e2e suite. Born from a
   real incident, mechanically enforced, and it caught a bug it wasn't written for (§1.2).
   The best-designed part of the project.
2. **Version-gated open.** Skipping DDL when `user_version >= 1` (`sqlite.rs:105-109`) makes
   read commands lock-free against a concurrent writer, with a dedicated regression test at
   `:4618-4635`. Clever, and the reasoning is written down.
3. **`BEGIN IMMEDIATE` for all `mutate()` writes** (`sqlite.rs:152-154`) — the correct choice
   for multi-process writers, predating the norms research that recommends it.
4. **The atomic claim guard** (`sqlite.rs:400-426`) re-reads `assignee` *inside* the IMMEDIATE
   transaction to close a genuine two-agent TOCTOU — the same primitive
   `web-beads-upstream.md` says upstream converged on.
5. **`empty_to_none` at one boundary** (`sqlite.rs:3066-3072`) — the right way to absorb a
   foreign schema convention.
6. **`should_auto_import`'s exhaustive match with no `_` arm** (`main.rs:160-210`) — new
   `Commands` variants force a compile error at the policy decision point.
7. **`obr schema` generated from live types by schemars** — cannot drift.
8. **Recursive CTEs using `UNION` (not `UNION ALL`) as an explicit cycle-termination
   guarantee**, reasoning at `sqlite.rs:1166-1168`; and `detect_all_cycles` (`:3517-3595`)
   using an explicit-stack iterative DFS to avoid stack overflow.
9. **Non-invasiveness.** No daemon, no hooks, no automatic git. Given
   `web-beads-upstream.md`'s field reports that the daemon, auto-sync and global
   `~/.beads/default.db` fallback were upstream's worst pain points, the exclusions *are* the
   product.
10. **`obr config list`'s `Computed values:` section** — the one place the tool reports what
    it actually decided rather than what you typed.

---

## 10. Ranked recommendations

**P0 — data integrity**

1. Replace `INSERT OR REPLACE` in `upsert_issue_for_import` (`sqlite.rs:3687`) with
   `INSERT … ON CONFLICT(id) DO UPDATE SET …`. Stops §1.1. Add a regression test asserting
   `events` count is preserved across an import that modifies an existing issue.
2. Make `run_auto_import` (`main.rs:219`) and `run_auto_flush` (`main.rs:261`) use
   `discover_beads_dir_with_cli`, and pass the already-resolved `ConfigPaths.jsonl_path` into
   `auto_flush` instead of letting it re-derive one (`sync/mod.rs:1929-1938`). Stops §1.2.
   Add an e2e test that mutates through `--db` from a foreign directory and asserts the
   foreign workspace's export file is byte-unchanged.
3. Promote auto-flush failure from `debug!` to `warn!` on stderr (`main.rs:294-296`), or make
   it non-zero exit. Silent non-persistence at exit 0 is indefensible for this tool.
4. Chunk `clear_dirty_issues`/`clear_dirty_flags` at `SQLITE_VAR_LIMIT`, hoist that constant
   to module scope (redeclared 6x), and wrap `finalize_export` and import Phase 3 in
   transactions.

**P1 — contract**

5. Publish the exit-code table in `--help`, README and the skill file; remap clap usage errors
   off 2; split `NOTHING_TO_DO` off 3 (or make `close` idempotent-succeeding to match
   `label add`/`dep add`).
6. Make `--robot` a `global = true` alias for `--json`, delete the 10 per-command bools and
   the four dispatch mechanisms, and pass `cli.json || cli.robot` to `handle_error`.
7. Handle `BrokenPipe` (or use `-Zon-broken-pipe=kill`, since the toolchain is already
   nightly) and route the TOON path through the same IO-tolerant writer as JSON.
8. Give `BeadsError` typed variants for the sync-safety cases that already have `ErrorCode`s
   — `ConflictMarkers` (with `{file, line, marker_type, branch}` in `context`),
   `PathTraversal`, `ConfigNotFound`, `ConfigParseError` — and pare back the 59
   `BeadsError::Config(String)` sites.

**P2 — subtraction**

9. Delete the 7 dead `StructuredError` constructors and `find_similar_ids`, **or** wire
   `issue_not_found` into the resolver and honour the norms warning about fuzzy suggestions
   for agents. Delete the 4 unreachable `generate_hint` arms and the 3 dead `BeadsError`
   methods (removing the misleading exit-code doc with them).
10. Delete `config/routing.rs`'s town-routing (8 unused public items),
    `src/format/{context,theme,rich,syntax,markdown}.rs`, the dead output components, the 14
    dead storage methods and the 3 duplicate method pairs.
11. Collapse the four "valid status/type" lists into one serving CLI parse, hint generation,
    schema and importer.

**P3 — structure**

12. Introduce `const ISSUE_COLUMNS: &str` and switch `issue_from_row` to named
    `row.get("col")`. Closes bead `beads_rust-2ckc` and removes 11 duplicated projections.
13. Split `storage/sqlite.rs` so `conn` is private to a small module and the `mutate()`
    protocol becomes compiler-enforced rather than conventional.
14. Break the `config` <-> `sync` cycle (move `--no-db` bootstrapping out of
    `open_storage_*`) and stop `storage` from returning `crate::format` DTOs.
15. Replace `ConfigLayer`'s two string maps with a typed, `deny_unknown_fields` config
    validated at load; keep `config list`'s computed section as the diagnostic.
16. Split `OutputMode` into orthogonal `Encoding` (text/json/toon/csv) x `Presentation`
    (rich/plain/quiet) so `--format csv --quiet` cannot silently emit nothing and `--quiet`
    never suppresses *data*.
17. Add `--fields` to JSON/TOON, ship a smaller default field set with `--full`, and add
    Examples to every subcommand's `--help` backed by `trycmd`.
