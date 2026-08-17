#!/usr/bin/env bash
# Fixture: mcp_serve_stale_write_lock
# FM: fm-agent_coordination-mcp-serve-stale-write-lock.
#
# Simulates a killed `obr serve` owner by planting an old regular lock inode and
# an orphan holder-pid sidecar. No process owns the advisory lock.

set -euo pipefail

target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-br}"

mkdir -p "$target_dir"
cd "$target_dir"

"$tool_bin" init --quiet 2>&1
"$tool_bin" create --title "mcp stale lock seed" --type task --priority 2 --json >/dev/null
# Flush the seed, as the sibling fixtures do. `create` alone leaves the database
# ahead of the export, so doctor reports counts.db_vs_jsonl and sync.metadata on
# top of the lock finding and exits 1 — and this fixture's whole contract is that
# a healthy persistent lock inode leaves doctor at exit 0. Same reason the
# workspace ignore file is sanitised below: keep it a single-finding case.
"$tool_bin" sync --flush-only >/dev/null 2>&1

# The workspace ignore file must stay valid for the `.obr/` surface.
printf '*\n' >.obr/.gitignore

: >.obr/.write.lock
printf '99999999\n' >.obr/.write.lock.holder.pid
touch -d '2024-01-01T00:00:00Z' .obr/.write.lock
touch -d '2024-01-01T00:00:00Z' .obr/.write.lock.holder.pid
lock_identity=$(stat -c '%d:%i' .obr/.write.lock 2>/dev/null || stat -f '%d:%i' .obr/.write.lock)
printf '%s\n' "$lock_identity" >.fixture_lock_identity

echo "fixture corrupt.sh: planted persistent .write.lock and orphan holder pid sidecar" >&2
