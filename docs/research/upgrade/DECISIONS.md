# Decision record — upstream sync & de-beads rename

**Status:** working defaults adopted 2026-08-06 by the executing session so the port can proceed autonomously. Every item below is **reversible before landing** (nothing is pushed; `main` is untouched). Items marked **USER — OPEN** are not resolved and will be presented at Phase 8.

## USER — DECIDED 2026-08-07 (live Q&A session; formerly OPEN)

| ID | Ruling |
|---|---|
| D-LICENSE (§8.1) | **Keep upstream's LICENSE (MIT + rider) untouched; the user's own contributions are MIT.** P4-08 unblocked: packaging license fields state MIT with a see-LICENSE rider reference; a Licensing note in README (Phase 8) records the split (upstream code MIT+rider; new contributions MIT). |
| Landing mode (§8.3) | **Ancestry-preserving merge**: from org-forward-port, `git merge -s ours main` (records old main as a parent without taking content), then fast-forward main. Both histories reachable; no force anywhere. Phase 8 presents the exact commands; execution still awaits the user's explicit go. |
| D-RESIDUALS | **Document only, for now.** Upstream-carried defects (incl. the verified Phase-6 inventory below) stay documented, not fixed; user may queue fixes as ordinary issues later against a stable base. |
| D-SURFACE dir rule (amendment) | **doc/ then docs/ then root**: PLAN.org lives in `doc/` if present, else `docs/`, else the repo root. Highest-priority existing file wins; shadowing warns. (Supersedes the "doc/ literal" first reading.) |
| D-DISTRIBUTION (supersedes the packaging/install.sh half of D-SELFUPDATE) | **Keep + repoint + fix**: packaging/ manifests de-branded to jwiegley/obr (obr.rb, obr.json, PKGBUILDs), install.sh de-branded (upstream's 61 KB version), release.yml fixed (drop --features self_update, obr asset names). The self-updater itself stays deleted. |
| Report artifact | User wants a Claude artifact after the dust settles: all work done, port status, functionality lost/gained accounting. |

### Upstream-carried defect inventory (verified against d1fb0d3a during Phase 6, 2026-08-07; NOT fixed per D-RESIDUALS discipline)

| Class | Evidence | Size |
|---|---|---|
| e2e_routing write-lock timeouts + routed-mutation deadlock | branch and baseline failure NAME SETS identical (23=23, both comm diffs empty) | ~15 tests |
| `init` into a pre-existing EMPTY workspace dir fails (pending-sync-merge guard runs before init dispatch; database missing) | reproduces verbatim on the baseline binary with `.beads` | ~10 tests overlap w/ above |
| Write-authority lock sidecars `.obr/.br-jsonl-write-<digest>.lock` not in sync path allowlist (introduced upstream 251b501b) | in committed baseline inventory (e2e_sync_git_safety, 3-4 tests) | 3 tests |
| Born-broken tests: `--id` flag that never existed; schema-4 fixture DB vs schema-17 product; never-implemented `persistent_advisory_inode` doctor classification | in committed baseline inventory | 3 tests |
| macOS `/var`-symlink TMPDIR class | env-dependent; measured 34 failures on default TMPDIR, 0 on clean TMPDIR | env |

**Resolved since (2026-08-15, obr-m6m):** the `persistent_advisory_inode` half of the born-broken row. The classification was never implemented and is not being implemented; the two assertions demanding it (`tests/doctor_fixtures/mcp_serve_stale_write_lock`, `tests/e2e_concurrency.rs`) now demand `probe_acquired_free`, the implemented answer for an old lock file with no live holder. This became checkable only once read-only `obr doctor` stopped taking the workspace write lock: until then the probe collided with its own caller and answered `probe_would_block_live_holder` for every workspace, so the fixture's apparent pass was unrelated to what it claimed to test. The other two born-broken items (`--id` flag, schema-4 fixture DB) are untouched.

### Port-found fail-safe Org-blind comparisons (documented, deliberately left)

- No-op-flush anchor certification (`sync.rs:~2333`): always-false on Org → loses only the inode-preserving fast path; fallback writes the correct derived anchor.
- `sync --witness` merkle-diffs JSONL lines against Org text → spurious drift REPORT only; gates nothing. Making the witness surface format-aware is future feature work.
- A4 partial: a hand-made workspace containing ONLY `beads.jsonl` still resolves there for write (read/write split declined as beyond a rename); init-created workspaces can never reach it.

### Port regressions found and FIXED during Phase 6

- Merge-anchor writers still wrote `beads.base.jsonl` (5ee2209a).
- `doctor --repair` write_scopes refused `.obr/` — repair non-functional on renamed workspaces (5ee2209a).
- `sync --merge` impossible on Org workspaces — format-blind anchor verification at four sites (bfbaa959).
- mcp feature didn't compile after the field rename (190d715d).
- Test-harness PATH clobber made the zero-git sentinel unreachable (e6ce9954) — with it fixed, `br` is PROVEN to never invoke git across all sync/mutation paths (positive control: the explicit read-only `vcs-status` probe fires the same sentinel). Supports D-NOBRANCH.

## USER DECISIONS — 2026-08-07 (mid-execution, authoritative; override plan text and adopted defaults)

| ID | Decision | Detail |
|---|---|---|
| D-REPOTRACKER | **This repository does not self-track. `.beads/` dropped entirely.** | User: "Drop any existing .beads or beads branch. I'm not using Beads with this repository and I never plan to." Overrides plan §7.4 fork-tracker-survival and D-UPSTREAMTRACKER; P5-03 prefix rename moot. The verified Org conversion machinery survives as product code. 549-issue corpus remains in git history. SYNC_*.md product specs relocated to docs/. No beads-named branches existed. |
| D-SURFACE | **Human surface = `doc/PLAN.org` if a `doc/` directory exists, else `./PLAN.org`, tracked in git. `.obr/` holds only the git-ignored cached SQLite db (+ internal artifacts).** *Implementation sketch:* `.obr/.gitignore` contains `*` (self-ignoring dir — nothing under `.obr/` is ever tracked); `PLAN.org` gains an `#+ISSUE_PREFIX:` file keyword written on flush and honored on import so a fresh clone bootstraps from the tracked file alone; discovery order `doc/PLAN.org` → `./PLAN.org` → legacy in-dir chain (LR); sync path allowlist extended to exactly the resolved PLAN.org location. | User: "It's fine [if] obr maintains a .obr directory with a cached sqlite db that is ignored by Git, and a file called doc/PLAN.org (if there is a doc/ directory, otherwise ./PLAN.org) that it tracks in version control." Supersedes U4 (`issues.org` inside the dot-dir) and the §8.2 file layout. `doc/` taken literally (not `docs/`) pending user correction. Requires allowlist extension beyond the dot-dir. |
| D-NOBRANCH | **No secondary tracking branch, ever; nothing invasive.** | User: "I don't want obr to use any kind of secondary tracking branch the way that Beads did. Beads was far too invasive, and I don't intend to replicate its behaviors here." Hard product invariant; obr never runs git (already true — keep tested). |
| D-DOCS | **All documents and Markdown refer to obr, describe intended usage, convey accurate expectations.** | User directive; executed as the docs pass after the binary/crate rename so docs match the built reality. Agent-swarm workflow content (Agent Mail, bv, cass, rch) removed rather than renamed. |

## Adopted working defaults (recommendation of the cited brief; flag at Phase 8)

| ID | Decision | Adopted default | Source |
|---|---|---|---|
| §8.2 | `.obr` artifact names | `obr.db`; history at `.obr/history/`; recovery at `.obr/recovery/` | plan's own recommendation + R4 §7.7-7.8 |
| U1 | `agent_context` in Org | Carry, as `** Agent Context` level-2 child with JSON `src` block (drawer property not viable — sanitizer flattens newlines) | R2 |
| U2 | Description fidelity | (a) preserve-by-raw-slice via org2jsonl `char_begin`/`char_end` when body has non-Paragraph elements; fall back to (c) document-the-drop only if (a) proves infeasible without modifying org2jsonl | R2 |
| U3 | Colon labels (R9) | Fix now via canonical `:LABELS:` drawer property (colon rejection would break upstream's live `provides:<cap>` capability feature) | R2 |
| U-DRAFT | `Status::Draft` keyword | `DRAFT`, added to the TODO keyword set | R1/R2 |
| U5 | Schema-version property key | Rename to `:OBR_SCHEMA_VERSION:` in Phase 3 (value 17); reader accepts legacy `:BEADS_SCHEMA_VERSION:` | R2 |
| U-EXTERNAL | `.org` via `--allow-external-jsonl`/`BEADS_JSONL` | **Keep hard-reject** (narrowest safety change; external escape hatch stays JSONL-only; documented) | conservative — no brief recommendation |
| U-MERGEART | Internal merge/ancillary artifacts (`beads.base/left/right.jsonl`, `sync_base.jsonl`, `deletions.jsonl`, `interactions.jsonl`) | Stay JSONL (internal, never hand-edited; renamed `obr.*` in Phase 6); workspace is mixed-format internally, Org-only at the human surface | R1 |
| U4 | `init` seeding | ~~`issues.org` only~~ **SUPERSEDED by D-SURFACE (2026-08-07):** seeds `doc/PLAN.org` / `./PLAN.org` | R2 |
| U-TESTPOLICY | 160 JSONL-hardcoding test files | Per-FUNCTION Class A/B/C rule (pin JSONL / convert to Org / split); never branch on file existence in a test | R2 |
| U6 | Org auto-flush performance | Accept full re-export per mutation for Org (incremental line-splice declined); upstream's regression test pinned to JSONL | R2 |
| Q-LINES | `:ID:`-less heading recovery | (a) abort, with cached-parse heading-ordinal diagnostics; option (c) (line numbers via ~8-line org2jsonl change) deferred — ground rules forbid pushing to remotes and org2jsonl must stay pinned to a published rev | conservative |
| D-HOOKS | `.githooks/` vs lefthook | lefthook, with upstream's `br doctor --quick` folded in as a hook; `.githooks/` dropped | R3 |
| D-AUDIT | `.cargo/audit.toml` | Keep upstream's file (do not re-apply fork's deletion); note CI advisory step remains non-blocking pending user | R3 |
| D-SHFMT | shell formatting | SETTLED during P4-04: shfmt defaults (tabs), zero-config, converge-via-lefthook on touched files only; doctor fixtures excluded from the hook glob | R3 |
| D-PLUGIN / §8.4 | `.claude-plugin/`, `.claude/`, `skills/`, `.agent-mail.yaml`, `.ntm/`, `refactor/`, `sample_beads_db_files/` | Drop all (P4-01); recoverable from upstream history if wanted later | R3/R4 |
| D-SUPPLYCHAIN | sha-pinned actions machinery | Adopt (maintenance commitment noted) | R3 |
| D-ACFS | `notify-acfs.yml` | Drop (points at upstream's registry) | R3 |
| D-CHANGELOG | upstream `CHANGELOG.md` | Keep verbatim as labelled upstream history; excluded from G-rename | R3/R4 |
| D-PREFIX | tracker issue IDs `beads_rust-*` | Re-prefix to `obr-*` via `sync --rename-prefix` after a verified dry-run; fallback: keep IDs + allowlist | R4 |
| D-SELFUPDATE | `obr upgrade` / self_update / packaging | Disable & delete (`self_update` out of default features; `upgrade` cmd, `packaging/`, `install.sh` removed) — unpublished personal fork; interaction with cfg-gated `e2e_schema` golden test handled in P6-05 | R4 |
| D-DOMAINSEP | 9 branded hash domain separators | Keep (never reach disk; renaming invalidates caches/receipts); documented residual | R4 |
| D-MONOREPO | `_beads` variant | Accept `_obr` + legacy-read `_beads`, mirroring upstream semantics | R4 |
| D-GITIGNORE | `.beads/recovery*/` glob | Tighten to exact path during P6-01a | R4 |
| D-DELETIONS | bulk deletions P4-01/P6-00 | Per the recommended lists (~450 files) | R3/R4 |
| D-UPSTREAMTRACKER | upstream's 953-issue tracker | ~~Replaced by the fork's 549-issue corpus~~ **SUPERSEDED by D-REPOTRACKER (2026-08-07):** no in-repo tracker at all; both corpora live in git history only | critic |
