#!/usr/bin/env bash
# Fixture: write_lock_symlink_node
# FM: fm-concurrency_primitives-orphaned-write-lock (beads_rust-5sej) — detect-only.
#
# Plant a symlink; write admission must refuse it without touching either inode.

set -euo pipefail
target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-obr}"

mkdir -p "$target_dir"
cd "$target_dir"
"$tool_bin" init >/dev/null 2>&1

rm -f .obr/.write.lock
: > .obr/.lock_target
ln -s .lock_target .obr/.write.lock
# GNU stat first, BSD stat fallback (macOS workers).
{ stat -c '%d:%i' .obr/.lock_target 2>/dev/null \
    || stat -f '%d:%i' .obr/.lock_target; } > .fixture_target_identity

echo "fixture corrupt.sh: planted symlinked .write.lock -> .lock_target" >&2
