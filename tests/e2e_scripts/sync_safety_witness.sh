#!/usr/bin/env bash
# tests/e2e_scripts/sync_safety_witness.sh
#
# beads_rust-yyxo: filesystem-witness regression script for obr sync.
#
# Creates a workspace, runs `obr sync --flush-only` and `obr sync --import-only`,
# and asserts every filesystem mutation is in the allowlist defined by
# SYNC_SAFETY_INVARIANTS.md PC-1 / PC-RECOVERY.
#
# Mutations are found by diffing two content-hashed snapshots of the tree, so
# a pre-existing file the sync OVERWROTE is caught alongside the ones it
# created or removed. A path-only diff is blind to the overwrite, which is the
# PC-1 violation this witness exists to catch.
#
# The snapshot diff is the ONLY mechanism. `strace` is invoked and its output
# retained for human debugging, but nothing has ever parsed it; the header used
# to claim strace captured the mutations "with a polling fallback", and no such
# fallback path existed either.
#
# Emits a structured JSON event log to /tmp/sync_safety_witness_<ts>.jsonl
# with one event per filesystem mutation (op is create, modify or delete):
#   {ts, op, path, allowed, reason_if_blocked}
#
# Exit codes:
#   0   all mutations within allowlist (PASS)
#   1   one or more mutations outside allowlist (FAIL — printed details)
#   2   prerequisite missing (obr binary, tmpdir, etc.)
#   3   tracing tool unavailable (strace/inotifywait/dtrace)

set -euo pipefail

LOG_TS=$(date -u +%Y%m%dT%H%M%SZ)
EVENT_LOG="/tmp/sync_safety_witness_${LOG_TS}.jsonl"
PASS_FAIL_LOG="/tmp/sync_safety_witness_${LOG_TS}.summary.txt"

# --- helpers ---------------------------------------------------------------

emit_event() {
	local op="$1"
	local path="$2"
	local allowed="$3"
	local reason="${4:-}"
	printf '{"ts":"%s","op":"%s","path":"%s","allowed":%s,"reason_if_blocked":"%s"}\n' \
		"$(date -u +%Y-%m-%dT%H:%M:%S.%6NZ)" "$op" "$path" "$allowed" "$reason" \
		>>"$EVENT_LOG"
}

# PC-1 / D-SURFACE / PC-RECOVERY allowlist for the filesystem witness.
#
# Fine-grained on purpose: this is NOT "anything under .obr/". The Rust
# witness tests/e2e_sync_git_safety.rs runs in no per-push CI job (only
# .github/workflows/nightly.yml), so this script is the only per-push guard
# on what sync writes -- including PC-RECOVERY's "arbitrary other contents
# in .obr/recovery/ are still rejected".
#
# Keep in step with is_allowed_sync_file in tests/e2e_sync_git_safety.rs,
# which derives its extensions from obr::sync::path::ALLOWED_EXTENSIONS and
# its lock names from obr::sync::path::is_workspace_lock_sidecar_name. Bash
# cannot read those, so this list is checked by review, not by the compiler;
# a drift shows up here as a false VIOLATION.
is_allowed_path() {
	local rel="$1"
	# D-SURFACE: the tracked surface is the one artifact sync writes OUTSIDE
	# `.obr/`, by design -- it is what git is meant to see. Until the snapshots
	# carried content hashes it was exempt by accident (created during setup, so
	# present in both snapshots and never examined); it is now allowed by name
	# and location. Mirrors is_surface_sync_file in tests/e2e_sync_git_safety.rs,
	# which derives these from obr::config::SURFACE_FILENAME and
	# obr::config::SURFACE_SUBDIRS. Spelled out per directory instead of as a
	# `*PLAN.org` glob because bash case globs span '/': one glob would also
	# bless .obr/history/PLAN.<stamp>.org and any other path merely ENDING in
	# the surface name, and the point is that the surface is allowed at exactly
	# the three locations obr resolves it to.
	case "$rel" in
	PLAN.org | doc/PLAN.org | docs/PLAN.org) return 0 ;;
	PLAN.org.tmp | doc/PLAN.org.tmp | docs/PLAN.org.tmp) return 0 ;;
	esac
	case "$rel" in
	PLAN.org.*.tmp | doc/PLAN.org.*.tmp | docs/PLAN.org.*.tmp)
		# The globs above span '/' like every bash case glob, so this arm alone
		# would also bless PLAN.org.x/y.org.123.tmp — a NEW allowance outside
		# .obr/, which is where this witness must be strictest. Re-check the
		# basename so the directory part stays exactly the three the arms name.
		local surface_base="${rel##*/}"
		case "$surface_base" in
		PLAN.org.*.tmp)
			local surface_pid="${surface_base##*.org.}"
			surface_pid="${surface_pid%.tmp}"
			[[ "$surface_pid" =~ ^[0-9]+$ ]] && return 0
			;;
		esac
		;;
	esac
	case "$rel" in
	.obr/.manifest.json | .obr/metadata.json | .obr/last-touched) return 0 ;;
	.obr/*.jsonl | .obr/*.jsonl.tmp | .obr/*.db | .obr/*.db-wal | .obr/*.db-shm | .obr/*.db-journal) return 0 ;;
	.obr/*.db-fsqlite-ns-gate | .obr/*.db-fsqlite-ns-use) return 0 ;;
	# D-SURFACE: the history payload follows the tracked surface. It was
	# .obr/history/issues.<stamp>.jsonl, which the *.jsonl arm above matched
	# because bash case globs span '/'; the surface is PLAN.org now, so it
	# needs naming. Its .meta.json sidecar was always listed separately.
	.obr/*.org | .obr/*.org.tmp) return 0 ;;
	.obr/history/*.meta.json) return 0 ;;
	.obr/recovery/*.bak | .obr/recovery/*.rebuild-failed | .obr/recovery/*.truncated-wal) return 0 ;;
	# Advisory write-lock sidecars. Zero-byte files whose inode IS the lock,
	# so they are deliberately never unlinked: the advisory lock lives on the
	# open file description, and removing one would let a second process
	# create a fresh inode at the same path and hold a second "exclusive"
	# authority. The hex is a digest of the canonical resource path, not a
	# nonce, so the set is bounded by distinct resources, not by operations.
	# Mirrors obr::sync::path::is_workspace_lock_sidecar_name.
	.obr/.write.lock | .obr/.sync.lock) return 0 ;;
	esac
	case "$rel" in
	.obr/.br-db-write-*.lock | .obr/.br-jsonl-write-*.lock)
		local digest="${rel##*-write-}"
		digest="${digest%.lock}"
		[[ "$digest" =~ ^[0-9a-f]{24}$ ]] && return 0
		;;
	esac
	case "$rel" in
	.obr/*.jsonl.*.tmp)
		local pid="${rel##*.jsonl.}"
		pid="${pid%.tmp}"
		[[ "$pid" =~ ^[0-9]+$ ]] && return 0
		;;
	.obr/*.org.*.tmp)
		local pid="${rel##*.org.}"
		pid="${pid%.tmp}"
		[[ "$pid" =~ ^[0-9]+$ ]] && return 0
		;;
	esac
	return 1
}

log_summary() {
	echo "[$(date -u +%H:%M:%S)] $*" | tee -a "$PASS_FAIL_LOG"
}

# sha256 of one file's contents. SHA_CMD is chosen in preflight below; the
# content is fed on stdin so the digest is the only thing on the output line.
hash_file() {
	"${SHA_CMD[@]}" <"$1" | cut -d' ' -f1
}

# Snapshot the tree as sorted "<path><TAB><sha256>" lines. Hashing every file
# is what lets Phase 5 see a MODIFIED pre-existing file: a bare `find` listing
# can only ever answer "did this path appear or vanish", which is why sync
# overwriting a pre-existing file outside the allowlist -- the PC-1 violation
# -- went unexamined until now.
#
# A file that cannot be read hashes to the literal `<unhashable>` on both
# sides, so it compares equal and an unreadable file that sync overwrote emits
# no modify event. That blind spot is deliberate: the fallback is what stops
# `set -e` from killing the run on a transient read, and obr writes no
# unreadable files. Naming it here so it is a known limit rather than a
# surprise.
snapshot_tree() {
	local out="$1"
	find . -type f -not -path "./logs/*" 2>/dev/null | sort | while IFS= read -r path; do
		printf '%s\t%s\n' "$path" "$(hash_file "$path" 2>/dev/null || echo '<unhashable>')"
	done >"$out"
}

# Path column of a snapshot, for the created/deleted `comm` diffs.
snapshot_paths() {
	cut -f1 "$1" | sort
}

# --- preflight -------------------------------------------------------------

log_summary "=== sync_safety_witness.sh START ts=${LOG_TS} ==="
log_summary "  event log: ${EVENT_LOG}"
log_summary "  summary log: ${PASS_FAIL_LOG}"

# Pick a sha256 tool: sha256sum on GNU/Linux (the CI runner), shasum on macOS.
# Same pair, same order as scripts/build-release.sh. Without one there is no
# content comparison and therefore no PC-1 verdict at all, so refuse to run
# rather than report a PASS that examined nothing.
if command -v sha256sum >/dev/null 2>&1; then
	SHA_CMD=(sha256sum)
elif command -v shasum >/dev/null 2>&1; then
	SHA_CMD=(shasum -a 256)
else
	log_summary "ERROR: no sha256 tool found (need sha256sum or shasum)"
	exit 2
fi
log_summary "  hasher: ${SHA_CMD[*]}"

# Locate obr binary
if [[ -n "${OBR_BIN:-${BR_BIN:-}}" ]]; then
	BR_BINARY="${OBR_BIN:-$BR_BIN}"
elif [[ -n "${CARGO_BIN_EXE_obr:-}" ]]; then
	BR_BINARY="$CARGO_BIN_EXE_obr"
elif command -v obr >/dev/null 2>&1; then
	BR_BINARY=$(command -v obr)
else
	# Try project-local release build
	BR_BINARY="$(pwd)/target/release/obr"
	if [[ ! -x "$BR_BINARY" ]]; then
		# Maybe CARGO_TARGET_DIR
		if [[ -n "${CARGO_TARGET_DIR:-}" && -x "$CARGO_TARGET_DIR/release/obr" ]]; then
			BR_BINARY="$CARGO_TARGET_DIR/release/obr"
		else
			log_summary "ERROR: obr binary not found (set BR_BIN= or build with cargo build --release)"
			exit 2
		fi
	fi
fi
log_summary "  obr binary: ${BR_BINARY}"

WORKSPACE=$(mktemp -d)
cd "$WORKSPACE"
log_summary "  workspace: ${WORKSPACE}"

# Initialize event log
: >"$EVENT_LOG"
emit_event "harness_start" "$WORKSPACE" "true" ""

# --- choose tracing strategy ----------------------------------------------

TRACER=""
if command -v strace >/dev/null 2>&1; then
	TRACER="strace"
elif command -v inotifywait >/dev/null 2>&1; then
	TRACER="inotifywait"
elif command -v dtrace >/dev/null 2>&1; then
	TRACER="dtrace"
else
	log_summary "WARNING: no kernel tracing tool found; falling back to polling stat snapshots (less precise)"
	TRACER="polling"
fi
log_summary "  tracer: ${TRACER}"

# --- exercise sync ---------------------------------------------------------

log_summary "Phase 1: obr init"
"$BR_BINARY" init >/dev/null 2>&1 || {
	log_summary "ERROR: obr init failed"
	exit 2
}

log_summary "Phase 2: obr create x3 (no-auto-flush)"
"$BR_BINARY" create "Witness test 1" -t task --no-auto-flush -q >/dev/null
"$BR_BINARY" create "Witness test 2" -t bug --no-auto-flush -q >/dev/null
"$BR_BINARY" create "Witness test 3" -t feature --no-auto-flush -q >/dev/null

# Snapshot fs state before sync (path + content hash)
SNAPSHOT_BEFORE=$(mktemp)
snapshot_tree "$SNAPSHOT_BEFORE"
log_summary "  files before sync: $(wc -l <"$SNAPSHOT_BEFORE") (snapshot: $SNAPSHOT_BEFORE)"

log_summary "Phase 3: obr sync --flush-only (export)"
if [[ "$TRACER" == "strace" ]]; then
	STRACE_LOG=$(mktemp)
	strace -f -e trace=open,openat,creat,unlink,rename,renameat,renameat2 \
		-o "$STRACE_LOG" \
		"$BR_BINARY" sync --flush-only 2>/dev/null
	EXIT=$?
	log_summary "  strace log: $STRACE_LOG (exit: $EXIT)"
else
	"$BR_BINARY" sync --flush-only 2>/dev/null
	EXIT=$?
	log_summary "  sync exit: $EXIT (no strace; using fs-snapshot diff)"
fi

log_summary "Phase 4: obr sync --import-only --force (import + maybe-rebuild)"
"$BR_BINARY" sync --import-only --force 2>/dev/null || true

# Snapshot fs state after
SNAPSHOT_AFTER=$(mktemp)
snapshot_tree "$SNAPSHOT_AFTER"
log_summary "  files after sync: $(wc -l <"$SNAPSHOT_AFTER")"

# --- assert allowlist ------------------------------------------------------

log_summary "Phase 5: assert each created/modified/deleted file is in PC-1/PC-RECOVERY allowlist"

# Note: pipe-fed `while` loops below run in a subshell under bash, so any
# in-loop counter increments would NOT persist to the outer shell. We rely
# on the event-log file (appended via `emit_event`) as the source of truth
# and count lines in that file after all three loops complete.

# Diff: new files in after
comm -13 <(snapshot_paths "$SNAPSHOT_BEFORE") <(snapshot_paths "$SNAPSHOT_AFTER") | while IFS= read -r path; do
	rel="${path#./}"
	if is_allowed_path "$rel"; then
		emit_event "create" "$rel" "true" ""
	else
		emit_event "create" "$rel" "false" "not in allowlist"
		log_summary "  VIOLATION: created '$rel' (not in PC-1/PC-RECOVERY allowlist)"
	fi
done

# Files present in both snapshots whose CONTENT the sync changed. This is the
# PC-1 case proper -- sync overwriting something that was already there -- and
# it is invisible to a path-only diff. Same allowlist as create/delete, so the
# tracked surface is admitted by the explicit D-SURFACE arm in is_allowed_path
# and nothing else outside `.obr/` is.
#
# `FILENAME == ARGV[1]`, not the usual `NR == FNR`: with an empty before-file
# the NR idiom makes the whole modify pass a silent no-op rather than an error,
# so a future edit that moved the snapshot earlier would disable this check
# without any signal. Phase 1 guarantees a non-empty snapshot today; this keeps
# it from degrading quietly if that stops being true.
awk -F'\t' 'FILENAME == ARGV[1] { before[$1] = $2; next }
	($1 in before) && before[$1] != $2 { print $1 }' \
	"$SNAPSHOT_BEFORE" "$SNAPSHOT_AFTER" | while IFS= read -r path; do
	rel="${path#./}"
	if is_allowed_path "$rel"; then
		emit_event "modify" "$rel" "true" ""
	else
		emit_event "modify" "$rel" "false" "not in allowlist"
		log_summary "  VIOLATION: modified '$rel' (not in PC-1/PC-RECOVERY allowlist)"
	fi
done

# Removed files (rare, but possible during rebuild)
comm -23 <(snapshot_paths "$SNAPSHOT_BEFORE") <(snapshot_paths "$SNAPSHOT_AFTER") | while IFS= read -r path; do
	rel="${path#./}"
	if is_allowed_path "$rel"; then
		emit_event "delete" "$rel" "true" ""
	else
		emit_event "delete" "$rel" "false" "not in allowlist"
		log_summary "  VIOLATION: deleted '$rel' (not in PC-1/PC-RECOVERY allowlist)"
	fi
done

# Cleanup snapshots
rm -f "$SNAPSHOT_BEFORE" "$SNAPSHOT_AFTER"

# --- summary ---------------------------------------------------------------

emit_event "harness_end" "$WORKSPACE" "true" ""

# Count from the event log (the source of truth across subshell boundaries).
# Use awk so a zero-match doesn't trigger set -e under -o pipefail.
ACTUAL_VIOLATIONS=$(awk '/"allowed":false/ { c++ } END { print c+0 }' "$EVENT_LOG")
ACTUAL_TOTAL=$(awk '/"op":"(create|modify|delete)"/ { c++ } END { print c+0 }' "$EVENT_LOG")

log_summary "=== sync_safety_witness.sh SUMMARY ==="
log_summary "  total operations: ${ACTUAL_TOTAL}"
log_summary "  violations: ${ACTUAL_VIOLATIONS}"
log_summary "  event log: ${EVENT_LOG}"

if [[ "$ACTUAL_VIOLATIONS" -gt 0 ]]; then
	log_summary "FAIL"
	rm -rf "$WORKSPACE"
	exit 1
else
	log_summary "PASS"
	rm -rf "$WORKSPACE"
	exit 0
fi
