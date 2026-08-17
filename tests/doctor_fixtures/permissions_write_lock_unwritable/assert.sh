#!/usr/bin/env bash
# Fixture assertions: permissions_write_lock_unwritable

set -euo pipefail
target_dir="${1:?usage: assert.sh <target_dir> <stage>}"
stage="${2:?usage: assert.sh <target_dir> <stage>}"
tool_bin="${TOOL_BIN:-br}"
cd "$target_dir"

assert_lock_preserved() {
  [ -f .obr/.write.lock ] || {
    echo "ASSERT FAIL[$stage]: .obr/.write.lock vanished" >&2
    exit 1
  }
  if [ -L .obr/.write.lock ]; then
    echo "ASSERT FAIL[$stage]: .obr/.write.lock became a symlink" >&2
    exit 1
  fi
  mode=$(stat -c '%a' .obr/.write.lock)
  if [ "$mode" != "444" ]; then
    echo "ASSERT FAIL[$stage]: .obr/.write.lock mode changed to $mode" >&2
    exit 1
  fi
}

case "$stage" in
  detect)
    # Precondition re-check (beads_rust-ypwu): the fixture plants a regular
    # 0444 `.write.lock` and needs the OS to actually refuse this uid write
    # access to it. Environments where permission bits do not bind — root,
    # CAP_DAC_OVERRIDE container sandboxes (some CI/remote build workers),
    # or filesystems that drop mode bits — cannot hold the precondition, so
    # the detector legitimately never fires there. Exit 3 is the suite's
    # skip protocol; a precondition the environment cannot hold is not a
    # product failure and must not read as one.
    if [ ! -f .obr/.write.lock ] || [ -L .obr/.write.lock ]; then
      echo "SKIP[$stage]: planted .obr/.write.lock is missing or not a regular file before doctor ran; the harness environment did not preserve the fixture state" >&2
      exit 3
    fi
    mode=$(stat -c '%a' .obr/.write.lock)
    if [ "$mode" != "444" ]; then
      echo "SKIP[$stage]: planted .write.lock mode is $mode (expected 444) before doctor ran; the filesystem did not preserve the fixture's permission bits" >&2
      exit 3
    fi
    if [ -w .obr/.write.lock ]; then
      echo "SKIP[$stage]: uid $(id -u) can write a mode-444 file (root or CAP_DAC_OVERRIDE); this environment cannot hold the unwritable-lock precondition" >&2
      exit 3
    fi

    # obr-m6m: this stage used to assert the STARTUP-ABORT envelope
    # (`details.startup_error`, one synthetic check, `inspection_state:
    # not_started`), which existed only because a read-only `obr doctor` took
    # the workspace write lock before inspecting anything — so an unopenable
    # `.write.lock` killed it at startup. That same self-collision blinded the
    # sibling `write_lock` probe, which answered "held by a live process" on
    # every workspace including ones with no holder at all, i.e. exactly the
    # orphan case it exists to detect. Read-only doctor no longer takes the
    # lock, so the abort envelope is unreachable here by design and the
    # condition arrives as an ordinary in-report finding instead.
    #
    # Every SEMANTIC claim the old assertion made is kept: the workspace is not
    # healthy, the detector fires with the planted mode, it carries its FM id,
    # and doctor does not report success. What changed is only the route, so
    # the extra assertion below pins the route: a full inspection ran (many
    # checks) rather than the single-check abort envelope.
    out=$("$tool_bin" doctor --json 2>/dev/null) || true
    echo "$out" | jq -e '
      .ok == false
      and .workspace_health == "degraded"
      and ((.checks | length) > 1)
      and (.checks[] | select(.name == "permissions.write_lock")
        | select(.status == "warn")
        | select(.details.mode_octal == "444")
        | select(.details.finding_id == "fm-state_files-orphaned-write-lock"))
    ' >/dev/null || {
      echo "ASSERT FAIL[$stage]: plain doctor did not emit permissions.write_lock diagnostic" >&2
      echo "$out" | jq '.' >&2
      exit 1
    }
    # The degraded verdict must be carried by the classifier now that no
    # startup abort hand-writes it. Pin the anomaly, or a future refactor
    # could drop the classifier arm and leave a warn on a "healthy" workspace.
    echo "$out" | jq -e '
      .reliability_audit.anomalies[]
      | select(.code == "write_lock_not_writable")
      | select(.severity == "degraded")
    ' >/dev/null || {
      echo "ASSERT FAIL[$stage]: unwritable lock did not produce a degraded write_lock_not_writable anomaly" >&2
      echo "$out" | jq '.reliability_audit' >&2
      exit 1
    }
    assert_lock_preserved
    ;;

  post_repair)
    if [ ! -f "$target_dir/_diag/repair.json" ]; then
      echo "ASSERT FAIL[$stage]: missing _diag/repair.json" >&2
      exit 1
    fi
    jq -e '
      .ok == false
      and .code == "concurrency_lost"
      and .exit_code == 5
      and (.detail | contains("Failed to open workspace write lock"))
    ' "$target_dir/_diag/repair.json" >/dev/null || {
      echo "ASSERT FAIL[$stage]: --repair did not refuse with concurrency_lost" >&2
      cat "$target_dir/_diag/repair.json" >&2
      exit 1
    }
    assert_lock_preserved
    ;;

  post_undo)
    assert_lock_preserved
    ;;

  *)
    echo "unknown stage: $stage" >&2
    exit 2
    ;;
esac
