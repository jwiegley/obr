#!/usr/bin/env bash
# Fixture assertions: audit_suspect_close_reasons
set -euo pipefail
target_dir="${1:?usage: assert.sh <target_dir> <stage>}"
stage="${2:?usage: assert.sh <target_dir> <stage>}"
tool_bin="${TOOL_BIN:-br}"
cd "$target_dir"

case "$stage" in
  detect)
    out=$("$tool_bin" doctor --json 2>/dev/null) || true
    # The audit.suspect_close_reasons check should be present AND
    # flagged at warn level (the recently-landed audit policy).
    echo "$out" | jq -e '
      .checks[] | select(.name == "audit.suspect_close_reasons")
      | select(.status == "warn" or .status == "error")
    ' >/dev/null || {
      echo "ASSERT FAIL[$stage]: audit.suspect_close_reasons not flagged" >&2
      echo "$out" | jq '.checks[] | select(.name == "audit.suspect_close_reasons")' >&2
      exit 1
    }
    ;;
  post_repair)
    # Detect-only FM. The bead must still be present, still closed, and
    # MUST NOT carry the audit-historical-cycle-close-YYYY-MM-DD label
    # — the doctor must never silently add it (that would defeat the
    # audit purpose). Use python3 since sqlite3 CLI isn't guaranteed
    # in the harness env.
    py_out=$(python3 <<'PY' 2>&1
import sqlite3

# The two facts are queried separately and on purpose. A missing issue_labels
# table is a legitimate way to satisfy "no label was silently added" — the
# schema simply has nowhere to put one. It must NOT also excuse the bead
# assertions, which is what the single combined try/except used to do: one
# OperationalError printed the has_label=0 success token and swallowed the
# bead query, so a --repair that destroyed the database passed this stage.
try:
    conn = sqlite3.connect(".obr/obr.db")
    cur = conn.cursor()
except sqlite3.Error as e:
    print(f"db_error={e}")
    raise SystemExit(0)

try:
    cur.execute("SELECT status FROM issues WHERE id='br-suspect01'")
    row = cur.fetchone()
except sqlite3.Error as e:
    print(f"db_error={e}")
    raise SystemExit(0)

bead_present = 1 if row else 0
status = row[0] if row else "absent"

try:
    cur.execute("""
        SELECT COUNT(*) FROM issue_labels
        WHERE issue_id = 'br-suspect01'
          AND label LIKE 'audit-historical-cycle-close-%'
    """)
    has_label = cur.fetchone()[0]
except sqlite3.OperationalError:
    # No issue_labels table in this schema: no label can exist.
    has_label = 0

conn.close()
print(f"has_label={has_label} bead_present={bead_present} status={status}")
PY
)
    # The bead must still be present, still closed, and must NOT carry the audit
    # label. All three are asserted; `bead_present` used to be computed, printed,
    # and never checked.
    case "$py_out" in
      *db_error=*)
        echo "ASSERT FAIL[$stage]: database unqueryable after --repair: $py_out" >&2
        exit 1
        ;;
    esac
    echo "$py_out" | grep -q "bead_present=1" || {
      echo "ASSERT FAIL[$stage]: --repair removed the audited bead: $py_out" >&2
      exit 1
    }
    echo "$py_out" | grep -q "status=closed" || {
      echo "ASSERT FAIL[$stage]: --repair reopened the audited bead: $py_out" >&2
      exit 1
    }
    echo "$py_out" | grep -q "has_label=0" || {
      echo "ASSERT FAIL[$stage]: doctor --repair silently added audit-historical-cycle-close label (unsafe): $py_out" >&2
      exit 1
    }
    ;;
  post_undo)
    [ -d .obr ] || { echo "ASSERT FAIL[$stage]: .obr gone after undo" >&2; exit 1; }
    ;;
  *)
    echo "unknown stage: $stage" >&2
    exit 2
    ;;
esac
