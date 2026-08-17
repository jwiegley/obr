# gitignore_bare_pattern

- **FM**: `fm-configs-gitignore-leaking-obr` (variant: extension glob, surface not named)
- **Severity**: P0
- **Subsystem**: configs
- **Detect**: `gitignore.obr_inner` check goes to `warn`, naming `*.org`
- **Repair contract**: Removes nothing. `*.org` is a broad rule, and
  `fix_root_gitignore_if_warned` only deletes lines whose whole meaning is
  "hide the surface". Repair reports the refusal and names the rule.
- **Round-trip**: N/A — repair is a no-op, so undo has nothing to restore.
- **Expected exit codes**:
    - detect: 1
    - repair: 0 or 2
    - undo: 0

Confirms the detector catches a glob that hides `PLAN.org` without naming it,
not just a literal `PLAN.org` line — and that the fixer stops there.

Removing `*.org` would un-hide every other org file in the tree; that is an
edit to the operator's intent, not a repair. The sibling fixture
`gitignore_leaking_obr` covers the case the fixer *does* rewrite, where a line
names the surface outright. Read the two together before changing either: a
change that makes this fixture's `*.org` disappear has broken the policy, not
satisfied it.
