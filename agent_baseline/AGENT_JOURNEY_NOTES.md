# Agent Journey Notes (Zero-Shot)

Goal: validate that a "fresh" agent can use obr using only docs + `--help` (no source reading).

Date: 2026-01-25

Freshness: these files are current snapshots, not historical examples. The
integration test `agent_baseline_snapshots_match_current_binary` compares the
captured help, schema, example, version, and error artifacts against the built
`obr` binary.

Regenerate after intentional CLI/schema/output changes:

```bash
UPDATE_AGENT_BASELINE=1 cargo test --test e2e_schema agent_baseline_snapshots_match_current_binary -- --nocapture
```

Check without rewriting files:

```bash
cargo test --test e2e_schema agent_baseline_snapshots_match_current_binary
```

## Tasks attempted

1. List ready issues in TOON and decode to JSON.
2. Fetch schemas for `issue-details` and `error`.
3. Find a smoke test for robot outputs.

## What worked

- TOON + decode pipeline was discoverable from:
  - `docs/agent/EXAMPLES.md`
  - `docs/agent/QUICKSTART.md`
- `obr ready --help` documents `--format` and `--robot` (when available).

## Confusions / gaps found

- If the installed `obr` binary is behind `main`, `obr schema` may be missing even though it is
  documented in `docs/agent/SCHEMA.md`. This is now called out in `docs/agent/SCHEMA.md`.
- TOON decoding depends on `tru --decode`. We now document what `tru` is and how to verify it is installed.
- The "robot output smoke test" is now explicitly linked from the agent docs via `scripts/agent_smoke_test.sh`.
