# Fuzz Targets

This directory contains cargo-fuzz targets for `obr`.

Run the JSONL import harness in bounded mode:

```bash
cargo fuzz run jsonl_import -- -runs=10000 -max_len=65536
```

Run the three-way merge harness in bounded mode:

```bash
cargo fuzz run merge_issue -- -runs=10000 -max_len=8192
```

Run the content-hash harness in bounded mode:

```bash
cargo fuzz run content_hash -- -runs=10000 -max_len=4096
```

Run the sync-cycle harness in bounded mode:

```bash
cargo fuzz run sync_cycle -- -runs=10000 -max_len=16384
```

Run a compile check without fuzzing:

```bash
cargo fuzz check jsonl_import
cargo fuzz check merge_issue
cargo fuzz check content_hash
cargo fuzz check sync_cycle
```

The harnesses create isolated temporary workspaces and must not read or write
the repository's real `.beads/` directory.

The `merge_issue` target feeds randomized base/local/external triples into the
same sync merge logic used by `br sync --merge`. It checks deterministic
termination, kept-issue content hashes, explicit conflict reporting for manual
resolution, documented `--force-db`/`--force-jsonl` winners, and tombstone
protection in the one-issue `three_way_merge` path. Hand-written JSON seeds live
under `fuzz/corpus/merge_issue/` and can be promoted to normal regression tests
if they expose a minimized failure.

The `content_hash` target treats JSON serialization whitespace, JSON field
order, unknown JSON fields, empty optional fields, labels, relationships, and
metadata-only issue fields as formatting-insensitive. Whitespace inside included
issue text fields such as title, description, design, acceptance criteria, and
notes is intentionally treated as meaningful issue content.

The `sync_cycle` target builds isolated temp workspaces and exercises broader
sync state-machine paths: JSONL export, staleness/status checks, import
preflight, import, re-export, DB reopen, rejected `.git` paths, symlink escape
attempts, sidecar files, base snapshots, custom JSONL locations, and
rebuild-style imports into a fresh database. It checks that invalid combinations
return non-empty structured errors and that the outside sentinel directory is
not modified.

Run the Org-mode parser harness (the default export format's import
boundary) against its seeded corpus:

```bash
cargo fuzz run org_parse -- -runs=10000 -max_len=131072
```

Run the string-parser/validator harness:

```bash
cargo fuzz run validation -- -runs=10000
```
