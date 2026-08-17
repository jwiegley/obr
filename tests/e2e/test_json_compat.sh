#!/usr/bin/env bash
# E2E test for JSON compatibility
# Verifies JSON output is valid and consistent

set -euo pipefail

# Setup logging
LOG_DIR="/tmp/br_e2e_logs"
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/json_compat_$(date +%Y%m%d_%H%M%S).log"
exec > >(tee -a "$LOG_FILE") 2>&1

log() { echo "[$(date '+%H:%M:%S')] $*"; }
log_section() {
	echo ""
	log "════════════════════════════════════════"
	log "$*"
}

log_section "JSON COMPATIBILITY TEST"
log "Log file: $LOG_FILE"

# Setup test environment
TESTDIR=$(mktemp -d)
cd "$TESTDIR"
log "Test directory: $TESTDIR"
# NOTE: We intentionally do not delete this directory automatically.
# Agents on this machine must not run destructive filesystem commands (including rm -rf)
# without explicit user approval in-session. Leave the workspace behind for inspection.

# Initialize workspace
obr init --prefix jc
obr create "Test issue one" --type task --priority 1
obr create "Test issue two" --type bug --priority 2
obr create "Test issue three" --type feature --priority 0

# Add a dependency
ID1=$(obr list --json | jq -r '.issues[0].id')
ID2=$(obr list --json | jq -r '.issues[1].id')
obr dep add "$ID2" "$ID1"

log "Created test workspace with 3 issues and 1 dependency"

# Test each JSON-producing command
FAILED=0

test_json_command() {
	local cmd="$1"
	local args="${2:-}"

	log_section "Testing: obr $cmd $args --json"

	# Redirect stderr to separate log to avoid mixing with JSON
	# shellcheck disable=SC2086  # cmd/args are word lists; splitting intended
	OUTPUT=$(obr $cmd $args --json 2>/dev/null || true)

	# Validate JSON syntax
	if echo "$OUTPUT" | jq -e '.' >/dev/null 2>&1; then
		log "✓ $cmd produces valid JSON"
		echo "$OUTPUT" | jq -c '.' | head -3
		return 0
	else
		log "✗ FAIL: $cmd produces invalid JSON"
		log "Output: $OUTPUT"
		FAILED=1
		return 1
	fi
}

# Core commands that produce JSON
test_json_command "list"
test_json_command "ready"
test_json_command "blocked"
test_json_command "stale"
test_json_command "orphans"
test_json_command "stats"
test_json_command "count"
test_json_command "show" "$ID1"

# Test JSON structure for list command
log_section "Validating JSON structure for list"
if obr list --json | jq -e '.issues[0] | has("id", "title", "status", "priority", "issue_type")' >/dev/null; then
	log "✓ List JSON has required fields"
else
	log "✗ FAIL: List JSON missing required fields"
	FAILED=1
fi

# Test JSON structure for stats command
log_section "Validating JSON structure for stats"
if obr stats --json 2>/dev/null | jq -e '.summary | has("total_issues", "open_issues", "closed_issues")' >/dev/null; then
	log "✓ Stats JSON has required fields"
else
	log "✗ FAIL: Stats JSON missing required fields"
	FAILED=1
fi

# Test JSON structure for blocked command
log_section "Validating JSON structure for blocked"
BLOCKED_JSON=$(obr blocked --json)
if echo "$BLOCKED_JSON" | jq -e '. | type == "array"' >/dev/null; then
	log "✓ Blocked JSON is an array"
	if [ "$(echo "$BLOCKED_JSON" | jq 'length')" -gt 0 ]; then
		if echo "$BLOCKED_JSON" | jq -e '.[0] | has("id", "blocked_by", "blocked_by_count")' >/dev/null; then
			log "✓ Blocked JSON entries have required fields"
		else
			log "✗ FAIL: Blocked JSON entries missing required fields"
			FAILED=1
		fi
	fi
else
	log "✗ FAIL: Blocked JSON is not an array"
	FAILED=1
fi

log_section "JSON COMPATIBILITY TEST COMPLETE"

if [ $FAILED -eq 0 ]; then
	log "All tests passed"
	log "NOTE: Test workspace left in place at: $TESTDIR"
	exit 0
else
	log "Some tests failed"
	log "NOTE: Test workspace left in place at: $TESTDIR"
	exit 1
fi
