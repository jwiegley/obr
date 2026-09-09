#!/usr/bin/env bash
# lint-gate.sh — strict all-target Clippy gate.
#
# No findings are allowed. Return Clippy's status and stream its diagnostics;
# historical lint inventories do not grant an allowance.
#
# IMPORTANT: invokes the `cargo-clippy` binary, not `cargo clippy`. The
# space form resolves through $CARGO_HOME/bin rustup shims ahead of any
# toolchain on PATH and can silently run a stale nightly.
# The direct invocation still needs the `clippy` subcommand argument:
# cargo-clippy skips it before parsing options, so omitting it drops --all-targets.
set -uo pipefail

# Sanitize git's hook environment: when run from a pre-commit hook, git
# exports GIT_DIR/GIT_INDEX_FILE/GIT_WORK_TREE, and any test that shells out
# to `git commit` in its own temp directory would resolve them to THE REAL
# REPOSITORY and commit there (observed live: a hook-context unit run left a
# foreign "initial" commit on the branch). Tests must see a clean git env.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_PREFIX GIT_OBJECT_DIRECTORY
unset GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_AUTHOR_DATE
unset GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL GIT_COMMITTER_DATE

# Source archives and Nix builds have no .git directory.
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)" || exit 2
cd -- "$repo_root" || exit 2

exec cargo-clippy clippy --all-targets -- -D warnings
