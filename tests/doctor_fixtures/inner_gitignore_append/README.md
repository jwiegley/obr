# inner_gitignore_append

- **FM**: `fm-configs-gitignore-leaking-obr` (P2, inner subset) —
  `.obr/.gitignore` does not ignore the directory wholesale, so the
  per-machine cache it holds can leak into git history. Under D-SURFACE
  nothing under `.obr/` is ever tracked, so the required content is a bare
  `*` (a root `.gitignore` covering `.obr/` satisfies it equally).
- **Subsystem**: configs
- **Detect**: `gitignore.obr_inner_present` goes to `warn` when neither the
  inner `.gitignore` nor the root one hides the whole workspace directory.
- **Repair contract**: SAFETY — `--repair` appends the self-ignore line
  via the `mutate()` chokepoint (`Op::AppendFile`).
  Symlinked `.obr/.gitignore` is REFUSED (operator intent may
  point at a vendored shared config). Existing operator-written
  lines are preserved verbatim; only the missing canonical lines
  are appended at end-of-file, with a separator newline inserted
  if the file's last byte is not `\n`.
- **Round-trip**: write a `.obr/.gitignore` that enumerates artifacts
  (`*.lock`) plus an operator-custom line but never ignores the directory
  wholesale → detect the missing wholesale rule → `--repair` appends `*` →
  re-detect ok with operator lines preserved → `doctor undo` restores the
  incomplete state.
- **Idempotence**: a second `--repair` finds no divergence; zero
  actions.
- **Expected exit codes**:
    - detect: 1 (warn present)
    - repair: 0 (self-ignore line appended)
    - undo: 0 (incomplete state restored byte-deterministically)
