#!/usr/bin/env bash
# Fixture assertions: jsonl_malformed_utf8
set -euo pipefail
target_dir="${1:?usage: assert.sh <target_dir> <stage>}"
stage="${2:?usage: assert.sh <target_dir> <stage>}"
tool_bin="${TOOL_BIN:-br}"
cd "$target_dir"

case "$stage" in
  detect)
    out=$("$tool_bin" doctor --json 2>/dev/null) || true
    echo "$out" | jq -e '
      .checks[] | select(.name == "jsonl.parse")
      | select(.status == "error")
      | select(.message // "" | test("valid UTF-8|stream did not contain"; "i"))
    ' >/dev/null || {
      echo "ASSERT FAIL[$stage]: jsonl.parse did not flag malformed UTF-8" >&2
      echo "$out" | jq '.checks[] | select(.name == "jsonl.parse")' >&2
      exit 1
    }
    ;;
  post_repair)
    # Safety gate: the malformed bytes MUST still be present (doctor refuses
    # to silently destroy operator data). The check status may remain error.
    # The planted bytes must survive, and that is asserted unconditionally. This
    # was previously nested inside `if [ "$sz" -le 100 ]`, but the fixture plants
    # 461 bytes — so a rewrite to anything larger than 100 bytes, which is exactly
    # the silent canonical re-export the fixture exists to forbid, skipped the
    # check and passed. Size is not the contract; byte preservation is.
    [ -f .obr/issues.jsonl ] || {
      echo "ASSERT FAIL[$stage]: --repair deleted the malformed JSONL outright" >&2
      exit 1
    }
    if ! python3 -c '
import sys
data = open(".obr/issues.jsonl", "rb").read()
sys.exit(0 if b"\xff\xfe\xfdInvalid bytes follow" in data else 1)
' 2>/dev/null; then
      echo "ASSERT FAIL[$stage]: --repair silently rewrote malformed JSONL (data destruction)" >&2
      echo "(planted invalid-UTF8 sequence is no longer present; size was $(wc -c < .obr/issues.jsonl) bytes)" >&2
      exit 1
    fi
    ;;
  post_undo)
    [ -f .obr/issues.jsonl ] || {
      echo "ASSERT FAIL[$stage]: issues.jsonl gone after undo" >&2
      exit 1
    }
    ;;
  *)
    echo "unknown stage: $stage" >&2
    exit 2
    ;;
esac
