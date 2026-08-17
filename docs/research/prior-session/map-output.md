# Subsystem Map: Output / Formatting Layer & Agent-Friendliness (`obr` / beads_rust)

Research notes for the multi-agent deep-research effort. Repo: `/Users/johnw/src/obr` (READ-ONLY).
All paths below are repo-relative unless absolute. Citations are `path:line`.

---

## 0. TL;DR orientation

There are **two parallel, partially-overlapping output subsystems** in this crate:

| Module | Status | Role |
|---|---|---|
| `src/output/` | **LIVE** — used by 38 files, all CLI commands | The real output coordinator: `OutputContext`, `Theme`, `components/*` |
| `src/format/` | **SPLIT** — data types + text helpers live; rich/theme/context/syntax/markdown effectively dead | `output.rs` (JSON schema types), `text.rs`, `csv.rs` are live; `rich.rs`, `theme.rs`, `context.rs`, `syntax.rs`, `markdown.rs` are an orphaned earlier generation |

Both define a struct named `OutputContext` and an enum named `OutputMode`, and both define a
struct named `Theme` — with *different fields and different semantics*. This is the single most
important structural fact about this subsystem.

Line counts (`wc -l`):

```
 344 src/format/context.rs      <- dead (only used by format/syntax.rs + format/markdown.rs)
 294 src/format/csv.rs          <- LIVE (list, search)
 502 src/format/markdown.rs     <- dead
  67 src/format/mod.rs
 309 src/format/output.rs       <- LIVE (all JSON output types + schemars schemas)
 427 src/format/rich.rs         <- dead
 387 src/format/syntax.rs       <- dead
 467 src/format/text.rs         <- LIVE (plain/ANSI text primitives)
 267 src/format/theme.rs        <- dead
 379 src/output/context.rs      <- LIVE (the real OutputContext)
  56 src/output/mod.rs
 153 src/output/theme.rs        <- LIVE Theme
  98 src/output/components/dep_tree.rs     <- dead (DependencyTree unused)
 245 src/output/components/issue_panel.rs  <- LIVE (show)
 321 src/output/components/issue_table.rs  <- LIVE (list, ready, search, orphans)
  11 src/output/components/mod.rs
  63 src/output/components/progress.rs     <- dead (name-collides with util::progress)
  50 src/output/components/stats.rs        <- dead (StatsPanel unused)
```

---

## 1. Output modes: definitions and selection

### 1.1 The live enum — five variants, not four

`src/output/context.rs:25-37`:

```rust
pub enum OutputMode { Rich, Plain, Json, Toon, Quiet }
```

The documented "four output modes" (CLAUDE.md, `AGENTS.md:386-406`, `src/output/mod.rs:6-14`) is
**stale**: `Toon` was added as a fifth mode by commit `b1882b8 feat(output): add TOON format output
mode for token-optimized serialization` and none of the mode-count prose was updated.

The dead enum in `src/format/context.rs:33-50` still has exactly four (`Rich, Plain, Json, Quiet`)
and derives `Default = Rich` — the live one derives nothing and has no default.

### 1.2 Global flags (clap)

`src/cli/mod.rs:658-705` — global flags on `Cli`:

- `--db <PATH>` (`:659`)
- `--actor <NAME>` (`:663`)
- `--json` (`:667-668`) — **global**
- `--no-daemon`, `--no-auto-flush`, `--no-auto-import`, `--allow-stale`, `--lock-timeout`, `--no-db`
- `-v/--verbose` (count) (`:695`)
- `-q/--quiet` (`:699-700`) — **global**
- `--no-color` (`:703-704`) — **global**

`--robot` is **not** global. It exists as a per-command bool on exactly 10 arg structs
(`src/cli/mod.rs`, `pub robot: bool` × 10): `DeferArgs, UndeferArgs, ReadyArgs, BlockedArgs,
CloseArgs, ReopenArgs, SyncArgs, StatsArgs, OrphansArgs, ChangelogArgs`. Its doc string is
consistently `/// Machine-readable output (alias for --json)` (e.g. `src/cli/mod.rs:1857-1859`).

`--format` exists on 7 arg structs: `SchemaArgs, ListArgs, ShowArgs, DepListArgs, ReadyArgs,
BlockedArgs, StatsArgs` (`SearchArgs` inherits it by embedding `ListArgs` as `filters`, see
`src/cli/commands/search.rs:72`). `--stats` (TOON token-savings report) exists on the same 7.

`--wrap` exists on 6: `ListArgs, ShowArgs, CommentsArgs, CommentListArgs, ReadyArgs, BlockedArgs`.

### 1.3 Format enums and env precedence

`src/cli/mod.rs:1200-1212` — `OutputFormat { Text (default), Json, Csv, Toon }`.
`src/cli/mod.rs:1244-1254` — `OutputFormatBasic { Text (default), Json, Toon }` (no CSV), with
`From<OutputFormatBasic> for OutputFormat` at `:1256-1264`.

Env resolution, `src/cli/mod.rs:1214-1241`:

```rust
pub fn from_env() -> Option<Self> {
    BR_OUTPUT_FORMAT  -> parse     // highest
    TOON_DEFAULT_FORMAT -> parse   // fallback
    None
}
fn parse_env_value: "text"|"plain" => Text, "json" => Json, "csv" => Csv, "toon" => Toon
```

Unrecognized values fall through silently to the next source (no error, no warning).

Full precedence, `src/cli/mod.rs:1266-1294`:

```rust
pub fn resolve_output_format(requested, json, robot) -> OutputFormat {
    if json || robot { Json }            // --json / --robot beat everything
    else if let Some(r) = requested { r }// --format
    else { from_env().unwrap_or(Text) }  // BR_OUTPUT_FORMAT > TOON_DEFAULT_FORMAT > text
}
pub fn resolve_output_format_basic(...) // same, then maps Csv -> Text
```

So: **`--json`/`--robot` > `--format` > `BR_OUTPUT_FORMAT` > `TOON_DEFAULT_FORMAT` > `text`.**
This matches `CLI_SCHEMA.json` ("Force machine-readable JSON output (overrides --format)") and
`docs/agent/ROBOT_MODE.md`.

### 1.4 Mode selection — three separate constructors

`OutputContext` (live) has three construction paths, all in `src/output/context.rs`:

**(a) `from_args(&Cli)` → `detect_mode`** (`:45-52`, `:107-121`) — used exactly once, in
`src/main.rs:19`, producing the `outer_ctx` handed to every command:

```
1. args.json                    -> Json
2. args.quiet                   -> Quiet
3. args.no_color || NO_COLOR    -> Plain
4. !stdout.is_terminal()        -> Plain
5. otherwise                    -> Rich
```

**(b) `from_flags(json, quiet, no_color)`** (`:59-77`) — same ladder, collapsed. Not used by
`main.rs`; present for library/embedding use.

**(c) `from_output_format(format, quiet, no_color)`** (`:81-105`) — the one commands actually
use to build their *local* ctx:

```rust
Json  -> OutputMode::Json
Toon  -> OutputMode::Toon
Text | Csv -> if quiet { Quiet }
              else if no_color || NO_COLOR || !stdout.is_terminal() { Plain }
              else { Rich }
```

Note there is **no `OutputMode::Csv`** — CSV rides the Text/Rich/Plain branch and is dispatched by
matching on the *format* enum, not the mode (`src/cli/commands/list.rs:117-121`).

Note also that in (c) `quiet` is checked **before** color/TTY, but only inside the `Text|Csv` arm —
so `--quiet --format json` yields `Json`, not `Quiet`, whereas `--quiet --json` via path (a) yields
`Quiet`. Commands that build their own ctx therefore honor `--quiet` only for text output.

**Typical command wiring** (canonical example `src/cli/commands/ready.rs:41-44`):

```rust
let output_format = resolve_output_format_basic(args.format, outer_ctx.is_json(), args.robot);
let quiet = cli.quiet.unwrap_or(false);
let ctx = OutputContext::from_output_format(output_format, quiet, !use_color);
```

where `use_color = config::should_use_color(&config_layer)` (`src/cli/commands/ready.rs:35`).

### 1.5 `should_use_color` — a fourth, config-aware color path

`src/config/mod.rs:892-907`:

```
1. config display.color / display-color / display_color   (if set, WINS)
2. NO_COLOR env var                                        -> false
3. stdout.is_terminal()
```

`--no-color` reaches this via `main.rs:333`: `display_color: if cli.no_color { Some(false) } else { None }`
→ inserted into the CLI config layer. So `--no-color` works, but only by masquerading as config.
Documented key: `docs/ARCHITECTURE.md:388` (`display.color | auto | ANSI color output`).

### 1.6 Terminal width — two implementations

- `src/format/text.rs:168-187` `terminal_width()`: `COLUMNS` env → crossterm `size()` → `80`.
- `src/output/context.rs:157-159` `OutputContext::width()`: lazily `self.console().width()` (rich_rust).
- `src/format/context.rs:246-266` `terminal_height()`: `LINES` env → crossterm → `None`. (dead path)

Commands mix both in the same function, e.g. `src/cli/commands/ready.rs:36-40` uses
`format::terminal_width()` for the text-line truncation budget while `:123` uses `ctx.width()` for
the rich table. Same in `src/cli/commands/list.rs:36-40` vs `:151`.

---

## 2. `OutputContext` (live) — API surface and semantics

`src/output/context.rs:14-23`. Fields: `mode: OutputMode` (eager, cheap) plus `OnceLock` for
`width`, `console: Console`, `theme: Theme`. Documented design principle: *"Zero overhead in
JSON/Quiet modes: Console and theme are lazy-initialized"* (`src/output/mod.rs:43-48`).

Console construction (`:124-131`): `Rich` → `Console::new()`; everything else →
`Console::builder().no_color().force_terminal(false).build()`.

### 2.1 Mode predicates
`mode()`, `is_rich()`, `is_json()`, `is_toon()`, `is_quiet()`, `is_plain()` — `:137-154`.

### 2.2 Output methods (with call-site counts across `src/`)

| Method | Site | Behavior by mode | uses |
|---|---|---|---|
| `print(&str)` | `:172-179` | Rich/Plain → `console.print`; Quiet/Json/Toon → **no-op** | 15 |
| `render<R: Renderable>` | `:181-185` | **Rich only** — silently drops in Plain | 27 |
| `json<T>` | `:190-203` | Json only; `serde_json::to_writer` (compact) + `\n`, streamed via `BufWriter` | 10 |
| `json_pretty<T>` | `:208-227` | Rich → `rich_rust::renderables::Json` (highlighted); Json → `to_writer_pretty` + `\n` | 70 |
| `toon<T>` | `:234-249` | Toon only; `toon_rust::encode` with `indent:2, key_folding: Safe` | 4 |
| `toon_with_stats<T>` | `:256-289` | as above + optional `[stats] JSON: N chars, TOON: M chars (P% savings)` on **stderr** if `show_stats \|\| TOON_STATS` env | 8 |
| `success(&str)` | `:295-304` | Rich → `[bold green]✓[/] msg`; Plain → `println!("✓ {}")`; Quiet/Json/Toon no-op | 10 |
| `error(&str)` | `:306-316` | Rich → `Panel`; Plain/Quiet → `eprintln!("Error: ")`; Json/Toon no-op | 1 |
| `warning(&str)` | `:318-327` | Rich → stdout markup; Plain → **stderr**; else no-op | 3 |
| `info(&str)` | `:329-337` | Rich → `[blue]ℹ[/] msg`; Plain → `println!` (stdout); else no-op | 10 |
| `section(&str)` | `:339-348` | Rich → `Rule`; Plain → `\n─── t ───\n` | **0** |
| `newline()` | `:350-354` | suppressed in Quiet/Json/Toon | 1 |
| `error_panel(...)` | `:356-378` | Rich → Panel w/ suggestions; Plain/Quiet → stderr lines | **0** |

`section` and `error_panel` are **dead API**. `error` has a single call site.

### 2.3 Serialization panic contract

`json`/`json_pretty` assert that any `serde_json` failure `err.is_io()` (`:196-200`, `:220-224`) —
i.e. serialization errors panic with *"JSON serialization failed - value is not serializable"*, but
broken-pipe/IO errors are swallowed. `toon`/`toon_with_stats` use `.expect(...)` outright
(`:237-238`, `:257-261`), so they panic on IO too — a `br list --format toon | head -1` could panic
where `--format json` would not.

### 2.4 A structurally unreachable branch

`json_pretty`'s `is_rich()` branch (`:209-215`) renders syntax-highlighted JSON. But every call site
first builds `ctx` from `OutputFormat::Json` → `OutputMode::Json`, so `is_rich()` is false. The
highlighted-JSON path is effectively unreachable through the CLI.

---

## 3. Theming

### 3.1 Live theme: `src/output/theme.rs`

Struct `Theme` (`:14-61`) with **41 fields** of `rich_rust::Style` plus
`box_style: &'static BoxChars` (defaults to `ROUNDED`, `:110`). Groups:

- Semantic: `success, error, warning, info, dimmed, accent, highlight, muted, emphasis`
- Issue: `issue_id, issue_title, issue_description`
- Status: `status_open (green), status_in_progress (yellow+bold), status_blocked (red),
  status_deferred (blue+dim), status_closed (bright_black)` — `:80-84`
- Priority: `priority_critical (red+bold), priority_high (red), priority_medium (yellow),
  priority_low (green), priority_backlog (bright_black)` — `:86-90`
- Type: `type_task (blue), type_bug (red), type_feature (green), type_epic (magenta+bold),
  type_chore (bright_black), type_docs (cyan), type_question (yellow)` — `:92-98`
- Chrome: `table_header, table_border, panel_title, panel_border, section, label, timestamp,
  username, comment`

Dispatchers return **owned clones**: `status_style(&Status) -> Style` (`:117-127`),
`priority_style(Priority) -> Style` (`:129-138`), `type_style(&IssueType) -> Style` (`:140-152`).
`Status::Tombstone | Status::Custom(_)` → `muted`; `Status::Pinned` → `highlight`.

Color parsing helper `color(name)` (`:7-12`) does `Color::parse(name).unwrap_or_else(|_| {
debug_assert!(false, ...); Color::default_color() })` — i.e. it **panics in debug, silently degrades
in release**. Same helper duplicated at `src/format/theme.rs:15-20`.

**There is no theme selection mechanism at all.** `ctx.theme()` (`src/output/context.rs:164-166`)
always does `self.theme.get_or_init(Theme::default)`. No env var, no config key, no CLI flag.

### 3.2 Dead theme: `src/format/theme.rs`

A *different* `Theme` with 24 fields (`:27-63`), borrowed-return dispatchers (`&Style` not `Style`,
`:116-151`), and — notably — the only theme *variants* in the codebase: `Theme::dark()`
(`:157-176`, brightened colors) and `Theme::minimal()` (`:182-219`, bold/dim monochrome).
**Nothing references `format::Theme`** — grep for `format::Theme` across `src/` and `tests/`
returns zero hits outside the `pub use theme::Theme;` re-export at `src/format/mod.rs:58`.

The two themes also *disagree* on colors, despite both claiming to match `text.rs`:

| Concept | `output::Theme` | `format::Theme` | `format::text` (actual plain-text renderer) |
|---|---|---|---|
| feature type | green (`output/theme.rs:94`) | cyan (`format/theme.rs:92`) | cyan (`format/text.rs:152`) |
| task type | blue (`:92`) | unstyled (`:93`) | unstyled (`format/text.rs:153`) |
| docs type | cyan (`:97`) | blue (`:95`) | blue (`format/text.rs:155`) |
| priority low (P3) | green (`:89`) | bright_black (`:88`) | grey (`format/text.rs:123`) |

So **Rich-mode colors do not match Plain-mode colors** for type badges and P3/P4 priorities.
`src/format/theme.rs:3-6` explicitly claims the mapping matches `text.rs` — true for the dead
theme, false for the live one.

---

## 4. Component system (`src/output/components/`)

`src/output/components/mod.rs:1-11` declares 5 components and re-exports them; `src/output/mod.rs:54`
does `pub use components::*;`.

### 4.1 `IssueTable` — LIVE (the workhorse)

`src/output/components/issue_table.rs:10-19`. Builder over `&[Issue]` + `&Theme`.

- `IssueTableColumns` (`:21-34`): 10 bools — `id, priority, status, issue_type, title, assignee,
  labels, created, updated, context`.
- Presets `compact()` (`:38-46`), `standard()` (`:49-59`), `full()` (`:62-75`). Only `standard()`
  is referenced, as the `new()` default (`:84`). **`compact()` and `full()` are dead** — every call
  site builds a struct literal instead (`list.rs:125-145`, `ready.rs:106-113`, `orphans.rs:149`,
  `search.rs`).
- Fluent setters: `width(Option<usize>)`, `wrap(bool)`, `columns()`, `title()`,
  `highlight_query()` (`:118-124`), `context_snippets(HashMap<String,String>)` (`:127-132`).
- `build() -> Table` (`:136-261`): column construction with explicit min/max widths
  (`ID` min 10, `P` width 3 centered, `Status` min 8, `Type` min 7, `Title` min 20 /
  max `title_max_width`, `Assignee` max 20, `Labels` max 30, `Created`/`Updated` width 10,
  `Context` 20–60).
- Title budget (`:143`): `self.width.map_or(60, |w| w.saturating_sub(100).max(60))` — "Reserve ~100
  chars for other columns (conservative) or min 60". Because `.width(Some(..))` is only ever set
  together with `wrap(true)` (see `list.rs:150-152`, `ready.rs:122-124`) and `wrap` skips truncation
  (`:216-220`), the `w-100` computation only ever affects `Column::max_width`, never truncation.
- Search highlighting: `build_highlight_regex` (`:264-274`) does `regex::escape` + case-insensitive
  — literal match only, no regex injection. `highlight_text` (`:276-304`) splices
  `theme.highlight` styled spans. **Byte-index slicing** (`&text[last..start]`) is safe here because
  regex match offsets are on char boundaries.
- Consumers: `list.rs:146`, `ready.rs:114`, `search.rs:163`, `orphans.rs:162`.

### 4.2 `IssuePanel` — LIVE (`show` only)

`src/output/components/issue_panel.rs:7-14`. `from_details(&IssueDetails, &Theme)` (`:30-39`) is the
only constructor used (`src/cli/commands/show.rs:78`). `print(&self, ctx, wrap)` (`:59-181`) builds
one big `Text` with header badges, title, description, a `───` divider, assignee/labels/timestamps,
dependency and dependent lists (`render_dependency_list`, `:197-226`), comments, then a `Panel`.

Width: `let panel_width = if wrap { ctx.width() } else { 80 };` (`:169`) — **hardcoded 80** when not
wrapping, ignoring the terminal. `wrap_rich_text` (`:184-195`) wraps to `panel_width - 4`.
Divider strings are hardcoded 35-char runs (`:90`, `:208`, `:234`) regardless of panel width.

### 4.3 Dead components

- `DependencyTree` (`dep_tree.rs:6-71`) — zero references outside the file. Builds a `rich_rust::Tree`
  with `TreeGuides::Rounded`, `max_depth` default 10, and a local `truncate()` (`:73-81`) that counts
  **chars** not display width (unlike `format::truncate_title`).
- `StatsPanel` (`stats.rs:5-50`) — zero references.
- `ProgressTracker` (`progress.rs:6-63`) — zero references, **and its name collides** with the live
  `crate::util::progress::ProgressTracker` (`src/util/progress.rs:121`). Because
  `src/output/mod.rs:54` does `pub use components::*;`, `crate::output::ProgressTracker` and
  `crate::util::progress::ProgressTracker` are two different public types with the same name.

`dep tree` rendering actually lives in `src/cli/commands/dep.rs` (and supports `--format text|mermaid`,
a **third** `--format` semantic — a plain `String` field, `src/cli/mod.rs:1552-1554`).

---

## 5. `src/format/` — the live parts

### 5.1 `src/format/output.rs` — the JSON schema surface

This is where the machine-readable contract lives. Every type derives
`Serialize, Deserialize, JsonSchema` (schemars).

| Type | Line | Shape | Notes |
|---|---|---|---|
| `StaleIssue` | `:9-17` | 7 flat fields | *"bd parity ... only the fields bd's stale command outputs"* |
| `ReadyIssue` | `:24-46` | 14 fields, 8 `skip_serializing_if = Option::is_none` | *"Does NOT include: compaction_level, original_size, dependency_count, dependent_count"* (`:22`) |
| `BlockedIssueOutput` | `:74-88` | flat + `blocked_by: Vec<String>`, `blocked_by_count` | `:71-72` |
| `IssueWithCounts` | `:105-111` | `#[serde(flatten)] issue: Issue` + 2 counts | list/search |
| `IssueDetails` | `:114-130` | flatten `Issue` + `labels, dependencies, dependents, comments, events, parent`, all `skip_serializing_if` empty | show |
| `IssueWithDependencyMetadata` | `:132-140` | `id,title,status,priority` + `dep_type` **renamed to `dependency_type`** (`:138`) | |
| `BlockedIssue` | `:143-149` | flatten `Issue` + blockers | |
| `TreeNode` | `:152-159` | flatten `Issue` + `depth, parent_id, truncated` | |
| `StatsSummary` | `:163-176` | 10 counts + optional `average_lead_time_hours` | |
| `Breakdown` / `BreakdownEntry` | `:180-190` | `dimension` + `[{key,count}]` | |
| `RecentActivity` | `:194-202` | git-derived counters | |
| `Statistics` | `:206-212` | `summary` + `breakdowns` + `recent_activity` | |

`From<&Issue>` impls: `ReadyIssue` (`:48-67`), `StaleIssue` (`:90-102`).

**The `#[serde(flatten)]` pattern is load-bearing for stability**: `IssueWithCounts`,
`IssueDetails`, `BlockedIssue`, `TreeNode` all flatten the full `model::Issue`, so *any* field added
to `Issue` immediately appears in list/show/blocked/tree JSON. `ReadyIssue`/`StaleIssue`/
`BlockedIssueOutput` are the hand-curated, non-flattened bd-parity subsets.

### 5.2 `src/format/text.rs` — plain-text primitives

- `mod icons` (`:14-31`): `○` open, `◐` in_progress, `●` blocked, `❄` deferred, `✓` closed,
  `✗` tombstone, `📌` pinned, `?` unknown.
- `format_status_icon` (`:54-65`), `format_status_label` (`:75-90`), `format_status_icon_colored`
  (`:94-109`), `format_priority` → `"P{n}"` (`:69-71`), `format_priority_label` (`:113-126`),
  `format_priority_badge` → `"[● P2]"` (`:132-134`, *"Matches bd format"*),
  `format_type_badge` → `"[task]"` (`:138-140`), `format_type_badge_colored` (`:144-160`).
- Colors applied via `crossterm::style::Stylize` — a **different** styling stack from rich_rust.
- `terminal_width()` (`:169-187`).
- `truncate_title(title, max_len)` (`:193-230`) — **unicode-width aware** (`UnicodeWidthChar`),
  appends `...` when `max_len > 3`, hard-truncates when `max_len <= 3`, returns `""` at 0.
  Regression-tested at `tests/repro_truncate_width.rs` and
  `src/output/components/issue_table.rs:310-320` (60 emoji = 120 columns).
- `format_issue_line_with` (`:241-274`) — canonical bd-parity line:
  `{icon} {id} [● {priority}] [{type}] - {title}`, with a prefix-width budget computed at `:248-255`
  including a hardcoded `+3` for `" - "`. Asserted exactly at `:377`.

### 5.3 `src/format/csv.rs` — CSV export

- `DEFAULT_FIELDS` (8) `:10-19`; `ALL_FIELDS` (15) `:22-38`.
- `escape_field` (`:45-55`) — RFC-4180-ish: quotes when value contains `,`, `"`, `\n`, or `\r`;
  doubles embedded quotes.
- `get_field_value` (`:59-82`) — unknown field → `""` (silent).
- `parse_fields` (`:88-97`) — **silently drops unknown field names** via
  `filter_map(|f| ALL_FIELDS.iter().find(...))`; empty/`None` → defaults. Asked for
  `--fields id,invalid,title` you get `id,title` with no diagnostic (test asserts this,
  `:251-255`).
- Timestamps emitted as RFC-3339 (`:69-77`).
- Only consumers: `src/cli/commands/list.rs:117-121` and `search.rs:131`. CSV is *not* reachable via
  `OutputFormatBasic`, so `--format csv` only exists on `list`/`search`.

### 5.4 The dead `src/format/` subtree

- `src/format/context.rs` — 4-variant `OutputMode` + `OutputContext` with `detect()`, `json()`,
  `quiet()`, `with_mode()`, `from_flags()`, `with_width()`. Only consumed by `format/syntax.rs:24`
  and `format/markdown.rs:24`.
- `src/format/rich.rs` — `RichIssueTable` (`:46-128`), `RichIssuePanel` (`:131-190`),
  `RichDependencyTree` (`:193-250`), `format_status_badge` (`:254-262`), `format_count_badges`
  (`:266-305`), `build_completion_bar` (`:309-319`). All referenced only from its own `mod tests`.
  Curiously, it imports `crate::output::Theme` (`:40`), not the sibling `format::theme::Theme` —
  evidence it was half-migrated. Its module doc (`:19`) still tells you to
  `use crate::format::{OutputContext, OutputMode, Theme}`.
- `src/format/syntax.rs` — mode-aware syntect highlighting: Quiet → `""`, Json → raw,
  Plain → 4-space-indented, Rich → syntect `base16-ocean.dark`, line numbers when >5 lines
  (`:48-96`). Language alias normalization `:99-118`; `parse_code_fence` `:131-169`;
  `detect_language_from_filename` `:173-209`.
- `src/format/markdown.rs` — mode-aware markdown: Quiet → `""`, Json → raw, Plain →
  `strip_markdown` (a 200-line hand-rolled stripper, `:63-277`), Rich → `rich_rust` `Markdown`
  with `hyperlinks(true)` (`:280-296`). Plus `contains_markdown` (`:302-325`) and
  `escape_markdown` (`:331-346`).

Nothing in `src/cli/`, `src/storage/`, `src/sync/`, `tests/`, or `benches/` calls any of these.
**Issue descriptions/comments are therefore never markdown-rendered or syntax-highlighted** despite
the modules existing — `IssuePanel` prints the raw description at `issue_panel.rs:82-86`.

---

## 6. JSON output schema stability guarantees

### 6.1 What's claimed

`AGENTS.md:416-421`:

> JSON mode guarantees:
> - Stable schema (changes are versioned and documented)
> - No ANSI escape codes
> - Clean stdout (diagnostics go to stderr)
> - Exit codes for success/failure

### 6.2 What's actually enforced

**(a) `br schema` — a real, runtime schema surface.** `src/cli/commands/schema.rs`.

- Targets (`src/cli/mod.rs`, `SchemaTarget`; built in `schema.rs:91-136`): `all, issue,
  issue-with-counts, issue-details, ready-issue, stale-issue, blocked-issue, tree-node, statistics,
  error`. Matches `CLI_SCHEMA.json`'s `schema_surface.targets` exactly.
- Uses `schemars::schema_for!` on the live Rust types — so the schema **cannot drift from the code**.
- Envelope (`schema.rs:40-45`): `{ tool: "obr", generated_at: <RFC3339>, schemas: {Name: RootSchema} }`.
  `BTreeMap` keys → deterministic ordering.
- Works without an initialized workspace (`tests/e2e_schema.rs:11-36`), and text mode still prints
  pretty JSON rather than requiring `--json` (`schema.rs:81-85`, comment at `:82`).
- **`generated_at: Utc::now()` breaks byte-for-byte determinism** — acknowledged as a known gap in
  `AGENT_FRIENDLINESS_REPORT.md` ("Schema outputs include `generated_at` (useful, but not
  deterministic byte-for-byte)").
- `ErrorEnvelope`/`ErrorBody` (`schema.rs:21-38`) are a **hand-written mirror** of the real
  `StructuredError` (`src/error/structured.rs:250-264`), not derived from it. They agree today
  (`code, message, hint, retryable, context`) but nothing enforces that.

**(b) Snapshot tests — the actual regression fence.** `tests/snapshots/` (insta):

- `tests/snapshots/json_output.rs` — `assert_json_snapshot!` for list/show/ready/blocked/count
  (incl. grouped variants)/create/close/comments/dep-list/graph, with `normalize_json` to strip
  volatile fields. ~40+ `.snap` files under `tests/snapshots/snapshots/`.
- `tests/snapshots/cli_output.rs` — text snapshots for help, list, ready, show, stats, version, etc.
- `tests/snapshots/error_messages.rs` — 7 error snapshots (cycle, invalid priority/status,
  not found, not initialized, self-dependency, update-closed).
- `tests/snapshots/jsonl_format.rs` — JSONL wire format.

**(c) bd (Go) conformance.** `tests/conformance*.rs` compare against the Go `bd` binary; they
`skip_if_no_bd!` when `bd` isn't installed (`tests/conformance_text_output.rs:24-31`), so **in CI
they are no-ops**. `tests/conformance_text_output.rs` normalizes ANSI and IDs by regex before
comparing.

**(d) Static baseline snapshots.** `agent_baseline/` — `README_first_80_lines.md`,
`AGENT_JOURNEY_NOTES.md`, `help/{br_help,br_list_help,br_schema_help}.txt`,
`schemas/{schema_all,schema_error,schema_issue_details}.json`,
`examples/{list_limit3,ready,show_one}.{json,toon}` + `version.json`,
`errors/show_not_found.json`. These are *frozen artifacts*, not asserted by any test — a
documentation fallback ("As a fallback, this repo also includes a captured snapshot bundle",
`docs/agent/SCHEMA.md`).

### 6.3 What is *not* guaranteed

- **No envelope.** Read commands emit bare arrays/objects. `AGENT_FRIENDLINESS_REPORT.md` flags this
  itself: *"Many commands return bare arrays/objects rather than a consistent `{data, metadata,
  errors}` envelope."*
- **No version field** in any data payload. Only `br schema` carries `tool`/`generated_at`.
- **Pretty vs compact is inconsistent.** 70 sites use `json_pretty`, 10 use compact `json`:
  `epic.rs:97`, `version.rs:74,201,223`, `config.rs:726`, `create.rs:397`, `doctor.rs:67`,
  `stale.rs:51`, `q.rs:133`. So `br stale --json` is single-line while `br list --json` is
  multi-line. (Both parse fine; it matters for diffing and for line-oriented tooling.)
- **`#[serde(flatten)]` makes `Issue` field additions silently additive** to list/show/blocked/tree
  output — good for forward compat, but it means the "schema" is whatever `model::Issue` currently is.
- **TOON key folding changes the shape.** `KeyFoldingMode::Safe` (`src/output/context.rs:242`) may
  emit `schemas.IssueDetails` as a dotted key instead of a nested object; the test at
  `tests/e2e_schema.rs:63-72` explicitly accepts *either* form. `docs/agent/SCHEMA.md` documents this.

---

## 7. The error envelope

`src/main.rs:300-325` — `handle_error(err, json_mode)`:

```rust
let structured = StructuredError::from_error(err);
let exit_code = structured.code.exit_code();
let use_json = json_mode || !io::stdout().is_terminal();   // <-- key
if use_json { eprintln!("{}", to_string_pretty(structured.to_json())) }
else        { eprintln!("{}", structured.to_human(io::stderr().is_terminal())) }
std::process::exit(exit_code);
```

Note: `use_json` keys off `--json` **or non-TTY stdout** — so *piping* `br` produces JSON errors on
stderr even without `--json`. This is what `docs/agent/ERRORS.md` and `ROBOT_MODE_EXAMPLES.jsonl`
describe. `--robot` is *not* consulted (only the global `cli.json` is passed at `main.rs:34,129`),
so `br ready --robot 2>err.json` on a TTY yields a **human-readable** error, not JSON.

Shape (`src/error/structured.rs:450-460`):

```json
{"error": {"code": "...", "message": "...", "hint": "...|null", "retryable": bool, "context": {...}|null}}
```

Captured canonical example: `agent_baseline/errors/show_not_found.json`. Note `to_json` (`:450`)
always emits `hint`/`context` keys (possibly `null`), whereas the `Serialize` derive on the struct
skips them when `None` (`:257-258`, `:262-263`) — **two different serializations of the same type**.
The `br schema error` output describes the *struct*, but `handle_error` emits the *`to_json`* form.

`ErrorCode::exit_code()` (`src/error/structured.rs:199-239`) groups exit codes:
1 internal, 2 database, 3 issue/operational, 4 validation, 5 dependency, 6 sync/JSONL, 7 config,
8 I/O. `is_retryable()` (`:174-185`) marks database-locked / validation-family errors retryable.
`StructuredError::issue_not_found` (`:284-299`) computes Levenshtein-ish "Did you mean" suggestions
via `find_similar_ids` and stuffs them into `context.similar_ids` — a genuinely agent-oriented
self-correction affordance.

---

## 8. TOON — the token-efficiency story

- Dependency: `toon_rust` pinned to a **git rev**, `Cargo.toml:63` /
  `Cargo.lock:3957-3959` (`rev = 788589d`).
- Encoding options fixed at `src/output/context.rs:239-245` / `:263-269`:
  `indent: Some(2), delimiter: None, key_folding: Some(Safe), flatten_depth: None, replacer: None`.
- Available on 8 command modules: `list, show, ready, blocked, search, stats, dep, schema`.
- `--stats` / `TOON_STATS=1` prints a savings line to **stderr** (`:281-285`) — importantly *not*
  stdout, so it doesn't corrupt the payload.
- Decoding requires the external `tru` CLI (`docs/agent/ROBOT_MODE.md`, "TOON decode tool (`tru`)").
  `scripts/agent_smoke_test.sh` hard-requires both `jq` and `tru` (`need_cmd jq; need_cmd tru`).
- Sample outputs: `agent_baseline/examples/ready.toon`, `list_limit3.toon` — array header `[3]:`
  followed by `- key: value` blocks.

---

## 9. The agent-friendliness artifacts

### 9.1 `ROBOT_MODE_EXAMPLES.jsonl` (3 lines, 722 bytes)

Machine-readable "how do I call this" index. Fields per line:
`name, cmd, stdout_format, [decode_cmd], [schema_cmd], [stderr_format], notes`.

1. `ready_toon` — `br ready --format toon --limit 10`, decode `tru --decode`,
   schema `br schema ready-issue --format json`.
2. `show_json` — `br show bd-XXXX --format json`, schema `br schema issue-details --format json`.
3. `error_envelope` — `br show bd-NOTEXIST --format json > /dev/null 2>err.json || true`,
   `stdout_format: "(none)"`, `stderr_format: "json"`, schema `br schema error --format json`.

### 9.2 `CLI_SCHEMA.json` (1.8 KB)

Static machine-readable CLI descriptor: `tool`, `repo`, `purpose`, `entrypoints.{binary,
agent_docs[6], deep_docs[2]}`, `global_flags[--json,--no-color,--quiet]`,
`format_env.{BR_OUTPUT_FORMAT, TOON_DEFAULT_FORMAT}` (each with `values` and `notes`),
`schema_surface.{supported, command, targets[10]}`, `output_notes[3]`.

Explicitly framed as interim: *"No dynamic `--help-json` surface yet; `CLI_SCHEMA.json` is an
interim static artifact"* (`AGENT_FRIENDLINESS_REPORT.md`, Gaps section).

**It under-documents the real surface**: it lists only 3 global flags (the real `Cli` has 12,
`src/cli/mod.rs:658-705`) and never mentions `--robot`, `--wrap`, `--stats`, `NO_COLOR`, or
`TOON_STATS`.

### 9.3 `AGENT_FRIENDLINESS_REPORT.md`

Audit dated 2026-01-25 by "WildAnchor (Codex / GPT-5)", bead `bd-3s2`. Key content:

- Enumerates the agent surfaces: JSON, TOON, `br schema`, error envelope.
- **Interface Modality Decision: CLI-only, no MCP surface.** Rationale: br is a composable local
  primitive (pipes, git hooks, scripts); MCP would add distribution/auth/permission surface for no
  core value; the CLI already covers triage (`ready`), mutation (`update`/`close`), sync
  (`sync --flush-only`).
- Self-declared gaps: no `--help-json`; no `{data, metadata, errors}` envelope; `generated_at`
  non-determinism.
- Scorecard: Documentation 5, CLI ergonomics 5, Robot/machine mode 5, Schemas 4, Errors 4,
  Consistency 4, Overall 4.5.

### 9.4 `CHANGELOG_AGENT_FRIENDLY.md` (479 bytes)

Single dated entry (2026-01-25) recording: agent doc entrypoints under `docs/agent/`,
`agent_baseline/` snapshots, `ROBOT_MODE_EXAMPLES.jsonl` + `CLI_SCHEMA.json`, and removal of
`rm -rf` from local scripts/tests per the no-deletion policy in `AGENTS.md`.

### 9.5 `docs/agent/` (6 files)

`AGENTS.md`, `QUICKSTART.md`, `ROBOT_MODE.md`, `SCHEMA.md`, `ERRORS.md`, `EXAMPLES.md`.
Highlights:

- `QUICKSTART.md` — a 5-step, sub-30-second loop: `init` → `ready --format json --limit 10` →
  `update --status in_progress --claim --format json` → `close --reason "..." --format json` →
  `sync --flush-only`. Explicit gotcha: *"When scripting, route stderr separately; errors may be
  emitted as structured JSON on stderr."*
- `ROBOT_MODE.md` — format selection, env defaults, stdout/stderr split, `--wrap`, `tru`, smoke test.
- `EXAMPLES.md` — includes a **determinism smoke check**: run `br list --format json --limit 5 |
  jq -S .` twice and `diff -u`.

### 9.6 `scripts/agent_smoke_test.sh`

Asserts, in order: JSON list/show parse (`jq -e`), TOON list/show decode+parse (`tru --decode | jq`),
`TOON_DEFAULT_FORMAT` defaulting, `BR_OUTPUT_FORMAT` beating `TOON_DEFAULT_FORMAT`, and
`.error.code == "ISSUE_NOT_FOUND"` in captured stderr. Explicitly **does not delete** its temp
workspace ("NOTE: Workspace is left in place (no auto-delete)") — a deliberate consequence of the
no-deletion policy. Pins `RUST_LOG=${RUST_LOG:-error}` so tracing doesn't pollute stderr.

---

## 10. So what does "agent-friendly CLI" concretely mean here?

Synthesizing code + docs, the project's operative definition is a checklist of nine properties:

1. **Machine format on demand, three ways** — a global `--json`, a per-command `--robot` alias, and
   a `--format {text|json|toon|csv}`, with documented precedence (`src/cli/mod.rs:1266-1294`).
2. **Env-configurable default format** so a wrapper can set `BR_OUTPUT_FORMAT=json` once and stop
   passing flags (`src/cli/mod.rs:1214-1241`).
3. **Token efficiency as a first-class concern** — TOON output plus an opt-in savings meter on
   stderr (`src/output/context.rs:256-289`). This is unusual and is the most LLM-specific feature.
4. **Runtime-introspectable schemas** — `br schema <target> --format json|toon`, generated from the
   live Rust types via schemars, so agents never read source (`src/cli/commands/schema.rs`).
5. **Structured, self-correcting errors** — `{code, message, hint, retryable, context}` on stderr
   with stable exit-code families and "did you mean" ID suggestions
   (`src/error/structured.rs:199-239, 284-299, 450-460`).
6. **Strict stdout/stderr discipline** — payload on stdout; tracing forced to stderr
   (`src/logging.rs:23-24`); TOON stats to stderr; error envelopes to stderr; auto-import/auto-flush
   report only via `debug!` (`src/main.rs:245-252, 283-293`).
7. **Auto-degradation** — piping a command flips it out of Rich into Plain, and flips errors into
   JSON, with no flags (`src/output/context.rs:64-67`, `src/main.rs:309`).
8. **Non-invasiveness** — br never shells out to git; `sync --flush-only` then the agent commits
   manually. Combined with the repo-wide no-deletion policy (see `CHANGELOG_AGENT_FRIENDLY.md`,
   `scripts/agent_smoke_test.sh`).
9. **Frozen reference artifacts** — `ROBOT_MODE_EXAMPLES.jsonl`, `CLI_SCHEMA.json`,
   `agent_baseline/`, `docs/agent/`, `scripts/agent_smoke_test.sh` — so an agent can bootstrap
   without running anything.

And explicitly **not** part of the definition: an MCP server. That was considered and rejected on
record (`AGENT_FRIENDLINESS_REPORT.md`, "Interface Modality Decision").

---

## 11. Risks, oddities, inconsistencies (detailed)

### 11.1 Structural

**R1. Duplicate `OutputContext`/`OutputMode`/`Theme`.** `src/format/{context,theme,rich,syntax,
markdown}.rs` (~1,927 lines) are an earlier, abandoned generation with the same type names as the
live `src/output/` code. All are `pub`, so they're part of the library's public API and produce no
dead-code warnings. `src/format/mod.rs:29-33` documents the 4-mode model; the live model has 5.
Anyone reading `src/format/mod.rs` first will map the wrong subsystem.

**R2. `format::rich` imports `output::Theme`.** `src/format/rich.rs:40` — a half-finished migration.
Its own doc comment at `:19` says to import `Theme` from `crate::format`.

**R3. Duplicate `ProgressTracker`.** `crate::output::ProgressTracker`
(`src/output/components/progress.rs:6`, dead) vs `crate::util::progress::ProgressTracker`
(`src/util/progress.rs:121`, live). Re-exported into `crate::output::*` by
`src/output/mod.rs:54`, so both are reachable as `ProgressTracker` depending on the `use`.

### 11.2 Correctness / behavioral

**R4. Rich colors ≠ Plain colors.** `output::Theme` disagrees with `format::text` on `feature`
(green vs cyan), `task` (blue vs unstyled), `docs` (cyan vs blue), and P3/P4 (green vs grey). See
§3.2 table. Means `br list` in a terminal and `br list --no-color | less -R` disagree on semantics.

**R5. `--quiet` leaks output in commands that use the outer ctx.** Commands that build a *local*
ctx early-return on `OutputMode::Quiet` (17 files, 22 sites: `list.rs:77`, `ready.rs:89`,
`show.rs:62`, `blocked.rs:127`, `schema.rs:63`, `stats.rs:90`, `search.rs:112`, `dep.rs:348`,
`init.rs:107`, `orphans.rs:316`, `lint.rs:110`, `doctor.rs:70`, `history.rs:66,168,271,312`,
`config.rs:373,475,637,727,783,874`). But `stale.rs` and `count.rs` use the outer ctx and fall to
an `else` branch of raw `println!` — `br stale --quiet` still prints. There are **382 raw `println!`
and 49 `eprintln!` in `src/cli/commands/`** vs 82 `ctx.print/success/info` calls, so quiet-mode
suppression is opt-in per call site, not enforced by the abstraction.

**R6. ANSI can leak into Plain mode.** `use_color` comes from `config::should_use_color`
(`src/config/mod.rs:899-907`) which puts **config `display.color` above `NO_COLOR`**, violating the
no-color.org convention. Meanwhile `OutputContext::from_output_format` independently checks
`NO_COLOR` (`src/output/context.rs:89`). With `display.color = true` in `.beads/config` plus either
`NO_COLOR=1` or a pipe, you get `ctx.mode() == Plain` but `use_color == true`, and the plain-text
formatters (`format_issue_line_with(issue, format_options)` at `list.rs:158`) emit crossterm ANSI
into "plain" output. This directly contradicts the `AGENTS.md:418` guarantee "No ANSI escape codes".

**R7. `--robot` does not affect the error envelope.** `handle_error` receives only `cli.json`
(`src/main.rs:34,129`). On a TTY, `br ready --robot` prints a human error while printing JSON data —
a mixed-mode failure for an agent that captures stderr.

**R8. Two serializations of `StructuredError`.** `to_json()` (`structured.rs:450-460`) always
includes `hint`/`context` (possibly `null`); the derived `Serialize` skips them when `None`
(`:257-263`). `br schema error` describes the latter; the CLI emits the former.

**R9. `ErrorEnvelope` schema is hand-maintained.** `src/cli/commands/schema.rs:21-38` mirrors
`StructuredError` by hand rather than deriving `JsonSchema` on the real type. Silent drift risk.

**R10. TOON path panics on IO errors.** `toon`/`toon_with_stats` use `println!` + `.expect(...)`
(`src/output/context.rs:237, 247, 257-261, 287`) instead of the `is_io()`-tolerant assertion used by
`json`/`json_pretty` (`:196-200`). `br list --format toon | head -1` risks a panic where the JSON
path degrades gracefully.

**R11. `ctx.print` runs content through rich_rust markup parsing.** `src/output/context.rs:172-179`
passes the string to `Console::print`, the same method used with literal markup elsewhere
(`:299` `"[bold green]✓[/] {}"`). User-controlled content flows through it —
`src/cli/commands/create.rs:76-78` does `ctx.print(&format!("Title: {}", issue.title))`. An issue
title containing `[bold]` or `[/]` would be interpreted as markup rather than printed literally
(rich_rust is a crates.io dep, `Cargo.lock:3201-3204`, not vendored — worth confirming against its
`Console::print` contract, but the codebase's own usage strongly implies markup parsing).

**R12. `IssuePanel` hardcodes width 80 when not wrapping.** `issue_panel.rs:169`
(`if wrap { ctx.width() } else { 80 }`), plus hardcoded 35-char `───` dividers (`:90, 208, 234`).
`br show` in a 200-column terminal renders an 80-column box.

**R13. CSV silently drops unknown `--fields`.** `src/format/csv.rs:88-97`. `--fields id,titel` gives
you a one-column CSV with no error.

**R14. Three incompatible `--format` semantics.** `OutputFormat` (text/json/csv/toon),
`OutputFormatBasic` (text/json/toon), and a bare `String` for `dep tree` accepting `text|mermaid`
(`src/cli/mod.rs:1552-1554`). An agent that learned `--format json` from `br list` gets a different
error class on `br dep tree --format json`.

**R15. Unknown env format values fail silently.** `BR_OUTPUT_FORMAT=jsonl` falls through to
`TOON_DEFAULT_FORMAT`, then to `Text` — no diagnostic (`src/cli/mod.rs:1220-1230`).

### 11.3 Dead / vestigial

**R16.** `format::Theme::dark()` and `::minimal()` (`src/format/theme.rs:157-219`) are the only
theme *variants* in the repo, and they belong to the dead theme. The live theme has no variant
mechanism at all (`ctx.theme()` → `Theme::default` unconditionally, `src/output/context.rs:164-166`).

**R17.** `ctx.section()` and `ctx.error_panel()` have zero call sites; `ctx.error()` has one.

**R18.** `IssueTableColumns::compact()` and `::full()` are dead; every call site builds a literal.

**R19.** `DependencyTree` and `StatsPanel` components are dead.

**R20.** 32 command `execute` functions take a `_json: bool` parameter they ignore (grep
`_json: bool` in `src/cli/commands/`), because the real signal now arrives via `outer_ctx.is_json()`
plus `args.robot`. `main.rs` still computes and passes `cli.json || args.robot` for several
(`:58, 61, 78, 94, 101, 104, 107, 110`).

**R21.** Robot-flag plumbing uses four different mechanisms across commands:
`resolve_output_format_basic(..., args.robot)` (`ready.rs:41`, `stats.rs:39`, `blocked.rs`),
`ctx.is_json() || args.robot` (`orphans.rs:50`), the passed `json` bool (`changelog.rs:65`,
`close.rs:47`), and pure `ctx.is_json()` (`show.rs:42`, `list.rs:74`).

### 11.4 Documentation drift

**R22.** Binary is **`obr`**, not `br`. `Cargo.toml:12-14` (`[[bin]] name = "obr"`);
`schema.rs:70` emits `tool: "obr"`; tests use `cargo_bin!("obr")` (`tests/e2e_create_output.rs:11`).
But `CLAUDE.md`, `AGENTS.md`, all of `docs/agent/`, `CLI_SCHEMA.json` (`"binary": "br"`),
`ROBOT_MODE_EXAMPLES.jsonl` (all three `cmd` fields), and error hints
(`structured.rs:288` — `"Run 'br list' to see available issues."`) say `br`. The smoke-test script
even reveals the confusion: `elif command -v br ...; then BR="obr"` (`scripts/agent_smoke_test.sh`).
An agent following `ROBOT_MODE_EXAMPLES.jsonl` verbatim will get "command not found" unless an
alias/symlink exists.

**R23.** "Four output modes" is stale everywhere (`CLAUDE.md`, `AGENTS.md:386-406`,
`src/output/mod.rs:6-14`, `src/format/mod.rs:29-33`); there are five (`Toon`).

**R24.** `AGENTS.md:399-406` lists mode detection matching `detect_mode` — but that ladder only
governs the outer ctx. Every read command re-derives its mode via `from_output_format`, where
`--format toon` produces `Toon` and `--quiet` only wins for text formats.

**R25.** `CLI_SCHEMA.json` omits `--robot`, `--wrap`, `--stats`, `NO_COLOR`, `TOON_STATS`, and 9 of
the 12 real global flags.

**R26.** Conformance tests against Go `bd` skip silently when `bd` is absent
(`tests/conformance_text_output.rs:24-31`), which is the documented CI state — so the bd-parity
claims baked into `src/format/output.rs` doc comments and `format_issue_line_with` are unverified in
CI.

---

## 12. Quick reference — where to look for what

| Question | File:line |
|---|---|
| What modes exist (live)? | `src/output/context.rs:25-37` |
| How is mode picked from global flags? | `src/output/context.rs:107-121`, `src/main.rs:19` |
| How is mode picked per-command? | `src/output/context.rs:81-105` |
| Format/env precedence | `src/cli/mod.rs:1214-1294` |
| Global flags | `src/cli/mod.rs:658-705` |
| Color decision (config-aware) | `src/config/mod.rs:892-907`, `src/main.rs:333` |
| JSON payload types + schemars | `src/format/output.rs` |
| `br schema` implementation | `src/cli/commands/schema.rs` |
| Error envelope emission | `src/main.rs:300-325`, `src/error/structured.rs:450-488` |
| Exit-code families | `src/error/structured.rs:187-239` |
| TOON encoding options | `src/output/context.rs:239-245, 263-269` |
| Rich table | `src/output/components/issue_table.rs` |
| Rich panel (`show`) | `src/output/components/issue_panel.rs` |
| Live theme | `src/output/theme.rs` |
| Plain-text primitives / bd parity | `src/format/text.rs` |
| CSV | `src/format/csv.rs` |
| Canonical 5-mode dispatch example | `src/cli/commands/ready.rs:88-140`, `list.rs:74-163` |
| JSON snapshot fence | `tests/snapshots/json_output.rs` + `tests/snapshots/snapshots/*.snap` |
| Agent smoke test | `scripts/agent_smoke_test.sh` |
| Frozen agent artifacts | `agent_baseline/`, `ROBOT_MODE_EXAMPLES.jsonl`, `CLI_SCHEMA.json` |
