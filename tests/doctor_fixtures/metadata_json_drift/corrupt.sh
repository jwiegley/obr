#!/usr/bin/env bash
# Fixture: metadata_json_drift
# FM: fm-configs-metadata-json-stale (P1) — detect-only.
#
# Plant a `.obr/metadata.json` that declares a `jsonl_export`
# pointing at a file that doesn't exist on disk. The new pass-2
# detector `check_metadata_json` must surface the drift with the
# field name + the expected path. `--repair` must NOT silently
# rewrite the file.

set -euo pipefail
target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-br}"

mkdir -p "$target_dir"
cd "$target_dir"
"$tool_bin" init >/dev/null 2>&1

# Overwrite metadata.json to declare a jsonl_export that points at a
# non-existent file. Keep `database` honest with an absolute path so
# ambient OBR_CACHE_DIR cannot redirect the partial-drift assertion.
#
# The path must be PHYSICAL: on macOS `mktemp -d` hands back a /var/... path
# whose parent is a symlink into /private/var, and a configured database route
# with a symlinked parent component is refused outright — which fails the whole
# doctor run before the drift assertion is ever reached.
physical_dir="$(pwd -P)"
printf '{\n  "database": "%s/.obr/obr.db",\n  "jsonl_export": "renamed-by-operator.jsonl"\n}\n' \
  "$physical_dir" > .obr/metadata.json

if [ -e .fixture_baseline ]; then
  echo "fixture baseline already exists; expected a fresh workspace" >&2
  exit 1
fi
mkdir -p .fixture_baseline
tar --exclude=.fixture_baseline -cf .fixture_baseline/state.tar .
