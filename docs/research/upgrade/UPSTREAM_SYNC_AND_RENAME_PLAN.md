# Upstream Sync & De-Beads Rename Plan

**Consumer:** an AI coding agent (Claude Code or equivalent) executing in a fresh session.
**Written:** 2026-08-06, by a Claude Code session that performed the reconnaissance below.
**Repo:** `/Users/johnw/src/obr` (branch `main`, remote `origin` = `git@github.com:jwiegley/obr.git`).
**Status of this plan:** reconnaissance verified; execution NOT started. No working-tree changes were made except adding the `upstream` remote (already fetched) and creating this file plus `docs/research/`.

---

## 0. Mission (the user's goals, in order)

1. **Upstream sync:** update `obr` to the latest `beads_rust` (`https://github.com/Dicklesworthstone/beads_rust`, currently v0.2.22), which has advanced ~5 months past the fork point, **while preserving all Org-mode related changes** made in this fork — the Org-mode format is the fork's reason to exist.
2. **De-beads rename:** stop using any beads naming or branding in the command structure or in created/managed artifacts. Use a `.obr` directory instead of `.beads`. The **only** permitted beads reference is crediting beads as the creative origin of the tool.

Deliver both on an isolated branch. `main` must not be touched; nothing is pushed. Landing is the user's decision (see §8).

## 1. Ground rules (non-negotiable)

- Never commit to or rewrite `main`. All work happens on branch **`org-forward-port`** created from `upstream/main`.
- Never push to any remote. Never force-anything.
- The fork's own issue-tracker data (`.beads/` contents, see §7.4) must survive the entire process.
- `org2jsonl` lives at `/Users/johnw/src/org2jsonl` (local, unpublished crate). Do not modify it without noting why; prefer pinning obr to a specific rev (see §7.1).
- After every code change: `cargo clippy --all-targets -- --deny warnings` (project convention, per CLAUDE.md).
- Use `nix develop` for the toolchain (nightly Rust via rust-overlay). Non-Nix fallback: respect `rust-toolchain.toml`.
- Gates use a **no-NEW-failures baseline** methodology (§7.3) — the pre-existing test estate is known-broken in places; do not chase pre-existing failures, and do not use them as an excuse either. Record the baseline before changing anything.

## 2. Lineage and context

- **beads** (Go, binary `bd`) — Steve Yegge's agent-first issue tracker (now `github.com/gastownhall/beads`). Original architecture: SQLite cache + JSONL-in-git as the durable/sync format. Upstream Go later moved to Dolt; that pivot is irrelevant here except as history.
- **beads_rust** (Rust, binary `br`) — Jeffrey Emanuel (Dicklesworthstone)'s Rust port of *classic* (SQLite+JSONL) beads. This is `upstream`.
- **obr** (this repo) — John Wiegley's fork of beads_rust, frozen at the Feb 2026 fork point, adding: **Org-mode storage format** (`issues.org` instead of `issues.jsonl`, via a new `src/sync/org_bridge.rs` + the local `org2jsonl` crate), the `br`→`obr` binary rename, BSD-3 relicense, and productization (lefthook, nix flake rework, fuzz targets, CI tightening).

A 24-agent deep-research dossier on the fork's current state exists at `docs/research/DOSSIER.md` (copied into this repo for durability; originally produced 2026-08-06). It contains a full architecture map, 32 ranked confirmed risks, and prior-art research. Consult it — especially §2.3 (sync engine), §5 (risks), and the two companion files `docs/research/map-sync.md` (sync engine map with merge-decision index) and `docs/research/gap-org2jsonl.md` (org2jsonl seam analysis). Trust but verify: it describes the FORK's code, not upstream's.

## 3. Verified facts (re-verify before relying; commands included)

| Fact | Value | Re-verify with |
|---|---|---|
| Merge-base (fork point) | `e805fe98`, 2026-02-15 | `git merge-base main upstream/main` |
| Local commits since fork | 20 (list in §3.1) | `git log --oneline e805fe98..main` |
| Upstream commits since fork | 1,843; HEAD = v0.2.22 (2026-08-06) | `git rev-list --count e805fe98..upstream/main` |
| Upstream diff vs fork point | 4,025 files, +368,685 / −26,266 | `git diff --stat e805fe98 upstream/main \| tail -1` |
| Upstream identity | still crate `beads_rust`, binary `br`, SQLite + JSONL, `.beads` dir (also accepts `_beads`), `issues.jsonl` default | `git show upstream/main:Cargo.toml`; `git show upstream/main:src/config/mod.rs \| grep -n DEFAULT_JSONL` |
| Upstream growth | `src/sync/mod.rs` 21,609 lines (was ~5,139); `src/storage/sqlite.rs` 34,865 (was ~5,157); 108 src `.rs` files | `git show upstream/main:src/sync/mod.rs \| wc -l` |
| New upstream modules | `src/sync/witness.rs`, `src/sync/db_inode_lock.rs`, `src/cache.rs`, `src/cli/commands/doctor_subsystems/`, new commands: audit, capabilities, capacity, changelog, coordination, gate, delete, defer, … | `git ls-tree -r upstream/main --name-only \| grep '^src/'` |
| Upstream does NOT have | `src/sync/org_bridge.rs`, `lefthook.yml` | `git cat-file -e upstream/main:<path>` |
| Upstream DOES have (own versions) | `flake.nix`, `fuzz/`, `tarpaulin.toml`, `rust-toolchain.toml`, `CHANGELOG.md`, `.githooks/`, `.claude-plugin/`, `skills/`, `.agent-mail.yaml`, `.ntm/`, `refactor/`, `sample_beads_db_files/` | `git ls-tree upstream/main --name-only` |
| Overlap (both sides touched) | ~80 files incl. `src/sync/mod.rs`, `src/storage/sqlite.rs`, `src/config/mod.rs`, `src/cli/mod.rs`, `src/main.rs`, `src/sync/path.rs`, `src/cli/commands/{sync,init,doctor,stats,history,schema,upgrade}.rs`, `Cargo.toml`, `flake.nix`, `install.sh`, README, LICENSE, CI workflows, many tests | `comm -12 <(git diff --name-only e805fe98 main \| sort) <(git diff --name-only e805fe98 upstream/main \| sort)` |
| Upstream license NOW | "MIT License (with OpenAI/Anthropic Rider)", © Jeffrey Emanuel | `git show upstream/main:LICENSE \| head -5` |
| Fork license | BSD 3-Clause, © John Wiegley | `head LICENSE.md` |
| Fork tracker data | `.beads/issues.jsonl` (STILL JSONL — the Org migration changed the default for new workspaces; this repo's own data was never converted), plus `beads.db` (gitignored), `config.yaml`, `metadata.json`, and several beads-branded spec docs (`SYNC_SAFETY_INVARIANTS.md` etc.) | `ls .beads/` |

### 3.1 The fork's 20 commits, classified

**Org-core (the fork's purpose — forward-port as specification, see §6):**
- `afe1351e` feat: use Org-mode format (issues.org) instead of JSONL
- `a0c54428` Fix doctor/stats commands for Org-mode format and improve org_bridge
- `c046b35e` Migrate e2e tests from JSONL to Org-mode format
- `03c5cd2c` Map internal status names to standard Org-mode keywords
- `33bb2e82` Add org2jsonl flake input for sandboxed builds
- `56e4a0fc` Fix clippy match_same_arms in keyword_to_status (rides with the above)

**Rename (superseded by the larger §7.6 rename; use as reference for surface):**
- `a2760e61` Rename CLI binary from br to obr
- `376e9dbf` Fix binary name in test helper after br→obr rename
- `6a0518d0` Update CI workflows for obr rename and tighten gates

**Productization (audit each against upstream; §5 Phase 4):**
- `aa129a2c` fix(storage): propagate errors instead of silent fallbacks in SQLite layer
- `f826ac4e` build(nix): modernize flake with rust-overlay replacing crane+fenix
- `1c32e5e5` Add nix flake checks and extend dev shell
- `4cbf6348` Add lefthook pre-commit hooks
- `5312cb5a` Add cargo-fuzz targets for input boundaries
- `5eb13fb7` Format shell scripts with shfmt
- `fcbbce29` Remove unused .cargo/audit.toml
- `b3415739` Replace MIT license with BSD 3-Clause  ← interacts with upstream's new license; USER DECISION, §8
- `a2ba738e` Rewrite README in maintainer voice

**Repo-local (carry data/docs forward, not code):**
- `9f1f2db0` chore(beads): create and close issues from code review findings (tracker data)
- `22adc72b` docs: add CLAUDE.md for Claude Code session context (keep; update facts after sync)

## 4. Strategy (decided; rationale recorded)

**Forward-port, not literal rebase.** The files the Org work integrates with grew ~4× upstream (sync/mod.rs 5K→21.6K lines). Replaying the fork's diffs as patches cannot succeed; instead:

1. Branch `org-forward-port` from `upstream/main`.
2. Re-implement the fork's changes as a **fresh, coherent commit series**, using the old commits (§3.1) as the *specification* and the new upstream code as the *substrate*.
3. **Sync first, rename second.** The de-beads rename touches thousands of sites; doing it before or during the sync would conflict with everything. It is the last major phase.
4. `main` is left intact as the pre-sync fork. At the end, the user decides how to land (§8).

Recommended execution style: sequential commits (builds are global; parallel mutation of one tree does not work), with parallel *read-only* subagents for recon and review. Each phase ends with its gate (§7.3) before the next begins.

## 5. Phase plan

### Phase 0 — Environment and baseline
1. `nix develop` (or ensure nightly per `rust-toolchain.toml`). Confirm `/Users/johnw/src/org2jsonl` exists.
2. `git fetch upstream --prune` (remote already configured).
3. Record the **upstream baseline**: `git worktree add /tmp/obr-baseline upstream/main` (worktree outside the repo is fine), then in it run and LOG: `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo test --lib --bins`, and a smoke of `cargo test --test <2-3 e2e targets that look central>` . Whatever fails HERE is pre-existing upstream breakage — the no-NEW-failures reference. Note: upstream has no org2jsonl dep, so it should build without it.
4. `git checkout -b org-forward-port upstream/main` in the main repo (or use a dedicated worktree; either is fine as long as `main` is untouched).
5. Delete the fork-only junk that should not carry forward if present after later steps: `temp_test/`, `temp_test_2/`, `custom.db` exist in BOTH trees (upstream has them too — leave upstream's tree as-is for now; flag for a cleanup commit at the end).

### Phase 1 — Recon (read-only; run as parallel subagents; write findings to `docs/research/upgrade/`)
Four briefs. Each must cite `ref:path` or `path:line` evidence and distinguish VERIFIED from INFERRED.

- **(R1) upstream-evolution:** Read `git show upstream/main:CHANGELOG.md` (~69KB), cluster the 1,843 commits by release/prefix. Map the new world: where export/import/format-selection now live in `src/sync/mod.rs` (grep for function names; find the analogues of the fork's `export_to_jsonl_with_policy`, `import_from_jsonl`, `emit_issue_heading` hook points); what `witness.rs`, `db_inode_lock.rs`, `cache.rs` do; how config discovery changed; the full new `Commands` enum; whether the JSONL wire format gained headers/`_type` records/new fields; whether upstream fixed defects the dossier found in the fork (concurrent-flush loss, fixed temp filename, LWW, `debug!`-swallowed errors, 32,767 ceiling — dossier §5 R1–R8; `db_inode_lock.rs` and `witness.rs` suggest upstream addressed some concurrency).
- **(R2) org-port-spec:** From the six Org commits (§3.1) + current `src/sync/org_bridge.rs` + `docs/research/map-sync.md` + `docs/research/gap-org2jsonl.md`, extract the full Org feature semantics (§6). Then locate each integration point's new home in upstream code and write per-touchpoint instructions.
- **(R3) productization-audit:** For each §3.1 productization commit: superseded / still-needed / semantic-merge, with evidence (e.g. diff `aa129a2c` against upstream's sqlite.rs — did upstream fix the same silent fallbacks? compare fork `flake.nix` vs upstream's; fork fuzz targets vs upstream's; lefthook vs upstream `.githooks/`). Also disposition for each new upstream root artifact (`.claude-plugin/`, `.agent-mail.yaml`, `.ntm/`, `refactor/`, `sample_beads_db_files/`, `skills/`): adopt, adapt, or drop.
- **(R4) branding-surface:** Inventory the complete beads/bd/br naming surface of the FUTURE tree (upstream/main + Org additions): `git grep -inE '(beads|BEADS|\.beads|_beads)' upstream/main -- ':!*.lock'` plus targeted greps for `\bbd\b`, `\bbr\b`, `BD_`, `BR_`. Categorize per §7.6's map and fill in exact file counts and any entries the draft map misses (config keys, completions, `.claude-plugin`, AUR/packaging, man pages, help strings, error text, `~/.beads` and `~/.config/bd` user-config paths, `.beads/redirect`, `.br_history`).

### Phase 2 — Integration plan
Synthesize Phase 1 into `docs/research/upgrade/INTEGRATION_PLAN.md`: the exact commit series (each with scope, files, source-of-truth, done-criteria), per-hotspot anchors (function + line in upstream code), the finalized rename map, gate definitions, and a risk register that states, for each dossier §5 risk, whether upstream already fixed it or it carries forward. Surface genuine user decisions (§8) — do not guess them.

### Phase 3 — Org forward-port (the heart; sequential commits)
Expected series (refine per Phase 2):
1. `build: add org2jsonl dependency` — Cargo path dep + flake input, **pinned by git rev** (mirror how the fork pins `toon_rust`; see §7.1).
2. `feat(sync): org_bridge module` — port `org_bridge.rs`, adapted to upstream's new model types (upstream `src/model/` grew; new fields need Org representations — R2 tells you which).
3. `feat(sync): Org format selection + emission + parsing` — wire extension-based format choice into upstream's current export/import paths; `DEFAULT_JSONL_FILENAME = "issues.org"` with `issues.jsonl` legacy read fallback (upstream `src/config/mod.rs:62` area).
4. `feat: status↔Org-keyword mapping` — re-apply `03c5cd2c` semantics (§6.3).
5. `fix: doctor/stats for Org` — re-apply `a0c54428` intent against upstream's new `doctor_subsystems/`.
6. `test: Org e2e coverage` — adapt `c046b35e`'s approach to upstream's (heavily grown) test estate; at minimum, the Org round-trip smoke gate (§7.3) must pass as a real test.

Gate after each commit. Do not proceed on a red gate.

### Phase 4 — Productization re-application
Apply the Phase-1 R3 disposition table: likely keep lefthook (upstream has none), graft fork's flake improvements onto upstream's flake (or vice versa — R3 decides), merge fuzz target sets, re-run shfmt, keep CLAUDE.md (updated), README rewrite deferred to Phase 6 (it must reflect the rename too). License handling per §8 decision.

### Phase 5 — Fork tracker data
Preserve this repo's own issues: copy `main`'s `.beads/issues.jsonl` (+ `config.yaml`, `metadata.json`) onto the branch, then once the Org port works, convert via the tool itself (`obr` import from JSONL → flush to `issues.org`) so the tracker data lands in the fork's own format. Verify issue count survives (dossier says 549 total / 3 open; re-verify with `obr count` equivalents). The beads-branded docs inside `.beads/` (`SYNC_SAFETY_INVARIANTS.md` etc.) migrate to the renamed directory in Phase 6 with names/content de-branded where they describe THIS tool (references to upstream-as-origin stay).

### Phase 6 — De-beads rename (mechanical, several commits, in this order)
Apply the finalized R4 map. Draft map (R4 completes it):

| Old | New | Compat policy |
|---|---|---|
| `.beads/` dir | `.obr/` | READ legacy `.beads` if `.obr` absent, with one-line deprecation warning; NEVER create beads-named artifacts |
| `_beads` (monorepo variant) | `_obr` | same legacy-read policy |
| `beads.db` | `obr.db` | legacy-read fallback |
| `issues.jsonl` default | `issues.org` (already the fork default) | `.jsonl` remains an explicit export format by extension |
| `.br_history/` | `.obr/history/` or `.obr_history/` (R4 recommends) | migrate on first write |
| `.beads/redirect` | `.obr/redirect` | legacy-read |
| `BEADS_DIR` env | `OBR_DIR` | accept legacy with warning |
| `BD_*` / `BEADS_*` env (3 spellings, see dossier map-cli) | `OBR_*` | accept legacy with warning |
| `BR_OUTPUT_FORMAT` | `OBR_OUTPUT_FORMAT` | accept legacy with warning |
| `~/.beads/`, `~/.config/beads/`, `~/.config/bd/` user config | `~/.config/obr/` | legacy-read chain preserved |
| crate `beads_rust`, bin `br` | crate `obr`, bin `obr` | hard rename |
| `BeadsError` and internal `beads_*` identifiers | `ObrError` / `obr_*` | internal-only; rename in ONE dedicated commit for reviewability |
| Help/error strings, completions, skills, `.claude-plugin`, packaging (AUR, install.sh), CI workflow names, docs | de-branded | hard rename |

Commit order: (a) artifact/env/CLI surface + compat shims; (b) crate/binary identity + Cargo/flake/CI; (c) internal identifiers; (d) docs/packaging/completions. Gate after each.

### Phase 7 — Verification sweep (see §7.3 for gate definitions)
Full gates + the rename-completeness audit + Org round-trip test + a fresh `obr init` → create → flush → clone-simulation → import cycle in a throwaway dir confirming: only `.obr/` artifacts are created, legacy `.beads` workspaces still open with a warning, and the file written is `issues.org`.

### Phase 8 — Docs, credit, landing
- README: maintainer voice (per `a2ba738e`), updated for the new upstream base and the rename, with an **Origins** section crediting Steve Yegge's beads (concept) and Dicklesworthstone's beads_rust (the codebase this derives from), with links. This is the ONE sanctioned beads reference besides license attribution.
- Update `CLAUDE.md`/`AGENTS.md` facts (binary `obr`, `.obr`, Org format, new commands).
- Present the user: summary of the series, gate results, the license question, and landing options (fast-forward `main` [history replacement, force-push], or merge, or keep branch). **Do not land.**

## 6. Org-mode feature specification (must-preserve semantics)

Extracted from the fork's implementation; R2 refines against the old diffs. The port must preserve *behavior*, not code shape.

1. **Format selection:** chosen per-file by extension — `.org` → Org bridge, else JSONL. `DEFAULT_JSONL_FILENAME = "issues.org"` (fork's `src/config/mod.rs:33`); discovery prefers `issues.org`, falls back to legacy `issues.jsonl` (fork's `src/config/mod.rs:100-116`).
2. **Org emission** (fork `src/sync/org_bridge.rs`, `emit_issue_heading`): one heading per issue; status ↔ TODO keyword; priority ↔ `[#A]`..`[#E]` cookies; labels ↔ Org tags (KNOWN BUG: labels containing `:` are destroyed by tag round-trip — dossier R9; do not worsen; fixing is in-scope if cheap); issue fields in a `:PROPERTIES:` drawer (including `:ID:`, timestamps as RFC3339, `:BEADS_SCHEMA_VERSION:` — rename this property key in Phase 6); dependencies and comments as JSON `src` blocks; description as body text. Emission is deterministic (sorted labels/deps/comments).
3. **Status↔keyword mapping** (`03c5cd2c`): internal statuses map to standard Org keywords — verify the exact map from the commit (`git show 03c5cd2c`); e.g. open→TODO, in_progress→STRT/NEXT, closed→DONE, deferred→WAIT (READ THE DIFF; do not trust this line).
4. **Parsing:** via `org2jsonl` crate — the fork uses exactly `org_to_entries_with_keywords` plus five types (see `docs/research/gap-org2jsonl.md` §seam); obr writes Org by hand and parses via orgize-backed org2jsonl. KNOWN ISSUES to not worsen: `:ID:`-less heading bricks parsing with no line info; O(n²) parse ≥100KB.
5. **doctor/stats:** must understand the Org file as the export artifact (`a0c54428`).
6. **`--no-db` mode:** seeds in-memory DB by importing the org file; must keep working (it is the user's preferred lightweight mode).
7. **e2e tests:** the fork migrated its e2e suite to Org expectations (`c046b35e`); upstream's e2e suite is much larger now and JSONL-based — the port sets Org as default, so R2/Phase 3 must decide per test file: pin to JSONL explicitly (testing the JSONL path) or convert to Org (testing the default path). Both paths need coverage.

## 7. Landmines and risks

1. **org2jsonl supply chain:** unpublished path dep. Fork's `flake.nix:22` pins `git+file:///Users/johnw/src/org2jsonl` at rev `5ea7586` — 15 commits behind that repo's worktree; `Cargo.lock` was stale (missing `walkdir`), so `cargo build --locked` failed. FIX during Phase 3: pin a current rev in both Cargo.toml (git dep w/ rev, like `toon_rust` at fork `Cargo.toml:54`) and flake.nix; verify `cargo build --locked` passes. CI must be able to fetch it or vendored — otherwise CI stays structurally broken (dossier R14).
2. **Upstream JSONL schema growth:** if upstream added record kinds/fields (R1 verifies), the Org representation must cover them or explicitly reject them with a clear error. Silent drops are the #1 historical failure mode of this codebase (dossier themes).
3. **Test-estate reality:** the dossier proved the fork's integration tests were largely unexecuted and partly red, and upstream's estate is 4× bigger with unknown health. Hence the §5 Phase-0 baseline and no-NEW-failures rule. Conformance tests additionally require a Go `bd` binary — out of scope; skip them.
4. **Fork tracker data:** `.beads/issues.jsonl` in `main` is the project's own issue history. It must exist, converted, in the final branch (`.obr/issues.org`). Loss = failure.
5. **Upstream's new concurrency machinery** (`witness.rs`, `db_inode_lock.rs`) may assume JSONL in its safety checks (conflict-marker scans, allowlists of files sync may touch, git-safety tests with hardcoded filenames — the fork hit exactly this class: dossier R14's allowlist failures). Grep for hardcoded `issues.jsonl`/`.jsonl` in upstream's sync safety layer and tests; every site needs Org awareness.
6. **`Cargo.lock`:** regenerate on the new base (upstream's lock + org2jsonl + toon if kept); commit it; verify `--locked` builds.
7. **Licensing:** §8 — do not resolve autonomously.
8. **Verification gates (definitions):**
   - G-build: `cargo build` clean.
   - G-lint: `cargo clippy --all-targets -- -D warnings` clean (fork convention; if upstream's own tree fails this at baseline, gate on no-NEW-warnings instead).
   - G-unit: `cargo test --lib --bins` — no new failures vs Phase-0 baseline.
   - G-e2e-subset: the 2–3 targets chosen in Phase 0 — no new failures.
   - G-org-roundtrip: throwaway workspace; init → create issues incl. labels/deps/comments/unicode → flush → verify `issues.org` well-formed → re-import into fresh DB → field-level equality (allowing known-lossy fields ONLY if pre-existing, documented in R2).
   - G-rename: `git grep -inE '(beads|\bbd\b)' -- ':!docs/research' ':!CHANGELOG.md'` returns ONLY the allowlist: README Origins section, LICENSE attribution, legacy-compat identifiers (the shims from Phase 6), and historical references in UPGRADE_LOG. Plus `git grep -n '\.beads\|BEADS_DIR\|beads\.db'` hits only the legacy-compat shim code and its tests. Binary check: built artifact is named `obr`; `obr --help` contains no "beads"/"br "/"bd ".

## 8. Open decisions — ASK THE USER, do not decide

1. **License.** Upstream is now "MIT with OpenAI/Anthropic Rider" (© Jeffrey Emanuel); the fork replaced MIT with BSD-3 (© John Wiegley). Forward-porting onto upstream's tree means the combined work contains upstream's code under its license+rider. Options: (a) keep upstream LICENSE + add fork's changes under BSD-3 dual notice; (b) BSD-3 for the fork's contributions with upstream's MIT+rider text preserved as required attribution; (c) something else. Needs the user (and possibly reading the rider's exact terms).
2. **`.obr` artifact names:** confirm `obr.db` (vs keeping `beads.db` name hidden inside `.obr/` — recommend `obr.db`), and `.obr/history/` vs `.obr_history/`.
3. **Landing:** replace `main` (force-push; clean history: upstream + port series) vs merge commit vs long-lived branch.
4. **Upstream root adoptions:** `.claude-plugin/`, `skills/`, `.agent-mail.yaml`, `.ntm/` — adopt renamed, or drop? (R3 will give a recommendation; adoption implies maintaining them de-branded.)

## 9. Reference materials

- **In-repo (durable):** `docs/research/DOSSIER.md` (full research dossier, 2026-08-06); `docs/research/map-sync.md`; `docs/research/gap-org2jsonl.md`; this plan.
- **Ephemeral** (same machine, `/private/tmp`, may vanish on reboot — copy anything you need early): full research notes at `/private/tmp/claude-501/-Users-johnw-src-obr/f80d1967-0fc0-44fa-a53f-3054d39727e2/scratchpad/research/` (22 files: subsystem maps, web research incl. upstream-beads history, five adversarial analyses, gap reports); staged upstream file dumps at `.../scratchpad/upgrade/_up_*.rs` (regenerable via `git show upstream/main:<path>`).
- **Remotes:** `origin` = `git@github.com:jwiegley/obr.git`; `upstream` = `https://github.com/Dicklesworthstone/beads_rust.git` (configured + fetched 2026-08-06).
- **Prior session's aborted recon workflow:** produced no findings (only the `_up_*.rs` staging dumps); nothing to recover.
