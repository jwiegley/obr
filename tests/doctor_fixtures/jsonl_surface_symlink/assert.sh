#!/usr/bin/env bash
# Fixture assertions: jsonl_surface_symlink
#
# obr-4y0. Detect: `jsonl_crlf` must fire on a symlinked surface and say so
# (`details.resolved_via_symlink`), because the bytes live in the target.
# Repair: the TARGET is rewritten and the operator's link survives. Undo:
# the CRLF target is byte-restored.

set -euo pipefail
target_dir="${1:?usage: assert.sh <target_dir> <stage>}"
stage="${2:?usage: assert.sh <target_dir> <stage>}"
tool_bin="${TOOL_BIN:-br}"
cd "$target_dir"

has_crlf() {
    python3 - <<'PY'
with open("store/PLAN.org", "rb") as f:
    print("1" if b"\r\n" in f.read() else "0")
PY
}

require_link() {
    [ -L PLAN.org ] || {
        echo "ASSERT FAIL[$stage]: PLAN.org is no longer a symlink" >&2
        ls -la PLAN.org >&2 || true
        exit 1
    }
}

case "$stage" in
detect)
    require_link
    out=$("$tool_bin" doctor --json 2>/dev/null) || true
    echo "$out" | jq -e '
      .checks[] | select(.name == "jsonl_crlf")
      | select(.status == "warn")
      | select(.details.resolved_via_symlink == true)
      | select(.details.path | endswith("store/PLAN.org"))
      | select(.details.surface_path | endswith("/PLAN.org"))
    ' >/dev/null || {
        echo "ASSERT FAIL[$stage]: jsonl_crlf did not fire against the link target" >&2
        echo "$out" | jq '.checks[] | select(.name == "jsonl_crlf")' >&2
        exit 1
    }
    # The healthy detectors must stay quiet: following the link must not
    # turn every check into a warning.
    echo "$out" | jq -e '
      [.checks[] | select(.name == "jsonl_bom" or .name == "jsonl_size"
                          or .name == "permissions.jsonl_world_writable")
       | select(.status != "ok")] | length == 0
    ' >/dev/null || {
        echo "ASSERT FAIL[$stage]: a clean detector fired on the resolved target" >&2
        echo "$out" | jq '.checks[] | select(.name == "jsonl_bom" or .name == "jsonl_size" or .name == "permissions.jsonl_world_writable")' >&2
        exit 1
    }
    [ "$(has_crlf)" = "1" ] || {
        echo "ASSERT FAIL[$stage]: expected CRLF in the pre-fix target" >&2
        exit 1
    }
    ;;
post_repair)
    require_link
    if [ "$(has_crlf)" != "0" ]; then
        echo "ASSERT FAIL[$stage]: CRLF still present in the link target after repair" >&2
        exit 1
    fi
    now=$(sha256sum store/PLAN.org | awk '{print $1}')
    baseline=$(cat .fixture_target_post_repair_sha256)
    if [ "$now" != "$baseline" ]; then
        echo "ASSERT FAIL[$stage]: post-repair target doesn't match the LF baseline" >&2
        echo "  baseline: $baseline" >&2
        echo "  now:      $now" >&2
        exit 1
    fi
    redetect=$("$tool_bin" doctor --json 2>/dev/null) || true
    status=$(echo "$redetect" | jq -r '.checks[] | select(.name == "jsonl_crlf") | .status' 2>/dev/null || echo "")
    if [ -n "$status" ] && [ "$status" != "ok" ]; then
        echo "ASSERT FAIL[$stage]: jsonl_crlf still '$status' after repair" >&2
        exit 1
    fi
    ;;
post_undo)
    require_link
    if [ "$(has_crlf)" != "1" ]; then
        echo "ASSERT FAIL[$stage]: undo did not restore CRLF in the link target" >&2
        exit 1
    fi
    now=$(sha256sum store/PLAN.org | awk '{print $1}')
    pre=$(cat .fixture_target_pre_sha256)
    if [ "$now" != "$pre" ]; then
        echo "ASSERT FAIL[$stage]: undo didn't byte-restore the CRLF target" >&2
        echo "  pre: $pre" >&2
        echo "  now: $now" >&2
        exit 1
    fi
    ;;
*)
    echo "unknown stage: $stage" >&2
    exit 2
    ;;
esac
