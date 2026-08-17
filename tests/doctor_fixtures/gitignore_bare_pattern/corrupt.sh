#!/usr/bin/env bash
# Fixture: gitignore_bare_pattern
# Variant of fm-configs-gitignore-leaking-obr using the `*.org` extension glob
# rather than naming the surface outright. A matcher that only looks for the
# literal filename slips straight past it, yet git hides `PLAN.org` all the
# same. Confirms the detector handles both shapes.

set -euo pipefail
target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-br}"

mkdir -p "$target_dir"
cd "$target_dir"
git init --quiet 2>/dev/null || true
"$tool_bin" init >/dev/null 2>&1

cat > .gitignore <<'EOF'
*.log
*.org
build/
EOF

if [ -e .fixture_baseline ]; then
  echo "fixture baseline already exists; expected a fresh workspace" >&2
  exit 1
fi
mkdir -p .fixture_baseline
tar --exclude=.fixture_baseline -cf .fixture_baseline/state.tar .
