# Workspace Failure Fixtures

This corpus stores small, sanitized workspace roots for reliability regression work.

Each fixture directory contains:

- `fixture.json`: human-readable metadata about the failure family and why it matters
- `fixture.json.expected_command_outcomes`: the replay contract for the key surfaces this fixture is expected to exercise
- `beads/`: the workspace payload that the loader materializes into `.obr/` inside an isolated test root

Conventions:

- Every fixture is a complete workspace root, not just a raw `.obr` dump.
- The checked-in payload lives under visible `beads/` because the remote `rch` transport used for cargo test/check/clippy does not preserve newly added hidden untracked directories reliably.
- Sidecars, recovery artifacts, and other debris are preserved when they are the point of the case.
- The payloads are intentionally small so they can live in git and be inspected by hand.
- New fixtures should model one primary anomaly each. If a future incident needs a new combination, add a new directory instead of mutating an unrelated case.

Current families covered here:

- corrupt or non-SQLite `obr.db`
- JSONL conflict-marker corruption
- DB/JSONL disagreement
- duplicate config rows after legacy-schema drift
- metadata-based custom path discovery
- WAL sidecar without matching SHM
- interrupted rebuild leftovers and recovery debris
