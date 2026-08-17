#!/usr/bin/env bash
# Fixture assertions: jsonl_conflict_markers
set -euo pipefail
target_dir="${1:?usage: assert.sh <target_dir> <stage>}"
stage="${2:?usage: assert.sh <target_dir> <stage>}"
tool_bin="${TOOL_BIN:-br}"
cd "$target_dir"

case "$stage" in
  detect)
    out=$("$tool_bin" doctor --json 2>/dev/null || true) || true
    echo "$out" | jq -e '
      .checks[] | select(.name == "sync_conflict_markers")
      | select(.status == "error" or .status == "warn")
    ' >/dev/null || {
      echo "ASSERT FAIL[$stage]: sync_conflict_markers not flagged" >&2
      echo "$out" | jq '.checks[] | select(.name == "sync_conflict_markers")' >&2
      exit 1
    }
    grep -q '<<<<<<< ' .obr/issues.jsonl || {
      echo "ASSERT FAIL[$stage]: planted conflict markers missing" >&2
      exit 1
    }
    ;;
  post_repair)
    # Currently auto-fixed: no. The conflict markers must NOT have been
    # auto-removed (that would risk destroying operator data without consent).
    # Either: the JSONL still contains the markers, OR repair refused with
    # exit 4 and left the file alone.
    # Deleting the export outright is the MAXIMAL case of the destruction this
    # fixture forbids, so it must fail here — not be skipped. This guard used to
    # be wrapped in `if [ -f .obr/issues.jsonl ]`, which passed silently in
    # exactly that case.
    [ -f .obr/issues.jsonl ] || {
      echo "ASSERT FAIL[$stage]: --repair deleted the conflicted JSONL outright" >&2
      echo "(safety contract: doctor must refuse to auto-resolve merge conflicts)" >&2
      exit 1
    }
    # If markers are gone, that's unexpected destructive behavior — fail.
    if ! grep -q '<<<<<<< ' .obr/issues.jsonl; then
      echo "ASSERT FAIL[$stage]: conflict markers silently removed by --repair" >&2
      echo "(safety contract: doctor must refuse to auto-resolve merge conflicts)" >&2
      exit 1
    fi
    ;;
  post_undo)
    # No-op: this fixture's repair is expected to refuse, so undo has nothing
    # to restore. Just check the workspace wasn't trashed.
    [ -d .obr ] || { echo "ASSERT FAIL[$stage]: .obr gone after undo" >&2; exit 1; }
    ;;
  *)
    echo "unknown stage: $stage" >&2
    exit 2
    ;;
esac
