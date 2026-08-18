#!/usr/bin/env bash
# Fixture: permissions_obr_dir_readonly
# FM: fm-permissions-obr-dir-readonly (P0) — detect-only.
#
# Plant a `.obr/issues.jsonl` with mode 0444 (no user-write bit).
# The new pass-2 detector `check_permissions_obr_dir` must surface
# the file's mode and the canonical `chmod u+w` fix. --repair must
# NOT silently re-chmod (operator intent unknowable).

set -euo pipefail
target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-br}"

mkdir -p "$target_dir"
cd "$target_dir"
"$tool_bin" init >/dev/null 2>&1

# Strip user-write bit from issues.jsonl (POSIX mode 0444).
chmod 0444 .obr/issues.jsonl

if [ -e .fixture_baseline ]; then
  echo "fixture baseline already exists; expected a fresh workspace" >&2
  exit 1
fi
mkdir -p .fixture_baseline
# tar captures the POST-chmod state so undo restores mode 0444; tar reads a
# mode-0444 file regardless of the missing user-write bit.
tar --exclude=.fixture_baseline -cf .fixture_baseline/state.tar .
