#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
	echo "usage: $0 <br-binary> <baseline-db> <issues-jsonl>" >&2
	exit 64
fi

br_binary=$1
baseline_db=$2
issues_jsonl=$3

if [[ ! -x "$br_binary" ]]; then
	echo "obr binary is not executable: $br_binary" >&2
	exit 65
fi
if [[ ! -f "$baseline_db" ]]; then
	echo "baseline DB is missing: $baseline_db" >&2
	exit 66
fi
if [[ ! -f "$issues_jsonl" ]]; then
	echo "issues JSONL is missing: $issues_jsonl" >&2
	exit 66
fi

tmp_root="${BR_IMPORT_INTEGRITY_TMPDIR:-${TMPDIR:-/tmp}}"
workdir=$(mktemp -d "$tmp_root/br-import-integrity.XXXXXX")
db_path="$workdir/beads.db"
jsonl_path="$workdir/issues.jsonl"
import_stdout="$workdir/import.stdout.json"
import_stderr="$workdir/import.stderr.txt"

cp "$baseline_db" "$db_path"
cp "$issues_jsonl" "$jsonl_path"

RUST_LOG="${RUST_LOG:-warn}" BEADS_JSONL="$jsonl_path" "$br_binary" --db "$db_path" \
	sync --import-only --json --allow-external-jsonl >"$import_stdout" 2>"$import_stderr"

python3 - "$db_path" "$workdir" "$import_stdout" "$import_stderr" <<'PY'
import json
import sqlite3
import sys

db_path, workdir, stdout_path, stderr_path = sys.argv[1:5]
conn = sqlite3.connect(db_path)
cur = conn.cursor()
cur.execute("PRAGMA integrity_check")
integrity = [row[0] for row in cur.fetchall()]
cur.execute("PRAGMA page_count")
page_count = cur.fetchone()[0]
cur.execute("PRAGMA freelist_count")
freelist_count = cur.fetchone()[0]
cur.execute(
    """
    SELECT name, type, rootpage
    FROM sqlite_master
    WHERE name IN (
        'metadata',
        'idx_metadata_key',
        'blocked_issues_cache',
        'sqlite_autoindex_blocked_issues_cache_1',
        'idx_blocked_cache_blocked_at'
    )
    ORDER BY name
    """
)
objects = [
    {"name": name, "type": object_type, "rootpage": rootpage}
    for name, object_type, rootpage in cur.fetchall()
]
report = {
    "workdir": workdir,
    "import_stdout": open(stdout_path, encoding="utf-8").read(),
    "import_stderr": open(stderr_path, encoding="utf-8").read(),
    "integrity_check": integrity,
    "page_count": page_count,
    "freelist_count": freelist_count,
    "objects": objects,
}
print(json.dumps(report, indent=2, sort_keys=True))
if integrity != ["ok"]:
    raise SystemExit(1)
PY
