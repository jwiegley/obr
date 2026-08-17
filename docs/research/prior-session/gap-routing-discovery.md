# GAP-FILL: Workspace discovery via routing/redirect (`src/config/routing.rs`)

**Subject repo:** `/Users/johnw/src/obr` (crate `beads_rust`, binary `obr`, v0.1.14) — treated strictly read-only.
**Binary exercised:** `/etc/profiles/per-user/johnw/bin/obr` → `obr 0.1.14` / `br version 0.1.14 (release)`.
**Throwaway workspaces:** under `…/scratchpad/rt`, `rt2`, `rt3`, `rt4`. Test build: `…/scratchpad/build/obr` (rsync copy, no `.git`, no `target/`).

---

## 0. Headline verdict

DOSSIER R27 ("`src/config/routing.rs` is abandoned dead surface") is **half right and dangerously
mislabelled**. Twelve public items live in that module; eleven of them are genuinely unreachable from
any non-test caller. The twelfth — `follow_redirects` — is the **first thing that runs in 33 of the
39 subcommands**, and additionally runs inside both `run_auto_import` and `run_auto_flush`. Deleting
the module on R27's advice deletes the workspace-resolution primitive for the whole CLI.

Conversely, §4.1's claim that writes "cannot escape `.beads/`" because of `src/sync/path.rs` is
**false as stated**. `follow_redirects` re-points `beads_dir` itself before the allowlist ever sees a
path; the allowlist is anchored on the *redirected* directory, so it validates happily. I reproduced
`obr create` from a workspace whose only content is `.beads/redirect` writing a SQLite database and
an `issues.org` into an arbitrary unrelated directory, silently, exit 0.

---

## 1. Reachability: who calls `follow_redirects` and `resolve_route`

### 1.1 Complete non-test call graph (exhaustive grep over `**/*.rs` excluding `target/`)

`routing::follow_redirects` — 4 non-test call sites:

| Call site | Context |
|---|---|
| `src/config/mod.rs:214` | `discover_beads_dir_with_env`, `env_override` branch |
| `src/config/mod.rs:220` | `discover_beads_dir_with_env`, `BEADS_DIR` env branch |
| `src/config/mod.rs:233` | `discover_beads_dir_with_env`, the upward-walk branch (the ordinary case) |
| `src/cli/commands/where.rs:37` | `where::execute`, *second* application on the already-resolved dir |

Import at `src/cli/commands/where.rs:4` (`use crate::config::routing::follow_redirects;`) — the only
`use` of the module outside `src/config/mod.rs:12` (`pub mod routing;`).

`routing::read_redirect` — called only from `follow_redirects` (`src/config/routing.rs:205`) and its
own unit tests (`:400`, `:409`, `:422`).

`routing::resolve_route` — **zero** non-test callers. Only `src/config/routing.rs:445`, `:456`,
`:478` (its own `#[cfg(test)]` module). Same for `resolve_route_entry` (private, called only from
`resolve_route` at `:263`, `:274`).

`find_town_root` — called only from `resolve_route` (`src/config/routing.rs:267`) and tests
(`:500`, `:507`). Since `resolve_route` is dead, so is this.

`load_routes` / `find_route` — called only from `resolve_route` (`:257`, `:262`, `:271`, `:273`) and
tests. Dead.

`extract_prefix` — called from `resolve_route` (`:250`) and `is_external_id` (`:326`) and tests. Both
callers dead ⇒ dead.

`is_external_id` — called only from `routing.rs:433-436` (tests). Dead.

`RoutingResult` / `RouteEntry` / `RoutingResult::local` / `RoutingResult::external` — constructed only
inside `routing.rs`. `RoutingResult::is_external` is **never read outside `routing.rs`**: the only
other `is_external` identifiers in the tree are unrelated locals in
`src/cli/commands/sync.rs:75,104,212,248,257,484,858`, `src/sync/mod.rs:526,537,551,756,766,780` and
`src/config/mod.rs:924` — all separate concepts (external *JSONL path* opt-in), not routing.

No references from `benches/`, `fuzz/fuzz_targets/` (`fuzz_jsonl_parse.rs`, `fuzz_org_parse.rs`,
`fuzz_validation.rs`), or `src/lib.rs` (no re-export of `routing`).

### 1.2 The discovery function itself

```rust
// src/config/mod.rs:204-242
pub fn discover_beads_dir(start: Option<&Path>) -> Result<PathBuf> {
    discover_beads_dir_with_env(start, None)                       // :205
}

fn discover_beads_dir_with_env(start: Option<&Path>, env_override: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = env_override {
        if path.is_dir() { return routing::follow_redirects(path, 10); }        // :214
    } else if let Ok(value) = env::var("BEADS_DIR") {
        if !value.trim().is_empty() {
            let path = PathBuf::from(value);
            if path.is_dir() { return routing::follow_redirects(&path, 10); }   // :220
        }
    }
    let mut current = match start { Some(p) => p.to_path_buf(), None => env::current_dir()? };
    loop {
        let candidate = current.join(".beads");
        if candidate.is_dir() { return routing::follow_redirects(&candidate, 10); }  // :233
        if !current.pop() { break; }
    }
    Err(BeadsError::NotInitialized)                                              // :241
}
```

Notes:
* `env_override` is only ever `None` (`:205` is the sole caller), so `:214` is dead in practice —
  `discover_beads_dir_with_env` is private and has no other callers.
* `:233` is the ordinary path. It **returns** the `Result`, so a redirect error aborts discovery
  immediately; it does *not* keep walking up to find another `.beads`.
* `BEADS_DIR` is only honoured if it `is_dir()`; a nonexistent value silently falls through to the
  CWD walk (reproduced, §3.7).

### 1.3 Which of the 39 commands go through it

`Commands` has exactly 39 variants (`src/cli/mod.rs`, `pub enum Commands`), matching `obr --help`
(40 rows incl. `help`).

**Reach `follow_redirects` via their own discovery call (33):**

* via `config::discover_beads_dir_with_cli(cli)` — `graph:61`, `audit:128`, `list:31`, `epic:38,86`,
  `reopen:54`, `dep:30`, `close:132`, `create:47,402`, `show:22`, `query:240`, `delete:80`,
  `update:44`, `sync:90`, `comments:27`, `stale:20`, `lint:87`, `count:39`, `history:20`,
  `defer:52,190` (Defer+Undefer), `search:41`, `info:82`, `changelog:69`, `q:36`, `stats:34`
  (Stats+Status), `label:26`, `ready:29`.
* via `config::discover_beads_dir(...)` directly — `where.rs:33`, `doctor.rs:840`, `orphans.rs:49`,
  `blocked.rs:33`, `config.rs:92,105,264,418,585`.

**Do not (6):** `Init` (writes `./.beads` literally, `init.rs:21`), `Version`, `Completions`,
`Schema` (`schema.rs` has no storage/discovery at all — 150 lines, only `config::CliOverrides` in a
signature at `:55`), `Agents`, `Upgrade` (feature-gated).

**Important `--db` caveat:** `discover_beads_dir_with_cli` (`src/config/mod.rs:255-262`) *bypasses*
routing when `--db` is given — it calls `derive_beads_dir_from_db_path` (`:267-295`) instead. So the
command body escapes redirects under `--db`… **but the hooks do not** (see next).

### 1.4 The auto-import and auto-flush hooks — confirmed on the redirect path

* `run_auto_import` — `src/main.rs:219`: `config::discover_beads_dir(Some(Path::new(".")))`. Runs
  before the command for every variant in `should_auto_import` (`main.rs:160-209`): List, Show,
  Search, Ready, Blocked, Count, Stale, Lint, Stats, Status, Orphans, Changelog, Graph, Create,
  Update, Delete, Close, Reopen, Q, Defer, Undefer, Comments, Dep, Label, Epic, Query.
* `run_auto_flush` — `src/main.rs:261`: same call. Runs after every mutating command
  (`is_mutating_command`, `main.rs:139-158`) unless `--no-auto-flush`/`--no-db`.

Both call `discover_beads_dir` **directly**, i.e. they ignore `--db` for *discovery* while still
passing `overrides.db` to `open_storage`. That mismatch is exploited in §3.9.

Confirmed at runtime — one `obr create` emits two "Following redirect" debug lines (one for the
auto-import hook, one for the auto-flush hook) plus the command's own:

```
2026-08-06T19:06:39.997849Z DEBUG beads_rust::config::routing: Following redirect from=./.beads to=…/rt3/A/.beads
2026-08-06T19:06:40.000909Z DEBUG beads_rust::config::routing: Following redirect from=./.beads to=…/rt3/A/.beads
2026-08-06T19:06:40.001540Z DEBUG beads_rust::sync: Auto-flush: exporting dirty issues dirty_count=2
```

**Conclusion for Q1: CONFIRMED.** `follow_redirects` is on the hot path of ordinary workspace
discovery, in 33/39 commands directly and in both R8 hooks. `resolve_route` is genuinely dead.

---

## 2. What `read_redirect`/`follow_redirects` actually accept

```rust
// src/config/routing.rs:162-191
pub fn read_redirect(beads_dir: &Path) -> Result<Option<PathBuf>> {
    let redirect_path = beads_dir.join("redirect");
    if !redirect_path.is_file() { return Ok(None); }             // :164 — is_file() FOLLOWS symlinks
    let content = fs::read_to_string(&redirect_path)?;           // :168
    let target = content.trim();                                 // :169
    if target.is_empty() { warn!(…); return Ok(None); }          // :171-174
    let target_path = PathBuf::from(target);                     // :176
    let resolved = if target_path.is_absolute() { target_path }  // :177-178
        else { beads_dir.parent().unwrap_or(beads_dir).join(target_path) };  // :181
    debug!(from=…, to=…, "Following redirect");                  // :184-188
    Ok(Some(resolved))
}

// src/config/routing.rs:200-232
pub fn follow_redirects(start: &Path, max_depth: usize) -> Result<PathBuf> {
    let mut current = start.to_path_buf();
    let mut visited = vec![start.to_path_buf()];
    for _ in 0..max_depth {                                      // :204
        match read_redirect(&current)? {
            Some(next) => {
                if visited.iter().any(|p| p == &next) {          // :208 — EXACT PathBuf equality
                    return Err(BeadsError::Config(format!("Redirect loop detected: {} -> {}", …)));
                }
                visited.push(next.clone()); current = next;      // :216-217
            }
            None => break,
        }
    }
    if !current.is_dir() {                                       // :224 — is_dir() FOLLOWS symlinks
        return Err(BeadsError::Config(format!("Redirect target not found: {}", current.display())));
    }
    Ok(current)                                                  // :231
}
```

### Answer to Q2, item by item

| Input | Accepted? | Evidence |
|---|---|---|
| **Absolute path** | Yes, verbatim. | `:177-178`; reproduced §3.1, §3.2 |
| **Relative path** | Yes, resolved against `beads_dir.parent()` — i.e. the *project root*, not `.beads`, and in the ordinary CWD-walk case that root is the literal `"."`, so it is CWD-relative | `:181`; reproduced §3.6 (`./A/.beads`) |
| **`..` traversal** | Yes, accepted and followed with no rejection. No component inspection at all. | reproduced §3.6 (`../A/.beads` → resolves fine) |
| **Symlinked target directory** | Yes — `is_dir()` at `:224` follows symlinks; no `read_link`, no `canonicalize` | reproduced §3.10 |
| **Redirect file that is itself a symlink** | Yes — `is_file()` at `:164` follows symlinks; content read from the link target | reproduced §3.11 |
| **Leading/trailing whitespace, trailing newlines** | Trimmed (`:169`), accepted | reproduced §3.7 |
| **Multi-line file** | Interior newline survives `trim()` → path with embedded `\n` → `is_dir()` false → "Redirect target not found" | reproduced §3.7 |
| **Empty / whitespace-only file** | Ignored with a `warn!` (which is invisible at default log level) | `:171-174`; e2e test `e2e_routing_redirect_empty_file` |
| **`~`** | **Not expanded** — treated as a literal relative component (`./~/nonexistent`) | reproduced §3.12 |
| **Path into a `.git` directory** | **Yes, fully accepted.** No `.git` check anywhere in `routing.rs`. | reproduced §3.3 — `beads.db` created inside `.git/` |
| **Path to a regular file (not a dir)** | Rejected at `:224` ("Redirect target not found") | reproduced §3.12 |
| **Loop** | Detected only by *exact `PathBuf` string equality* against visited (`:208`). `/x/.beads` vs `/x/./.beads` vs a symlink alias are **not** recognised as the same node. | reproduced §3.4 (self-loop caught) |
| **Chain longer than `max_depth`** | **Silently truncated — not an error.** The `for` loop at `:204` just ends; whatever directory hop #10 is gets returned if it `is_dir()`. | reproduced §3.5 — a 15-hop chain resolves to hop 10 for every command but hop 15 for `obr where` |

### Is `validate_no_git_path` / `validate_sync_path` ever applied to a redirect target?

**No — never, before or after.** `src/config/routing.rs` imports only
`crate::error::{BeadsError, Result}`, `serde`, `std::fs`, `std::io`, `std::path`, `tracing`
(`:21-26`). There is no reference to `crate::sync::path` anywhere in the module, and no caller
validates the returned `PathBuf` either (`src/config/mod.rs:214/220/233` return it straight to the
caller; `where.rs:37` only re-runs `follow_redirects`).

### Contrast with what `src/sync/path.rs` claims

`src/sync/path.rs:1-39` documents the allowlist as "a critical safety boundary… All sync I/O
operations MUST pass through `validate_sync_path()`", and `:31-35` states "Sync operations NEVER
access `.git/` directories. This is a hard safety invariant… Even with `--allow-external-jsonl`, git
paths are always rejected."

Both guarantees are *relative to `beads_dir`*, and `beads_dir` is the redirect output:

* `validate_sync_path(path, beads_dir)` (`path.rs:207-353`) canonicalizes `beads_dir` (`:238`) and
  then asks whether `path` is under it (`:326`). If `beads_dir` is `/anywhere/OUTSIDE`, then
  `/anywhere/OUTSIDE/issues.org` is "inside `.beads`" by construction.
* `src/cli/commands/sync.rs:212` — `let is_external = !jsonl_path.starts_with(&canonical_beads);` —
  same anchoring.
* The extension/name allowlist (`path.rs:48-59`) constrains the *filename*, never the directory.

Observed proof, from a redirect pointing at a plain directory (§3.1):

```
DEBUG beads_rust::sync::path: Validating sync path path=…/rt3/A/.beads/issues.org beads_dir=…/rt3/A/.beads
DEBUG beads_rust::sync::path: Path validated for sync I/O path=…/rt3/A/.beads/issues.org
```

The one guarantee that *does* survive is the `.git` component check — because it inspects the path
string rather than the boundary (`path.rs:140-180`, `sync.rs:238-241`, `sync.rs:261-266`
`contains_git_dir`). But it only protects the **JSONL/Org export**, not the **SQLite database**:
`config::open_storage` → `SqliteStorage::open_with_timeout(&paths.db_path, …)`
(`src/config/mod.rs:315`, `:412`) performs **no path validation whatsoever**. See §3.3, where
`obr create` created a 229 376-byte `beads.db` inside a `.git` directory and exited 0.

---

## 3. Empirical results (verbatim transcripts)

Setup used throughout: `A` = a real workspace (`obr init`), `B` = a directory whose `.beads`
contains *only* a `redirect` file, `OUTSIDE` = a bare directory.

### 3.1 Redirect → another workspace's `.beads`: reads and writes both follow it

```
$ cd rt/A && obr init
Initialized beads workspace in .beads/
$ cd rt/A && obr create "Issue in A" --type task --priority 2
✓ Created bd-3n2: Issue in A
2026-08-06T19:01:06.807854Z  INFO beads_rust::sync: Auto-flush complete exported=1

$ mkdir -p rt/B/.beads && printf '%s' "$PWD/rt/A/.beads" > rt/B/.beads/redirect
$ ls -la rt/B/.beads
-rw-r--r-- 1 johnw wheel 104 Aug  6 12:01 redirect        # that is ALL that exists in B

$ cd rt/B && obr where
/…/rt/A/.beads
  prefix: bd
  database: /…/rt/A/.beads/beads.db
exit=0

$ cd rt/B && obr list
○ bd-3n2 [● P2] [task] - Issue in A
exit=0

$ cd rt/B && obr ready
📋 Ready work (1 issue with no blockers):
1. [● P2] [task] bd-3n2: Issue in A
exit=0

$ cd rt/B && obr create "Written from B" --type task --priority 1
✓ Created bd-223: Written from B
2026-08-06T19:01:14.018746Z  INFO beads_rust::sync: Auto-flush complete exported=2
exit=0
```

After: `rt/A/.beads/issues.org` grew from 327 → 580 bytes and contains `* TODO [#B] Written from B`;
`rt/A/.beads/.br_history/` was created. `rt/B` still contains nothing but `.beads/redirect`.

**No warning of any kind was printed.** `obr where` did **not** print the `(via redirect from …)`
line (see §4.1 for why).

### 3.2 Redirect → an arbitrary directory that is not a beads workspace

```
$ mkdir -p rt/OUTSIDE && printf '%s\n' "$PWD/rt/OUTSIDE" > rt/B/.beads/redirect
$ cd rt/B && obr where
/…/rt/OUTSIDE
  database: /…/rt/OUTSIDE/beads.db
exit=0
$ cd rt/B && obr list            # (empty)
exit=0
$ cd rt/B && obr create "Written to OUTSIDE" --type task --priority 3
✓ Created bd-w96: Written to OUTSIDE
2026-08-06T19:01:22.464771Z  INFO beads_rust::sync: Auto-flush complete exported=1
exit=0
$ ls -la rt/OUTSIDE
-rw-r--r-- 1 johnw wheel 229376 Aug  6 12:01 beads.db
-rw------- 1 johnw wheel    335 Aug  6 12:01 issues.org
```

`obr` created a SQLite database **and** an Org export in a directory the user never named, that is
not called `.beads`, that contains no beads workspace, exit 0, no warning. A subsequent
`obr sync --flush-only` from `B` also succeeds ("Nothing to export (no dirty issues)"), i.e. the
`src/sync/path.rs` allowlist is fully satisfied because `beads_dir` *is* `OUTSIDE`.

### 3.3 Redirect → a `.git` directory

```
$ mkdir -p rt/REPO/.git/hooks && printf '%s\n' "$PWD/rt/REPO/.git" > rt/B/.beads/redirect
$ cd rt/B && obr where
/…/rt/REPO/.git
  database: /…/rt/REPO/.git/beads.db
exit=0
$ cd rt/B && obr create "Written into dot-git" --type task --priority 3
✓ Created bd-241: Written into dot-git
exit=0
$ ls -la rt/REPO/.git
-rw-r--r-- 1 johnw wheel 229376 Aug  6 12:01 beads.db      # <-- inside .git
drwxr-xr-x 2 johnw wheel     64 Aug  6 12:01 hooks

$ cd rt/B && obr sync --flush-only
WARN beads_rust::cli::commands::sync: Rejected JSONL path inside .git directory path=…/rt/REPO/.git/issues.org
{"error":{"code":"CONFIG_ERROR","message":"Configuration error: Refusing to use JSONL path inside .git directory: …/rt/REPO/.git/issues.org.\nMove the JSONL path outside .git to proceed.",…}}
exit=7
```

Split verdict: the **JSONL/Org** guard holds (`sync.rs:232-241`), and the auto-flush that would have
written `.git/issues.org` failed — but *silently*, because `run_auto_flush` swallows the error at
`src/main.rs:293-296` (`debug!(?e, "Auto-flush failed (non-fatal)")`). Note there is no `✓ Auto-flush
complete` line in the `create` output above, and no error either. Meanwhile the **database** file was
created inside `.git` with no check at all. `obr create` exited 0.

### 3.4 Self-redirect (loop detection works)

```
$ printf '%s\n' "$PWD/rt/B/.beads" > rt/B/.beads/redirect
$ cd rt/B && obr list
{"error":{"code":"CONFIG_ERROR","message":"Configuration error: Redirect loop detected: /…/rt/B/.beads -> /…/rt/B/.beads",…}}
exit=7
```

Loop detection is exact-string only (`routing.rs:208`); `/x/.beads` vs `/x/./.beads` vs a symlink
alias would not be caught, and would instead be truncated by `max_depth` (§3.5).

### 3.5 Chain longer than `max_depth = 10` — silent truncation, and `obr where` lies

Built `chainstart/.beads → chain/h0 → h1 → … → h14`, where `h14` is a copy of a real `.beads`.

```
$ cd rt/chainstart && obr where
/…/rt/chain/h14
  (via redirect from /…/rt/chain/h9)          # <-- the ONLY way this line ever prints
  prefix: bd
  database: /…/rt/chain/h14/beads.db
exit=0

$ cd rt/chainstart && obr list                # (empty — it is looking at h9, not h14)
exit=0

$ cd rt/chainstart && obr create "chain write" --type task
✓ Created bd-20i: chain write
2026-08-06T19:02:33.244275Z  INFO beads_rust::sync: Auto-flush complete exported=1
exit=0
$ ls -la rt/chain/h9
-rw-r--r-- 1 johnw wheel 229376 beads.db
-rw------- 1 johnw wheel    328 issues.org
-rw-r--r-- 1 johnw wheel    106 redirect
```

`obr where` reports `h14`; every other command reads and writes `h9`. `where` is the documented way
to answer "which `.beads` am I using?" (`docs/CLI_REFERENCE.md:740` — "Show the active `.beads`
directory (after redirects, if any)") and it is wrong in exactly the case where it matters.

### 3.6 Relative redirects, `..` traversal, and the resolution base

```
$ printf '%s\n' "../../rt/A/.beads" > rt/B/.beads/redirect ; cd rt/B && obr where
/…/rt/A/.beads                                              exit=0     # `..` accepted, followed

$ printf '%s\n' "A/.beads" > rt/B/.beads/redirect ; cd rt/B && obr where
No beads directory found.
Run `br init` to create one.                                exit=1     # error MASKED, see §5

$ printf '%s\n' "A/.beads" > rt/B/.beads/redirect ; cd rt/B && obr list
{"error":{…"message":"Configuration error: Redirect target not found: ./A/.beads"…}}   exit=7

$ printf '%s\n' "../A/.beads" > rt/B/.beads/redirect ; cd rt/B && obr where
/…/rt/A/.beads                                              exit=0
```

`./A/.beads` in the error text proves the resolution base: `beads_dir` from the CWD walk is the
literal `"./.beads"` (because `main.rs:219/261` and `where.rs:33` pass `Path::new(".")`), so
`beads_dir.parent()` is `"."` and the redirect is **CWD-relative**, not `.beads`-relative.

**Consequence (`..` + write path):** a relative redirect containing `..` produces a `beads_dir` that
still literally contains a `ParentDir` component, which `validate_sync_path` rejects at
`path.rs:223-234`. So the DB write succeeds and the export is silently dropped:

```
$ cd rt4/B && obr create "second dotdot" --type task
✓ Created bd-dyg: second dotdot
WARN beads_rust::sync::path: Path validation rejected path=./../A/.beads/issues.org reason=Path './../A/.beads/issues.org' contains traversal sequences
exit=0

$ obr --db rt4/A/.beads/beads.db list --allow-stale
○ bd-dyg [● P2] [task] - second dotdot        # in the DB
$ wc -c rt4/A/.beads/issues.org
0                                              # never in the git-tracked artifact
```

Exit 0, a WARN nobody sees at default verbosity, and permanent DB↔Org divergence.

### 3.7 `BEADS_DIR` behaves identically (and redirects from there too)

```
$ mkdir -p rt/ENVDIR && printf '%s\n' "$PWD/rt/OUTSIDE" > rt/ENVDIR/redirect
$ BEADS_DIR="$PWD/rt/ENVDIR" obr where
/…/rt/OUTSIDE
  database: /…/rt/OUTSIDE/beads.db            exit=0
$ BEADS_DIR="$PWD/rt/ENVDIR" obr list
○ bd-4cu [● P2] [task] - flushcheck
○ bd-w96 [● P3] [task] - Written to OUTSIDE   exit=0

$ cd rt/B && BEADS_DIR="/definitely/not/here" obr where
/…/rt/A/.beads                                exit=0      # nonexistent BEADS_DIR silently ignored
```

`BEADS_DIR` need not be named `.beads` and need not contain a workspace — a bare directory holding
only a `redirect` file is enough. Whitespace handling:

```
$ printf '   %s   \n\n' "$PWD/rt/A/.beads" > rt/B/.beads/redirect ; obr where
/…/rt/A/.beads                                exit=0      # trimmed, accepted
$ printf '%s\n%s\n' "$PWD/rt/A/.beads" "extra junk" > rt/B/.beads/redirect ; obr where
No beads directory found.                     exit=1      # embedded newline survives trim()
```

### 3.8 Redirect chain through non-`.beads` directories

`B/.beads/redirect → rt/C` (a bare dir containing only `redirect`) `→ A/.beads` resolves to
`A/.beads`, exit 0. Nothing requires any hop to be named `.beads` or to look like a workspace.

### 3.9 `--db` + redirect: cross-wired auto-flush writes database X into workspace Y

Because `run_auto_flush` (`main.rs:261`) discovers via `discover_beads_dir` (redirect-following) but
opens storage at `overrides.db`, the two disagree.

```
# rt3/M = a real workspace with its own issues; rt3/A2 = a fresh empty workspace
# rt3/B/.beads/redirect -> rt3/A2/.beads
$ cd rt3/B && obr --db rt3/M/.beads/beads.db create "cross wired 5" --type task
✓ Created bd-1vl: cross wired 5
2026-08-06T19:07:21.970276Z  INFO beads_rust::sync: Auto-flush complete exported=6
exit=0

$ grep '^\* ' rt3/A2/.beads/issues.org        # A2 is the REDIRECT TARGET
* TODO [#C] cross wired 5
* TODO [#C] cross wired 2
* TODO [#C] cross wired
* TODO [#C] cross wired 3
* TODO [#C] cross wired 4
* TODO [#C] M only issue                      # <-- M's entire DB, dumped into A2

$ grep '^\* ' rt3/M/.beads/issues.org         # M's OWN tracked file, now stale
* TODO [#C] M only issue

$ cd rt3/A2 && obr list                       # next command auto-imports the pollution into A2's DB
○ bd-1vl … ○ bd-72s … ○ bd-43l … ○ bd-2ze … ○ bd-31u … ○ bd-ok3 [task] - M only issue
```

So `--db X` run from a redirected directory Y exports **X's whole database into Y's git-tracked
`issues.org`**, and leaves X's own tracked file un-updated. There is one accidental brake: if the
target file already holds issues the source DB lacks, a data-loss guard fires and the flush is
dropped (again silently):

```
DEBUG obr: Auto-flush failed (non-fatal) e=Config("Refusing to export stale database that would lose issues.\nDatabase has 5 issues, JSONL has 1 unique issues.\nExport would lose 1 issue(s): bd-1fw\nHint: Run import first, or use --force to override.")
```

That guard is about issue counts, not paths; an empty or subset target sails through, as above.

### 3.10 / 3.11 Symlinks

```
$ ln -sfn "$PWD/rt/A/.beads" rt/SYMTARGET ; printf '%s\n' "$PWD/rt/SYMTARGET" > rt/B/.beads/redirect
$ cd rt/B && obr where →  /…/rt/A/.beads    exit=0        # symlinked target followed

$ printf '%s\n' "$PWD/rt/OUTSIDE" > rt/elsewhere_redirect
$ ln -s "$PWD/rt/elsewhere_redirect" rt/B/.beads/redirect
$ cd rt/B && obr where →  /…/rt/OUTSIDE     exit=0        # redirect file itself may be a symlink
```

`src/sync/path.rs` has an explicit `SymlinkEscape` rejection (`path.rs:300-316`, unit-tested at
`path.rs:850-872`) — it protects files *inside* `.beads`, and is irrelevant here because the escape
happens one level up, on the directory itself.

### 3.12 Misc target shapes

```
$ printf '%s\n' "$PWD/rt/A/.beads/metadata.json" > .beads/redirect ; obr list
… "Redirect target not found: /…/rt/A/.beads/metadata.json"   exit=7     # file, not dir → rejected
$ printf '%s\n' "~/nonexistent" > .beads/redirect ; obr list
… "Redirect target not found: ./~/nonexistent"                exit=7     # ~ NOT expanded
```

### 3.13 `obr init` ignores redirects (and is the reason a checked-in redirect is plausible)

```
$ cd rt/B                             # .beads/redirect -> rt/A/.beads already present
$ obr init
Initialized beads workspace in .beads/                exit=0
$ ls rt/B/.beads
.gitignore  beads.db  config.yaml  issues.org  metadata.json  redirect
$ obr where
/…/rt/A/.beads                                        # B's brand-new DB is instantly orphaned
$ obr list
○ bd-223 [● P1] [task] - Written from B
○ bd-3n2 [● P2] [task] - Issue in A
```

`init` writes `./.beads` directly (`src/cli/commands/init.rs:21-37`) with no discovery, so it happily
creates a second, permanently-shadowed database next to the redirect.

### 3.14 Is `redirect` git-tracked? — **Yes, in workspaces this binary creates.**

`src/cli/commands/init.rs:84-95` writes this `.beads/.gitignore` verbatim:

```
# Database
*.db
*.db-shm
*.db-wal

# Lock files
*.lock

# Temporary
last-touched
*.tmp
```

`redirect` is **not** listed. Reproduced: a fresh `obr init` workspace's `.beads/.gitignore` is
exactly the 10 lines above.

Contrast the repo's own `.beads/.gitignore` (inherited from the Go `bd`, `/Users/johnw/src/obr/.beads/.gitignore`), which *does* contain:

```
# Worktree redirect file (contains relative path to main repo's .beads/)
# Must not be committed as paths would be wrong in other clones
redirect
```

So the Go tool knew this file must never be committed; the Rust `init` template dropped that line.
Nothing in the Rust tree ever *writes* a `redirect` file (exhaustive grep: the only writers are the
unit tests at `routing.rs:407`, `:420` and the e2e helpers at `e2e_routing.rs:24`, `:30`) — it is a
pure, unvalidated, git-committable input.

**Answer to Q3: yes.** A `.beads/redirect` committed to a repository causes a fresh clone's `obr` to
read and write a directory the cloner never chose — including creating a SQLite DB and an `.org`
file there — with **no warning at any verbosity below `RUST_LOG=debug`**, and exit 0.

---

## 4. `routes.jsonl`, `resolve_route`, `find_town_root`

### 4.1 Static: entirely unreachable

`resolve_route` (`routing.rs:249-281`) is not called from anywhere outside its own tests (§1.1).
Therefore `load_routes`, `find_route`, `find_town_root`, `extract_prefix`, `resolve_route_entry`,
`RouteEntry`, `RoutingResult`, `RoutingResult::local/external` and `RoutingResult::is_external` are
all unreachable at runtime. `is_external_id` is separately unreachable.

`find_town_root`'s unbounded upward walk (`routing.rs:85-101` — `loop { … if !current.pop() { break; } }`,
one `is_file()` stat per ancestor, no depth cap, terminating only at `/`) is therefore never
executed. It would be a real cost/hazard if `resolve_route` were ever wired up (an `obr` run inside a
deep tree would stat `mayor/town.json` at every ancestor up to root, including network mounts).

### 4.2 Empirical: `routes.jsonl` is completely inert

```
# rt2/ext = workspace with issue_prefix: ext and issue ext-32t
# rt2/main/.beads/routes.jsonl = {"prefix":"ext-","path":"…/rt2/ext"}
$ cd rt2/main && obr show ext-32t
{"error":{"code":"ISSUE_NOT_FOUND","message":"Issue not found: ext-32t","hint":"Run 'br list' to see available issues.","context":{"searched_id":"ext-32t"}}}
exit=3
```

The route exists, points at a valid workspace containing that exact issue, and is ignored.

```
$ printf 'not valid json\n' > rt2/main/.beads/routes.jsonl
$ cd rt2/main && obr list ; obr create "with malformed routes" --type task
exit=0
✓ Created bd-2qo: with malformed routes                exit=0
```

A syntactically invalid `routes.jsonl` produces **no** error, because `load_routes`
(`routing.rs:110-144`, which would raise `"Invalid route at …"`) is never called.

```
$ mkdir -p rt2/mayor && printf '{}' > rt2/mayor/town.json
$ cd rt2/main && obr where →  /…/rt2/main/.beads       exit=0
```

A town root has no effect.

### 4.3 `RoutingResult::is_external` — consumers and writability

**Nothing consumes it** (§1.1). If `resolve_route` were revived, note that
`resolve_route_entry` (`:284-319`) ends with `follow_redirects(&target_path, 10)` (`:309`) and
returns a plain `PathBuf`; `is_external` is advisory metadata only — there is no code path that
would *degrade* an external target to read-only. An external target reached through routing would be
opened by `config::open_storage*` exactly like a local one, i.e. fully writable. The empirically
demonstrated redirect writes (§3.1–§3.3) are the same mechanism minus the prefix lookup.

Absolute route paths are taken verbatim (`:294-295`); a route path that is not named `.beads` gets
`.beads` appended (`:301-305`); a missing target surfaces as "Redirect target not found" from
`follow_redirects`; "outside the repo" is not a concept the code has.

---

## 5. `tests/e2e_routing.rs`: what it asserts, what it does not, and does it pass

### 5.1 Does it pass today? **Yes — all 14 tests.**

Built an out-of-tree rsync copy at `…/scratchpad/build/obr` with the sibling
`org2jsonl` copied to `…/scratchpad/build/org2jsonl` (the `Cargo.toml:64` path dep `../org2jsonl`
resolves correctly by relative position). The system `rustc` is **stable 1.97.1** (no rustup, so
`rust-toolchain.toml`'s `channel = "nightly"` is not honoured) and the build fails on
`rich_rust`'s `#![feature(let_chains)]` (`E0554`). Under `nix develop` (Rust 1.95.0-nightly) it
builds and runs:

```
$ nix develop --command cargo test --test e2e_routing -- --test-threads=4
test e2e_routing_db_flag_requires_beads_component ... ok
test e2e_routing_invalid_beads_dir_env ... ok
test e2e_routing_not_initialized_error ... ok
test e2e_routing_local_prefix_no_routes_file ... ok
test e2e_routing_db_flag_external_path ... ok
test e2e_routing_redirect_empty_file ... ok
test e2e_routing_path_normalization ... ok
test e2e_routing_redirect_file_absolute_path ... ok
test e2e_routing_redirect_missing_target ... ok
test e2e_routing_redirect_file_relative_path ... ok
test e2e_routing_routes_jsonl_malformed_line ... ok
test e2e_routing_routes_jsonl_local_route ... ok
test e2e_routing_show_external_issue_not_found ... ok
test e2e_routing_routes_jsonl_external_route ... ok

test result: FAILED. 137 passed; 1 failed; …
```

The single failure is **not** in `e2e_routing`: it is
`common::dataset_registry::tests::test_metadata_includes_source_commit`
(`tests/common/dataset_registry.rs:1226`, "source_commit should be captured for git repos"), which
fails only because my copy has no `.git`. Every `e2e_routing_*` test passes. (Also note: the shared
`tests/common/` module compiles its own unit tests into *every* integration target, so this one
unrelated assertion turns `cargo test --test <anything>` red outside a git checkout — worth knowing
for anyone re-enabling these gates.)

### 5.2 What the 14 tests actually assert

| Test | line | Asserts | Real coverage? |
|---|---|---|---|
| `local_prefix_no_routes_file` | `:38` | init/create/list work with no routes file | Baseline only |
| `routes_jsonl_local_route` | `:72` | create/list still work with `{"prefix":"bd-","path":"."}` | Vacuous — routes are never read |
| `routes_jsonl_malformed_line` | `:109` | `if !create.status.success() { assert stderr mentions "Invalid route"/"invalid"/"JSON" }` | **Vacuous by construction** — create always succeeds (§4.2), so the assertion never runs |
| `routes_jsonl_external_route` | `:149` | writes a route in `main`, then creates and lists **in `external_workspace` itself** | Never exercises routing at all |
| `redirect_file_absolute_path` | `:210` | creates `redirect_workspace/.beads/redirect` → `actual_beads`, then runs every command **in `actual_workspace`** | **Never runs a command from the redirected directory.** Comment at `:231` states "The redirect is used during route resolution, not BEADS_DIR discovery" — **this comment is factually wrong** (`config/mod.rs:233`) and is precisely the belief that hides the bug |
| `redirect_file_relative_path` | `:258` | `redirect` containing `"."`, create/list succeed | `"."` resolves to the project root's `.` → the *parent* of `.beads`… and passes because `.` is a dir. Does not test relative escape |
| `redirect_missing_target` | `:300` | `show ext-abc123` with a route+redirect; `if !show.status.success() { assert stderr contains "not found"/"Redirect"/"redirect"/"Issue"/"route" }`; comment at `:346`: "If it succeeds… that's also acceptable" | Vacuous — passes on the substring `"Issue"` from `ISSUE_NOT_FOUND`, which is the *no-routing* outcome |
| `redirect_empty_file` | `:350` | empty `redirect` ignored, `list` succeeds | Genuine, narrow |
| `db_flag_external_path` | `:376` | `--db` to an external `.beads` works | Tests the `--db` bypass, not redirects |
| `db_flag_requires_beads_component` | `:426` | `--db` without a `.beads` component errors | Genuine |
| `path_normalization` | `:454` | `--db …/actual/subdir/../.beads/beads.db` works | The **only** "path normalization" assertion in the file, and it is about `--db`, not redirects |
| `not_initialized_error` | `:498` | uninitialised → clear error | Genuine |
| `invalid_beads_dir_env` | `:518` | `BEADS_DIR=/nonexistent/path/.beads` → failure | Genuine, but note the mechanism is the silent fall-through at `config/mod.rs:219`, not a rejection |
| `show_external_issue_not_found` | `:545` | route to external workspace, `show ext-nonexistent` fails | Passes for the **wrong reason** — routing never happens |

### 5.3 What the file's own header promises but does not deliver

`tests/e2e_routing.rs:1-8` advertises:

* "Prefix-based route lookup (routes.jsonl)" — **not tested**; the two route tests are vacuous, and
  the feature does not exist at runtime.
* "Redirect file following" — tested only in configurations where the redirect is never consulted.
  **No test runs any command from a directory whose `.beads` contains a redirect to a different
  workspace.**
* "Redirect loop detection" — **section header exists, no test.** No test writes a self- or
  mutual-redirect. (I verified the behaviour works, §3.4, but the gate does not.)
* "External DB reference safety and path normalization" — the three tests under that banner
  (`:376`, `:426`, `:454`) are all about the `--db` flag. **No test asserts anything about where a
  redirect is permitted to point.** There is no test for a redirect into `.git`, outside the repo,
  through `..`, through a symlink, or beyond `max_depth`. `grep -rln redirect tests/` returns only
  `conformance.rs` (a field-name volatility list at `:808`), `e2e_installer.rs` (HTTP redirects,
  unrelated), and `e2e_routing.rs` — `e2e_sync_git_safety.rs` and `e2e_git_safety_full_cli.rs` do
  not mention redirects at all.

So: "External DB reference safety" is **not genuinely tested**, and the one comment in the file that
addresses the mechanism (`:231`) asserts the opposite of the truth.

---

## 6. Verdict on `src/config/routing.rs`, item by item

`routing.rs` is 510 lines: ~330 of implementation + ~180 of `#[cfg(test)]`. Public surface = 12
items (R27's "8 of 10" undercounts and, more importantly, does not distinguish *which* item matters).

| # | Item | Line | Status | Notes |
|---|---|---|---|---|
| 1 | `follow_redirects` | `:200` | **LOAD-BEARING — do not delete** | 4 call sites; runs first in 33/39 commands + both R8 hooks. Deleting it breaks every command. |
| 2 | `read_redirect` | `:162` | **LOAD-BEARING (transitively)** | Sole helper of #1. `pub` is unnecessary — could be `fn`. |
| 3 | `resolve_route` | `:249` | Dead | Zero non-test callers. |
| 4 | `resolve_route_entry` (private) | `:284` | Dead | Only from #3. |
| 5 | `load_routes` | `:110` | Dead | Only from #3. |
| 6 | `find_route` | `:148` | Dead | Only from #3. |
| 7 | `find_town_root` | `:85` | Dead | Only from #3. Unbounded upward walk if revived. |
| 8 | `extract_prefix` | `:77` | Dead | Only from #3 and #12. Thin wrapper over `util::id::split_prefix_remainder` (`src/util/id.rs:280`), which has 7 live callers of its own. |
| 9 | `is_external_id` | `:325` | Dead | Tests only. |
| 10 | `RouteEntry` | `:30` | Dead | Only the (dead) route path constructs it. |
| 11 | `RoutingResult` (+ `.is_external`, `.project_path`) | `:39` | Dead | Never constructed or read outside `routing.rs`. |
| 12 | `RoutingResult::local` / `::external` | `:51`, `:61` | Dead | Only from #3/#4. |

**Recommendation for a reader:** items 3–12 (roughly `routing.rs:28-150` + `:234-327`, plus their
tests) are safe to remove *if* the `.beads/routes.jsonl` / `mayor/town.json` feature is being
formally abandoned — but that is a **product** decision, since the file formats are part of the
`bd` compatibility surface (`routing.rs:6-19` documents them as "classic beads routing used by
`show`, `update`, `close`"), and abandoning them silently means `obr` ignores a `routes.jsonl` that
`bd` would honour. Items 1–2 must stay, and should gain the validation they lack.

### 6.1 Concrete defects worth filing (all reproduced above)

1. `follow_redirects` applies no policy to its output: no `validate_no_git_path`, no repo-root
   containment, no canonicalization, no symlink resolution (`routing.rs:200-232`). It is the only
   unvalidated `beads_dir` producer, and it feeds `src/sync/path.rs`'s *anchor*.
2. `SqliteStorage` opening is not path-validated at all (`config/mod.rs:315`, `:412`), so a redirect
   into `.git` creates `beads.db` there (§3.3) despite `SYNC_SAFETY_INVARIANTS` NGI-3.
3. `max_depth` exhaustion is not an error (`routing.rs:204-221`); the 10th hop is silently adopted,
   and `obr where` — which follows *again* (`where.rs:37`) — then reports a different directory from
   the one every other command uses (§3.5).
4. The `redirected_from` reporting in `where.rs:38-42` / `:123-124` / `:169-174` is effectively dead
   in normal operation, because `discover_beads_dir` already resolved the chain; it only fires in the
   >10-hop case, i.e. exactly when it is misleading.
5. `obr init` does not honour redirects (`init.rs:21`), so `init` in a redirected worktree silently
   creates a shadow database that nothing will ever read (§3.13).
6. `init`'s `.gitignore` template (`init.rs:84-95`) omits `redirect`, which the Go tool's template
   explicitly excludes with the comment "Must not be committed as paths would be wrong in other
   clones". This is what makes the checked-in-redirect scenario realistic (§3.14).
7. Redirect errors are masked by `let Ok(..) else` in `where.rs:33` ("No beads directory found. Run
   `br init`"), `doctor.rs:840` ("Missing .beads directory (run `br init`)"), `orphans.rs:49`, and
   `config.rs:92/105/418/585` (`obr config list` prints defaults, exit 0). `obr list`/`create`
   surface the true `CONFIG_ERROR` (exit 7).
8. A relative redirect containing `..` yields a `beads_dir` with a `ParentDir` component, which
   `validate_sync_path` (`path.rs:223-234`) rejects — so mutations succeed into the DB while the
   export is dropped with only a WARN (§3.6). Silent DB↔Org divergence.
9. `--db` + redirect cross-wiring: `run_auto_flush` discovers with redirects but opens the `--db`
   database, exporting one workspace's contents into another workspace's git-tracked file (§3.9).
10. `run_auto_flush`'s blanket `debug!(?e, "Auto-flush failed (non-fatal)")` (`main.rs:293-296`)
    swallows every one of these, including the `.git` rejection and the traversal rejection.

---

## 7. Cross-references for the dossier

* R27 ("routing.rs is dead") — **amend**: 10 of 12 public items dead, but `follow_redirects` /
  `read_redirect` are the hot-path workspace resolver. Do not recommend deletion of the module.
* §4.1 ("writes cannot escape `.beads/`, mechanically enforced by `src/sync/path.rs`") — **amend**:
  the enforcement is relative to a `beads_dir` that an untrusted, git-committable file chooses.
  Escape reproduced with a plain `obr create`.
* R8 (auto-flush) — the hook itself performs redirect-following discovery (`main.rs:261`), and it
  ignores `--db` while `open_storage` honours it.
* "97 never-executed integration targets" — `tests/e2e_routing.rs` compiles and all 14 of its tests
  pass today; the shared `tests/common/dataset_registry.rs:1226` unit test is the only thing that
  reddens the target, and only outside a git checkout.
