#!/usr/bin/env bash
# Fixture assertions: gitignore_bare_pattern
set -euo pipefail
target_dir="${1:?usage: assert.sh <target_dir> <stage>}"
stage="${2:?usage: assert.sh <target_dir> <stage>}"
tool_bin="${TOOL_BIN:-br}"
cd "$target_dir"

# The planted rules, none of which repair may touch. `*.org` hides the surface,
# but it is a broad rule: removing it would un-hide every other org file in the
# tree, which is an edit to operator intent rather than a repair.
assert_planted_rules_intact() {
  local stage="$1"
  local pattern
  for pattern in '^\*\.log$' '^\*\.org$' '^build/$'; do
    grep -qE "$pattern" .gitignore || {
      echo "ASSERT FAIL[$stage]: $pattern no longer in .gitignore" >&2
      cat .gitignore >&2
      exit 1
    }
  done
}

# A file left alone because the detector never saw the problem looks identical
# to one left alone deliberately. Every stage that asserts "still present" also
# re-checks that the warning is live and still names the rule, so this fixture
# cannot pass by going blind.
assert_warns_and_names_the_glob() {
  local stage="$1"
  local out
  out=$("$tool_bin" doctor --json 2>/dev/null) || true
  echo "$out" | jq -e '
    .checks[] | select(.name == "gitignore.obr_inner")
    | select(.status == "warn" or .status == "error")
    | select(.message | test("\\*\\.org"))
  ' >/dev/null || {
    echo "ASSERT FAIL[$stage]: gitignore.obr_inner does not warn naming *.org" >&2
    echo "$out" | jq '.checks[] | select(.name == "gitignore.obr_inner")' >&2
    exit 1
  }
}

case "$stage" in
  detect)
    assert_warns_and_names_the_glob "$stage"
    ;;
  post_repair)
    # Repair declines the broad rule and says so; the file is untouched.
    assert_planted_rules_intact "$stage"
    assert_warns_and_names_the_glob "$stage"
    ;;
  post_undo)
    # Repair was a no-op here, so undo has nothing to restore. What it must not
    # do is damage a file it never rewrote.
    assert_planted_rules_intact "$stage"
    assert_warns_and_names_the_glob "$stage"
    ;;
  *)
    echo "unknown stage: $stage" >&2
    exit 2
    ;;
esac
