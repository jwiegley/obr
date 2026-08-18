#!/usr/bin/env bash
# unit-gate.sh — no-NEW-failures gate for the fast tests a commit must pass.
#
# Two phases, both fail-closed:
#
#   1. the unit suite (`cargo test --lib --bins`). It is not fully green (see
#      ci/baseline/README.md), and under macOS's default symlinked TMPDIR
#      doctor tests abort the harness outright — which is why this script
#      pins a canonical TMPDIR below before recording anything. This phase
#      runs the fast (parallel) tier and fails only on failure NAMES absent
#      from the committed baseline inventory (recorded under the same
#      canonical-TMPDIR contract; see ci/baseline/README.md).
#
#   2. a small, curated INTEGRATION set. Until this phase existed the gate ran
#      `--lib --bins` only, so no integration test guarded a commit at all —
#      which is how tests/golden_rich_panels.rs came to be DEAD on macOS
#      (util-linux `script -c`, which BSD rejects outright) for four releases
#      without anyone noticing, and with it the estate's only coverage of
#      TTY-only output. These three are green, fast, and each guards a class
#      nothing else does; they are expected to stay green, so this phase has
#      no inventory and demands a clean run.
#
# The baselines are only meaningful when recorded in the same environment the
# gate runs in, which is why TMPDIR is pinned below for both phases.
set -uo pipefail

# The integration binaries this gate runs, and why each one is worth the
# seconds it costs:
#
#   vocabulary_lint     — the staleness gate: drives the real binary over a
#                         command matrix in every output mode (including a
#                         real pty) and fails on pre-fork vocabulary.
#   golden_obr_init     — freezes the `.obr/` layout `obr init` writes.
#   golden_rich_panels  — the estate's only pseudo-terminal harness; without
#                         it in a gate, Rich-mode output is unguarded again.
INTEGRATION_TESTS=(vocabulary_lint golden_obr_init golden_rich_panels)

# Sanitize git's hook environment: when run from a pre-commit hook, git
# exports GIT_DIR/GIT_INDEX_FILE/GIT_WORK_TREE, and any test that shells out
# to `git commit` in its own temp directory would resolve them to THE REAL
# REPOSITORY and commit there (observed live: a hook-context unit run left a
# foreign "initial" commit on the branch). Tests must see a clean git env.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_PREFIX GIT_OBJECT_DIRECTORY
unset GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_AUTHOR_DATE
unset GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL GIT_COMMITTER_DATE

repo_root="$(git rev-parse --show-toplevel)"
# Unit failures are platform-dependent (filesystem semantics, permission
# models, and OS-gated tests all differ), so each OS carries its own
# inventory. An inventory recorded elsewhere would absolve the wrong names, so
# an unrecorded platform gets an EMPTY one — meaning the gate demands green
# until someone records a real inventory. See ci/baseline/README.md.
baseline="$repo_root/ci/baseline/unit-failures-baseline-$(uname -s).txt"
if [ ! -f "$baseline" ]; then
	echo "unit-gate: no inventory for $(uname -s); requiring a fully green run." >&2
	echo "unit-gate: see ci/baseline/README.md to record one." >&2
	baseline=/dev/null
fi
# The integration phase has no committed inventory by design: these three are
# green everywhere they run, and a file here would be a place for that to stop
# being true quietly. Recording one is a deliberate act; see
# ci/baseline/README.md.
integration_baseline="$repo_root/ci/baseline/integration-failures-baseline-$(uname -s).txt"
if [ ! -f "$integration_baseline" ]; then
	integration_baseline=/dev/null
fi

log="$(mktemp)"
integration_log="$(mktemp)"
gate_tmpdir=""
trap 'rm -f "$log" "$integration_log"; [ -n "$gate_tmpdir" ] && rm -rf "$gate_tmpdir"' EXIT

# Pin a canonical TMPDIR. macOS defaults TMPDIR to a path under /var, which is
# a symlink to /private/var; tests that canonicalize a workspace path and then
# compare it against the path they were handed fail for a reason that has
# nothing to do with the code. That masked real failures behind an inflated
# baseline once already (see ci/baseline/README.md), so the baseline and every
# run compared against it must share one symlink-free temporary root. Linux
# /tmp is already canonical and is left alone.
if [ "$(uname -s)" = "Darwin" ]; then
	gate_tmpdir="$(mktemp -d /private/tmp/obr-gate.XXXXXX)"
	export TMPDIR="$gate_tmpdir"
fi

# Fail closed: an empty failure set only means "green" if the run actually
# happened. A tree that does not compile emits `error[E....]` lines and zero
# ` ... FAILED` lines, so the failure set is empty and the gate used to print
# success and exit 0 — as a lefthook pre-commit command, that let a
# non-compiling commit through. Require positive evidence of a completed run
# before trusting any comparison.
#
# The exit status alone is not the evidence: the unit suite is expected to
# fail (the baseline records known failures), so a non-zero status is normal.
# What distinguishes "ran and some tests failed" from "never ran" is a `test
# result:` summary line, plus the absence of a compile error or an abnormal
# termination of the harness itself.
#
# $1 phase label, $2 log, $3 cargo exit status, $4 how many `test result:`
# summaries a complete run must produce (one per test binary).
assert_run_completed() {
	local label="$1" logfile="$2" status="$3" expected="$4"
	local completed
	completed=$(grep -c '^test result:' "$logfile")

	if grep -qE '^error(\[|: could not compile)' "$logfile"; then
		echo "unit-gate[$label]: the tree did not compile; refusing to report green." >&2
		grep -E '^error(\[|: could not compile)' "$logfile" | head -20 >&2
		return 2
	fi
	if [ "$status" -ne 0 ] && [ "$completed" -eq 0 ]; then
		echo "unit-gate[$label]: the test run did not complete (cargo exited $status," >&2
		echo "unit-gate[$label]: no 'test result:' summary produced). Refusing to report green." >&2
		grep -E '^(error(\[|:)|warning: build failed)' "$logfile" | head -20 >&2
		return 2
	fi
	# Harness abort (signal/panic outside a test) truncates the run: whatever
	# had not executed yet never prints FAILED, so silence is not evidence
	# about it.
	if grep -qE 'error: test failed, to rerun pass|^error: process didn.t exit successfully' "$logfile" &&
		[ "$completed" -eq 0 ]; then
		echo "unit-gate[$label]: the harness terminated abnormally with no completed run." >&2
		return 2
	fi
	# A named set of binaries must each report. One that never started (build
	# failure for that target, harness abort, a renamed file silently dropping
	# out of the set) would otherwise contribute no FAILED lines and read as
	# green — the same vacuous-pass shape as a non-compiling tree.
	if [ "$completed" -lt "$expected" ]; then
		echo "unit-gate[$label]: only $completed of $expected test binaries reported a" >&2
		echo "unit-gate[$label]: 'test result:' summary. Refusing to report green." >&2
		grep -E '^(error|warning: build failed)' "$logfile" | head -20 >&2
		return 2
	fi
	return 0
}

# LC_ALL=C on both sides: comm requires identical collation, and the committed
# baseline must not depend on the invoking shell's locale.
new_failures_vs() {
	local logfile="$1" inventory="$2"
	grep ' \.\.\. FAILED' "$logfile" | sed 's/ \.\.\. FAILED//' | LC_ALL=C sort -u |
		LC_ALL=C comm -13 <(LC_ALL=C sort -u "$inventory") -
}

# --- phase 1: unit suite ----------------------------------------------------

cargo test --lib --bins >"$log" 2>&1
cargo_status=$?

# `--lib --bins` builds one test binary for the library and one per bin
# target; only the library one has tests, so a complete run reports at least
# one summary.
assert_run_completed unit "$log" "$cargo_status" 1 || exit 2

new_failures=$(new_failures_vs "$log" "$baseline")
if [ -n "$new_failures" ]; then
	echo "unit-gate: NEW unit failures vs baseline:" >&2
	echo "$new_failures" >&2
	exit 1
fi

# --- phase 2: curated integration set ---------------------------------------

integration_args=()
for test_name in "${INTEGRATION_TESTS[@]}"; do
	integration_args+=(--test "$test_name")
done

cargo test "${integration_args[@]}" >"$integration_log" 2>&1
integration_status=$?

assert_run_completed integration "$integration_log" "$integration_status" \
	"${#INTEGRATION_TESTS[@]}" || exit 2

new_integration_failures=$(new_failures_vs "$integration_log" "$integration_baseline")
if [ -n "$new_integration_failures" ]; then
	echo "unit-gate: NEW integration failures vs baseline:" >&2
	echo "$new_integration_failures" >&2
	echo "unit-gate: (set: ${INTEGRATION_TESTS[*]})" >&2
	exit 1
fi

echo "unit-gate: no new unit failures vs baseline"
echo "unit-gate: integration set green (${INTEGRATION_TESTS[*]})"
