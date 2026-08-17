#!/usr/bin/env bash
# Fixture: inner_gitignore_append
# FM: fm-configs-gitignore-leaking-obr (P2, inner subset)
#
# Initialises a workspace, replaces .obr/.gitignore with a hand-written file
# that enumerates a few artifacts but never ignores the directory wholesale.
# Under D-SURFACE nothing inside `.obr/` is ever tracked, so anything short of
# a bare `*` leaves cache files exposed. Doctor should append the missing `*`
# while preserving every existing line.

set -euo pipefail
target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-br}"

mkdir -p "$target_dir"
cd "$target_dir"
"$tool_bin" init >/dev/null 2>&1

"$tool_bin" create --title "fixture issue 1" --type task --priority 2 >/dev/null 2>&1
"$tool_bin" sync --flush-only >/dev/null 2>&1

# Capture export bytes pre-corruption — Op::AppendFile on the gitignore must
# NEVER touch the export. Under D-SURFACE that export is the tracked surface at
# the workspace root, not a file inside `.obr/`.
sha256sum PLAN.org | awk '{print $1}' > .fixture_jsonl_pre_sha256

# Replace the auto-generated inner .gitignore with an incomplete file that has
# an operator-custom line plus per-artifact rules, but no wholesale ignore.
cat > .obr/.gitignore <<'GITIGNORE'
# operator-custom rule
local-cache/
*.lock
GITIGNORE

# Capture the incomplete bytes so post_undo can verify byte-
# deterministic restore.
sha256sum .obr/.gitignore | awk '{print $1}' > .fixture_inner_gitignore_pre_sha256
cp .obr/.gitignore .fixture_inner_gitignore_pre.txt

if [ -e .fixture_baseline ]; then
  echo "fixture baseline already exists; expected a fresh workspace" >&2
  exit 1
fi
mkdir -p .fixture_baseline
tar --exclude=.fixture_baseline -cf .fixture_baseline/state.tar .
