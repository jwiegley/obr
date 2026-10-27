#!/usr/bin/env bash
# Fixture: wal_without_shm
# FM: fm-state_files-wal-shm-sidecar-orphan (P1) — WAL-without-SHM variant.
#
# Start with a regular WAL and no SHM file. A subsequent native read may
# create shared-index bookkeeping; both regular layouts must stay healthy.
# Repair must not discard a valid WAL without first checkpointing its data.

set -euo pipefail
target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-obr}"

mkdir -p "$target_dir"
cd "$target_dir"
"$tool_bin" init >/dev/null 2>&1
[ -f .obr/obr.db-wal ] || { echo "fixture WAL missing after init" >&2; exit 1; }
# Establish the input shape before any diagnostic read can create SHM.
rm -f .obr/obr.db-shm
[ ! -e .obr/obr.db-shm ] || { echo "fixture SHM should be absent" >&2; exit 1; }

if [ -e .fixture_baseline ]; then
  echo "fixture baseline already exists; expected a fresh workspace" >&2
  exit 1
fi
mkdir -p .fixture_baseline
tar --exclude=.fixture_baseline -cf .fixture_baseline/state.tar .
