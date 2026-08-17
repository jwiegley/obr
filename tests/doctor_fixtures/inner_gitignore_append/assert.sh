#!/usr/bin/env bash
# Fixture assertions: inner_gitignore_append
#
# Pass-5 cycle 27: fm-configs-gitignore-leaking-obr (inner subset)
# graduates from detect-only (Tier B from cycle 13) to auto-fixed
# (Tier A). --repair appends the missing self-ignore line via
# Op::AppendFile while preserving every operator-written line.

set -euo pipefail
target_dir="${1:?usage: assert.sh <target_dir> <stage>}"
stage="${2:?usage: assert.sh <target_dir> <stage>}"
tool_bin="${TOOL_BIN:-br}"
cd "$target_dir"

has_pattern() {
  # $1 is the canonical pattern to look for after removing only line endings
  # and trailing whitespace. Leading whitespace is literal in gitignore.
  python3 - "$1" <<'PY'
import sys
needle = sys.argv[1]

def normalize_gitignore_line(line):
    line = line.rstrip("\r\n")
    while line.endswith(" "):
        prefix = line[:-1]
        escaping_backslashes = len(prefix) - len(prefix.rstrip("\\"))
        if escaping_backslashes % 2 == 1:
            break
        line = prefix
    return line

try:
    with open(".obr/.gitignore", "r", encoding="utf-8") as f:
        present = any(normalize_gitignore_line(line) == needle for line in f)
except FileNotFoundError:
    present = False
print("1" if present else "0")
PY
}

case "$stage" in
  detect)
    out=$("$tool_bin" doctor --json 2>/dev/null) || true
    echo "$out" | jq -e '
      .checks[] | select(.name == "gitignore.obr_inner_present")
      | select(.status == "warn" or .status == "error")
    ' >/dev/null || {
      echo "ASSERT FAIL[$stage]: gitignore.obr_inner_present not flagged" >&2
      echo "$out" | jq '.checks[] | select(.name == "gitignore.obr_inner_present")' >&2
      exit 1
    }
    # The pre-fix state enumerates artifacts but never ignores the directory
    # wholesale, which is exactly what the new model requires.
    if [ "$(has_pattern '*.lock')" != "1" ]; then
      echo "ASSERT FAIL[$stage]: expected *.lock in pre-fix state" >&2
      exit 1
    fi
    if [ "$(has_pattern '*')" != "0" ]; then
      echo "ASSERT FAIL[$stage]: expected the self-ignore line to be missing pre-fix" >&2
      exit 1
    fi
    # Operator-custom line preserved through the corrupt stage.
    if [ "$(has_pattern 'local-cache/')" != "1" ]; then
      echo "ASSERT FAIL[$stage]: operator-custom line not preserved during corrupt" >&2
      exit 1
    fi
    ;;
  post_repair)
    # Repair adds the wholesale ignore; the operator's narrower rules survive.
    if [ "$(has_pattern '*.lock')" != "1" ]; then
      echo "ASSERT FAIL[$stage]: *.lock missing after repair" >&2
      exit 1
    fi
    if [ "$(has_pattern '*')" != "1" ]; then
      echo "ASSERT FAIL[$stage]: self-ignore line missing after repair" >&2
      exit 1
    fi
    # Operator-custom line MUST be preserved verbatim.
    if [ "$(has_pattern 'local-cache/')" != "1" ]; then
      echo "ASSERT FAIL[$stage]: operator-custom line lost during repair (CRITICAL)" >&2
      exit 1
    fi
    if [ -f "$target_dir/_diag/repair.json" ]; then
      jq -e '
        .repaired == true
        and (.recovery_audit.applied_actions | index("inner_gitignore_appended"))
      ' "$target_dir/_diag/repair.json" >/dev/null || {
        echo "ASSERT FAIL[$stage]: repair output did not report inner_gitignore_appended" >&2
        cat "$target_dir/_diag/repair.json" >&2
        exit 1
      }
    fi
    # SACRED INVARIANT: the export stays byte-identical — touching the
    # gitignore must NEVER affect the tracked surface.
    jsonl_now=$(sha256sum PLAN.org | awk '{print $1}')
    jsonl_pre=$(cat .fixture_jsonl_pre_sha256)
    if [ "$jsonl_now" != "$jsonl_pre" ]; then
      echo "ASSERT FAIL[$stage]: JSONL bytes mutated by inner-gitignore repair" >&2
      exit 1
    fi
    redetect=$("$tool_bin" doctor --json 2>/dev/null) || true
    status=$(echo "$redetect" | jq -r '.checks[] | select(.name == "gitignore.obr_inner_present") | .status' 2>/dev/null || echo "")
    if [ "$status" != "ok" ] && [ -n "$status" ]; then
      echo "ASSERT FAIL[$stage]: gitignore.obr_inner_present still '$status' after repair" >&2
      exit 1
    fi
    runs_root="$target_dir/.doctor/runs"
    append_count=0
    while IFS= read -r d; do
      a="$d/actions.jsonl"
      [ -s "$a" ] || continue
      c=$(grep -c '"op":"append_file"' "$a" 2>/dev/null || echo 0)
      c="${c//[[:space:]]/}"
      append_count=$((append_count + ${c:-0}))
    done < <(find "$runs_root" -maxdepth 1 -mindepth 1 -type d 2>/dev/null)
    if [ "${append_count:-0}" -lt 1 ]; then
      echo "ASSERT FAIL[$stage]: expected >=1 append_file action, got $append_count" >&2
      find "$runs_root" -name actions.jsonl -exec cat {} \; >&2 2>/dev/null || true
      exit 1
    fi
    ;;
  post_undo)
    # The incomplete state restores byte-deterministically.
    gi_now=$(sha256sum .obr/.gitignore | awk '{print $1}')
    gi_pre=$(cat .fixture_inner_gitignore_pre_sha256)
    if [ "$gi_now" != "$gi_pre" ]; then
      echo "ASSERT FAIL[$stage]: undo didn't byte-restore the incomplete .obr/.gitignore" >&2
      echo "  pre: $gi_pre" >&2
      echo "  now: $gi_now" >&2
      diff -u .fixture_inner_gitignore_pre.txt .obr/.gitignore >&2 || true
      exit 1
    fi
    jsonl_now=$(sha256sum PLAN.org | awk '{print $1}')
    jsonl_pre=$(cat .fixture_jsonl_pre_sha256)
    if [ "$jsonl_now" != "$jsonl_pre" ]; then
      echo "ASSERT FAIL[$stage]: surface bytes drifted across undo" >&2
      exit 1
    fi
    ;;
  *)
    echo "unknown stage: $stage" >&2
    exit 2
    ;;
esac
