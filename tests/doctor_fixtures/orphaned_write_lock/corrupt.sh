#!/usr/bin/env bash
# Fixture: orphaned_write_lock
# FM: fm-concurrency_primitives-orphaned-write-lock (P1) — detect-only.
#
# Plants an old `.obr/.write.lock` regular file. Production keeps this inode
# across successful commands; its mtime is deliberately irrelevant to the
# advisory lock held by an open file description.

set -euo pipefail
target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-br}"

mkdir -p "$target_dir"
cd "$target_dir"
"$tool_bin" init >/dev/null 2>&1

: > .obr/.write.lock
touch -d '2024-01-01T00:00:00Z' .obr/.write.lock
# `stat -c` is GNU coreutils; BSD/macOS stat spells the same two stat(2)
# fields with `-f`. `%d` (st_dev) and `%i` (st_ino) mean the same thing in
# both dialects, so the recorded identity is comparable on either host.
lock_identity=$(stat -c '%d:%i' .obr/.write.lock 2>/dev/null || stat -f '%d:%i' .obr/.write.lock)
printf '%s\n' "$lock_identity" > .fixture_lock_identity

if [ -e .fixture_baseline ]; then
  echo "fixture baseline already exists; expected a fresh workspace" >&2
  exit 1
fi
mkdir -p .fixture_baseline
tar --exclude=.fixture_baseline -cf .fixture_baseline/state.tar .
