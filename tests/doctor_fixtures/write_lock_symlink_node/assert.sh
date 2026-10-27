#!/usr/bin/env bash
# Fixture assertions: write_lock_symlink_node
set -euo pipefail
target_dir="${1:?usage: assert.sh <target_dir> <stage>}"
stage="${2:?usage: assert.sh <target_dir> <stage>}"
tool_bin="${TOOL_BIN:-obr}"
cd "$target_dir"

assert_symlink_and_target_untouched() {
  [ -L .obr/.write.lock ] || {
    echo "ASSERT FAIL[$stage]: symlinked .write.lock was removed or replaced (doctor must never touch the node)" >&2
    exit 1
  }
  [ -f .obr/.lock_target ] || {
    echo "ASSERT FAIL[$stage]: symlink target vanished (target traversal / mutation is forbidden)" >&2
    exit 1
  }
  [ "$(readlink .obr/.write.lock)" = ".lock_target" ] || {
    echo "ASSERT FAIL[$stage]: symlink target changed" >&2
    exit 1
  }
  [ ! -s .obr/.lock_target ] || {
    echo "ASSERT FAIL[$stage]: symlink target bytes changed" >&2
    exit 1
  }
  expected_identity=$(cat .fixture_target_identity)
  actual_identity=$(stat -c '%d:%i' .obr/.lock_target 2>/dev/null \
    || stat -f '%d:%i' .obr/.lock_target)
  if [ "$actual_identity" != "$expected_identity" ]; then
    echo "ASSERT FAIL[$stage]: symlink target inode changed $expected_identity -> $actual_identity" >&2
    exit 1
  fi
}

case "$stage" in
  detect)
    assert_symlink_and_target_untouched
    set +e
    out=$("$tool_bin" doctor --json 2>&1)
    doctor_rc=$?
    set -e
    # Read-only doctor reports the unsafe node; mutators must refuse admission.
    if [ "$doctor_rc" -eq 0 ]; then
      echo "ASSERT FAIL[$stage]: symlinked .write.lock did not fail doctor (beads_rust-5sej)" >&2
      echo "$out" >&2
      exit 1
    fi
    echo "$out" | jq -e '
      .ok == false and any(.checks[];
        .name == "write_lock" and .status == "error"
        and .details.reason == "non_regular_lock_node"
        and .details.node_kind == "symlink")
    ' >/dev/null || {
      echo "ASSERT FAIL[$stage]: typed write-lock diagnostic missing" >&2
      echo "$out" >&2
      exit 1
    }
    before_db=$(sha256sum .obr/obr.db | awk '{print $1}')
    set +e
    mutation=$("$tool_bin" create "Unsafe lock must refuse admission" --json 2>&1)
    mutation_rc=$?
    set -e
    if [ "$mutation_rc" -eq 0 ]; then
      echo "ASSERT FAIL[$stage]: mutator admitted an unsafe write lock" >&2
      exit 1
    fi
    echo "$mutation" | grep -q "Refusing unsafe workspace write lock path" || {
      echo "ASSERT FAIL[$stage]: mutator startup refusal missing: $mutation" >&2
      exit 1
    }
    echo "$mutation" | grep -q "expected a regular file, not a symlink or special file" || {
      echo "ASSERT FAIL[$stage]: mutator refusal did not identify unsafe shape" >&2
      exit 1
    }
    after_db=$(sha256sum .obr/obr.db | awk '{print $1}')
    [ "$before_db" = "$after_db" ] || {
      echo "ASSERT FAIL[$stage]: refused mutator changed database bytes" >&2
      exit 1
    }
    assert_symlink_and_target_untouched
    ;;
  post_repair)
    # Detect-only: --repair must not remove, retarget, or replace the node.
    assert_symlink_and_target_untouched
    ;;
  post_undo)
    [ -d .obr ] || { echo "ASSERT FAIL[$stage]: .obr gone after undo" >&2; exit 1; }
    assert_symlink_and_target_untouched
    ;;
  *)
    echo "unknown stage: $stage" >&2
    exit 2
    ;;
esac
