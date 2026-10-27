# wal_without_shm

- **FM**: `fm-state_files-wal-shm-sidecar-orphan` (P1) — WAL-without-SHM
  variant (benign for frankensqlite).
- **Subsystem**: state_files
- **Detect**: the fixture verifies a regular WAL with no SHM before any
  diagnostic read. `db.sidecars` stays `ok` whether that read creates a regular
  shared WAL-index file or leaves the WAL-only layout intact. SHM without WAL
  remains a separate error; file presence alone does not replace integrity
  validation.
- **Repair contract**: SAFETY — `--repair` MUST NOT delete the WAL (it may
  carry committed-but-not-checkpointed data).
- **Round-trip**: N/A — no destructive auto-fix.
- **Expected exit codes**:
    - detect: 0 with the documented `RUST_LOG=error` environment (an unrelated
      verbose-log advisory may otherwise make the aggregate doctor exit 1)
    - repair: 0 or 2
    - undo: 0
