# Upstream research: Steve Yegge's "beads" (`bd`) — the Go project that `obr` ports

Research date: **2026-08-06**. Agent: `beads-upstream` (web research leg).

Every nontrivial claim below carries either a source URL or a `path:line` citation into
`/Users/johnw/src/obr` (read-only). Where a source was unreachable I say so explicitly rather
than guessing.

---

## 0. Executive summary (read this first)

1. **The upstream repo moved.** `github.com/steveyegge/beads` now redirects to
   **`github.com/gastownhall/beads`** (GitHub API `repos/steveyegge/beads` returns
   `"full_name":"gastownhall/beads"`). Docs moved from `steveyegge.github.io/beads`
   (now **404**) to **https://beads.gascity.com**.
2. **The architecture `obr` ports no longer exists upstream.** The SQLite + JSONL + daemon
   "classic beads" design was replaced by **Dolt** (a versioned SQL database) between
   **v0.50.0 (2026-02-14)** and **v0.58.0 (2026-03-02)**. JSONL is now demoted from
   "source of truth" to "an export for viewers and interchange."
3. **But SQLite is coming back upstream** — the unreleased changelog (post-v1.1.2) states
   "embedded Dolt, Dolt server, and **SQLite** are the supported storage paths," behind a
   capability-gated pluggable storage seam. That seam was itself validated by a
   *Rust* spike (`bts-rs`) inside the beads project.
4. **`obr` conforms to `bd` v0.46.0** (released 2026-01-06/07), the last widely-used classic
   release — see the 13 `INTENTIONAL DIVERGENCE` markers in
   `/Users/johnw/src/obr/tests/conformance.rs`.
5. **Upstream evolves extremely fast**: ~10,336 commits in the last 52 weeks, 26.1k stars,
   484 open issues, 1,923 issues filed total, latest release v1.1.2 (2026-07-26).
6. **The dominant public criticism** is not of the idea but of the execution: AI-generated
   docs, runaway scope, instability/daemon sync bugs, and "landing the plane" jargon. The core
   idea (structured, queryable, dependency-aware task graph as agent memory) is widely praised,
   including by Simon Willison.

---

## 1. Identity, provenance, and current state of the upstream project

| Fact | Value | Source |
|---|---|---|
| Canonical repo | `gastownhall/beads` (was `steveyegge/beads`) | `gh api repos/steveyegge/beads` → `"full_name":"gastownhall/beads"` |
| Description | "Beads - A memory upgrade for your coding agent" | same |
| Homepage / docs | https://beads.gascity.com | same (`homepage` field) |
| Created | 2025-10-12T03:09:46Z | same |
| Last push (as of research) | 2026-08-06T14:03:26Z (same day) | same |
| Stars / forks | 26,087 / 1,750 | same |
| Open issues (incl. PRs) | 484 | same |
| Total issues ever filed | 1,923 | `gh api search/issues?q=repo:gastownhall/beads+type:issue` |
| License | MIT | same |
| Language | Go (22.2 MB Go; also 356 KB Python, 353 KB Shell, **54 KB Rust**) | `gh api repos/gastownhall/beads/languages` |
| Commits on main | 10,341 | https://github.com/gastownhall/beads |
| Commits in last 52 weeks | 10,336 (~199/week) | `gh api repos/gastownhall/beads/stats/participation` |
| Contributors | 100+ (API page cap); `steveyegge` 4,750 commits, `maphew` 754, `coffeegoddd` 616 (a DoltHub engineer), `julianknutsen` 166 | `gh api repos/gastownhall/beads/contributors` |
| Latest release | **v1.1.2**, 2026-07-26 | `gh api repos/gastownhall/beads/releases/latest` |

**Note on `coffeegoddd`**: a top-3 contributor; DoltHub staff involvement is consistent with
Yegge's essay line that "The Dolt and Gas City teams (both Beads co-maintainers with me) are
doing amazing work here" (https://yegge.ai/essays/the-shape-of-things-to-come/).

### 1.1 The rename is a live source of breakage upstream

Open issue **#3059, "Migration gap: ~400+ references to steveyegge/* remain after repo move to
gastownhall"** (10 comments) — https://github.com/gastownhall/beads/issues/3059. This matters to
`obr`, which still points users at the old org:

- `/Users/johnw/src/obr/tests/bench_real_datasets.rs:584` → `"Install bd from: https://github.com/steveyegge/beads"`
- `/Users/johnw/src/obr/tests/common/binary_discovery.rs:57` → same install URL
- `/Users/johnw/src/obr/.beads/README.md:9,64,75,77` → four `steveyegge/beads` URLs, including a
  `curl | bash` of `raw.githubusercontent.com/steveyegge/beads/main/scripts/install.sh`

These still resolve today via GitHub's rename redirect, but raw.githubusercontent redirects are
not guaranteed and the docs paths they cite (`docs/FAQ.md`, `docs/` tree) have been
**restructured** upstream (see §4.1), so several are already dead links in spirit.

---

## 2. Origin story and motivation ("memory for agents")

### 2.1 The launch essay

**"Introducing Beads: A coding agent memory system"** — Steve Yegge, Medium, ~2025-10-13.
https://steve-yegge.medium.com/introducing-beads-a-coding-agent-memory-system-637d7d92514a

Core argument as extracted from the post:

- **Provenance**: Yegge had built `vibecoder`, an agent-orchestration project of ~350k lines,
  which failed on two architectural mistakes: over-reliance on Temporal, and a markdown-based
  master-plan system. His pivot: *"Screw it. Let's just move all known work from the plans into
  an issue tracker."*
- **The amnesia framing**: agent sessions last ~10 minutes and start blank. *"It's the movie
  Memento in real life, or Fifty First Dates."* Agents only know "whatever video cassette they
  find in the recorder when they wake up."
- **Why markdown plans fail**: an agent decomposes a task into six phases, gets compacted/
  restarted, reaches phase 3, and re-declares a *new* six-phase plan nested inside the old one,
  unlabeled. He reports finding **"six hundred and five markdown plan files in varying stages of
  decay."** A centralized "PlanStore" attempt required ripping out **"70 thousand lines of
  plan-management code"** — he calls it a "fatal design flaw."
- **The fix**: `bd`, a CLI issue tracker agents drive directly. *"you install the `bd` tool,
  point your `AGENTS.md` or `CLAUDE.md` at it with one line."*
- **Schema origin**: prototyped in TypeScript/PostgreSQL, built by Claude in "ultrathink" in
  about "twelve minutes." Yegge's only schema constraint was support for "parent/child pointers
  (for epics) and blocking-issue pointers." Claude then invented **four** dependency link kinds.
- **Storage rationale**: JSONL committed to git rather than a DB server gives *"the best of both
  the database and the version-control worlds: queries and versioning."*
- **Discovered work** is the differentiator he emphasizes: agents file issues for problems they
  stumble on instead of dismissing them as "pre-existing," using a **`discovered-from`** link type.
- Quotes from the appendix (Claude's own assessment) that state the design intent bluntly:
  markdown plans are *"write-only memory for agents"*; *"I'm not interpreting text; I'm querying
  structured data"*; *"I run `bd ready --json` and get a definitive list of unblocked work"*;
  git-versioned markdown gives only *"line-level changes, not semantic 'status changed from
  `open` to `in_progress`'"*; and the summary framing: *"these aren't features bolted onto a
  human tool. They're primitives for agent cognition."*

### 2.2 The follow-up essay

**"The Beads Revolution: How I Built The TODO System That AI Agents Actually Want to Use"** —
Medium, ~2025-10-15.
https://steve-yegge.medium.com/the-beads-revolution-how-i-built-the-todo-system-that-ai-agents-actually-want-to-use-228a5f9be2a9

- Built in **six days**, "from an idea I was discussing with Claude on Wednesday morning" to a
  public repo with "a thousand stars and ~fifty forks."
- *"I vibe-coded this whole project, from design, to implementation, to validation and testing,
  to publishing it."*
- At the time: **"only about 15k lines of code."** (Contrast §7.2, where HN commenters cite
  240,000 lines a few months later.)
- Three named defects of markdown plans: *"Markdown plans are text, not structured data"*;
  *"They're not queryable"*; *"Agents rarely update the plans as they work"* → they "bit-rot very
  fast."
- Explicit on git-as-database self-healing: *"the actual database is a JSONL file that's checked
  into your git project"* so *"you never actually lose anything"* — illustrated with a transcript
  where an agent loses issues (180 → 72), over-imports to "405 issues" / "11,286 updated," and
  finally restores "193 clean issues from the remote." He calls this "self-healing."
- Positions beads against `git-bug` and Radicle as those being "too heavyweight" for "fast
  agentic workflow loops."

### 2.3 Yegge's current framing (2026)

**"The Shape of Things to Come, Part 1: The Continuous Thunderdome"** — https://yegge.ai/essays/the-shape-of-things-to-come/
(published ~August 2026 per the page).

Verbatim passages about beads:

- *"Beads is an issue tracker, knowledge graph, and brain-builder for the agentic era."*
- *"Nothing else is as fast, as useful, nor as beloved by the agents themselves."*
- *"Beads is the magic sauce for building modern orchestrators."*
- *"Gas Town was nothing but a Beads machine."* / *"Gas City, also a Beads machine."* /
  *"Wheelhouse... is yet another Beads machine."*
- *"Beads is unfortunately still a bit janky, because its unique work footprint strains
  databases"* — and *"Companies figure out Beads and suddenly All the World's a Bead."*
- *"Beads can be both a database and a Git ledger."*
- *"Beads **is** a graph, one that includes dependency and parent/child edges."*
- *"it handles atomic claiming, leasing, gates, triggers, and other critical orchestration
  features"*
- *"It is the journal of all the work that ever happened"* … *"you can't usually **boot** from
  beads; that's what the brain is for."* (i.e. beads is deliberately *not* the whole memory —
  a markdown "project brain" still boots the agent.)
- Operationally: *"Wheelhouse runs its Beads on a shared Dolt server, backed by GCS"* and
  *"My Beads DB is still a bit cranky, what with 12,000 git commits/day."*
- *"agents burn tokens invisibly, keeping your beads synced, repaired, backed up, etc."*

### 2.4 Talks / podcasts

- **Software Engineering Daily**, "Gas Town, Beads, and the Rise of Agentic Development with
  Steve Yegge," **2026-02-12**, host Kevin Ball —
  https://softwareengineeringdaily.com/2026/02/12/gas-town-beads-and-the-rise-of-agentic-development-with-steve-yegge/
  **Transcript not retrievable via WebFetch** (only page metadata returned); I did not extract
  quotes. Framing per the page: "task graphs and Git-backed ledgers change the nature of work."
  Also on HN: https://news.ycombinator.com/item?id=46994362 (1 point, 0 comments).
- Yegge's site indexes Talks and Podcasts pages (https://yegge.ai/) but no dedicated Beads page
  (https://yegge.ai/beads → **404**).

---

## 3. The "classic" architecture — exactly what `obr` ports

This section is the most load-bearing for `obr`, so it is sourced from upstream docs **pinned at
tag `v0.46.0`** (2026-01-06), the version `obr`'s conformance suite targets.

Retrieved via `gh api repos/gastownhall/beads/contents/docs/<F>.md?ref=v0.46.0`; local copies at
`/private/tmp/claude-501/-Users-johnw-src-obr/f80d1967-0fc0-44fa-a53f-3054d39727e2/scratchpad/bd046/`.

### 3.1 The three-layer data model

`docs/ARCHITECTURE.md@v0.46.0`
(https://github.com/gastownhall/beads/blob/v0.46.0/docs/ARCHITECTURE.md):

```
CLI layer  →  SQLite (.beads/beads.db, gitignored, per-machine)
           →  auto-sync, 5s debounce
           →  JSONL (.beads/issues.jsonl, git-tracked SOURCE OF TRUTH)
           →  git push/pull  →  remote
```

Stated rationale (verbatim):
- *"**SQLite for speed:** Local queries complete in milliseconds."*
- *"**JSONL for git:** One entity per line means git diffs are readable and merges usually
  succeed automatically. No binary database files in version control."*
- *"**Git for distribution:** No special sync server needed. Issues travel with your code."*

**Write path**: command → SQLite write (immediate) → mark dirty → 5s debounce → incremental
JSONL export → optional git hook commit.

**Read path**: `git pull` → next `bd` command auto-detects JSONL newer than DB → import/merge by
content hash → query SQLite.

Critical detail: *"Hash-based comparison (not mtime) prevents git pull false positives (issue
bd-84)"* — `docs/INTERNALS.md@v0.46.0`. `obr`'s
`/Users/johnw/src/obr/EXISTING_BEADS_STRUCTURE_AND_ARCHITECTURE.md` records the same
reconciliation task ("Reconciled **staleness detection vs auto-import** semantics (mtime/Lstat vs
content hash)").

### 3.2 Import merge rule (content-addressed dedup)

From `ARCHITECTURE.md@v0.46.0`:

> For each issue in JSONL: 1. Compute content hash 2. Look up existing issue by ID 3. Compare
> hashes: Same hash → skip; Different hash → update (newer version); No match → create.

And: *"Same ID + different content = update, same ID + same content = skip."* Internal fields
carrying `json:"-"` (`content_hash`, `source_repo`, `id_prefix`) are **never exported to JSONL**.

### 3.3 IDs

- IDs derive from a **random UUID**, not from content: *"Issue creation: Generate random UUID,
  derive short hash as ID"* (`ARCHITECTURE.md@v0.46.0`). The current docs muddy this by saying
  IDs are generated from "Issue title / Creation timestamp / Random salt"
  (https://beads.gascity.com/core-concepts/hash-ids) — the v0.46 statement is the one a
  conforming port should follow.
- **Adaptive length** (`docs/ADAPTIVE_IDS.md@v0.46.0`, feature `bd-ea2a13`, "Implemented v0.21+"):
  birthday-paradox sizing `P(collision) ≈ 1 - e^(-n²/2N)`, `N = 36^length`.
  Defaults: `max_collision_prob = 0.25`, `min_hash_length = 4`, `max_hash_length = 8`.
  Thresholds: 0–500 issues → 4 chars (~7% at 500); 501–1500 → 5 chars (~2% at 1500); 1501+ →
  6 chars. Config lives in the SQLite `config` table.
- **Collision resolution**: try base length, base+1, base+2, **10 nonces per length = 30 attempts**.
- **Hierarchical child IDs**: `bd-a3f8`, `bd-a3f8.1`, `bd-a3f8.1.1` — up to **3 levels**
  (https://beads.gascity.com/core-concepts/hash-ids).

### 3.4 Dependency types and blocking semantics (v0.46 — 4 types)

`ARCHITECTURE.md@v0.46.0`:

| Type | Semantic | Affects `bd ready`? |
|---|---|---|
| `blocks` | X must close before Y starts | **Yes** |
| `parent-child` | epic/subtask | **Yes** (children blocked if parent blocked) |
| `related` | soft link | No |
| `discovered-from` | found during work on parent | No |

Blocking rule (`INTERNALS.md@v0.46.0`, "Blocked Issues Cache (bd-5qim)"):

> An issue is blocked if: 1. **Direct blocking**: has a `blocks` dependency on an
> open/in_progress/blocked issue. 2. **Transitive blocking**: parent is blocked and issue is
> connected via `parent-child`. Closed issues never block others. Related and discovered-from
> dependencies don't affect blocking.

Implementation notes worth mirroring: a materialized `blocked_issues_cache` table replaced a
recursive CTE (752 ms → 29 ms on 10k issues), **fully rebuilt** (DELETE+INSERT) inside the same
transaction as the triggering change, invalidated **only** by `blocks`/`parent-child` dep add or
remove, `UpdateIssue` status change, and `CloseIssue`. Parent-child propagation is depth-limited
to **50**.

### 3.5 Statuses, types, priorities, and JSONL field set (v0.46)

`ARCHITECTURE.md@v0.46.0` "JSONL Issue Schema":

- `status` ∈ `open`, `in_progress`, `blocked`, `deferred`, `closed`, `tombstone`, `pinned`,
  `hooked` (default `open`).
- `issue_type` ∈ `bug`, `feature`, `task`, `epic`, `chore`, `message`, `merge-request`,
  `molecule`, `gate`, `agent`, `role`, `convoy` (default `task`).
- `priority` int 0–4, 0 = critical, 4 = backlog.
- Content fields: `title` (required), `description`, `design`, `acceptance_criteria`, `notes`.
- Assignment: `assignee`, `estimated_minutes`.
- Timestamps: `created_at`, `created_by`, `updated_at`, `closed_at`, `close_reason` (all RFC3339).
- External: `external_ref` (e.g. `gh-9`, `jira-ABC`).
- Relational, embedded in the issue line: `labels []string`, `dependencies []Dependency`,
  `comments []Comment`.
- Tombstone: `deleted_at`, `deleted_by`, `delete_reason`, `original_type`.
- Status flow: `open → in_progress → closed`, with reopen back to `open`.
- Core types: `Issue`, `Dependency` (FromID/ToID/Type), `Label` (Name/Color/Description),
  `Comment` (IssueID/Author/Content/Timestamp), `Event` (audit trail).

**Verified against real data**: the first line of `.beads/issues.jsonl` at tag `v0.46.0` is a
single flat issue object keyed on `id`, sorted by ID, with no envelope or header record —
byte-shape-compatible with `/Users/johnw/src/obr/.beads/issues.jsonl`.

### 3.6 Deletions / tombstones

`docs/DELETIONS.md@v0.46.0`:

- Deletion converts the issue to `status: "tombstone"` **in place, inline in `issues.jsonl`** —
  no separate manifest. Rationale given: "Atomic sync with issue data (no separate manifest to
  merge)" and "Proper 3-way merge conflict resolution."
- On delete: record `deleted_at`/`deleted_by`/`delete_reason`, preserve `original_type`, and
  **remove all dependencies** ("tombstones don't block anything").
- **TTL default 30 days** (`tombstone.ttl_days` in `.beads/config.yaml`), + 1 hour grace;
  `bd admin compact` prunes expired tombstones from the JSONL; git history is the fallback.
- Commands: `bd delete <id>` previews by default; `--force`, `--cascade`, `--from-file`,
  `--dry-run`. `bd list --status=tombstone`.

### 3.7 The JSONL merge driver (this is a *format contract*, not just tooling)

`docs/GIT_INTEGRATION.md@v0.46.0`, "Intelligent Merge Driver (Auto-Configured)":

- Auto-configured by `bd init` since v0.21:
  `git config merge.beads.driver "bd merge %A %O %A %B"` plus a `.gitattributes` line
  `.beads/issues.jsonl merge=beads`.
- Algorithm is **beads-merge by @neongreen**, vendored into `bd`; standalone binary lives at
  `github.com/neongreen/mono/beads-merge`.
- Semantics: **field-level 3-way merge**; issues **matched by identity = `id` + `created_at` +
  `created_by`**; timestamps merge by **max**; dependencies merge by **union**; status/priority
  by 3-way merge; conflict markers only for genuinely unresolvable fields.
- `bd import` **rejects** files containing git conflict markers.
- A `jj` (Jujutsu) merge-tool recipe is documented, with
  `merge-conflict-exit-codes = [1]`.

This is the single most under-appreciated compatibility surface: the identity triple
(`id`+`created_at`+`created_by`) and the max/union field rules are what make two independently
written JSONL files converge.

### 3.8 Daemon, wisps, and other things `obr` deliberately drops

- **Daemon**: one per workspace, Unix socket `.beads/bd.sock` (named pipes on Windows), RPC
  protocol in `internal/rpc/protocol.go`, auto-starts unless `BEADS_NO_DAEMON=1`; CLI tries
  daemon first and falls back to direct DB. (`ARCHITECTURE.md@v0.46.0`)
- **FlushManager**: event-driven single-owner goroutine replacing timer+shared-state; channels
  `markDirtyCh` (buffer 10), `timerFiredCh` (1), `flushNowCh` (1), `shutdownCh` (1); default
  debounce 5s; JSONL hash validated before each flush and a mismatch forces a **full re-export**
  (issue bd-160). (`INTERNALS.md@v0.46.0`)
- **Export modes**: incremental (only rows in the `dirty_issues` table, merged into existing
  JSONL) vs full (rebuild from scratch; required after ID-changing ops like `rename-prefix`).
- **Wisps**: ephemeral molecule-step issues that live **only in local SQLite**, are **never
  exported to JSONL**, and are **hard-deleted** (not tombstoned) on `bd mol squash`.
- **Directory layout** (`ARCHITECTURE.md@v0.46.0`): `.beads/{beads.db, issues.jsonl, bd.sock,
  daemon.log, config.yaml, export_hashes.db}`.

`obr` explicitly rejects the daemon and all implicit git behavior — see
`/Users/johnw/src/obr/PLAN_TO_PORT_BEADS_WITH_SQLITE_AND_ISSUES_JSONL_TO_RUST.md:16-24`
("No automatic git hook installation … No daemon/RPC architecture … Explicit over implicit").

---

## 4. The Dolt pivot — timeline, and what it changed

Sourced from `CHANGELOG.md` on `main`
(https://github.com/gastownhall/beads/blob/main/CHANGELOG.md) and release metadata.

| Version | Date | Storage-relevant event |
|---|---|---|
| v0.46.0 | 2026-01-06 | Classic SQLite+JSONL+daemon. (`obr`'s conformance target.) |
| v0.50.0 | **2026-02-14** | *"Dolt is now the default backend for new `bd init` projects. Existing SQLite projects are unaffected. `BEADS_DB` env var auto-detects SQLite."* |
| v0.51.0 | 2026-02-16 | *"Dolt-native cleanup (dolt-1s40) — massive 8-phase refactoring to remove all legacy SQLite/JSONL/daemon infrastructure … Phase 6: Remove SQLite backend entirely … Phase 8: CLI & config cleanup"* |
| v0.58.0 | **2026-03-02** | Removed: *"**Beads Classic SQLite backend** — the SQLite storage layer and all migration infrastructure have been removed. Dolt is the only backend."*; *"`go-sqlite3` dependency — no more CGO requirement; tests use Dolt testcontainers"*; `bd migrate --to-dolt` removed. |
| v1.0.0 | 2026-04-02 | First stable. |
| v1.1.0 | 2026-07-04 | Migration guards, sync repair, compaction archive-before-discard, import stale-guard inside transactions, `--allow-stale`. |
| v1.1.2 | 2026-07-26 | Latest release. |
| [Unreleased] | after 2026-07-16 | **SQLite returns** as a supported storage path (§4.3). |

### 4.1 What the Dolt design looks like now

- https://beads.gascity.com/architecture/dolt — two modes:
  **Embedded** (default; Dolt in-process; data in `.beads/embeddeddolt/`; single writer,
  file-locked) and **Server** (`bd init --server`; external `dolt sql-server`; data in
  `.beads/dolt/`; concurrent writers; default port 3307, `BEADS_DOLT_SERVER_*` env vars, Unix
  socket option).
- Cross-machine sync is **`bd dolt push` / `bd dolt pull` against `refs/dolt/data` on the git
  remote** — issue history rides in the same repo as code but on a separate ref namespace.
  `bd init` auto-detects `git remote get-url origin`; fresh clones run `bd bootstrap`.
  (https://beads.gascity.com/core-concepts/sync-concepts)
- **JSONL's new status, verbatim**: *"`.beads/issues.jsonl` is an export. It exists for viewers,
  interchange, migration, and backup. It is not the canonical cross-machine sync channel. Do not
  use routine `bd import .beads/issues.jsonl` as a replacement for `bd dolt pull`. JSONL import
  is upsert-only; it cannot infer that records absent from an export were deleted, pruned, or
  simply never exported."* (same page)
- Justification (https://beads.gascity.com/reference/faq): version-controlled SQL, **cell-level
  merge**, native push/pull, multi-writer, portability via `bd export`.
- **Schema version guard** in the binary: e.g. `schema version mismatch: database is at v45,
  binary knows up to v42 (3 migrations ahead)`. Schema is at **v53+** by v1.1.0 (changelog
  mentions "repair v53 wisp dependency drift").
- Maintenance verbs are new: `bd prune` (delete old closed beads; reference-aware — skips beads
  whose ID appears in the description/notes/comments of any open bead), `bd purge` (ephemeral),
  `bd gc`, `bd flatten`, `bd backup init|sync|restore`, `bd admin compact`.

### 4.2 Semantics that drifted since v0.46 (the parts a conforming port would have to choose about)

**Dependency types expanded from 4 to 10** (https://beads.gascity.com/core-concepts/dependencies):

*Blocking*: `blocks` (default), `parent-child`, **`conditional-blocks`** ("B runs only if A
fails"), **`waits-for`** ("B waits for all of A's children").
*Non-blocking*: `related`, **`tracks`**, `discovered-from`, **`caused-by`**, **`validates`**,
**`supersedes`**. The current README also advertises `duplicates` and `replies-to` for
knowledge-graph use.

Other drift:
- **Cross-repo dependencies**: `bd dep add local-issue external:other-project:remote-issue`;
  *"External dependencies always block"*, checked at query time.
- **Cross-type blocking now allowed** ([Unreleased], bd-wg7ve, PR #4034): the old blanket
  same-type rule (GH#1495) that made `bd dep add <task> <epic>` fail is replaced by a
  "hierarchy deadlock guard" rejecting only ancestor/descendant self-gating.
- **Cycles rejected at write time**: *"Beads also rejects cycles at write time — `bd dep add`
  checks for cycles before committing"*; plus `bd dep cycles`.
- **Gates** — a new issue type that blocks on external conditions: `gh:pr`, `gh:run`, `timer`,
  `bead` (cross-rig), `human`; `bd gate check|discover|resolve|list|show`. The docs are explicit
  that gates exist *because* Dolt broke the atomicity JSONL had: *"With file-based storage
  (JSONL), issue updates land atomically with code in the same commit. With Dolt, they don't."*
  **This is a direct architectural argument in `obr`'s favor** for the JSONL model.
- **Atomic claim**: `bd update <id> --claim` sets assignee + `in_progress` in one write.
  Docs recommend `--claim` over `bd assign` for self-service agents.
- **Molecules / wisps / formulas**: `bd cook` compiles a TOML/JSON formula → proto; `bd mol pour`
  instantiates a persistent molecule; `bd mol wisp` instantiates ephemeral ones.
- **Memories**: `bd remember "insight"` / `bd prime`; memory records are a second record kind in
  the JSONL export.

**JSONL interchange format drift** (https://beads.gascity.com/reference/json-schema):
export lines are now discriminated by **`_type`** (`"issue"` / `"memory"`), preceded by an
optional header record **`{"_schema":"beads-jsonl/1"}"`** that readers skip, and issue records may
carry **`wisp_plane`** (a boolean routing marker that supersedes the v0.35–v0.37 `wisp` key,
which is retained as a read-side legacy alias). `obr` has **no handling** for `_type`, `_schema`,
`memory`, or `wisp_plane` — `grep -rn '"_type"\|_schema\|wisp_plane\|"memory"' src/sync/ src/model/`
returns nothing relevant.

**JSON output envelope** (same page): all `--json` commands can wrap output as
`{"schema_version": 1, "data": <payload>}` when `BD_JSON_ENVELOPE=1`, and *"This will become the
default format in v2.0"*, with `BD_JSON_ENVELOPE=0` as a one-release escape hatch. Today's
default (legacy) mode: object commands emit `schema_version` as a sibling top-level field; list
commands emit a **raw JSON array**; errors go to **stderr** as
`{"schema_version":1,"error":...,"code":...}`. Also documented: `bd show --json` returns
`comment_count` only unless `--include-comments`, plus an optional `comments_omitted` boolean.

### 4.3 SQLite is returning upstream (important, and easy to miss)

The `[Unreleased]` section of `CHANGELOG.md` on `main` (entry dated by a 2026-07-16 supersede
note) states, verbatim:

> **Storage backend scope simplified** (bd-sadcd). The recently merged direct PostgreSQL and
> MySQL adapters have been rolled back before entering a tagged release. … The storage interface,
> shared issue core, **SQLite implementation**, and conformance harness remain; **embedded Dolt,
> Dolt server, and SQLite are the supported storage paths.**

Design record: `PROPOSAL-pluggable-storage-backends.md` at the repo root
(https://github.com/gastownhall/beads/blob/main/PROPOSAL-pluggable-storage-backends.md),
dated 2026-07-02, "Status: Historical — partially implemented, then superseded on 2026-07-16."
Highlights:

- End state: `bd init --backend=dolt|sqlite|postgres`, choice recorded in the workspace locator.
- *"**One core command set** (the gc-contract 16 plus the portable CORE inventory, ~67 commands)
  behaves identically on every backend, proven by a differential conformance harness — not by
  code review."*
- *"**Storage-specific commands are capability-gated addons.** `bd history`, `bd dolt push/pull`,
  `bd vc`, `bd branch`, `bd diff`, `bd federation`, `bd backup`, `bd sql` are live only when the
  opened store advertises the matching capability … Pick SQLite → you get a fast, zero-dep,
  pure-Go single-writer store and those commands stub out."*
- **The seam was validated by a Rust reimplementation.** *"Informed by: the bts-rs Rust spike
  (a from-scratch Rust reimplementation of the gc-contract surface that validated a
  backend-agnostic storage seam at 299/299 byte-parity across two radically different
  backends)"*; *"Six small core traits (33 data methods) + optional capability traits reached via
  accessors defaulting to 'absent'."* It even notes Rust's non-object-safe async traits forced a
  399-line delegation enum that Go gets for free.

**Caveat / apparent contradiction:** the *published* CLI reference on `main` still says the
opposite — `docs/cli-reference/init.md`: *"Dolt is the default (and only supported) storage
backend. The legacy SQLite backend has been removed. Use `--backend=sqlite` to see migration
instructions,"* and the flag help reads `--backend string … --backend=sqlite prints deprecation
notice.` The `docs/architecture/storage-backends.md` that the changelog links **does not exist**
in `main` (404 via API; `docs/architecture/` contains only `dolt.md` and `index.md`). So: SQLite
support is landed-but-unreleased and not yet documented. Treat as "in flight," not shipped.

Related live issue: **#4547 "Proposal: pluggable storage backends — one seam, capability-gated
commands, Postgres first"** (5 comments) — https://github.com/gastownhall/beads/issues/4547.

---

## 5. Documentation structure today (for anyone re-deriving semantics)

Docs site: **https://beads.gascity.com** (the old `steveyegge.github.io/beads` is **404**).
Source under `docs/` in the repo, now lowercase and sectioned (v0.46 used `SHOUTING_CASE.md` at
the top level — a specific complaint in issue #376):

- `core-concepts/`: `issues.md`, `dependencies.md`, `hash-ids.md`, `adaptive-ids.md`,
  `graph-links.md`, `labels.md`, `metadata.md`, `sync-concepts.md`
- `architecture/`: `dolt.md`, `index.md`
- `reference/`: `faq.md`, `json-schema.md`, `configuration.md`, `git-integration.md`,
  `advanced.md`, `observability.md`, `protected-branches.md`, `worktrees.md`, `troubleshooting.md`,
  `antivirus.md`
- `workflows/`: `formulas.md`, `gates.md`, `molecules.md`, `wisps.md`, `todo.md`
- `multi-agent/`: `coordination.md`, `federation.md`, `routing.md`, `multi-repo-migration.md`
- `getting-started/`, `integrations/` (19 editors/agents: claude-code, cursor, aider, codex,
  copilot, gemini, kiro, windsurf, cody, kilocode, junie, factory, mux, opencode, azure-devops,
  mcp-server, …), `recovery/` runbooks, `cli-reference/` (auto-generated, ~90 pages, pinned by
  `docs/cli-docs.pin`), `community-tools.md`, `related-projects.md`
- Engineering docs live in `engdocs/` (e.g. `COLLISION_MATH.md`).

Roadmap policy, verbatim from the FAQ: *"The roadmap lives in beads itself: `bd list
--priority-max 1 --json`."* There is no separate roadmap document.

---

## 6. Community, forks, and the ecosystem `obr` sits in

- **`Dicklesworthstone/beads_rust`** — the direct ancestor of this repo. Its README (mirrored at
  `/Users/johnw/src/obr/agent_baseline/README_first_80_lines.md:16`) describes itself as
  *"A Rust port of Steve Yegge's beads, frozen at the 'classic' SQLite + JSONL architecture I
  built my Agent Flywheel tooling around."* Line 36 states the reason plainly: *"The hybrid
  SQLite + JSONL-git architecture that I built my tooling around … is being replaced with
  approaches better suited to Steve's vision."* Line 38: *"The command is `br` to distinguish it
  from the original `bd`."* Line 40: *"Steve has given his full endorsement of this project."*
  HN post: https://news.ycombinator.com/item?id=46674515
  (2026-01-19, 3 points, **0 comments** — it got no traction).
- **Community tools** upstream (https://github.com/gastownhall/beads/blob/main/docs/community-tools.md)
  lists dozens of viewers/TUIs/kanban UIs. Several are Rust-flavored (`bsv`, and
  Tauri-based `beads-web`, `beads-kanban-ui`). **`beads_rust`/`br` is not listed there** in the
  version I read — the Rust port is not part of upstream's advertised ecosystem.
- **Adjacent-project doc** (https://github.com/gastownhall/beads/blob/main/docs/related-projects.md)
  notes `scry` "independently arrived at the same hash-based-ID convention (`bd-a1b2`, `~hash`)
  for the same reason."
- **Competitors/replacements spawned by beads fatigue**: `wedow/ticket` (bash + flat markdown),
  `hmans/beans`, `MrLesk/Backlog.md`, `obsfx/trekker`, `sandover/ergo`, `beadhub.ai` (Postgres,
  server-backed, agent-to-agent chat), `nmelo/bdui`, `Dicklesworthstone/beads_viewer`.
- **Anthropic influence**: reported (https://paddo.dev/blog/from-beads-to-tasks/) that an
  Anthropic engineer said "We took inspiration from projects like Beads by Steve Yegge" when
  discussing Claude Code's task coordination. I did **not** independently verify the original
  tweet.

---

## 7. Reception and criticism

### 7.1 Discussion venues and sizes

| Venue | Item | Date | Score / comments |
|---|---|---|---|
| HN | "Beads: A coding agent memory system" https://news.ycombinator.com/item?id=45566864 | 2025-10-13 | 19 pts / 1 |
| HN | **"Beads – A memory upgrade for your coding agent"** https://news.ycombinator.com/item?id=46075616 | 2025-11-28 | **111 pts / ~40** |
| HN | "Show HN: I replaced Beads with a faster, simpler Markdown-based task tracker" https://news.ycombinator.com/item?id=46487580 | 2026-01-04 | **84 pts / ~45** |
| HN | "Show HN: Beads Viewer (Bv)" https://news.ycombinator.com/item?id=46130500 | 2025-12-03 | 4 pts / 0 |
| HN | "A fast Rust port of Steve Yegge's beads" https://news.ycombinator.com/item?id=46674515 | 2026-01-19 | 3 pts / 0 |
| HN | SE Daily podcast https://news.ycombinator.com/item?id=46994362 | 2026-02-12 | 1 pt / 0 |
| lobste.rs | "beads: A memory upgrade for your coding agent" (~alper) | 2025-10-14 | 3 pts / **0 comments** |

(HN data pulled via `hn.algolia.com/api/v1`. Direct `news.ycombinator.com` WebFetch returned
**HTTP 429**; comment trees were reconstructed from the Algolia items API instead.)

Note: lobste.rs essentially ignored beads; the substantive discussion is all on HN and GitHub.

### 7.2 The praise

- **Simon Willison** (HN 46075616): *"That's exactly what this is, but it's one that's designed
  with coding agents in mind as its principle users."* And on why it works: *"Beads is a much
  more structured way of achieving the same thing. I expect it works well partly because LLM
  training data makes them familiar with the issue/bug tracker style of working already."* He
  also singles out *"the way it uses the birthday paradox to decide when to increase the length
  of the hash IDs."*
- `adamgordonbell`: *"It doesn't compete with gh issues as much as it competes with markdown
  specs."*
- `qudat`: *"I've been using beads for a few projects and I find it superior to spec kit or any
  other form of structured workflow … the agent doesn't need to hold so much context."*
- `iand675` on the real pain being solved: *"Ending up with loads of markdown poop feels like the
  new `.DS_Store`, but harder to `.gitignore`."*
- `jauntywundrkind`: *"Somewhat aside but I love the data architecture. Jsonl lines checked into
  git, and a sqlite local cache."* — praise aimed squarely at the architecture `obr` preserves.
- `pbw`: *"it's turning the corner from 'teaching AIs to develop using tools that were designed
  for humans' to 'inventing new tools and techniques that are designed specifically for AI use'."*

### 7.3 The criticism (thematically grouped)

**(a) AI-generated docs / hype.** The canonical thread is upstream issue
**#376, "I want to love Beads but the AI generated docs make it impossible"**
(https://github.com/gastownhall/beads/issues/376), opened 2025-11-25 by `srobertson`:

> *"the bold, overhyped claims (yes, I can spot Claude-style autogenerated docs when I see them)
> are getting in the way of addressing real bugs. I'm using worktrees and have followed all the
> instructions … and I still end up with merge conflicts, a locked database, and lost issues.
> … what's missing is an architectural overview of the data model. If there were a clear
> explanation of how JSON, SQL, and Git are supposed to work together, you'd get contributors …
> can we please have just one human-edited document that explains the data interaction model
> between Git, JSON, and SQLite?"*

Yegge's reply, same day, opens with *"You're right to call me out on this. :)"* and commits to
(1) rename ARCHITECTURE.md → INTERNALS.md, (2) write a real ARCHITECTURE.md covering the data
model, (3) add recovery procedures. **He did all three** — that split is exactly the
`ARCHITECTURE.md` / `INTERNALS.md` pair I quote in §3, and `docs/recovery/` runbooks now exist.
This is the single most useful thing that ever happened for anyone porting beads.

A later comment on the same issue (`rgardler-msft`, 2025-12-23) is harsher: *"STOP with the 100%
autogenerated rapid release of 'fixes and features'. START refactoring, remove duplication, and
testing with real users … As for BD Doctor - it's literally a joke."* And a doc-drift inventory
from `luizlf` (2025-12-03): `beads-metadata` vs `beads-sync` branch name mismatch between docs and
code, `bd daemon start` vs `bd daemon --start`, dangling `docs/README.md` links.

The HN thread mirrors this: `mimischi`: *"A classic issue of AI generated READMEs. Never to the
point, always repetitive and verbose."* `themgt` wrote a long comment tracing the README's
"`.beads/beads.jsonl` — Issue data in JSONL format (source of truth …)" to a repo where that file
no longer exists: *"beads.jsonl is the spoon, but there is no spoon."*

**(b) Scope creep / weight.** `bccdee` (HN 46669791, 2026-01-18): *"Beads, which is basically some
sort of issue tracker for agents, is 240,000 li[nes]…"* — versus Yegge's own "about 15k lines" in
October 2025. `shankspeaks`: *"I agree with OP that beads has great primitives, but I think its
become a bit unwieldy in trying to becoming something 'everyone' including larger teams can use."*
`azeirah`: *"Beads is an incredibly difficult-to-follow mess for something that is at its core a
pretty simple idea."* `nmfisher`: *"when I got to the phrase 'landing the plane' I started looking
for simpler alternatives."*

**(c) Reliability, specifically the daemon and auto-sync.** The `wedow/ticket` Show HN post
(2026-01-04) is the sharpest data point because the author was a heavy user:

> *"Steve Yegge released Beads back in October and I found that giving Claude tools for proper
> task tracking was a massive unlock. But Beads grew massively in a short time and every release
> made it slower and more frustrating to use. I started battling it several times a week as its
> background daemon took to syncing the wrong things at the wrong times."*

Same author on workspace discovery: *"One of my pain points with beads was that agents would
sometimes create a bead outside of the correct directory and get dumped into a global
`~/.beads/default.db` and make a mess."*

Other reports: `ac29` (HN 46467414) *"I've used it. I'm …"* (truncated; negative);
`meowface` (HN 46678421) frames Yegge as "not an idiot … possibly just hypomanic";
`SwellJoe` (2026-04-15) on the token economics: *"everything about the whole stack is designed to
burn tokens."* One commenter reported having *"to uninstall Beads after seeing it only confuse my
agents."*

**(d) "Why not just use X?"** Recurrent: GitHub Issues + `gh` CLI (`frodo76`, `jannniii`,
`CuriouslyC`), Taskwarrior (`mbanerjeepalmer`: *"The benefit is most LLMs already understand
Taskwarrior. They've never heard of Beads."*), `git notes` (`arjie`), `git-bug` / `git-issue`
(`khimaros`), self-hosted Forgejo (`xrd`: *"I really don't understand the raison-d'être for beads
other than yak shaving"*). Upstream's FAQ now answers the GitHub Issues and Taskwarrior versions
of this head-on (https://beads.gascity.com/reference/faq).

**(e) The "agents enjoy it" claim.** `thih9`: *"Can a trained LLM develop a preference for a given
tool within some context and reliably report on that? Is 'what AI reports enjoying' aligned with
AI's optimal performance?"* — a fair epistemics objection to the README's marketing.

**(f) A design suggestion worth remembering** (Simon Willison, HN 46075616): store beads data on a
separate orphan `beads` branch so the noisy commit history stays out of `main`. `amonks` gave the
correct counter-argument: *"do you want your beads state to be coupled with commits (eg: checking
out an old commit also shows you the beads state at that snapshot)? Using a separate branch would
decouple this. I think the coupling is a nice feature."* Upstream's answer is
`docs/reference/protected-branches.md` (a `beads-sync` branch mode) and, since the Dolt pivot,
`refs/dolt/data`.

### 7.4 Current open-issue texture (roadmap-by-bug-report)

Newest open issues as of 2026-08-06 (`gh api repos/gastownhall/beads/issues?state=open`):

- #5380 (2026-08-06) migration 0058 cannot apply — FK on a base column of a stored generated column
- #5370 (2026-08-05) `bd note <id>` has the same unguarded id-as-subcommand-typo hole `bd comment` had
- #5362 (2026-08-05) `bd list` reports a false Total when truncating at its default limit;
  `--json` silently ignored without `--flat`; `--limit 0` undocumented on `bd ready`
- #5358 (2026-08-05) `bd ready --label-any` silently ignored in embedded-Dolt mode (still broken in 1.1.2)
- #5347 (2026-08-05) migrations 50–53 drop `DEFAULT (uuid())` on id columns, breaking every
  `bd dep add` with Error 1105

Most-discussed open issues: #4380 (19 comments, Dolt storage drift crashing the migration pass),
#2559 (16, "On system restart, or beads updates, connecting to dolt fails"), #3059 (10, the
org-rename reference debt), #3583 (8, session attribution redesign), #4293 (7, `panic: cannot
create context from nil parent` on full-table scan in embedded Dolt).

**Reading of this**: the bug surface today is overwhelmingly *Dolt-migration and
storage-engine* pain, plus CLI polish. Almost none of it is issue-tracker-semantics pain. That is
a strong signal that the classic SQLite+JSONL core semantics `obr` implements are stable and that
the churn is elsewhere.

---

## 8. Implications and constraints for `obr` (the Rust port)

### 8.1 What "conformance" currently means here

- `obr`'s conformance suite shells out to a real `bd` binary
  (`/Users/johnw/src/obr/tests/conformance.rs`, 13,491 lines) and requires **`MIN_BD_VERSION =
  "0.5.0"`** (`/Users/johnw/src/obr/tests/common/binary_discovery.rs:13`) — but every
  documented divergence is written against **v0.46.0**.
- There are **13** `INTENTIONAL DIVERGENCE` markers. Ten of them are cases where *`bd` v0.46.0 is
  buggy and `obr` is correct*: `list --json` returns an empty array (`conformance.rs:1478`),
  `list --type` empty (`:3056`), `list --status closed --json` **invalid JSON** (`:5737`),
  `list --assignee` empty (`:5803`), `list --limit` empty (`:5872`), `list --status open` invalid
  JSON (`:5924`), `list --status in_progress` invalid JSON (`:5993`), `list --priority` range
  empty (`:6066`), `list --label` invalid JSON (`:6138`), multi-filter `list` invalid JSON
  (`:6207`), `list --sort created` empty (`:6366`). One is an `obr`-only feature (`graph
  --compact`, `:11388`). One is the deliberate non-invasiveness split: *"bd v0.46.0 attempts git
  commit during sync, which fails in non-repo dirs"* (`:12905`).
- **Risk**: `bd` ≥ 0.58 cannot satisfy these tests at all (different backend, different flags,
  `bd init` creates Dolt), and `bd` ≥ v0.50 changes `bd init` semantics. A `MIN_BD_VERSION` of
  `0.5.0` will happily accept a v1.1.2 binary and then fail confusingly. **Consider pinning a
  max version, or an explicit "classic-era" version window (≥0.40, <0.50).**

### 8.2 Semantics upstream would want you to get right (still true in the classic model)

1. **The merge-driver identity triple.** Issues are matched across three-way merges by
   `id` + `created_at` + `created_by`, with timestamps merged by **max** and dependencies by
   **union** (§3.7). If `obr` ships any merge tooling, matching on `id` alone silently diverges
   from `bd merge` and from `neongreen`'s standalone `beads-merge`.
2. **Blocking is exactly two edge types.** `blocks` (direct) and `parent-child` (transitive,
   depth-capped at 50). `related` and `discovered-from` must never enter the blocked cache.
   Closed issues never block. (§3.4)
3. **Import is upsert-by-content-hash**, not last-writer-wins by timestamp: same ID + same hash →
   *skip*, same ID + different hash → *update*. Internal fields (`content_hash`, `source_repo`,
   `id_prefix`) never appear in JSONL. (§3.2)
4. **Staleness detection must use content hash, not mtime** — bd learned this the hard way
   (bd-84), because `git pull` rewrites mtimes on unchanged files. (§3.1)
5. **Tombstones are inline in the JSONL with a 30-day TTL**, dependencies stripped, `original_type`
   preserved; expiry pruning is a separate `compact` step. Getting the TTL grace period wrong
   causes issue resurrection across clones. (§3.6)
6. **Adaptive ID length**: `P ≈ 1 - e^(-n²/2N)`, `N = 36^len`, threshold 0.25, min 4, max 8, 10
   nonces × 3 lengths on collision. IDs come from a **random UUID**, not from title content —
   don't "improve" this into a content-addressed ID, or two agents writing the same title collide.
   (§3.3)
7. **A JSONL hash mismatch must force a full re-export**, not an incremental one (bd-160). (§3.8)

### 8.3 Where `obr` is now *ahead of*, not behind, upstream

- Upstream **re-added a SQLite backend** and explicitly frames it as *"a fast, zero-dep, pure-Go
  single-writer store"* with Dolt-only commands stubbed out (§4.3). `obr` already is that store.
  There may be an opportunity to align `obr`'s storage trait shape with the upstream
  `PROPOSAL-pluggable-storage-backends.md` seam (six core traits, 33 data methods, capability
  traits defaulting to absent), and even with the `bts-rs` spike's conformance harness idea
  (299/299 byte-parity scenarios).
- Upstream's **gates feature exists because Dolt lost the atomicity JSONL gave you**
  (https://beads.gascity.com/core-concepts/dependencies): *"With file-based storage (JSONL), issue
  updates land atomically with code in the same commit. With Dolt, they don't."* `obr` keeps that
  atomicity for free — that is a genuine, citable design advantage worth stating in the README.
- Upstream's loudest field complaints — daemon syncing "the wrong things at the wrong times,"
  agents writing into `~/.beads/default.db`, `bd doctor` being ineffective, DB lock/corruption
  under worktrees — are all consequences of features `obr` deliberately does not have
  (`/Users/johnw/src/obr/PLAN_TO_PORT_BEADS_WITH_SQLITE_AND_ISSUES_JSONL_TO_RUST.md:16-24`).

### 8.4 Interop decisions `obr` should make consciously

- **JSONL interchange drift**: current `bd export` emits an optional `{"_schema":"beads-jsonl/1"}`
  header, `_type`-discriminated `issue`/`memory` records, and a `wisp_plane` marker. `obr` neither
  emits nor tolerates these. At minimum, **`obr`'s importer should skip an unknown `_schema`
  header record and ignore `_type:"memory"` lines instead of erroring** — that is cheap and buys
  one-way readability of modern `bd export` output.
- **JSON envelope**: `bd` v2.0 will make `{"schema_version":1,"data":…}` the default. If `obr`
  claims `--json`/`--robot` compatibility, decide now whether to add an opt-in envelope
  (`BD_JSON_ENVELOPE`-equivalent) or to document `obr` as pinned to the legacy shape.
- **Dependency vocabulary**: upstream now has 10 dep types plus `duplicates`/`replies-to`.
  `obr` should decide whether importing a JSONL row with `--type tracks` / `caused-by` /
  `validates` / `supersedes` / `conditional-blocks` / `waits-for` is a hard error (data loss) or a
  pass-through-as-non-blocking. Pass-through is the safer default; a hard error will bite anyone
  who ever ran a newer `bd` in the same repo.
- **Issue types**: v0.46 already allowed `message`, `merge-request`, `molecule`, `gate`, `agent`,
  `role`, `convoy`, and v0.46.0's changelog added *"Custom type support — Configure custom issue
  types in beads config.yaml (bd-649s)"*. A closed enum in `obr` will reject valid classic data.
- **Naming/branding**: `obr` calls itself `obr` in the README but `br` in
  `agent_baseline/README_first_80_lines.md:1`, and `CLAUDE.md` says the binary is `br`. Upstream
  reserves `bd`. Worth settling before publishing anything user-facing.

### 8.5 Things upstream did that are worth stealing

- **`bd prime`** — prints agent workflow context + memories in one call, explicitly to keep agent
  context cost low. The FAQ argues CLI+hooks costs "a couple thousand tokens versus tens of
  thousands for a full set of MCP tool schemas."
- **`bd update <id> --claim`** — atomic assignee+in_progress, the correct primitive for
  self-service multi-agent work (vs. read-then-write races).
- **Reference-aware `prune`** — refuses to delete a closed bead whose ID is cited in the
  description/notes/comments of any open bead, with `--ignore-references` to override.
- **Import reporting**: v1.1 `import --json` returns `created/updated/skipped/dedup_skipped`,
  `tie_kept_local_ids` (equal-`updated_at` rows where local won), `stale_skipped_ids`,
  `skipped_dependencies` (edges whose target was absent). That tie-break rule — *equal timestamps
  keep the local row* — is a good deterministic convention to copy.
- **A schema-version guard that refuses writable opens of a schema-newer DB**, with a message
  naming both versions and the migration delta.

---

## 9. Source inventory (with retrieval status)

| Source | Status |
|---|---|
| https://github.com/gastownhall/beads (README, main) | OK |
| https://github.com/gastownhall/beads/blob/main/CHANGELOG.md (6,975 lines) | OK |
| https://github.com/gastownhall/beads/blob/main/PROPOSAL-pluggable-storage-backends.md | OK |
| `docs/{ARCHITECTURE,INTERNALS,ADAPTIVE_IDS,DELETIONS,GIT_INTEGRATION,COLLISION_MATH,graph-links}.md` @ **tag v0.46.0** | OK (local copies in scratchpad `bd046/`) |
| https://beads.gascity.com/{core-concepts/*,architecture/dolt,reference/faq,reference/json-schema} | OK |
| https://steveyegge.github.io/beads/ | **404 — site moved** |
| https://steve-yegge.medium.com/introducing-beads-a-coding-agent-memory-system-637d7d92514a | OK |
| https://steve-yegge.medium.com/the-beads-revolution-…-228a5f9be2a9 | OK |
| https://steve-yegge.medium.com/beads-best-practices-2db636b9f0d1 (linked from HN) | **404** |
| https://yegge.ai/essays/the-shape-of-things-to-come/ | OK |
| https://yegge.ai/beads | **404 — no dedicated page** |
| https://softwareengineeringdaily.com/2026/02/12/gas-town-beads-… | Page OK, **transcript not retrievable**; no quotes extracted |
| news.ycombinator.com direct fetch | **HTTP 429**; used `hn.algolia.com/api/v1` instead |
| https://github.com/gastownhall/beads/issues/376 (+ comments) | OK |
| https://github.com/gastownhall/beads/issues/{3059,4547} and open-issue listings | OK |
| lobste.rs search for "beads" | OK (one story, 3 pts, 0 comments) |
| Reddit | **No substantive thread found** across several query formulations; do not assume none exists |

## 10. Open questions I could not settle

1. Whether the unreleased SQLite backend upstream will use the **same** `.beads/beads.db` schema
   as classic beads (and therefore as `obr`), or a fresh Dolt-shaped schema mapped onto SQLite.
   The proposal implies the latter ("shared issue core"), but `docs/architecture/storage-backends.md`
   does not exist yet.
2. Whether the current `bd export` JSONL is still round-trippable into a classic-era importer
   without loss (memories and wisps clearly are not).
3. The exact `bd` version at which `.beads/issues.jsonl` stopped being written by default
   — v0.50 changed the *default backend*, but auto-export behavior evolved separately
   (a v1.1.0 fix mentions "auto-export JSONL in SQL-server mode via working-set state hash").
4. The provenance of the "Anthropic took inspiration from Beads" claim (secondary source only).
