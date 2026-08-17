#!/usr/bin/env bash
# Fixture: base_jsonl_missing_post_flush
# FM: fm-state_files-base-jsonl-missing-or-stale (missing-post-flush subset)
#
# Older skeletons removed .obr/merge.base.jsonl after a sync flush. This
# fixture avoids deletion: a fresh `obr init` workspace already has no anchor,
# so we synthesize the post-flush evidence by setting metadata.last_export_time
# directly.
#
# The detector only warns when the anchor is missing AND the workspace is not
# `workspace_verifiably_in_sync`; when it can prove the database and the export
# agree it reports `missing_but_in_sync` as ok, because the next flush simply
# recreates the anchor. A bare `obr init` used to satisfy "not verifiably in
# sync" for the wrong reason: init seeded the export without certifying it, so
# there was no stored content hash to compare against. That was a defect — it
# also made the first `obr sync --flush-only` in every new workspace fail
# closed — and init now certifies the empty export it writes. A fresh workspace
# is therefore genuinely in sync, and the drift this fixture needs has to be
# planted on purpose.
#
# Plant it as an *absent* certificate — `jsonl_content_hash` empty, which is
# how the schema seeds the row and therefore the literal state of every
# workspace initialized before init learned to certify. The detector cannot
# compare a hash it does not have, so it correctly declines to call the
# workspace in sync and warns about the missing anchor.
#
# Not a *wrong* hash: `doctor --repair` recomputes a stored hash that disagrees
# with the export (`export_hash_cache_recomputed`), which would make the run
# report `repaired: true` and break the post_repair stage's "nothing to repair"
# assertion. An empty one it leaves alone, which is what makes this the plant
# that survives the repair the fixture requires to be a no-op.
#
# Not an edit to the export file either: the harness's `_obr_pinned` wrapper
# rewrites a fresh workspace back to the pre-D-SURFACE shape (empty
# `.obr/issues.jsonl`, no `PLAN.org`), so a fixture that edited one named file
# would be planting into whichever shape it guessed. Metadata is where the
# evidence lives in either shape.

set -euo pipefail
target_dir="${1:?usage: corrupt.sh <target_dir>}"
tool_bin="${TOOL_BIN:-br}"

mkdir -p "$target_dir"
cd "$target_dir"
"$tool_bin" init >/dev/null 2>&1

sqlite3 .obr/obr.db \
	"UPDATE metadata SET value='' WHERE key='jsonl_content_hash';"

# Avoid unrelated inner-gitignore repair noise; this fixture is testing the
# detect-only missing-post-flush branch.
for pattern in ".write.lock" "*.tmp"; do
	if ! grep -Fxq "$pattern" .obr/.gitignore 2>/dev/null; then
		printf '\n%s\n' "$pattern" >>.obr/.gitignore
	fi
done

sqlite3 .obr/obr.db \
	"UPDATE metadata SET value='2026-05-01T00:00:00Z' WHERE key='last_export_time';"

if [ -e .obr/merge.base.jsonl ]; then
	echo "corrupt.sh: fresh workspace unexpectedly has .obr/merge.base.jsonl" >&2
	exit 1
fi

sqlite3 .obr/obr.db \
	"SELECT value FROM metadata WHERE key='last_export_time' ORDER BY rowid DESC LIMIT 1;" \
	>.fixture_last_export_time

if [ "$(cat .fixture_last_export_time)" != "2026-05-01T00:00:00Z" ]; then
	echo "corrupt.sh: failed to plant metadata.last_export_time" >&2
	exit 1
fi

if [ -e .fixture_baseline ]; then
	echo "fixture baseline already exists; expected a fresh workspace" >&2
	exit 1
fi
mkdir -p .fixture_baseline
tar --exclude=.fixture_baseline -cf .fixture_baseline/state.tar .
