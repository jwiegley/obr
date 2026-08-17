#!/usr/bin/env bash
# Phase 9: real-world fixture suite driver for `obr doctor`.
#
# Iterates every subdirectory under this script's location, plants the
# failure via that dir's corrupt.sh, runs obr doctor / obr doctor --repair /
# obr doctor undo latest, and checks each stage's assert.sh.
#
# Exit 0 if every fixture passes; non-zero (1) on the first failure with a
# clear diagnostic. Per-fixture isolation is provided by tempdir; the
# source tree is never mutated.
#
# Env:
#   TOOL_BIN  — path to the `obr` binary (default: $CARGO_BIN_EXE_obr, or
#               `cargo run --quiet --bin obr --`)
#   FIXTURES_ROOT — override the fixtures directory
#   SKIP — space-separated fixture names to skip
#   ONLY — space-separated allowlist of fixture names; everything else skipped
#   FAIL_FAST — if "1" (default), exit on first failure; if "0", run all
#   REPLAY_IDEMPOTENCE — if "1", run --repair a second time and require
#                        newly-created replay run actions to be empty
#   REPLAY_IDEMPOTENCE_SKIP — space-separated fixture names to skip for replay
#
# Fixture-local .fixture_env keys:
#   BR_DOCTOR_FIXTURE_PATH_PREPEND — prepend dirs to PATH for env-sensitive checks
#   BR_DOCTOR_FIXTURE_REPAIR_ARGS — extra whitespace-separated args for the
#                                   harness's `obr doctor --repair` invocations

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURES_ROOT="${FIXTURES_ROOT:-$SCRIPT_DIR}"
FAIL_FAST="${FAIL_FAST:-1}"

if [ -z "${TOOL_BIN:-}" ]; then
	if [ -n "${CARGO_BIN_EXE_obr:-}" ]; then
		TOOL_BIN="$CARGO_BIN_EXE_obr"
	elif command -v obr >/dev/null 2>&1; then
		TOOL_BIN="$(command -v obr)"
	else
		echo "run_all.sh: cannot locate \`obr\` binary (set TOOL_BIN or CARGO_BIN_EXE_obr)" >&2
		exit 2
	fi
fi
export TOOL_BIN

if [ ! -x "$TOOL_BIN" ]; then
	echo "run_all.sh: TOOL_BIN=$TOOL_BIN is not executable" >&2
	exit 2
fi

if ! command -v jq >/dev/null 2>&1; then
	echo "run_all.sh: \`jq\` is required (apt-get install jq)" >&2
	exit 2
fi

declare -a fixtures=()
while IFS= read -r dir; do
	fixtures+=("$dir")
done < <(find "$FIXTURES_ROOT" -mindepth 1 -maxdepth 1 -type d | sort)

if [ "${#fixtures[@]}" -eq 0 ]; then
	echo "run_all.sh: no fixtures found under $FIXTURES_ROOT" >&2
	exit 2
fi

total=${#fixtures[@]}
pass=0
fail=0
skipped=0

contains() {
	local needle="$1"
	shift
	for item in "$@"; do [ "$item" = "$needle" ] && return 0; done
	return 1
}

list_run_ids() {
	local runs_dir="$1"
	[ -d "$runs_dir" ] || return 0
	find "$runs_dir" -maxdepth 1 -mindepth 1 -type d -exec basename {} \; | sort
}

# Parse allowlist/blocklist
ONLY_LIST=()
SKIP_LIST=()
if [ -n "${ONLY:-}" ]; then
	read -ra ONLY_LIST <<<"$ONLY"
fi
if [ -n "${SKIP:-}" ]; then
	read -ra SKIP_LIST <<<"$SKIP"
fi

# Returns 0 on pass, 1 on fail, 2 on skip.
run_fixture() {
	local fixture_dir="$1"
	local name
	name="$(basename "$fixture_dir")"

	if [ "${#ONLY_LIST[@]}" -gt 0 ] && ! contains "$name" "${ONLY_LIST[@]}"; then
		return 2
	fi
	if [ "${#SKIP_LIST[@]}" -gt 0 ] && contains "$name" "${SKIP_LIST[@]}"; then
		echo "[SKIP] $name (explicitly listed in SKIP)"
		return 2
	fi

	local corrupt_sh="$fixture_dir/corrupt.sh"
	local assert_sh="$fixture_dir/assert.sh"
	if [ ! -x "$corrupt_sh" ] || [ ! -x "$assert_sh" ]; then
		echo "[FAIL] $name: corrupt.sh or assert.sh missing/non-executable" >&2
		return 1
	fi

	local tmp
	tmp="$(mktemp -d -t br-doctor-fixture-XXXXXX)"
	local diag="$tmp/_diag"
	mkdir -p "$diag"

	# D-SURFACE compatibility for the fixture estate.
	#
	# These fixtures are Class A by nature: their corrupt.sh scripts seed and
	# mangle raw JSONL at `.obr/issues.jsonl`. The surface relocation made the
	# default export `PLAN.org` outside `.obr/`, which invalidated that
	# assumption in 46 scripts at once.
	#
	# Rather than edit 46 scripts, pin the workspace at the one place every
	# fixture passes through: TOOL_BIN. The shim below runs the real binary and,
	# immediately after a successful `init`, rewrites metadata.json to pin the
	# export to `issues.jsonl` and seeds it — the shell equivalent of the
	# `pin_jsonl` helper the Rust Class A tests use. The pin must happen AFTER
	# init because a pre-existing `.obr/` with a JSONL but no database trips the
	# merge-pending gate, and a second `init` refuses on an initialised
	# workspace.
	#
	# Fixtures that exist to test the surface model opt out with a
	# `.no_jsonl_pin` marker and keep the default PLAN.org resolution.
	local fixture_tool_bin="$TOOL_BIN"
	if [ ! -f "$fixture_dir/.no_jsonl_pin" ]; then
		fixture_tool_bin="$tmp/_obr_pinned"
		cat >"$fixture_tool_bin" <<'SHIM'
#!/usr/bin/env bash
"$OBR_FIXTURE_REAL_BIN" "$@"
rc=$?
if [ "$rc" -eq 0 ] && [ "${1:-}" = "init" ]; then
	obr_dir="${OBR_DIR:-.obr}"
	metadata="$obr_dir/metadata.json"
	if [ -f "$metadata" ] && ! grep -q '"jsonl_export": *"issues.jsonl"' "$metadata"; then
		tmp_meta="$metadata.pin.$$"
		sed 's/"jsonl_export": *"[^"]*"/"jsonl_export": "issues.jsonl"/' \
			"$metadata" >"$tmp_meta" && mv "$tmp_meta" "$metadata"
		: >"$obr_dir/issues.jsonl"
		# Drop the seeds the surface model writes, so the workspace is exactly
		# the pre-D-SURFACE shape these fixtures were written against.
		rm -f "$obr_dir/issues.org" "$(dirname "$obr_dir")/PLAN.org"
	fi
fi
exit "$rc"
SHIM
		chmod +x "$fixture_tool_bin"
	fi

	# `obr doctor` walks parent directories to discover `.obr/`. We pin it
	# to the fixture tempdir by:
	#   1. cd-ing into $tmp before invoking obr/scripts
	#   2. exporting OBR_DIR=$tmp/.obr so even nested invocations honor it
	#   3. clearing BD_*, BEADS_*, RUST_LOG-noisy env that the developer's
	#      shell may have set
	# We do NOT use `env -i` because it strips PATH and PWD, and obr needs PATH
	# to discover `git` for fixture-side `git init`.
	local doctor_env=(
		env
		# EVERY --unset MUST precede the first NAME=VALUE. `env` stops
		# parsing options at the first assignment, and a `--unset=` after
		# one is swallowed with no error and no effect — so for as long as
		# these sat below the assignments, none of them did anything and a
		# developer with OBR_DB or XDG_CACHE_HOME set silently contaminated
		# every fixture run. (`-u NAME` in that position is worse: `env`
		# tries to exec `-u`.)
		#
		# Strip developer-shell overrides, both naming generations: the
		# pre-rename spellings still steer resolution via the compat shims.
		--unset=OBR_DB --unset=OBR_DATABASE
		--unset=BD_DB --unset=BD_DATABASE --unset=BEADS_DB
		--unset=OBR_STARTUP_CACHE --unset=BR_STARTUP_CACHE
		# The startup cache resolves XDG_CACHE_HOME BEFORE the HOME
		# fallback, so pinning HOME alone does not scope it: with
		# XDG_CACHE_HOME set, fixtures wrote into the developer's real
		# ~/.cache/obr/startup instead of the workspace.
		--unset=XDG_CACHE_HOME
		HOME="$tmp"
		NO_COLOR=1
		RUST_LOG=error
		TOOL_BIN="$fixture_tool_bin"
		OBR_FIXTURE_REAL_BIN="$TOOL_BIN"
		OBR_NO_AUTO_FLUSH=1
		OBR_DIR="$tmp/.obr"
	)

	# Stage 1: plant the failure.
	if ! (cd "$tmp" && "${doctor_env[@]}" bash "$corrupt_sh" "$tmp") \
		>"$diag/corrupt.stdout" 2>"$diag/corrupt.stderr"; then
		echo "[FAIL] $name: corrupt stage failed" >&2
		sed 's/^/  /' "$diag/corrupt.stderr" >&2
		echo "  (workspace at $tmp)" >&2
		return 1
	fi

	# Fixture-local environment overlays. Keep this deliberately narrow:
	# the runner still invokes the real binary via TOOL_BIN, while a fixture
	# can prepend synthetic executables to PATH for detectors that inspect the
	# operator shell environment. Fixtures can also opt into extra repair args
	# for fixers whose public contract requires an explicit flag, such as
	# `--unsafe-auto-fix`.
	local fixture_repair_args=()
	if [ -f "$tmp/.fixture_env" ]; then
		local fixture_path_prepend=""
		local fixture_env_line=""
		while IFS= read -r fixture_env_line; do
			case "$fixture_env_line" in
			BR_DOCTOR_FIXTURE_PATH_PREPEND=*)
				fixture_path_prepend="${fixture_env_line#BR_DOCTOR_FIXTURE_PATH_PREPEND=}"
				;;
			BR_DOCTOR_FIXTURE_REPAIR_ARGS=*)
				read -ra fixture_repair_args <<<"${fixture_env_line#BR_DOCTOR_FIXTURE_REPAIR_ARGS=}"
				;;
			esac
		done <"$tmp/.fixture_env"
		if [ -n "$fixture_path_prepend" ]; then
			doctor_env+=(PATH="$fixture_path_prepend:${PATH:-}")
		fi
	fi

	# Stage 2: detect-stage assertions. Exit code 3 is the fixture-skip
	# protocol: the environment cannot hold the fixture's planted
	# precondition (e.g. permission bits that do not bind for this uid on
	# this host), which is not a product failure and must not read as one.
	local detect_rc=0
	(cd "$tmp" && "${doctor_env[@]}" bash "$assert_sh" "$tmp" detect) \
		>"$diag/detect.stdout" 2>"$diag/detect.stderr" || detect_rc=$?
	if [ "$detect_rc" -eq 3 ]; then
		echo "[SKIP] $name: environment cannot hold the fixture precondition"
		sed 's/^/  /' "$diag/detect.stderr"
		return 2
	elif [ "$detect_rc" -ne 0 ]; then
		echo "[FAIL] $name: detect stage failed" >&2
		sed 's/^/  /' "$diag/detect.stderr" >&2
		echo "  (workspace at $tmp)" >&2
		return 1
	fi

	# Stage 3: --repair (don't abort on non-zero exit — assert.sh judges).
	local runs_dir="$tmp/.doctor/runs"
	local before_first_repair="$diag/runs_before_first_repair"
	list_run_ids "$runs_dir" >"$before_first_repair"
	local repair_cmd=(doctor --repair)
	if [ "${#fixture_repair_args[@]}" -gt 0 ]; then
		repair_cmd+=("${fixture_repair_args[@]}")
	fi
	repair_cmd+=(--json)
	(cd "$tmp" && "${doctor_env[@]}" "$TOOL_BIN" "${repair_cmd[@]}") \
		>"$diag/repair.json" 2>"$diag/repair.stderr" || true
	# Snapshot the run-ids created by the FIRST --repair. Under
	# REPLAY_IDEMPOTENCE=1 a second --repair below may add a
	# no-op run-dir that becomes the new "latest", so we resolve
	# the undo target explicitly against this pre-replay snapshot.
	local after_first_repair="$diag/runs_after_first_repair"
	list_run_ids "$runs_dir" >"$after_first_repair"
	local first_repair_run_id
	first_repair_run_id="$(comm -13 "$before_first_repair" "$after_first_repair" | tail -n1)"

	# Stage 3.5 (pass-3, opt-in): idempotence replay gate. The
	# chokepoint contract requires that running `--repair` twice
	# in a row is a no-op on the second invocation. A second run
	# that produces any non-empty actions.jsonl line means either
	# the detector is impure (mutates a side-channel) or the fixer
	# isn't idempotent.
	#
	# OPT-IN: REPLAY_IDEMPOTENCE=1 enables the gate. The
	# post-replay `undo` target below is resolved against the
	# FIRST repair's run-id (not `latest`), so fixtures whose
	# post_undo asserts byte-deterministic restore of the first
	# repair are robust against the no-op replay run becoming
	# the new "latest".
	#
	# CI / pass-3 idempotence-audit invocation:
	#   REPLAY_IDEMPOTENCE=1 \
	#   REPLAY_IDEMPOTENCE_SKIP="gitignore_leaking_obr gitignore_bare_pattern" \
	#   bash tests/doctor_fixtures/run_all.sh
	#
	# Per-fixture opt-out (independent of the suite-level gate):
	# drop a `.skip_replay` marker file inside the fixture dir.
	# Historically these were needed because stage-5 used `undo
	# latest`; the explicit-run-id resolution above makes most of
	# them unnecessary, but the opt-out is retained for fixtures
	# whose detectors are genuinely non-idempotent.
	if [ "${REPLAY_IDEMPOTENCE:-0}" = "1" ]; then
		local skip_replay=0
		if [ -n "${REPLAY_IDEMPOTENCE_SKIP:-}" ]; then
			local skip_item
			for skip_item in ${REPLAY_IDEMPOTENCE_SKIP}; do
				if [ "$skip_item" = "$name" ]; then
					skip_replay=1
					break
				fi
			done
		fi
		if [ -f "$fixture_dir/.skip_replay" ]; then
			skip_replay=1
		fi
		if [ "$skip_replay" -eq 0 ]; then
			local before_runs="$diag/replay_runs.before"
			local after_runs="$diag/replay_runs.after"
			list_run_ids "$runs_dir" >"$before_runs"
			(cd "$tmp" && "${doctor_env[@]}" "$TOOL_BIN" "${repair_cmd[@]}") \
				>"$diag/repair_replay.json" 2>"$diag/repair_replay.stderr" || true
			list_run_ids "$runs_dir" >"$after_runs"
			local new_run_ids=()
			mapfile -t new_run_ids < <(comm -13 "$before_runs" "$after_runs")
			local new_run_id
			for new_run_id in "${new_run_ids[@]}"; do
				local newest_run="$runs_dir/$new_run_id"
				if [ -f "$newest_run/actions.jsonl" ]; then
					local replay_action_count
					replay_action_count="$(grep -c -v '^[[:space:]]*$' "$newest_run/actions.jsonl" 2>/dev/null || echo 0)"
					replay_action_count="${replay_action_count//[[:space:]]/}"
					if [ "${replay_action_count:-0}" -gt 0 ]; then
						echo "[FAIL] $name: idempotence replay failed — second --repair produced $replay_action_count action(s)" >&2
						echo "  --- replay actions.jsonl ---" >&2
						sed 's/^/  /' "$newest_run/actions.jsonl" >&2
						echo "  (workspace at $tmp)" >&2
						return 1
					fi
				fi
			done
		fi
	fi

	# Stage 4: post_repair assertions.
	if ! (cd "$tmp" && "${doctor_env[@]}" bash "$assert_sh" "$tmp" post_repair) \
		>"$diag/post_repair.stdout" 2>"$diag/post_repair.stderr"; then
		echo "[FAIL] $name: post_repair stage failed" >&2
		sed 's/^/  /' "$diag/post_repair.stderr" >&2
		echo "  --- repair.json head ---" >&2
		head -c 1024 "$diag/repair.json" >&2 || true
		echo >&2
		echo "  (workspace at $tmp)" >&2
		return 1
	fi

	# Stage 5: undo the FIRST repair (best-effort). We pin to the
	# specific run-id captured before any replay so that a no-op
	# second --repair under REPLAY_IDEMPOTENCE=1 doesn't shadow
	# the meaningful run via the `latest` alias. If for any
	# reason we failed to capture a first-repair run-id (e.g., a
	# detector that emits zero findings), fall back to `latest`.
	local undo_target="${first_repair_run_id:-latest}"
	if [ -z "$undo_target" ]; then
		undo_target="latest"
	fi
	(cd "$tmp" && "${doctor_env[@]}" "$TOOL_BIN" doctor undo "$undo_target" --json) \
		>"$diag/undo.json" 2>"$diag/undo.stderr" || true

	# Stage 6: post_undo assertions.
	if ! (cd "$tmp" && "${doctor_env[@]}" bash "$assert_sh" "$tmp" post_undo) \
		>"$diag/post_undo.stdout" 2>"$diag/post_undo.stderr"; then
		echo "[FAIL] $name: post_undo stage failed" >&2
		sed 's/^/  /' "$diag/post_undo.stderr" >&2
		echo "  --- undo.json ---" >&2
		cat "$diag/undo.json" >&2 || true
		echo "  (workspace at $tmp)" >&2
		return 1
	fi

	echo "[PASS] $name (workspace retained: $tmp)"
	return 0
}

echo "run_all.sh: $total fixture(s) under $FIXTURES_ROOT"
echo "run_all.sh: TOOL_BIN=$TOOL_BIN"
echo

for fixture_dir in "${fixtures[@]}"; do
	rc=0
	run_fixture "$fixture_dir" || rc=$?
	case "$rc" in
	0) pass=$((pass + 1)) ;;
	1)
		fail=$((fail + 1))
		if [ "$FAIL_FAST" = "1" ]; then
			echo
			echo "Summary: pass=$pass fail=$fail skipped=$skipped of $total"
			exit 1
		fi
		;;
	2) skipped=$((skipped + 1)) ;;
	*) fail=$((fail + 1)) ;;
	esac
done

echo
echo "Summary: pass=$pass fail=$fail skipped=$skipped of $total"
[ "$fail" -eq 0 ] || exit 1
