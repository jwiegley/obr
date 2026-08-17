# gitignore_leaking_obr

- **FM**: `fm-configs-gitignore-leaking-obr`
- **Severity**: P0
- **Subsystem**: configs
- **Detect**: `gitignore.obr_inner` check goes to `warn` — the root
  `.gitignore` hides the tracked surface (`PLAN.org`) from git.
- **Repair contract**: `--repair` rewrites the root `.gitignore` through the
  `mutate()` chokepoint, removing `PLAN.org` while preserving every other line.
- **Round-trip**: YES — chokepointed. `undo latest` restores `.gitignore`
  byte-identically; post-undo state matches `.fixture_baseline/state.tar`.
- **Expected exit codes**:
    - detect (`obr doctor --json`): 1 (findings present)
    - repair (`obr doctor --repair --json`): 0 or 2 (repair_applied / partial)
    - undo (`obr doctor undo latest --json`): 0

This is the chokepoint reference fixture — the only one in this suite where
the full corrupt → detect → repair → undo cycle restores byte-equivalence,
because every mutation goes through `mutate()`. Other fixtures may have
chokepoint coverage gaps; see `tests/doctor_fixtures/README.md`.
