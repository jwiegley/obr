# Subsystem map: `cli` (argument parsing, dispatch, config, logging)

Repo: `/Users/johnw/src/obr` — crate `beads_rust` v0.1.14, **binary name `obr`** (`Cargo.toml:12-14`),
clap `#[command(name = "obr")]` (`src/cli/mod.rs:652`). Note that `CLAUDE.md`, `AGENTS.md`, help
text and installation instructions still say `br` — see Risks.

## 1. File inventory & sizes

| Path | Lines | Role |
|---|---:|---|
| `src/main.rs` | 433 | Binary entry: parse → auto-import → dispatch → auto-flush → error exit |
| `src/lib.rs` | 74 | Crate root; `pub mod cli/config/error/format/logging/model/output/storage/sync/util/validation`; `run()` is a no-op stub (`src/lib.rs:44-48`) |
| `src/cli/mod.rs` | 2392 | Clap derive types (`Cli`, `Commands`, all `*Args`), dynamic completion machinery, output-format resolution |
| `src/cli/commands/mod.rs` | 39 | Module list; `upgrade` gated on `#[cfg(feature = "self_update")]` (`:38-39`) |
| `src/cli/commands/*.rs` | ~24k | 38 command modules (see §5) |
| `src/config/mod.rs` | 2205 (code 1-1195, tests 1196-2205) | Path discovery, layered config, storage opening |
| `src/config/routing.rs` | 510 (code 1-328) | `routes.jsonl` / `redirect` / town-root cross-project resolution |
| `src/logging.rs` | 166 | `tracing-subscriber` init |
| `src/output/context.rs` | — | `OutputContext` / `OutputMode` (Rich/Plain/Json/Toon/Quiet) |

Largest command modules: `sync.rs` 1425, `dep.rs` 1318, `graph.rs` 1136, `agents.rs` 1054,
`create.rs` 1049, `config.rs` 1030, `doctor.rs` 993, `stats.rs` 943, `query.rs` 907.

## 2. Argument parsing approach

- **clap 4.5 derive**, features `["derive", "env", "unstable-ext"]`; `clap_complete` 4.5 with
  `["unstable-dynamic"]` (`Cargo.toml:18-19`). `unstable-ext` is required for the
  `#[arg(add = ArgValueCompleter::new(..))]` attribute used throughout.
- Single `Cli` struct (`src/cli/mod.rs:651-705`) with `#[command(subcommand)] pub command: Commands`
  plus 12 `global = true` flags. Every sub-`Args` struct is a plain `#[derive(Args)]` struct; there
  is no trait-object/registry indirection — dispatch is one big `match` in `main`.
- **Dynamic shell completion** is wired at the very first line of `main`:
  `CompleteEnv::with_factory(Cli::command).complete();` (`src/main.rs:16`). This intercepts
  `COMPLETE=<shell> obr …` invocations and exits before `Cli::parse()`.
- `br completions <shell>` (`src/cli/commands/completions.rs:31-70`) does **not** emit a static
  completion script; it emits the *registration stub* via
  `clap_complete::env::Shells::builtins()` + `write_registration("COMPLETE", …)`
  (`completions.rs:53-69`). Shells: bash, zsh, fish, powershell (alias `pwsh`), elvish
  (`src/cli/mod.rs:906-920`).

### 2.1 Completion candidate providers (`src/cli/mod.rs:20-648`)

Two lazily-built `OnceLock` indices back all dynamic completions:

- `COMPLETION_INDEX` (`:68`) built by `build_completion_index()` (`:202-268`): discovers the beads
  dir, resolves paths, and **streams the JSONL export file line-by-line** with serde
  (`CompletionIssue` at `:37-51`), collecting issue ids/titles/statuses plus sorted unique labels,
  assignees, owners, and issue types. It never opens SQLite. Malformed lines are skipped (`:228-231`);
  a read error aborts the scan (`:221-223`).
- `CONFIG_INDEX` (`:69`) built by `build_config_index()` (`:270-309`): unions key names from the
  default layer, legacy-user, user, env, project layers, and the DB config table, and harvests saved
  query names from DB keys prefixed `saved_query:` (`SAVED_QUERY_PREFIX` at `:187`). This one *does*
  open storage (`config::open_storage_with_cli`, `:287-288`).

Static candidate tables with help text: `STATUS_CANDIDATES` (`:71`), `STATUS_WITH_ALL_CANDIDATES`
(`:81`), `ISSUE_TYPE_CANDIDATES` (`:92`), `PRIORITY_CANDIDATES` (`:102`),
`PRIORITY_NUMERIC_CANDIDATES` (`:115`), `DEP_TYPE_CANDIDATES` (`:123`, 11 dependency types),
`SORT_KEY_CANDIDATES` (`:137`), `DEP_TREE_FORMAT_CANDIDATES` (`:146`), `CSV_FIELD_CANDIDATES`
(`:149`, 15 fields), `EXPORT_ERROR_POLICY_CANDIDATES` (`:167`), `ORPHAN_MODE_CANDIDATES` (`:180`).

Notable completers:
- `issue_id_completer` / `open_issue_id_completer` / `closed_issue_id_completer` (`:409-419`) filter
  by `IssueCompletionFilter::{Any,Open,Closed}` (`:20-35`), rendering help as
  `"{status} | {truncated title(60)}"` (`:439-441`).
- Delimited (comma-separated) variants via `split_delimited_prefix` (`:353-366`) +
  `CompletionCandidate::add_prefix`, used for `--labels`, `--status`, `--type`, `--priority`,
  `--fields`.
- `deps_completer` (`:566-584`) is two-level: first completes `type:` from `DEP_TYPE_CANDIDATES`
  with a `":"` suffix, then completes issue ids after the colon.
- `config_key_assignment_completer` (`:607-623`) emits both `key` and `key=` forms and bails once
  the current word already contains `=`.

## 3. Global flags (`src/cli/mod.rs:655-705`)

All are `global = true`, so they may appear before or after the subcommand.

| Flag | Type | Line | Effect |
|---|---|---:|---|
| `--db <PATH>` | `Option<PathBuf>` | 658-660 | Explicit DB path; also used to *derive* the `.beads` dir |
| `--actor <NAME>` | `Option<String>` | 662-664 | Audit-trail actor; becomes startup config key `actor` |
| `--json` | bool | 666-668 | JSON output mode |
| `--no-daemon` | bool | 670-672 | **No-op** ("effectively no-op in br v1") |
| `--no-auto-flush` | bool | 674-676 | Skip post-command JSONL export |
| `--no-auto-import` | bool | 678-680 | Skip pre-command JSONL import; turns staleness into a hard error |
| `--allow-stale` | bool | 682-684 | Tolerate JSONL-newer-than-DB without importing |
| `--lock-timeout <MS>` | `Option<u64>` | 686-688 | SQLite busy timeout (default 30000, `src/config/mod.rs:313,382`) |
| `--no-db` | bool | 690-692 | JSONL-only mode: in-memory SQLite seeded from JSONL |
| `-v/--verbose` | `u8` count | 694-696 | Log verbosity |
| `-q/--quiet` | bool | 698-700 | Quiet output mode |
| `--no-color` | bool | 702-704 | Disable ANSI |

There is **no global `--robot`**. `--robot` is a *per-command* flag ("Machine-readable output (alias
for --json)") declared on 10 arg structs: `DeferArgs` (`:1859`), `UndeferArgs` (`:1871`),
`ReadyArgs` (`:1936`), `BlockedArgs` (`:1977`), `CloseArgs` (`:2005`), `ReopenArgs` (`:2021`),
`SyncArgs` (`:2104`), `StatsArgs` (`:2195`), `OrphansArgs` (`:2278`), `ChangelogArgs` (`:2298`).

Per-command `--format` (`text|json|csv|toon` or `text|json|toon`) exists on `list`, `show`, `ready`,
`blocked`, `stats`, `schema`, `dep list`. Enums `OutputFormat` (`:1201-1212`) and
`OutputFormatBasic` (`:1245-1254`).

### 3.1 Output-format precedence

`resolve_output_format(requested, json, robot)` (`src/cli/mod.rs:1266-1280`):
`--json`/`--robot` → `Json`; else `--format`; else `OutputFormat::from_env()`; else `Text`.
`from_env` (`:1218-1231`) reads `BR_OUTPUT_FORMAT` then `TOON_DEFAULT_FORMAT`.
`resolve_output_format_basic` (`:1284-1294`) additionally downgrades `Csv → Text`.

`OutputContext::detect_mode` for the *global* context (`src/output/context.rs:104-117`):
`--json` → Json; `--quiet` → Quiet; `--no-color` or `NO_COLOR` → Plain; non-TTY stdout → Plain;
else Rich. Console and Theme are `OnceLock`-lazy so JSON/Quiet paths pay nothing
(`context.rs:14-23, 119-127`).

Commands that support `--format`/`--robot` build a **second, local** `OutputContext` via
`OutputContext::from_output_format(format, quiet, !use_color)` (`context.rs:80-102`), e.g.
`ready.rs:40-42`, `list.rs:72-74`, `show.rs:41-43`, `blocked.rs:38-41`, `stats.rs:40-42`,
`search.rs`, `schema.rs:58-61`. The outer context is only consulted for `is_json()`.

## 4. Entry point & process lifecycle (`src/main.rs`)

```
1.  CompleteEnv::with_factory(Cli::command).complete();          main.rs:16
2.  let cli = Cli::parse();                                      main.rs:18
3.  let output_ctx = OutputContext::from_args(&cli);             main.rs:19
4.  init_logging(cli.verbose, cli.quiet, None)  (failure is non-fatal)  main.rs:22-25
5.  let overrides = build_cli_overrides(&cli);                   main.rs:27  (327-341)
6.  let is_mutating = is_mutating_command(&cli.command);         main.rs:30  (139-158)
7.  if should_auto_import(&cmd) && !cli.no_db { run_auto_import(...) }  main.rs:32-36 (160-253)
8.  let result = match cli.command { ...39 arms... };            main.rs:38-125
9.  if let Err(e) = result { handle_error(&e, cli.json) }  // diverges  main.rs:128-130 (304-325)
10. if is_mutating && !cli.no_auto_flush && !cli.no_db { run_auto_flush(&overrides) }  main.rs:133-135 (259-298)
```

- `is_mutating_command` (`:139-158`): Create, Update, Delete, Close, Reopen, Q, Dep, Label, Comments,
  Defer, Undefer are always mutating; `Epic close-eligible` is mutating only when `!dry_run`
  (`:152-155`); everything else `false`.
- `should_auto_import` (`:160-210`) is an **exhaustive** match (no `_` arm), so adding a `Commands`
  variant is a compile error here — a deliberate safety net. Excluded from auto-import: `init`,
  `sync`, `doctor`, `info`, `schema`, `where`, `version`, `completions`, `audit`, `config`,
  `history`, `agents`, `upgrade`.
- `run_auto_import` (`:213-253`): discovers `.beads` from `"."`, returns `Ok(())` on
  `BeadsError::NotInitialized` (so `br init` works), opens storage, bails if `no_db` (`:231-233`),
  reads `issue_prefix` from the DB config table, then calls
  `sync::auto_import_if_stale(storage, beads_dir, jsonl_path, prefix, allow_stale, no_auto_import)`.
  In `sync/mod.rs:1795-1847`: if JSONL is not newer → no-op; if `--allow-stale` → warn + skip; if
  `--no-auto-import` → hard `BeadsError::Config` telling the user to run `br sync --import-only`;
  otherwise import with `skip_prefix_validation: false` (strict prefix check).
- `run_auto_flush` (`:259-298`): best-effort and **fully silent on failure** — discovery errors,
  storage-open errors and flush errors are only `debug!`-logged (`:264-269, 276-279, 293-296`). It
  uses `config::open_storage` (not `open_storage_with_cli`), i.e. a *fresh third* connection that
  ignores `--no-db`. `sync::auto_flush` (`sync/mod.rs:1918-1965`) early-returns when
  `get_dirty_issue_count() == 0`, re-resolves the export path from `metadata.json`.
- `handle_error` (`:304-325`): builds `StructuredError::from_error`, exits with
  `structured.code.exit_code()`. Chooses JSON **when `--json` OR stdout is not a TTY** (`:309`),
  writing the pretty JSON envelope to **stderr**; otherwise a human-readable colored string when
  stderr is a TTY.
- `build_cli_overrides` (`:327-341`) maps CLI → `config::CliOverrides`. Note `identity: None`
  (`:331`) — there is no `--identity` flag; and `display_color` is `Some(false)` only when
  `--no-color` is passed, otherwise `None` (`:333`).

Cost note: a mutating command opens SQLite **three times** — auto-import, the command itself,
auto-flush. There is a regression test named `tests/repro_auto_flush_inefficiency.rs`.

## 5. Dispatch structure and command inventory

`Commands` (`src/cli/mod.rs:707-891`) has **39 variants**:

`Init Create Q List Show Update Close Reopen Delete Ready Blocked Search Dep Label Epic Comments
Stats Status Count Stale Lint Defer Undefer Config Sync Doctor Info Schema Where Version Upgrade
Completions Audit History Orphans Changelog Query Graph Agents`

Aliases: `comments`→`comment` (`:776`), `completions`→`completion` (`:864`),
`status`→duplicate variant of `stats` (`:782-783`, dispatched together at `main.rs:93-95`),
`dep remove`→`rm` (`:1442`), `config delete`→`unset` (`:2139`),
`update --description`→`--body` (`:1030`), `update --acceptance-criteria`→`--acceptance` (`:1038`).

`Upgrade` is `#[cfg(feature = "self_update")]` (`:860-861`); `self_update` is in `default`
(`Cargo.toml:100-102`).

### 5.1 One line per command module (`src/cli/commands/`)

| Module | Lines | Summary |
|---|---:|---|
| `agents.rs` | 1054 | Detect/add/update/remove the beads workflow "blurb" in `AGENTS.md`/`CLAUDE.md` (markers `<!-- br-agent-instructions-v1 -->`, `BLURB_VERSION = 1`, `:16-22`); searches up to 3 parent dirs (`:225`, called at `:365`); pure filesystem, never touches storage. |
| `audit.rs` | 712 | `record`/`label` append to an append-only interactions JSONL; `log`/`summary` read the DB `events` table (`:140-169`). Loads config with `storage = None` (`:129`). |
| `blocked.rs` | 697 | Lists blocked issues from `blocked_issues_cache`, then merges **external-project blockers** via `external_project_db_paths` (`:38, 65-95`); filters by type/priority/labels, sorts, truncates. |
| `changelog.rs` | 478 | Groups closed issues by type since `--since` / `--since-tag` / `--since-commit` (git-derived dates) into release notes. |
| `close.rs` | 678 | Closes issues (falls back to `last-touched` when no IDs, `:135-140`); `--force` past blockers, `--suggest-next` returns newly unblocked issues, `--session`; writes `last-touched` (`:246`). |
| `comments.rs` | 513 | `comments add|list` (+ bare `comments <id>` list). Author chain: `--author` → config actor → `BD_ACTOR` → `BEADS_ACTOR` → `git config user.name` → `USER` → `"unknown"` (`:318-349`). |
| `completions.rs` | 197 | Emits the dynamic-completion registration stub per shell; `print_install_instructions` (`:94`) is **dead code** (never called). |
| `config.rs` | 1030 | `list/get/set/delete/edit/path`; rebuilds the 7 layers with provenance labels (`ConfigSource`, `:29-63`; `build_layers`, `:110-168`) so `config list` can show where each value came from. |
| `count.rs` | 368 | Counts issues with `--by status|priority|type|assignee|label` (plus `--by-*` boolean aliases) and list-style filters. |
| `create.rs` | 1049 | Creates an issue (ID generation, validation, deps/parent/labels); `--dry-run`, `--silent` (ID only), `--file` bulk markdown import (`execute_import`, `:388`). Does **not** write `last-touched`. |
| `defer.rs` | 663 | `defer --until <flexible timestamp>` / `undefer`; both write `last-touched` (`:125, :246`). |
| `delete.rs` | 611 | Tombstones issues; `--from-file`, `--cascade`, `--force` (conflicts with cascade), `--hard` (prune from JSONL), `--dry-run`; refuses when dependents exist without `--force`/`--cascade` (`:106-110`). |
| `dep.rs` | 1318 | `add/remove(rm)/list/tree/cycles`; `tree --format text|mermaid`; resolves external-project dependency statuses for `list`/`tree` (`:43`). |
| `doctor.rs` | 993 | Read-only diagnostics: merge artifacts, JSONL parse, sync path allowlist, conflict markers, required schema tables, `PRAGMA integrity_check`, DB-vs-JSONL count, sync metadata. Opens SQLite `SQLITE_OPEN_READ_ONLY` (`:914`). **`std::process::exit(1)`** on any error check (`:853, 870, 947`). |
| `epic.rs` | 550 | `epic status [--eligible-only]`, `epic close-eligible [--dry-run]` (closes epics whose children are all closed). |
| `graph.rs` | 1136 | `br graph <id>` DFS over *dependents* (reverse deps); `--all` renders connected components of open/in_progress/blocked; `--compact` one line per issue. |
| `history.rs` | 375 | Manages `.beads/.br_history` backups: `list` (default), `diff <file>`, `restore <file> [-f]`, `prune --keep N --older-than D`. |
| `info.rs` | 367 | Workspace metadata: db path/size, jsonl path/size, issue count, DB config map, optional `--schema`; `--whats-new`/`--thanks` are stubs (`:71-80`). Hardcodes `mode: "direct"`, `daemon_connected: false` (`:109-112`). |
| `init.rs` | 423 | Creates `.beads/`, opens/creates `beads.db` (honoring `BEADS_CACHE_DIR` via `util::db_path`), writes `metadata.json` (`database: beads.db`, `jsonl_export: issues.org`), a commented `config.yaml` template, and `.gitignore`. `--force` overwrites; `--backend` is accepted and ignored (`src/cli/mod.rs:719-721`, `main.rs:42`). |
| `label.rs` | 674 | `add/remove/list/list-all/rename` labels. |
| `lint.rs` | 440 | Warns about missing recommended template sections per issue type; `--type`, `--status` (default open, `all` supported). |
| `list.rs` | 449 | Primary discovery command; ~25 filters, client-side post-filters for some, `--format text|json|csv|toon`, `--fields` for CSV, `--long`, `--pretty`, `--wrap`. |
| `orphans.rs` | 407 | Scans `git log` for issue-ID references and reports issues still open; degrades to an *empty result, exit 0* when git/DB/beads dir is missing (`:49-58, :64-73`). |
| `q.rs` | 165 | Quick capture: joins positional words into a title, generates an ID, prints it. |
| `query.rs` | 907 | Saved queries stored in the DB config table under `saved_query:<name>`; `save/run/list/delete`. `run` merges CLI filters over the saved ones. Names may not contain `:` or `/` (`:255-260`); `save` refuses to overwrite (`:272-277`). |
| `ready.rs` | 240 | Unblocked/undeferred work list; sort policies `hybrid`(default)`/priority/oldest`; fetches all candidates, removes issues blocked by *external* projects, then applies `--limit` (`:70-80`). |
| `reopen.rs` | 240 | Reopens closed issues (last-touched fallback), optional `--reason` stored as a comment; writes `last-touched` (`:136`). |
| `schema.rs` | 197 | Emits JSON Schema (via `schemars`) for Issue, IssueWithCounts, IssueDetails, ReadyIssue, StaleIssue, BlockedIssue, TreeNode, Statistics, Error, or `all` (default). Output is always machine-readable; text mode prints pretty JSON. |
| `search.rs` | 606 | LIKE search over title/description/id, flattening all `ListArgs` filters (`SearchArgs` at `src/cli/mod.rs:1408-1415`). |
| `show.rs` | 493 | Full issue details incl. relations/comments/events (limit 10 events, `:55`); accepts multiple IDs, prefix/hash-abbreviated IDs via `IdResolver`; last-touched fallback. |
| `stale.rs` | 258 | Issues not updated in `--days` (default 30), default statuses open+in_progress, sorted oldest-first. |
| `stats.rs` | 943 | Counts by status plus optional `--by-type/-priority/-assignee/-label` breakdowns; computes git-based recent activity by default (`--no-activity` to skip, `--activity-hours`, default 24). |
| `sync.rs` | 1425 | `--status` (read-only) / `--flush-only` (export) / `--merge` (3-way with `base_snapshot.jsonl`) / default `--import-only`; `--force`, `--allow-external-jsonl`, `--manifest`, `--error-policy`, `--orphans`, `--rename-prefix`. |
| `update.rs` | 527 | Field updates, label add/remove/set, reparenting, `--claim` (atomic assignee+in_progress with `claim-exclusive` guard, `:53`), blocked-claim guard unless `--force` (`:75-90`); writes `last-touched` (`:125`). |
| `upgrade.rs` | 491 | `self_update`-backed self-upgrade with `--check`, `--dry-run`, `--force`, `--version`; rustls + signature verification (`Cargo.toml:70`, `src/release_public_key.bin`). |
| `version.rs` | 379 | Version/build/commit/branch/rustc/target from `vergen-gix` `option_env!`s; `--short`, `--check` (exit 0 up-to-date / 1 update available). |
| `where.rs` | 211 | Prints the resolved `.beads` dir (after following `redirect`), original dir if redirected, detected prefix, db path, jsonl path. |

### 5.2 Sub-subcommand inventory

- `dep`: `add`, `remove`(`rm`), `list`, `tree`, `cycles` (`src/cli/mod.rs:1437-1450`)
- `label`: `add`, `remove`, `list`, `list-all`, `rename` (`:1564-1577`)
- `epic`: `status`, `close-eligible` (`:1453-1460`)
- `comments`: `add`, `list` (optional; bare form lists by `--id`) (`:1632-1636`)
- `config`: `list`, `get`, `set`, `delete`(`unset`), `edit`, `path` (`:2107-2151`)
- `audit`: `record`, `label`, `log`, `summary` (`:1671-1681`)
- `query`: `save`, `run`, `list`, `delete` (`:2302-2312`)
- `history`: `list`(default), `diff`, `restore`, `prune` (`:2204-2230`)

Dependency types offered: `blocks` (default), `parent-child`, `conditional-blocks`, `waits-for`,
`related`, `discovered-from`, `replies-to`, `relates-to`, `duplicates`, `supersedes`, `caused-by`
(`src/cli/mod.rs:123-135`).

## 6. How commands share storage handles

There is **no shared handle**. Each command re-derives everything. The canonical prologue is:

```rust
let beads_dir   = config::discover_beads_dir_with_cli(cli)?;          // or discover_beads_dir(None)
let mut storage_ctx = config::open_storage_with_cli(&beads_dir, cli)?; // OpenStorageResult
let config_layer = config::load_config(&beads_dir, Some(&storage_ctx.storage), cli)?;
let actor  = config::resolve_actor(&config_layer);
let id_cfg = config::id_config_from_layer(&config_layer);
let resolver = IdResolver::new(ResolverConfig::with_prefix(id_cfg.prefix));
let all_ids  = storage_ctx.storage.get_all_ids()?;
…
storage_ctx.flush_no_db_if_dirty()?;   // mutating commands only
```

Examples: `close.rs:132-141`, `dep.rs:30-42`, `label.rs:26-34`, `comments.rs:26-34`,
`create.rs:47-58`, `update.rs:44-51`, `defer.rs:52-60`, `q.rs:36-40`, `graph.rs:61-67`,
`show.rs:22-43`, `list.rs:31-34`, `ready.rs:29-33`, `stats.rs:34-37`.

`OpenStorageResult` (`src/config/mod.rs:320-361`) is the shared bundle: `{ storage, paths, no_db }`.
Its `flush_no_db_if_dirty()` (`:333-360`) is a no-op unless `no_db` is set and there are dirty
issues; then it exports to JSONL and calls `finalize_export`. Callers that mutate always invoke it:
`close.rs:336`, `comments.rs:65`, `create.rs:95,606`, `defer.rs:166,294`, `delete.rs:202,228`,
`dep.rs:72`, `epic.rs:155`, `label.rs:48`, `q.rs:140`, `reopen.rs:182`, `update.rs:145`.

**Two discovery entry points**, and commands are inconsistent about which they use:
- `discover_beads_dir_with_cli(cli)` (`config/mod.rs:255-262`) — honors `--db` by walking the path
  for a `.beads` component (`derive_beads_dir_from_db_path`, `:267-295`), erroring if none exists.
  Used by the majority of commands.
- `discover_beads_dir(None)` / `discover_beads_dir(Some("."))` — ignores `--db` for discovery.
  Used by `blocked.rs:33`, `doctor.rs:840`, `orphans.rs:49`, `config.rs:92,105,264,418,585`,
  `where.rs:33`, and both `main.rs:219` and `main.rs:261`.

## 7. Configuration subsystem (`src/config/mod.rs`)

### 7.1 `.beads` directory discovery (`:204-242`)

1. `BEADS_DIR` env var, if set/non-blank and a directory → `routing::follow_redirects(path, 10)`.
2. Otherwise walk **up** from `start` (or CWD) looking for a `.beads` directory; first hit is
   passed through `follow_redirects`.
3. Else `Err(BeadsError::NotInitialized)`.

### 7.2 File-name resolution

Constants (`:30-48`): `DEFAULT_DB_FILENAME = "beads.db"`, `DEFAULT_JSONL_FILENAME = "issues.org"`,
`LEGACY_JSONL_FILENAME = "issues.jsonl"`, `OLDER_LEGACY_JSONL_FILENAME = "beads.jsonl"`,
`EXCLUDED_JSONL_FILES = [deletions.jsonl, interactions.jsonl, beads.base.jsonl, beads.left.jsonl,
beads.right.jsonl, sync_base.jsonl]`.

`Metadata` (`:51-95`) is `.beads/metadata.json` (`database`, `jsonl_export`, `backend`,
`deletions_retention_days`), with blank fields coerced back to defaults (`:86-91`).

`resolve_db_path` (`:527-545`): `--db` override wins; else `metadata.database` (absolute as-is,
relative joined to `util::resolve_cache_dir(beads_dir)` so `BEADS_CACHE_DIR` can move the DB off a
slow network mount).

`resolve_jsonl_path` (`:547-590`), 5 priorities:
1. `BEADS_JSONL` env var (absolute, unconditional).
2. If `--db` was given, sibling `<db_parent>/issues.org`.
3. `metadata.jsonl_export` when it differs from the default and isn't in `EXCLUDED_JSONL_FILES`.
4. `discover_jsonl(beads_dir)` (`:107-128`): `issues.org` → `issues.jsonl` → `beads.jsonl`.
5. `beads_dir/issues.org`.

### 7.3 Config file locations

- Project: `<.beads>/config.yaml` (`load_project_config`, `:746-748`).
- User: `$HOME/.config/beads/config.yaml`, falling back to `$HOME/.config/bd/config.yaml`
  (`load_user_config`, `:755-766`).
- Legacy user: `$HOME/.beads/config.yaml` (`load_legacy_user_config`, `:773-779`).
- DB: the SQLite `config` table (`ConfigLayer::from_db`, `:672-682`).

### 7.4 Layers and precedence

`ConfigLayer` (`:593-683`) splits keys into two maps: `startup` and `runtime`.
`merge_layers` folds lowest→highest (`:612-618`).

`load_config(beads_dir, storage, cli)` (`:815-840`) merges, low→high:
```
defaults → DB → legacy user → user → project → env → CLI
```
matching the module doc-comment at `:3-10`. `load_startup_config` (`:786-798`) omits both defaults
and DB (used before storage exists): `legacy user → user → project → env`.

`default_config_layer()` (`:802-808`) contributes exactly one key: `issue_prefix = "bd"`.

`is_startup_key` (`:1030-1063`) — startup-only keys can never come from the DB:
any key prefixed `git.`, `routing.`, `validation.`, `directory.`, `sync.`, `external-projects.`,
plus the exact keys `no-db`, `no-daemon`, `no-auto-flush`, `no-auto-import`, `json`, `db`, `actor`,
`identity`, `flush-debounce`, `lock-timeout`, `remote-sync-interval`, `no-git-ops`, `no-push`,
`sync-branch`, `sync.branch`, `external-projects`, `hierarchy.max-depth`.
`ConfigLayer::from_db` silently *drops* any startup key found in the DB (`:677-678`).

YAML is flattened dot-wise (`flatten_yaml`, `:1153-1182`); sequences become comma-joined strings
(`:1168-1175`); nulls/mappings inside sequences are dropped (`yaml_scalar_to_string`, `:1184-1194`).

Key lookup is **asymmetric**: `get_startup_value` (`:1094-1106`) normalizes both sides via
`normalize_key` (lowercase, `_`→`-`), but `get_value` (`:1108-1115`) does exact `HashMap::get` on
each candidate — so runtime lookups rely on callers passing every spelling explicitly, e.g.
`&["issue_prefix", "issue-prefix", "prefix"]` (`:845`), `&["display.color", "display-color",
"display_color"]` (`:888`).

### 7.5 Environment variables

`ConfigLayer::from_env` (`:637-665`):
- Any `BD_<NAME>` var → key `<name>` lowercased, inserted under **three** spellings via
  `env_key_variants` (`:1077-1084`): raw, `_`→`.`, `_`→`-`. So `BD_DISPLAY_COLOR` sets
  `display_color`, `display.color`, and `display-color`.
- `BEADS_FLUSH_DEBOUNCE` → `flush-debounce`
- `BEADS_IDENTITY` → `identity`
- `BEADS_REMOTE_SYNC_INTERVAL` → `remote-sync-interval`
- `BEADS_AUTO_START_DAEMON` → inverted into `no-daemon` (`:658-662`)

Other env vars read directly elsewhere:
`BEADS_DIR` (`:216`), `BEADS_JSONL` (`:553`), `BEADS_CACHE_DIR` (`src/util/mod.rs:37-53`),
`HOME` (`:170,186,756,774`), `USER` (`:1007`), `NO_COLOR` (`:903`, `output/context.rs:64,88,112`),
`BR_OUTPUT_FORMAT` / `TOON_DEFAULT_FORMAT` (`src/cli/mod.rs:1220,1225`),
`TOON_STATS` (`src/output/context.rs:272`), `RUST_LOG` (`src/logging.rs:50`),
`EDITOR`/`VISUAL` (`src/cli/commands/config.rs`), `BD_ACTOR`/`BEADS_ACTOR`
(`src/cli/commands/comments.rs:329,334`), `COLUMNS`/`LINES` (format module).

### 7.6 Derived values

- `id_config_from_layer` (`:844-861`): `issue_prefix` (default `"bd"`), `min_hash_length` 3,
  `max_hash_length` 8, `max_collision_prob` 0.25.
- `default_priority_from_layer` (`:868-871`) default MEDIUM (P2); errors on invalid value.
- `default_issue_type_from_layer` (`:878-881`) default Task; custom types allowed.
- `should_use_color` (`:899-907`): config `display.color` → `NO_COLOR` → `stdout().is_terminal()`.
- `resolve_actor` (`:1004-1013`): config `actor` → `$USER` → `"unknown"`.
- `claim_exclusive_from_layer` (`:1020-1023`): reads `claim-exclusive` / `claim.exclusive`
  (accepts `"true"` case-insensitive or `"1"`) — note it does **not** accept `yes/on` unlike
  `parse_bool` (`:1086-1092`).

### 7.7 `--no-db` mode (`open_storage_with_cli`, `:368-419`)

When `no-db` resolves true (from CLI or startup layer, `no_db_from_layer` `:421-423`):
open an **in-memory** SQLite (`:387`), resolve a prefix via `resolve_no_db_prefix` (`:425-449`:
project `config.yaml` → single common prefix inferred from the export file → parent dir name →
`"bd"`), set `issue_prefix`, and import the JSONL/Org file into memory (`:391-404`).
`common_prefix_from_jsonl` (`:451-512`) parses `.org` through `sync::org_bridge` and returns `None`
on mixed prefixes, but for `.jsonl` it *errors* on mixed prefixes (`:503-508`) — asymmetric.

## 8. Multi-repo / cross-project support

Two independent mechanisms, only one of which is wired up.

### 8.1 `external_projects` config (live)

`external_projects_from_layer` (`:914-945`) scans both maps for keys prefixed
`external_projects.` / `external-projects.` (case-insensitive), splitting at the first `.`; relative
values resolve against the *project root* (parent of `.beads`).
`external_project_db_paths` (`:952-991`) turns each into a `.beads` dir (appending `.beads` if the
path isn't already one), warns and skips when missing, and resolves each one's DB path.

Consumers: `ready.rs:34` (drops issues blocked by external deps), `blocked.rs:38` (merges external
blockers into the blocked list), `dep.rs:43` (feeds `dep list` and `dep tree`). Backed by
`storage.resolve_external_dependency_statuses(...)` / `storage.external_blockers(...)`.

### 8.2 `routes.jsonl` / town-root routing (`src/config/routing.rs`) — mostly inert

Documented resolution order (`routing.rs:12-19`): extract prefix → local `.beads/routes.jsonl` →
town-root `.beads/routes.jsonl` → resolve path (`"."` means town-level `.beads`) → follow
`.beads/redirect`.

- `RouteEntry { prefix, path }` (`:29-35`); `RoutingResult { beads_dir, is_external, project_path }`
  (`:38-68`).
- `extract_prefix(id)` (`:77-79`) returns everything before the **first** hyphen plus the hyphen,
  delegating to `util::id::split_prefix_remainder` — tests show `"bead-me-up-3e9"` → `"bead-me-up-"`
  and `"no-hyphen-here"` → `"no-hyphen-"` (`:336-351`), i.e. it is really *last*-hyphen semantics via
  the util helper, contradicting the doc comment.
- `find_town_root(start)` (`:85-101`) walks up for `mayor/town.json`.
- `read_redirect` (`:162-191`) reads `.beads/redirect`, resolving relative targets against the
  **parent** of the beads dir; blank file → `None`.
- `follow_redirects(start, max_depth)` (`:200-232`) chases redirects with a visited-list loop guard
  and validates the final directory exists.
- `resolve_route(issue_id, local_beads_dir)` (`:249-281`) and `is_external_id` (`:325-327`) are
  **never called from production code** — only from their own unit tests. The only live consumer of
  this module is `follow_redirects`, called from `discover_beads_dir_with_env` (`config/mod.rs:214,
  220, 233`) and `where.rs:36`. So `routes.jsonl` and town roots are dead weight today; `redirect`
  is live.

## 9. Daemon / watch modes

**Neither exists.** There is no file watcher (no `notify` dependency, no `watch` command) and no
daemon. `--no-daemon` is documented as "effectively no-op in br v1" (`src/cli/mod.rs:670-672`); it
flows into `CliOverrides.no_daemon` → config key `no-daemon` (`config/mod.rs:724-726`) and is never
read back. `BEADS_AUTO_START_DAEMON` is translated into `no-daemon` (`:658-662`) and likewise
ignored. `br info` hardcodes `mode: "direct"`, `daemon_connected: false`,
`daemon_fallback_reason: "no-daemon"`, `daemon_detail: "br runs in direct mode only"`
(`info.rs:109-112`). These exist purely for Go-`bd` CLI/JSON compatibility.

The nearest thing to background behavior is the implicit auto-import/auto-flush pair in `main`
(§4) and the debounce key `flush-debounce`, which is parsed but unused.

## 10. Logging (`src/logging.rs`)

- `init_logging(verbosity, quiet, log_file)` (`:20-47`) builds a `tracing_subscriber::registry` with
  an `EnvFilter` plus a `fmt` layer writing to **stderr**, `with_target(true)`, `with_level(true)`,
  file/line only in debug builds (`:27-28`), ANSI only when stderr is a TTY (`:29`).
- Optional JSON file layer when `log_file` is `Some` (`:35-42`) — `main.rs:22` always passes `None`,
  so there is no CLI flag to enable file logging.
- `resolve_env_filter` (`:49-53`): `RUST_LOG` wins; otherwise `default_filter` (`:69-86`):
  `quiet` → `"error"`; `-v` levels → `beads_rust=info` (release) / `beads_rust=debug` (debug build)
  at 0, `beads_rust=debug` at 1, `beads_rust=debug,rusqlite=debug` at 2, `beads_rust=trace` at ≥3.
- `init_test_logging()` (`:89-99`) is `Once`-guarded with `with_test_writer`.
- Init failure is printed to stderr and **swallowed** — the process continues unlogged
  (`main.rs:22-25`).

## 11. Testing surface

`tests/common/cli.rs` drives the real binary via `assert_cmd` in a `TempDir` workspace
(`BrWorkspace`, `run_br`, `run_br_with_env`, `run_br_with_stdin`), capturing stdout/stderr/duration
into per-test log files. CLI-relevant suites include `e2e_global_flags.rs`,
`e2e_config_precedence.rs`, `e2e_env_overrides.rs`, `e2e_completions.rs`, `e2e_errors.rs`,
`e2e_workspace_commands.rs`, `e2e_workspace_scenarios.rs`, plus `conformance*.rs` which diff `obr`
against the Go `bd`.

---

## 12. Risks, oddities, and inconsistencies

1. **Binary is `obr`, docs say `br`.** `Cargo.toml:13` and `src/cli/mod.rs:652` say `obr`, but
   `CLAUDE.md`, the `sync` long-help (`src/cli/mod.rs:811-841`) and every
   `completions::print_install_instructions` string (`completions.rs:94-170`) say `br`. Users
   copy-pasting the install instructions get a broken completion path.

2. **`--robot` is ignored for `sync --flush-only` and `sync --merge`.** `main.rs:80` passes
   `cli.json` (not `cli.json || args.robot`); `sync::execute` computes
   `use_json = ctx.is_json() || args.robot` at `sync.rs:97` and uses it for `--status` (`:110`) and
   import (`:152`), but passes the raw `json` for flush (`:129`) and merge (`:139`) — into a
   parameter literally named `use_json` (`sync.rs:474, 1105`). `obr sync --flush-only --robot` on a
   TTY emits human output.

3. **`--robot` never reaches error handling.** `handle_error(&e, cli.json)` (`main.rs:129`) only
   knows the global `--json`. A failing `--robot` command on a TTY prints a human error; piping
   rescues it only because of the `!stdout().is_terminal()` fallback (`main.rs:309`).

4. **Several config keys are declared but never consumed.** `no-auto-flush`, `no-auto-import`, and
   `json` are classified as startup keys (`config/mod.rs:1046-1049`) and round-trip through
   `CliOverrides::as_layer`, but `main` reads only `cli.no_auto_flush` (`:133`),
   `cli.no_auto_import` (`:33`) and `cli.json`. Putting `no-auto-flush: true` in
   `.beads/config.yaml` or setting `BD_NO_AUTO_FLUSH=1` has **no effect**. Same for `identity`,
   `flush-debounce`, `remote-sync-interval`, `no-git-ops`, `no-push`, `sync-branch`,
   `hierarchy.max-depth` — parsed, classified, never read.

5. **`no-db` set via config (not CLI) bypasses the auto-flush guard.** `main.rs:133` gates on
   `cli.no_db`, while `open_storage_with_cli` honors `no_db_from_layer` (`config/mod.rs:373`). With
   `no-db: true` in YAML, the command runs against an in-memory DB and flushes JSONL itself
   (`flush_no_db_if_dirty`), and then `run_auto_flush` opens the **on-disk** DB via
   `config::open_storage` (`main.rs:274`, which has no no-db awareness) and would export it over the
   same JSONL if that DB has dirty rows. In practice `auto_flush` early-returns at
   `dirty_count == 0` (`sync/mod.rs:1920-1924`), so the hazard is latent rather than routine — but
   it is a real divergence between two "sources of truth" for `no-db`.

6. **`--db` is honored inconsistently for discovery.** `blocked`, `doctor`, `orphans`, `config`,
   `where`, and both `main` auto-hooks use `discover_beads_dir(None|Some("."))`, ignoring `--db`,
   while ~30 other commands use `discover_beads_dir_with_cli`. `obr blocked --db
   /other/.beads/beads.db` reads the *right* database (the override still reaches
   `ConfigPaths::resolve`) but resolves project config, JSONL path, and external projects against
   the *local* `.beads`.

7. **`routes.jsonl` / town-root routing is dead code.** `resolve_route` and `is_external_id`
   (`routing.rs:249, 325`) have no production callers; the entire "Resolution Order" doc block
   (`routing.rs:12-19`) describes behavior that never runs. `follow_redirects` and `read_redirect`
   *are* live. `completions::print_install_instructions` (`completions.rs:94`) is likewise dead.

8. **`extract_prefix` doc contradicts its own tests.** The doc says "the substring before the first
   hyphen" (`routing.rs:71-75`), but the tests assert `"bead-me-up-3e9"` → `"bead-me-up-"` and
   `"no-hyphen-here"` → `"no-hyphen-"` (`routing.rs:336-351`), i.e. `split_prefix_remainder` splits
   at the *last* hyphen.

9. **Stale comments about the default export filename.** `DEFAULT_JSONL_FILENAME` was changed to
   `issues.org` (`config/mod.rs:33`), but surrounding comments still say `issues.jsonl`
   (`:583` "prefer issues.jsonl, fall back to beads.jsonl", `:588` "Priority 5: Default
   (issues.jsonl)"). `sync`'s long-help also advertises `.beads/issues.jsonl` (`cli/mod.rs:838`).

10. **`config set` and `config delete` are asymmetric.** `set` always writes to
    `.beads/config.yaml` when a beads dir exists, falling back to the user config otherwise
    (`config.rs:418-425`), and never touches the DB. `delete` removes the key from the DB *and*
    project YAML *and* user YAML (`config.rs:577-627`). Its validation message also references a
    non-existent flag syntax: `"Use: --set key=value or --set key value"` (`config.rs:406`).

11. **`create` never records `last-touched`.** Only `close.rs:246`, `reopen.rs:136`,
    `update.rs:125`, and `defer.rs:125,246` write it; `show`, `close`, and `reopen` read it as a
    fallback when no IDs are given. So `obr create "X" && obr show` does not show the new issue.

12. **Runtime-key lookup does not normalize.** `get_value` (`config/mod.rs:1108-1115`) is exact-match
    only. A YAML key spelled `issue-prefix` works only because callers enumerate the variants;
    a novel key like `defaultPriority` or `default-Priority` silently does nothing.

13. **`claim-exclusive` uses a bespoke boolean parser.** `claim_exclusive_from_layer`
    (`config/mod.rs:1020-1023`) accepts only `"true"`/`"1"`, unlike `parse_bool`
    (`:1086-1092`) which also accepts `yes/y/on`. `claim-exclusive` is also *not* in
    `is_startup_key`'s list, yet it is read via `get_startup_value` — so a value set in the DB or
    in YAML lands in `runtime` and will never be found. This looks like a live bug: the key appears
    unreachable in practice.

14. **Three SQLite opens per mutating command** (auto-import, command body, auto-flush) — see §4;
    `tests/repro_auto_flush_inefficiency.rs` exists for exactly this.

15. **Silent auto-flush failures.** Every error path in `run_auto_flush` (`main.rs:264-296`) is
    `debug!`-only, so at default verbosity a failed export after a successful mutation is invisible
    and the process still exits 0.

16. **`doctor` and `orphans` bypass the normal error path.** `doctor` calls `std::process::exit(1)`
    directly at three points (`doctor.rs:853, 870, 947`), never reaching `handle_error`; `orphans`
    swallows missing git/DB/beads-dir into an empty successful result (`orphans.rs:49-73`).

17. **Duplicate `AgentsArgs`.** `crate::cli::AgentsArgs` (`cli/mod.rs:2368-2392`) and
    `commands::agents::AgentsArgs` (`agents.rs:344`) are distinct structs with identical fields,
    manually copied field-by-field in `main.rs:115-123`.

18. **Two `Commands` variants for one command.** `Stats(StatsArgs)` and `Status(StatsArgs)`
    (`cli/mod.rs:780-783`) both dispatch to `commands::stats::execute` (`main.rs:93-95`) and both
    appear in `should_auto_import` (`:173-174`) — an alias implemented as a clone rather than
    `#[command(alias = "status")]`.

19. **Completion index reads the export file, not the DB.** `build_completion_index`
    (`cli/mod.rs:202-268`) parses `paths.jsonl_path` with a JSON-per-line deserializer. Since the
    default export is now `issues.org` (`config/mod.rs:33`), every line will fail
    `serde_json::from_str` and be skipped (`:228-231`), yielding **empty** id/label/assignee
    completions on a default workspace. Static candidates (status/type/priority/dep-type) still
    work. Worth verifying against a real `.beads/issues.org`.

20. **`init --backend` is accepted and discarded.** Declared at `cli/mod.rs:719-721` ("Backend type
    (ignored, always sqlite)") and destructured as `backend: _` at `main.rs:42`.

21. **`--quiet` is not uniformly honored.** `OutputContext` suppresses `print`/`render`, but many
    modules use bare `println!` (agents 40 sites, sync 35, config 29, completions 27, stats 25,
    upgrade 17, delete 16, audit 16 …). Some guard with `ctx.is_quiet()` (e.g. `config.rs:635`,
    `lint.rs:109`), many do not.

22. **`common_prefix_from_jsonl` treats `.org` and `.jsonl` differently on mixed prefixes**
    (`config/mod.rs:451-512`): `.org` returns `Ok(None)` and falls through to the directory-name
    heuristic, `.jsonl` returns a hard `BeadsError::Config`.

23. **`ConfigPaths::user_config_path` / `legacy_user_config_path`** (`config/mod.rs:169-188`) are
    `&self` methods that never touch `self` — they duplicate the free functions
    `load_user_config`/`load_legacy_user_config` path logic, and `config.rs` uses its own third
    copy (`get_user_config_path`).
