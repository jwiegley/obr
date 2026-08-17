# permissions_write_lock_unwritable

- **FM**: `fm-state_files-orphaned-write-lock`
- **Detector**: `permissions.write_lock`
- **Severity**: warn
- **Repair contract**: detect-only. `obr doctor --repair` must refuse before
  mutation because it cannot acquire the workspace write lock, and it must not
  chmod or remove an operator-controlled `.obr/.write.lock`.
- **Round-trip**: create a regular `.obr/.write.lock` with no owner-write
  bit -> plain `obr doctor --json` emits a doctor report with
  `permissions.write_lock` -> `--repair` returns `concurrency_lost` -> undo is
  a no-op and the file remains read-only.
- **Which envelope (obr-m6m)**: read-only `obr doctor` no longer takes the
  workspace write lock (it was blinding the sibling `write_lock` probe, which
  collided with its own caller and reported "held by a live process" for every
  workspace). So plain doctor now *starts* on an unwritable lock and reports
  the condition as an ordinary in-report check: `permissions.write_lock` warn,
  `workspace_health` degraded via the `write_lock_not_writable` anomaly,
  `ok=false`. The old startup-abort envelope — one synthetic check plus
  `details.startup_error` — is unreachable for plain doctor by design.
  `--repair` still acquires the lock, so it still fails at startup with
  `concurrency_lost` / exit 5, and the `post_repair` stage still asserts that.
- **Environment skip (beads_rust-ypwu)**: the detect stage re-checks the
  planted precondition before running doctor and exits 3 (the suite's skip
  protocol) when the environment cannot hold it: the lock vanished or became
  a non-regular file, the filesystem dropped the 0444 mode bits, or the
  current uid can still write the file despite mode 444 (root or
  `CAP_DAC_OVERRIDE`, as on some remote build workers). A precondition the
  environment cannot hold is not a product failure.

