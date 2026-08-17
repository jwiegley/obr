#!/usr/bin/env bash
# Fixture: jsonl_surface_symlink
# FM: fm-state_files-jsonl-crlf-line-endings (D-SURFACE symlink subset)
#
# obr-4y0: D-SURFACE moved the export to the workspace-root surface, and
# `sync::path::is_workspace_surface_path` explicitly permits that surface to
# be a symlink whose target stays inside the workspace. Six content/mode
# detectors used to short-circuit to `ok` the moment the surface was a
# symlink, so any defect behind the link was invisible.
#
# This fixture keeps the default PLAN.org surface (see .no_jsonl_pin), moves
# the real export into `store/`, and leaves a symlink behind at the surface
# path. The target's line endings are rewritten to CRLF. Doctor's
# `jsonl_crlf` must fire against the TARGET (naming both paths), `--repair`
# must rewrite the target and leave the link intact, and `doctor undo` must
# byte-restore the CRLF target.

set -euo pipefail
target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-br}"

mkdir -p "$target_dir"
cd "$target_dir"
"$tool_bin" init >/dev/null 2>&1

"$tool_bin" create --title "fixture issue 1" --type task --priority 2 >/dev/null 2>&1
"$tool_bin" create --title "fixture issue 2" --type task --priority 2 >/dev/null 2>&1
"$tool_bin" sync --flush-only >/dev/null 2>&1

surface="PLAN.org"
[ -f "$surface" ] || {
    echo "corrupt: expected the default workspace surface at $surface" >&2
    ls -la >&2
    exit 1
}

# Move the real export aside and put a symlink in its place. The target
# stays inside the workspace, which is exactly the shape
# is_workspace_surface_path allows.
mkdir -p store
mv "$surface" store/PLAN.org
ln -s "$target_dir/store/PLAN.org" "$surface"

# Record the LF baseline of the TARGET — post_repair must match it.
sha256sum store/PLAN.org | awk '{print $1}' >.fixture_target_post_repair_sha256

python3 - <<'PY'
path = "store/PLAN.org"
with open(path, "rb") as f:
    data = f.read()
with open(path, "wb") as f:
    f.write(data.replace(b"\n", b"\r\n"))
PY

sha256sum store/PLAN.org | awk '{print $1}' >.fixture_target_pre_sha256

# Sanity: the link must be a link, and the CRLF must be in the target, not
# in the link. A fixture that fails to plant its precondition proves nothing.
[ -L "$surface" ] || {
    echo "corrupt: expected $surface to be a symlink" >&2
    exit 1
}
grep -q $'\r' store/PLAN.org || {
    echo "corrupt: failed to inject CRLF into the link target" >&2
    exit 1
}

if [ -e .fixture_baseline ]; then
    echo "fixture baseline already exists; expected a fresh workspace" >&2
    exit 1
fi
mkdir -p .fixture_baseline
tar --exclude=.fixture_baseline -cf .fixture_baseline/state.tar .
