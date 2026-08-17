#!/usr/bin/env bash
# Fixture assertions: orphaned_write_lock
set -euo pipefail
target_dir="${1:?usage: assert.sh <target_dir> <stage>}"
stage="${2:?usage: assert.sh <target_dir> <stage>}"
tool_bin="${TOOL_BIN:-br}"
cd "$target_dir"

# Restored from 9bae34ac, which added the three call sites below but whose
# definition was lost in merge 77ae88ff; without it every post-repair stage
# dies with "assert_lock_identity_preserved: command not found".
# GNU `stat -c` / BSD `stat -f` name the same st_dev:st_ino pair.
assert_lock_identity_preserved() {
  [ -f .fixture_lock_identity ] || {
    echo "ASSERT FAIL[$stage]: missing baseline lock identity" >&2
    exit 1
  }
  expected_identity=$(cat .fixture_lock_identity)
  actual_identity=$(stat -c '%d:%i' .obr/.write.lock 2>/dev/null || stat -f '%d:%i' .obr/.write.lock)
  if [ "$actual_identity" != "$expected_identity" ]; then
    echo "ASSERT FAIL[$stage]: lock identity changed $expected_identity -> $actual_identity" >&2
    exit 1
  fi
}

# Force the stale-mtime branch by overriding the staleness threshold to 0.
# Any non-future mtime is then "older than threshold" — which, since
# GitHub #395, only selects the file for a non-blocking flock PROBE: the
# planted lock is free, so the probe acquires it and the check must
# classify Ok. Lock acquisition never updates mtime, so file age alone
# is not evidence of an orphan.
export OBR_DOCTOR_STALE_LOCK_THRESHOLD_SECS=0

case "$stage" in
  detect)
    out=$("$tool_bin" doctor --json 2>/dev/null) || true
    # A free lock — however old the file — must be Ok via the probe.
    echo "$out" | jq -e '
      .checks[] | select(.name == "write_lock")
      | select(.status == "ok")
    ' >/dev/null || {
      echo "ASSERT FAIL[$stage]: free stale-mtime lock must be ok (GH #395)" >&2
      echo "$out" | jq '.checks[] | select(.name == "write_lock")' >&2
      exit 1
    }
    # The classification must come from the probe FINDING THE LOCK FREE.
    #
    # obr-m6m: this used to accept `probe_would_block_live_holder` as well,
    # and that is the branch it actually took — plain `obr doctor` held the
    # workspace write lock for its whole run, so the probe collided with its
    # own caller and answered "live holder" on a workspace whose lock nobody
    # held. The fixture was green while exercising the opposite path from the
    # one its README describes, and it would have stayed green with the probe
    # deleted entirely. Read-only doctor no longer takes the lock, so the free
    # branch is reachable and is the only correct answer for a planted,
    # unheld, ancient lock file.
    echo "$out" | jq -e '
      .checks[] | select(.name == "write_lock")
      | (.details.reason == "probe_acquired_free")
    ' >/dev/null || {
      echo "ASSERT FAIL[$stage]: details.reason != probe_acquired_free" >&2
      echo "$out" | jq '.checks[] | select(.name == "write_lock") | .details' >&2
      exit 1
    }
    # Pin the declared FM id to the check (coverage manifest contract).
    # The warn path (stale_unprobed) needs a lock file that exists but
    # cannot be opened at all, which no fixture can plant portably, so the
    # probe fixture is where the
    # `fm-concurrency_primitives-orphaned-write-lock` id is pinned.
    echo "$out" | jq -e '
      .checks[] | select(.name == "write_lock")
      | select(.details.finding_id == "fm-concurrency_primitives-orphaned-write-lock")
    ' >/dev/null || {
      echo "ASSERT FAIL[$stage]: write_lock finding_id drifted from fm-concurrency_primitives-orphaned-write-lock" >&2
      echo "$out" | jq '.checks[] | select(.name == "write_lock") | .details' >&2
      exit 1
    }
    # The old move-aside advice was the inode-split hazard; it must be gone.
    echo "$out" | jq -e '
      .checks[] | select(.name == "write_lock")
      | ((.details.recommended_fix // "") | test("\\.stale-") | not)
    ' >/dev/null || {
      echo "ASSERT FAIL[$stage]: move-aside advice must not be suggested" >&2
      exit 1
    }
    ;;
  post_repair)
    # The inode is not a finding and must remain untouched.
    [ -f .obr/.write.lock ] || {
      echo "ASSERT FAIL[$stage]: .write.lock vanished after --repair (unsafe; could corrupt a live writer)" >&2
      exit 1
    }
    if [ -L .obr/.write.lock ]; then
      echo "ASSERT FAIL[$stage]: .write.lock became a symlink after --repair (unsafe)" >&2
      exit 1
    fi
    assert_lock_identity_preserved
    ;;
  post_undo)
    [ -d .obr ] || { echo "ASSERT FAIL[$stage]: .obr gone after undo" >&2; exit 1; }
    [ -f .obr/.write.lock ] || { echo "ASSERT FAIL[$stage]: .write.lock gone after undo" >&2; exit 1; }
    assert_lock_identity_preserved
    ;;
  *)
    echo "unknown stage: $stage" >&2
    exit 2
    ;;
esac
