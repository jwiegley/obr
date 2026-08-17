#!/usr/bin/env bash
# agent_smoke_test.sh - Minimal agent-centric smoke tests for obr output surfaces.
#
# Notes:
# - This script intentionally does NOT delete its temp workspace automatically.
# - It verifies JSON and TOON outputs can be parsed/decoded, and checks env precedence.

set -euo pipefail

log() { echo "[agent_smoke $(date +%H:%M:%S)] $*" >&2; }

# Keep output predictable unless the caller explicitly opts into verbose logs.
export RUST_LOG="${RUST_LOG:-error}"

need_cmd() {
	if ! command -v "$1" >/dev/null 2>&1; then
		log "Missing required command: $1"
		exit 1
	fi
}

need_cmd jq
need_cmd tru
need_cmd grep

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ -n "${BR_BIN:-}" ]]; then
	BR="$BR_BIN"
elif [[ -x "$ROOT/target/debug/obr" ]]; then
	BR="$ROOT/target/debug/obr"
elif [[ -x "$ROOT/target/release/obr" ]]; then
	BR="$ROOT/target/release/obr"
elif command -v obr >/dev/null 2>&1; then
	# Fallback for environments where obr is installed but the repo isn't built.
	BR="obr"
else
	log "obr binary not found. Build it with:"
	log "  CARGO_TARGET_DIR=target cargo build"
	exit 1
fi

WORKDIR="$(mktemp -d)"
log "Workspace: $WORKDIR"
log "NOTE: Workspace is left in place (no auto-delete)."

cd "$WORKDIR"

log "Init workspace"
"$BR" init --prefix smoke >/dev/null

log "Create 3 issues"
"$BR" create "Smoke one" --type task --priority 2 --description "Short desc" --json >/dev/null
"$BR" create "Smoke two" --type bug --priority 0 --json >/dev/null
"$BR" create "Smoke three" --type feature --priority 1 --json >/dev/null

list_is_array_or_page='if type=="array" then true else (has("issues") and (.issues | type=="array")) end'
first_issue_id='if type=="array" then .[0].id else .issues[0].id end'

ID1=$("$BR" list --format json --limit 1 | jq -r "$first_issue_id")

log "JSON: list/show parse"
"$BR" list --format json --limit 5 | jq -e "$list_is_array_or_page" >/dev/null
"$BR" show "$ID1" --format json | jq -e 'if type=="array" then (.[0] | has("id") and has("title")) else (has("id") and has("title")) end' >/dev/null

log "Docs: agent quickstarts do not put --format on mutation commands"
if grep -En 'obr (update|close|create|delete|reopen|defer|undefer|label add|label remove)\b.*--format' \
	"$ROOT/docs/agent/QUICKSTART.md" "$ROOT/docs/agent/EXAMPLES.md"; then
	log "Unsupported --format flag found on mutation-command examples"
	exit 1
fi

log "JSON: mutation commands parse via global --json"
"$BR" --json update "$ID1" --status in_progress --claim | jq -e 'if type=="array" then (.[0].status == "in_progress") else (.status == "in_progress") end' >/dev/null
"$BR" --json close "$ID1" --reason "agent smoke test" | jq -e 'if type=="array" then (.[0].status == "closed") else (.status == "closed") end' >/dev/null

log "TOON: list/show decode+parse"
"$BR" list --format toon --limit 5 | tru --decode | jq -e "$list_is_array_or_page" >/dev/null
"$BR" show "$ID1" --format toon | tru --decode | jq -e 'if type=="array" then (.[0] | has("id") and has("title")) else (has("id") and has("title")) end' >/dev/null

log "Env: TOON_DEFAULT_FORMAT defaults output when --format not provided"
TOON_DEFAULT_FORMAT=toon "$BR" list --limit 1 | tru --decode | jq -e "$list_is_array_or_page" >/dev/null

log "Env: BR_OUTPUT_FORMAT takes precedence over TOON_DEFAULT_FORMAT"
BR_OUTPUT_FORMAT=json TOON_DEFAULT_FORMAT=toon "$BR" list --limit 1 | jq -e "$list_is_array_or_page" >/dev/null

log "Error envelope (stderr JSON)"
OUT_JSON="$WORKDIR/out.json"
ERR_JSON="$WORKDIR/err.json"
set +e
"$BR" show bd-NOTEXIST --json >"$OUT_JSON" 2>"$ERR_JSON"
EC=$?
set -e
test "$EC" -eq 3
test ! -s "$OUT_JSON"
jq -e ".error.code == \"ISSUE_NOT_FOUND\"" "$ERR_JSON" >/dev/null
log "Exit code for not-found: $EC"
log "OK"
