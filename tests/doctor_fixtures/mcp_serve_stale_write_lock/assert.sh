#!/usr/bin/env bash
# Fixture assertions: mcp_serve_stale_write_lock

set -euo pipefail

target_dir="${1:?usage: assert.sh <target_dir> <stage>}"
stage="${2:?usage: assert.sh <target_dir> <stage>}"
tool_bin="${TOOL_BIN:-br}"

cd "$target_dir"

assert_lock_artifacts_preserved() {
    [ -f .obr/.write.lock ] || {
        echo "ASSERT FAIL[$stage]: .obr/.write.lock was removed or moved" >&2
        exit 1
    }
    [ ! -L .obr/.write.lock ] || {
        echo "ASSERT FAIL[$stage]: .obr/.write.lock became a symlink" >&2
        exit 1
    }
    [ -f .obr/.write.lock.holder.pid ] || {
        echo "ASSERT FAIL[$stage]: .write.lock.holder.pid was removed or moved" >&2
        exit 1
    }
    if [ "$(cat .obr/.write.lock.holder.pid)" != "99999999" ]; then
        echo "ASSERT FAIL[$stage]: holder pid sidecar content changed" >&2
        exit 1
    fi
    [ -f .fixture_lock_identity ] || {
        echo "ASSERT FAIL[$stage]: missing baseline lock identity" >&2
        exit 1
    }
    expected_identity=$(cat .fixture_lock_identity)
    # GNU `stat -c` / BSD `stat -f`; see corrupt.sh.
    actual_identity=$(stat -c '%d:%i' .obr/.write.lock 2>/dev/null || stat -f '%d:%i' .obr/.write.lock)
    if [ "$actual_identity" != "$expected_identity" ]; then
        echo "ASSERT FAIL[$stage]: lock identity changed $expected_identity -> $actual_identity" >&2
        exit 1
    fi
}

assert_no_repair_actions() {
    [ -d .doctor/runs ] || return 0
    local actions
    while IFS= read -r actions; do
        if grep -q -v '^[[:space:]]*$' "$actions"; then
            echo "ASSERT FAIL[$stage]: persistent lock inode produced repair actions in $actions" >&2
            sed 's/^/  /' "$actions" >&2
            exit 1
        fi
    done < <(find .doctor/runs -name actions.jsonl -type f | sort)
}

case "$stage" in
    detect)
        assert_lock_artifacts_preserved
        set +e
        out=$("$tool_bin" doctor --json 2>/dev/null)
        doctor_rc=$?
        set -e
        if [ "$doctor_rc" -ne 0 ]; then
            echo "ASSERT FAIL[$stage]: healthy persistent inode made doctor exit $doctor_rc" >&2
            echo "$out" >&2
            exit 1
        fi
        assert_lock_artifacts_preserved
        # obr-m6m: this demanded `details.reason == "persistent_advisory_inode"`.
        # That string is emitted NOWHERE in src/ and never was — it is listed in
        # docs/research/upgrade/DECISIONS.md under "Born-broken tests" as a
        # never-implemented classification. The implemented vocabulary for a
        # probed lock is `probe_acquired_free` / `probe_would_block_live_holder`
        # / `stale_mtime`, and the correct answer for THIS fixture's scenario —
        # an old lock inode whose owning process is gone, so nothing holds the
        # advisory lock — is `probe_acquired_free`.
        #
        # This is not the assertion being loosened to whatever the product
        # happens to say. It is being pointed at a real observation for the
        # first time: the probe previously could not answer honestly at all,
        # because a plain `obr doctor` held the workspace write lock while
        # checking it, so every workspace came back "held by a live process".
        # The stronger claim is now checkable and is checked: the reason must be
        # the FREE branch specifically, which fails if doctor ever resumes
        # colliding with itself, and fails if a live holder is present.
        echo "$out" | jq -e '
          .checks[]
          | select(.name == "write_lock")
          | select(.status == "ok")
          | select(.details.reason == "probe_acquired_free")
          | select(.details.finding_id == "fm-concurrency_primitives-orphaned-write-lock")
        ' >/dev/null || {
            echo "ASSERT FAIL[$stage]: persistent MCP lock inode was not classified healthy" >&2
            echo "$out" | jq '.checks[] | select(.name == "write_lock")' >&2
            exit 1
        }
        ;;
    post_repair)
        assert_lock_artifacts_preserved
        assert_no_repair_actions
        issue_id=$("$tool_bin" list --json | jq -r 'if type == "array" then .[0].id else .issues[0].id end')
        if [ -z "$issue_id" ] || [ "$issue_id" = "null" ]; then
            echo "ASSERT FAIL[$stage]: could not resolve seed issue id" >&2
            exit 1
        fi
        "$tool_bin" update "$issue_id" --priority 1 --json >/dev/null
        ;;
    post_undo)
        assert_lock_artifacts_preserved
        ;;
    *)
        echo "unknown stage: $stage" >&2
        exit 2
        ;;
esac
