# write_lock_symlink_node

**FM:** `fm-concurrency_primitives-orphaned-write-lock` (bead `beads_rust-5sej`) — detect-only.

## Shape

`.obr/.write.lock` is a **symlink** to a sibling regular file
(`.obr/.lock_target`). Write admission must reject this node from `lstat`
before opening it: following the symlink would move mutual exclusion to
another inode and split authority from the canonical lock path. Read-only
doctor can inspect and report this node without acquiring write admission.

## Contract

- **Detect:** `write_lock` reports `status: error`,
  `reason: non_regular_lock_node` and `node_kind: symlink`; doctor exits
  non-zero. A separate mutator must fail with the typed startup refusal
  while leaving database bytes unchanged.
- **Never mutates:** neither the symlink nor its target is removed, renamed,
  retargeted, or replaced in any stage, including `--repair` and undo. Moving
  a lock node aside while a holder has the old inode locked would split
  mutual exclusion across two files; the operator must intervene manually.

The sibling **directory** shape cannot be exercised through this harness:
a directory in the lock slot makes startup lock acquisition fail before any
check runs (which is itself fail-closed). That shape is covered by the unit
test `check_orphaned_write_lock_directory_fails_closed` and the CLI e2e in
`tests/e2e_doctor_write_lock_shapes.rs`.
