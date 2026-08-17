# Web research: SQLite in Rust CLIs — idioms, PRAGMA hygiene, WAL, multi-process contention, migrations, alternatives

Research target: `/Users/johnw/src/obr` (crate `beads_rust`, binary `br`) — a Rust port of Steve Yegge's Go `beads`
issue tracker. SQLite is the local speed cache; JSONL-in-git is the sync/source-of-truth format. `br` is a
**short-lived CLI process** that many agents/humans may run **concurrently against the same `.beads/*.db` file**.
That shape — many independent OS processes, each opening the DB for tens of milliseconds, then exiting — drives
almost every recommendation below.

Every nontrivial claim carries its source URL. Where a source was unreachable that is stated explicitly.

---

## 0. Executive summary of the research

1. **SQLite is the right choice for `br`.** It is essentially the only mature embedded engine that supports
   *multiple concurrent OS processes* on one file. redb, sled, LevelDB, RocksDB, and Pebble all take an
   exclusive per-process file lock and explicitly do not support multi-process access
   (§7). SQLite's own "Appropriate Uses" page endorses exactly this shape
   ([whentouse.html](https://www.sqlite.org/whentouse.html)).
2. **WAL is correct here, but not free.** WAL is the only journal mode that lets a reader (`br list`) run while
   a writer (`br create`) commits. It is persistent in the file header, requires all processes on the same host,
   and leaves `-wal`/`-shm` companion files ([wal.html](https://www.sqlite.org/wal.html)).
3. **`BEGIN IMMEDIATE` is mandatory, not optional.** A DEFERRED transaction that reads and *then* writes gets an
   instant `SQLITE_BUSY`, with the busy handler deliberately bypassed as a deadlock-avoidance measure
   ([busy_handler.html](https://www.sqlite.org/c3ref/busy_handler.html), §4). obr already does this
   (`src/storage/sqlite.rs:154`).
4. **PRAGMA hygiene is a per-connection duty, and obr currently has a real gap here.** `foreign_keys`,
   `synchronous`, `temp_store`, `cache_size`, and `busy_timeout` are all connection-scoped and must be re-applied
   on *every* open. obr applies them only inside `apply_schema()`, which is skipped whenever
   `user_version >= CURRENT_SCHEMA_VERSION` — i.e. on essentially every real invocation (§8.1).
5. **Hand-rolled `user_version` migrations are a legitimate, well-precedented choice** (it is literally what the
   most popular SQLite-specific Rust migration crate does), but obr's version counter is stuck at 1 while the
   real migration logic probes columns, which forfeits the benefit (§6, §8.2).

---

## 1. SQLite as an application file format

Primary source: <https://www.sqlite.org/appfileformat.html>

The page's thesis: *"An SQLite database file with a defined schema often makes an excellent application file
format."* It positions SQLite as a fourth option alongside (a) fully custom binary formats, (b) pile-of-files
formats (Git is the named example), and (c) wrapped pile-of-files (EPUB, ODT — a zip of a pile of files).

Twelve claimed advantages, condensed:

| # | Advantage | Notable detail |
|---|---|---|
| 1 | Simplified application development | *"No new code is needed for reading or writing the application file."* |
| 2 | Single-file documents | Supports a 4-byte **Application ID** in the header so `file(1)` can identify the format |
| 3 | High-level query language | Express "what" not "how" |
| 4 | Accessible content | *"An SQLite database file is not an opaque blob."* Format stable since 2004; recommended by the **US Library of Congress** for long-term digital preservation |
| 5 | Cross-platform | 32/64-bit, endian-neutral, UTF-8/UTF-16 handled automatically |
| 6 | Atomic transactions | *"Writes to an SQLite database are atomic. They either happen completely or not at all, even during system crashes or power failures."* |
| 7 | Incremental & continuous updates | *"Only those parts of the file that actually change are written out to disk"* — no full rewrite, so File/Save becomes superfluous |
| 8 | Easily extensible | Add tables/columns without breaking old queries |
| 9 | Performance | *"SQLite can often read and write smaller BLOBs (less than about 100KB in size) from its database faster than those same blobs can be read or written as separate files from the filesystem"* (the "35% Faster Than The Filesystem" result) |
| 10 | **Concurrent use by multiple processes** | SQLite auto-coordinates multi-thread/multi-process access; writes serialize but complete in milliseconds. The page calls hand-rolled concurrency logic a *"notorious bug-magnet"* |
| 11 | Multiple programming languages | Bindings everywhere — relevant to obr, which must interoperate with the Go `bd` binary |
| 12 | Better applications | Fred Brooks: *"Show me your tables, and I won't usually need your flowcharts; they'll be obvious."* |

Caveats stated on the page:
- *"SQLite is robust against maliciously malformed database files and SQL inputs. An attacker will not be able to
  provoke a memory error by corrupting an SQLite database used as an application file."*
- But: *"There are attacks that a clever attacker can carry out against an application by tricking a user to open
  an application file that is an SQLite database."* → see the "defense against dark arts" doc, untrusted-database
  section.
- Closing posture is measured: *"SQLite is not the perfect application file format for every situation. But in
  many cases, SQLite is a far better choice than either a custom file format, a pile-of-files, or a wrapped
  pile-of-files."*

**Tension relevant to obr:** the WAL doc explicitly says the `-wal`/`-shm` companions *"can make SQLite less
appealing for use as an application file-format"* (<https://www.sqlite.org/wal.html>). obr sidesteps this
cleanly — the *shipped* format is JSONL-in-git and the `.db` is gitignored (`.beads/.gitignore` lists `*.db`,
`*.db?*`, `*.db-journal`, `*.db-wal`, `*.db-shm`), so obr gets WAL's concurrency without paying the
application-file-format cost. That is the right architecture and worth stating explicitly in design docs.

Also on the same theme: <https://www.sqlite.org/whentouse.html> — *"SQLite does not compete with client/server
databases. SQLite competes with fopen()."*

---

## 2. WAL vs rollback journal for CLI-lifetime processes

Primary source: <https://www.sqlite.org/wal.html>

### 2.1 Mechanism
*"The original content is preserved in the database file and the changes are appended into a separate WAL file. A
COMMIT occurs when a special record indicating a commit is appended to the WAL. Thus a COMMIT can happen without
ever writing to the original database, which allows readers to continue operating from the original unaltered
database while changes are simultaneously being committed into the WAL."*

Three primitive operations instead of two: **reading, writing, checkpointing**.

### 2.2 Concurrency properties
- Readers do not block writers; writers do not block readers.
- **Still exactly one writer at a time.** Writers append to the end of the WAL.
- Each reader records an "end mark" at transaction start → stable snapshot for the whole read transaction.
- A checkpoint runs concurrently with readers but must stop at the end mark of the oldest live reader
  ("checkpoint starvation" if a long reader lingers).

### 2.3 The shared-memory constraint (decisive for multi-process)
A **wal-index** lives in shared memory (implemented as an ordinary mmapped `-shm` file, not `/dev/shm`).

> *"The wal-index greatly improves the performance of readers, but the use of shared memory means that all readers
> must exist on the same machine. This is why the write-ahead log implementation will not work on a network
> filesystem."*

Rationale given for using a real file rather than anonymous shared memory: chroot'd processes would otherwise see
different shared memory (→ corruption), and there is no portable nameless-shared-memory primitive across Unix
flavors and Windows. The `-shm` file rarely exceeds 32 KiB, is never synced, and is deleted when the last
connection disconnects.

**Implication for obr:** a `.beads/` directory on NFS/SMB/Dropbox or any network share is unsupported for concurrent access.
This is worth an explicit doc statement and possibly a `br doctor` warning.

### 2.4 Persistence — the big CLI win
> *"Unlike the other journaling modes, `PRAGMA journal_mode=WAL` is persistent. If a process sets WAL mode, then
> closes and reopens the database, the database will come back in WAL mode."*

> *"The WAL journal mode will be set on all connections to the same database file if it is set on any one
> connection."*

Confirmed on the PRAGMA page: *"The WAL journaling mode is persistent; after being set it stays in effect across
multiple database connections and after closing and reopening the database."*
(<https://www.sqlite.org/pragma.html>)

So a short-lived CLI need only set it once at creation. It is nonetheless harmless (and cheap — a no-op if already
WAL) to re-issue it; sqlx deliberately does *not* auto-set journal_mode precisely because flipping a live DB
*out of* WAL requires an exclusive lock and can throw `SQLITE_BUSY`
(<https://docs.rs/sqlx/latest/sqlx/sqlite/struct.SqliteConnectOptions.html>). The danger is only in *changing*
modes, not in re-asserting the same one.

### 2.5 Checkpointing
- Default auto-checkpoint threshold: **1000 pages** (~4 MB at 4 KiB pages).
  *"By default, SQLite will automatically checkpoint whenever a COMMIT occurs that causes the WAL file to be 1000
  pages or more in size, or when the last database connection on a database file closes."*
- PRAGMA doc: *"Autocheckpointing is enabled by default with an interval of 1000 or
  SQLITE_DEFAULT_WAL_AUTOCHECKPOINT."* and *"All automatic checkpoints are PASSIVE."*
  (<https://www.sqlite.org/pragma.html>)
- Checkpoint does **not** truncate the WAL by default; it overwrites from the start once fully transferred and no
  readers are pinned. `PRAGMA journal_size_limit` or `wal_checkpoint(TRUNCATE)` bounds the file size.
- Checkpoint modes: PASSIVE (default), FULL, RESTART, TRUNCATE.

**For a CLI:** every `br` invocation is "the last database connection" on exit *if no other `br` is running*, so
in the common single-user case the WAL gets checkpointed and deleted on each exit. Under agent fan-out (several
`br` processes overlapping) the WAL persists between invocations — which is fine and is exactly what makes WAL
worth having.

### 2.6 `-wal`/`-shm` file lifecycle
> *"The WAL file exists for as long as any database connection has the database open. Usually, the WAL file is
> deleted automatically when the last connection to the database closes."*

> *"The WAL file is part of the persistent state of the database and should be kept with the database if the
> database is copied or moved. If a database file is separated from its WAL file, then transactions that were
> previously committed to the database might be lost, or the database file might become corrupted."*

> *"The only safe way to remove a WAL file is to open the database file using one of the sqlite3_open() interfaces
> then immediately close the database using sqlite3_close()."*

An unclean process exit (SIGKILL, panic-abort, OOM) leaves the WAL behind; the next opener performs recovery.

### 2.7 Full list of WAL disadvantages (verbatim structure from wal.html)
1. All processes must be on the same host — no network filesystems.
2. Transactions across multiple ATTACHed databases are atomic per-database but **not atomic as a set**.
3. `page_size` cannot be changed after entering WAL mode (needs a round-trip through rollback mode / VACUUM).
4. Read-only WAL databases are problematic (see §2.8).
5. *"perhaps 1% or 2% slower"* than rollback journal for read-mostly, rarely-write workloads.
6. Extra `-wal`/`-shm` files *"can make SQLite less appealing for use as an application file-format."*
7. Checkpointing is an extra operation developers must be mindful of.
8. Historically bad for very large transactions: *"For transactions larger than about 100 megabytes, traditional
   rollback journal modes will likely be faster. For transactions in excess of a gigabyte, WAL mode may fail with
   an I/O or disk-full error."* — **fixed in 3.11.0 (2016-02-15)**: *"WAL mode works as efficiently with large
   transactions as does rollback mode."* (Relevant to obr's bulk JSONL import, which is a single large
   transaction; with rusqlite 0.38 bundling SQLite 3.51.x this is a non-issue.)

### 2.8 Read-only media
Pre-3.22.0, reading a WAL database required write access. Since **3.22.0 (2018-01-22)** a WAL DB on read-only
media is readable if any of: the `-shm`/`-wal` files already exist and are readable; there is write permission on
the containing directory; or the connection uses the `immutable` URI parameter. Best practice for read-only
distribution remains `PRAGMA journal_mode=DELETE` first.

### 2.9 Backwards compatibility trap
While in WAL mode, header bytes 18–19 are bumped from 1 to 2. Pre-3.7.0 SQLite opening such a file reports
*"file is encrypted or is not a database"*. `PRAGMA journal_mode=DELETE` reverts them. Relevant if obr's `.db`
were ever handed to an ancient tool — not a practical concern in 2026, but worth knowing when debugging weird
"not a database" reports.

### 2.10 The WAL-reset bug (2026) — worth knowing, obr is safe
<https://www.sqlite.org/howtocorrupt.html> §8.1 and wal.html:

> *"When there are two or more database connections in separate threads or processes, both open on the same
> WAL-mode database, and if both connections try to write or run checkpoint at the same time, then there is a race
> condition that might corrupt the database file. This is the WAL-reset bug. It existed in all versions of SQLite
> from 3.7.0 through 3.51.2."*

Fixed in **3.51.3 (2026-03-13)**; backports for 3.44.6 and 3.50.7. SQLite's own risk assessment: could not be
reproduced organically, only via `sqlite3_test_control()` hooks; estimated occurrence *"less than or equal to the
expected occurrence rate of SSD malfunctions and/or cosmic-ray hits."*

**obr relevance:** `Cargo.toml:22` pins `rusqlite = { version = "0.38", features = ["bundled", ...] }`, and
rusqlite 0.38 / libsqlite3-sys 0.36 bundles **SQLite 3.51.1** — i.e. *inside* the affected range
(<https://docs.rs/crate/rusqlite/latest>, <https://github.com/rusqlite/rusqlite>). obr is a multi-process
WAL-mode writer, which is exactly the trigger shape. This is an argument for tracking rusqlite/libsqlite3-sys
bumps to whichever release vendors ≥ 3.51.3. Given the stated occurrence rate this is hygiene, not an emergency.

### 2.11 Verdict for CLI-lifetime processes
Community sources split on this. For a *single-writer batch job with no concurrent readers*, WAL buys nothing
("WAL won't hurt, but you're not gaining anything either" — <https://blog.sqlite.ai/journal-modes-in-sqlite>).
But obr's premise is concurrent agents, so WAL is the correct default. The residual costs (extra files,
same-host requirement, checkpoint management) are all acceptable given the `.db` is a rebuildable cache and is
gitignored.

Additional readable background: <https://fly.io/blog/sqlite-internals-wal/> (how WAL scales read concurrency).

---

## 3. PRAGMA hygiene: persistent vs per-connection

Primary source: <https://www.sqlite.org/pragma.html>

### 3.1 The persistence split — the single most important operational fact

**Persistent (stored in the database file; set once at creation):**
`journal_mode` (only for the WAL ↔ non-WAL transition), `page_size` (needs VACUUM to change on an existing DB),
`auto_vacuum` (incremental transitions need VACUUM), `encoding` (creation-time only), `application_id`,
`user_version`, `legacy_file_format`.

**Per-connection / session-only (MUST be re-applied on every single open):**
`foreign_keys`, `synchronous`, `busy_timeout`, `cache_size`, `temp_store`, `mmap_size`, `locking_mode`,
`wal_autocheckpoint`, `analysis_limit`, `recursive_triggers`, `defer_foreign_keys`, `automatic_index`,
`case_sensitive_like`, `max_page_count`, `ignore_check_constraints`, `reverse_unordered_selects`.

Sources: the SQLite pragma reference itself plus the canonical mailing-list enumeration
(<https://sqlite-users.sqlite.narkive.com/e3iOB8Z3/sqlite-which-pragmas-are-persistent>) — the latter is a user
forum post, so treat it as corroborating rather than normative, but it agrees with pragma.html clause by clause.

`cache_size` is explicitly documented as session-only: *"When you change the cache size using the cache_size
pragma, the change only endures for the current session. The cache size reverts to the default value when the
database is closed and reopened."* The persistent variant `default_cache_size` exists but is deprecated
("you should not use this pragma").

### 3.2 `foreign_keys`
- *"As of SQLite version 3.6.19, the default setting for foreign key enforcement is OFF. However, that might
  change in a future release of SQLite."*
- *"To minimize future problems, applications should set the foreign key enforcement flag as required by the
  application and not depend on the default setting."*
- *"This pragma is a no-op within a transaction; foreign key constraint enforcement may only be enabled or
  disabled when there is no pending BEGIN or SAVEPOINT."* → set it **before** opening any transaction.
- *"Changing the foreign_keys setting affects the execution of all statements prepared using the database
  connection, including those prepared before the setting was changed."*

**Consequence:** if `PRAGMA foreign_keys=ON` is not issued on a connection, every `FOREIGN KEY ... ON DELETE
CASCADE` in the schema is silently inert and every FK constraint is unenforced. See §8.1 — this is obr's live gap.

### 3.3 `synchronous`
Values OFF(0) / NORMAL(1) / FULL(2) / EXTRA(3). Key documented sentences:

> *"WAL mode is safe from corruption with synchronous=NORMAL... WAL mode is always consistent with
> synchronous=NORMAL, but WAL mode does lose durability. A transaction committed in WAL mode with
> synchronous=NORMAL might roll back following a power loss or system crash. Transactions are durable across
> application crashes regardless of the synchronous setting or journal mode."*

> *"The synchronous=NORMAL setting provides the best balance between performance and safety for most applications
> running in WAL mode. You lose durability across power lose [sic] with synchronous NORMAL in WAL mode, but that
> is not important for most applications. Transactions are still atomic, consistent, and isolated, which are the
> most important characteristics in most use cases."*

Durability matrix from pragma.html:

| Setting | Rollback mode | WAL mode |
|---|---|---|
| EXTRA | ACID | ACID |
| FULL | Maybe not durable | ACID |
| NORMAL | Maybe not consistent | Maybe not durable |
| OFF | Not consistent | Not consistent |

Also: *"The TEMP schema always has synchronous=OFF"* and attempts to change it are silently ignored.

**obr fit:** `synchronous=NORMAL` is a very good fit because the `.db` is a *derived cache*. Even total loss of the
database is recoverable by re-importing `issues.jsonl`. The relevant risk boundary is: a power loss can roll back
the last few commits, so the JSONL flush (`br sync --flush-only`) is the durability boundary, not the SQLite
commit. That is a design property worth writing down.

Cited empirical magnitude: Purohith, Mohan & Chidambaram found an **11.8×** performance difference from journal
mode alone, **1.5×** from sync mode alone, **5×** from journal size — via
<https://fractaledmind.com/2023/09/21/enhancing-rails-sqlite-performance-metrics/>.

### 3.4 `journal_mode`
Values DELETE (default) / TRUNCATE / PERSIST / MEMORY / WAL / OFF.
- *"Note also that the journal_mode cannot be changed while a transaction is active."*
- In-memory DBs: journal_mode is MEMORY or OFF only; other values are ignored. (This is why obr's tests assert
  `WAL || MEMORY` — `src/storage/schema.rs:549`, `src/storage/sqlite.rs:4654`.)
- `MEMORY` and `OFF` are corruption hazards on crash and are disabled under `SQLITE_DBCONFIG_DEFENSIVE`.

### 3.5 `busy_timeout`
> *"Each database connection can only have a single busy handler. This PRAGMA sets the busy handler for the
> process, possibly overwriting any previously set busy handler."*

Purely connection-scoped. See §4 for full semantics.

### 3.6 `wal_autocheckpoint`
> *"When the write-ahead log is enabled (via the journal_mode pragma) a checkpoint will be run automatically
> whenever the write-ahead log equals or exceeds N pages in length. Setting the auto-checkpoint size to zero or a
> negative value turns auto-checkpointing off."* … *"All automatic checkpoints are PASSIVE."*

### 3.7 `user_version` and `application_id`
- `user_version`: *"gets or sets the value of the user-version integer at offset 60 in the database header. The
  user-version is an integer that is available to applications to use however they want. SQLite makes no use of
  the user-version itself."*
- `application_id`: *"used to query or set the 32-bit signed big-endian 'Application ID' integer located at offset
  68 into the database header. Applications that use SQLite as their application file-format should set the
  Application ID integer to a unique integer so that utilities such as file(1) can determine the specific file
  type rather than just reporting 'SQLite3 Database'."*

obr sets `user_version` (`src/storage/schema.rs:229`) but not `application_id`. Setting an application_id is a
cheap, one-line improvement to observability and to `br doctor`'s ability to reject a wrong-file-type `.db`.

### 3.8 `optimize` and `analysis_limit` — the piece almost everyone misses
Three documented usage rules:
1. *"Applications with short-lived database connections should run 'PRAGMA optimize;' once, just prior to closing
   each database connection."* ← **this is obr's exact shape**
2. Long-lived connections: `PRAGMA optimize=0x10002;` on open, then `PRAGMA optimize;` periodically.
3. *"All applications should run 'PRAGMA optimize;' after a schema change, especially after one or more CREATE
   INDEX statements."*

And: *"This pragma is usually a no-op or nearly so and is very fast."*

Since 3.46.0: *"the recommended way of running ANALYZE is with the PRAGMA optimize command. The PRAGMA optimize
will automatically set a reasonable, temporary analysis limit... Applications that use the PRAGMA optimize instead
of running ANALYZE directly do not need to set an analysis limit."* So the older
`PRAGMA analysis_limit=400; PRAGMA optimize;` pairing recommended by
<https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/> is now redundant on modern SQLite —
bare `PRAGMA optimize` suffices with rusqlite's bundled 3.51.x.

### 3.9 `cache_size`, `temp_store`, `mmap_size`
- `cache_size`: default is `-2000` ≈ 2,048,000 bytes. Negative N means "approximately |N|·1024 bytes". obr uses
  `-8000` (~8 MB) — reasonable, though for a CLI whose whole DB is likely < 8 MB the OS page cache already covers
  this; cj.rs cautions it "might end up wasting memory."
- `temp_store=MEMORY(2)`: *"temporary tables and indices are kept as if they were in pure in-memory databases."*
  Also: *"When the temp_store setting is changed, all existing temporary tables, indices, triggers, and views are
  immediately deleted."*
- `mmap_size`: *"may be a no-op if the prior mmap_size is non-zero and there are other SQL statements running
  concurrently on the same database connection."* Caution: howtocorrupt.html §5 notes mmap I/O widens the blast
  radius of a stray pointer — a corrupting write need not even go through `write()`. For a `#![forbid(unsafe_code)]`
  Rust crate this risk is much lower than for C, but it is not zero (the C library itself is in-process).

### 3.10 `locking_mode`
- *"In NORMAL locking-mode (the default...) a database connection unlocks the database file at the conclusion of
  each read or write transaction. When the locking-mode is set to EXCLUSIVE, the database connection never
  releases file-locks."*
- *"If the locking mode is EXCLUSIVE when first entering WAL journal mode, then the locking mode cannot be changed
  to NORMAL until after exiting WAL journal mode."*
- wal.html adds: with EXCLUSIVE set before first WAL access, **no shared-memory wal-index is created at all** —
  this is how WAL works on VFSes lacking `xShmMap`. Chrome and Firefox both use exclusive locking mode.
- **Never use this in obr**: an EXCLUSIVE-mode connection makes every *other* `br` process fail with
  `SQLITE_BUSY` for the whole process lifetime (wal.html, SQLITE_BUSY case 1).

### 3.11 STRICT tables
`CREATE TABLE t(...) STRICT;` requires SQLite ≥ 3.37.0 (2021-11-27); enforces declared types instead of SQLite's
dynamic affinity. Recommended by <https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/>
"when working with strictly typed languages" — directly applicable to a Rust codebase that already models every
column with a Rust type. Available in obr's bundled 3.51.x.

### 3.12 The consensus per-connection preamble

```sql
PRAGMA journal_mode = WAL;      -- persistent, but harmless to re-assert
PRAGMA busy_timeout = 5000;     -- or higher; per-connection
PRAGMA synchronous = NORMAL;    -- per-connection
PRAGMA foreign_keys = ON;       -- per-connection, no-op inside a transaction
PRAGMA cache_size = -20000;     -- per-connection
PRAGMA temp_store = MEMORY;     -- per-connection
-- and, just before close, for short-lived connections:
PRAGMA optimize;
```

Corroborating sources: <https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/>,
<https://fractaledmind.com/2023/09/07/enhancing-rails-sqlite-fine-tuning/>,
<https://www.sqlite.org/pragma.html>. Community connection pools commonly raise busy_timeout to 30 s to avoid
"database is locked" under contention.

---

## 4. `busy_timeout`, busy handlers, and the DEFERRED-upgrade footgun

### 4.1 What SQLITE_BUSY means
<https://www.sqlite.org/rescode.html>:

> *"The SQLITE_BUSY result code indicates that the database file could not be written (or in some cases read)
> because of concurrent activity by some other database connection, usually a database connection in a separate
> process."*

> *"An SQLite_BUSY error can occur at any point in a transaction: when the transaction is first started, during any
> write or update operations, or when the transaction commits. To avoid encountering SQLITE_BUSY errors in the
> middle of a transaction, the application can use **BEGIN IMMEDIATE** instead of just BEGIN to start a
> transaction. The BEGIN IMMEDIATE command might itself return SQLITE_BUSY, but **if it succeeds, then SQLite
> guarantees that no subsequent operations on the same database through the next COMMIT will return
> SQLITE_BUSY**."*

That last guarantee is the whole argument for `BEGIN IMMEDIATE`, straight from the authoritative source.

`SQLITE_BUSY` vs `SQLITE_LOCKED`: *"SQLITE_BUSY indicates a conflict with a separate database connection, probably
in a separate process, whereas SQLITE_LOCKED indicates a conflict within the same database connection (or
sometimes a database connection with a shared cache)."*

Extended codes:
- **`SQLITE_BUSY_RECOVERY` (261)**: *"another process is busy recovering a WAL mode database file following a
  crash. The SQLITE_BUSY_RECOVERY error code only occurs on WAL mode databases."*
- **`SQLITE_BUSY_SNAPSHOT` (517)**: *"occurs on WAL mode databases when a database connection tries to promote a
  read transaction into a write transaction but finds that another database connection has already written to the
  database and thus invalidated prior reads."* The documented 3-step scenario is exactly "read, someone else
  writes, then try to write."
- **`SQLITE_BUSY_TIMEOUT` (773)**: blocking POSIX advisory lock timeout in the VFS; only with the proprietary
  `SQLITE_ENABLE_SETLK_TIMEOUT` build option.

### 4.2 The busy handler is *deliberately* skipped on potential deadlock
<https://www.sqlite.org/c3ref/busy_handler.html> — the authoritative sentence:

> *"The presence of a busy handler does not guarantee that it will be invoked when there is lock contention. If
> SQLite determines that invoking the busy handler could result in a deadlock, it will go ahead and return
> SQLITE_BUSY to the application instead of invoking the busy handler. Consider a scenario where one process is
> holding a read lock that it is trying to promote to a reserved lock and a second process is holding a reserved
> lock that it is trying to promote to an exclusive lock. The first process cannot proceed because it is blocked
> by the second and the second process cannot proceed because it is blocked by the first. If both processes invoke
> the busy handlers, neither will make any progress. Therefore, SQLite returns SQLITE_BUSY for the first process,
> hoping that this will induce the first process to release its read lock and allow the second process to
> proceed."*

Also from that page:
> *"There can only be a single busy handler defined for each database connection. Setting a new busy handler clears
> any previously set handler."*
> *"Note that calling sqlite3_busy_timeout() or evaluating PRAGMA busy_timeout=N will change the busy handler and
> thus clear any previously set busy handler."*

→ **Never mix `busy_timeout` and a custom `busy_handler`** — the last one set wins and silently clobbers the other.

### 4.3 The transaction-type semantics
<https://www.sqlite.org/lang_transaction.html>:

> *"Transactions can be DEFERRED, IMMEDIATE, or EXCLUSIVE. The default transaction behavior is DEFERRED."*

> *"If the first statement after BEGIN DEFERRED is a SELECT, then a read transaction is started. **Subsequent write
> statements will upgrade the transaction to a write transaction if possible, or return SQLITE_BUSY.**"*

> *"IMMEDIATE causes the database connection to start a new write immediately, without waiting for a write
> statement. The BEGIN IMMEDIATE might fail with SQLITE_BUSY if another write transaction is already active on
> another database connection."*

> *"EXCLUSIVE and IMMEDIATE are the same in WAL mode, but in other journaling modes, EXCLUSIVE prevents other
> database connections from reading the database while the transaction is underway."*

And the general statement:
> *"If a write statement occurs while a read transaction is active, then the read transaction is upgraded to a
> write transaction if possible. If some other database connection has already modified the database or is already
> in the process of modifying the database, then upgrading to a write transaction is not possible and the write
> statement will fail with SQLITE_BUSY."*

### 4.4 The well-known engineering write-up
Bert Hubert, *"What to do about SQLITE_BUSY errors despite setting a timeout"*
<https://berthub.eu/articles/posts/a-brief-post-on-sqlite3-database-locked-despite-timeout/> (Feb 2025; boosted by
Simon Willison at <https://simonwillison.net/2025/Feb/17/sqlite-busy/> and discussed at
<https://news.ycombinator.com/item?id=43071700> / <https://lobste.rs/s/yapvon/what_do_about_sqlite_busy_errors_despite>).

Demonstration: two shells, left does `begin; select count(1) from t;`, right does `begin; insert...`. When the left
side then inserts: *"Runtime error: database is locked (5) -- immediately."* Hubert: *"No matter how high you set
your .timeout, this scenario always delivers an instant SQLITE_BUSY error."*

His prescription:
> *"don't ever upgrade transactions to read-write. If you know you are going to write in a transaction, use 'BEGIN
> IMMEDIATE', or start off with the write."*

With a counter-caution: don't reflexively `BEGIN IMMEDIATE` *everything*, since "that will easily cause timeouts,
since you can only have a single write transaction open at a time." Read-only paths should stay read-only.

He also reports a **startup-specific hazard**: opening several connections in parallel at program start can trigger
`SQLITE_BUSY_RECOVERY` because of WAL recovery after an unclean shutdown; his advice is to open connections
sequentially. For obr this maps to: if `br` ever opens more than one connection (e.g. a reader + a writer, or a
daemon), serialize the opens.

Simon Willison maintains a running tag on this topic: <https://simonwillison.net/tags/sqlite-busy/>.

### 4.5 Framework-level confirmations
- **Rails / sqlite3-ruby** defaulted transactions to IMMEDIATE for exactly this reason.
- **Datasette 1.0a14** (Aug 2024) adopted the same.
- Stephen Margheim's analysis: <https://fractaledmind.com/2024/04/15/sqlite-on-rails-the-how-and-why-of-optimal-performance/>.

### 4.6 The backoff-fairness problem (subtle, real)
SQLite's default busy callback (`sqliteDefaultBusyCallback`) uses a hard-coded delay table:

`delays[] = { 1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100 }` ms, with cumulative totals
`{ 0, 1, 3, 8, 18, 33, 53, 78, 103, 128, 178, 228 }`, then 100 ms per retry thereafter, clipped so the total does
not overshoot `busy_timeout`. Sources: <https://sqlite.org/forum/info/3fd33f0b9be72353> and
<https://www.sqlite.org/c3ref/busy_timeout.html>.

Margheim's fairness argument
(<https://fractaledmind.com/2024/04/15/sqlite-on-rails-the-how-and-why-of-optimal-performance/>): a brand-new
waiter starts at a 1 ms delay while an old waiter is stuck at 100 ms, so newcomers "cut in line". He measured that
a new query "will be allowed to retry to acquire the write lock **three times** before the original query is
allowed to retry _once_", starving long-waiting queries and inflating p99.99. His fix in sqlite3-ruby: a custom
busy_handler with a **constant 1 ms** retry interval, which flattened the tail-latency curve.

Caveat on granularity: *"The OS is entirely responsible for the actual granularity of the sleep no matter what is
requested"* — on Windows the timer tick is ~15.6 ms by default, so sub-tick delays round up
(<https://sqlite.org/forum/info/3fd33f0b9be72353>).

**obr relevance:** under heavy agent fan-out (say 8 agents each running `br` in a loop), the default backoff means
the unluckiest invocation can time out while newer ones succeed. A constant-interval busy_handler, or an
application-level retry loop around `SQLITE_BUSY` at the *command* level, addresses this. Note the trade-off: a
custom `busy_handler` in rusqlite **replaces** `busy_timeout`, so the timeout budget must be enforced inside the
callback.

### 4.7 Full taxonomy of SQLITE_BUSY causes in WAL mode
The canonical enumeration lives at Clément Joly's blog. **Note: the exact article URL I attempted
(`https://cj.rs/blog/sqlite-busy-error-despite-timeout/`) returned HTTP 404 and I could not read it directly.**
The five scenarios are reconstructable from primary sources and from search summaries of that article, and each is
individually verified below against sqlite.org:

1. **Two concurrent write transactions.** Only one writer at a time (wal.html; whentouse.html: *"SQLite supports an
   unlimited number of simultaneous readers, but it will only allow one writer at any instant in time."*).
   → Mitigation: `busy_timeout` + short transactions.
2. **DEFERRED-to-write upgrade** → immediate `SQLITE_BUSY` / `SQLITE_BUSY_SNAPSHOT`, busy handler bypassed
   (rescode.html, busy_handler.html, lang_transaction.html). → Mitigation: `BEGIN IMMEDIATE`.
3. **Exclusive locking mode held by another connection** — *"all queries return SQLITE_BUSY"* (wal.html).
   → Mitigation: don't use `locking_mode=EXCLUSIVE`.
4. **Last-connection cleanup window**: the closing connection briefly takes an exclusive lock to delete
   `-wal`/`-shm`; a concurrent open during that window sees `SQLITE_BUSY` (wal.html).
5. **WAL recovery after a crash**: the next opener holds an exclusive lock while recovering; a third connection
   gets `SQLITE_BUSY_RECOVERY` (wal.html, rescode.html).

Cross-cutting mitigations, all documented: raise `busy_timeout`; keep transactions short and non-dangling; wrap
write transactions in a **finite retry loop with backoff**; enable the SQLite error log.

---

## 5. rusqlite idioms

Sources: <https://docs.rs/rusqlite/latest/rusqlite/struct.Connection.html>,
<https://github.com/rusqlite/rusqlite>, <https://lib.rs/crates/rusqlite/features>,
<https://docs.rs/crate/rusqlite/latest>.

### 5.1 Connection defaults
`Connection::open` is equivalent to `open_with_flags` with
`SQLITE_OPEN_READ_WRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_URI | SQLITE_OPEN_NO_MUTEX`.
`NO_MUTEX` is chosen because *"Rusqlite enforces thread-safety at compile time, so additional locking is not
needed."* `Connection` is `Send` but **not** `Sync`.

Note the practical consequence of `SQLITE_OPEN_URI` being on by default: a path containing `?` is interpreted as a
URI. For a CLI that takes DB paths from config/env, this is a small sharp edge worth being aware of.

`path()` returns `Some("")` for temp/in-memory databases (not `None`).
`close()` is equivalent to `Drop` but returns `Err((Connection, Error))` so you can retry.

### 5.2 Busy handling
> *"Newly created connections currently have a default busy timeout of 5000ms, but this may be subject to change."*

and

> *"Newly created connections default to a `busy_timeout()` handler with a timeout of 5000ms."*

This is a **rusqlite** default, not a SQLite default (raw SQLite has no busy handler by default). The
"subject to change" hedge means production code should set it explicitly rather than relying on it.

`busy_handler(Option<fn(i32) -> bool>)`: the argument is the retry count; `false` → give up with `SQLITE_BUSY`;
`true` → retry. `busy_timeout(Duration::ZERO)` turns off all busy handlers.

### 5.3 Transactions
- `transaction()` → DEFERRED, takes `&mut self` (compile-time nesting prevention), rolls back on drop unless
  committed.
- `transaction_with_behavior(TransactionBehavior::Immediate)` → what obr uses.
- `set_transaction_behavior(...)` → *"Set the default transaction behavior for the connection"*, applying to
  subsequent `transaction()`/`unchecked_transaction()` calls. **This is the cleanest way to make IMMEDIATE the
  default everywhere and prevent a future contributor from accidentally introducing a DEFERRED write path.**
- `unchecked_transaction()` → `&self`, runtime nesting checks; for `Rc<Connection>` designs.
- `savepoint()` / `savepoint_with_name()` for nesting.

### 5.4 Statement caching
`prepare_cached(sql)` (requires the `cache` feature; keyed by SQL text) returns a cached prepared statement,
avoiding re-parse/re-plan. `set_prepared_statement_cache_capacity` tunes it (default is "relatively small").
`rusqlite-macros` provides `prepare_and_bind` / `prepare_cached_and_bind` for binding Rust identifiers directly.
The standard idiom: use `prepare_cached` for any statement executed more than once per connection lifetime.
For a short-lived CLI the win is confined to loops (bulk JSONL import, per-issue event writes) — but those are
precisely obr's hot paths.

### 5.5 `bundled` vs system SQLite
- Default: `libsqlite3-sys` finds a system SQLite via pkg-config / vcpkg.
- With `bundled`: *"libsqlite3-sys will use the cc crate to compile SQLite... from source and link against
  that."* The vendored source is **SQLite 3.51.1** as of rusqlite 0.38.0 / libsqlite3-sys 0.36.0.
  *"This is probably the simplest solution to any build problems."*
- Real-world reason to bundle: system SQLite is compiled with unpredictable feature flags. Concrete example from
  the ecosystem — `SQLITE_ENABLE_DBSTAT_VTAB` is on in the bundled build but absent from macOS system SQLite and
  many Linux distro builds; *"if you want a tool that Just Works on anyone's machine, you bundle it."*
- Licensing: bundled SQLite is public domain; no effect on the crate's own license.
- Since rusqlite 0.10.1, pregenerated bindings ship for several SQLite versions, so no bindgen/C toolchain is
  needed beyond `cc`.

**obr already uses `bundled`** (`Cargo.toml:22`), which is the correct choice for a distributable CLI that must
behave identically on every machine and that relies on modern features. The cost is that SQLite CVE/bugfix
tracking becomes obr's responsibility rather than the distro's — see §2.10.

obr also enables `modern_sqlite` (unlocks `db_name`, `transaction_state`, `is_interrupted`, `set_errmsg`) and
`fallible_uint`.

### 5.6 Other useful rusqlite surface
- `execute_batch(sql)` — multiple semicolon-separated parameterless statements; the right tool for DDL scripts
  (obr uses it at `src/storage/schema.rs:210`).
- `pragma_update` / `pragma_query_value` / `pragma_query` / `pragma_update_and_check`; docs recommend preferring
  SQLite's **PRAGMA table-valued functions** (SQLite ≥ 3.20, e.g. `pragma_table_info('t')`) over the convenience
  methods for read-only pragmas — obr already does this in places (`src/storage/schema.rs:244`).

---

## 6. Schema migrations

### 6.1 `rusqlite_migration` (Clément Joly)
<https://docs.rs/rusqlite_migration>, <https://github.com/cljoly/rusqlite_migration>,
<https://cj.rs/rusqlite_migration/>

Design: a `const` slice of `M::up("...")` values wrapped in `Migrations::from_slice()`, applied with
`Migrations::to_latest(&mut conn)`, documented as updating *"the database schema, atomically"*.

Why `user_version` and not a table:
> *"to keep track of the current migration state, most tools create one or more tables in the database. These
> tables require parsing by SQLite and are queried with SQL statements. This library uses the `user_version` value
> instead. It's much lighter as it is just an integer at a fixed offset in the SQLite file."*

Other properties: no macros (fast compile), no external CLI, `#![forbid(unsafe_code)]`, `validate()` for use in
tests, `Debug` impl for `insta` snapshot testing, optional `from-directory` feature to load `*.sql` files, hook
support (`MigrationHook`, `HookResult`, `HookError`), and downward-migration examples in the repo.

Documented limits:
1. *"if your program or any other library changes it, this library will behave in an unspecified way: it may
   return an error, apply the wrong set of migrations, do nothing at all."*
2. `user_version` is effectively `i32` → ~2 billion migration cap (`MIGRATIONS_MAX`); the docs note you'd need
   "10 000 new migrations, every day, for over 5 centuries" to hit it.

MSRV tracks rusqlite's.

### 6.2 `refinery`
<https://github.com/rust-db/refinery>, <https://docs.rs/refinery/>

- Multi-backend: postgres, tokio-postgres, mysql, mysql_async, rusqlite, tiberius; SQLx supported by passing a
  `Config` instead of a connection.
- Migrations as `.sql` files or Rust modules exposing `fn migration() -> String`; works with `barrel`.
- Naming: `V{n}__{name}.sql` (strictly versioned) or `U{n}__{name}.sql` (unversioned, for teams that create/deploy
  migrations out of order).
- Applied via the `embed_migrations!` macro or `refinery_cli`.
- **Flyway-derived philosophy: no down migrations.** *"To undo/rollback a migration, you have to generate a new one
  and write specifically what you want to undo."*
- Uses a tracking table (not `user_version`).
- For rusqlite in async contexts, run inside `tokio::task::spawn_blocking`.

### 6.3 Hand-rolled `user_version`
This is the approach obr uses and it is entirely respectable — `rusqlite_migration` is essentially a thin,
well-tested wrapper around it. The canonical pattern:

```rust
let v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
if v < TARGET {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // ... apply steps v+1 ..= TARGET ...
    tx.pragma_update(None, "user_version", TARGET)?;   // inside the tx
    tx.commit()?;
}
```

Two properties matter and are easy to get wrong:
- **SQLite DDL is transactional.** Wrapping the whole migration (DDL + the `user_version` bump) in one IMMEDIATE
  transaction makes the upgrade atomic: either the schema and the version both advance, or neither does. Without
  it, a crash mid-migration leaves a half-migrated schema whose version number no longer describes it.
  (`PRAGMA user_version` is a header write and participates in the transaction; `PRAGMA journal_mode` and
  `PRAGMA foreign_keys` do **not** and cannot be issued inside a transaction — pragma.html.)
- **Multi-process migration race.** N concurrent `br` processes may all observe `user_version < TARGET` and all
  try to migrate. `BEGIN IMMEDIATE` serializes them; the losers must **re-read `user_version` after acquiring the
  write lock** and no-op if another process already migrated. Idempotent `IF NOT EXISTS` DDL (which obr uses)
  makes this survivable, but the re-check is the correct pattern.
- The `user_version` namespace is shared. If obr's `.db` is ever also opened by the Go `bd` binary (conformance
  testing does exactly this: `tests/conformance.rs` compares `br` against `bd`), both tools must agree on the
  meaning of `user_version`, or one will mis-migrate the other's database. This is precisely
  `rusqlite_migration`'s documented "shared state" caveat.

### 6.4 Choosing
For a single-binary CLI with a modest, SQLite-only schema, hand-rolled `user_version` or `rusqlite_migration` are
both right; `refinery` buys multi-backend support and a CLI that obr does not need, at the cost of a tracking
table, a proc-macro, and compile time. Given obr's constraints (`#![forbid(unsafe_code)]`, clippy pedantic+nursery,
fast builds), `rusqlite_migration` is the natural upgrade path from the current hand-rolled code if the migration
list ever grows unwieldy.

---

## 7. When people choose redb / sled / native formats instead

### 7.1 redb
<https://github.com/cberner/redb>, <https://www.redb.org/>

- *"A simple, portable, high-performance, ACID, embedded key-value store"*, pure Rust, *"loosely inspired by
  lmdb"*, storage is *"a collection of copy-on-write B+trees."*
- Features: *"Fully ACID-compliant transactions"*, *"MVCC support for concurrent readers & writer, without
  blocking"*, *"Zero-copy, thread-safe, BTreeMap based"* API, savepoints and rollbacks, crash-safe by default.
- Status: *"Stable and maintained."* *"The file format is stable, and a reasonable effort will be made to provide
  an upgrade path if there are any future changes to it."* (1.0 released 2023-06;
  <https://www.redb.org/post/2023/06/16/1-0-stable-release/>)
- Performance: *"similar performance to other top embedded key-value stores such as lmdb and rocksdb"*; the
  README benchmark table compares redb / lmdb / rocksdb / fjall / sqlite.
- **Multi-process:** the upstream README does not state a policy. A redb fork's docs are explicit that the design
  is single-process: it uses `try_lock`/`try_lock_shared` to stop the *same* process opening a DB twice, "on
  platforms where file locks are unsupported... the lock is silently skipped, and multi-process access to the same
  file is not supported and will corrupt data"
  (<https://github.com/varun29ankuS/shodh-redb>). Treat this as strong indicative evidence about the family of
  designs rather than an upstream redb guarantee — **I could not find an authoritative upstream redb statement
  either way.**

### 7.2 sled
<https://github.com/spacejam/sled>, <https://docs.rs/sled>

- Self-described as *"the champagne of beta embedded databases"*; API like `BTreeMap<[u8], [u8]>`, multiple Trees,
  ACID transactions across items.
- Its own README warns it is *"quite young and should be considered unstable for the time being, since the on-disk
  format is going to change in ways that require manual migrations before the 1.0.0 release."* Still no stable
  1.0 as of this research.
- The README itself redirects users: *"if storage price/performance is your primary constraint, you should use
  RocksDB"* (sled "uses too much space sometimes"), and *"if you have a multi-process workload that rarely writes,
  use LMDB."*

### 7.3 The general multi-process picture
- **LevelDB**: *"a database may only be opened by one process at a time,"* enforced by an OS lock
  (<https://github.com/google/leveldb/issues/182>).
- **RocksDB**: has a known `fcntl` bug where opening the LOCK file and closing it releases the first instance's
  lock (<https://github.com/facebook/rocksdb/issues/1780>) — the same POSIX advisory-lock quirk SQLite documents.
- **Pebble**: `pebble.Open` takes an exclusive lock even in read-only mode; multi-process is an open feature
  request (<https://github.com/cockroachdb/pebble/issues/1583>).
- **Turso** (SQLite-compatible rewrite) has an opt-in `multiprocess_wal` feature, confirming that multi-process WAL
  is a *hard* thing that most engines skip.

**Conclusion for obr:** multi-process access is the hard requirement that eliminates every pure-Rust KV
alternative. SQLite's decades of investment in POSIX advisory locking, WAL shared memory, hot-journal recovery,
and busy handling is precisely the thing obr needs and cannot cheaply rebuild. Swapping to redb would mean either
serializing all `br` invocations behind a lock file or running a daemon.

### 7.4 When a native/custom format wins instead
From <https://www.sqlite.org/whentouse.html>, SQLite is the wrong tool when:
- *"the same database will be accessed directly (without an intervening application server) and simultaneously
  from many computers over a network"*;
- the workload is write-intensive at web scale;
- data grows into the terabyte range;
- **many concurrent writers** — *"SQLite only supports one writer at a time per database file"* (with the caveat
  that it *"will handle more write concurrency than many people suspect"*, since *"Writers queue up. Each
  application does its database work quickly and moves on, and no lock lasts for more than a few dozen
  milliseconds."*).

And on network filesystems specifically:
> *"SQLite will work over a network filesystem, but because of the latency associated with most network
> filesystems, performance will not be great. Also, file locking logic is buggy in many network filesystem
> implementations (on both Unix and Windows). If file locking does not work correctly, two or more clients might
> try to modify the same part of the same database at the same time, resulting in corruption."*

Final rule of thumb from that page: *"For device-local storage with low writer concurrency and less than a
terabyte of content, SQLite is almost always a better solution... It keeps things simple. SQLite 'just works.'"*

**obr's hybrid is exactly the recommended pattern**: SQLite for device-local speed, a text format (JSONL) for the
network/sharing layer. It deliberately avoids the "SQLite over a network filesystem" antipattern by shipping the
*text* through git rather than the database.

---

## 8. Known pitfalls for concurrent multi-process writers

Primary source: <https://www.sqlite.org/howtocorrupt.html>. This page is the single most important read for a
multi-process CLI.

### 8.1 Locking hazards

**Network filesystems** (§2.1):
> *"SQLite depends on the underlying filesystem to do locking as the documentation says it will. But some
> filesystems contain bugs in their locking logic such that the locks do not always behave as advertised. This is
> especially true of network filesystems and NFS in particular. If SQLite is used on a filesystem where the
> locking primitives contain bugs, and if two or more threads or processes try to access the same database at the
> same time, then database corruption might result."*

**POSIX advisory locks cancelled by `close()`** (§2.2) — the nastiest one:
> *"The default locking mechanism used by SQLite on unix platforms is POSIX advisory locking. Unfortunately, POSIX
> advisory locking has design quirks that make it prone to misuse and failure... One particularly pernicious
> problem is that the close() system call will cancel all POSIX advisory locks on the same file for all threads and
> all file descriptors in the process."*

Realistic trigger: some *other* part of the program opens the `.db` file directly (to stat it, sniff its type,
copy it, hash it) and closes it — silently dropping every lock SQLite thought it held.
> *"This problem only arises when a thread tries to bypass the SQLite library and read the database file directly."*

Mitigation added in **3.51.0 (2025-11-04)**: *"SQLite implements additional defenses to try to avoid problems
caused by locks that are broken by close(). These new defenses help when the database is in WAL mode and is being
accessed from multiple processes. But they are not a cure-all. To avoid corruptions, developers should be careful
to never use close() on an SQLite database file while one or more database connections are open, even in other
threads."*

**Multiple copies of SQLite in one process** (§2.3): two statically linked copies keep two separate global lock
lists and cannot see each other's connections → *"A close() operation on one connection might unknowingly clear
the locks on a different database connection, leading to database corruption."* Relevant if a Rust binary ever
ends up with both `libsqlite3-sys` bundled and a dynamically linked system SQLite via another dependency (e.g. a
transitive dep pulling in a different sqlite crate). Worth a `cargo tree -i libsqlite3-sys` check in CI.

**Mixed locking protocols** (§2.4): all connections must use the same protocol (POSIX advisory vs dot-file), or
they will not see each other's locks.

**Unlink / rename while open** (§2.5): two processes end up on different inodes sharing one WAL name.
> *"In other words, unlinking or renaming an open database file results in behavior that is undefined and probably
> undesirable."*
SQLite ≥ 3.7.17 logs `SQLITE_WARNING` in this case.

**Multiple hard/soft links to the same file** (§2.6): different names → different journal/WAL names → recovery
looks in the wrong place. SQLite ≥ 3.10.0 canonicalizes symlinks. **Directly relevant to obr's git-worktree
support** — `.beads/redirect` (a relative path to the main repo's `.beads/`) exists precisely to avoid duplicating
the database. Whatever path resolution obr does, all processes must arrive at the *same canonical path string*, or
they will use different `-wal` files on the same inode.

**`fork()`** (§2.7):
> *"Do not open an SQLite database connection, then fork(), then try to use that database connection in the child
> process. All kinds of locking problems will result and you can easily end up with a corrupt database... Any
> database connection that is used in a child process must be opened in the child process, not inherited from the
> parent."*
Relevant to any future daemon mode, and to `std::process::Command` usage while a connection is open (fork+exec is
fine as long as the child never touches the connection; but `Command` with pre-exec hooks or a forking helper is
not).

### 8.2 Copy / backup hazards (relevant to git operations on `.beads/`)
- *"Systems that run automatic backups in the background might try to make a backup copy of an SQLite database file
  while it is in the middle of a transaction. The backup copy then might contain some old and some new content, and
  thus be corrupt."*
- *"It is also safe to make a copy of an SQLite database file as long as there are no transactions in progress
  while the copy is taking place. If the previous write transaction failed, then it is important that any rollback
  journal (the *-journal file) or write-ahead log (the *-wal file) be copied together with the database file
  itself."*
- Safe live-copy methods: **`sqlite3_rsync`** (SQLite ≥ 3.47.0, 2024-10-21), **`VACUUM INTO 'file'`**
  (SQLite ≥ 3.27.0), and the **online backup API** (<https://sqlite.org/backup.html>).
- Deleting a hot journal: *"If the hot journal files are moved, deleted, or renamed after a crash or power failure,
  then automatic recovery will not work and the database may go corrupt."*
- Mispairing DB and journal: copying a database file without its journal/WAL, or overwriting a DB while a hot
  journal for the old one exists, are both listed as corruption causes.

**obr's `.beads/.gitignore` already excludes `*.db`, `*.db?*`, `*.db-journal`, `*.db-wal`, `*.db-shm`.** That is
exactly right: git checkouts never overwrite a live database, and a database is never separated from its WAL by a
branch switch. This should be treated as a load-bearing invariant, not incidental hygiene — worth a comment in
that file and a `br doctor` check.

### 8.3 Configuration self-inflicted corruption (§7 of howtocorrupt.html)
- `PRAGMA synchronous=OFF` → vulnerable to power-failure corruption.
- Changing `PRAGMA schema_version` while other connections are open.
- `journal_mode=OFF` or `journal_mode=MEMORY` + an application crash mid-transaction.
- `PRAGMA writable_schema=ON` with careless DML on `sqlite_schema`.

obr does none of these (`synchronous=NORMAL`, `journal_mode=WAL`).

### 8.4 Other
- Disks/USB sticks that lie about `fsync`; fake-capacity flash media.
- Memory corruption in-process (stray pointers, mmap widens the blast radius) — largely mitigated by
  `#![forbid(unsafe_code)]`, but the bundled C library is still in the address space.
- QNX `mmap()` bug; old LinuxThreads (pre-NPTL) lock semantics.

---

## 9. Cross-check against obr's current implementation

Read-only inspection of the repository. Not a review — just the places where the research above lands.

### 9.1 CONFIRMED GAP: per-connection PRAGMAs are skipped on the common path

`src/storage/sqlite.rs:100-111`:
```rust
pub fn open_with_timeout(path: &Path, lock_timeout_ms: Option<u64>) -> Result<Self> {
    let conn = Connection::open(path)?;
    if let Some(timeout) = lock_timeout_ms {
        conn.busy_timeout(Duration::from_millis(timeout))?;
    }
    let user_version: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if user_version < CURRENT_SCHEMA_VERSION {
        apply_schema(&conn)?;
    }
    Ok(Self { conn })
}
```

All the PRAGMAs live inside `apply_schema()` (`src/storage/schema.rs:216-229`):
`journal_mode=WAL`, `foreign_keys=ON`, `synchronous=NORMAL`, `temp_store=MEMORY`, `cache_size=-8000`,
`user_version=CURRENT_SCHEMA_VERSION`.

`CURRENT_SCHEMA_VERSION` is `1` (`src/storage/schema.rs:5`). So on **every open of an already-initialized
database**, `user_version == 1`, `apply_schema` is skipped, and the connection ends up with:

| PRAGMA | Intended | Actual on an existing DB | Why |
|---|---|---|---|
| `journal_mode` | WAL | **WAL** ✅ | persistent in the file header |
| `foreign_keys` | ON | **OFF** ❌ | per-connection, default OFF |
| `synchronous` | NORMAL | **FULL** ❌ | per-connection, default FULL |
| `temp_store` | MEMORY | **DEFAULT** ❌ | per-connection |
| `cache_size` | -8000 | **-2000** ❌ | per-connection, explicitly session-only |

The `foreign_keys` row is the consequential one: `src/storage/schema.rs` contains **9** `ON DELETE CASCADE`
clauses (verified by grep). With FK enforcement off, none of them fire and no FK constraint is checked — so
deleting an issue leaves orphaned dependencies, comments, events, and cache rows on every real invocation, while
every test passes.

The existing tests do not catch this because both of them run against a connection where `apply_schema` just ran:
- `src/storage/sqlite.rs:4638` `test_pragmas_are_set_correctly` uses `open_memory()`
  (`src/storage/sqlite.rs:118-122`), which always calls `apply_schema`.
- `src/storage/schema.rs:544-554` asserts pragmas right after `apply_schema`.

The missing test is: create a DB on disk, drop it, **reopen it**, then assert `PRAGMA foreign_keys == 1`.

Authorities: <https://www.sqlite.org/pragma.html> (foreign_keys default OFF and *"applications should set the
foreign key enforcement flag as required by the application and not depend on the default setting"*; cache_size
*"only endures for the current session"*), plus the persistent/session split in §3.1.

### 9.2 `busy_timeout` is left to a rusqlite default that is documented as unstable
`src/storage/sqlite.rs:102-104` only sets `busy_timeout` when `lock_timeout_ms.is_some()`. Otherwise the
connection relies on rusqlite's *"default busy timeout of 5000ms, **but this may be subject to change**"*
(<https://docs.rs/rusqlite/latest/rusqlite/struct.Connection.html>). A crate upgrade could silently drop the
timeout to zero, turning every contended `br` invocation into an instant "database is locked" failure. Setting it
explicitly costs one line.

5000 ms is also on the low side for the agent-swarm workload obr targets; ecosystem pools commonly use 30 s.

### 9.3 Things obr already gets right
- `BEGIN IMMEDIATE` for all mutations: `src/storage/sqlite.rs:154`
  (`transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)`). This is the single most important
  correctness decision for a multi-process SQLite writer (§4).
- `bundled` rusqlite (`Cargo.toml:22`) — reproducible SQLite version across platforms (§5.5).
- WAL mode, `synchronous=NORMAL` — right defaults *when they are actually applied* (§3.3).
- `.beads/.gitignore` excludes `*.db`, `*.db-wal`, `*.db-shm`, `*.db-journal` — avoids the entire class of
  git-checkout-vs-hot-journal corruption in §8.2.
- The "never runs git itself" rule keeps obr out of the unlink/rename-while-open hazard (§8.1 / howtocorrupt §2.5).
- `PRAGMA integrity_check` in `br doctor` (`src/cli/commands/doctor.rs:284`).

### 9.4 Things absent that the research suggests adding
- `PRAGMA optimize` before close — sqlite.org's explicit recommendation for short-lived connections
  (<https://www.sqlite.org/pragma.html>, §3.8).
- `PRAGMA application_id` — lets `file(1)` and `br doctor` identify a beads database
  (<https://www.sqlite.org/appfileformat.html>, §3.7).
- An application-level `SQLITE_BUSY` retry around whole commands, or a constant-interval `busy_handler`, to fix
  the backoff-fairness starvation described in §4.6.
- Migrations wrapped in a single IMMEDIATE transaction with a post-lock re-read of `user_version` (§6.3).
- A `journal_size_limit` or periodic `wal_checkpoint(TRUNCATE)` if the WAL is observed growing under sustained
  agent load (§2.5).
- Bumping the pinned rusqlite/libsqlite3-sys once a release vendors SQLite ≥ 3.51.3, closing the WAL-reset bug
  window (§2.10).

---

## 10. Sources

### Authoritative (sqlite.org)
- <https://www.sqlite.org/wal.html> — WAL design, disadvantages, checkpointing, SQLITE_BUSY cases, WAL-reset bug
- <https://www.sqlite.org/appfileformat.html> — SQLite as an application file format
- <https://www.sqlite.org/whentouse.html> — appropriate uses; concurrency and network-filesystem guidance
- <https://www.sqlite.org/pragma.html> — full PRAGMA reference (foreign_keys, synchronous, journal_mode,
  busy_timeout, wal_autocheckpoint, user_version, application_id, cache_size, temp_store, mmap_size, optimize,
  analysis_limit, locking_mode, secure_delete)
- <https://www.sqlite.org/howtocorrupt.html> — every documented corruption cause; the multi-process bible
- <https://www.sqlite.org/lang_transaction.html> — DEFERRED / IMMEDIATE / EXCLUSIVE semantics
- <https://www.sqlite.org/rescode.html> — SQLITE_BUSY, BUSY_SNAPSHOT, BUSY_RECOVERY, BUSY_TIMEOUT, LOCKED
- <https://www.sqlite.org/c3ref/busy_handler.html> — the deadlock-avoidance / handler-not-invoked paragraph
- <https://www.sqlite.org/c3ref/busy_timeout.html> — busy timeout API semantics
- <https://sqlite.org/backup.html> — online backup API
- <https://sqlite.org/forum/info/3fd33f0b9be72353> — the `sqliteDefaultBusyCallback` delay table, OS sleep
  granularity
- <https://sqlite.org/forum/info/2cc132d53cbc3b1b89d5a9d6e62edbc5971dd140b568638b32f3bc2c2f6468ba> —
  SQLITE_BUSY_SNAPSHOT on transaction upgrade
- <https://sqlite.org/forum/forumpost/d2d81b46bc> — maintainer response on busy-handler-not-called

### Crate documentation
- <https://docs.rs/rusqlite/latest/rusqlite/struct.Connection.html> — Connection API, defaults, busy handling
- <https://github.com/rusqlite/rusqlite> — features, bundled SQLite version
- <https://docs.rs/crate/rusqlite/latest> — rusqlite 0.38 / libsqlite3-sys 0.36 → SQLite 3.51.1
- <https://lib.rs/crates/rusqlite/features> — feature matrix
- <https://docs.rs/rusqlite_migration> / <https://github.com/cljoly/rusqlite_migration> — user_version migrations
- <https://github.com/rust-db/refinery> / <https://docs.rs/refinery/> — multi-backend migrations
- <https://docs.rs/sqlx/latest/sqlx/sqlite/struct.SqliteConnectOptions.html> — why sqlx refuses to auto-set
  journal_mode
- <https://github.com/cberner/redb> / <https://www.redb.org/> — redb design, MVCC, file-format stability
- <https://github.com/spacejam/sled> / <https://docs.rs/sled> — sled's own maturity warnings

### Engineering posts
- <https://berthub.eu/articles/posts/a-brief-post-on-sqlite3-database-locked-despite-timeout/> — Bert Hubert on the
  DEFERRED-upgrade footgun
- <https://simonwillison.net/2025/Feb/17/sqlite-busy/> and <https://simonwillison.net/tags/sqlite-busy/>
- <https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/> — Clément Joly's PRAGMA cheatsheet
- <https://fractaledmind.com/2024/04/15/sqlite-on-rails-the-how-and-why-of-optimal-performance/> — IMMEDIATE
  transactions, busy-handler fairness, constant-interval retries
- <https://fractaledmind.com/2023/09/07/enhancing-rails-sqlite-fine-tuning/> — PRAGMA tuning
- <https://fractaledmind.com/2023/09/21/enhancing-rails-sqlite-performance-metrics/> — the 11.8× / 1.5× / 5×
  research numbers
- <https://fly.io/blog/sqlite-internals-wal/> — how WAL scales read concurrency
- <https://blog.sqlite.ai/journal-modes-in-sqlite> — journal-mode comparison incl. the "single-writer batch job"
  case
- <https://github.com/google/leveldb/issues/182>, <https://github.com/facebook/rocksdb/issues/1780>,
  <https://github.com/cockroachdb/pebble/issues/1583> — single-process locking in other embedded engines
- <https://github.com/varun29ankuS/shodh-redb> — explicit "multi-process access... will corrupt data" statement
  for a redb-derived store (indicative, not upstream)
- <https://sqlite-users.sqlite.narkive.com/e3iOB8Z3/sqlite-which-pragmas-are-persistent> — persistent-vs-session
  PRAGMA enumeration (mailing list; corroborates pragma.html)

### Unreachable
- `https://cj.rs/blog/sqlite-busy-error-despite-timeout/` — **HTTP 404**. The five-scenario SQLITE_BUSY taxonomy
  attributed to it in §4.7 was reconstructed and independently verified against sqlite.org primary sources; the
  attribution should be treated as approximate.
