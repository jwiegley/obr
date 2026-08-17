# multiple_obr_in_path

- **FM**: `fm-external_artifacts-multiple-obr-in-path` (P1)
- **Subsystem**: external_artifacts
- **Detect**: `obr_path_dupes` warns when more than one executable named `obr`
  exists on `$PATH`, and the check details include the canonical FM id.
- **Repair contract**: detect-only. `obr doctor --repair` must not rewrite or
  remove any discovered `obr` binary. The operator must fix PATH ordering or
  stale installs manually.
- **Round-trip**: create two executable `obr` stubs under fixture-local
  directories, prepend those directories only for doctor invocations, assert
  the detector fires, assert the stubs remain byte-identical after repair, and
  assert undo does not touch the stubs.
