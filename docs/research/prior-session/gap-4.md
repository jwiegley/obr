# GAP-FILL: obr's backup/restore recovery mechanism (`.beads/.br_history` + `obr history restore`)

Research target: `/Users/johnw/src/obr` (crate `beads_rust`, binary `obr` v0.1.14). Repo treated as
strictly read-only; all experiments ran in throwaway workspaces under
`/private/tmp/claude-501/-Users-johnw-src-obr/f80d1967-0fc0-44fa-a53f-3054d39727e2/scratchpad/w1..wB`
driving the installed binary at `/etc/profiles/per-user/johnw/bin/obr`.

---

## 0. HEADLINE (read this if you read nothing else)

The dossier's §7 Q9 premise ("recovery is impossible, therefore build `doctor --repair`") is wrong in
both directions, and the correct answer is *cheaper and worse* than the dossier says.

**The single most important finding is one nobody has recorded: `.br_history` is dead code in the
default agent workflow.** Auto-flush — the path taken by *every* mutating command — never creates a
backup at all, because `run_auto_flush` discovers a *relative* beads dir (`./.beads`) and the backup
call site is gated on an absolute-path `starts_with` comparison that can therefore never be true.
Proven at DEBUG log level and by 244 concurrent `obr create` calls producing exactly zero backups.
Backups only exist if the user runs `obr sync` explicitly.

Second most important: **the actual recovery command for two of the three confirmed defects is
`obr sync --flush-only --force`, and it has nothing to do with `history restore`.** The SQLite DB
survives all three defects; only the file is damaged; re-exporting from the DB repairs it in one
command. `history restore` is the *wrong* tool for all three — for the export wedge it is actively
counterproductive (it reinstalls the wedge).

Third: **the same `obr sync --flush-only --force` is a total-data-loss command in the one situation
where `history restore` would matter** (fresh clone, no DB, corrupt file) — it exported 0 issues over
a 5-issue file, leaving 78 bytes. Nothing in obr's output lets a user distinguish the two situations.

So Q9's remediation is not "build `doctor --repair`". It is: (a) fix a one-line path bug so backups
exist at all, (b) make error messages name the correct existing command *for the situation*, and (c)
close a path-traversal hole in `history restore` that lets it copy `/etc/passwd` or `.git/config` over
the durable artifact.

---

## 1. What `history restore` actually does

### 1.1 The surface

`HistoryArgs` / `HistoryCommands` are declared at `src/cli/mod.rs:2198-2230`: four subcommands
`List`, `Diff { file }`, `Restore { file, force }`, `Prune { keep, older_than }`. Dispatch is
`src/cli/commands/history.rs:19-33`, with `List` as the `None` default (`:31`).

`restore_backup` is `src/cli/commands/history.rs:233-290`. The whole operative body is four lines:

- `src/cli/commands/history.rs:240` — `let backup_path = history_dir.join(filename);`
- `:241-245` — error if `!backup_path.exists()`
- `:247` — `let target_path = beads_dir.join(crate::config::DEFAULT_JSONL_FILENAME);`
- `:249-254` — if target exists and `!force`, refuse with "Use --force to overwrite."
- `:257` — `std::fs::copy(&backup_path, &target_path)?;`

Everything after `:257` is output formatting.

### 1.2 Answers

**Does restore re-import into SQLite? No.** It copies one file over another and then *tells* the user
to run an import: `:265` emits `"next_step": "br sync --import-only --force"` in JSON, `:278` renders
`"Next: br sync --import-only --force"` in rich mode, `:286` prints
`"Run 'br sync --import-only --force' to import this state into the database."` in plain mode.

**The suggested command does not exist.** The binary is `obr`; `br` is not on PATH:

```
$ which br || echo "'br' NOT on PATH"
br not found
'br' NOT on PATH -> the suggested next-step command does not exist
```

This is not isolated: `obr doctor` prints the literal header `br doctor`, and every recovery
instruction in `docs/TROUBLESHOOTING.md:391-393` (`br history list` / `br history restore <backup>`)
and `docs/CLI_REFERENCE.md:775` uses the stale name.

**What does `--force` change?** Only whether restore refuses when `.beads/issues.org` already exists
(`:249-254`). Since `obr init` unconditionally creates an empty `issues.org`
(`src/cli/commands/init.rs`, "Write empty issues.org for compatibility with bv"), the target *always*
exists, so `--force` is mandatory in every real invocation. It gates nothing meaningful — it is a
speed bump, not a guard. Verbatim:

```
$ obr history restore issues.20260806_190219.org
{ "error": { "code": "CONFIG_ERROR",
  "message": "Configuration error: Current issues.org exists. Use --force to overwrite.", ... } }
```

**Restore does not snapshot the file it destroys.** There is no `backup_before_export` call anywhere
in `src/cli/commands/history.rs`. Proven in workspace w6:

```
$ ls .beads/.br_history/            # 1 backup
issues.20260806_190804.org
$ cp .beads/issues.org /tmp/w6_pre.org
$ obr history restore issues.20260806_190804.org --force
$ ls .beads/.br_history/            # still 1 backup
issues.20260806_190804.org
$ diff -q /tmp/w6_pre.org .beads/.br_history/issues.20260806_190804.org
*** NO - the state restore destroyed is not in history ***
```

Restoring the wrong backup is unrecoverable from within obr.

### 1.3 PATH TRAVERSAL — the `file` argument is completely unvalidated

`src/cli/commands/history.rs:240` does `history_dir.join(filename)` with an unsanitised user string.
It never calls `validate_sync_path`, `validate_no_git_path`, `require_valid_sync_path`, or anything
else from `src/sync/path.rs`. `Path::join` with an absolute path *replaces*; with `../` it *escapes*.
All three escapes reproduced:

```
$ printf 'root:x:0:0\n' > .../scratchpad/evil.txt
$ obr history restore /private/tmp/.../scratchpad/evil.txt --force
Restored /private/tmp/.../scratchpad/evil.txt to issues.org
$ cat .beads/issues.org
root:x:0:0
```

```
$ obr history restore "../../../evil.txt" --force
Restored ../../../evil.txt to issues.org
$ head -c 100 .beads/issues.org
root:x:0:0
```

```
$ obr history restore "../../.git/config" --force
Restored ../../.git/config to issues.org
$ head -5 .beads/issues.org
[core]
	repositoryformatversion = 0
	filemode = true
	bare = false
	ignorecase = true
```

Notes on severity:

- The **write** target is fixed (`beads_dir.join(DEFAULT_JSONL_FILENAME)`, `:247`), so this is not an
  arbitrary-write primitive. It is an **arbitrary-read → durable-artifact-overwrite** primitive: any
  file the user can read gets copied over the git-tracked `.beads/issues.org`, destroying it.
- The `.git/config` case directly violates the invariant `src/sync/path.rs:31-35` declares as hard
  ("Sync operations NEVER access `.git/` directories. This is a hard safety invariant enforced by
  `validate_no_git_path()`") and that `validate_no_git_path` (`src/sync/path.rs:140-180`) implements
  — `history restore` simply never calls it.
- `diff_backup` has the identical unvalidated join at `src/cli/commands/history.rs:137`, giving an
  arbitrary-file-**read-to-stdout** primitive:

```
$ obr history diff /etc/hosts
Diffing current issues.org vs /etc/hosts...
--- /private/tmp/.../w2/.beads/issues.org	2026-08-06 12:03:03 -0700
+++ /etc/hosts	2026-08-03 11:44:52 -0700
@@ -1,111 +1,12 @@
[... full contents of both files printed ...]
```

- No test covers this. `grep -rn 'restore.*\.\./|restore.*/etc/|traversal' tests/e2e_history*.rs
  tests/storage_history.rs` returns nothing across 2158 lines of history tests.

Missing-backup case is handled correctly (exit 7, `CONFIG_ERROR`).

---

## 2. THE BURIED DEFECT: auto-flush never creates a backup

This was not in any prior notes file and it invalidates every retention/granularity question below
for the default workflow.

### 2.1 Mechanism

- `src/main.rs:261` — `run_auto_flush` calls `config::discover_beads_dir(Some(Path::new(".")))`.
- `src/config/mod.rs:225-239` — discovery does `current.join(".beads")` starting from `"."`, and
  `routing::follow_redirects` (`src/config/routing.rs:200-232`) returns it verbatim. No
  canonicalisation. Result: **`beads_dir == "./.beads"`, a relative path.**
- `src/sync/mod.rs:1276-1286` — the backup gate:

```rust
let output_abs = if output_path.is_absolute() { output_path.to_path_buf() }
    else if let Ok(cwd) = std::env::current_dir() { cwd.join(output_path) }
    else { output_path.to_path_buf() };
if output_abs.starts_with(beads_dir) {
    history::backup_before_export(beads_dir, &config.history, &output_abs)?;
}
```

`output_abs` is forced absolute; `beads_dir` is `./.beads`. `starts_with` is component-wise, so an
absolute path can never start with a relative one. **The branch is dead in the auto-flush path.**

### 2.2 Proof at DEBUG level

```
$ obr -vv create --title="tracecheck" --type=task -p 2 2>&1 | grep -i "beads_dir\|backup"
DEBUG beads_rust::sync::path: Validating sync path path=./.beads/issues.org beads_dir=./.beads
DEBUG beads_rust::sync: Export path validated output_path=./.beads/issues.org beads_dir=./.beads allow_external=false
DEBUG beads_rust::sync::path: Validating sync path path=./.beads/issues.org.tmp beads_dir=./.beads
DEBUG beads_rust::sync::path: Validating sync path path=./.beads/issues.org beads_dir=./.beads
```

No `"Created backup: ..."` line (`src/sync/history.rs:94`) is ever emitted.

### 2.3 Proof by A/B on the same workspace

`BEADS_DIR` is consulted first by `discover_beads_dir_with_env` (`src/config/mod.rs:216-222`) and, if
absolute, produces an absolute `beads_dir` — flipping the gate on:

```
--- create WITHOUT BEADS_DIR (relative discovery) ---
✓ Created bd-3ho: relcheck
INFO beads_rust::sync: Auto-flush complete exported=8
$ ls .beads/.br_history/
issues.20260806_190219.org                       <-- unchanged

--- create WITH absolute BEADS_DIR ---
✓ Created bd-wyu: abscheck
INFO beads_rust::sync: Auto-flush complete exported=9
$ ls .beads/.br_history/
issues.20260806_190219.org
issues.20260806_190229.org                       <-- new backup
```

### 2.4 Proof at scale

Workspace w2, 2000 seeded issues + one explicit flush to seed exactly one backup, then **244
concurrent `obr create`** across 8 bursts (12, 24×3, 40×4):

```
round=1 headings=2037  ... round=7 headings=2245
$ ls .beads/.br_history/ | wc -l
1
```

244 mutations, 244 auto-flushes, **zero** new backups.

### 2.5 Which commands *do* back up

| Path | Backup? | Evidence |
|---|---|---|
| `obr create/update/close/delete` (auto-flush) | **No** | §2.2–2.4; w8 `obr delete` left history unchanged |
| `obr sync --flush-only [--force]` | Yes | `src/cli/commands/sync.rs:576-586` builds `ExportConfig` from `path_policy.beads_dir` (absolute) |
| `obr sync --merge` | Yes | `src/cli/commands/sync.rs:1211-1220` (`force: true`); w8 produced `issues.20260806_190948.org` |
| `obr sync --import-only [--force]` | **No** | w8: history file count unchanged across import |
| `obr history restore` | **No** | §1.2 |
| refused export (guards fired) | Yes | backup at `src/sync/mod.rs:1285` runs *before* the guards at `:1293-1340`; w6 got a backup from a failing sync |

### 2.6 Why 2158 lines of tests never caught it

Every history e2e test creates issues with `--no-auto-flush` and then calls explicit sync
(`tests/e2e_history.rs:28-33`, `tests/e2e_history_restore_prune.rs:37-42`,
`tests/e2e_history_custom_path.rs:5-8`). The comment at `tests/e2e_history.rs:39-40` explains why:

> "Note: We use --no-auto-flush to prevent automatic export after create, which would clear dirty
> flags and prevent the explicit sync from triggering backups."

The author assumed auto-flush *would* have backed up and only wanted to control timing. The test
suite routes around the only code path a real user takes. `docs/CLI_REFERENCE.md:781` happens to be
accurate ("Backups are created during `obr sync --flush-only`"), while `docs/SYNC_SAFETY.md:95-97`
and the `docs/ARCHITECTURE.md:179` pipeline diagram ("2. Create history backup — Optional timestamped
copy (if overwriting)") imply it happens on every export.

---

## 3. Empirical recovery against each confirmed defect

### 3.1 FILE CORRUPTION (fixed temp filename, `src/sync/mod.rs:1409-1443`)

The fixed name is confirmed: `src/sync/mod.rs:1420-1426` computes
`temp_path = output_path.with_extension("org.tmp")` — one inode, `.beads/issues.org.tmp`, shared by
all concurrent exporters; `File::create(&temp_path)` at `:1442` truncates whatever is there.

I did **not** reproduce interleaved-write corruption on this machine (244 concurrent creates, 0
duplicate IDs, `obr list` exit 0 every round) — SQLite lock contention appears to serialise the
flushes enough here. I *did* reproduce the same underlying race loudly on the explicit-sync path
(§3.4). For recovery testing I injected the exact damage the dossier describes (record truncated
mid-`:ISSUE_TYPE:` property) into w2's 2245-issue file.

**Symptom.** Total wedge, and the error names nothing:

```
$ obr list
{ "error": { "code": "VALIDATION_FAILED",
    "message": "Validation failed: id: Missing required :ID: property",
    "hint": null, "retryable": true,
    "context": { "field": "id", "reason": "Missing required :ID: property" } } }
exit=4
```

Same for `obr ready`, `obr create`, everything. No file named, no line number, `hint: null`.

**`obr doctor` detects it, names no fix:**

```
$ obr doctor
br doctor
OK jsonl.merge_artifacts
OK sync_jsonl_path: JSONL path is within sync allowlist
OK sync_conflict_markers: No merge conflict markers found
ERROR jsonl.parse: Failed to parse Org file: Validation failed: id: Missing required :ID: property
OK schema.tables
OK schema.columns
OK sqlite.integrity_check
WARN counts.db_vs_jsonl: DB and JSONL counts differ
OK sync.metadata: External changes pending import
$ echo $?
1
```

Doctor exits 1 and the JSON detail carries the file path, but there is no remediation string anywhere.

**Was a usable pre-corruption snapshot in `.br_history`? Effectively no.** The only backup was the
2000-issue seed state from the one explicit sync; 245 issues of subsequent work were never
snapshotted (§2.4). In a real agent session with no explicit `obr sync`, `.br_history` would not
exist at all.

**Recovery is ONE command and it is not `history restore`:**

```
$ obr sync --flush-only --force
Exported:
  2245 issues
  0 dependencies / 0 labels / 0 comments
Cleared dirty flag for 2245 issues
$ grep -c '^\* ' .beads/issues.org   -> 2245
$ obr list | wc -l                   -> 2245
```

The DB was never touched by the corruption; the file is a derived artifact and re-deriving it is
exact and lossless. (`obr sync --flush-only` *without* `--force` is a no-op — "Nothing to export (no
dirty issues)" — because dirty flags were already cleared. `--force` is required, and nothing tells
you that.) `--no-auto-import` is *not* needed; the sync command does not run the auto-import that
wedges `obr list`.

Verdict: **(ii/iii) possible but undiscoverable** — one command, zero loss, and the user is never
told. `history restore` would have *lost 245 issues* here.

### 3.2 EXPORT WEDGE (content-hash dedup leaves an id the DB will never have)

Reproduced deterministically in w6 by cloning an existing record under a new ID (`bd-ghost`) so
content-addressed dedup maps it onto the original and the ID is never created in the DB:

```
$ grep '^:ID:' .beads/issues.org
:ID:       bd-1k3
:ID:       bd-wur
:ID:       bd-ghost
$ obr list --json | ...   -> ['bd-1k3', 'bd-wur']        # bd-ghost never enters the DB
```

**Silent loss confirmed.** Four subsequent creates all report success and exit 0, and none reach the
durable file:

```
$ obr create --title="post-wedge" ...   ✓ Created bd-1gm: post-wedge     (exit 0)
$ obr create --title="lost work 1..3"   create exit=0 (×3)
$ obr list | wc -l           -> 6      # DB
$ grep -c '^\* ' .beads/issues.org -> 3  # git-tracked file
$ grep '^:ID:' .beads/issues.org
:ID: bd-1k3   :ID: bd-wur   :ID: bd-ghost
```

Four issues exist only in the gitignored SQLite DB. The refusal is real but swallowed:
`src/main.rs:293-296` logs auto-flush failure at `debug!` with the comment "Log but don't fail -
auto-flush errors shouldn't break the command".

**`obr doctor` does not flag it as an error:**

```
OK jsonl.parse: Parsed 3 issues from Org format
WARN counts.db_vs_jsonl: DB and JSONL counts differ
$ echo $?    -> 0
```

Exit 0. The single most diagnostic signal (`db: 6, jsonl: 3`) is a WARN, and `has_error`
(`src/cli/commands/doctor.rs:58`) only escalates on ERROR.

**Explicit sync gives the only good error in the whole system:**

```
$ obr sync --flush-only
{ "error": { "code": "CONFIG_ERROR", "message": "Configuration error: Refusing to export stale
  database that would lose issues.\nDatabase has 3 issues, JSONL has 3 unique issues.\nExport would
  lose 1 issue(s): bd-ghost\nHint: Run import first, or use --force to override.", ... } }
```

(Text from `src/sync/mod.rs:1319-1339`. It names the offending ID and gives a hint — this is the
model every other error should follow.)

**`history restore` makes it PERMANENT.** The documented two-command recovery:

```
$ obr history restore issues.20260806_190804.org --force
Restored issues.20260806_190804.org to issues.org
Run 'br sync --import-only --force' to import this state into the database.
$ grep '^:ID:' .beads/issues.org   -> bd-1k3, bd-wur, bd-ghost      # wedge reinstalled
$ obr sync --import-only --force
Imported from JSONL: Processed: 3 issues
$ obr list | wc -l -> 6 ; grep -c '^\* ' .beads/issues.org -> 3      # still divergent
$ obr sync --flush-only
Refusing to export stale database that would lose issues.            # still wedged
```

The refused export had snapshotted the *wedged* file (backup runs before the guards), so the only
available backup is itself poisoned.

**Recovery is again ONE command:** `obr sync --flush-only --force` → file gains all 6 IDs, wedge
gone, next `obr create` flushes normally (7 records).

Verdict: **(iii) possible only with knowledge no output provides** — and the documented history
workflow is actively harmful here.

### 3.3 CONCURRENT FLUSH LOSS + the dedup interaction (this is Q3c)

Constructed deterministically in w5. Sequence and outcome:

1. seed 3 issues, flush twice (1.2 s apart) so the newest backup **equals** the current file S1:
   `issues.20260806_190734.org -> 0`, `issues.20260806_190735.org -> 3`.
2. `obr create "CRITICAL BEAD"` — auto-flush writes S2 (4 records) and, per §2, **creates no backup**.
3. simulate the losing rename: file reverts to S1.
4. one more mutation + explicit flush.

Result:

```
history after:
  issues.20260806_190734.org -> 0
  issues.20260806_190735.org -> 3
CRITICAL BEAD present in any backup?
  *** NONE - dedup skipped the snapshot ***
```

`files_are_identical` (`src/sync/history.rs:83-91`, implementation `:226-267`) compared the reverted
S1 against the newest backup S1, found them identical, and returned early at `:89` — so the state
that was about to be overwritten was skipped, and the state that was *lost* (S2) had never been
captured. **Yes: dedup-against-latest can cause the one snapshot you need to be skipped**, and it does
so precisely when the file has moved *backwards*, which is exactly the concurrent-flush-loss
signature.

Silver lining that reframes the defect's severity: the DB retained CRITICAL BEAD, so the *next*
mutation's auto-flush (which exports `get_all_issues_for_export()`, not just dirty rows) silently
healed the file back to 5 records. **Concurrent flush loss is self-healing on the next mutation as
long as the DB survives.** The real damage window is "you `git commit .beads/issues.org` before the
next mutation".

Verdict: **(i) automatic, conditionally** — self-heals on the next mutation; **(iv) impossible** if
the bad file is committed and the DB is later discarded, because history never captured the good
state.

### 3.4 Bonus: the same race, loud, on explicit sync

10–30 concurrent `obr sync --flush-only --force`:

```
$ sort /tmp/ez/ex_* | uniq -c
      3 0
      1 7
      6 8
$ grep -h message /tmp/ez/log_* | sort -u
    "message": "Configuration error: Export verification failed: expected 10 issues, JSONL has 11 lines",
    "message": "I/O error: No such file or directory (os error 2)",
```

21/30 failures in one run, 9/10 in another. I confirmed this is the **fixed temp filename**, not the
history code, by rerunning against an external JSONL path where the backup branch is skipped entirely
(history dir untouched at 2 files) — identical 6/10 failure rate. The ENOENT is process B's
`fs::rename` of `issues.org.tmp` after process A already consumed it.

The message is a bare `I/O error: No such file or directory (os error 2)` with `hint: null`,
`context: null`, no path. On the auto-flush path this identical error is swallowed at `debug!`
(`src/main.rs:293-296`) and the command exits 0 — i.e. **the loud failure and the silent data loss
are the same bug wearing different clothes.**

### 3.5 The scenario where recovery genuinely is impossible

Fresh-clone simulation in wB (DB is gitignored, so a clone has only the file): 5 issues, file
corrupted, `beads.db*` removed.

```
$ obr list
{ "error": { "code": "VALIDATION_FAILED", "message": "Validation failed: id: Missing required :ID: property", "hint": null ... } }

$ obr doctor
ERROR jsonl.parse: Failed to parse Org file: ...
OK counts.db_vs_jsonl: Both have 0 records          <-- reassuring and wrong
OK sqlite.integrity_check

$ obr sync --flush-only --force      # the §3.1 "fix"
Exported:
  0 issues
$ grep -c '^\* ' .beads/issues.org  -> 0
$ wc -c .beads/issues.org           -> 78
```

**The corruption playbook destroys everything here.** The empty-DB guard at `src/sync/mod.rs:1298-1306`
exists to prevent exactly this ("Refusing to export empty database over non-empty JSONL file... This
would result in data loss!") but `--force` bypasses it, and `--force` is mandatory for the §3.1 fix.
Nothing in obr's output distinguishes "DB intact, file corrupt" from "DB gone, file corrupt".

Partial consolation: that destructive export *did* snapshot the corrupt file first
(`issues.20260806_191203.org`, 4 recoverable records), and `history restore` brings it back. But
`obr sync --import-only --force` on it then fails on the unbypassable parse guard, so the DB stays at
0 and the workspace is back to "hand-edit the .org file". The only real recovery here is
`git checkout .beads/issues.org` — which obr never mentions and, per its own charter, will never run.

### 3.6 What is lost — events confirmed never exported

```
$ python3 -c "import sqlite3;c=sqlite3.connect('.beads/beads.db');print(c.execute('select count(*) from events').fetchone())"
(245,)
$ grep -ci event .beads/issues.org
0
```

`grep -n "events" src/sync/mod.rs` matches only a test name (`:3316`). `src/sync/org_bridge.rs` (900
lines, the org writer) contains no case-insensitive match for "event" outside unrelated comments.
**Events/audit history exist only in the gitignored SQLite DB, are never in the export file, and are
therefore never in any `.br_history` backup.** Any recovery that rebuilds the DB from the file
discards the entire audit trail. Comments, dependencies and labels *are* exported.

### 3.7 Restore + import does not converge

The documented `history restore` → `obr sync --import-only --force` sequence:

- **Does** roll back field-level edits on issues present in both (w7: title reverted from
  "IMPORTANT NEW TITLE" to "original title" — `force_upsert: args.force` at
  `src/cli/commands/sync.rs:937` overwrites unconditionally, with **no DB snapshot taken anywhere**).
- **Does not** roll back creations. Import is additive; it never deletes DB rows absent from the file.
  w6: after restore+import, DB=6 / file=3, and the next flush wrote all 6 back out.

So the "restore" is really a **merge of (old file ∪ current DB)**, not a rollback. It is destructive
to the DB in one direction and inert in the other, and no backup of the DB exists to undo it.

---

## 4. Quantifying the three limits of `backup_before_export` (`src/sync/history.rs:45-99`)

### 4.1 (a) One-second filename granularity + overwriting `fs::copy` — CONFIRMED DESTRUCTIVE

`src/sync/history.rs:72` formats `Utc::now().format("%Y%m%d_%H%M%S")`; `:77-78` build
`{stem}.{ts}.{ext}`; `:93` is `fs::copy(target_path, &backup_path)` which silently overwrites.

Controlled measurement, w3b — 40 create+flush cycles as fast as the CLI allows:

```
40 export cycles in 3368 ms
backup files retained: 5
  issues.20260806_190625.org -> 0
  issues.20260806_190626.org -> 17
  issues.20260806_190627.org -> 29
  issues.20260806_190628.org -> 37
  issues.20260806_190629.org -> 39
current: 40
```

**35 of 40 snapshots (87.5%) silently destroyed.** Retained states are `{0, 17, 29, 37, 39}`; the gap
between the first two is 17 mutations. Effective granularity is *one snapshot per wall-clock second in
which an export occurred*, holding whichever content the last `fs::copy` of that second wrote.

Smaller replication in w3 (7 flushes, 7 distinct states) retained 2 files holding states {5, 8}.

Under the concurrent-create scenario this collapses to its worst case: N concurrent flushes all land
in the same second, so at most one backup can exist for the entire burst — and per §2 the real count
is zero, because auto-flush never calls the backup at all.

**The test suite already knows.** Eleven `thread::sleep(Duration::from_millis(1100))` calls across
`tests/e2e_history.rs` (`:168, :271, :568, :661`) and `tests/e2e_history_restore_prune.rs`
(`:107, :329, :368, :411, :468, :602, :705`), commented "Ensure different timestamps". The
granularity limit was normalised as a test workaround rather than treated as a defect.

### 4.2 (b) `max_count = 100` / `max_age_days = 30`

Defaults at `src/sync/history.rs:22-29`. `rotate_history` (`:107-138`) runs after every backup
(`:97`), deleting entries where `entry.timestamp < now - max_age_days` or `idx >= max_count`
(`:124-131`, list pre-sorted newest-first at `:211`).

Rotation enforced, verified by planting 150 synthetic backups then triggering one real one:

```
planted: 151
after one real backup+rotate: 100 files
```

**How much does 100 backups actually cover in realistic agent use?** The answer is dominated by §2,
not by the retention numbers:

- **Default agent workflow (create/update/close/dep/label/comment):** 0 backups produced, ever. The
  retention window is irrelevant.
- **Workflow that runs explicit `obr sync` after each change:** the one-per-second collapse (§4.1)
  means 100 files = 100 *distinct seconds containing an export*, not 100 mutations. Measured
  throughput was ~12 export cycles/sec, so 100 backups ≈ the last ~100 active seconds, but sampling
  only ~8% of states within them. **Is 100 backups one working session? In wall-clock terms it is
  generous (bursty activity stretches it over hours). In state-coverage terms it is worthless: you
  can only ever roll back to a coarsely sampled sequence with 15–20-mutation gaps.**

The two limits interact perversely: the same-second collision *extends* the count window by throwing
away the states that would have filled it.

Secondary hazards in this code, unexercised by tests but real:

- `rotate_history` deletes with `fs::remove_file(&entry.path).map_err(BeadsError::Io)?`
  (`src/sync/history.rs:128`) — a bare `?`. Two concurrent flushes racing to delete the same
  over-limit entry make one of them return Err from `backup_before_export`, which aborts the whole
  export at `src/sync/mod.rs:1285`. On the auto-flush path that abort is swallowed at `debug!` and the
  mutation silently never reaches the file. (`prune_backups` at `:295-299` correctly downgrades this to
  `tracing::warn!`; `rotate_history` does not. The inconsistency is unexplained.)
- `files_are_identical` (`:226-267`) opens the latest backup with `File::open(...)?` — if a
  concurrent rotate deleted it between the `list_backups` scan and the open, same abort.
- `list_backups` requires exactly 15 timestamp characters (`:190-192`) and a `.jsonl`/`.org`
  extension (`:175-180`), so any hand-renamed backup vanishes from `obr history list` — but
  `history restore` will still happily copy it, since restore never consults `list_backups`.

### 4.3 (c) Dedup-against-latest — CONFIRMED it can skip the snapshot you need

Fully covered in §3.3. Summary of the trace through `src/sync/history.rs:83-91`: when a losing rename
moves the file *backwards* to a state that happens to equal the newest backup, `files_are_identical`
returns true and `backup_before_export` returns `Ok(())` at `:89` without copying. The intermediate
(good) state was never captured because auto-flush does not back up, so it exists in neither the file
nor the history. Reproduced: `*** NONE - dedup skipped the snapshot ***`.

### 4.4 Other structural point: the current good state is never in history

`backup_before_export` snapshots the file **about to be replaced** (`:93`), never the file just
written. Therefore `.br_history` is always at least one export behind, and the newest good state is
never recoverable from it. In wB the pre-corruption 5-issue file was never backed up for exactly this
reason.

Two smaller observations:

- Backups inherit source permissions via `fs::copy` — observed `-rw-------` on backups vs
  `-rw-r--r--` on `.beads/issues.org`, i.e. the mode is whatever the last writer left.
- `backup_before_export` early-returns if the target doesn't exist (`:56-58`), so a deleted
  `issues.org` is never snapshotted before the export that recreates it.

---

## 5. Backups outside the export path (Q4)

| Operation | Snapshot taken? | Evidence |
|---|---|---|
| Before **import** (`sync --import-only [--force]`) | **NO** | w8: history file count identical before/after; w2: 2000-issue import created no `.br_history` at all |
| Before `sync --merge` (force export, bypasses both guards) | **YES** | backup at `src/sync/mod.rs:1285` precedes the guards at `:1293-1340`; w8 gained `issues.20260806_190948.org` |
| Before `delete` / `delete --hard` | **NO** | auto-flush path (§2); w8 history unchanged after `obr delete` |
| Before `history restore` (destroys the current file) | **NO** | `src/cli/commands/history.rs:257`, §1.2 |
| Of the **SQLite DB**, ever | **NO** | nothing in the tree snapshots `beads.db`; `force_upsert` at `src/cli/commands/sync.rs:937` overwrites rows unconditionally |
| A refused export | **YES** (unhelpfully) | the snapshot is of the already-bad file — see §3.2 |

**Yes: import can destroy state with no snapshot.** `obr sync --import-only --force` — the exact
command `history restore` instructs you to run — sets `force_upsert: true` and `skip_prefix_validation:
args.force && !args.rename_prefix` (`src/cli/commands/sync.rs:933-937`), overwriting live DB rows from
the file. Demonstrated in w7: the DB's "IMPORTANT NEW TITLE" was replaced by the backup's "original
title", with no DB backup and no file backup of the discarded state.

---

## 6. Config plumbing (Q5)

**There is none.** `HistoryConfig` (`src/sync/history.rs:16-29`) is constructed in exactly three
production places and all three are `::default()`:

- `src/cli/commands/sync.rs:584` (`--flush-only`)
- `src/cli/commands/sync.rs:1219` (`--merge`)
- `src/sync/mod.rs:1942-1946` and `src/config/mod.rs:342-348`, via `..Default::default()` on
  `ExportConfig` (derived `Default`, `src/sync/mod.rs:35-37`)

`grep -rn '"history' src/` returns exactly one hit and it is `beads_dir.join("history")` in a path
test (`src/sync/path.rs:1059`). No key registry, no YAML deserialisation, no CLI flag.

`PLAN_TO_PORT_BEADS_WITH_SQLITE_AND_ISSUES_JSONL_TO_RUST.md:895-897` specifies `history.max_count`,
`history.max_age_days`, `history.enabled` as configurable. **None was implemented.**

**Worse: `obr config set` accepts them, persists them, and ignores them.**

```
$ obr config set history.enabled false
INFO ...: Config updated key="history.enabled" new_value="false"
Set history.enabled=false in .../.beads/config.yaml
$ obr config set history.max_count 5
Set history.max_count=5 ...
$ obr config get history.enabled
false
$ cat .beads/config.yaml
history:
  enabled: 'false'
  max_count: '5'

# with history.enabled=false, does a flush still back up?
before: 2 files
$ obr sync --flush-only --force
after: 3 files            <-- backup created anyway
```

**Can history be silently disabled?** Not by config — but it *is* silently disabled for the entire
auto-flush path (§2), which is a far larger blast radius than any config key would have been. It is
also silently disabled for any export whose target resolves outside `.beads/` (the
`starts_with` gate at `src/sync/mod.rs:1284`), including `--allow-external-jsonl` and `BEADS_JSONL`
paths — verified in §3.4 where the external-path run left the history dir untouched.

**Documentation state:** `docs/CLI_REFERENCE.md:770-783` documents only `list` and `restore`, omitting
`diff`, `prune`, and `--force`, and says (correctly, by accident) that backups happen during
`obr sync --flush-only`. `docs/SYNC_SAFETY.md:95-97` and `docs/ARCHITECTURE.md:179` imply backups on
every export. `docs/TROUBLESHOOTING.md:391-393` prescribes `br history list` / `br history restore`
for a corrupt file — i.e. the wrong tool (§3.1) invoked with the wrong binary name.
`docs/E2E_COVERAGE_MATRIX.md:175-179,239-240` claims `history restore` and `history prune` have no
e2e coverage; that doc is stale — `tests/e2e_history_restore_prune.rs` has 18 tests.

**`.br_history` is not gitignored by `obr init`.** `src/cli/commands/init.rs:80-96` writes an 11-line
`.beads/.gitignore` covering `*.db`, `*.db-shm`, `*.db-wal`, `*.lock`, `last-touched`, `*.tmp` — and
nothing else. Verified:

```
$ git add -A .beads && git status --short
A  .beads/.br_history/issues.20260806_190219.org
A  .beads/.br_history/issues.20260806_190229.org
A  .beads/.gitignore
A  .beads/config.yaml
A  .beads/issues.org
A  .beads/metadata.json
```

Meanwhile obr's own hand-maintained `/Users/johnw/src/obr/.beads/.gitignore` *does* contain
`.br_history/` under the comment "# Local history backups". So the obr developers' workspace is
protected and every workspace `obr init` creates is not: up to 100 near-complete copies of the issue
database get committed (in w2 that would be ~50 MB), leaking closed/deleted issue content into git
history. `PLAN_TO_PORT...md:892` explicitly promised "`.br_history/` is automatically added to
`.gitignore` during `br init`".

---

## 7. VERDICT — the recovery story, rewritten

### 7.1 Per-defect classification

| Defect | Is `history restore` the answer? | What actually recovers it | Category |
|---|---|---|---|
| **FILE CORRUPTION** (fixed temp name) | No — the only backup would be stale by however many auto-flushes have run; often no backup exists at all | `obr sync --flush-only --force` (one command, lossless, DB is intact) | **(ii/iii) possible but undiscoverable.** Error is `hint: null`; `doctor` reports ERROR with no remediation; nothing anywhere names the fix, and `--force` is required but never mentioned. |
| **EXPORT WEDGE** (dedup ghost ID) | **No — actively harmful.** Restore reinstalls the wedged file; the only backup was taken *of* the wedged file | `obr sync --flush-only --force` (one command) | **(iii) possible only with knowledge no output provides.** `obr create` exits 0 while losing data; `doctor` exits 0 with a WARN. The one good error text only appears if you happen to run `obr sync` manually. |
| **CONCURRENT FLUSH LOSS** | No — dedup skips exactly the snapshot you need (§3.3/§4.3) | Nothing, if you notice nothing: the next mutation's auto-flush re-exports the full DB and heals the file | **(i) automatic** while the DB lives; **(iv) impossible** once the bad file is committed and the DB is discarded — the good state was never in history. |
| **Fresh clone, DB gone, file corrupt** | History is empty in a clone (unless `.br_history` was committed, which `obr init` permits) | `git checkout .beads/issues.org` — obr will never say this | **(iv) impossible from obr.** And the §3.1 playbook (`sync --flush-only --force`) *destroys the remaining data* here (5 issues → 78 bytes). |

So: R5's "the workspace is unusable until a human hand-edits the file" is **wrong in the common case**
(DB intact → one command) and **right in the clone case** (and there `--force` makes it worse). The
dossier states neither. The severity is not lowered to "a discoverability defect" — it is *split*: a
discoverability defect when the DB survives, and a genuine unrecoverable-loss defect when it does not,
with the same error message and no way to tell them apart.

### 7.2 Does the existing mechanism deserve the credit the gap-fill hypothesised?

Largely no, and for a reason nobody had looked for. `obr history restore` is not an under-advertised
recovery path; it is a **non-functioning** one:

1. It produces no snapshots on the path 100% of agent mutations take (§2).
2. When it does snapshot, it drops ~88% of states to same-second collisions (§4.1) and can skip the
   one you need via dedup (§4.3).
3. It never captures the current good state, only the previous one (§4.4).
4. It never snapshots the DB (§5), never snapshots before import (§5), and never snapshots the file
   it is itself about to destroy (§1.2).
5. It restores the file only, and the follow-up import it prescribes cannot roll back creations —
   the result is a union, not a rollback (§3.7).
6. It never captures events (§3.6).
7. Its `file` argument is an unvalidated path, so a typo or a hostile string copies `/etc/passwd` or
   `.git/config` over the durable artifact (§1.3).
8. Its own next-step instruction names a binary that does not exist (§1.2).

### 7.3 Minimum hardening to make the existing mechanism a real recovery story

Ordered by (impact ÷ cost). Items 1–3 are one-to-ten-line changes.

1. **Canonicalise `beads_dir` in `run_auto_flush`** (`src/main.rs:261`) or compare canonical paths at
   `src/sync/mod.rs:1284`. **One line. Turns the entire mechanism on.** Ship a test that mutates
   *without* `--no-auto-flush` and asserts a backup appeared — the exact test the suite avoids
   (§2.6).
2. **Unique backup filenames.** Append PID + a monotonic counter, or use `%Y%m%d_%H%M%S%.6f`, and
   switch `fs::copy` (`src/sync/history.rs:93`) to a create-new (`O_EXCL`) copy so a collision is an
   error rather than a silent overwrite. `list_backups`'s 15-char timestamp check (`:190`) must widen
   with it. **Recovers the 87.5% of snapshots currently destroyed** and removes the 11 test sleeps.
3. **Name the fix in the errors.** Three strings, each attached to a condition already detected:
   - parse failure (`VALIDATION_FAILED` from the org reader) → hint:
     `"The export file is corrupt. If your database is intact (obr doctor: sqlite.integrity_check OK, counts.db_vs_jsonl db>0), run: obr sync --flush-only --force. If the DB is empty, do NOT use --force — restore the file from git or: obr history list / obr history restore <file> --force."`
   - stale-DB guard (`src/sync/mod.rs:1319`) already has the best message in the codebase; **just
     surface it** — stop swallowing auto-flush failures at `debug!` (`src/main.rs:293-296`) and emit
     at `warn!` with a non-zero exit or at minimum a stderr line.
   - `doctor`: promote `counts.db_vs_jsonl` from WARN to ERROR when the divergence is one-directional
     with unexported IDs, and attach the same remediation strings.
4. **Snapshot before every destructive operation that currently doesn't:** before `history restore`
   overwrites the file, before `sync --import-only --force` overwrites DB rows (a `beads.db` copy —
   the only durable protection for the 245-event audit trail that the file format cannot carry), and
   before `delete --hard`.
5. **Validate the `file` argument** in `restore_backup` (`src/cli/commands/history.rs:240`) and
   `diff_backup` (`:137`): reject absolute paths, reject any `..` component, and call the existing
   `validate_no_git_path`. Roughly five lines reusing `src/sync/path.rs`.
6. **Add `.br_history/` to the `.gitignore` `obr init` writes** (`src/cli/commands/init.rs:83-95`) —
   one line, matching what obr's own repo already does.
7. **Either implement or reject `history.enabled/max_count/max_age_days`.** Today `obr config set`
   silently accepts and ignores them (§6), which is worse than not supporting them.
8. **`br` → `obr`** in the three restore output strings (`src/cli/commands/history.rs:265, 278, 286`),
   the `doctor` header, and `docs/TROUBLESHOOTING.md:391-393`.

### 7.4 Cost comparison against `doctor --repair`

`doctor --repair` as the dossier proposes it would have to: detect corruption, decide whether the DB
or the file is authoritative, reconstruct partial records, and roll back. That is a new subsystem
with its own failure modes, and — critically — **it would need exactly the same authoritative-source
decision that item 3 above encodes in three sentences of prose.** The hard part of `doctor --repair`
is not the repair; it is knowing which side to trust, and that judgement cannot be automated safely
(§3.5 shows the wrong choice is unrecoverable).

Items 1, 2, 3, 5 and 6 are together well under 100 lines of production change plus tests, and they
deliver:

- backups that exist at all (1),
- backups that are complete rather than 12%-sampled (2),
- a user who is told which one-command fix applies to their situation (3),
- a restore that cannot be pointed at `/etc/passwd` (5),
- a repo that isn't bloated by its own backups (6).

**Recommendation: Q9's conclusion should be inverted.** The highest-value work is not a new
`doctor --repair` feature; it is the one-line path fix in `src/main.rs:261`, unique backup filenames,
and putting the remediation command into the error text — after which `doctor --repair` becomes a
convenience wrapper over a mechanism that finally works, rather than a replacement for one that
never did.

---

## Appendix: workspace map

| Dir | Purpose |
|---|---|
| `w1` | baseline; `.gitignore` inspection; relative-vs-absolute `BEADS_DIR` A/B; path-escape tests; DEBUG trace |
| `w2` | 2000-issue workspace; 244 concurrent creates; corruption injection + one-command recovery |
| `w3`, `w3b` | same-second collision quantification (7 flushes → 2 files; 40 cycles → 5 files) |
| `w4`, `w5` | concurrent-flush-loss simulation; dedup-skips-the-needed-snapshot proof |
| `w6` | export wedge via content-hash dedup; restore-makes-it-permanent; `--flush-only --force` fix |
| `w7` | restore + `import --force` regresses DB fields with no DB backup |
| `w8` | which operations snapshot (import: no, merge: yes, delete: no); config-key no-op proof |
| `w9` | `max_count=100` rotation enforcement (151 → 100); concurrent explicit sync failure rates |
| `wA` | concurrent sync failure isolation (internal vs external JSONL path) |
| `wB` | fresh-clone simulation: `--force` export destroys a corrupt-but-recoverable file |
