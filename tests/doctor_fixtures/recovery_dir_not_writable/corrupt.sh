#!/usr/bin/env bash
# Fixture: recovery_dir_not_writable
# FM: fm-permissions-recovery-dir-not-writable.

set -euo pipefail

target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-br}"

mkdir -p "$target_dir"
cd "$target_dir"

"$tool_bin" init --quiet 2>&1

# Keep the fixture detect-only under the `.obr/` surface.
printf '*\n' > .obr/.gitignore

mkdir -p .obr/recovery
printf 'fixture-seed\n' > .obr/recovery/sentinel.txt
chmod 0555 .obr/recovery

echo "fixture corrupt.sh: locked .obr/recovery at mode 0555" >&2
