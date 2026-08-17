#!/usr/bin/env bash
# scripts/doctor-precommit.sh — doctor pre-commit guard, run via lefthook
# (relocated from upstream's .githooks/pre-commit; see lefthook.yml).
#
# Skip a single commit with either:
#   OBR_DOCTOR_SKIP_PRECOMMIT=1 git commit ...
#   LEFTHOOK=0 git commit ...
#
# What it does:
#   1. If an obr workspace exists in the repo root, runs
#      `obr doctor --quick --json` and inspects the result.
#   2. If `obr` is missing, or the repo root has no workspace, or the
#      environment override is set, the hook exits 0 and lets the
#      commit through.
#   3. If `obr doctor --quick` returns non-zero, the hook BLOCKS the commit
#      with a one-line summary and the recommended next step. ANY finding
#      blocks, including warn-level ones on an otherwise healthy workspace —
#      `workspace_health` is read only to sharpen the message, never to soften
#      the verdict. This comment used to claim the health had to be non-healthy
#      too, which was never what the code did; the block below keys on the exit
#      status alone.
#
#      Practical consequence, since it will bite: a warn is enough to wedge
#      every commit until it is cleared. Bumping the version in Cargo.toml
#      without reinstalling the binary trips `binary_version` and does exactly
#      that. Use the documented bypass for the commit that creates the skew.
#
# The hook is intentionally fail-open on missing tooling so checking
# out a stale branch on a fresh machine doesn't strand the developer.

set -u

# --- 0. Bypasses ---------------------------------------------------------------

if [ "${OBR_DOCTOR_SKIP_PRECOMMIT:-0}" = "1" ]; then
	echo "obr doctor pre-commit: skipped (OBR_DOCTOR_SKIP_PRECOMMIT=1)" >&2
	exit 0
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
if [ -z "$repo_root" ]; then
	exit 0
fi

# Every workspace directory name obr discovers, current and pre-rename. Only
# checking `.beads` here would silently disable the hook on any obr-era repo.
if [ ! -d "$repo_root/.obr" ] &&
	[ ! -d "$repo_root/_obr" ] &&
	[ ! -d "$repo_root/.beads" ] &&
	[ ! -d "$repo_root/_beads" ]; then
	# No workspace in the repo root; nothing for the doctor to inspect.
	exit 0
fi

if ! command -v obr >/dev/null 2>&1; then
	echo "obr doctor pre-commit: obr not on PATH — skipping" >&2
	exit 0
fi

# --- 1. Run the doctor fast path ----------------------------------------------

# `--quick` runs only the cheap detectors and is expected to return in
# well under a second on a healthy workspace.
out="$(cd "$repo_root" && RUST_LOG=error OBR_NO_AUTO_FLUSH=1 obr doctor --quick --json 2>/dev/null)"
status=$?

# --- 2. Decide outcome -------------------------------------------------------

if [ "$status" -eq 0 ]; then
	exit 0
fi

# Try to extract workspace_health for a precise diagnosis. If jq is not
# available or the JSON is malformed, fall back to a generic block.
health=""
if command -v jq >/dev/null 2>&1; then
	health="$(printf '%s' "$out" | jq -r '.workspace_health // empty' 2>/dev/null)"
fi

cat >&2 <<EOF
obr doctor pre-commit: BLOCKED (exit $status${health:+, workspace_health=$health})

Recommended next step:
    obr doctor --repair --dry-run   # preview the plan (chokepoint-audited)
    obr doctor --repair             # apply with backups + actions.jsonl + undo

Bypass (use sparingly):
    OBR_DOCTOR_SKIP_PRECOMMIT=1 git commit ...
EOF
exit 1
