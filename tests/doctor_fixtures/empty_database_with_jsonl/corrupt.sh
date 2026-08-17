#!/usr/bin/env bash
# Fixture: empty_database_with_jsonl
# FM: fm-state_files-empty-or-truncated-database (P0)
#
# Plants a workspace where `.obr/obr.db` has been truncated to 0 bytes but
# `.obr/issues.jsonl` retains real records. doctor reports
# schema.tables=error (no tables) and workspace_health=recoverable;
# `--repair` rebuilds the DB from JSONL.

set -euo pipefail
target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-br}"

mkdir -p "$target_dir"
cd "$target_dir"
"$tool_bin" init >/dev/null 2>&1

# Seed a real issue so the JSONL has content to rebuild from.
"$tool_bin" create --title "fixture seed" --description "for db rebuild" \
  --priority 2 --no-auto-flush >/dev/null 2>&1 || true
# Flush to JSONL.
"$tool_bin" sync --flush-only >/dev/null 2>&1 || true

# Now truncate the DB and WAL.
: > .obr/obr.db
[ -f .obr/obr.db-wal ] && : > .obr/obr.db-wal || true

if [ -e .fixture_baseline ]; then
  echo "fixture baseline already exists; expected a fresh workspace" >&2
  exit 1
fi
mkdir -p .fixture_baseline
tar --exclude=.fixture_baseline -cf .fixture_baseline/state.tar .
