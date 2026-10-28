//! Synchronous facade over the async FrankenSQLite 0.3 engine API.
//!
//! fsqlite 0.2 made every engine entry point `async` with `!Send` futures
//! (the engine is `Rc<RefCell<..>>` internally; it was already `!Send` at
//! 0.1.x — only the call shape changed), and fsqlite 0.3 moved the runtime
//! family to asupersync 0.4.4. br's storage layer is fully synchronous, so
//! this module preserves the pre-0.2 blocking call shape by driving each
//! engine future to completion on the calling thread with a private
//! current-thread `asupersync` runtime (the proven sqlmodel/cass
//! `block_on` bridge pattern; see coding_agent_session_search
//! `src/franken_sync.rs`).
//!
//! Every future is created, polled, and dropped entirely within one bridge
//! call, so the engine's `Rc<RefCell<..>>` state never crosses a thread
//! boundary between poll steps. `Runtime::block_on` has no `Send` bound and
//! saves/restores the ambient runtime handle, so nesting inside a consumer's
//! own `block_on` is safe.
//!
//! The runtime lives in a thread-local slot and is *taken out* while a
//! future is being driven: a reentrant bridge call (e.g. SQL issued from
//! inside a row-mapping closure) finds the slot empty and builds a fresh
//! runtime instead of re-entering `block_on` on the same runtime instance.
//!
//! Everything outside this module refers to the engine through
//! `crate::franken_sync::` (or `beads_rust::franken_sync::` from integration
//! tests); only this module names the `fsqlite` dependency directly for
//! connection/statement driving.

use std::cell::RefCell;
use std::future::Future;

use asupersync::runtime::{Runtime, RuntimeBuilder};

pub use fsqlite::{FrankenError, Row, SqliteValue};

// ---------------------------------------------------------------------------
// Bridge driver
// ---------------------------------------------------------------------------

thread_local! {
    static DRIVER: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

/// Drive a `!Send` fsqlite future to completion on the calling thread.
fn drive<T>(future: impl Future<Output = T>) -> T {
    let runtime = DRIVER
        .with(|slot| slot.borrow_mut().take())
        .unwrap_or_else(|| {
            RuntimeBuilder::current_thread()
                .build()
                .expect("failed to build FrankenSQLite sync-bridge runtime")
        });
    let output = runtime.block_on(future);
    DRIVER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(runtime);
        }
    });
    output
}

/// True when `err` can mean the connection's schema image predates another
/// connection's DDL commit.
///
/// fsqlite 0.2.1+ behavior (verified by standalone probe, absent at 0.1.x):
/// a connection opened before another connection CREATEs a table may not see
/// that table through the plain `query`/`execute` paths — but `prepare()`
/// refreshes the shared schema publication before resolving, after which the
/// same SQL succeeds. The facade therefore treats these errors as
/// possibly-stale-schema, drives a `prepare()` of the same SQL to force the
/// refresh, and retries once. Plan-time resolution failures have no side
/// effects, so the retry is safe.
fn schema_stale(err: &FrankenError) -> bool {
    // `SchemaChanged` is the engine's explicit stale-schema-cookie signal
    // (a plan compiled against a schema image another connection has since
    // replaced); the upstream error-taxonomy recipe for it is exactly the
    // re-prepare + single retry this facade already performs for the
    // name-resolution staleness shapes below.
    matches!(
        err,
        FrankenError::SchemaChanged
            | FrankenError::NoSuchTable { .. }
            | FrankenError::NoSuchColumn { .. }
            | FrankenError::NoSuchIndex { .. }
    )
}

/// Bounded retry for `FrankenError::BusyRecovery`.
///
/// fsqlite 0.2+ ns-lifecycle opens can put a database into a short
/// "recovery in progress" window; statements admitted during that window
/// fail with `BusyRecovery` immediately instead of waiting out the
/// connection's busy timeout. C SQLite's busy handler covers
/// `SQLITE_BUSY_RECOVERY`, and the 0.1.x line had no recovery windows at
/// all, so a bounded caller-side retry restores the pre-0.2 observable
/// behavior. Plain `Busy` is deliberately NOT retried here: br classifies
/// ordinary lock contention itself and the engine owns that timeout.
fn retry_busy_recovery<T>(
    mut attempt: impl FnMut() -> Result<T, FrankenError>,
) -> Result<T, FrankenError> {
    const RETRY_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);
    const BACKOFF_CAP: std::time::Duration = std::time::Duration::from_millis(250);
    // ubs:ignore — this monotonic clock bounds retries; it generates no token or randomness.
    let start = std::time::Instant::now();
    let mut backoff = std::time::Duration::from_millis(5);
    loop {
        match attempt() {
            Err(FrankenError::BusyRecovery) if start.elapsed() < RETRY_BUDGET => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(BACKOFF_CAP);
            }
            other => return other,
        }
    }
}

/// Bounded retry for the engine's transient errors on one statement.
///
/// `BusyRecovery` is always retried (see [`retry_busy_recovery`]).
/// `BusySnapshot` is first-committer-wins loss at commit; the engine
/// contract says "retry the whole transaction". When the connection was in
/// autocommit before the statement ran, the statement IS the whole
/// transaction, so retrying it here is exactly that contract. Inside an
/// explicit transaction the error is surfaced instead: only the caller can
/// re-run its transaction body.
fn retry_transient<T>(
    conn: &fsqlite::Connection,
    mut attempt: impl FnMut() -> Result<T, FrankenError>,
) -> Result<T, FrankenError> {
    const RETRY_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);
    const BACKOFF_CAP: std::time::Duration = std::time::Duration::from_millis(250);
    let was_autocommit = !conn.in_transaction();
    let start = std::time::Instant::now();
    let mut backoff = std::time::Duration::from_millis(5);
    loop {
        match attempt() {
            Err(error) => {
                let retryable = matches!(error, FrankenError::BusyRecovery)
                    || (matches!(error, FrankenError::BusySnapshot { .. })
                        && was_autocommit
                        && !conn.in_transaction());
                if !retryable || start.elapsed() >= RETRY_BUDGET {
                    return Err(error);
                }
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(BACKOFF_CAP);
            }
            ok => return ok,
        }
    }
}

macro_rules! with_engine_retries {
    ($conn:expr, $sql:expr, $attempt:expr) => {{
        let first = retry_transient(&$conn, || $attempt);
        match first {
            Err(ref err) if schema_stale(err) => {
                // `prepare` refreshes the schema image from the shared
                // publication plane even when it ultimately fails to resolve.
                let _ = drive($conn.prepare($sql));
                retry_transient(&$conn, || $attempt)
            }
            other => other,
        }
    }};
}

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

/// Synchronous wrapper over [`fsqlite::Connection`] with the pre-0.2
/// blocking method signatures.
pub struct Connection {
    inner: fsqlite::Connection,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("path", &self.inner.path())
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Open (or create) a database at `path`.
    pub fn open(path: impl Into<String>) -> Result<Self, FrankenError> {
        let inner = drive(fsqlite::Connection::open(path))?;
        Self::from_inner(inner, true)
    }

    #[cfg(unix)]
    pub(crate) fn open_observer_existing(
        path: &str,
        identity: fsqlite_vfs::FileIdentity,
    ) -> Result<Self, FrankenError> {
        let inner = drive(
            fsqlite::Connection::open_schema_only_with_expected_identity(path.to_owned(), identity),
        )?;
        Self::from_inner(inner, false)
    }

    fn from_inner(inner: fsqlite::Connection, serialized: bool) -> Result<Self, FrankenError> {
        let connection = Self { inner };
        if !serialized {
            return Ok(connection);
        }
        // br already serializes mutations through its workspace write lock
        // and owns whole-transaction retries. Keep the engine on SQLite's
        // single-writer semantics so a schema rebuild cannot be rejected by
        // MVCC validation against its own INSERT..SELECT + DROP write set.
        connection.execute("PRAGMA fsqlite.concurrent_mode = OFF")?;
        Ok(connection)
    }

    /// Access the wrapped async connection (escape hatch for callers that
    /// drive engine APIs this facade does not wrap).
    #[must_use]
    pub const fn as_async(&self) -> &fsqlite::Connection {
        &self.inner
    }

    /// Execute a single SQL statement, returning the affected row count.
    pub fn execute(&self, sql: &str) -> Result<usize, FrankenError> {
        with_engine_retries!(self.inner, sql, drive(self.inner.execute(sql)))
    }

    /// Execute a single SQL statement with positional parameters.
    pub fn execute_with_params(
        &self,
        sql: &str,
        params: &[SqliteValue],
    ) -> Result<usize, FrankenError> {
        with_engine_retries!(
            self.inner,
            sql,
            drive(self.inner.execute_with_params(sql, params))
        )
    }

    /// Query, returning all rows.
    pub fn query(&self, sql: &str) -> Result<Vec<Row>, FrankenError> {
        with_engine_retries!(self.inner, sql, drive(self.inner.query(sql)))
    }

    /// Query with positional parameters, returning all rows.
    pub fn query_with_params(
        &self,
        sql: &str,
        params: &[SqliteValue],
    ) -> Result<Vec<Row>, FrankenError> {
        with_engine_retries!(
            self.inner,
            sql,
            drive(self.inner.query_with_params(sql, params))
        )
    }

    /// Query, returning exactly one row.
    pub fn query_row(&self, sql: &str) -> Result<Row, FrankenError> {
        with_engine_retries!(self.inner, sql, drive(self.inner.query_row(sql)))
    }

    /// Query with positional parameters, returning exactly one row.
    pub fn query_row_with_params(
        &self,
        sql: &str,
        params: &[SqliteValue],
    ) -> Result<Row, FrankenError> {
        with_engine_retries!(
            self.inner,
            sql,
            drive(self.inner.query_row_with_params(sql, params))
        )
    }

    /// Prepare a statement for repeated execution.
    pub fn prepare(&self, sql: &str) -> Result<PreparedStatement<'_>, FrankenError> {
        Ok(PreparedStatement {
            inner: retry_busy_recovery(|| drive(self.inner.prepare(sql)))?,
        })
    }

    /// Last-inserted rowid on this connection.
    #[must_use]
    pub fn last_insert_rowid(&self) -> i64 {
        self.inner.last_insert_rowid()
    }

    /// Close the connection (rolls back any active transaction, then runs the
    /// final passive WAL checkpoint).
    pub fn close(mut self) -> Result<(), FrankenError> {
        drive(self.inner.close_in_place())
    }

    /// Close in place, retaining the handle on error so callers can retry.
    pub fn close_in_place(&mut self) -> Result<(), FrankenError> {
        drive(self.inner.close_in_place())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // fsqlite 0.1.x closed on drop (best-effort, no checkpoint); 0.2+'s
        // `Drop` cannot await and so skips that teardown. Driving the same
        // best-effort close here restores the 0.1.x observable contract that
        // writes made through a dropped connection are visible to any later
        // open (br #270 relies on Drop flushing the WAL). This is a no-op if
        // the connection was already explicitly closed.
        drive(self.inner.close_best_effort_in_place());
    }
}

// ---------------------------------------------------------------------------
// Prepared statements
// ---------------------------------------------------------------------------

/// Synchronous wrapper over [`fsqlite::PreparedStatement`].
pub struct PreparedStatement<'conn> {
    inner: fsqlite::PreparedStatement<'conn>,
}

impl PreparedStatement<'_> {
    /// Render the compiled program for diagnostics (sync in fsqlite).
    #[must_use]
    pub fn explain(&self) -> String {
        self.inner.explain()
    }

    /// Query, returning all rows.
    pub fn query(&self) -> Result<Vec<Row>, FrankenError> {
        drive(self.inner.query())
    }

    /// Query with positional parameters, returning all rows.
    pub fn query_with_params(&self, params: &[SqliteValue]) -> Result<Vec<Row>, FrankenError> {
        drive(self.inner.query_with_params(params))
    }

    /// Query, returning exactly one row.
    pub fn query_row(&self) -> Result<Row, FrankenError> {
        drive(self.inner.query_row())
    }

    /// Query with positional parameters, returning exactly one row.
    pub fn query_row_with_params(&self, params: &[SqliteValue]) -> Result<Row, FrankenError> {
        drive(self.inner.query_row_with_params(params))
    }

    /// Execute, returning the affected row count.
    pub fn execute(&self) -> Result<usize, FrankenError> {
        drive(self.inner.execute())
    }

    /// Execute with positional parameters, returning the affected row count.
    pub fn execute_with_params(&self, params: &[SqliteValue]) -> Result<usize, FrankenError> {
        drive(self.inner.execute_with_params(params))
    }
}

// ---------------------------------------------------------------------------
// compat: rusqlite-style open flags, synchronous form
// ---------------------------------------------------------------------------

pub mod compat {
    use super::{Connection, FrankenError, drive};

    pub use fsqlite::compat::OpenFlags;

    /// Open a database with rusqlite-style open flags (synchronous form of
    /// [`fsqlite::compat::open_with_flags`]).
    pub fn open_with_flags(path: &str, flags: OpenFlags) -> Result<Connection, FrankenError> {
        let serialized = flags.contains(OpenFlags::SQLITE_OPEN_READ_WRITE);
        let inner = drive(fsqlite::compat::open_with_flags(path, flags))?;
        Connection::from_inner(inner, serialized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_execute_query_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("bridge.db");
        let conn =
            Connection::open(db.to_string_lossy().into_owned()).expect("open bridge database");
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
            .expect("create table");
        let inserted = conn
            .execute_with_params(
                "INSERT INTO t (v) VALUES (?1)",
                &[SqliteValue::from("hello")],
            )
            .expect("insert row");
        assert_eq!(inserted, 1);
        let rows = conn.query("SELECT v FROM t").expect("query rows");
        assert_eq!(rows.len(), 1);
        let row = conn
            .query_row_with_params("SELECT v FROM t WHERE id = ?1", &[SqliteValue::from(1i64)])
            .expect("query row");
        assert_eq!(row.get(0).and_then(SqliteValue::as_text), Some("hello"));
        conn.close().expect("close");
    }

    #[test]
    fn prepared_statement_roundtrip() {
        let conn = Connection::open(":memory:").expect("open in-memory database");
        conn.execute("CREATE TABLE t (k TEXT)").expect("create");
        conn.execute_with_params("INSERT INTO t (k) VALUES (?1)", &[SqliteValue::from("a")])
            .expect("insert");
        let stmt = conn
            .prepare("SELECT count(*) FROM t WHERE k = ?1")
            .expect("prepare");
        let row = stmt
            .query_row_with_params(&[SqliteValue::from("a")])
            .expect("query");
        assert_eq!(row.get(0).and_then(SqliteValue::as_integer), Some(1));
    }

    #[test]
    fn string_in_list_predicates_match_equality_forms() {
        let conn = Connection::open(":memory:").expect("open in-memory database");
        conn.execute("CREATE TABLE dependencies (issue_id TEXT, depends_on_id TEXT, type TEXT)")
            .expect("create");
        for (a, b, t) in [
            ("i1", "i2", "blocks"),
            ("i2", "i1", "blocks"),
            ("i3", "i1", "related"),
            ("i4", "i1", "waits-for"),
        ] {
            conn.execute_with_params(
                "INSERT INTO dependencies (issue_id, depends_on_id, type) VALUES (?1, ?2, ?3)",
                &[
                    SqliteValue::from(a),
                    SqliteValue::from(b),
                    SqliteValue::from(t),
                ],
            )
            .expect("insert");
        }
        // The dependency-cycle graph loader depends on bare full-scan
        // string IN-list predicates returning exactly the equality union.
        let in_list = conn
            .query(
                "SELECT issue_id, depends_on_id FROM dependencies \
                 WHERE type IN ('blocks', 'conditional-blocks', 'waits-for')",
            )
            .expect("in-list query");
        assert_eq!(in_list.len(), 3, "IN-list must match blocks + waits-for");
        let eq = conn
            .query("SELECT issue_id FROM dependencies WHERE type = 'blocks'")
            .expect("equality query");
        assert_eq!(eq.len(), 2, "equality predicate must see both blocks rows");
        let or_form = conn
            .query(
                "SELECT issue_id FROM dependencies \
                 WHERE type = 'blocks' OR type = 'conditional-blocks' OR type = 'waits-for'",
            )
            .expect("or query");
        assert_eq!(or_form.len(), 3, "OR form must agree with the IN form");
    }

    #[test]
    fn connections_default_to_serialized_engine_mode() {
        let conn = Connection::open(":memory:").expect("open in-memory database");
        let row = conn
            .query_row("PRAGMA fsqlite.concurrent_mode")
            .expect("query engine mode");
        assert_eq!(row.get(0).and_then(SqliteValue::as_integer), Some(0));
    }

    #[test]
    fn writable_compat_connections_use_serialized_engine_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("compat.db");
        let path = db.to_string_lossy().into_owned();
        let initial = Connection::open(path.clone()).expect("create compat database");
        initial.close().expect("close initial connection");

        let conn = compat::open_with_flags(&path, compat::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .expect("open writable compat connection");
        let row = conn
            .query_row("PRAGMA fsqlite.concurrent_mode")
            .expect("query compat engine mode");
        assert_eq!(row.get(0).and_then(SqliteValue::as_integer), Some(0));
    }

    #[test]
    fn schema_changed_enters_the_stale_schema_retry_path() {
        assert!(schema_stale(&FrankenError::SchemaChanged));
    }

    #[test]
    fn reentrant_bridge_calls_build_fresh_runtime() {
        // A bridge call issued while another bridge call's runtime is checked
        // out must not panic or deadlock (the thread-local slot is empty, so
        // a fresh runtime is built).
        let row_count = drive(async {
            let conn = Connection::open(":memory:").expect("nested open");
            conn.execute("CREATE TABLE t (k INTEGER)")
                .expect("nested create");
            conn.execute("INSERT INTO t (k) VALUES (1)")
                .expect("nested insert");
            conn.query("SELECT k FROM t").expect("nested query").len()
        });

        assert_eq!(row_count, 1);
    }

    #[cfg(unix)]
    mod observer_feasibility {
        use std::collections::BTreeMap;
        use std::fs::{self, File, OpenOptions};
        use std::io::{BufRead as _, BufReader, Read as _, Write as _};
        use std::os::fd::OwnedFd;
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
        use std::os::unix::net::UnixStream;
        use std::path::{Path, PathBuf};
        use std::process::{Child, Command, ExitStatus, Stdio};
        use std::time::{Duration, Instant};

        use fsqlite_vfs::{
            DatabaseNamespaceBinding, FileIdentity, NamespaceOpenIntent, PendingNamespaceOpen,
        };
        use sha2::{Digest as _, Sha256};

        use super::*;

        const WORKER_STACK_BYTES: usize = 16 * 1024 * 1024;
        const CHILD_MODE: &str = "OBR_OBSERVER_FEASIBILITY_CHILD";
        const CHILD_DATABASE: &str = "OBR_OBSERVER_FEASIBILITY_DATABASE";
        const CHILD_LEASE: &str = "OBR_OBSERVER_FEASIBILITY_LEASE";
        const CHILD_TEST: &str = concat!(
            "franken_sync::tests::observer_feasibility::",
            "observer_feasibility_subprocess_worker"
        );

        #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
        struct FileEvidence {
            sha256: Option<String>,
            device: u64,
            inode: u64,
            mode: u32,
            links: u64,
            uid: u32,
            gid: u32,
            bytes: u64,
            modified: (i64, i64),
            changed: (i64, i64),
        }

        #[derive(Debug)]
        struct ProbeLease(File);

        impl Drop for ProbeLease {
            fn drop(&mut self) {
                self.0.unlock().expect("release native probe opener lease");
            }
        }

        #[derive(Debug)]
        struct Probe {
            // Field order keeps engine teardown ahead of every retained guard during unwinding.
            connection: Connection,
            namespace: std::sync::Arc<DatabaseNamespaceBinding>,
            identity_file: File,
            _lease: ProbeLease,
        }

        struct ProbeChild {
            process: Child,
            control: BufReader<UnixStream>,
        }

        impl ProbeChild {
            fn start(mode: &str, database: &Path, lease: &Path) -> Self {
                let (parent, peer) = UnixStream::pair().expect("owned child control socket");
                parent
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .expect("bounded child control reads");
                parent
                    .set_write_timeout(Some(Duration::from_secs(10)))
                    .expect("bounded child control writes");
                let input =
                    Stdio::from(OwnedFd::from(peer.try_clone().expect("child input socket")));
                let output = Stdio::from(OwnedFd::from(peer));
                let process =
                    Command::new(std::env::current_exe().expect("native test executable"))
                        .args([
                            "--exact",
                            CHILD_TEST,
                            "--ignored",
                            "--nocapture",
                            "--test-threads=1",
                        ])
                        .env(CHILD_MODE, mode)
                        .env(CHILD_DATABASE, database)
                        .env(CHILD_LEASE, lease)
                        .env("RUST_BACKTRACE", "0")
                        .stdin(input)
                        .stdout(output)
                        .stderr(Stdio::piped())
                        .spawn()
                        .expect("owned native fixture subprocess");
                Self {
                    process,
                    control: BufReader::new(parent),
                }
            }

            fn message(&mut self) -> String {
                loop {
                    let mut line = String::new();
                    let bytes = self
                        .control
                        .read_line(&mut line)
                        .expect("bounded child message");
                    assert_ne!(bytes, 0, "child exited before its expected control message");
                    if let Some((_, message)) = line.split_once("OBSERVER_CHILD_") {
                        return message.trim().to_owned();
                    }
                }
            }

            fn send(&mut self, message: &str) {
                writeln!(self.control.get_mut(), "{message}").expect("child control command");
                self.control.get_mut().flush().expect("flush child command");
            }

            fn finish(&mut self) -> ExitStatus {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    if let Some(status) = self.process.try_wait().expect("owned child status") {
                        return status;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "native child missed exit deadline"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
            }

            fn stderr(&mut self) -> String {
                let mut text = String::new();
                self.process
                    .stderr
                    .take()
                    .expect("owned child error pipe")
                    .take(16 * 1024)
                    .read_to_string(&mut text)
                    .expect("bounded child error output");
                text
            }
        }

        impl Drop for ProbeChild {
            fn drop(&mut self) {
                if matches!(self.process.try_wait(), Ok(None)) && self.process.kill().is_ok() {
                    let _ = self.process.wait();
                }
            }
        }

        fn file_evidence(root: &Path) -> BTreeMap<PathBuf, FileEvidence> {
            let paths = std::iter::once(root.to_owned()).chain(
                fs::read_dir(root)
                    .expect("read fixture directory")
                    .map(|entry| entry.expect("fixture entry").path()),
            );
            paths
                .map(|path| {
                    let metadata = fs::symlink_metadata(&path).expect("fixture metadata");
                    let sha256 = metadata.is_file().then(|| {
                        crate::util::hex_encode(&Sha256::digest(
                            fs::read(&path).expect("fixture bytes"),
                        ))
                    });
                    (
                        path.strip_prefix(root)
                            .expect("fixture-relative path")
                            .to_owned(),
                        FileEvidence {
                            sha256,
                            device: metadata.dev(),
                            inode: metadata.ino(),
                            mode: metadata.mode(),
                            links: metadata.nlink(),
                            uid: metadata.uid(),
                            gid: metadata.gid(),
                            bytes: metadata.len(),
                            modified: (metadata.mtime(), metadata.mtime_nsec()),
                            changed: (metadata.ctime(), metadata.ctime_nsec()),
                        },
                    )
                })
                .collect()
        }

        fn protected_evidence(
            evidence: &BTreeMap<PathBuf, FileEvidence>,
        ) -> BTreeMap<PathBuf, FileEvidence> {
            let mut protected = evidence.clone();
            // WAL-index bytes/times are bookkeeping; its identity and every other file stay protected.
            if let Some(shm) = protected.get_mut(Path::new("observer.db-shm")) {
                shm.sha256 = None;
                shm.modified = (0, 0);
                shm.changed = (0, 0);
            }
            protected
        }

        fn write_fixture_issue(database: &Path, id: &str) {
            let mut writer = crate::storage::SqliteStorage::open(database)
                .expect("prepare through ordinary obr storage writer");
            let now = chrono::Utc::now();
            let issue = crate::model::Issue {
                id: id.to_owned(),
                title: "Native observer fixture".to_owned(),
                created_at: now,
                updated_at: now,
                ..crate::model::Issue::default()
            };
            writer
                .create_issue(&issue, "observer-feasibility")
                .expect("seed issue");
        }

        fn prepared_fixture() -> (PathBuf, PathBuf, PathBuf) {
            let root = tempfile::Builder::new()
                .prefix("obr-observer-feasibility-")
                .tempdir()
                .expect("external fixture directory")
                .keep()
                .canonicalize()
                .expect("canonical fixture directory");
            let database = root.join("observer.db");
            write_fixture_issue(&database, "probe-1");
            fs::write(root.join("config.yaml"), "issue-prefix: probe\n")
                .expect("fixture application configuration");
            fs::write(
                root.join("PLAN.org"),
                "#+TITLE: External observer fixture\n",
            )
            .expect("fixture tracked surface");
            let leases: Vec<_> = fs::read_dir(&root)
                .expect("fixture entries")
                .map(|entry| entry.expect("fixture entry").path())
                .filter(|path| {
                    path.file_name()
                        .expect("fixture filename")
                        .to_string_lossy()
                        .starts_with(".br-db-openers-")
                })
                .collect();
            assert_eq!(leases.len(), 1, "ordinary writer prepared one opener lease");
            eprintln!("OBSERVER_FIXTURE {}", root.display());
            (root, database, leases[0].clone())
        }

        fn verify_protected(root: &Path, before: &BTreeMap<PathBuf, FileEvidence>, stage: &str) {
            let after = file_evidence(root);
            eprintln!(
                "OBSERVER_CHECK {stage} {} {}",
                root.display(),
                serde_json::to_string(&(before, &after)).expect("serialize file-family evidence")
            );
            assert_eq!(
                protected_evidence(before),
                protected_evidence(&after),
                "{stage} must preserve application state and protected file attributes"
            );
        }

        fn existing_input(path: &Path) -> Result<File, String> {
            let expected = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
            if !expected.is_file() || expected.nlink() != 1 {
                return Err(format!(
                    "unsafe nonregular/aliased input: {}",
                    path.display()
                ));
            }
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)
                .map_err(|error| error.to_string())?;
            let held = file.metadata().map_err(|error| error.to_string())?;
            if !held.is_file()
                || held.nlink() != 1
                || held.dev() != expected.dev()
                || held.ino() != expected.ino()
            {
                return Err(format!(
                    "input changed during descriptor admission: {}",
                    path.display()
                ));
            }
            Ok(file)
        }

        #[allow(clippy::incompatible_msrv)]
        fn open_probe(database: &Path, lease_path: &Path) -> Result<Probe, String> {
            let lease = ProbeLease(
                existing_input(lease_path).map_err(|error| format!("opener lease: {error}"))?,
            );
            lease
                .0
                .try_lock_shared()
                .map_err(|error| format!("opener lease lock: {error:?}"))?;
            let retained =
                existing_input(database).map_err(|error| format!("database input: {error}"))?;
            let metadata = retained.metadata().map_err(|error| error.to_string())?;
            if metadata.len() < 100 {
                return Err("database input: missing complete header".to_owned());
            }
            let identity = FileIdentity::from_file(&retained)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "native descriptor identity unsupported".to_owned())?;
            let pending =
                PendingNamespaceOpen::begin(database, NamespaceOpenIntent::ReadOnlyExisting)
                    .map_err(|error| format!("namespace admission: {error}"))?;
            if pending.expected_identity() != Some(identity) {
                return Err("namespace generation absent or mismatched".to_owned());
            }
            let namespace = pending.bind(identity).map_err(|error| error.to_string())?;
            namespace
                .validate_path_identity()
                .map_err(|error| error.to_string())?;
            let path = database.to_str().ok_or_else(|| {
                "database pathname is not representable by the native API".to_owned()
            })?;
            let inner = drive(
                fsqlite::Connection::open_schema_only_with_expected_identity(
                    path.to_owned(),
                    identity,
                ),
            )
            .map_err(|error| format!("native open: {error}"))?;
            let connection =
                Connection::from_inner(inner, false).map_err(|error| error.to_string())?;
            Ok(Probe {
                connection,
                namespace,
                identity_file: retained,
                _lease: lease,
            })
        }

        fn read_sample(probe: &Probe) -> Result<i64, String> {
            let conn = &probe.connection;
            conn.execute("BEGIN").map_err(|error| error.to_string())?;
            let observed = (|| {
                let version = conn
                    .query_row("PRAGMA user_version")
                    .map_err(|error| error.to_string())?;
                let version = version.get(0).and_then(SqliteValue::as_integer);
                if version != Some(17) {
                    return Err(format!("unsupported schema: {version:?}"));
                }
                let row = conn
                    .query_row("SELECT count(*) FROM issues")
                    .map_err(|error| error.to_string())?;
                row.get(0)
                    .and_then(SqliteValue::as_integer)
                    .ok_or_else(|| "issue count is not an integer".to_owned())
            })();
            let finish = if observed.is_ok() {
                "COMMIT"
            } else {
                "ROLLBACK"
            };
            conn.execute(finish).map_err(|error| error.to_string())?;
            observed
        }

        #[allow(clippy::incompatible_msrv)]
        fn checkpoint_available(lease_path: &Path) -> bool {
            let file = File::open(lease_path).expect("existing checkpoint contender");
            match file.try_lock() {
                Ok(()) => {
                    file.unlock().expect("release checkpoint contender");
                    true
                }
                Err(std::fs::TryLockError::WouldBlock) => false,
                Err(error) => panic!("unexpected checkpoint-lock failure: {error:?}"),
            }
        }

        fn prepared_native_read() {
            let (root, database, lease_path) = prepared_fixture();
            let before = file_evidence(&root);
            for stage in ["first-open", "warm-open"] {
                let mut probe =
                    open_probe(&database, &lease_path).expect("native observer admission");
                assert_eq!(read_sample(&probe), Ok(1));
                let length = probe
                    .connection
                    .query_row(
                        "SELECT length(CAST(title AS BLOB)) FROM issues WHERE id = 'probe-1'",
                    )
                    .expect("native byte-length preflight candidate");
                assert_eq!(length.get(0).and_then(SqliteValue::as_integer), Some(23));
                for statement in [
                    "UPDATE issues SET title = 'forbidden' WHERE id = 'probe-1'",
                    "CREATE TABLE forbidden_observer_schema_change (id INTEGER)",
                ] {
                    assert!(
                        matches!(
                            probe.connection.execute(statement),
                            Err(FrankenError::ReadOnly)
                        ),
                        "native read-only handle must refuse {statement}"
                    );
                }
                assert!(!checkpoint_available(&lease_path));
                probe
                    .connection
                    .close_in_place()
                    .expect("explicit owner-thread close");
                probe
                    .namespace
                    .validate_path_identity()
                    .expect("namespace bound through close");
                assert!(
                    probe
                        .connection
                        .query("SELECT count(*) FROM issues")
                        .is_err()
                );
                assert!(
                    !checkpoint_available(&lease_path),
                    "close does not release the guard early"
                );
                drop(probe);
                assert!(checkpoint_available(&lease_path));
                verify_protected(&root, &before, stage);
            }
        }

        fn missing_and_unsafe_inputs() {
            for stage in [
                "missing-db",
                "empty-db",
                "missing-lease",
                "missing-namespace",
                "stale-namespace",
            ] {
                let (root, database, lease_path) = prepared_fixture();
                eprintln!("OBSERVER_REFUSAL_STAGE {stage} {}", root.display());
                assert!(
                    checkpoint_available(&lease_path),
                    "{stage}: ordinary fixture writer still holds its opener lease"
                );
                match stage {
                    "missing-db" => fs::rename(&database, root.join("retired.db"))
                        .expect("owned missing-DB setup"),
                    "empty-db" => fs::write(&database, []).expect("owned empty-DB setup"),
                    "missing-lease" => fs::rename(&lease_path, root.join("lease-away"))
                        .expect("owned missing-lease setup"),
                    "missing-namespace" => fs::rename(
                        root.join("observer.db-fsqlite-ns-use"),
                        root.join("namespace-away"),
                    )
                    .expect("owned missing-namespace setup"),
                    "stale-namespace" => fs::write(
                        root.join("observer.db-fsqlite-ns-use"),
                        b"invalid namespace record",
                    )
                    .expect("owned stale-namespace setup"),
                    _ => unreachable!(),
                }
                let before = file_evidence(&root);
                let error =
                    open_probe(&database, &lease_path).expect_err("unsafe input must refuse");
                let expected = if stage.ends_with("db") {
                    "database input:"
                } else if stage == "missing-lease" {
                    "opener lease:"
                } else {
                    "namespace"
                };
                assert!(error.starts_with(expected), "{stage}: {error}");
                assert!(
                    stage == "missing-lease" || checkpoint_available(&lease_path),
                    "{stage}: refused probe still holds its opener lease"
                );
                verify_protected(&root, &before, stage);
            }
            let (root, database, lease_path) = prepared_fixture();
            let symlink = root.join("symlink.db");
            std::os::unix::fs::symlink(&database, &symlink).expect("owned symlink alias");
            let before = file_evidence(&root);
            assert!(
                open_probe(&symlink, &lease_path)
                    .expect_err("symlink refused")
                    .contains("unsafe")
            );
            verify_protected(&root, &before, "symlink-alias");
            let alias = root.join("hardlink.db");
            fs::hard_link(&database, &alias).expect("owned hard-link alias");
            let before = file_evidence(&root);
            for path in [&database, &alias] {
                assert!(
                    open_probe(path, &lease_path)
                        .expect_err("hard links refused")
                        .contains("unsafe")
                );
            }
            verify_protected(&root, &before, "hardlink-alias");
        }

        #[allow(clippy::incompatible_msrv)]
        fn contended_admission() {
            let (root, database, lease_path) = prepared_fixture();
            let exclusive = File::open(&lease_path).expect("exclusive lease contender");
            exclusive.try_lock().expect("exclusive lease setup");
            let before = file_evidence(&root);
            assert!(
                open_probe(&database, &lease_path)
                    .expect_err("exclusive opener must block")
                    .starts_with("opener lease lock:")
            );
            verify_protected(&root, &before, "exclusive-opener-refusal");
            drop(exclusive);
            let exclusive =
                PendingNamespaceOpen::begin(&database, NamespaceOpenIntent::ReservedExclusive)
                    .expect("exclusive native namespace setup");
            let before = file_evidence(&root);
            assert!(
                open_probe(&database, &lease_path)
                    .expect_err("exclusive namespace must block")
                    .starts_with("namespace")
            );
            assert!(checkpoint_available(&lease_path));
            verify_protected(&root, &before, "exclusive-namespace-refusal");
            drop(exclusive);
            let mut probe =
                open_probe(&database, &lease_path).expect("admission recovers after contention");
            assert_eq!(read_sample(&probe), Ok(1));
            probe
                .connection
                .close_in_place()
                .expect("close recovered reader");
            drop(probe);
        }

        fn schema_and_corruption_refusals() {
            for version in [16, 18] {
                let (root, database, lease_path) = prepared_fixture();
                let writer = Connection::open(database.to_string_lossy().into_owned())
                    .expect("fixture version writer");
                writer
                    .execute(&format!("PRAGMA user_version = {version}"))
                    .expect("fixture version change");
                writer.close().expect("close fixture version writer");
                let before = file_evidence(&root);
                let mut probe =
                    open_probe(&database, &lease_path).expect("native schema-only open");
                assert_eq!(
                    read_sample(&probe),
                    Err(format!("unsupported schema: Some({version})"))
                );
                probe
                    .connection
                    .close_in_place()
                    .expect("close refused-schema reader");
                drop(probe);
                assert!(checkpoint_available(&lease_path));
                verify_protected(&root, &before, &format!("schema-{version}-refusal"));
            }
            let (root, database, lease_path) = prepared_fixture();
            fs::write(&database, [0_u8; 100]).expect("owned corrupt-header fixture");
            let before = file_evidence(&root);
            assert!(
                open_probe(&database, &lease_path)
                    .expect_err("corrupt header refused")
                    .starts_with("native open:")
            );
            assert!(checkpoint_available(&lease_path));
            verify_protected(&root, &before, "corrupt-header-refusal");
        }

        fn wal_resident_schema() {
            let (root, database, lease_path) = prepared_fixture();
            let writer = Connection::open(database.to_string_lossy().into_owned())
                .expect("fixture WAL writer");
            writer
                .execute("PRAGMA user_version = 16")
                .expect("old main version");
            writer
                .execute("PRAGMA wal_checkpoint(TRUNCATE)")
                .expect("checkpoint fixture before observation");
            writer
                .execute("PRAGMA user_version = 17")
                .expect("current version only in WAL");
            let bytes = fs::read(&database).expect("main header witness");
            assert_eq!(
                u32::from_be_bytes(bytes[60..64].try_into().expect("user_version bytes")),
                16
            );
            let before = file_evidence(&root);
            let mut probe = open_probe(&database, &lease_path).expect("WAL-aware native reader");
            assert_eq!(read_sample(&probe), Ok(1));
            probe
                .connection
                .close_in_place()
                .expect("close WAL-aware reader");
            drop(probe);
            verify_protected(&root, &before, "WAL-resident-schema-17");
            writer
                .close()
                .expect("close fixture writer after observation evidence");
        }

        fn replacement_and_close_error() {
            let (root, database, lease_path) = prepared_fixture();
            let mut probe =
                open_probe(&database, &lease_path).expect("native reader before invalidation");
            probe
                .connection
                .execute("BEGIN")
                .expect("active read transaction");
            probe
                .connection
                .query_row("SELECT count(*) FROM issues")
                .expect("materialize read snapshot");
            fs::rename(&database, root.join("retired.db"))
                .expect("owned uncooperative path invalidation");
            let before = file_evidence(&root);
            let error = probe
                .connection
                .close_in_place()
                .expect_err("namespace drift must fail native rollback/close");
            eprintln!("OBSERVER_CLOSE_ERROR {error}");
            assert!(
                !checkpoint_available(&lease_path),
                "failed close retains opener guard"
            );
            assert!(matches!(
                PendingNamespaceOpen::begin(&database, NamespaceOpenIntent::ReservedExclusive),
                Err(FrankenError::Busy)
            ));
            drop(probe);
            assert!(checkpoint_available(&lease_path));
            let exclusive =
                PendingNamespaceOpen::begin(&database, NamespaceOpenIntent::ReservedExclusive)
                    .expect("native namespace locks released after failed-close teardown");
            drop(exclusive);
            verify_protected(&root, &before, "native-close-error-and-drop");
        }

        fn identity_replacement_refusal() {
            let (root, database, lease) = prepared_fixture();
            let (other_root, other_database, _) = prepared_fixture();
            let probe = open_probe(&database, &lease).expect("identity-bound reader guard");
            let retained = existing_input(&other_database).expect("unrelated retained identity");
            let wrong = FileIdentity::from_file(&retained)
                .expect("unrelated identity")
                .expect("native identity");
            let before = file_evidence(&root);
            let result = drive(
                fsqlite::Connection::open_schema_only_with_expected_identity(
                    database.to_str().expect("fixture pathname").to_owned(),
                    wrong,
                ),
            );
            assert!(matches!(result, Err(FrankenError::CannotOpen { .. })));
            verify_protected(&root, &before, "wrong-retained-identity");
            let expected = FileIdentity::from_file(&probe.identity_file)
                .expect("retained identity")
                .expect("native identity");
            fs::rename(&database, root.join("retired.db")).expect("owned retired generation");
            fs::rename(&other_database, &database).expect("owned pathname replacement");
            let before = file_evidence(&root);
            let other_before = file_evidence(&other_root);
            let result = drive(
                fsqlite::Connection::open_schema_only_with_expected_identity(
                    database.to_str().expect("fixture pathname").to_owned(),
                    expected,
                ),
            );
            assert!(matches!(result, Err(FrankenError::CannotOpen { .. })));
            assert!(
                open_probe(&database, &lease)
                    .expect_err("replacement needs new approval")
                    .starts_with("namespace")
            );
            drop(probe);
            assert!(checkpoint_available(&lease));
            verify_protected(&root, &before, "replacement-between-identity-and-open");
            verify_protected(&other_root, &other_before, "replacement-source-unchanged");
        }

        fn child_notice(message: &str) {
            println!("OBSERVER_CHILD_{message}");
            std::io::stdout()
                .flush()
                .expect("flush child control notice");
        }

        fn subprocess_worker() {
            let mode = std::env::var(CHILD_MODE).expect("parent-selected native child role");
            let database =
                PathBuf::from(std::env::var_os(CHILD_DATABASE).expect("owned fixture DB"));
            let lease = PathBuf::from(std::env::var_os(CHILD_LEASE).expect("owned fixture lease"));
            let root = database
                .parent()
                .expect("fixture parent")
                .canonicalize()
                .expect("fixture root");
            assert!(
                root.starts_with(
                    std::env::temp_dir()
                        .canonicalize()
                        .expect("external temp root")
                )
            );
            assert!(
                root.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("obr-observer-feasibility-"))
            );
            assert_eq!(
                database.file_name().and_then(|name| name.to_str()),
                Some("observer.db")
            );
            assert_eq!(lease.parent(), database.parent());
            if mode == "writer" {
                write_fixture_issue(&database, "probe-2");
                child_notice("WRITTEN");
                return;
            }
            assert_eq!(mode, "reader");
            let mut probe = open_probe(&database, &lease).expect("child native reader");
            probe
                .connection
                .execute("BEGIN")
                .expect("child read transaction");
            let row = probe
                .connection
                .query_row("SELECT count(*) FROM issues")
                .expect("child snapshot");
            assert_eq!(row.get(0).and_then(SqliteValue::as_integer), Some(1));
            child_notice("READY");
            loop {
                let mut command = String::new();
                assert_ne!(
                    std::io::stdin()
                        .read_line(&mut command)
                        .expect("child command"),
                    0
                );
                match command.trim() {
                    "close" => {
                        probe
                            .connection
                            .close_in_place()
                            .expect("native close before guard release");
                        child_notice("CLOSED");
                    }
                    "release" => {
                        drop(probe);
                        child_notice("RELEASED");
                        return;
                    }
                    "panic" => panic!("intentional native observer panic"),
                    "exit" => std::process::exit(0),
                    other => panic!("unexpected owned child command: {other}"),
                }
            }
        }

        fn subprocess_lifetimes() {
            for mode in ["close", "drop", "panic", "exit"] {
                let (root, database, lease) = prepared_fixture();
                let before = file_evidence(&root);
                let mut child = ProbeChild::start("reader", &database, &lease);
                assert_eq!(child.message(), "READY");
                assert!(!checkpoint_available(&lease));
                match mode {
                    "close" => {
                        child.send("close");
                        assert_eq!(child.message(), "CLOSED");
                        assert!(
                            !checkpoint_available(&lease),
                            "explicit close retains guard until release"
                        );
                        child.send("release");
                    }
                    "drop" => child.send("release"),
                    "panic" | "exit" => child.send(mode),
                    _ => unreachable!(),
                }
                if matches!(mode, "close" | "drop") {
                    assert_eq!(child.message(), "RELEASED");
                }
                let status = child.finish();
                let errors = child.stderr();
                if mode == "panic" {
                    assert!(!status.success(), "intentional panic must be detected");
                    assert!(
                        errors.contains("intentional native observer panic"),
                        "{errors}"
                    );
                } else {
                    assert!(status.success(), "{mode}: {errors}");
                }
                eprintln!(
                    "OBSERVER_CHILD_EXIT mode={mode} pid={} code={:?} stderr={errors:?}",
                    child.process.id(),
                    status.code()
                );
                assert!(checkpoint_available(&lease));
                let exclusive =
                    PendingNamespaceOpen::begin(&database, NamespaceOpenIntent::ReservedExclusive)
                        .expect("kernel namespace locks released after child shutdown");
                drop(exclusive);
                verify_protected(&root, &before, &format!("subprocess-{mode}"));
            }
        }

        fn subprocess_coherent_reads() {
            let (root, database, lease) = prepared_fixture();
            let before = file_evidence(&root);
            let mut probe = open_probe(&database, &lease).expect("reader before separate writer");
            probe
                .connection
                .execute("BEGIN")
                .expect("pinned read transaction");
            let row = probe
                .connection
                .query_row("SELECT count(*) FROM issues")
                .expect("pre-commit issue count");
            assert_eq!(row.get(0).and_then(SqliteValue::as_integer), Some(1));
            let mut child = ProbeChild::start("writer", &database, &lease);
            assert_eq!(child.message(), "WRITTEN");
            let status = child.finish();
            let errors = child.stderr();
            assert!(status.success(), "{errors}");
            eprintln!(
                "OBSERVER_WRITER_EXIT pid={} code={:?} stderr={errors:?}",
                child.process.id(),
                status.code()
            );
            for table in ["issues", "events"] {
                let row = probe
                    .connection
                    .query_row(&format!("SELECT count(*) FROM {table}"))
                    .expect("same committed snapshot");
                assert_eq!(
                    row.get(0).and_then(SqliteValue::as_integer),
                    Some(1),
                    "{table} must remain on the old snapshot"
                );
            }
            probe
                .connection
                .execute("COMMIT")
                .expect("finish old snapshot");
            probe.connection.close_in_place().expect("close old reader");
            drop(probe);
            assert!(checkpoint_available(&lease));
            let mut probe =
                open_probe(&database, &lease).expect("fresh reader after writer commit");
            assert_eq!(read_sample(&probe), Ok(2));
            let events = probe
                .connection
                .query_row("SELECT count(*) FROM events")
                .expect("new complete event state");
            assert_eq!(events.get(0).and_then(SqliteValue::as_integer), Some(2));
            probe
                .connection
                .close_in_place()
                .expect("close fresh reader");
            drop(probe);
            let after = file_evidence(&root);
            for file in ["config.yaml", "PLAN.org"] {
                assert_eq!(before[Path::new(file)], after[Path::new(file)]);
            }
            eprintln!(
                "OBSERVER_COHERENCE {} old=1 after_writer=1 reopened=2 writer_pid={}",
                root.display(),
                child.process.id()
            );
        }

        fn application_mutation_control() {
            let (root, database, _) = prepared_fixture();
            let before = file_evidence(&root);
            write_fixture_issue(&database, "probe-2");
            let after = file_evidence(&root);
            assert_ne!(
                before[Path::new("observer.db")].sha256,
                after[Path::new("observer.db")].sha256,
                "ordinary committed issue creation changes the database"
            );
            assert_ne!(protected_evidence(&before), protected_evidence(&after));
            fs::write(root.join("config.yaml"), "issue-prefix: changed\n")
                .expect("positive control application-config mutation");
            assert_ne!(
                protected_evidence(&after),
                protected_evidence(&file_evidence(&root)),
                "the bookkeeping exception must not hide application-config writes"
            );
        }

        fn on_worker(probe: fn()) {
            std::thread::Builder::new()
                .name("observer-feasibility".to_owned())
                .stack_size(WORKER_STACK_BYTES)
                .spawn(probe)
                .expect("native observer worker")
                .join()
                .expect("native observer probe passed");
        }

        #[test]
        fn observer_feasibility_prepared_native_read_preserves_application_state() {
            on_worker(prepared_native_read);
        }

        #[test]
        fn observer_feasibility_detects_application_mutation() {
            on_worker(application_mutation_control);
        }

        #[test]
        fn observer_feasibility_missing_and_unsafe_inputs_refuse() {
            on_worker(missing_and_unsafe_inputs);
        }

        #[test]
        fn observer_feasibility_contention_refuses_without_degrading() {
            on_worker(contended_admission);
        }

        #[test]
        fn observer_feasibility_schema_and_corruption_refuse() {
            on_worker(schema_and_corruption_refusals);
        }

        #[test]
        fn observer_feasibility_schema_is_wal_aware() {
            on_worker(wal_resident_schema);
        }

        #[test]
        fn observer_feasibility_real_close_error_retains_and_releases_guards() {
            on_worker(replacement_and_close_error);
        }

        #[test]
        fn observer_feasibility_identity_mismatch_and_replacement_refuse() {
            on_worker(identity_replacement_refusal);
        }

        #[test]
        fn observer_feasibility_subprocess_close_drop_panic_exit_release_locks() {
            on_worker(subprocess_lifetimes);
        }

        #[test]
        fn observer_feasibility_separate_writer_preserves_snapshot() {
            on_worker(subprocess_coherent_reads);
        }

        #[test]
        #[ignore = "native subprocess entrypoint exercised by parent tests"]
        fn observer_feasibility_subprocess_worker() {
            on_worker(subprocess_worker);
        }
    }
}
