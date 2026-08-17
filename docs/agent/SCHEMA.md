# Schemas

obr provides a schema surface describing the primary machine-readable outputs.

## Emit schemas

```bash
obr schema all --format json
obr schema issue-details --format json
obr schema error --format json
```

TOON is also supported:

```bash
obr schema all --format toon
```

## If `obr schema` is missing

If `obr schema --help` fails with "unrecognized subcommand", you're running an older `obr` binary.

Options:

Build from source in this repo and use the local binary:

```bash
CARGO_TARGET_DIR=target cargo build
./target/debug/obr schema all --format json
```

As a fallback, this repo also includes a captured snapshot bundle under:

- `agent_baseline/schemas/`

Those snapshots are checked against the built binary by the
`agent_baseline_snapshots_match_current_binary` test. After intentional schema
changes, regenerate them with:

```bash
UPDATE_AGENT_BASELINE=1 cargo test --test e2e_schema agent_baseline_snapshots_match_current_binary -- --nocapture
```

## Key folding (TOON)

When emitting TOON, obr may "fold" nested keys into dotted keys (safe folding) to save tokens.
Example: `schemas.IssueDetails` instead of `{ "schemas": { "IssueDetails": ... } }`.

If you need to parse TOON as nested JSON, decode with safe path expansion:

```bash
obr schema issue-details --format toon | tru --decode --expand-paths safe | jq .
```
