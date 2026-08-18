# Known residuals

Everything on this page is a thing that is currently wrong, incomplete, or
carried. It exists so that nobody has to rediscover any of it by surprise.

Three categories, and the distinction matters:

- **Carried from upstream** — already broken at the fork point. Not caused by
  the rename or the Org port, and not fixed by them either.
- **Introduced by this fork** — gaps the rename or the surface relocation opened
  and did not close.
- **Deliberate** — decided, not overlooked.

Commit hashes cited in entries dated before 2026-08-17 may refer to the fork's
pre-compaction history, which was deliberately squashed away; those cites stay
as dated records even where the hashes no longer resolve from a fresh clone.

---

## Carried from upstream

These fail on the untouched upstream tree as well. Where a count is given, it
was measured against a build of the fork point, not estimated.

### Test-suite failures present at the fork point

| Area | What |
|---|---|
| Unit suite | 2 failures at baseline, inventoried in `ci/baseline/unit-failures-baseline-Darwin.txt`. Measured under a canonical `TMPDIR`; the default macOS one reports 266 failures and then truncates the run, for reasons that belong to the recording host rather than the tree. See `ci/baseline/README.md`. |
| Clippy | ~86 first-party findings under `pedantic` + `nursery`. `ci/baseline/clippy-lints-baseline.txt` histograms the per-lint-kind `-D` notes (54), which is what the gate compares — see `ci/baseline/README.md`. |
| Workflow action pins | `tests/workflow_action_pins.rs::repository_workflow_action_pins_are_inventory_backed` — upstream's `.github/action-pins.jsonl` drifted from its own workflows before the fork. Every SHA mismatch it reports is upstream's. |
| Sidecar allowlist | Three `e2e_sync_git_safety` integration tests fail because sync writes `.br-jsonl-write-<digest>.lock` authority sidecars that the test's allowlist does not admit. |
| Routing | ~23 `e2e_routing` failures, largely lock-timeout behaviour under the routing paths. |
| Empty-directory init guard | `e2e_env_overrides` external-JSONL and no-DB rows, five in total, around `OBR_JSONL` pointing outside the workspace. |
| Concurrency | `e2e_concurrency` is genuinely flaky here: three consecutive full-suite runs produced 2, 4 and 3 failures from a fluctuating set, and the ones that look new pass in isolation. |

`scripts/unit-gate.sh` and `scripts/lint-gate.sh` exist because of the first two
rows: they fail only on findings **absent from the committed baseline**, which
is what makes a red suite still useful as a gate. Shrink the inventories as debt
is paid; never grow them.

### Doctor fixtures: 12 of 65 still failing

The harness was repaired so that 53 of 65 pass (`tests/doctor_fixtures/run_all.sh`
pins each scratch workspace to the in-dir JSONL export that these fixtures were
written against). The remaining twelve each need their own diagnosis. Sampled
causes:

| Fixture | Sampled cause |
|---|---|
| `orphaned_write_lock` | `assert.sh` calls `assert_lock_identity_preserved`, a shell function that is never defined. |
| `orphan_tmp_quarantine` | Expects the quarantine path under a literal `.beads` component. |
| `db_missing_with_jsonl`, `empty_database_with_jsonl` | Expect `--repair` to rebuild the database from JSONL; it does not. |
| `merge_artifact_stuck`, `recovery_artifacts_aged` | Expect a populated `.doctor/runs/<id>/quarantine/`; it comes back empty. |
| `mcp_serve_stale_write_lock` | A persistent MCP lock inode is not classified healthy. |
| `doctor_runs_dir_growth`, `obr_history_growth` | Corrupt stage fails without a diagnostic. |
| `permissions_write_lock_unwritable`, `sqlite_version_downgrade` | Detect stage does not surface the expected diagnostic. |

### The `startup_cache.health` doctor check could not fire — retired 2026-08-15

`startup_cache.health` reported `ok` on every workspace, including the two states
it existed to detect. It is now removed, together with its `--repair` fixer
(`doctor.startup_cache_quarantine`), its rows in `CHECK_NAME_TO_FINDING_ID` and in
the capabilities envelope's detector and fixer registries, the
`config::doctor_inspect_startup_cache` family that served only it, the
`startup_cache_poisoned` fixture, and the fixer's unit test.

The docstring stated the premise that fails: these are "the cache failures the
production read path silently swallows via `.ok()?` — the file stays on disk
poisoning future invocations until something cleans it up". It does not stay on
disk. Doctor's own startup is what cleans it up, before `execute()` reaches the
check. Measured on the pre-removal binary, in fresh workspaces with `HOME` pinned
so the cache resolved to `$HOME/.cache/obr/startup/`:

| Planted state | Check said | State afterwards |
|---|---|---|
| Current-key file overwritten with `not-json-at-all`, doctor run with `OBR_STARTUP_CACHE=1` | `ok` | file holds a fresh `{"version":2,"key":…}` record |
| Same file `chmod 000` (mode 0, unreadable) | `ok` | mode 644, readable, fresh record |
| Poisoned file present, doctor run *without* the flag | `ok` | poison untouched — the run looked under a different key |
| Nothing poisoned | `ok` | — |

The read path fails, `.ok()?` swallows it, full resolution runs, and a valid cache
is written back. Removing read permission does not protect the poison either: the
write is temp-file + `rename`, which needs only directory permission, and it
restores the mode as a side effect. The third row is the fixture's own shape and a
second, independent reason the check was blind — `startup_cache_key` hashes an env
witness containing every `OBR_*` variable, including the flag that enables caching,
so a doctor run without it resolves a different filename than the run that primed
the cache.

Note the inversion: the check inspected the one file that cannot stay broken, and
deliberately ignored the stale-key `startup-*.json` entries, which nothing ever
rewrites and which are the only ones that can persist. Repointing it at those would
be a different finding (unbounded cache growth) under a different id, and was not
done here.

The blast radius either way is small: the startup cache is opt-in and off by default
(`OBR_STARTUP_CACHE`), so on a default install there is no cache file to poison.

The defect is upstream's — `a9791555` added the detector and fixer, and both are
present at the fork point `d1fb0d3a` — so removing them is a deliberate divergence
from upstream under the rule stated in the dead-code assessment below: inert
upstream code is kept, code carrying *active* harm is not, and a check that answers
`ok` while incapable of answering anything else is a misleading product contract.
`fm-configs-startup-cache-poisoned` is now declared an `exception` row in
`tests/doctor_fixtures/COVERAGE.md` rather than deleted, so the id remains
explicable to anyone who finds it in an archived report.

The sibling defect is **not** resolved by this. `write_lock` is blind for the same
structural reason — plain `obr doctor` holds the workspace lock it probes — and it
is still live, tracked as `obr-m6m`. Unlike the cache, that check guards state a
default install really has.

### The `bv` snapshot goldens

`snapshots::robot_output::robot_golden_bv_{next,plan,triage}` shell out to `bv`,
a separate tool that is not installed here, so the tests hard-panic rather than
skip. They cannot be regenerated or verified without it.

Those three goldens still contain `br update` / `br show` / `br ready` /
`br blocked` command strings — but `bv` emits them, so whether they should read
`obr` depends on `bv` itself being rename-aware. Not resolvable from inside this
repository.

---

## Introduced by this fork

### Gitignore detection: whole-directory patterns are not flagged — resolved 2026-08-15

`check_root_gitignore` detected root-`.gitignore` lines that hide the tracked
`PLAN.org` surface, but skipped whole-directory forms (`doc/`, `doc/*`) entirely.
The stated reason was sound and still holds: deleting those would unmask files
the operator meant to ignore, and for `doc/` specifically git cannot re-include a
file underneath an excluded directory at all — so removing the line is a rewrite
of intent, not a repair.

The error was concluding that a line obr must not REPAIR is a line obr should not
REPORT. Measured before the fix: a repository with `doc/` in `.gitignore` and its
surface at `doc/PLAN.org` answered `ok: true`, `workspace_health: healthy`, zero
non-OK checks — while git never saw the surface, every issue lived only in the
per-machine `.obr/` cache, and a fresh clone got nothing. The surface reaching git
is the entire product; staying silent about it being unreachable was the worst of
the available answers.

The whole-directory forms (`doc/`, `/doc/`, `doc`, `/doc`, `doc/*`, `/doc/*`) are
now listed as BROAD in `surface_hiding_gitignore_pattern_kinds`, which reports
them and — by the same existing split that governs `*.org` — never removes them.
`--repair` names the offending line and tells the operator to narrow it. A surface
at the repository root is unaffected, so ignoring a `doc/` directory you do not
export into is not a false positive.

### Naming debt in the doctor checks

The check names `gitignore.obr_inner` and `gitignore.obr_inner_present`, the
finding id `fm-configs-gitignore-leaking-obr`, and the fixture directory names
now mean close to the **inverse** of what they say: `.obr/` is supposed to be
hidden, and the surface is supposed to be visible. The names were kept stable
because renaming ripples through the capabilities registry, the exit-code and
finding maps, and `tests/doctor_fixtures/COVERAGE.md`.

### Org-format regressions relative to JSONL

| Gap | Effect |
|---|---|
| Witness merkle spurious drift | The additive-reconcile witness can report drift on Org exports where the JSONL path was stable. |
| No-op-flush fast path | The cheap "nothing changed, skip the write" path does not engage for Org exports, so a redundant flush does more work than it used to. |

### `beads.jsonl` is read but never written

Legacy compatibility covers *reading* an in-dir `beads.jsonl`, but there is no
path that writes one. A workspace resolved onto that artifact keeps using it;
nothing migrates it, and nothing creates a new one.

### `BR_BINARY` in the helper scripts

`scripts/conformance.sh`, `scripts/e2e.sh` and `scripts/bench.sh` document and
export `BR_BINARY` to point the harness at a built binary. The harness prefers
`OBR_BINARY` and accepts `BR_BINARY` only as a legacy alias, so the scripts
work but advertise the old name. (`BD_BINARY` alongside it is correct — that
one names the Go `bd` binary the conformance suite compares against.)

### `BeadsError`

The crate's error type is still named `BeadsError` throughout. It is internal
and appears in no user-facing output, so it was left alone rather than churn
every `Result` signature in the tree.

### `vcs-status` help text — resolved 2026-08-12

`b525a0ed` already changed the clap doc comment (`src/cli/mod.rs:980`) to
"Explicitly inspect Git visibility for the configured export", and `obr --help`
now prints exactly that. This residual then predicted its own consequence: the
source was corrected and the help golden was not, so
`snapshots__snapshots__cli_output__help_output.snap` kept the old line for
months behind a `#[cfg(feature = "mcp")]` that no local or CI gate ever
compiled. Both are now correct.

One sibling line is still stale and was **not** covered by that fix: the
EXAMPLES block of `obr sync --help` (`src/cli/mod.rs:973`) advertises
`obr vcs-status --json` as "Explicitly inspect JSONL Git visibility". No golden
pins it — `snapshot_help_output` captures the root help only.

### `agent_baseline/README_first_80_lines.md`

Nothing regenerates this automatically — it is maintained by hand as
`head -80 README.md`. Any README rewrite invalidates it.

---

## Deliberate

### Read-only doctor is lock-free — decided 2026-08-16, completed 2026-08-17

Plain `obr doctor` (no `--repair`/`--fix`/`--repair-indexes`) does not take the
workspace write lock. Three observable consequences, all decided rather than
incidental:

- **Under a live foreign lock holder, doctor inspects and exits 0**, reporting
  the holder through the `write_lock` probe — where it previously refused with
  `concurrency_lost` / exit 5. A diagnostic that refuses whenever another
  process is running is close to useless on multi-agent repositories, and this
  is what makes the orphan probe meaningful at all.
- **Divergence findings can be transiently wrong under concurrent writers.**
  `counts.db_vs_jsonl` and `sync.metadata` compare two stores read at different
  instants with no lock held; when they are non-OK on a lock-free run they carry
  `details.unsynchronized_read: true` and a note saying a re-run confirms or
  clears the drift. Consumers should treat a single unsynchronised divergence as
  a prompt, not a verdict.
- **`db.write_probe` — the one check that writes — takes the workspace lock for
  its own duration**, with a bounded (1.5 s) wait: under `--repair` the process
  already holds the lock on another file description, so an unbounded wait would
  deadlock, and a busy foreign writer should not stall a diagnostic. On timeout
  the probe still runs under SQLite's own locking and reports
  `details.serialized: false`.

`--repair` still holds the lock for its whole run; its own `write_lock` finding
is annotated `probe_would_block_self_held` rather than reporting a phantom
foreign holder.



### Packaging checksums are placeholders

`packaging/homebrew/obr.rb`, `packaging/scoop/obr.json` and both AUR PKGBUILDs
carry all-zero SHA-256 values. obr has never published a release, so no true
digest exists; pairing upstream's real hashes with `jwiegley/obr` URLs would be
false data about bytes nobody can fetch. A zero digest fails an install loudly,
and `.github/workflows/update-package-manifests.yml` rewrites all of them from
the published `.sha256` sidecars on the first release.

**Consequence:** the manifests are not installable until a release is cut.

### Release signing has no published key

`.github/workflows/release.yml` signs archives only when `MINISIGN_SECRET_KEY`
is configured, and the release notes advertise verification using
`vars.MINISIGN_PUBLIC_KEY`. Until that variable is set the verification block
renders an empty key. Upstream's public key was removed rather than carried,
because it did not sign these artifacts.

### The Org surface normalizes some stored values, once

The tracked surface is a fixpoint from the first write: flush, import, and
flush again produce identical bytes. Reaching that from arbitrary database
content means a few values cannot be carried verbatim, and the first flush
converts them. Each is a shape the Org grammar has no room for, and each
converges immediately — the second flush changes nothing.

| Value | Stored as | Why |
|---|---|---|
| Free text with trailing newlines (`"a\n\n"`) | Newlines dropped | Every emission path is `str::lines()`-based; a trailing newline never reaches the file. |
| Free text with leading blank lines | Blank lines dropped | The blank line after the drawer is the body's delimiter; the reader consumes leading blanks inside example blocks too. |
| Free text that is entirely whitespace | Absent | It parses to no elements and reads back as `None`. |
| CRLF line endings | LF | One line-ending convention on the surface. |
| Padded strings (`" alice "`, in a property or a heading title) | Trimmed | Every Org parser trims drawer values and heading titles. |
| `status = pinned` with `pinned = false` | `pinned = true` | The `NOTE` keyword *is* pinned on read; the two fields cannot disagree on the surface. |
| Newlines inside a drawer property value | Spaces | A drawer property is one line. Multi-line close/delete reasons avoid this by traveling as `** Close Reason` / `** Delete Reason` sections instead. |
| Seconds and sub-seconds on any of the seven instants | Dropped | An Org timestamp is `[YYYY-MM-DD Ddd HH:MM]`; there is no seconds field. See the next section — this one is not merely cosmetic. |
| A weekday that disagrees with the date | Recomputed | The weekday is derived, never carried, so a hand-edit that names the wrong day still imports and is corrected. |
| An Org timestamp in the `<active>` form | Rewritten `[inactive]` | obr writes one form. Reading both costs a `strip_prefix`. |
| A local reading that does not exist (the DST spring-forward hour) | The transition instant | Only a hand-edit can produce one; obr renders from real instants. |

`src/sync/org_bridge.rs::canonical_body_text` is the single implementation of
the free-text rules; the round-trip property test asserts against an explicit
statement of this table rather than pre-applying it to its generator.
`format_org_timestamp` and `parse_org_timestamp_in` are the corresponding pair
for instants.

### Org timestamps are local, and minute precision becomes the data model

The seven instants an issue carries are Org-native inactive timestamps in the
machine's local zone — `:CREATED:`, `:MODIFIED:`, `:FINISHED:`, `:DUE:`,
`:DEFERRED:`, `:DELETED:`, `:COMPACTED:`. The reason to do it is that Org's
own tooling can then read them — `org-sort-entries` by property (the format
sorts lexicographically in chronological order), `org-entry-get` and `org-ql`,
column view, `C-c .` to edit one — where an RFC3339 string was opaque to all of
it.

**Inactive is a choice, not a limitation.** An earlier revision of this
document and of the README justified the inactive form by claiming that a
drawer property "does not reach the plain agenda in either form". That claim is
false, and was measured false against Org 9.7.11 (Emacs 30; the list is byte-identical on Org 9.8.7 / Emacs 31): an **active**
`<…>` timestamp sitting in a `:PROPERTIES:` drawer *does* show up in the day
agenda — the agenda's timestamp scan matches an active timestamp anywhere in an
entry's text, and a property drawer is part of that text — while an inactive
`[…]` one does not *by default*. Not "never": `org-agenda-include-inactive-timestamps`
is a standard user option (default nil, toggled in-agenda with `v [`), and with
it set the inactive drawer timestamps do appear — all seven properties across
every issue. So the bracket keeps the corpus out of the agenda for a
default-configured Emacs only; for a reader who enables that option, nothing
here does.

So the bracket is load-bearing, in the opposite direction from what was
written. obr emits `[…]` because putting 549 issues (and every `:CREATED:` and
`:MODIFIED:` among them) into the day agenda is not what anyone wants their
agenda to be. The opt-in exists and is deliberately not taken: emitting `<…>`
instead is a one-call-site change in `format_org_timestamp`. A hand-edit of a
single `:DUE:` to `<…>` works too but is not durable — the reader accepts both
forms and the writer emits one, so the next flush rewrites it back to `[…]`
(see the normalization table above). A per-issue, durable agenda presence would
mean emitting a real `DEADLINE:` planning line, which is a different change
with its own round-trip questions.

Org has nowhere to put a UTC offset and nowhere to put seconds, so three things
follow. None of them is an accident, and none of them is free.

**1. Minute precision is the data model, not a display choice.** The Org
surface is the source of truth and `.obr/` is a derived cache, so an instant
that has been through one flush-and-import cycle *is* minute-precision
everywhere afterwards. Reading never truncates — that is what lets a legacy
RFC3339 file import losslessly — but writing always does, so the database
converges on what the file can hold. The consequence lands on `updated_at`,
which drives last-write-wins: two edits inside the same minute are now
indistinguishable, where before they differed by seconds. Every site that
compares `updated_at` therefore has a stated rule for the tie, and every one of
them is a function of the two records alone — never of iteration order, never
of when the merge ran:

| Site | Tie resolves to |
|---|---|
| `additive` reconcile (`source_is_newer`) | Not "newer". Falls through to an `equal_timestamp_shared_scalar_drift` conflict the operator resolves. |
| `determine_action` (collision import) | `Skip` — keep what is already stored. |
| `merge_issue`, `PreferNewer`, both-modified and convergent-creation | Keep the local side, and say so: the note reads `kept local (equal timestamps)`, not `kept local (newer)`. `resolve_prefer_newer` is the one implementation. |
| `dirty_issues_missing_from_jsonl` | Do not preserve — the flushed copy is as good, so the rebuild's own import restores it. |

Keeping local on a `PreferNewer` tie is a bias, not a coin flip: it is the side
the merge already keeps when only one end changed, and it is stable when the
same merge runs twice. Breaking the tie on a content hash instead would be
symmetric between the two sides, but it would discard a local edit on the
strength of a comparison nobody can predict, which is worse for the person who
made the edit.

**2. `PLAN.org` is no longer byte-identical across machines in different
zones.** The same instant renders as `[2026-08-09 Sun 12:31]` in one zone and
`[2026-08-09 Sun 19:31]` in another, so two developers flushing the same
database produce different files and git sees a diff. This was a real property
of the surface and it has been traded away deliberately, for timestamps Org's
own tooling can read. It costs nothing in correctness — the instant survives,
and the fixpoint invariant is per-machine, which is where it is asserted — but
the tracked file will churn when it moves between zones.

**3. DST ambiguity resolves to the earlier instant.** One local hour occurs
twice a year, and an Org timestamp cannot say which of the two it means. The
rule is *always the earlier instant*, and it is stable: both candidates render
to the same local text, so re-flushing the file does not move it again. The
cost is that reading such a timestamp can shift the stored instant by the
offset delta (one hour, once a year, on a fall-back boundary) — the surface
stays a fixpoint, but the instant is not the one that went in. The mirror case,
the local hour that does not occur at all, can only come from a hand-edit;
it resolves to the first representable instant at or after it and the next
flush rewrites the property to a reading that exists.

Files written before this change spelled the seven properties `CREATED_AT`,
`UPDATED_AT`, `CLOSED_AT`, `DUE_AT`, `DEFER_UNTIL`, `DELETED_AT` and
`COMPACTED_AT`, with RFC3339 values. The reader still takes them — and takes
RFC3339 under the current spellings too, and Org timestamps under the legacy
ones, because it is one function — so an old `PLAN.org` imports without loss
and re-exports in the current form. Nothing writes a legacy spelling. If a file
somehow carries more than one spelling of one field, the current one wins
regardless of drawer order.

`closed_at` is the one field with two retired spellings rather than one, and
they rank newest-first:

| Drawer key | Status | Loses to |
|---|---|---|
| `:FINISHED:` | Current; written | — |
| `:CLOSED:` | Read only; one unreleased build, retired for the reason in the next section | `:FINISHED:` |
| `:CLOSED_AT:` | Read only; the pre-Org RFC3339 spelling | `:FINISHED:`, then `:CLOSED:` |

Precedence is by key, not by drawer order or value format: any of the three may
carry an Org timestamp or an RFC3339 string, because one value parser serves
them all.

### `CLOSED` is an org-special-property, so the close instant is `:FINISHED:`

The obvious name for `closed_at` is `:CLOSED:`, and one unreleased build used
it. It cannot work. `CLOSED` is a member of `org-special-properties` — names
Org reserves for its own derived or planning-line values — so Org shadows the
drawer key with the `CLOSED:` planning-line keyword: `(org-entry-get nil
"CLOSED")` returns `nil` even when the drawer plainly holds the property, and
`(org-entry-put nil "CLOSED" …)` signals an error rather than writing it. Every
reason for having Org-native timestamps at all — `org-entry-get`, `org-ql`,
column view, `C-c .` — is void for that one property. The close instant is
therefore spelled `:FINISHED:`, which is not reserved.

The full list, as `org-special-properties` evaluates in Org 9.7.11, so the next
person adding a property can check against it without booting Emacs:

```elisp
("ALLTAGS" "BLOCKED" "CLOCKSUM" "CLOCKSUM_T" "CLOSED" "DEADLINE" "FILE"
 "ITEM" "PRIORITY" "SCHEDULED" "TAGS" "TIMESTAMP" "TIMESTAMP_IA" "TODO")
```

`FINISHED`, and the other six instants obr writes (`CREATED`, `MODIFIED`,
`DUE`, `DEFERRED`, `DELETED`, `COMPACTED`), are all absent from it.
`src/sync/org_bridge.rs::no_emitted_property_collides_with_org_special_properties`
pins the list against a fully-populated record, so a future property that
collides fails a test rather than silently losing its Org tooling.

**Known wrinkle: obr does not read the `CLOSED:` planning line.** Because
`CLOSED` is special, a user who closes an issue *inside Emacs* — `C-c C-t` to a
done keyword with `org-log-done` set to `time` — gets Org's own planning line
inserted under the heading:

```org
* DONE [#B] Ship it
CLOSED: [2026-08-10 Mon 09:14]
:PROPERTIES:
...
```

obr's reader does not look at planning lines, and `PLAN.org` is rewritten from
the database on every flush, so that line is dropped on the next reflush and
the close time it recorded is lost. The issue's status still round-trips (that
comes from the TODO keyword).

**It is worse than losing the annotation, in the common case.** If you close in
Emacs an issue obr had not already closed, obr has no `closed_at` of its own —
so on import `normalize_issue` (`src/sync/mod.rs`) fills it from `updated_at`.
Measured: a heading marked `DONE` with `CLOSED: [2026-03-04 Wed 05:06]` and
`:MODIFIED: [2026-01-02 Fri 03:05]` imports with `closed_at` = the *modified*
instant, and re-exports `:FINISHED: [2026-01-02 Fri 03:05]`. The file does not
merely drop your annotation; it silently asserts a close time two months before
the real one. Close through `obr close` if the timestamp matters. This is
stated rather
than fixed: reading it would mean treating the planning line as a second,
higher-precedence source for `closed_at` and deciding what happens when it
disagrees with `:FINISHED:`, and writing it would put a value in the file that
obr's own property already holds.

### Unmodelled Org sections are dropped, with a warning

`PLAN.org` is rewritten from the database on every flush, so a level-2 section
under an issue that obr does not store (anything outside `Design`,
`Acceptance Criteria`, `Notes`, `Close Reason`, `Delete Reason`,
`Agent Context`, `Dependencies`, `Comments`) cannot survive. Import emits an
`UNRECOGNIZED_ORG_SECTION` warning naming the issue and the section; the
rewrite drops the text. Carrying it would mean an `unknown_sections` bag on
`Issue` and a place for it in every other export format.

**Consequence:** hand-edits must stay inside the recognized sections. A broken
`#+begin_src json` block in one of the three JSON-bearing sections is treated
differently — it fails the import outright, because importing it as empty
would delete the stored dependencies or comments on the next flush.

### Every git invocation goes through one hardened builder

obr shells out to `git` in exactly three read-only places — `vcs-status`,
`changelog` and `orphans` — and all of them build the command with
`sync::…::vcs::hardened_git`, which disables `core.fsmonitor` and the
repository's `hooksPath`, isolates system and global config, scrubs every
inherited `GIT_*`/askpass variable, and closes stdin.

That is not decoration: `core.fsmonitor` and `hooksPath` both name programs
the *repository* controls, so plain `git` inside a clone you did not create is
arbitrary code execution — and `obr orphans --fix` closes issues based on what
git reports. `tests/e2e_orphans.rs::orphans_and_changelog_invoke_git_only_through_the_hardened_builder`
asserts the flags per invocation (not across their union, which a single
hardened call would otherwise vouch for).

**Consequence:** a new git call site must use `hardened_git`. Building a bare
`Command::new("git")` is the bug the test exists to catch.

### Compatibility surface is narrow on purpose

`src/legacy_compat.rs` reads pre-rename **on-disk workspace artifacts** only —
`.beads`/`_beads` directories, `beads.db`, in-dir `beads.jsonl` / `issues.jsonl`
exports, merge/history/recovery/snapshot filenames, the Org drawer's
`BEADS_SCHEMA_VERSION`, `br.*.v1` schema ids, and the agents marker.

Environment variables and user-config locations are **not** compatible.
`OBR_*` and `~/.config/obr/config.yaml` are the only spellings obr reads; the
`BEADS_*` / `BD_*` / `BR_*` names and the `~/.config/{beads,bd}` and `~/.beads`
locations were removed outright rather than deprecated.

### Licensing

The project is MIT **with an additional rider** restricting use by specific
parties. The rider is part of the license conditions and must accompany any
redistribution — `packaging/aur/PKGBUILD` therefore installs `LICENSE` verbatim
rather than a plain MIT notice.

Copyright for the upstream work remains with Jeffrey Emanuel. `CHANGELOG.md` is
retained verbatim as that project's history. Contributions made in this fork are
MIT. Read [`LICENSE`](../LICENSE) in full before depending on this project.

## CI: what is still red, and why (2026-08-12 — historical; hosted CI has been fully green since f3fe87fc, 2026-08-18)

CI is telemetry; lefthook is the gate. That is a decision, not a description: a
job is kept only if it can fail, jobs that cannot pass are marked informational
in the open, and what remains red is written down here rather than absorbed.

Everything below was measured. The reference point is the last full run before
this pass — `jwiegley/obr` run `31576202757` (CI) and `31576202752` (Doctor), both
at `3641b837`, 2026-08-12T07:57:54Z. Local figures come from this working tree.

### What the first-ever executions found

Unblocking `Check` let eight jobs run that GitHub had been skipping, and each
fix since has revealed the next step behind it. Three suites executed for the
first time in the history of *either* repository, and all three fail. They are
newly **visible**, not newly broken, and nothing here has a baseline claiming it
ever passed:

| suite | first result | tracked as |
| --- | --- | --- |
| Quick E2E (`scripts/e2e.sh`, six binaries) | 0 passed, 6 failed, 1080 s — ran to completion, so not a timeout artifact | `obr-jmv` |
| Sync safety witness (`yyxo`, PC-1 / PC-RECOVERY) | 8 operations, 3 violations | `obr-i7z` |
| Crash-injection sync matrix (`e2e_sync_failure_injection`) | fails, undiagnosed | `obr-8ng` |

None is diagnosed here. Diagnosing them is a fresh piece of work with an
unknown floor — the honest position is that this pass restored *visibility*
into three suites that were asserting nothing, and the findings are filed
rather than absorbed.

### Jobs

| Job | At `3641b837` | After this pass |
|---|---|---|
| **Check** | green, 6 m 30 s | unchanged |
| **Security Audit** | green, 18 s | unchanged in effect. The `continue-on-error: true` copy of `cargo audit --deny warnings` was deleted; it duplicated the *blocking* scan in `audit.yml` and made the advisory posture look enforced twice. `--deny yanked` stays and still blocks. |
| **Build** (5-way matrix) | green, 5 m 37 s – 13 m 40 s | unchanged |
| **Benchmarks** | green, 10 m 02 s | unchanged, but marginal: the same job was `cancelled` at 20 m 17 s against `timeout-minutes: 20` one commit earlier (`86adde87`). |
| **Test Suite** | `cancelled` at 45 m 00 s against a 40-minute cap; `failure` at 22 m 35 s on the run before | should go green, and **asserts almost nothing** — see below |
| **Reliability Gates** | red, 9 m 46 s | still red, on a smaller set — see below |
| **Audit Guardrails** | red, 35 s | first real verdict pending — see below |
| **Doctor Health Check** | red, 6 m 41 s | the *filed* failure is fixed; the job is still expected to be red — see below |
| **Quick E2E** | red, wearing a `cancelled` badge | untouched; still red |
| **Version Audit** | green, 10 s | **deleted.** It could not fail: the `else` branch of its `--version` check printed a warning and exited 0. |
| **Code Coverage** | `skipped` | **deleted.** Unreachable behind `needs: test`, which had never produced a verdict, and `continue-on-error: true` on its only computing step. |
| **Nightly Full Test Suite** | did not exist | unknown; will produce the first complete verdict |

**Test Suite** is now the unit tier (`--lib --bins`) plus doctests, at
`timeout-minutes: 25`. Both unit steps are informational, in the
`doctor.yml:76-77` idiom, because the repository already says that tier is not
green: `scripts/unit-gate.sh:6-11` and `ci/baseline/README.md` record two
baseline failures and four doctor tests that abort the harness process outright.
The honest reading is that the job now asserts very little. `cargo test --doc`
is its only hard-failing step and it is green by construction — all ten
doctests under `src/` are `#[ignore]`d, so it reports
`0 passed; 0 failed; 10 ignored`. What survives is a build assertion: the crate
and its rustdoc must compile. The real gate for this tier is lefthook's `unit-gate` no-NEW-failures
comparison. One loose end: the step comment cites
`ci/baseline/unit-failures-baseline-Darwin.txt` while the job runs on
`ubuntu-latest`, and no Linux inventory exists. Nothing breaks today because CI
does not run `unit-gate.sh`; wiring it in there would demand a fully green Linux
run.

**Reliability Gates** failed at step 5, `cargo test --test workspace_failure_replay`,
with `157 passed; 6 failed`; steps 6-8 were reported `skipped`, so three of its
four witnesses have never executed once. The fixture restore fixes three of those
six failures — `test_isolated_workspace_failure_fixture_preserves_custom_metadata_targets`,
`..._preserves_sidecars_and_recovery_debris` and
`workspace_failure_replay_doctor_reliability_audit_matches_fixture_posture` all
pass locally now. The other three are the schema-v4 residual below, so step 5
stays red. What changes is that `if: always()` on steps 6-8 lets the
crash-injection matrix, the long-lived stress and the concurrency stress report
for the first time; whether they pass is genuinely unknown, because there is no
prior verdict to compare against.

**Audit Guardrails** has never run a single gate. It failed at 35 s in step 5,
`cargo install cargo-insta --locked`, and steps 6-10 were skipped in every run
(filed as `obr-ptk`). Installing the tool from a prebuilt binary should get past
it, and the snapshot gate should then pass — `cargo test --test snapshots` is
`261 passed; 3 failed` locally, all three of them the local-only failures noted
below, and there is no `*.snap.new` anywhere in the tree. The forced-cycle-close
audit, the sync-safety witness and the concurrency witness have still never
executed, and their outcome is unknown.

**Doctor Health Check** failed at step 10 with
`[FAIL] db_missing_with_jsonl: post_repair stage failed` and
`Summary: pass=12 fail=1 skipped=0 of 65` (filed as `obr-qnb`). **That failure is
fixed by the database-family gate**, measured both ways through the fixture
harness itself: the pre-gate binary refuses with exit 4 `refused_unsafe`,
`gate=sync.merge_pending`, "could not prove that no sync merge is pending"; the
patched binary passes the fixture. The job is nevertheless still expected to be
red. `run_all.sh` stops at the first failure — the `pass=12` in that summary is
twelve fixtures that ran *before* it, not a total — and the separate
"12 of 65 still failing" residual above is unchanged, so the job will simply
advance to the next failing fixture. *Which* fixture is unmeasured, and cannot be
measured here: `run_all.sh` does not run on macOS at all, dying at the third
fixture on GNU `stat -c`. Thirteen of the sixty-five shell out to GNU-only
`stat -c`, `date -d`, `touch -d` or `sed -i`.

**Quick E2E** is red and has been reading as `cancelled`. Step 5,
`Run quick E2E tests`, was cancelled at 10 m 17 s and at 10 m 16 s in two
consecutive runs against `timeout-minutes: 10`. That is the cap firing, not a
concurrency cancel. Untouched by this pass: either the budget or the scope has to
change, and neither was decided.

**Nightly Full Test Suite** is new. The full `--all-features` suite has never
once run to completion here: it failed at 22 m 35 s in one run and was cancelled
at 45 m 00 s against a 40-minute cap in the next. (The 22-minute failure's log
has since expired and its cause could not be re-read; runner disk exhaustion was
the working hypothesis behind the reclaim step.) It now runs on a daily cron,
outside `ci.yml`'s `cancel-in-progress` group, with 120 minutes, a disk-reclaim
step, `cache-targets: false` and `CARGO_PROFILE_TEST_DEBUG: "0"`. Whether that is
enough disk is unmeasured on the runner side; the first nightly run is the
experiment.

**Update 2026-08-18:** the experiment ran. The nightly completes in ~62 minutes
under the 120-minute cap and reports a verdict (red, as the committed full-suite
inventory predicts) instead of dying at the ceiling.

### Tests

**`workspace_failure_replay`: 3 of its 163 tests still fail.** The binary reports
`157 passed; 6 failed` locally; the other three failures are the local-only
`dataset_registry` ones described next, which skip on CI. The three that are real
are `manifest_expectations_hold_on_fresh_copies`,
`core_read_surfaces_match_expected_posture` and
`core_write_surfaces_match_expected_posture`, for a product-side reason that
predates the rename. Each of the three iterates the fixture catalog and panics at
the first fixture, `corrupt_db_text`, whose surfaces exit 2 `SCHEMA_MISMATCH`:
"ordinary commands never migrate an existing tracker database; run
`obr doctor migrate-schema plan` … Schema version mismatch: expected 17, found 4".
The same wall stands behind the rest — nine of the ten fixture databases are
`user_version = 4` against `CURRENT_SCHEMA_VERSION = 17`
(`src/storage/schema.rs:11`), and `reviewed_schema_migration_required`
(`src/main.rs:1194`) refuses to auto-migrate. (The tenth,
`interrupted_rebuild_leftovers`, deliberately plants a 58-byte text file where
the database belongs; that is the anomaly it models, not schema drift.)
`CURRENT_SCHEMA_VERSION` was already
17 at the merge base `d1fb0d3a` and is 17 on `upstream/main`, whose fixture
database is the byte-identical blob `0945a06b` at `user_version = 4`, so these
are red upstream too. Fixing them means either migrating the committed fixture
databases to schema 17 or teaching the replay harness to run
`doctor migrate-schema` first — a decision, not an oversight.

**Three `dataset_registry` failures are local-only.**
`test_isolated_dataset_copy`, `test_isolated_from_override` and
`test_run_with_integrity` all fail here on `isolated.metadata.issue_count > 0`,
because `issue_count` comes from `count_jsonl_lines(.obr/issues.jsonl)` and this
workspace has no such file — obr's own tracked surface is `docs/PLAN.org`. On a
CI checkout `.obr/` is absent, `is_available` returns false, and the three skip
("obr dataset not available"). They are not a regression and not a CI failure;
they simply make every local integration run that links `tests/common` report
three failures.

**The test amplifier.** `tests/common` holds 158 `#[test]` functions and 105
binaries under `tests/` declare `mod common`, so 16,590 of the 18,585 `#[test]`
executions in the integration tier — 89% — are the same 158 tests recompiled and
re-run. That is the dominant term in the suite's wall time, and the most likely
driver of the runner's disk usage. Not fixed; named here so nobody re-derives it.

### Accepted costs of the database-family gate

Six gates used to classify a workspace by whether one file existed.
`database_family_is_absent_at` now asks about the whole family. The prices, each
measured in its own fresh workspace so no earlier command could have rebuilt the
database:

**A read-only command can now rebuild the database.** With the whole family
absent, `obr list` — a read-only command — exits 0 and creates a 319,488-byte
database from the export, and `obr doctor --repair` goes from exit 4
(`RefusedUnsafe`) to exit 0. This is aligned with the product direction (`.obr`
is an ignored cache; the Org file is truth), and it is what unbricks
`db_missing_with_jsonl`. The cost: if the database ever held truth the export did
not, and the entire family was removed, obr now silently rebuilds from the staler
export and exits 0 where it used to fail loudly. Nothing can detect that
afterwards — the evidence is gone.

**A long-but-spent WAL refuses forever.** `len > 32` is sufficient for safety but
not necessary for liveness. SQLite's `wal_autocheckpoint` is PASSIVE and leaves
the WAL at full length with zero un-checkpointed frames; only `Drop`'s
`wal_checkpoint(TRUNCATE)` shrinks it, and that fires only on a normal exit with
`mutation_count > 0` (`src/storage/sqlite.rs:18705-18719`, whose own comment names
the gap). An abnormal exit can therefore leave a long, already-spent WAL that the
gate refuses permanently. Measured against a 935,272-byte WAL carrying real
committed frames: `list` 6, `doctor` 1, `doctor --repair` 4, `create` 6,
`--no-db create` 6, `init` 6, with the WAL byte-identical after every one.

The remedy is `rm .obr/obr.db-wal`, and it is **not** named in the refusal, which
says only "Pending sync-merge state is unknown because the authorized database is
missing while its sidecars still hold data".

There is exactly one in-tool escape, and it is silent and destructive.
`obr sync --merge` is not stopped by this gate: measured on the same fixture it
exits 6 with "merge.base.jsonl is missing and the database differs from JSONL" —
but only *after* materialising a 319,488-byte database and truncating the WAL
from 935,272 bytes to 32. The committed frames are gone, and the command that
destroyed them reports failure. This behaviour predates the gate and is unchanged
by it; it is recorded here so that the refusal is never described as a dead end
with nothing behind it.

**Refusing on a `-journal` is conservatism, not necessity.** With the main file
gone, a rollback journal can restore nothing. obr forces `journal_mode = WAL`
(`src/storage/schema.rs:1037-1043`), so a `-journal` should only ever appear from
an interrupted first open or a doctor backup. Refusing on a non-empty one is a
deliberate choice to fail closed.

**The zero-length-main-database brick is NOT fixed.** When the main file exists at
zero length, the write lock has already bound the database,
`bind_database_inode_for_mutation` returns `Ok(false)`, and the family predicate
is never consulted. Measured identical before and after: `list` 6, `doctor` 1,
`doctor --repair` 4, `create` 6, `--no-db create` 6, `init` 6. The
`symlink_metadata(main).is_ok()` guard inside the predicate is unreachable
defence-in-depth on both call paths, not the handler for this case.

### Snapshot masking

`normalize_output`'s `ID_RE = \b[a-zA-Z0-9_-]+-[a-z0-9]{3,}\b`
(`tests/snapshots/mod.rs:48-49`) destroys 18 real tokens in `obr --help` —
`Agent-first`, `Org-mode`, `append-only`, `machine-readable`, `issue-specific`,
`robot-docs`, `in-tool`, `per-command`, `vcs-status`, `auto-discover`,
`no-daemon`, `no-auto-flush`, `no-auto-import`, `allow-stale`, `lock-timeout`,
`write-lock`, `JSONL-only`, `no-color`. It would equally redact
`beads-anything`, `br-anything` and `bd-anything`, so it is blind precisely to
the hyphenated pre-rename tokens the rename exists to eliminate, and the help
golden cannot pin two real subcommand names (`vcs-status`, `robot-docs`).
Deliberately not fixed: 15 of the 78 committed goldens carry `ID-REDACTED` and
would all need re-baselining, and the product is guarded independently
by `tests/vocabulary_lint.rs::every_subcommand_help_is_free_of_legacy_vocabulary`,
which walks the root help, forbids `\bbr(\s|$)`, `\.beads`, `beads\.db`, `\bbd-`,
`BEADS_`/`BD_`/`BR_`, is unmaskable by construction, runs in the lefthook gate, and
passes `7 passed; 0 failed; 2 ignored` under both default and all features.

### Fixture prose that contradicts its own code

`tests/fixtures/workspace_failures/orphaned_lock_file/fixture.json:4` describes
the anomaly as `.obr.lock`. The only detector, `src/health.rs:494-495`, joins the
literal `.beads.lock`, which also appears at `src/health.rs:293` (the
operator-facing message) and `:931` (its unit test). The `190d715d` prose sweep
renamed the description and not the code, and `.obr.lock` appears nowhere else in
the tree. The payload file is deliberately restored as `.beads.lock`, because
naming it anything else silently stops the fixture from modelling its own
anomaly. Either the prose or the three code sites should move; nothing decided.

### Local tooling that does not match CI

**`cargo clippy` runs the wrong compiler.** Inside the pinned nightly,
`nix shell "github:nix-community/fenix#complete.toolchain" --command cargo clippy --version`
reports **clippy 0.1.95** — the space form resolves through `$CARGO_HOME/bin`
rustup shims ahead of anything on `PATH`. Invoking the binary directly,
`cargo-clippy --version`, reports **clippy 0.1.99 (3d6c19bb9a 2026-08-11)**, which
is what CI runs. `scripts/lint-gate.sh` already does the latter and documents why;
it exits 0 on this tree ("no new clippy findings vs baseline"). Anything that
shells `cargo clippy` or `cargo fmt` by name is measuring a stale toolchain.

**`unit-gate.sh` inherits a stray workspace.** On Darwin the gate pins
`TMPDIR` under `/private/tmp` (`scripts/unit-gate.sh:79-82`) to avoid the
symlinked `/var`. Any `.obr/` directory left in `/private/tmp` therefore becomes
an ancestor workspace of every test temp dir, and
`util::tests::isolated_temp_root_is_obr_free_and_usable` fails with "temp dir …
resolved a obr workspace; TMPDIR is inside one". Flip-tested: under
`/private/tmp/obr-gate.X` it fails, under `/private/var/tmp/obr-gate.X` it passes.
The fix is to delete the stray `/private/tmp/.obr`, not to touch the test.

**Parts of the harness are GNU-only.**
`cli::commands::doctor::tests::test_fix_recovery_artifacts_aged_is_idempotent_no_op_on_second_call`
shells `touch -d "60 days ago"` (`src/cli/commands/doctor.rs:25248`); BSD `touch`
answers "out of range or illegal time specification". Thirteen of the sixty-five
doctor fixtures reach for the same GNU-only forms (`stat -c`, `date -d`,
`touch -d`, `sed -i`), which is why `tests/doctor_fixtures/run_all.sh` cannot
complete on macOS. That test name is not in
`ci/baseline/unit-failures-baseline-Darwin.txt`, so `unit-gate.sh` reports it as
a NEW failure on any machine without GNU coreutils first on `PATH`.

**Tests in the sync/authority paths need a canonical `TMPDIR`.** The default macOS
`/var/folders/…` resolves through a symlinked `/var` and the authority code
correctly refuses it. Use something under `/private/var/tmp` when running them by
hand.

**`.github/action-pin-upstreams.jsonl` was not refreshed.**
`tests/workflow_action_pins.rs` passes (`16 passed; 0 failed`) because its
inventory check reads `action-pins.jsonl` only, but
`scripts/audit-workflow-action-pins.sh` also consumes the upstreams file and now
reports 12 of 38 pins as `disallowed_downgrade` and 7 as `update_available`. The
audit script is advisory and nothing gates on it; refreshing it is unfinished
work, not a defect.

## CI: the picture after the fresh-workspace pass (2026-08-13 — historical; superseded by the port and the green shakeout of 2026-08-18)

Measured on run for `d6d4ca8d`. Six jobs that were red or unverdicted above are
now green, and what remains has been diagnosed rather than merely observed.

| Job | Then | Now |
|---|---|---|
| **Test Suite** | never produced a verdict | **green** — 2827 passed, 0 failed, 9 ignored. It first passed every step and still reported `cancelled`, finishing in 25 m 01 s against a 25-minute cap; the cap is now 40, because the time is two whole-graph compiles (12 m 06 s `--all-features`, 10 m 16 s `--no-default-features`) and only 42 s of testing. |
| **Audit Guardrails** | red | **green**, including the sync safety witness and the snapshot-freshness gate |
| **Security Audit** / **Cargo Audit** | red on RUSTSEC-2026-0253 | **green** |
| **Quick E2E** | 0 of 6 in 1080 s | **5 of 6**; `e2e_errors` is green, only `e2e_basic_lifecycle` fails |
| **Reliability Gates** | red, one step, rest unrun | 3 of 4 steps green (`e2e_sync_failure_injection` 179/0); step 1 is `workspace_failure_replay` 160/3, tracked as `obr-zzd` |
| **Doctor Health Check** | halted at fixture 18 of 65 | reaches **24**, now stopping at `gitignore_bare_pattern` |

### Not one cause: the gate has two exits, and they strand different tests

An earlier revision of this section claimed a single upstream gate accounted
for six failing tests, and that it should be left alone. Both halves were
wrong, and the correction is worth stating plainly because the wrong version
was load-bearing for a "leave it" decision.

The gate (`src/storage/sqlite.rs`, reached from `src/main.rs` at startup and
from doctor's own gate) has **two** failure exits:

1. **The `None` arm** — *"the database schema is missing or unreadable"* —
   fires when the main file carries no SQLite magic. This was a real product
   defect and is fixed (`fa24dc70`). It was a total brick: `obr list` exited 6,
   `obr create` 2 or 6, `obr doctor --repair` 4 `refused_unsafe`, and doctor's
   own report named the anomaly `database_not_sqlite` while advertising a
   rebuild it then refused to perform. The only recovery was deleting the file
   by hand. The fix mirrors the argument the fork already makes one branch
   above in `database_family_is_absent` (`b5c5155cc`, obr-qnb): a merge receipt
   lives in a table inside the SQLite database, so if the main file holds no
   database and no sidecar can carry committed pages, no member of the family
   can hold one.

2. **The `Some(found)` arm** — SchemaMismatch, routed to *"run `obr doctor
   migrate-schema plan`"* — fires on a **valid** database whose `user_version`
   is older than `CURRENT_SCHEMA_VERSION`. That is correct behavior, not a
   defect.

The three `workspace_failure_replay` failures are arm 2, not arm 1 — but the
first version of this paragraph got the mechanism backwards and is worth
correcting in place. The harness reads the **undotted** `obr/` payload
(`copy_workspace_failure_fixture_root`, `tests/common/dataset_registry.rs:571-583`,
which prefers `obr/` over `.obr/`); the `root.join(".obr")` at `:251` is the
destination temp directory. So `corrupt_db_text` really was loading the 22-byte
"not a sqlite database" file its manifest describes. What nullified it was a
650992-byte `obr.db-wal` shipped beside it, from which SQLite reconstitutes a
full `user_version` 4 database — working-tree debris force-added by
`e6206750`, mentioned by no manifest. Deleted in `a872fa8b`, along with the
same defect in `interrupted_rebuild_leftovers` and an orphaned WAL in
`metadata_custom_paths`.

All three replay tests now advance past those fixtures and stop at
`db_jsonl_disagreement`, whose database genuinely is the payload and genuinely
is at schema 4 — as is every other readable database in the corpus, against a
current 17. That is `obr-gqh`, and it is the last thing between the replay
corpus and a verdict. The missing guard that let the drift go unnoticed is
`obr-u01`.

The `None`-arm predicate checks length before magic, deliberately. A SQLite
database begins with a 100-byte header, so a shorter file provably is not one,
including the zero-length file a crash or an ENOSPC truncation leaves behind —
the most common real corruption there is, and one a magic-bytes read cannot
classify at all, because the read simply fails and that is indistinguishable
from a permission error. Verified across sizes 0/10/22/99/150 bytes; the
protected invariant still holds, with a 32-byte WAL (header, zero frames)
rescued and 33 bytes refused.

Two soundness questions about that arm were raised and are now settled by
measurement rather than by argument:

**Can the fsqlite namespace sidecars hold a receipt?** No.
`database_sidecars_may_hold_committed_bytes_at` iterates
`CLASSIC_SIDECAR_SUFFIXES` (`-wal`, `-shm`, `-journal`) and not the
`-fsqlite-ns-gate` / `-fsqlite-ns-use` files fsqlite keeps beside every
database, so the arm concludes "no member of the family can hold a receipt"
without inspecting them. That is correct, and the split is deliberate —
`config::db_sidecar_suffixes()` chains `FSQLITE_NAMESPACE_SIDECAR_SUFFIXES`
separately for the cases that *do* want every engine artifact. Measured: after
creating 40 issues and flushing, `obr.db` grew to 356 352 bytes while
`-fsqlite-ns-use` stayed at exactly 40 bytes (a fixed record with magic
`FSQLNS01`) and `-fsqlite-ns-gate` stayed at 0 (a lock inode). They are
namespace-admission artifacts, not page storage, and a merge receipt lives in
the `metadata` table inside the database.

**Can the 100-byte length test misclassify a real database?** No. SQLite's
header alone is 100 bytes and its minimum page is larger, so no SQLite database
can occupy the 1–99 byte window; measured, twelve fresh `obr init` runs each
produced exactly 339 968 bytes. The one file that *is* in the window at zero
bytes holds nothing, which is precisely the property the gate needs. The
theoretical hazard — observing a database mid-creation, before its header is
written — cannot bite either: the authoritative call site
(`inspect_pending_sync_merge_under_authority`) runs under the database-family
write lock, the doctor call site is advisory and only ever answers "no pending
merge", and the MCP call site refuses rather than rebuilds.

One caller is knowingly left worse, and the first version of this paragraph
described it wrongly. `src/mcp/mod.rs` reaches the same gate and its post-gate
open has no JSONL recovery path. It does **not** simply "surface a storage-open
error where it used to surface the gate's refusal", and the two outcomes are
**not** both failures. Measured:

- **0 bytes** — the open *succeeds*. SQLite treats an empty file as a new
  database and applies the schema, so MCP answers reads with an empty tracker
  while the issues are still in `PLAN.org`. That is a confident wrong answer,
  not a failure, and zero-length is precisely the case the fix singles out as
  the most common real corruption.
- **10 / 22 / 99 / 150 bytes** — the open fails, but through `to_mcp`, a flat
  string, where the mutation path used to return a structured
  `SYNC_MERGE_PENDING_UNKNOWN` envelope that `src/mcp/mod.rs:765` already
  treats as a contract.

The CLI is unaffected: with the surface present, `obr list` on a zeroed
database recovers the issue from `PLAN.org` and leaves the surface intact.
MCP is behind the optional `mcp` feature, so the default binary has no MCP
surface at all, which caps the blast radius. Tracked as `obr-zw8`, with the
harness to use spelled out. No MCP test covers it.

### Doctor fixtures: 52 of 65 locally, and why the two counts differ

`FAIL_FAST` defaults to 1, so CI reports the first failure and stops — 22 of 65
on Linux. A local `FAIL_FAST=0` run on macOS reports 52 pass / 13 fail. The
lists are not comparable: several macOS-local failures (`touch -d`, permission
bits that do not bind for this uid, a different `sqlite3`) pass on Linux, and
`tests/doctor_fixtures/doctor_runs_dir_growth/corrupt.sh` still uses GNU
`touch -d`, which BSD rejects, so that fixture cannot run on macOS without a
shim. Neither number is the other's baseline.

`empty_database_with_jsonl` ("repaired DB suspiciously small (0 bytes)") was
CI's next frontier and is now fixed — it was the same fail-closed gate, on a
zero-byte database. CI now stops at `gitignore_bare_pattern` instead.

The macOS number moved for a second reason worth recording: an earlier local
figure of 52/65 was measured with a **stale debug binary** predating the `--db`
surface fix. Rebuilt, and with the `stat -c` portability fix, the same host
reports 60/65 with no GNU tools and no shim on PATH.

### Flaky, not baselined

Two tests, not one, fail intermittently under the full parallel unit run and
pass in isolation:
`sync::tests::database_family_lock_serializes_external_db_across_workspaces_with_one_timeout_budget`
(a 250 ms wall-clock window around a 400 ms lock budget) and
`cli::commands::vcs::tests::finite_oversized_output_is_rejected_by_hard_capture_cap`.
Each was measured blocking a commit twice and then passing three full runs on
identical code, with a stashed-change control run clean — measured 3 passes and 2
failures on identical code, 5 of 5 when run alone. When it trips it blocks any
commit, because `scripts/unit-gate.sh` correctly reports it as a NEW failure.
Neither is added to `ci/baseline/`: they pass most of the time, so a baseline
entry would read as an expected failure and mask a real regression in the
behavior they defend. Tracked as `obr-mvp`.

### A `git` operation on the surface makes the merge anchor look stale

Any git command that rewrites `docs/PLAN.org` outside obr — `stash pop`,
`checkout`, `pull`, a rebase — leaves it newer than `.obr/merge.base.jsonl`,
and `obr doctor` then warns that "3-way merges will diff against stale state",
which blocks the pre-commit hook. This is the detector doing its job, not a
defect. The remedy is one `obr sync --flush-only`, which republishes the anchor.

## Dead-code assessment — the seven-pass review's F-cluster (2026-08-08)

The heavy review flagged four modules for deletion. Researched against the
upstream merge-base (`d1fb0d3a`), the decision is **keep all four**, because
every one is upstream-inherited and this fork's maintenance model is a
re-port, not a merge (see the sync note above): deleting inert *upstream*
code does not shrink the fork's maintenance surface — it *adds* a divergence
that must be re-applied on every future re-port, for no production benefit.

- **`tests/conformance*` (~20,445 lines) — NOT dead; the review mischaracterized
  it.** It is gated by `skip_if_no_bd!()` (222 sites), so it skips cleanly when
  the Go `bd` binary is absent — which is why it causes no local failures — and
  `.github/workflows/conformance.yml` deliberately builds Go beads to run it in
  CI. It guards the JSONL engine semantics (dependency logic, ready-set, dedup,
  import/export) that obr inherits *unchanged*; the Org-surface divergences are
  the ~31 `#[ignore]`d tests. Deleting it would remove real regression coverage
  of the shared core. **Keep.**
- **`src/write_combining.rs` (2,918 lines)** — inert upstream subsystem
  ("deliberately does not route"), exercised only by `tests/bench_contention_replay.rs`.
  No production consumer, no runtime cost, no active harm. **Keep** (deleting is
  pure re-port divergence).
- **`src/storage/events.rs` (973 lines)** — the **file is live**: `get_events`/
  `get_all_events` have 34 call sites on the production read path. Only the 13
  `insert_*_event` write helpers are unwired (the `obr audit` command records via
  `append_entry` to JSONL, not this table). They are upstream code and a plausible
  future wiring point on re-port. **Keep** (surgical removal would be re-port
  divergence for an internal, non-misleading surface).
- **`src/format/rich.rs` (540 lines)** — production-dead (its public types have no
  non-test callers), upstream, self-test-only. Inert. **Keep** on the same basis.

The fork-authored duplication the ponytail pass noted (the private test harness
in `tests/legacy_compat.rs`, a `warn_*` near-duplicate) is **live, working**
code, not dead code; refactoring it for DRYness would risk regressions in
passing tests for no production benefit, so it is left as-is.

The distinguishing rule applied throughout the port: delete upstream code only
when it carries *active* harm — security/network surface (the self-updater),
a misleading product contract (the agent-swarm docs), or bloat (the committed
perf captures). Inert upstream code is kept, because for a re-porting fork the
cheapest maintenance surface is the one closest to upstream.

## Before the first release (deferred, not defects)

These do not affect building or using obr from source; they matter only when a
release is first cut, and are best done at that moment rather than pre-emptively.

- **`cargo publish` token on the command line.** `.github/workflows/release.yml`
  passes `--token ${{ secrets.CRATES_IO_TOKEN }}`, which puts the token in the
  process argument list for the duration of the publish. On a hosted ephemeral
  runner the only plausible reader is an already-compromised build-time
  dependency, and the publish step is dormant in this unpublished fork — so it
  is a defense-in-depth item, not a live hole. Before enabling crates.io
  publishing, drop `--token` and set `CARGO_REGISTRY_TOKEN` in the step's `env:`
  instead (cargo reads it natively).
- **Packaging checksums** are deliberate all-zero sentinels until a first
  release exists (documented at each manifest); the update-manifests workflow
  rewrites them on release.
- **`vars.MINISIGN_PUBLIC_KEY`** must be set on the repository for the release
  notes' signature-verification instructions to be meaningful; unset, that
  block renders an empty key.
- **Build metadata orders inconsistently.** The version is `0.2.22+1`: the
  upstream `beads_rust` release the fork tracks, plus the fork generation since
  it, with the generation resetting to 1 on each upstream sync. Semver build
  metadata is the only legal home for that fourth component — `0.2.22.1` is not
  a version Cargo will parse (`cargo metadata` fails with "unexpected character
  '.' after patch version number"). What it costs is a single, portable answer
  to "is `0.2.22+2` newer than `0.2.22+1`?":

  | Who | Answer | Why |
  |---|---|---|
  | The semver **spec**, §10 | equal | Build metadata **MUST be ignored when determining version precedence**. |
  | `semver::VersionReq`, and the crates.io resolver on top of it | equal | Requirement matching is spec-faithful; `VersionReq` discards metadata at parse time. |
  | `semver::Version::cmp_precedence` | equal | The crate's explicit spec-faithful entry point: compares `(major, minor, patch, pre)` only. |
  | `semver::Version`'s `Ord`/`==`/`<` | **greater** | `Version` derives `Ord` with `build` as its last field. Numeric identifiers compare numerically, so `+1 < +2 < +10` — no ordering trap at generation 10. |
  | Homebrew `Version`, pacman `vercmp` | **greater** | Both order the trailing component. |

  One code path depends on the crate ordering it, deliberately: `obr doctor`'s
  `binary_version` check warns when the tree's `Cargo.toml` declares a newer
  fork generation than the running binary, which differs from it *only* in
  build metadata. Rewriting that to `VersionReq::matches` or `cmp_precedence`
  would silently stop warning forever; `fork_generation_bump_is_detected` in
  `src/cli/commands/doctor.rs` pins it.

  What the spec-faithful half blocks is publication. crates.io resolves with
  `VersionReq`, so a `0.2.22+2` published after `0.2.22+1` would not be selected
  as an upgrade of it. Before enabling the dormant `update-crates-io` job, or
  adding any "a newer obr exists" check that asks a registry rather than the
  local tree, the fork generation has to move somewhere the *spec* orders: a
  patch bump (`0.2.23`) or a real pre-release identifier.
  The manifest-lag assertion in `tests/package_manifests.rs` stays true for a
  simpler reason: the `semver` crate's `Ord` *does* order build metadata, so an
  ahead manifest trips the `Greater` arm directly. That test's explicit
  fork-generation comparison is a deliberate fallback for a future
  spec-faithful comparator, not the mechanism in use today.

  The practical rule: never rely on build-metadata ordering *across* tools.

## The version's two spellings

`0.2.22+1` reaches release artifacts as two different strings, and the
difference is load-bearing:

| Where | Spelling | Why |
|---|---|---|
| `Cargo.toml`, `obr --version`, manifest `version` fields, git tag | `0.2.22+1` | The true version. Git refnames accept `+` (`git check-ref-format` passes), and so does a URL path segment — `github.com/…/releases/download/jdk-17.0.9+9.1/…` returns 200 today. |
| Release asset **file names** | `0.2.22.1` | GitHub's asset-upload API takes the asset name as a URL *query* parameter, where a literal `+` decodes to a space and is stored back as `.` (github/rest-api-description#2968). Adoptium, which tags `jdk-17.0.9+9.1`, publishes 0 of 7,352 assets containing a `+`. |

A convenient side effect of dropping the `obr.` prefix: the flattened spelling
is now a plain four-component version. Assets read
`obr-0.2.22.1-linux_amd64.tar.gz`, which falls out of the `+`→`.` rewrite and is
spelled by hand nowhere.

`release.yml` therefore flattens `+` to `.` itself, into exactly the character
GitHub would have produced. Nothing uploads a `+`, so nothing can be rewritten,
and the name the workflow generates, uploads, checksums, signs, and later
reconstructs is one string. `install.sh`'s `release_asset_version` and
`release_download_tag` are the same split on the client side, and
`update-package-manifests.yml` carries both as separate step outputs.

One consequence: Scoop's `autoupdate` block cannot express the flattening with
`$version` alone, so `packaging/scoop/obr.json` splits the tag with a named
capture (`$matchCore`, `$matchFork`). That template is documented but
unexercised — this repository's own `update-package-manifests.yml` rewrites the
manifest on every release and is the authority.
