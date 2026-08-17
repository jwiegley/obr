# CRITIQUE — adversarial completeness review of `INTEGRATION_PLAN.md`

**Role:** Phase-1/2 completeness critic (read-only, adversarial).
**Target:** `/Users/johnw/src/obr/docs/research/upgrade/INTEGRATION_PLAN.md` (1,697 lines, 2026-08-06 14:08).
**Also read in full:** `UPSTREAM_SYNC_AND_RENAME_PLAN.md`, `upgrade/BASELINE.md`, `upgrade/R1`–`R4`,
`docs/research/DOSSIER.md` §5 index.
**Upstream worktree used for every anchor check:** `/Users/johnw/src/obr-org-forward-port` @ `d1fb0d3a`
(read-only; no builds run).

**Evidence convention.** Bare `path:line` = upstream worktree @ `d1fb0d3a`. `fork:path` =
`/Users/johnw/src/obr` @ `main`. **[V]** = I read/ran/grepped it myself in this session.
**[I]** = inferred. Every material claim below is **[V]** unless marked otherwise.

**Verdict: NOT complete.** The plan's *anchor quality is excellent* (24 of 24 spot-checked anchors
landed within ±4 lines — §6 below) and its risk register covers all 32 dossier risks. But it has
**three blocking defects** (a superseded test baseline wired into every gate; a Phase-5 sequence that
silently leaves the fork's tracker in JSONL; and no commit that owns the repo's own `.beads/`
directory), **ten major gaps**, and a set of smaller omissions. Nothing here is a matter of taste;
each item names the file, the line, and the commit that should own it.

---

## 0. Summary table

| # | Sev | Finding | Where it must be fixed |
|---|---|---|---|
| **B1** | **Blocker** | §0.1 and §1 are built on the **superseded v1 baseline** (9 unit failures, "clippy never lints first-party code"). `BASELINE.md` + `baseline/v3-*.txt` — written 4 minutes *earlier* and declared durable — record **266** unit failures, **4 process-aborting tests**, **85 first-party clippy errors**, and a canonical fenix toolchain. Every gate in §1 inherits the wrong reference set. | §0.1, §0.2, §1 G-lint, §1 G-unit |
| **B2** | **Blocker** | **P5-01 → P5-02 cannot work as written.** Copying `fork:.beads/metadata.json` (`"jsonl_export": "issues.jsonl"`) pins the tracker to JSONL through the exact `resolve_jsonl_path` priority-2 mechanism P3-09 documents. P5-02's flush emits `issues.jsonl`, not `issues.org` — silently. | P5-01/P5-02 |
| **B3** | **Blocker** | **No commit owns the repo's own `.beads/` directory.** `git mv .beads .obr` appears nowhere; 10 tracked files (5 branded `.md` docs, 18–61 hits each; `.beads/.gitignore`; `config.yaml` with `issue_prefix: beads_rust`; `metadata.json` naming `beads.db`; `interactions.jsonl`) are unassigned and unallowlisted. G-rename parts 1 and 3 fail on all of them. R4 §5.8 warned about exactly this; map row A14 says the opposite. | P5-01/02, P6-01a, §1 allowlist |
| **M1** | Major | **Six verified production Org-awareness anchors are missing**, incl. `cli/commands/history.rs:879` and `doctor_subsystems/surface.rs:413`. R2's own closing sweep (Q5) — the check that finds them — is not in the plan, and P3-10 cites a nonexistent `[R2 §Q6]`. | P3-11, P3-12, P3-10, Phase 7 |
| **M2** | Major | **No gate after P4-05 ever compiles `fuzz/` or `benches/`**, both of which `use beads_rust::`. `cargo build`, `--lib --bins --tests` clippy, and `--test <name>` all exclude them; `--all-targets` is deliberately retired. P6-02/P6-03 can ship a broken fuzz crate. | §1 gates, P6-02, P6-03, CLEANUP |
| **M3** | Major | `tests/legacy_compat.rs` is allowlist row **L7** but **no commit creates it**. ~20 legacy-read shim rows have no test deliverable and no done-criteria. | P6-01a |
| **M4** | Major | **Phase 7 is not concretized**, contrary to the plan's remit. Clone-simulation import cycle, "legacy `.beads` opens with a warning", and the full serial G-unit procedure appear nowhere. | new §, or CLEANUP |
| **M5** | Major | **12 of 32 commits lack required metadata** (files / source-of-truth / done-criteria). All of Phase 6 is table rows with no done-criteria. Header says "Twenty-nine commits" against 32 rows. | §3, §5.2, §6 |
| **M6** | Major | Root `.gitignore` (18 branded lines), `.ubsignore`, **root `UPGRADE_LOG.md`** (a *different file* from `docs/operations/UPGRADE_LOG.md`, 6 hits, allowlisted by neither) are unowned; G-rename part 1 fails. | P6-01a / P6-04 / §1 allowlist |
| **M7** | Major | Fork commit `22adc72b` (`CLAUDE.md`) has **no home in the series** — upstream has no `CLAUDE.md`, yet CLEANUP says "refresh CLAUDE.md facts". | CLEANUP or P4-x |
| **M8** | Major | **Plan §8 decisions silently resolved**: §8.2 (`obr.db`, `.obr/history/`) became map rows with recommendations, not §9 rows; §8.3 (landing) is absent from §9 entirely. | §9 |
| **M9** | Major | **G-locked inconsistency**: master table applies it only to P3-01; `§8 R14` says "on every commit"; `Cargo.lock` also changes at **P6-02** (package rename) with no G-locked gate there. | §1, master table, P6-02 |
| **M10** | Major | Risk-register **summary sentence omits R11, R15, R19, R20, R22, R26, R27** from its buckets (the 32 table rows themselves are complete). | §8 summary |
| **m1–m9** | Minor | R2 Q1/Q2 never surfaced; dangling `[R2 §Q6]`; `self_update` ↔ `agent_baseline` interaction; P6-01b fixture counts 2–3× low vs disk; `docs/ORIGINS.md` allowlisted but never created; CI-runner constraint noted but unowned; U-EXTERNAL blocks P3-03 with no recommendation; JSONL record-kind question never closed in writing; `interactions.jsonl` unmentioned. | §7 below |

---

## 1. Blockers

### B1 — the plan is wired to the wrong baseline

**What the plan says.** §0.1: unit = "**exit 5, 9 named failures**"; lint = "exit 101 — 28 ×
`error[E0658] cfg_select` … `could not compile sysinfo`"; and the load-bearing conclusion
*"Clippy therefore **never lints the workspace's own code** under `--all-targets`"* **[quoted from
INTEGRATION_PLAN.md:79-95]**. §1 G-unit diffs against
`…/scratchpad/baseline/unit-failures-baseline.txt`.

**What is actually of record.** `docs/research/upgrade/BASELINE.md` (14:04, four minutes *before*
the plan) plus the six durable inventories in `docs/research/upgrade/baseline/`:

| | INTEGRATION_PLAN §0.1 (v1 run, 13:38) | BASELINE.md + `baseline/v3-*` (13:55–14:03) |
|---|---|---|
| toolchain | ambient / "1.99.0-nightly" | pinned `fenix#…a7c3ef79…complete.toolchain`, and **`cargo clippy`/`cargo fmt` are unusable machine-wide** — rustup shims in `$CARGO_HOME/bin` resolve before PATH to a stale Feb-2026 nightly; must invoke `cargo-clippy` directly |
| unit failures | **9** | **266** (complete, serial, aborters skipped) — `v3-unit-failures-complete.txt` = 266 lines **[V]** |
| aborting tests | not mentioned | **4** `doctor::tests::execute_repair_indexes_*`, each kills the harness with exit 5 and truncates the run |
| clippy | aborts in third-party `sysinfo`; "never lints first-party code" | **85 first-party errors** (24 pedantic, 6 nursery, 6 `-D warnings`, …) — `v3-clippy-lints-baseline.txt` **[V]** |
| e2e | 3 / 4 / 23 | 3 / 4 / 23 (identical — the only agreeing row) |

I verified the primary artifacts myself: `wc -l scratchpad/baseline/unit-failures-baseline.txt` = **9**;
`wc -l docs/research/upgrade/baseline/v3-unit-failures-complete.txt` = **266**;
`summary-v3.txt` also prints "unit: 9 failures" **because the parallel run aborts** — which is
precisely why `run-unit-complete.sh` (14:00) was written and why BASELINE.md documents a two-step
procedure. **[V]**

**Why this is a blocker, not bookkeeping.** §1's G-unit is
`cargo test --lib --bins | grep FAILED | diff <baseline>` with no `--test-threads=1` and no
`--skip` of the four aborters. Two concrete failure modes:

1. If the harness aborts at the same point, 257 pre-existing failures stay invisible and **a genuine
   new failure hidden behind the abort passes the gate**.
2. Org work changes timing and ordering. The moment the abort point moves, dozens of *pre-existing*
   failures surface at once and are read as NEW → red gate → the executor spends a session chasing
   upstream's breakage, which plan §1 explicitly forbids.

**Also broken by the same root cause:**
- §1 G-lint says "On first use, record the output … that becomes the true reference." A reference
  **already exists** (`v3-clippy-lints-baseline.txt`), and it proves first-party code *is* linted
  and *is* dirty. The plan's "zero new warnings attributable to files this commit touched" is
  therefore operating on a false premise about what the baseline contains.
- Every gate command in §1 says `cargo clippy`. Per BASELINE.md that invocation is broken on this
  host. The plan never mentions the shim problem or the `cargo-clippy`-inside-`nix shell` workaround.
- §0.2 pre-flight bullet 6 says "`nix develop` (or nightly per `rust-toolchain.toml`)". BASELINE.md
  §3 records that the fork's dev shell pins nightly 1.95.0 which **cannot build upstream's deps**
  (`sysinfo 0.39.6` needs then-unstable `cfg_select`) and that upstream's own flake devShell is
  broken on Darwin in two independent ways. The pre-flight step, followed literally, fails.
- Every baseline path cited in §0.1/§1 is under
  `/private/tmp/claude-501/…/scratchpad/baseline/` — the governing plan §9 warns these "may vanish
  on reboot". The durable copies in `docs/research/upgrade/baseline/` are never referenced.

**Fix.** Rewrite §0.1 from BASELINE.md; repoint every gate at
`docs/research/upgrade/baseline/v3-*.txt`; adopt BASELINE.md's two-step G-unit (fast parallel check
+ serial `--test-threads=1 --skip <4 aborters>` at Phase 7 minimum, with "a new aborter is also a red
gate"); replace `cargo clippy` with the `nix shell … --command cargo-clippy` form; replace the
`nix develop` pre-flight with the fenix pin.

### B2 — Phase 5 leaves the tracker in JSONL, silently

P3-09 states the mechanism correctly and even calls it "the trap":

> Flipping the default silently reclassifies every existing `"jsonl_export": "issues.jsonl"` … from
> "the default" to "an explicit override". That is **correct and desirable**: legacy workspaces keep
> reading their JSONL and do not silently change format. Do not "fix" it.
> — INTEGRATION_PLAN.md:742-747

P5-01 then says: *"Copy `main`'s `.beads/issues.jsonl` + `config.yaml` + **`metadata.json`** onto the
branch"*, and P5-02 says: *"`obr sync --flush-only` with the Org default; the tracker lands as
`.beads/issues.org`."*

It will not. **[V]** `fork:.beads/metadata.json` is exactly:

```json
{ "database": "beads.db", "jsonl_export": "issues.jsonl" }
```

Copied verbatim, that file makes the fork's own workspace an explicitly-JSONL-pinned workspace by
the same `resolve_jsonl_path` priority-2 rule (`src/config/mod.rs:4219`, verified present **[V]**)
that P3-13's `pin_jsonl` helper uses deliberately. P5-02's flush writes `issues.jsonl`; its
done-criterion ("`obr sync --import-only` into a scratch DB reproduces every issue") passes anyway,
because JSONL round-trips fine. **The failure is silent and the gate cannot see it.** The project's
own tracker — the thing plan §7.4 says "loss = failure" — ends up in the format the fork exists to
replace.

Two more Phase-5 defects in the same neighbourhood:

- The copied `metadata.json` also says `"database": "beads.db"`. It is a **tracked** file, so after
  the A3 rename it trips G-rename part 3 (`beads\.db`) and is not on the allowlist. Phase 5 must
  rewrite both keys, or delete `metadata.json` and let `init`/discovery regenerate it.
- P5-01's done-criterion is *"`obr count` reports 549 issues, 3 open. `beads.db` migrates 1 → 17 on
  first open"*. The 1 → 17 claim is correct (`fork:src/storage/schema.rs:5` = `1`,
  `src/storage/schema.rs:11` = `17` **[V]**) but presupposes carrying the **untracked, gitignored**
  `fork:.beads/beads.db` across. The governing plan permits executing in a separate worktree, where
  that file does not exist. State explicitly: *do not copy the DB; import 549 issues from JSONL into
  a fresh DB* — which is both reproducible and the safer path.

### B3 — nobody owns the repo's own `.beads/` directory

`git mv .beads .obr` — or any equivalent — appears **nowhere** in the 32-commit series. P6-01a's
file list is "**54 `src/` files**"; P6-01b is `tests/`; P6-04 is `docs/`, `AGENTS.md`, `packaging/`,
`install.sh`, workflows, `scripts/`. The workspace directory itself is orphaned.

What is actually sitting there, tracked, on `upstream/main` **[V]** (`git ls-files .beads`, 10 files):

| File | Branding hits | Assigned to |
|---|---|---|
| `.beads/SYNC_CLI_FLAG_SEMANTICS.md` | 61 | — |
| `.beads/SYNC_THREAT_MODEL.md` | 32 | — |
| `.beads/README.md` | 24 | — |
| `.beads/SYNC_SAFETY_INVARIANTS.md` | 23 | — |
| `.beads/MCP_AGENT_MAIL_PATTERNS.md` | 18 | — |
| `.beads/.gitignore` | `bd.sock`, `beads.{base,left,right}.jsonl`, `.br_history/`, `.bv.lock`, "bv (beads viewer)" | — |
| `.beads/config.yaml` | `issue_prefix: beads_rust` (+ ~20 branded comment lines in the fork's copy) | — |
| `.beads/metadata.json` | `beads.db` | — (see B2) |
| `.beads/interactions.jsonl` | upstream's own 3 interaction records | **never mentioned anywhere** |
| `.beads/issues.jsonl` | upstream's 953-issue tracker | D-UPSTREAMTRACKER (decision only) |

C25 correctly notes the five `.md` files are upstream's and "arrive from `upstream/main` for free, in
their newer form" — but *arriving* is not *disposing of*. They describe **this tool** in beads
vocabulary, so under the mandate they must be de-branded or deleted, and they must move into `.obr/`.

**R4 §5.8 called this out in terms the plan then contradicted:**

> Upstream's own tracker uses prefix `beads_rust` … This is a **created artifact carrying the beads
> name** and **the very first thing the G-rename audit will trip over**.

Map row **A14** instead lists `config.yaml` under *"**KEEP** — already unbranded"*. The filename is
unbranded; the contents are not. And D-PREFIX only covers issue **IDs**, not the `issue_prefix` /
`issue-prefix` config keys that mint future ones — so under D-PREFIX option (ii) ("keep historical
IDs"), **new** issues would keep being minted as `beads_rust-*` after the rename.

**Fix.** Add an explicit commit (or extend P6-01a) that: `git mv .beads .obr`; rewrites
`.obr/config.yaml` (`issue_prefix: obr`, de-branded comments); rewrites `.obr/metadata.json`;
rewrites `.obr/.gitignore` from the A16 template; disposes of the five `.md` docs (de-brand or
delete — this is arguably a §9 decision); and decides `interactions.jsonl`. Then extend the G-rename
allowlist, or the pathspec, accordingly.

---

## 2. Major gaps

### M1 — six missing production anchors, and the sweep that would have found them

The plan's Org-awareness site list is drawn from R1/R2 and is very good inside `sync/`. It is
incomplete outside it. `grep -rn 'issues\.jsonl' src --include='*.rs'` = **515 hits across 25
files** **[V]**. Most are in `mod tests`. These are not:

| Site | Code | Plan's coverage |
|---|---|---|
| `src/cli/commands/history.rs:879` | `let default_target = beads_dir.join("issues.jsonl");` — the `history restore` default target (tests start at `:955`, so this is production) | **None.** P3-11's file list is `src/sync/history.rs` only. `obr history restore` in an Org workspace targets the wrong file. |
| `src/cli/commands/doctor_subsystems/surface.rs:413` | `let jsonl_present = beads.join("issues.jsonl").is_file() \|\| beads.join("beads.jsonl").is_file();` (tests start `:1992`) | P3-12 says "**possibly** `doctor_subsystems/`" and gives **no anchor**. Doctor's surface report says "no export present" for every Org workspace. |
| `src/mcp/mod.rs:120` | `"recovery": "Run br sync --flush-only … before committing .beads/issues.jsonl"` | **None.** P3-07 de-hardcodes only `cli/commands/mod.rs:86-133`. |
| `src/cli/mod.rs:962`, `:2865` | `br sync --flush-only  Export database to .beads/issues.jsonl` and `/// Export database to JSONL (DB → .beads/issues.jsonl)` | Only `:2915` (witness help) is assigned, in P3-10. |
| `src/cli/commands/robot_docs.rs:17` | agent-facing guide text: "exports `.beads/issues.jsonl` for git-friendly handoff" | None (G-rename part 4 catches the `.beads` half at P6-04; the format claim stays wrong). |
| `src/cli/commands/capabilities.rs:703` | published example `"br dep import .beads/issues.jsonl --robot"` | E18 covers `ENV_VARS` at `:213-237` only. |

**The systemic point.** R2 §9 Q5 asked exactly the right closing question:

> **Q5 — does anything in upstream write to the export file outside the paths audited here?** …
> A `rg -n 'durable_rename|write_all|fs::write' src/ | rg -i jsonl` sweep before the port declares
> done would close it.

That sweep is **absent from the plan** — not in P3-13, not in CLEANUP, not in Phase 7. Worse, the
plan cites `[R2 §Q6]` at P3-10 (INTEGRATION_PLAN.md:774); **R2 §9 has Q1–Q5 only** **[V]** — a
dangling citation on the one bullet that was supposed to catch hardcoded "JSONL" strings.

**Fix.** Add the six anchors to P3-10/P3-11/P3-12; make R2's Q5 sweep a numbered, gated step in
Phase 7 with its exact command.

### M2 — `fuzz/` and `benches/` are outside every gate after P4-05

**[V]** `fuzz/Cargo.toml:2` is `name = "beads_rust-fuzz"`, `:11` is
`beads_rust = { path = "..", default-features = false }`, and all 7 fuzz targets `use beads_rust::…`.
`benches/storage_perf.rs:22-23,92` likewise.

Now look at what the gates compile:

| Gate | Compiles `benches/`? | Compiles `fuzz/`? |
|---|---|---|
| G-build `cargo build` | no (needs `--benches`) | no (separate crate) |
| G-lint `cargo clippy --lib --bins --tests` | **no — explicitly excluded** | no |
| G-unit `cargo test --lib --bins` | no | no |
| G-e2e `cargo test --test X` | no | no |
| `cargo clippy --all-targets` | yes — but §1 declares it "**unusable as a hard gate**" | no |
| `cargo fuzz build` | — | yes, **but only at P4-05**, which runs *before* Phase 6 |

So P6-02 (`beads_rust` → `obr`) and P6-03 (11,812 internal tokens) can leave the fuzz crate and the
benches uncompilable, and **the series will go green through CLEANUP**. This also silently defeats
P4-05's own deliverable (`org_parse` fuzz target — the fork's only fuzz coverage of its new input
boundary).

**Fix.** Add `cargo build --benches` (or `--all-targets` under the fenix pin, where BASELINE.md
proves it *does* compile through to first-party lints) and `cargo fuzz build` to the P6-02, P6-03 and
CLEANUP gates; add `fuzz/Cargo.toml` and `benches/*.rs` to P6-02's file list explicitly.

### M3 — the compat shims have no test deliverable

Allowlist row **L7** reads: *"`tests/legacy_compat.rs` — The single file exercising L3–L6."*
That file **does not exist upstream** **[V]** and **no commit creates it**. P6-01a's prose says only
"All LR shims go in one `mod legacy_compat` region".

The map declares roughly twenty **LR** (legacy-read-with-warning) rows: A1, A2, A3, A4, A6, A7, A8,
A13, A17, E1–E14, D (the four-tier user-config chain), M1 (six on-disk schema IDs), M7. Each is *new
code* — §5.1 itself notes "**There is no deprecation machinery upstream**", `git grep -in deprecat --
src/` = 0 hits. Twenty new legacy paths, all reachable only by users with old workspaces, with **no
assigned tests and no done-criteria**, is the single largest untested surface the port introduces.

**Fix.** Make `tests/legacy_compat.rs` an explicit deliverable of P6-01a with a done-criterion per LR
row, and add a Phase-7 item: open a real legacy `.beads` workspace and assert both the warning and
the successful read (that item is *also* the plan §5 Phase-7 requirement M4 flags below).

### M4 — Phase 7 is asserted "unchanged", but it was never concrete

INTEGRATION_PLAN.md:60-61: *"Phases 7 (verification sweep) and 8 … are unchanged from the original
plan and are not commit series."* The original plan §5 Phase 7 is one sentence:

> Full gates + the rename-completeness audit + Org round-trip test + a fresh `obr init` → create →
> flush → **clone-simulation → import cycle** in a throwaway dir confirming: only `.obr/` artifacts
> are created, **legacy `.beads` workspaces still open with a warning**, and the file written is
> `issues.org`.

Against that, the plan delivers only G-rename part 4's five lines (fresh `init` + `find -name
'*beads*'`). Missing and unowned: the **clone-simulation import cycle** (the closest thing is
G-org-roundtrip's "copy G1 into a fresh workspace", which is not a clone simulation and does not
exercise `.obr/` artifact creation); the **legacy-workspace-opens-with-a-warning** check (M3);
the **full serial G-unit** BASELINE.md mandates "at Phase 7 minimum"; R2's Q5 sweep (M1); and
R18's two-second `obr list | head -1; echo $?` smoke, which §8 R18 mentions but no step owns.

**Fix.** Write a §6.5 "Phase 7 verification sweep" with numbered, copy-pasteable steps and pass
criteria, the same way §1 does for gates.

### M5 — a third of the commits are under-specified

The brief for Phase 2 required "the exact commit series (each with **scope, files, source-of-truth,
done-criteria**)" plus a gate. Audit of all 32 rows:

| Commit | Scope | Files | Source-of-truth | Done-criteria | Gate |
|---|---|---|---|---|---|
| P3-01 … P3-13 | ✅ | ✅ | ✅ | ✅ | ✅ |
| P4-01 | ✅ | ✅ (git rm list) | ✅ | ✅ | ✅ |
| P4-02 | ✅ | ~ (prose) | ✅ | ✅ | ✅ |
| P4-03 | ❌ | ❌ | ✅ | ~ ("passes or its runtime is measured") | ✅ |
| P4-04 | ❌ | ❌ | ✅ | **❌ none** | ✅ |
| P4-05 | ~ | ~ | ✅ | ~ | ✅ |
| P4-06 | ~ | ❌ | ✅ | **❌ none** | ✅ |
| P4-07 | ~ | ~ | ✅ | **❌ none** | ✅ |
| P5-01 / P5-02 | ✅ | ✅ | ~ | ✅ (but see B2) | ✅ |
| P5-03 | ~ | ❌ | ✅ | ❌ (only in the master table) | ❌ in body |
| P6-00 … P6-04 | table row only | ✅ counts | implied R4 | **❌ none** | "all" |
| P6-05 | ✅ | ✅ | ✅ | ~ | ✅ |
| CLEANUP | ✅ | ~ | — | **❌ none** (a bullet list of scope) | ✅ |

Phase 6 is where reviewability matters most (P6-01a is flagged "the only part of Phase 6 that needs
real review") and it is the phase with **zero done-criteria**. Also: the master table's caption says
"**Twenty-nine commits**" against **32 rows** — presumably discounting P4-07 (optional), P4-08
(blocked) and P5-03 (conditional), but the document never says so.

### M6 — unowned root-level files that fail G-rename

- **Root `UPGRADE_LOG.md`** is a *distinct file* from `docs/operations/UPGRADE_LOG.md` — different
  sizes (6,355 vs 8,121 bytes), `diff` reports them different **[V]** — with **6** case-insensitive
  `beads` hits. §5.1 "Deliberately NOT renamed" and G-rename part 1's pathspec both name only
  `docs/operations/UPGRADE_LOG.md`. The root copy is unowned and **fails part 1**.
- **Root `.gitignore`**: 18 branded lines **[V]** (`:34 /legacy_beads/`, `:39` "bv (beads viewer)",
  `:43-46 .beads/last-touched|.bv.lock|*.lock|.br_history/`, `:131-142` eight `.beads/*` patterns
  incl. the `:142 .beads/recovery*/` that D-GITIGNORE is *about*, `:148-149 sample_beads_db_files/**`,
  `:168 .beads.snapshot_*/`). D-GITIGNORE raises one line as a decision; **no commit owns the file**.
- **`.ubsignore`** (tracked, root): contains `legacy_beads/` and `.beads/` **[V]**. Not in P4-01's
  `git rm` list (which removes its sibling `.rchignore`), not in any rename row.
- `.gitattributes` is adopted in P4-04 for `fuzz/corpus/**` but never audited for branding.

### M7 — `CLAUDE.md` has no home

Upstream has **no `CLAUDE.md`** **[V]** (`git ls-files | grep -i '^CLAUDE'` → empty). Fork commit
`22adc72b` added a 75-line one and governing-plan §3.1 classifies it "**keep**; update facts after
sync". The INTEGRATION_PLAN's disposition of the 20 fork commits is otherwise complete — Org-core →
P3-\*, rename → P6-\*, productization → P4-\*, `9f1f2db0` tracker data → P5-01 — but `22adc72b` is
never mentioned, while CLEANUP instructs the executor to "**Refresh `CLAUDE.md` and `AGENTS.md`
facts**" on a file that will not exist. (CLEANUP's list of stale CLAUDE.md facts is otherwise
excellent and independently correct — C5's `forbid`→`deny` catch is real.)

### M8 — two of the governing plan's four §8 decisions are resolved, not surfaced

Plan §8 is titled "**Open decisions — ASK THE USER, do not decide**".

- **§8.2 (`.obr` artifact names)**: "confirm `obr.db` … and `.obr/history/` vs `.obr_history/`".
  The plan turns this into map rows **A3** ("`obr.db`", LR) and **A7** ("**`.obr/history/`**") plus
  correction **C9** ("the plan's question is mis-framed"). C9's *analysis* is right and verified —
  `.br_history` really is `beads_dir.join(...)` (`src/sync/history.rs:502`) and `.br_recovery` really
  is `src/config/mod.rs:66` **[V]** — but re-framing a question is not the same as answering it, and
  §9 has **no row for it**. R4 §7.7/§7.8 explicitly labelled both as **RECOMMENDATIONS**.
- **§8.3 (landing: force-push `main` / merge / long-lived branch)**: absent from §9 entirely. It is
  the highest-stakes decision in the whole project and §9 claims to be the decision list.

(§8.1 → D-LICENSE ✅, thoroughly and carefully. §8.4 → D-PLUGIN covers `.claude-plugin/`+`skills/`;
`.agent-mail.yaml` and `.ntm/` are folded into P4-01/D-DELETIONS, whose enumeration lists `.ntm/logs/`
but not `.agent-mail.yaml`, `.rchignore`, or `custom.db`.)

### M9 — G-locked is applied inconsistently, and misses the commit that needs it most

The master table gives **G-locked** to P3-01 alone; §1 defines "all" as build/lint/unit/e2e; but §8
row **R14** says *"**P3-01** + **G-locked** on every commit"*. Beyond the contradiction: **P6-02
renames the package**, which necessarily rewrites `Cargo.lock`'s root package entry and
`fuzz/Cargo.lock`'s path dependency — the single most likely place to break `--locked` — and it has
no G-locked gate. Same for P4-02 (flake/lock) and P4-05 (fuzz manifest).

### M10 — the risk-register summary does not add up

The 32 table rows are all present and titled consistently with the dossier (`grep -c '^### R[0-9]'
DOSSIER.md` = **32**, R1–R32, titles match **[V]**). The *summary paragraph* (INTEGRATION_PLAN.md:1437-1441)
buckets: 8 fixed + 2 partial + 6 carry-forward + 1 reintroduced + 9 re-assess. That accounts for 26
labels over a set of 32 risks and **never mentions R11, R15, R19, R20, R22, R26, R27** — four of
which the table itself marks PARTIAL (R19, R20, R22) or CARRIES-FORWARD (R11, R26, R27). A reader who
reads only the summary will believe seven risks were not considered.

---

## 3. Landmine-by-landmine audit (governing plan §7)

| §7 landmine | Owned by a specific commit? | Verdict |
|---|---|---|
| **7.1 org2jsonl pinning + lock** | **P3-01** (git rev pin, full 40-hex, `cargo build --locked`), **P4-02** (delete flake input + `postUnpack` hack) | ✅ **Fully addressed**, and C7's correction of the plan's description of the current pin is right (`fork:flake.nix` has a bare `git+file://` URL, no rev). |
| **7.2 upstream JSONL schema growth** | **P3-02** (two new fields + the field-destructure test) | ⚠️ **Fields yes, record kinds never closed in writing.** I verified independently that `write_export_issue_jsonl` (`:9386`) emits **only** `serde_json::to_writer(issue)` + `\n` — no header, no `_type`, no metadata record **[V]**, and that the `Issue` struct went **39 → 41 fields**, the delta being exactly `agent_context` and `source_repo_path` **[V]** (`git show e805fe98:src/model/mod.rs` vs upstream). So the plan's conclusion is *correct* — but it is asserted, not evidenced, and the landmine's actual question ("did the wire format gain record kinds/headers?") is never answered on the page. Add one line with this evidence. |
| **7.3 test-estate reality** | **P3-13** (Class A/B/C rule), U-TESTPOLICY | ✅ Strong. Verified: **59** of 143 `tests/*.rs` mention `issues.jsonl` **[V]**, matching the plan. `conformance*.rs` = 6 files **[V]**, correctly Class A. |
| **7.4 fork tracker data survival** | P5-01/02/03 | ❌ **See B2 and B3.** Present, but the mechanism as written does not produce Org, and the directory is never moved. |
| **7.5 git-safety hardcoded filenames** | **P3-03** + correction C3 | ✅ **Excellent** — and C3's correction is right on both halves: `src/sync/db_inode_lock.rs` has zero JSONL references, and the hand-copied second allowlist at `tests/e2e_sync_git_safety.rs:1274-1283` is real (I read it — 8 entries, byte-identical to `path.rs:85-94`, no `org`) **[V]**. Residual: M1's two production sites are in the same class. |
| **7.6 Cargo.lock regeneration** | P3-01 + G-locked | ⚠️ See **M9** (not gated at P6-02/P4-02/P4-05). |
| **7.7 licensing** | D-LICENSE | ✅ Outstanding — the rider-timing datum (landed v0.1.15, one week *after* the fork point) is a genuinely new and decision-relevant fact. |
| **7.8 gate definitions** | §1 | ❌ See **B1**. |

---

## 4. Rename-map completeness vs R4 (5 grep categories re-run myself)

I re-ran five of R4's inventory categories against the worktree. **The map holds up well**; the
numbers below are close enough that I found no *category* missing from §5.1.

| Category | R4 / plan claim | My count **[V]** | Verdict |
|---|---|---|---|
| `br.*` versioned schema IDs (M1) | 26 distinct | `git grep -Pho '"br\.[a-z0-9_.-]+"' -- src/ \| sort -u \| wc -l` = **26** | ✅ exact |
| Hash domain separators (M6) | 9 (5 + 4 sync-merge) | grepped each constant: `sync/mod.rs:988`, `:1003`, `:2602`, `:2603`, `:2605`, `:2606`, `witness.rs:20`, `:21`, `config/mod.rs:4992` = **9** | ✅ exact, all nine anchors real |
| `beads://` MCP URIs (M2) | 12 URIs, 10 files | 13 distinct patterns, **10 files** | ✅ (the 13th is a `{id}`-template variant) |
| `BeadsError` (P6-03) | ~2,300 | `git grep -Pow 'BeadsError' \| wc -l` = **2,334** | ✅ |
| `.br_recovery` (A8) | 32 files, 107 hits | 32 files | ✅ |
| `is_beads_dir_name` (A1/A2) | `config/mod.rs:55-57` | at `:55`, 9 call sites | ✅ |

**What the map still misses** (all from B3/M6 above): the repo's own `.beads/` payload; root
`.gitignore`; `.ubsignore`; root `UPGRADE_LOG.md`; and the `issue_prefix` config **key value** as
distinct from the issue IDs D-PREFIX governs.

One structural note in the map's favour: R4 category (a) lists 17 on-disk artifact names and §5.1
section A has exactly 17 rows A1–A17 — the correspondence is deliberate and complete.

---

## 5. Decision-coverage audit (§9)

**Every brief-surfaced decision is carried** except as noted:

| Source | Decision | In §9? |
|---|---|---|
| R2 §9 | U1, U2, U3, U4, U5 | ✅ all five, faithfully, with the recommendations marked as recommendations |
| R2 §9 | **Q1** — does `orgize 0.10.0-alpha.10` accept `-` in tags? "Affects whether `:LABELS:` is a nicety or a necessity" | ❌ **absent**. Partially mitigated (G-org-roundtrip's fixture includes "a hyphen label"), but the plan has **no open-questions section at all**, so an unclosed technical question silently disappears. |
| R2 §9 | **Q2** — what does the `br sync` witness CLI actually gate on? R2 read three call sites, not the flag handling | ❌ absent; P3-10 nonetheless prescribes a rejection at those three sites as if closed |
| R2 §9 | Q3 → **Q-LINES** ✅ ; Q4 → cited inline in P3-12 ✅ ; **Q5** → ❌ (see M1) | |
| R3 §9 | 1 → answered in §0.1 (but see B1); 2 → D-SHFMT; 3 → D-AUDIT; 4 → D-HOOKS; 5 → D-CHANGELOG; 6 → inline in P4-02; 7 → D-ACFS | ✅ all seven |
| R4 §11 | D1 → D-SELFUPDATE; D2 → D-PLUGIN; D3 → D-PREFIX; D4 → D-GITIGNORE; D5 → D-DOMAINSEP; D6 → D-CHANGELOG | ✅ all six |
| Plan §8 | 1 → D-LICENSE ✅ ; 2 → ❌ **resolved in the map**; 3 → ❌ **absent**; 4 → ✅ partial | see **M8** |
| Plan-implicit | D-MONOREPO, D-RESIDUALS, D-SUPPLYCHAIN, D-UPSTREAMTRACKER, U-DRAFT, U-EXTERNAL, U-MERGEART, U-TESTPOLICY, U6 | ✅ genuinely new and correctly surfaced |

**Sequencing hazard in the decision set.** U-EXTERNAL is the one row with *"(none offered)"* under
"Brief's recommendation", and it blocks **P3-03** — which §0.3 says must land before *anything* Org
can be tested. A decision with no recommendation sitting on the critical path at commit 3 of 32
deserves a flag in §0.2's pre-flight checklist (which lists U2, U3, U-DRAFT, U1, U-TESTPOLICY but
**not** U-EXTERNAL, U-MERGEART, U5, U6, or Q-LINES — all of which block P3-02/03/07/08).

---

## 6. Anchor spot-check (24 checked, 24 real)

The brief asked for ≥6. All are `/Users/johnw/src/obr-org-forward-port` @ `d1fb0d3a`, read with
`sed -n`/`grep -n` **[V]**.

| Cited | Symbol | Actual | Δ |
|---|---|---|---|
| `path.rs:85-94` | `ALLOWED_EXTENSIONS` (8 entries, no `org`) | `:85` | 0 |
| `path.rs:546-559` | `is_allowed_jsonl_temp_name` | `:548` | +2 |
| `path.rs:679-690` | `validate_external_jsonl_path` ("must be a .jsonl file") | `:683` | +4 |
| `path.rs:2274` | `compute_snapshot_content_sha256(backing, deadline)` — **no path param**, trim+skip-blank canonicalization | `:2274` | 0 |
| `path.rs:~2407` | sole call site, `path` in scope | `:2407` (only call site confirmed) | 0 |
| `sync/mod.rs:1818` | `export_temp_path_for_attempt` → `with_extension("jsonl.{pid}.tmp")` | `:1818` | 0 |
| `sync/mod.rs:8246` | `validate_jsonl_issue_records_from_reader` | `:8246` | 0 |
| `sync/mod.rs:9022` | `analyze_jsonl_from_reader` | `:9022` | 0 |
| `sync/mod.rs:9081` | `verify_exported_jsonl_snapshot_integrity` | `:9081` | 0 |
| `sync/mod.rs:9386` | `write_export_issue_jsonl` | `:9386` | 0 |
| `sync/mod.rs:9421` | `struct PreparedExportIssue { jsonl_line }` | `:9421` | 0 |
| `sync/mod.rs:9463/9493/9506` | `prepare_export_issue_jsonl` / `_chunk` / `_parallel` | `:9463` / `:9495` / `:9506` | 0/+2/0 |
| `sync/mod.rs:~9894` | `let mut hasher = Sha256::new();` (Org header insertion point) | `:9894` | 0 |
| `sync/mod.rs:11492` | `try_incremental_auto_flush`, `!jsonl_path.exists()` guard at `:11498` | `:11492` | 0 |
| `sync/mod.rs:12077` | `for_each_jsonl_import_issue` (the chokepoint) | `:12077` | 0 |
| `sync/mod.rs:12150` | `build_prefix_renames` | `:12150` | 0 (**and R4 §11 D3's `:12586` is wrong — the plan silently corrected it**) |
| `sync/mod.rs:12769/12805` | `compute_jsonl_hash_from_reader` / `compute_jsonl_hash` | `:12769` / `:12801` | 0/−4 |
| `sync/mod.rs:13923/13961/14018` | `save_base_snapshot_from_jsonl_snapshot` / `refresh_..._snapshot` (byte-copy confirmed) / `load_base_snapshot` | exact | 0 |
| `config/mod.rs:55/60/62/64/66/71-79/147/4219` | `is_beads_dir_name`, 4 constants, `EXCLUDED_JSONL_FILES` (6), `discover_jsonl` (2-tier), `resolve_jsonl_path` | all exact | 0 |
| `config/mod.rs:3352` | `is_default_path: … join("issues.jsonl")` | `:3352` | 0 |
| `cli/commands/sync.rs:1522/1553` | `validate_sync_paths` / `!= Some("jsonl")` | `:1522` / `:1551` | 0/−2 |
| `cli/commands/vcs.rs:259` | `.jsonl`-only diagnostic gate | `:259` | 0 |
| `cli/commands/stats.rs:92` | `\|\| beads_dir.join("issues.jsonl")` | `:92` | 0 |
| `doctor.rs:10439/10451/10580/5201/3202` | private `discover_jsonl`, `should_fallback_to_workspace_jsonl`, `check_jsonl`, `check_jsonl_duplicate_ids`, "Fix the offending records in `.beads/issues.jsonl`" | `:10439` / `:10451` / `:10580` / `:5201` / `:3202` | 0 |
| `model/mod.rs:64` `Status::Draft`; `schema.rs:11` = 17; `tests/e2e_sync_git_safety.rs:1274-1283`; `tests/e2e_schema.rs:920` | all present | exact | 0 |

**Conclusion: the anchor index is trustworthy.** This is the strongest part of the document and the
executor can rely on §7.

---

## 7. Minor findings

- **m1 — `self_update` silently disables P6-05's gate.** `tests/e2e_schema.rs:918` is
  `#[cfg(feature = "self_update")]` **[V]**, and `Cargo.toml [features] default = ["self_update"]`.
  D-SELFUPDATE's **recommended** option (b) drops `self_update` from default features — which
  compiles `agent_baseline_snapshots_match_current_binary` out of existence and makes P6-05's gate
  vacuous while still appearing green. Neither §9 nor P6-05 notes the interaction.
- **m2 — dangling citation.** `[R2 §Q6]` at INTEGRATION_PLAN.md:774; R2 has Q1–Q5.
- **m3 — P6-01b's fixture counts understate the tree.** Plan: `tests/doctor_fixtures/` "157 files,
  122 shell scripts"; `tests/fixtures/` "67"; `tests/snapshots/` "22". On disk **[V]**: **206** / **131
  .sh** / **205** / **97**. Presumably the plan counts *files containing branding* (R4 category (k)'s
  393 is consistent with the "~380" headline) — say so, or an executor will think the job is done at
  40 % coverage.
- **m4 — `docs/ORIGINS.md`** is exempted in the G-rename part-1 filter but is never created by any
  commit (Phase 8 creates only the README *Origins* section). Harmless, but it implies a deliverable
  that does not exist.
- **m5 — CI runner constraint noted, unowned.** §7.1 flags `perform_conditional_namespace_change`
  (`:1464` vs the hard-erroring `:1521`) as cfg-gated Linux/Android/Apple → "constrains CI runners".
  P4-06 (the CI commit) never revisits it, and there are 8 workflows **[V]**.
- **m6 — `.beads/interactions.jsonl`** (tracked, 3 records, upstream's) is not mentioned anywhere:
  not P4-01, not P5-01, not the rename map, not D-UPSTREAMTRACKER.
- **m7 — the "29 commits" caption** vs 32 table rows (see M5).
- **m8 — command-name anachronism.** Every done-criterion from P3-09 onward says `obr init`,
  `obr doctor`, `obr sync` — but the binary is still `br` until **P6-02**. Cosmetic, but a literal
  executor will burn a cycle on it. One sentence in §0.2 fixes it.
- **m9 — `benches/storage_perf.rs`** has 17 `beads`/`jsonl` hits and is in no rename row (P6-02's
  "266 files" plausibly covers it, but see M2 — nothing compiles it).

---

## 8. Corrections to the plan(s)

### 8.1 Where `INTEGRATION_PLAN.md` itself is wrong

1. **§0.1's three "baseline facts that change how you work" are two-thirds stale.** Fact 1
   ("clippy never lints the workspace's own code") is a property of the *ambient* toolchain, not of
   upstream; under the canonical fenix pin BASELINE.md mandates, clippy lints straight through to
   **85 first-party errors**. The unit row (9) is a truncated parallel run; the real figure is **266**
   with **4 aborters**. Fact 2 (`.br-jsonl-write-*.lock` trips the git-safety allowlist at baseline)
   and fact 3 (macOS `/tmp` symlink) are both correct and both valuable. → **B1**
2. **Map row A14 is wrong about `config.yaml`.** "KEEP — already unbranded" contradicts R4 §5.8 and
   the file itself (`issue_prefix: beads_rust`, plus ~20 branded comment lines in the fork's copy).
   → **B3**
3. **P5-02's claim that the tracker "lands as `.beads/issues.org`" is false** given P5-01's own
   instruction to copy `metadata.json`. → **B2**
4. **§8's summary sentence miscounts** the risk buckets. → **M10**
5. **`[R2 §Q6]`** does not exist. → **m2**
6. **"Twenty-nine commits"** ≠ 32 table rows. → **m7**
7. **G-locked** is described inconsistently between the master table, §1, and §8 R14. → **M9**

### 8.2 Where `INTEGRATION_PLAN.md` corrects the governing plan — and is right

I re-verified the load-bearing ones so nobody redoes the work. **All confirmed [V]:**

- **C3** (`db_inode_lock.rs` has zero JSONL references; the real safety layer is `path.rs` +
  `sync/mod.rs:1818` + the line-splice machinery + the hand-copied test allowlist) — confirmed;
  I read the duplicated 8-entry `ALLOWED_EXTENSIONS` at `tests/e2e_sync_git_safety.rs:1274-1283`.
- **C4** (stats needs one line, `stats.rs:92`; doctor needs ~12 sites) — confirmed at both ends.
- **C9** (`.br_history` and `.br_recovery` are already inside `.beads/`) — confirmed
  (`config/mod.rs:66`, `history.rs:502`).
- **C10** (`CURRENT_SCHEMA_VERSION = 17`) — confirmed at `schema.rs:11`; and the fork's is `1`
  (`git show main:src/storage/schema.rs:5`), so the "1 → 17 migration" claim is exactly right.
- **C11** (the plan's map is missing the schema IDs, MCP URIs, domain separators, `"br"` default
  prefix, …) — confirmed by independent counts (§4 above).
- **C26** (`git grep -E '\bbeads\b'` is a no-op; must be `-P`) — confirmed by construction.
- **C29's own re-verification list** — I re-checked the two most load-bearing entries: `sync/mod.rs`
  is 21,609 lines and `sqlite.rs` 34,865 **[V]**; `temp_test/`, `temp_test_2/`, `custom.db` really
  are present in the upstream tree **[V]**; `sample_beads_db_files/` really is **178 MB / 32 tracked
  files** **[V]**.
- **The two-new-fields claim (P3-02) is exactly right.** Independent diff of the `Issue` struct
  between `e805fe98` and `upstream/main`: **39 → 41 fields**, delta = `agent_context`,
  `source_repo_path` **[V]**. The "all 41 fields" figure in the mandatory destructure test is also
  correct.

### 8.3 Where the *governing* plan is wrong and the INTEGRATION_PLAN did not catch it

- **Plan §7.4 / Phase 5's "the beads-branded docs inside `.beads/` … migrate to the renamed directory
  in Phase 6"** is the *right instruction* — C25 correctly reattributes those docs to upstream, but
  in doing so the plan drops the migration instruction entirely rather than assigning it. → **B3**
- **Plan §5 Phase 7's clone-simulation and legacy-workspace checks** were dropped on the grounds that
  "Phase 7 is unchanged", which converts a checklist into a gap. → **M4**

---

## 9. What I verified myself in this session

Read-only. No builds, no writes outside this file, no modification of
`/Users/johnw/src/obr-org-forward-port`.

| What | How | Result |
|---|---|---|
| INTEGRATION_PLAN, the 4 briefs, governing plan, BASELINE.md | `Read`, in full | consumed |
| 24 upstream anchors | `sed -n` / `grep -n` per §6 | all real, max Δ 4 lines |
| Baseline discrepancy | `wc -l` on v1 vs v3 inventories; read `summary.txt`, `summary-v3.txt`, `v3-clippy-lints-baseline.txt`, `v3-unit-aborters.txt` | 9 vs **266**; clippy 85 first-party errors |
| `Issue` field delta | `git show e805fe98:src/model/mod.rs` vs `upstream/main`, field-name diff | 39 → 41, exactly `agent_context` + `source_repo_path` |
| JSONL record kinds | read `write_export_issue_jsonl` (`:9386-9418`) | only `Issue` records, one per line, no header/`_type` |
| Hardcoded `issues.jsonl` in `src/` | `grep -rn … \| uniq -c`, then `mod tests` boundary per file | **515 hits / 25 files**; 6 production sites unassigned (M1) |
| `.beads/` payload | `git ls-files .beads`; `grep -ci` per doc; `cat config.yaml`, `metadata.json`, `.gitignore` | 10 tracked files, 158 branding hits in the 5 docs, `issue_prefix: beads_rust` |
| Fork tracker | `wc -l fork:.beads/issues.jsonl` = 549; `cat fork:.beads/metadata.json` | confirms 549 and the JSONL pin (B2) |
| Fork schema version | `git show main:src/storage/schema.rs` | `CURRENT_SCHEMA_VERSION = 1` |
| Rename inventory | 6 independent greps per §4 | 26 schema IDs, 9 domain separators, 10 MCP files, 2,334 `BeadsError`, 32 `.br_recovery` files |
| Gate blind spots | `cat fuzz/Cargo.toml`; `grep beads_rust benches/ fuzz/` | `beads_rust-fuzz` + path dep + 7 targets; benches use the crate |
| Test-target existence | `ls tests/` | `e2e_history`, `e2e_doctor_fixture_suite`, `golden_beads_init`, `repro_auto_flush_inefficiency`, 6 `conformance*` all real; **`tests/legacy_compat.rs` does not exist** |
| Root files | `ls -a`; `diff UPGRADE_LOG.md docs/operations/UPGRADE_LOG.md`; `grep -n beads .gitignore .ubsignore` | two distinct UPGRADE_LOGs; 18 branded `.gitignore` lines; `.ubsignore` unlisted |
| CLAUDE.md | `git ls-files \| grep -i '^CLAUDE'` upstream | absent (M7) |
| Dossier risk set | `grep -c '^### R[0-9]' DOSSIER.md` + title comparison | 32, R1–R32, all present in §8's table |
