#!/usr/bin/env bash
# Fixture: base_jsonl_symlink_quarantine
# FM: fm-state_files-base-jsonl-missing-or-stale (P2, SYMLINK subset)
#
# Initialises a workspace with at least one issue (so .obr/issues.jsonl
# exists and is non-empty — the detector's stale-anchor branch requires
# this, but more importantly we use it as the symlink target so the link
# resolves cleanly during inspection). Plants a symlink at
# `.obr/merge.base.jsonl` pointing to `issues.jsonl` (relative path,
# stays inside .obr/). Doctor's `base_jsonl` check should fire warn
# with `details.kind == "symlink"`; `--repair` quarantines the symlink
# under the run-dir.

set -euo pipefail
target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-br}"

mkdir -p "$target_dir"
cd "$target_dir"
"$tool_bin" init >/dev/null 2>&1
"$tool_bin" create "seed-base-jsonl" --type task --priority 2 \
    >/dev/null 2>&1
"$tool_bin" sync --flush-only >/dev/null 2>&1

# Remove any real anchor that sync may have left, then plant a symlink
# pointing at the live JSONL.
rm -f .obr/merge.base.jsonl
ln -s issues.jsonl .obr/merge.base.jsonl

# Sanity: confirm we actually created a symlink (not a regular file).
if [ ! -L .obr/merge.base.jsonl ]; then
    echo "corrupt.sh: expected .obr/merge.base.jsonl to be a symlink" >&2
    exit 1
fi

if [ -e .fixture_baseline ]; then
    echo "fixture baseline already exists; expected a fresh workspace" >&2
    exit 1
fi
mkdir -p .fixture_baseline
tar --exclude=.fixture_baseline -cf .fixture_baseline/state.tar .
