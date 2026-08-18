# Decision register

**Provenance.** Distilled 2026-08-18 from the upgrade research record under `docs/research/upgrade/`, which was deleted once the port it planned had landed.

**Scope.** Only decisions still in force on this tree. The port that occasioned them — its phases, gates, baselines and commit series — is finished and is not described here. The tree sits on beads_rust v0.3.2 as obr `0.3.2+1`.

**Companion registers.** `docs/RESIDUALS.md` records what is broken, carried, or deliberately left, with the measured figures. `docs/PLAN.org` is the issue surface. `ci/baseline/` holds the inventories `scripts/unit-gate.sh` and `scripts/lint-gate.sh` compare against. Where a decision has a number attached, that number lives there rather than here.

---

## User rulings — 2026-08-07

Given directly by the user in a live session. Authoritative over everything below.

| ID | Ruling |
|---|---|
| **D-REPOTRACKER** | **This repository does not self-track.** `.beads/` was dropped entirely — "I'm not using Beads with this repository and I never plan to." No in-repo tracker data of any kind, from either project. The verified Org conversion machinery survives as product code; the fork's 549-issue corpus and upstream's 953-issue corpus live in git history only. Supersedes the earlier plan to carry upstream's tracker forward. |
| **D-SURFACE** | **The tracked human surface is `PLAN.org`; `.obr/` is a git-ignored per-machine cache.** — "obr maintains a `.obr` directory with a cached sqlite db that is ignored by Git, and a file called `doc/PLAN.org` (if there is a `doc/` directory, otherwise `./PLAN.org`) that it tracks in version control." The full rule is below. Supersedes the earlier design that kept `issues.org` inside the dot-directory. |
| **D-NOBRANCH** | **No secondary tracking branch, ever, and nothing invasive.** — "Beads was far too invasive, and I don't intend to replicate its behaviors here." obr never invokes git from a sync or mutation path; a test sentinel proves it, with the explicit read-only VCS probe as the positive control. The only git call sites are those read-only probes (`vcs-status`, the orphan scan), and every one of them goes through `hardened_git` in `src/cli/commands/vcs.rs`. Constructing a bare `Command::new("git")` is the bug that test exists to catch. |
| **D-DOCS** | **Every document describes obr, its intended usage, and accurate expectations.** Executed as a docs pass after the binary and crate rename, so the prose matches the built reality rather than the plan. Agent-swarm workflow content inherited from upstream was removed rather than renamed. |
| **D-RESIDUALS** | **Upstream-carried defects are documented, not fixed.** `docs/RESIDUALS.md` is that register. Fixes are queued as ordinary issues against a stable base; they are never folded into a port, where a fix and a port defect are indistinguishable. |
| **D-LICENSE** | **Upstream's LICENSE — MIT with its rider — stays untouched; contributions made in this fork are MIT.** Packaging license fields state MIT with a see-LICENSE rider reference. The rider is part of the license conditions and must accompany redistribution, which is why `packaging/aur/PKGBUILD` installs `LICENSE` verbatim rather than a plain MIT notice. Copyright for the upstream work remains with Jeffrey Emanuel. |
| **D-DISTRIBUTION** | **Keep, repoint, and fix the packaging estate.** `packaging/` manifests (Homebrew, Scoop, AUR) and `install.sh` are de-branded and pointed at `jwiegley/obr`; the release workflow uses `obr` asset names and no longer asks for a self-update feature. The self-updater itself stays deleted — see D-SELFUPDATE. |

---

## D-SURFACE — the tracked surface, as implemented

`.obr/` is a per-machine cache and nothing else: the SQLite database, config, metadata, history, recovery, locks, and merge artifacts. It is entirely invisible to git by way of a self-ignoring `.obr/.gitignore` whose whole content is `*`. Nothing under it is ever tracked.

The surface is `PLAN.org`, in the first existing directory of the ordered probe **`doc/`, then `docs/`, then the workspace root**. The highest-priority existing file wins; a shadowed lower-priority `PLAN.org` raises a one-shot, JSON-safe warning. The two are never merged.

Resolution, for read and write alike:

1. `metadata.json`'s `jsonl_export` set to a **non-default** value is an explicit override, resolved relative to `.obr/` exactly as before. This is what pinned-JSONL tests and power users rely on; external paths still obey U-EXTERNAL.
2. **Defaulted** — the value is `PLAN.org`, the legacy sentinel `issues.org`, or metadata is missing:
   - the computed surface exists → use it;
   - it does not, but an in-dir `.obr/issues.org`, `.obr/issues.jsonl`, or `.obr/beads.jsonl` does → keep using that artifact, warning once per process. Existing workspaces are never force-migrated;
   - neither exists → the computed location.
3. Both `doc/PLAN.org` and `./PLAN.org` present → `doc/` wins and the root file is reported as shadowed.

`init` creates `.obr/` with `obr.db`, `config.yaml`, `metadata.json`, and the self-ignoring `.gitignore`; seeds the surface at the computed location; and never creates `doc/`, only uses it when it is already there.

`PLAN.org` is deliberately **not** a workspace marker. The name is far too generic, and auto-creating `.obr/` beside an unrelated `PLAN.org` would be exactly the invasiveness D-NOBRANCH rules out. The documented fresh-clone flow is `obr init && obr sync --import-only --rebuild`; the `#+ISSUE_PREFIX:` file keyword written into the surface is authoritative over init's directory-derived prefix, so issue identity survives a clone that carries no `.obr/`.

The sync path allowlist admits the resolved default surface **without** `--allow-external-jsonl`, but narrowly: basename `PLAN.org` at the workspace root or its `doc/`, under the same symlink and canonicalization hardening applied to the workspace root itself. Everything else outside `.obr/` remains external and opt-in.

Doctor's gitignore checks are inverted relative to the pre-D-SURFACE model: `.obr/` **must** be invisible to git, and the surface **must not** be. The finding ids and fixture directory names were left at their older spellings because renaming them ripples through the capabilities registry and the exit-code and finding maps; `docs/RESIDUALS.md` records that the names now read close to the inverse of what they mean.

---

## Org surface semantics

| ID | Decision | Where it lives |
|---|---|---|
| **U1** | `agent_context` is carried as an `** Agent Context` level-2 child holding the stored JSON verbatim in a guarded `#+begin_src json` block. A drawer property is not viable: the sanitizer flattens newlines. | `src/sync/org_bridge.rs` |
| **U2** | **Description fidelity: nothing is silently dropped.** A body must reconstruct exactly. Implemented as sanitized Org text wherever parse-and-reconstruct is byte-exact — so lists, tables and src blocks stay native and hand-editable — falling back to a verbatim `#+begin_example` block for any body that is not (typically pasted code under a real list bullet, which drifted +2 spaces per import/flush cycle, observed unbounded on a real corpus). The originally adopted raw-slice route through org2jsonl `char_begin`/`char_end` is **not** what shipped. | `org_bridge.rs::body_to_emit` |
| **U3** | Labels live in a canonical `:LABELS:` drawer property holding a compact JSON array, authoritative over Org tags, which are read only when the property is absent. Rejecting colons outright was declined: it would break upstream's live `provides:<cap>` capability feature. | `src/sync/org_bridge.rs` |
| **U-DRAFT** | `Status::Draft` is the Org keyword `DRAFT`, carried in the TODO keyword set. | `ORG_TODO_KEYWORDS` |
| **U5** | The schema version is written as `:OBR_SCHEMA_VERSION:`, currently 17. It and its pre-rename spelling `:BEADS_SCHEMA_VERSION:` are reserved keys: emitted, but never re-surfaced as user properties on import. | `src/sync/org_bridge.rs`, `src/storage/schema.rs` |
| **U-EXTERNAL** | **External export paths stay JSONL-only.** `--allow-external-jsonl` and `OBR_JSONL` reject `.org`. This is the narrowest possible safety change: the escape hatch keeps the smallest surface it can. | `src/sync/path.rs` |
| **U-MERGEART** | Internal merge and ancillary artifacts stay JSONL — `merge.{base,left,right}.jsonl`, `sync_base.jsonl`, `deletions.jsonl`, `interactions.jsonl`, with the pre-rename `beads.{base,left,right}.jsonl` spellings still read. They are internal and never hand-edited. The workspace is mixed-format internally and Org-only at the human surface. | `src/config/mod.rs` |
| **U6** | **Org accepts a full re-export per mutation.** Incremental line-splicing was declined; upstream's auto-flush efficiency regression test stays pinned to JSONL. The consequence — the no-op-flush fast path never engaging for Org — is recorded in `docs/RESIDUALS.md`, not treated as a defect to chase. | `src/sync/mod.rs` |
| **Q-LINES** | A heading with no `:ID:` aborts the import, diagnosed by the heading's ordinal position and title. Recovering true line numbers would require a change to org2jsonl, which stays pinned to a published revision. | `src/sync/org_bridge.rs` |
| **U-TESTPOLICY** | Tests choose their format **per function**, never by branching on file existence at run time: pin to JSONL where the JSONL path is what is under test, convert to Org where the default path is, split the function where both are. | test estate |

`U4` — seeding `issues.org` inside the dot-directory — was superseded by D-SURFACE before it ever shipped.

---

## Naming, compatibility, and repository policy

| ID | Decision |
|---|---|
| **D-ARTIFACTS** | `.obr/` artifact names: `obr.db`; history under `.obr/history/`; recovery under `.obr/recovery/`, with the pre-rename `.br_recovery/` still honoured where it already exists. |
| **D-DOMAINSEP** | The branded hash domain separators keep their `beads-rust.*` spellings (`src/sync/mod.rs`). They never reach disk as user-visible text, and renaming them would invalidate every cached merge receipt. A documented residual, decided rather than overlooked. |
| **D-MONOREPO** | The dot-less workspace variant is `_obr`, with `_beads` read for compatibility — mirroring upstream's semantics for repositories that disallow dot-directories. |
| **D-PREFIX** | Tracker issue IDs carry the `obr-` prefix, re-prefixed from `beads_rust-*` through `sync --rename-prefix` after a verified dry run. |
| **D-SELFUPDATE** | `obr upgrade` and the `self_update` feature are deleted, not merely disabled. This is an unpublished personal fork; a binary that rewrites itself from a registry it does not publish to is a liability. The packaging estate around it survives under D-DISTRIBUTION. |
| **D-HOOKS** | lefthook, not `.githooks/`. Upstream's quick doctor pre-commit guard is folded in as a lefthook step rather than kept as a parallel hook mechanism. |
| **D-SHFMT** | Shell formatting is shfmt at its defaults (tabs), zero configuration, converging on touched files only through lefthook. `tests/doctor_fixtures/**` is excluded from the hook glob. |
| **D-AUDIT** | `.cargo/audit.toml` is upstream's file, kept — the fork's earlier deletion of it is not re-applied. **Amended since the original ruling:** advisory findings now *block*, in `.github/workflows/audit.yml`. Suppressions belong in `.cargo/audit.toml`, where they are reviewable, not in a non-blocking CI step where they are invisible. The duplicate `continue-on-error` copy in `ci.yml` was removed for asserting nothing while making the posture look enforced twice. |
| **D-SUPPLYCHAIN** | The SHA-pinned GitHub Actions machinery is adopted, with the maintenance commitment that implies. See `docs/CI_SUPPLY_CHAIN.md`. |
| **D-CHANGELOG** | `CHANGELOG.md` is kept verbatim as upstream's history and excluded from rename sweeps. It is that project's record, not this fork's changelog. |
| **D-PLUGIN** | Upstream's `.claude-plugin/`, `.claude/`, `skills/`, `.agent-mail.yaml`, `.ntm/`, `refactor/`, and `sample_beads_db_files/` are not carried. Adopting them would mean maintaining them de-branded; they remain recoverable from upstream history if ever wanted. |

---

## Upstream-carried defects — inventory of 2026-08-07

Verified during Phase 6 against beads_rust v0.2.22 (`d1fb0d3a`), the substrate the port started from. Under D-RESIDUALS, none of it was fixed by the port. The tree has since moved to v0.3.2 and the figures were re-measured: `docs/RESIDUALS.md` and `ci/baseline/` are the current record. This table is kept for the **classes** it names, not for counts.

| Class | Evidence at the time |
|---|---|
| `e2e_routing` write-lock timeouts and a routed-mutation deadlock | Branch and baseline failure name-sets identical; both `comm` diffs empty. |
| `init` into a pre-existing **empty** workspace directory fails — the pending-sync-merge guard runs before init dispatch, so the database is missing | Reproduced verbatim on the baseline binary. |
| Write-authority lock sidecars `.obr/.br-jsonl-write-<digest>.lock` absent from the sync path allowlist (introduced upstream) | In the committed baseline inventory, `e2e_sync_git_safety`. |
| **Born-broken tests** — assertions that never described anything the product does: an `--id` flag that never existed; a schema-4 fixture database against a schema-17 product; a never-implemented `persistent_advisory_inode` doctor classification | In the committed baseline inventory. |
| macOS `/var`-symlink `TMPDIR` class | Environment-dependent: measured 34 failures on the default `TMPDIR`, 0 on a clean one. |

**Resolved 2026-08-15 (obr-m6m)** — the `persistent_advisory_inode` half of the born-broken row. The classification was never implemented and is not going to be. The two assertions demanding it, `tests/doctor_fixtures/mcp_serve_stale_write_lock` and `tests/e2e_concurrency.rs`, now demand `probe_acquired_free`, the implemented answer for an old lock file with no live holder. This became checkable only once a read-only `obr doctor` stopped taking the workspace write lock: until then the probe collided with its own caller and answered `probe_would_block_live_holder` for every workspace, so the fixture's apparent pass had nothing to do with what it claimed to test. The other two born-broken items — the `--id` flag and the schema-4 fixture database — are untouched.

**Org-blind comparisons found during the port and deliberately left** — the no-op-flush anchor certification, always false on Org, which costs only the inode-preserving fast path; and `sync --witness` merkle-diffing JSONL lines against Org text, a spurious drift *report* that gates nothing — are recorded under "Org-format regressions relative to JSONL" in `docs/RESIDUALS.md`, together with the legacy `beads.jsonl` artifact that is read but never written.
