#!/usr/bin/env bash
# lint-gate.sh — no-NEW-clippy-findings gate.
#
# The tree carries pre-existing first-party clippy errors under
# pedantic+nursery with a current nightly (see ci/baseline/README.md for the
# counts and why the histogram total is not the finding total).
# This gate compares the lint-kind histogram against the committed baseline
# and fails only when a count exceeds it — a tightening gate that is green
# on day one and cannot silently absorb new debt.
#
# IMPORTANT: invokes the `cargo-clippy` binary, not `cargo clippy`. The
# space form resolves through $CARGO_HOME/bin rustup shims ahead of any
# toolchain on PATH and can silently run a stale nightly.
set -uo pipefail

# Sanitize git's hook environment: when run from a pre-commit hook, git
# exports GIT_DIR/GIT_INDEX_FILE/GIT_WORK_TREE, and any test that shells out
# to `git commit` in its own temp directory would resolve them to THE REAL
# REPOSITORY and commit there (observed live: a hook-context unit run left a
# foreign "initial" commit on the branch). Tests must see a clean git env.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_PREFIX GIT_OBJECT_DIRECTORY
unset GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_AUTHOR_DATE
unset GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL GIT_COMMITTER_DATE

repo_root="$(git rev-parse --show-toplevel)"
# Baselines are toolchain- and platform-sensitive (cfg-gated code). Prefer a
# per-OS inventory when present; the default file was generated on Darwin.
# A red first run on a new OS means: verify the toolchain, then commit that
# OS's inventory (see ci/baseline/README.md).
os_baseline="$repo_root/ci/baseline/clippy-lints-baseline-$(uname -s).txt"
baseline="$repo_root/ci/baseline/clippy-lints-baseline.txt"
if [ -f "$os_baseline" ]; then
	baseline="$os_baseline"
fi
log="$(mktemp)"
hist="$(mktemp)"
trap 'rm -f "$log" "$hist"' EXIT

if ! command -v cargo-clippy >/dev/null 2>&1; then
	echo "lint-gate: cargo-clippy not found on PATH (enter the dev shell)" >&2
	exit 2
fi

cargo-clippy --all-targets -- -D warnings >"$log" 2>&1
# shellcheck disable=SC2016  # literal backticks in the regex, not expansion
grep -oE '`-D clippy::[a-z-]+`|`-D warnings`' "$log" |
	sort | uniq -c | sort -rn | awk '{$1=$1; print}' >"$hist"

# Vacuous-pass guard: the baseline records real pre-existing findings, so an
# empty histogram means clippy never linted the workspace (e.g. a stale host
# toolchain dying inside a third-party crate) — not a clean tree.
if [ ! -s "$hist" ] && [ -s "$baseline" ]; then
	echo "lint-gate: clippy produced no findings but the baseline is non-empty;" >&2
	echo "lint-gate: the lint run did not complete (wrong toolchain?). Run inside the dev shell." >&2
	exit 2
fi

# The guard above only catches a run that produced NOTHING. A tree that fails
# to compile can still emit clippy notes from the crates linted before the
# error, leaving a non-empty histogram whose counts are all at or under
# baseline — green from a run that never finished.
#
# Match rustc DIAGNOSTIC CODES (`error[E0308]`) specifically. Clippy findings
# are `error: <description>` with a `-D clippy::…` note and never carry an
# E-code, and under `-D warnings` the run legitimately ends with "could not
# compile … due to N previous errors" while having linted everything — so
# keying on that phrase would refuse every normal run.
if grep -qE '^error\[E[0-9]+\]' "$log"; then
	echo "lint-gate: the tree did not compile; refusing to report green." >&2
	grep -E '^error\[E[0-9]+\]' "$log" | head -20 >&2
	exit 2
fi

status=0
while read -r count lint; do
	[ -z "$lint" ] && continue
	# Lint names contain spaces ("`-D clippy::foo`"), so match the whole
	# remainder-of-line, not an awk field.
	base_count=$(awk -v l="$lint" '{c=$1; sub(/^[0-9]+ /,""); if ($0==l) {print c; exit}}' "$baseline")
	base_count=${base_count:-0}
	if [ "$count" -gt "$base_count" ]; then
		echo "lint-gate: NEW findings for $lint: $count > baseline $base_count" >&2
		status=1
	fi
done <"$hist"

if [ "$status" -ne 0 ]; then
	echo "lint-gate: full clippy output follows" >&2
	grep -E '^(error|warning)' "$log" | head -40 >&2
	exit 1
fi

echo "lint-gate: no new clippy findings vs baseline"
