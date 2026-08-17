# Subsystem Map: `docs` — Intent, History, and Documentation Drift

Repository: `/Users/johnw/src/obr` (crate `beads_rust` v0.1.14, binary `obr`)
Research date: 2026-08-06
Scope: README, AGENTS.md, CLAUDE.md, the four large planning documents, `docs/`, `skills/`,
`.beads/` self-tracked issues, and the git log (598 commits).

---

## 0. TL;DR — the three things that matter

1. **The project is a fork-in-place that changed identity twice, and the documentation
   only partially followed.** It began as Jeffrey Emanuel's (`Dicklesworthstone`) Rust port
   of Steve Yegge's Go `beads` (`bd`), binary `br`. John Wiegley took it over in
   Feb–Mar 2026, renamed the binary `br` → `obr`, replaced JSONL storage with **Org-mode**
   (`issues.org`), swapped MIT → BSD-3-Clause, and rewrote the README. **Every planning
   document, `AGENTS.md`, `CLAUDE.md`, `VCS_INTEGRATION.md`, all of `docs/agent/`, all of
   `.beads/*.md`, and `skills/` still describe `br` + JSONL.**

2. **The intended architecture is documented at extraordinary length** (~16k lines across
   5 root docs + ~5k lines under `docs/`), and the *shape* of it survived: SQLite as source
   of truth, a flat-file git-friendly export, hash-based short IDs, content-addressed dedup,
   non-invasive (no daemon/hooks/auto-git), clap-derive CLI, robot/JSON output. But specific
   architectural prescriptions in `PROPOSED_ARCHITECTURE_FOR_BR_USING_RUST_BEST_PRACTICES.md`
   were largely **not** followed: no `rayon`, no `parking_lot`, no TOML config (YAML instead),
   no per-type module split (two 5k-line god modules instead), no `src/context.rs`.

3. **Almost nothing is left on the roadmap in the tracker.** `.beads/issues.jsonl` holds 549
   issues; **545 closed, 1 tombstone, 3 open**. The live roadmap is 3 small refactors. The
   *real* remaining work is in the untracked
   `docs/superpowers/plans/2026-03-16-productize.md` (Chunk 5 unimplemented) and in the
   accumulated doc drift catalogued in §6.

---

## 1. Document inventory

### 1.1 Root-level documents

| File | Lines | Date/Status | Role |
|---|---:|---|---|
| `README.md` | 147 | Rewritten 2026-03-16 (`a2ba738`) | User-facing intro, in John Wiegley's voice. **Only doc that says `obr` throughout.** |
| `AGENTS.md` | 816 | Last touched 2026-02-18 (`b6af118`) | Agent working rules + full architecture + external-tool manuals (bv, UBS, RCH, ast-grep, warp_grep). Says `br` 47×. |
| `CLAUDE.md` | 75 | Added 2026-02-18 (`22adc72`) | Short Claude Code session primer. Says `br` 10×. |
| `EXISTING_BEADS_STRUCTURE_AND_ARCHITECTURE.md` | 7937 | 2026-01-15..16 | **Reverse-engineered spec of the Go `bd` codebase**, written so the port could proceed without reading Go source (`legacy_beads/`, gitignored). 15 numbered sections + 98 sub-sections under §15 "Additional Legacy Findings". |
| `PLAN_TO_PORT_BEADS_WITH_SQLITE_AND_ISSUES_JSONL_TO_RUST.md` | 1763 | 2026-01-15, "Status: Planning Phase" | The *why* and *scope* doc. Author "Jeffrey Emanuel". |
| `PROPOSED_ARCHITECTURE_FOR_BR_USING_RUST_BEST_PRACTICES.md` | 3522 | 2026-01-16, "Author: Claude Opus 4.5", "Document version 1.1" | The *how* doc — code-level prescriptions synthesised from sibling projects `xf` and `cass`. |
| `RICH_INTEGRATION_PLAN.md` | 1655 | Added 2026-01-19 (`2a5d635`) | 6-week plan to replace `colored` with `rich_rust`. Largely executed. |
| `VCS_INTEGRATION.md` | 105 | 2026-01-25 (`7596071`) | Git/hg/jj/p4/sl equivalents for the manual commit workflow. |
| `UPGRADE_LOG.md` | 60 | 2026-01-18 | One-shot dependency-bump log (criterion, rusqlite, unicode-width, indicatif). |
| `AGENT_FRIENDLINESS_REPORT.md` | 82 | 2026-01-25, auditor "WildAnchor (Codex/GPT-5)" | Agent-surface audit + scorecard + the **CLI-only, no-MCP decision**. |
| `CHANGELOG_AGENT_FRIENDLY.md` | 10 | 2026-01-25 | Single-entry changelog for agent-facing changes. |
| `CLI_SCHEMA.json`, `ROBOT_MODE_EXAMPLES.jsonl` | — | 2026-01-25 | Static machine-readable artifacts (interim, per `AGENT_FRIENDLINESS_REPORT.md:68`). |

### 1.2 `docs/`

| File | Lines | Notes |
|---|---:|---|
| `docs/ARCHITECTURE.md` | 675 | Internal architecture overview. Partially renamed (1 `obr`, 5 `br`). |
| `docs/CLI_REFERENCE.md` | 956 | Per-command reference. Title says `obr`, body says `br` 86×. |
| `docs/TROUBLESHOOTING.md` | 973 | Title `obr`, body `br` 107×. |
| `docs/AGENT_INTEGRATION.md` | 539 | AI-agent guide. |
| `docs/INSTALLING.md` | 487 | Install methods; still points at the upstream GitHub repo. |
| `docs/TEST_HARNESS.md` | 478 | E2E / conformance / bench runner guide. |
| `docs/E2E_COVERAGE_MATRIX.md` | 354 | Claims 54 commands, 44 covered, **81 %** coverage. |
| `docs/E2E_SYNC_TESTS.md` | 362 | Sync-safety e2e suite guide. |
| `docs/ARTIFACT_LOG_SCHEMA.md` | 245 | JSONL schema for test artifacts under `target/test-artifacts/`. |
| `docs/SYNC_SAFETY.md` | 226 | User-facing sync safety model. |
| `docs/SYNC_MAINTENANCE_CHECKLIST.md` | 201 | Mandatory checklist referenced from `AGENTS.md:370`. |
| `docs/agent/{AGENTS,QUICKSTART,ROBOT_MODE,SCHEMA,ERRORS,EXAMPLES}.md` | 17–63 each | Thin agent-first entry points added 2026-01-25. **All still say `br`.** |
| `docs/superpowers/plans/2026-03-16-productize.md` | 219 | **Untracked** (only entry in `git status`). The most recent plan; see §5. |

### 1.3 `.beads/` prose (technical specs, git-tracked)

| File | Lines | Notes |
|---|---:|---|
| `.beads/SYNC_SAFETY_INVARIANTS.md` | 218 | 8 numbered Non-Goals (NG-1..8) + testable invariants (PC-*, AW-*, DL-*, IV-*, NGI-*) with risk levels. |
| `.beads/SYNC_THREAT_MODEL.md` | 188 | Root-cause analysis of the incident where "`bd sync` produced a commit that deleted all repository source files". |
| `.beads/SYNC_CLI_FLAG_SEMANTICS.md` | 255 | Safe-default flag matrix and user-intent gating. |
| `.beads/MCP_AGENT_MAIL_PATTERNS.md` | 310 | Research doc cataloguing agent-friendly error patterns from `mcp_agent_mail` (Python). |
| `.beads/README.md` | 81 | **Boilerplate from upstream Go `bd init`** — still advertises `bd`, `bd sync`, auto-sync and steveyegge/beads. Never updated. |

### 1.4 `skills/`

Single skill `skills/bd-to-br-migration/` (SKILL.md 332 lines + `references/{TRANSFORMS,PITFALLS,BULK}.md`
+ `scripts/{verify-migration.sh,find-bd-refs.sh}` + `subagents/batch-migrator.md` + `SELF-TEST.md`).
Its purpose is to mechanically migrate *other* repos' docs from `bd` → `br`
(`skills/bd-to-br-migration/SKILL.md:5`, "The ONLY difference is git handling — everything else
is find-replace"). Ironic: the skill was never run against this repo's own `br` → `obr` rename.

### 1.5 `agent_baseline/` — captured snapshots (stale)

`agent_baseline/help/{br_help.txt,br_list_help.txt,br_schema_help.txt}` are pre-rename
snapshots; `agent_baseline/help/br_help.txt:3` still reads `Usage: br [OPTIONS] <COMMAND>`.
`agent_baseline/{examples,schemas,errors}/` hold JSON/TOON baselines.

---

## 2. The INTENDED architecture (as documented)

### 2.1 Origin story and motivation

`PLAN_TO_PORT_BEADS_WITH_SQLITE_AND_ISSUES_JSONL_TO_RUST.md:12` states the thesis: Steve
Yegge's Go `bd` was being moved to a **Dolt** backend, which "fundamentally changes its
architecture"; this port "preserves the elegant SQLite + JSONL hybrid design". The now-deleted
original README (`git show a2ba738^:README.md`, lines ~35–43) is more explicit: the author's
"Agent Flywheel" tooling was built on classic beads, Yegge was moving to GasTown, and rather
than ask for a legacy mode he froze the architecture in a Rust fork — with "Steve's full
endorsement".

### 2.2 Non-negotiable requirements

`PROPOSED_ARCHITECTURE_FOR_BR_USING_RUST_BEST_PRACTICES.md:28-44` lists nine hard constraints:
SQLite + JSONL hybrid (no Dolt); schema compatibility with Go `bd`; CLI compatibility;
hash-based short IDs (`bd-abc123`, not autoincrement); deterministic content hashing;
non-invasive (no hooks/git/daemon/background processes); JSON output stability; robot mode
(clean JSON to stdout, diagnostics to stderr); `#![forbid(unsafe_code)]`.

### 2.3 Non-invasiveness — the central design commitment

`PLAN...:16-22` — "`br` will be LESS invasive than `bd`": no automatic hook installation,
no automatic git operations, no daemon/RPC, explicit over implicit, minimal footprint.
Restated at `PLAN...:103-110` (Non-Goals), `PROPOSED...:3501-3510` (Appendix D),
`.beads/SYNC_SAFETY_INVARIANTS.md:11-25` (NG-1..NG-8), `AGENTS.md:352`, `CLAUDE.md:52`,
`README.md:138-143`, `docs/ARCHITECTURE.md:28`.

### 2.4 Explicit scope exclusions

`PLAN...:387-441`:
- Not ported: `internal/storage/dolt/`, `internal/rpc/` (daemon), `internal/linear/`,
  `internal/jira.go`, `claude-plugin/` (MCP), `internal/hooks/`.
- **Gastown features explicitly excluded** (`PLAN...:399-437`): issue types `gate`, `agent`,
  `role`, `molecule`, `rig`, `convoy`; fields `agent_*`, `hop_*`, `molecule_*`, `gate_*`,
  `convoy_*`, `rig_*`, `external_agent_*`, `session_id`, `workflow_*`; tables `agents`,
  `molecules`, `gates`, `rigs`, `convoys`, `workflow_*`; commands `bd gate|agent|molecule|rig|convoy|hop|session`.
  Rationale at `PLAN...:437`: ~40 % of Go codebase complexity for a use case this fork doesn't need.
- **Division of labour with `bv` (beads_viewer)**: `PLAN...:295-370` — `bv` owns triage,
  graph analytics (PageRank, betweenness, critical path), and the TUI; `br` owns storage
  and CRUD. `PLAN...:1650` — "No TUI — that's `bv`'s domain."
- `AGENT_FRIENDLINESS_REPORT.md:56-65` — **"Decision: CLI-only (no MCP surface in this repo)"**,
  because MCP would add distribution/auth/permission surface without core value.

### 2.5 Sync safety — a threat model written after a real incident

This is the most distinctive part of the documentation set. `PLAN...:71-152` embeds a full
threat model, and `.beads/SYNC_THREAT_MODEL.md:5` records the triggering incident: the Go
`bd sync` "produced a commit that deleted all repository source files".

**This actually happened inside this repository.** Git log evidence:
- `b8020fa` (2026-01-19) `fix: restore README.md clobbered by bd sync`
- `a326295` (2026-01-19) `fix: restore all files destroyed by bd sync virus`

Everything downstream — the path allowlist in `src/sync/path.rs`, the `--force`-gated export
guards, the atomic temp+rename pattern, `docs/SYNC_SAFETY.md`,
`docs/SYNC_MAINTENANCE_CHECKLIST.md`, `.beads/SYNC_*.md`, the `tests/e2e_sync_*.rs` suite —
descends from that incident. It is the strongest example in this repo of documentation that
is genuinely load-bearing rather than decorative.

Testable invariants (`.beads/SYNC_SAFETY_INVARIANTS.md:33-96`): PC-1..4 (path confinement),
AW-1..4 (atomic writes), DL-1..5 (data-loss prevention), IV-1..4 (input validation),
NGI-1..3 (no git operations).

### 2.6 Features intended to go *beyond* `bd` parity

`PLAN...:1453-1547` names five: (1) local history backup `.br_history/` with rotation,
(2) bulk update across multiple IDs, (3) saved queries (`br query save/run/list/delete`),
(4) CSV export, (5) changelog generation from closed issues.
**All five shipped** — `src/sync/history.rs`, `src/cli/commands/update.rs`,
`src/cli/commands/query.rs` (907 lines), `src/format/csv.rs`, `src/cli/commands/changelog.rs`.

### 2.7 Planned phases

`PLAN...:1571-1652` (5 phases, 11–16 sessions estimated) and `PROPOSED...:3262-3353`
(5 phases, weeks 1–10). Both are fully worked through by the git history; every phase's
deliverables map to shipped commands.

`PLAN...:1736-1748` leaves three **open questions**, never explicitly resolved in prose:
1. ID format — generate identically to `bd`, or is semantic equivalence enough?
2. Schema version handling if the schema diverges from `bd`.
3. Full-text search — add Tantivy, or stay with SQLite FTS/LIKE?

Empirically: (1) resolved in favour of parity — `tests/storage_id_hash_parity.rs`,
`96b1e15`-era conformance work; (3) resolved as "stay with SQLite" — no `tantivy` in
`Cargo.toml`, and `src/cli/commands/search.rs` uses LIKE with escaping (`81266c8`
"escape LIKE pattern special characters in searches").

---

## 3. What the docs ADMIT changed

### 3.1 Admitted in-document

- `PROPOSED...:3429-3448` "Risk Mitigation" pre-registers schema drift from `bd`, JSONL
  format incompatibility, and performance regression as Medium/High risks — with
  "versioned JSONL format with compat shims" as a fallback. (Note: `AGENTS.md:116-121`
  simultaneously forbids compatibility shims. These two documents contradict each other.)
- `AGENT_FRIENDLINESS_REPORT.md:66-71` "Gaps / Next Improvements" — no dynamic `--help-json`
  (only the static `CLI_SCHEMA.json`); many commands return bare arrays/objects rather than a
  consistent `{data, metadata, errors}` envelope; schema outputs embed `generated_at` so they
  are not byte-deterministic.
- `UPGRADE_LOG.md:33-36` — `vergen-gix` bump skipped, "blocked by Rust version constraint".
  **Since resolved**: `Cargo.toml` now pins `vergen-gix = "9.1"`.
- `docs/E2E_COVERAGE_MATRIX.md:12-24` — self-reported gaps: Labels 60 %, Config 40 %,
  History 50 %, Dependencies 80 %, Diagnostics 80 %; overall 81 %.
- `EXISTING_BEADS_STRUCTURE_AND_ARCHITECTURE.md:29+` keeps a live "Working TODO" checklist
  inside the document itself, all items checked.

### 3.2 Changed but **not** admitted anywhere in prose

These are the big ones. Each is a silent divergence between the documented intent and the code.

| # | Documented intent | Actual state | Evidence |
|---|---|---|---|
| A | Binary is `br` | Binary is `obr` | `Cargo.toml:13-15` (`name = "obr"`); `src/cli/mod.rs:652` (`#[command(name = "obr")]`); rename commit `a2760e6` (2026-02-19) |
| B | Storage export is JSONL (`.beads/issues.jsonl`) | **Org-mode (`.beads/issues.org`) is the default**; JSONL is legacy fallback | `src/config/mod.rs:33` `DEFAULT_JSONL_FILENAME = "issues.org"`; `:35` `LEGACY_JSONL_FILENAME = "issues.jsonl"`; `:100-101` "Prefer `issues.org` if present (new default). Fall back to `issues.jsonl` if present (legacy)"; new module `src/sync/org_bridge.rs` (900 lines); commit `afe1351` |
| C | Config format is TOML (`PROPOSED...:3421`) | YAML (`serde_yaml`, `.beads/config.yaml`) | `Cargo.toml` dep `serde_yaml = "0.9"`; `docs/ARCHITECTURE.md:361` |
| D | `rayon` for parallelism (`PROPOSED...:243,3418`; `docs/ARCHITECTURE.md:666`) | **`rayon` is not a dependency at all** | `rg rayon Cargo.toml src/` → no hits |
| E | `parking_lot` for synchronization (`PROPOSED...:3419`) | Not a dependency | `Cargo.toml` |
| F | One module per type (`PROPOSED...:80-176`: `model/issue.rs`, `model/status.rs`, `storage/queries/*.rs`, `sync/export.rs`, `sync/import.rs`, `sync/merge.rs`, `config/loader.rs`, `error/format.rs`, `format/json.rs`, `format/tree.rs`, `format/table.rs`, `src/context.rs`) | Two god-modules and a flat layout: `src/storage/sqlite.rs` (5157 lines / 186 KB), `src/sync/mod.rs` (5139 lines / 184 KB), `src/model/mod.rs` (1494 lines, all types), no `src/context.rs`, no `storage/queries/`, no `sync/export.rs`/`import.rs`/`merge.rs` | `find src -name '*.rs' \| xargs wc -l` |
| G | License MIT (`8858ab7` "Add MIT License") | BSD 3-Clause | `LICENSE.md`, `Cargo.toml:8`, commit `b341573` |
| H | No git libraries in `Cargo.toml` (NGI-2, `.beads/SYNC_SAFETY_INVARIANTS.md:93`) | `vergen-gix = "9.1"` is a build-dependency (gitoxide); `UPGRADE_LOG.md:45` acknowledges "Various gix-* crates" | `Cargo.toml` `[build-dependencies]` |
| I | Distribution via crates.io / `cargo install` | **Impossible**: `org2jsonl = { path = "../org2jsonl" }` is a local path dependency to a sibling repo | `Cargo.toml`; `flake.nix:21-23` `url = "git+file:///Users/johnw/src/org2jsonl"` |

---

## 4. Development trajectory (git log, 598 commits)

Authors: `Dicklesworthstone <jeff141421@gmail.com>` 578 commits; `John Wiegley` 20 commits.

### Phase 0 — Specification-first (2026-01-15 → 01-16, ~59 commits)

Notably, the project started with **documentation, not code**. First commit `38cd152`
"Add AGENTS.md and comprehensive porting plan". Then `aeab8fc` moved the Go source into a
gitignored `legacy_beads/`, and `a376186` + 8 follow-ups built the 7937-line
`EXISTING_BEADS_STRUCTURE_AND_ARCHITECTURE.md` as a *substitute* for reading Go source.
`ec14cba` (2026-01-15) "Initialize Rust project" is the 11th commit.
`ecbb1ff` established the non-invasive philosophy and Gastown exclusion on day one.

### Phase 1 — Core implementation + sync-safety hardening (2026-01-16 → 01-17, ~200 commits)

Peak velocity: 158 commits on 2026-01-17 alone. Model types (`562e021`), CLI scaffold
(`16c98b8`), search/comments/doctor, then a concentrated safety burst:
`8a01571` → `934fc5f` → `3958782` → `cc605b2` → `90544e2` → `4d84e6f` → `6d30f92`
(sync safety plan → spec → flag matrix → allowlist → structured logging → path hardening →
export error policies). Config refactored from flags to subcommands (`da715c5`).
3-way merge implemented (`ee50802`, bead `beads_rust-07b`).

### Phase 2 — Conformance + test harness (2026-01-17 → 01-19, ~150 commits)

Scenario DSL, golden snapshots, dataset registry, `br`↔`bd` conformance harness
(`2634839`), cross-platform normalization, E2E coverage matrix (`71b11bd`), CI workflows
(`ce07c18`). Extensive "text output parity with bd" and "JSON shape parity" fixes.
**2026-01-19: the `bd sync` data-loss incident** (`b8020fa`, `a326295`).

### Phase 3 — Rich output + agent surfaces (2026-01-19 → 01-25, ~100 commits)

`2a5d635` adds `RICH_INTEGRATION_PLAN.md`; `1cbb7051`/`d85e89a`/`736a5ca` build the
`src/output/` foundation; then a per-command migration sweep (`b06dbcf` sync, `2df0736` dep,
`f3055cc` label, `eb6b57a` stats, `6a95245` comments, `cfbb07d` phase-5 batch). TOON output
(`b1882b8`), `schema` command (`9da03ba`), `docs/agent/` + `agent_baseline/` (`8684db7`),
`AGENT_FRIENDLINESS_REPORT.md` (`4507eda`), Nix flake (`d5e9821`).

### Phase 4 — Release engineering + long tail (2026-01-25 → 02-19, ~80 commits)

Versions 0.1.7 → 0.1.14. Shell completions (`4c2f107`, `676f7fb`, `70ec1de`), packaging
manifests (`b8e210f`), musl Linux builds (`7217ae0`), self-update signature verification
(`22b04e6`), performance PRAGMAs (`a97fac5`), atomic claim guard with IMMEDIATE transaction
(`0a52ac7`, `8df2de9`), Windows path handling via `dunce` (`4cf7717`), sync preflight
guardrails (`e539185`).

### Phase 5 — John Wiegley takeover (2026-02-18 → 03-16, 20 commits)

| Commit | Date | Change |
|---|---|---|
| `aa129a2` | 02-18 | Propagate errors instead of silent fallbacks in SQLite layer |
| `f826ac4` | 02-18 | Nix flake modernized: `rust-overlay` replaces `crane`+`fenix` |
| `9f1f2db` | 02-18 | Beads created/closed from code-review findings |
| `22adc72` | 02-18 | `CLAUDE.md` added |
| `afe1351` | 02-18 | **Org-mode replaces JSONL** (+800-line `src/sync/org_bridge.rs`) |
| `a0c5442` | 02-19 | doctor/stats fixed for Org format |
| `c046b35` | 02-19 | e2e tests migrated JSONL → Org |
| `a2760e6` | 02-19 | **`br` → `obr` rename** (partial doc coverage — see §6.1) |
| `03c5cd2` | 02-19 | Internal statuses mapped to standard Org keywords |
| `33bb2e8` | 02-19 | `org2jsonl` flake input for sandboxed builds |
| `b341573` | 03-16 | MIT → BSD 3-Clause |
| `a2ba738` | 03-16 | README rewritten (827 → 93 lines) |
| `1c32e5e` | 03-16 | `nix flake check` gates |
| `4cbf634` | 03-16 | lefthook pre-commit hooks |
| `5312cb5` | 03-16 | cargo-fuzz targets |
| `6a0518d` | 03-16 | CI workflows updated for `obr` |
| `5eb13fb`, `fcbbce2` | 03-16 | shfmt formatting; remove `.cargo/audit.toml` |

Observation: the takeover is a **productization** pass (license, README, hooks, fuzz, CI)
layered on top of a **format migration** (JSONL → Org). Neither pass touched the planning
documents or the agent-facing docs.

---

## 5. Open roadmap items

### 5.1 The beads tracker (`.beads/issues.jsonl`, 549 issues)

549 total: **545 closed, 1 tombstone, 3 open**. Types: 374 task, 83 feature, 42 epic,
41 bug, 8 chore, 1 `not_a_real_type` (a test artifact left in tracked data).

The three open issues (all P1–P3 tasks, created 2026-02-18 from a code review, unlabelled,
no dependencies):

| ID | P | Title | Substance |
|---|---|---|---|
| `beads_rust-2ckc` | 1 | Centralize 36-column SELECT list and `issue_from_row` | The 36-column SELECT is duplicated across 6+ query functions; `issue_from_row` uses fragile positional indices 0–35. A column reorder would silently corrupt data. Proposes `const ISSUE_COLUMNS` + named extraction. |
| `beads_rust-1lhm` | 3 | Add lifetime parameter to `MutationContext` to avoid actor cloning | `MutationContext` clones the actor `String` on every event record. `MutationContext<'a>` with `actor: &'a str` would remove the allocation. |
| `beads_rust-ddwo` | 2 | Migrate from unmaintained `serde_yaml` to `serde_yml` | `serde_yaml` 0.9 is archived; no security fixes. Supply-chain risk. Still present at `Cargo.toml` `serde_yaml = "0.9"`. |

The tombstone `beads_rust-1h4` ("Test reopen issue") is manual-test residue.

### 5.2 The productize plan (`docs/superpowers/plans/2026-03-16-productize.md`, untracked)

Seven chunks. Completion status verified against the tree:

| Chunk | Task | Status |
|---|---|---|
| 1 | BSD 3-Clause `LICENSE.md`, `Cargo.toml` license field | ✅ `b341573` |
| 1 | README rewrite in John's voice | ✅ `a2ba738` |
| 2 | `flake.nix` checks + dev tools | ✅ `1c32e5e` |
| 3 | `lefthook.yml` parallel pre-commit hooks | ✅ `4cbf634` (`lefthook.yml` has all 6 hooks) |
| 4 | cargo-fuzz targets | ⚠️ Done but diverged: plan named `fuzz_jsonl_import.rs` + `fuzz_issue_create.rs`; actual are `fuzz/fuzz_targets/{fuzz_jsonl_parse,fuzz_org_parse,fuzz_validation}.rs` (arguably better — covers the Org boundary) |
| **5** | **`scripts/coverage-check.sh` + `coverage-baseline.txt`** | ❌ **NOT DONE** — neither file exists |
| **5** | **Perf regression gate script** | ❌ **NOT DONE** — `scripts/bench_regression.sh` and `scripts/check_regression.py` pre-date the plan (`98a8a92`, 2026-01-20) and were not wired into a gate |
| 6 | CI mirrors lefthook, `br` → `obr` in artifacts | ✅ `6a0518d` (`.github/workflows/ci.yml:180-190` verifies `obr --version`) |
| 7 | shellcheck + shfmt on all scripts | ✅ `5eb13fb` |

Also mentioned in the plan header (`:9`, Tech Stack) but never implemented anywhere:
**Miri** — `rg miri .github/ flake.nix lefthook.yml scripts/` returns nothing.

### 5.3 Implicit roadmap from AGENT_FRIENDLINESS_REPORT

`AGENT_FRIENDLINESS_REPORT.md:66-71` — still open as of today:
- Dynamic `--help-json` surface (only static `CLI_SCHEMA.json` exists).
- Consistent `{data, metadata, errors}` envelope across commands (verified: `obr ready --json`
  returns a bare array).
- Deterministic schema output (`generated_at` breaks byte-equality).

---

## 6. Where the docs have DRIFTED from the code

All drift below was verified by reading code and/or executing `obr` against a throwaway
workspace at `/private/tmp/.../scratchpad/drifttest` (never against the repo's own `.beads/`).

### 6.1 The `br` → `obr` rename was applied to code and ~11 docs, not to the rest

`a2760e6` touched `README.md`, `docs/{AGENT_INTEGRATION,ARCHITECTURE,CLI_REFERENCE,
E2E_SYNC_TESTS,INSTALLING,SYNC_MAINTENANCE_CHECKLIST,SYNC_SAFETY,TEST_HARNESS,TROUBLESHOOTING}.md`,
`flake.nix`, `install.sh`, packaging, scripts, `src/`, `tests/` — and even then only a handful
of lines per doc (e.g. `docs/CLI_REFERENCE.md` got 6 changed lines out of 956).

Line counts of `br` vs `obr` mentions per file (lines containing each):

| File | `br` | `obr` |
|---|---:|---:|
| `AGENTS.md` | 47 | 0 |
| `CLAUDE.md` | 10 | 0 |
| `VCS_INTEGRATION.md` | 6 | 0 |
| `AGENT_FRIENDLINESS_REPORT.md` | 10 | 0 |
| `docs/TROUBLESHOOTING.md` | 107 | 3 |
| `docs/CLI_REFERENCE.md` | 86 | 3 |
| `docs/AGENT_INTEGRATION.md` | 68 | 7 |
| `docs/INSTALLING.md` | 32 | 2 |
| `docs/SYNC_SAFETY.md` | 18 | 7 |
| `docs/agent/*.md` (6 files) | 30 | 0 |
| `.beads/*.md` (4 files) | 61 | 0 |
| `skills/bd-to-br-migration/SKILL.md` | 44 | 0 |

Result: files that read like `docs/SYNC_SAFETY.md:9` — "**`br`** (beads_rust) is a
non-invasive issue tracker. The **`obr sync`** command…" — with both names in one sentence.

**Code is also only partially renamed.** `obr --help` still prints:
- `Agent-first issue tracker (SQLite + JSONL)` — the `Cargo.toml:6` description.
- `sync   Sync database with JSONL file (export or import)` — but the default is Org.
- `upgrade   Upgrade br to the latest version`.
- `--no-daemon   Force direct mode (no daemon) - effectively no-op in br v1`.

Internal identifiers keep the old name deliberately (`a2760e6` commit body: "Internal variable
names (BR_PATH, BR_BINARY, etc.) are preserved") — but so do user-visible ones like
`BR_OUTPUT_FORMAT` (`docs/agent/ROBOT_MODE.md:15`) and the crate name `beads_rust`.

### 6.2 The JSONL → Org migration is invisible in every document

**Not one document mentions `issues.org` or Org-mode.** `rg 'issues\.org|org.mode' README.md
AGENTS.md CLAUDE.md docs/ .beads/*.md` → zero hits.

Verified behaviour: `obr init` in a fresh directory creates `.beads/issues.org` (empty) and
`.beads/metadata.json` containing `{"database":"beads.db","jsonl_export":"issues.org"}` — note
the key is still literally `jsonl_export`. After `obr create "Test A" --type bug --priority 1`,
`.beads/issues.org` contains:

```org
#+TITLE: Beads Issues
#+SEQ_TODO: TODO DOING WAIT DEFER NOTE | DONE CANCELED

* TODO [#B] Test A
:PROPERTIES:
:BEADS_SCHEMA_VERSION: 1
:ID:       bd-cs8
:ISSUE_TYPE: bug
...
:END:
```

Mappings (`src/sync/org_bridge.rs`): status `Open|InProgress|Blocked|Deferred|Closed|
Tombstone|Pinned` → `TODO|DOING|WAIT|DEFER|DONE|CANCELED|NOTE` (`:262-273`, inverse at
`:276-287`); priority `0..4` → `[#A]..[#E]` (`:290-298`, inverse at `:301-309`).
Dependencies and comments are emitted as JSON `src` blocks inside the Org entry (`:20-26`).

Consequently, the following documented workflows are now **wrong**:
- `README.md:45-48` — "`obr sync --flush-only` exports the database to `.beads/issues.jsonl`".
- `VCS_INTEGRATION.md:38-52` — the entire "Conflict Handling (JSONL)" section, plus the
  per-VCS tables for git/hg/jj/p4/sl, all reference `.beads/issues.jsonl`.
- `docs/TROUBLESHOOTING.md` — 16 lines referencing `issues.jsonl`.
- `docs/ARCHITECTURE.md:286-298` — "JSONL Format: each line is a complete JSON object".
- `.beads/SYNC_CLI_FLAG_SEMANTICS.md`, `docs/SYNC_SAFETY.md`, `docs/E2E_SYNC_TESTS.md`.

The code hedges correctly (`src/config/mod.rs:100-118` prefers `.org`, falls back to `.jsonl`;
`src/sync/path.rs:48-56` allowlists both `org`/`org.tmp` and `jsonl`/`jsonl.tmp`), so mixed
workspaces work — but no reader of the docs would know Org exists. **This repo's own
`.beads/` is still JSONL** (`.beads/issues.jsonl`, 549 lines; no `issues.org`), which means the
project's self-tracking exercises only the legacy path.

### 6.3 "Never executes git commands" is overstated

`README.md:140` states flatly "Never executes git commands". `CLAUDE.md:52` says "`br` never
executes git commands automatically". `AGENTS.md:352` says "NEVER executes git commands
automatically".

The precise invariant is narrower and is correctly scoped in `src/validation/mod.rs:404-415`
("No `std::process::Command::new("git")` **in sync module**") and
`.beads/SYNC_SAFETY_INVARIANTS.md:91` (NGI-1, "**br sync** NEVER executes git subprocess
commands"). `obr` **does** shell out to git elsewhere, all read-only:

- `src/cli/commands/changelog.rs:290` — `git show -s --format=%cI <ref>`
- `src/cli/commands/orphans.rs:234` — `git rev-parse --git-dir`
- `src/cli/commands/orphans.rs:245` — `git log --oneline HEAD`
- `src/cli/commands/stats.rs:381` — `git log --oneline --since <t> -- <path>`
- `src/cli/commands/comments.rs:352` — `git config --get user.name`

None mutate the repo, so the *spirit* holds. But the README's unqualified claim is false as
written, and a security reviewer auditing against it would find a discrepancy.

### 6.4 `docs/ARCHITECTURE.md` — five concrete inaccuracies

| Doc claim | Reality |
|---|---|
| `:339-340` `ALLOWED_EXTENSIONS = [".jsonl",".json",".db",".yaml"]`, `ALLOWED_EXACT_NAMES = ["metadata.json","config.yaml"]` | `src/sync/path.rs:48-59`: extensions are `db, db-wal, db-shm, jsonl, jsonl.tmp, org, org.tmp` (no leading dots, no `.json`, no `.yaml`); exact names are `.manifest.json, metadata.json` — **`config.yaml` is NOT sync-writable** |
| `:249,270` table `blocked_cache` | Actual table is `blocked_issues_cache` (`src/storage/schema.rs:180`) |
| `:240-251` table list omits three tables | `src/storage/schema.rs` also creates `metadata` (`:157`), `export_hashes` (`:171`), `child_counters` (`:189`) |
| `:666` dependency `rayon` | Not in `Cargo.toml`; no `rayon` anywhere in `src/` |
| `:120-124` `format/` = `{mod,text,output,csv}.rs`; no `output/` module listed | Actual `src/format/` = `{mod,text,output,csv,markdown,rich,syntax,theme,context}.rs`, plus an entire `src/output/{mod,context,theme,components/}` tree that the doc never mentions |
| `:466` `#[command(name = "br")]` | `src/cli/mod.rs:652` says `"obr"` |
| `:38` "Lines of Code ~33k" | `find src -name '*.rs' \| xargs wc -l` = 53 493 |

### 6.5 `README.md` — three reproducible errors

1. **`--priority 0-1` does not work.** `README.md:57` shows
   `obr list --status open --priority 0-1`. Executing it yields:
   ```
   {"error":{"code":"INVALID_PRIORITY","message":"Priority must be 0-4, got: -1", ...}}
   ```
   `--priority` is a repeatable single value (`obr list --help`); ranges use
   `--priority-min` / `--priority-max`.

2. **The config example uses key names the code does not recognise.** `README.md:79-89` shows
   nested `id.prefix`, `defaults.priority`, `defaults.type`, `sync.auto_import`,
   `sync.auto_flush`. Verified: writing that exact YAML and running `obr create` produced
   `bd-2do` (default prefix), not `proj-…`. Writing flat `issue_prefix: "proj"` produced
   `proj-3id`. Recognised keys per `src/config/mod.rs:427,845,869,879` are
   `issue_prefix|issue-prefix|prefix`, `default_priority|default-priority`,
   `default_type|default-type`. Subtle trap: `obr config get id.prefix` *did* return `"proj"`
   because the YAML flattener dot-joins nested keys — so the key is *readable* but never
   *consulted* by ID generation.

3. **"Listing 10,000 issues takes under 100ms"** (`README.md:43`) is an unsourced performance
   claim. `benches/storage_perf.rs` and `tests/bench_*.rs` exist but nothing in the repo ties
   this number to a measurement, and no regression gate enforces it (§5.2).

Minor: `README.md:45-48` implies export is manual, but auto-flush is **on by default** —
`obr create` emitted `INFO beads_rust::sync: Auto-flush complete exported=1` without any
sync command. `--no-auto-flush` disables it.

### 6.6 `docs/CLI_REFERENCE.md` — missing and wrong commands

- **Undocumented commands** (present in `obr --help`, absent from the reference):
  `graph`, `agents`, `schema`, `status` (alias for `stats`), `undefer` (only `defer / undefer`
  as a combined heading).
- **`config` interface is wrong.** `:666-702` documents flag-style
  `-l/--list, -g/--get, -s/--set, -d/--delete, -e/--edit, -p/--path, --project, --user`.
  Actual (`obr config --help`) is subcommand-style: `list, get, set, delete (alias unset),
  edit, path`. `obr config --list` errors with `unexpected argument '--list' found`.
  This regressed with `da715c5` (2026-01-17, "Refactor config command from flags to
  subcommands") — the doc was never updated, so it has been wrong for ~7 months.
- **`sync --merge` and `sync --rename-prefix` undocumented.** `:632-637` lists only
  `--flush-only`, `--import-only`, `--status`. `obr sync --help` also has `--merge`
  (3-way merge using `.beads/base_snapshot.jsonl`) and `--rename-prefix`.
  `--merge` appears only in `docs/E2E_COVERAGE_MATRIX.md:129`; `--rename-prefix`
  (added `70ec1de`, 2026-01-25) appears in **no** document.

### 6.7 `AGENTS.md` — structural claims that no longer hold

- `:265` `storage/queries/` "Reusable query fragments" — **directory does not exist**
  (`src/storage/` is `{mod,sqlite,schema,events}.rs`).
- `:262,318` "sqlite.rs (181KB)", "sync/mod.rs (176KB)" — now 186 KB and 184 KB. Harmless.
- `:203,353` "stores issues in SQLite with JSONL export" — superseded by Org.
- `:66` lists `rich_rust` as a dependency with no version; `Cargo.toml` pins `0.2.0` from
  crates.io (`9ef0356` moved it off a git ref).
- `:486` "bv is a graph-aware triage engine for Beads projects (`.beads/beads.jsonl`)" —
  wrong filename even for the JSONL era (`issues.jsonl`), and `bv` is not installed here.
- `:714-777` contains an auto-generated `<!-- bv-agent-instructions-v1 -->` block that
  duplicates and partially contradicts the hand-written sections above it (e.g. it tells
  agents to run bare `bv`, then says to avoid it).

### 6.8 `CLAUDE.md`

- `:7` "Binary name: `br`", `:19` "Build the br binary via Nix", `:62-67` all `br` commands.
- `:30` "cargo test --lib --bins (781 tests)" — `rg -c '#\[test\]' src/` now counts 788.
- `:42` "`src/sync/mod.rs` — JSONL import/export engine" — Org now.
- `:51` "SQLite + JSONL hybrid" — Org now.
- Line counts at `:41-42` ("~5K lines" each) are accurate (5157 / 5139).

### 6.9 `.beads/README.md` is upstream Go boilerplate

Generated by `bd init` and never replaced. It teaches `bd create`, `bd list`, `bd sync`
(which "Auto-syncs with your commits" and does "Automatic sync with git commits") and links
to `github.com/steveyegge/beads`. Every one of those behaviours is an explicit **non-goal**
of this project (`.beads/SYNC_SAFETY_INVARIANTS.md:11-25`). Anyone opening `.beads/README.md`
gets instructions that would violate the project's central safety invariant.

### 6.10 `docs/INSTALLING.md` and `install.sh` point at the wrong repo

- `docs/INSTALLING.md` shows `cargo install --git https://github.com/Dicklesworthstone/beads_rust.git`
  **ten times**. That installs upstream `br`, not this fork's `obr`.
- `install.sh:33-34`: `REPO="${REPO:-beads_rust}"` but `BINARY_NAME="obr"` — it would look for
  `obr-*` release artifacts in a repo that publishes `br-*`.
- `src/cli/commands/upgrade.rs:15,18`: `REPO_OWNER = "Dicklesworthstone"`,
  `REPO_NAME = "beads_rust"` — `obr upgrade` self-updates from **upstream**, which would
  replace John's fork with Jeffrey's binary.
- `Cargo.toml:8` `repository = "https://github.com/Dicklesworthstone/beads_rust"`.

### 6.11 `docs/agent/*` — the agent entry points are the stalest tier

All six files predate both renames. Concretely broken instructions:
- `docs/agent/QUICKSTART.md:9` `br init --prefix bd`
- `docs/agent/QUICKSTART.md:47` "Export JSONL for git commit" → `br sync --flush-only`
- `docs/agent/SCHEMA.md:29` `./target/debug/br schema all --format json` (binary is `obr`)
- `docs/agent/ROBOT_MODE.md:15` env var `BR_OUTPUT_FORMAT` — this one is still correct
  (verified in `obr ready --help`).

The *semantic* claims all still verify: `--claim` exists (`obr update --help`,
"Atomic claim (assignee=actor + status=in_progress)"); `--format toon` works; `obr schema`
supports `all|issue|issue-with-counts|issue-details|ready-issue|stale-issue|blocked-issue|
tree-node|statistics|error`; stdout is clean JSON with logs on stderr (verified:
`obr ready --json 2>/dev/null` emits a bare, valid JSON array).

### 6.12 Committed cruft the docs never mention

`git ls-files` tracks: `custom.db` (176 KB SQLite), `temp_test/.beads/` and
`temp_test_2/.beads/` (leftover workspaces, including a `daemon.log` from the Go tool),
`br_illustration.webp` (352 KB, orphaned — the README that referenced it was deleted in
`a2ba738`), and `.ntm/logs/{am,bd,cm}-beads_rust.log`.

---

## 7. Notable tensions and oddities

1. **Two documents give contradictory instructions on compatibility shims.**
   `PROPOSED...:3447` fallback strategy: "If JSONL format diverges: Versioned JSONL format
   with compat shims." `AGENTS.md:116-121`: "Never create 'compatibility shims'… we're in
   early development with no users." The Org migration in practice split the difference —
   `src/config/mod.rs:100-118` *is* a compat shim (prefer `.org`, fall back to `.jsonl`).

2. **`AGENTS.md` is really three documents fused.** Project rules (§Toolchain..§Output Modes),
   third-party tool manuals for a specific multi-agent environment (`bv`, UBS, RCH,
   `warp_grep` with `repoPath: "/dp/beads_rust"`, MCP Agent Mail), and an auto-generated
   `bv-agent-instructions-v1` block. Much of it describes infrastructure that does not exist
   on this machine (`bd`, `bv`, `ubs`, `rch` are all absent from `$PATH`). `AGENTS.md:794-810`
   even contains a personal rant addressed to "Codex/GPT-5.2" about concurrent-agent file
   churn — reading it, this repo was developed by a *swarm* of agents editing simultaneously
   (corroborated by ~20 commits literally titled "Sync concurrent agent changes").

3. **The specification-first approach was unusually rigorous and it worked.** Writing a
   7937-line spec of the Go codebase *before* porting, then never consulting Go source again,
   is the single most distinctive process choice here. Section 15 alone has 98 numbered
   behavioural sub-specs. The conformance suite (`tests/conformance*.rs`) then validated the
   result against the real `bd`.

4. **The documentation set has a clear reliability gradient**, from most to least trustworthy:
   `.beads/SYNC_*.md` and `docs/SYNC_*.md` (invariants are mechanically enforced by
   `src/validation/mod.rs:404-415` and `tests/e2e_sync_*.rs`) > `docs/ARCHITECTURE.md` >
   `docs/CLI_REFERENCE.md` > `README.md`/`CLAUDE.md` > `AGENTS.md` > `docs/agent/*` >
   `.beads/README.md` (actively harmful). Ironically the newest doc (`README.md`, rewritten
   2026-03-16, i.e. *after* the Org migration) is one of the least accurate, because it was
   rewritten from the old README's content rather than from the code.

5. **Build reproducibility is machine-local.** `Cargo.toml` `org2jsonl = { path = "../org2jsonl" }`
   and `flake.nix:22` `url = "git+file:///Users/johnw/src/org2jsonl"`. The Nix flake handles
   this by copying `${inputs.org2jsonl}` into the sandbox (`flake.nix:59-60,102-103`), but
   **GitHub Actions has no provisioning for it** — `rg org2jsonl .github/` returns nothing,
   so `.github/workflows/ci.yml`'s `cargo fmt`/`cargo clippy`/`cargo check` steps would fail
   on a clean checkout. Only `nix flake check` (and therefore only the lefthook `nix-check`
   hook) can actually build this tree today.

6. **The `bd sync` incident is the origin myth and it is genuinely documented.** Most projects
   write threat models speculatively; this one wrote it after `a326295` "fix: restore all files
   destroyed by bd sync virus". That explains why the sync docs are the highest-quality tier.

7. **`serde_yaml` is archived** and flagged by an open bead (`beads_rust-ddwo`) — yet
   `.github/workflows/ci.yml:65` runs `cargo audit --deny warnings`, which suggests either the
   advisory is not yet filed or CI is not currently green.

8. **`obr` is installed system-wide** at `/etc/profiles/per-user/johnw/bin/obr` (v0.1.14),
   so the tool is in daily use — but the repo's own tracker has not been touched since
   2026-02-18 (`.beads/last-touched`, `.beads/issues.jsonl` mtime), i.e. the project stopped
   dogfooding its own issue tracking right as the Org migration landed.

---

## 8. Suggested reading order for other agents

- To understand **why** anything is the way it is: `PLAN_TO_PORT...md:1-150` then `:371-441`.
- To understand **safety constraints** (the only invariants actually enforced):
  `.beads/SYNC_SAFETY_INVARIANTS.md` + `src/sync/path.rs:1-60` + `src/validation/mod.rs:400-430`.
- To understand **current behaviour**: run `obr <cmd> --help`. Do not trust
  `docs/CLI_REFERENCE.md` for `config` or `sync` flags.
- To understand **the storage format**: `src/sync/org_bridge.rs` — no prose document covers it.
- To understand **what's left**: the 3 open beads (§5.1) + Chunk 5 of
  `docs/superpowers/plans/2026-03-16-productize.md` (§5.2).
- Ignore for current purposes: `.beads/README.md`, `agent_baseline/help/*`,
  `skills/bd-to-br-migration/`, `EXISTING_BEADS_STRUCTURE_AND_ARCHITECTURE.md` (a spec of the
  *Go* tool, useful only for conformance archaeology).
