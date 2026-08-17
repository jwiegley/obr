# CI/hook gate baselines

Three inventories back the no-NEW-findings gates (one of them deliberately
absent — see the integration set below):

- **clippy** (`pedantic` + `nursery`, `-D warnings`, nightly 1.99):
  `clippy-lints-baseline.txt` is a histogram of the `-D clippy::<lint>`
  *notes* clippy prints (`count lint`, single-space separated) — 21 lines
  summing to 54. That is deliberately NOT the finding count: clippy emits one
  such note per lint kind per compilation unit, while the tree currently
  reports ~86 individual first-party errors. `scripts/lint-gate.sh` derives
  the same histogram from a fresh run and compares it per lint kind, so the
  number to reconcile against when paying down debt is the per-lint count in
  this file, not a total.
- **unit tests** (`cargo test --lib --bins`): 2 failures, in
  `unit-failures-baseline-Darwin.txt`.
- **the curated integration set** (`vocabulary_lint`, `golden_obr_init`,
  `golden_rich_panels`): **no inventory file, by design** — all three are
  green everywhere they run and `scripts/unit-gate.sh` phase 2 demands a
  clean run. See "The integration set" below.

`scripts/lint-gate.sh` and `scripts/unit-gate.sh` compare against these
inventories and fail only on NEW findings — tightening gates that are green
on day one. Shrink the inventories as debt is paid down; never grow them.

## Not a gate: the full-suite inventory

`full-suite-inventory-Darwin.txt` records the complete failure set of
`cargo test --all-features --no-fail-fast` — 43 failures across 149 targets at
02663b0d. **No script reads it.** It is named so it can never be mistaken for
`integration-failures-baseline-$(uname -s).txt`, which unit-gate WOULD honour and
which would weaken the integration set from "fully green" to "no new failures".

It exists because that tier is invisible to the push path: ci.yml runs
`--lib --bins` plus doctests, so every `tests/*.rs` target runs only in
nightly.yml or when another job names it (obr-rh0). Two genuine failures sat there
unseen through an effort that took the Doctor job from 60/65 to 65/65 fixtures.
Recording the set means the next change can be judged against a number.

Regenerate it the same way it was made, and note the two things that will
otherwise mislead you: use a canonical TMPDIR (the macOS default lives under a
`/var` symlink that obr refuses as a database route, which aborts the lib target
with no summary at all), and read the totals from the complete log rather than a
filtered stream — computing them from a `tee | grep` pipeline is how the first
draft of this file reported 7 failures instead of 43.

## The integration set

Until this was added, `scripts/unit-gate.sh` ran `cargo test --lib --bins`
only, so **no integration test guarded a commit at all**. That is how
`tests/golden_rich_panels.rs` — the estate's only pseudo-terminal harness, and
therefore its only coverage of Rich-mode output — came to be dead on macOS for
four releases: it drove util-linux's `script -c`, which BSD rejects outright,
so all of its goldens were failing rather than guarding, and nothing local
ever ran them.

Phase 2 of the gate now runs three integration binaries, named in
`INTEGRATION_TESTS` at the top of the script:

| binary | what it guards |
| --- | --- |
| `vocabulary_lint` | staleness: drives the real binary over a command matrix in plain, `--json`, TOON, `--robot` and a real pty, and fails on pre-fork vocabulary (`.beads`, `beads.db`, `issues.jsonl`, `bd-`, `br `, `BEADS_`/`BD_`/`BR_`). Also walks every subcommand's `--help` from clap's own tree. |
| `golden_obr_init` | the `.obr/` layout `obr init` writes. |
| `golden_rich_panels` | Rich-mode panel/table rendering through a real terminal. |

Measured on an idle Darwin host: phase 2 adds roughly 13 s to a gate whose
unit phase already costs about 108 s.

Two properties make the phase honest rather than decorative:

- **no inventory.** A file named
  `integration-failures-baseline-$(uname -s).txt` would be honoured if it
  existed, but none is committed, so the gate demands a fully green run.
  Recording one has to be a deliberate act with a written reason, exactly like
  widening the unit inventory.
- **every named binary must report.** The gate counts `test result:` summary
  lines and requires one per entry in `INTEGRATION_TESTS`. A target that never
  ran — renamed file, build failure for that target, harness abort — produces
  no `FAILED` lines and would otherwise read as green. That is the same
  vacuous-pass shape as a non-compiling tree, and it is refused the same way
  (exit 2, "refusing to report green").

`ci/baseline/vocabulary-debt.txt` is the vocabulary gate's own inventory: a
list of stale strings the gate tolerates, currently empty. Its header explains
what may and may not go in it.

## Environment contract for the unit inventory

**Record and compare under a canonical, symlink-free `TMPDIR`.**
`scripts/unit-gate.sh` enforces this itself: on Darwin it mints a run
directory under `/private/tmp` and exports it, because macOS's default
`TMPDIR` lives under `/var`, which is a symlink to `/private/var`. Linux
`/tmp` is already canonical and is left alone.

This is not a formality. Under the default macOS `TMPDIR` the unit suite
reports **266** failures and the harness process then exits abnormally,
truncating the run. Under a canonical `TMPDIR` the same tree reports **2**
failures and runs to completion. Both symptoms have one cause: a test creates
a workspace under `TMPDIR`, the code canonicalizes the path it is given, and
the two no longer compare equal.

The 264 phantom failures were not merely noise. Because the gate matches on
failure NAMES, an inventory recorded on a polluted host silently absolves a
genuine regression in any test whose name it happens to list — which is how a
large set of real failures went unnoticed on this branch until the suite was
re-run under a clean `TMPDIR`. Never commit an inventory recorded on a
polluted host, and never widen one to make a run pass.

There is no cross-platform inventory. One recorded on another OS would
absolve the wrong names, so a platform without its own file gets an empty
inventory and the gate demands a fully green run until someone records one.

## Recording an inventory

Run the gate's own command in the gate's own environment, then keep the
failure names:

```sh
tmp="$(mktemp -d /private/tmp/obr-gate.XXXXXX)"   # Linux: skip, /tmp is fine
TMPDIR="$tmp" cargo test --lib --bins 2>&1 |
  grep ' \.\.\. FAILED' | sed 's/ \.\.\. FAILED//' |
  LC_ALL=C sort -u > "ci/baseline/unit-failures-baseline-$(uname -s).txt"
rm -rf "$tmp"
```

Keep the leading `test ` on each line: the gate compares raw harness lines
with only the ` ... FAILED` suffix removed, so a stripped inventory matches
nothing and every failure reads as new.

Before committing the result, run it twice more and drop any name that does
not fail every time. An intermittent failure in the inventory permanently
absolves that test, so report the flake instead of listing it.

Record on an otherwise idle machine, and re-run the gate before believing a
lone new name. A handful of unit tests assert hard wall-clock bounds and fail
when the host is busy — across six runs of one unchanged tree, four were
clean and the other two each failed a *different* single test. Observed so
far, all deliberately absent from the inventory:

- `config::tests::read_only_fast_open_miss_waits_for_write_lock_before_rebuild`
  asserts the exact figure in `Timed out after 1ms waiting for write lock`,
  but the composite lock authority shares one 1ms budget across its
  components, so under load a later component legitimately reports `0ms`.
- `cli::commands::vcs::tests::inherited_descendant_output_descriptors_do_not_delay_return`
  requires a spawned probe to return in under 500ms.
- `cli::commands::tests::routed_workspace_write_lock_respects_external_timeout`.

Listing them would absolve tests that are usually green, which is worse than
an occasional hook re-run. Tightening the assertions themselves is the real
fix and is still open.

## Known pre-existing failures

Both entries in the Darwin inventory are upstream debt, verified against an
untouched upstream worktree:

- `cli::commands::changelog::tests::test_git_tag_date_falls_back_for_lightweight_tags`
- `cli::commands::vcs::tests::non_utf8_jsonl_leaf_is_observed_without_lossy_argument_conversion`

Outside the unit suite,
`tests/workflow_action_pins.rs::repository_workflow_action_pins_are_inventory_backed`
also fails at upstream: its `.github/action-pins.jsonl` inventory drifted from
its own workflows before the fork point. The gate for workflow edits is
therefore "no NEW pin failures"; reconciling the inventory is upstream debt.

## Host hazard: cargo subcommand shims

`cargo clippy` / `cargo fmt` (space form) resolve through `$CARGO_HOME/bin`
rustup shims *before* `PATH` and can silently run a stale nightly. Always
invoke the `cargo-clippy` / `cargo-fmt` binaries directly.
