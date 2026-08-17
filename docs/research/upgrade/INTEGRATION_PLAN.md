# INTEGRATION_PLAN.md — the executable commit series for Phases 3–6

**Phase 2 deliverable** of `/Users/johnw/src/obr/UPSTREAM_SYNC_AND_RENAME_PLAN.md` §5.
**Written:** 2026-08-06, by the Phase-2 synthesis agent, from R1–R4 read in full plus independent
spot-verification against the upstream worktree and the Phase-0 baseline logs.

**Substrate:** branch `org-forward-port` cut from `upstream/main` = `d1fb0d3a` (beads_rust v0.2.22).
**Read-only upstream worktree used for all anchors:** `/Users/johnw/src/obr-org-forward-port`.
**Fork (specification source):** `/Users/johnw/src/obr` @ `main`, fork point `e805fe98`.

**Evidence convention.** Bare `path:line` = upstream worktree at `d1fb0d3a`. `fork:path:line` = the
fork's working tree. `[V]` = verified directly against the worktree or git. `[R1 §x]` / `[R2 §x]` /
`[R3 §x]` / `[R4 §x]` = inherited from the named recon brief (which cited its own evidence).
`[I]` = inferred.

> ### Revision 2 (2026-08-06) — what changed and why you must not use revision 1
>
> An adversarial completeness audit found three blockers and ten major gaps. Every one is fixed
> below. The five you are most likely to be burned by if you read a stale copy:
>
> 1. **§0.1 and §1 were wired to a superseded baseline.** Revision 1 said the unit baseline was
>    **9** failures and that clippy "never lints the workspace's own code". The durable record
>    (`docs/research/upgrade/BASELINE.md` + `baseline/v3-*.txt`) says **266** unit failures, **four
>    process-aborting tests** that truncate the run, and **85 FIRST-PARTY** clippy errors. Revision 1
>    also told you to run `nix develop` and `cargo clippy`; **neither works on this host.** §0.1,
>    §0.2 and §1 are rewritten. See §10.C30–C32.
> 2. **Phase 5 could not produce an Org tracker.** `.beads/metadata.json` pins `issues.jsonl`, which
>    P3-09 turns into an *explicit override* — so `sync --flush-only` wrote JSONL and the
>    done-criterion passed anyway. P5-01/P5-02 are rewritten around that trap. See §10.C35.
> 3. **No commit owned the repository's own `.beads/` directory** (ten tracked files **[V]**).
>    `git mv .beads .obr` and the de-branding of all ten now belong to **P6-01a** (§5.2a), and
>    G-rename gains **part 3b**.
> 4. **Nothing compiled `fuzz/` or `benches/` after P4-05**, so P6-02/P6-03 could ship an
>    uncompilable fuzz crate with every gate green. New gate **G-aux** (§1), wired into P4-05,
>    P6-02, P6-03, Phase 7 and CLEANUP.
> 5. **Phase 7 was declared "unchanged from the original plan" and therefore never made concrete.**
>    §6.1 is now ten numbered steps with pass criteria, including the clone-simulation cycle and the
>    legacy-workspace warning check the governing plan explicitly asks for. See §10.C34.
>
> Also new: `tests/legacy_compat.rs` is an explicit P6-01a deliverable with one test per LR row
> (§5.2a); Phase-4 and Phase-6 commits have the Files / source-of-truth / done-criteria they were
> missing; six previously-unassigned production Org anchors are placed (§7.4); §8's risk summary
> accounts for all 32 risks; §9 gains **D-ARTIFACTNAMES** and **D-LANDING** (the two governing-plan
> §8 items that had been silently re-framed or dropped) and a new **§9.4 open technical questions**.

> **How to use this document.** Read §0 (preconditions + baseline) and §1 (gates) once — **and read
> `docs/research/upgrade/BASELINE.md` before your first gate; it is the durable Phase-0 record and
> this document defers to it on every number.** Then execute §2 → §3 → §4 → §5 → §6 strictly in
> order, one commit at a time, running the named gate after each. §6.1 is the Phase-7 verification
> sweep and §6.2 is the CLEANUP commit. §7 is the master anchor index — the single table to consult
> when you cannot find a function. §8 is the risk register. §9 lists the decisions you **must not
> make yourself** (§9.1–§9.3) plus the open *technical* questions nobody has closed (§9.4) — get
> answers before the commits that reference them. §10 corrects the original plan; read it before
> trusting the plan.

---

## Master commit series (at a glance)

| # | Commit | Phase | Gate |
|---|---|---|---|
| **P3-01** | `build: add org2jsonl git dependency pinned by rev` | 3 | G-build, G-locked |
| **P3-02** | `feat(sync): add org_bridge module (emit + parse), no callers` | 3 | G-build, G-lint, G-unit |
| **P3-03** | `feat(sync): admit .org through every path gate` | 3 | G-build, G-unit, G-e2e |
| **P3-04** | `fix(sync): make the canonical content hash format-aware` | 3 | G-build, G-unit |
| **P3-05** | `feat(sync): Org record emission on the export path` | 3 | all |
| **P3-06** | `feat(sync): Org import through one format-aware chokepoint` | 3 | all |
| **P3-07** | `fix(sync): decline incremental auto-flush for non-JSONL exports` | 3 | all |
| **P3-08** | `fix(sync): re-serialize the merge base anchor for Org workspaces` | 3 | all |
| **P3-09** | `feat: make issues.org the default export artifact` | 3 | all + **G-org-roundtrip** |
| **P3-10** | `feat(cli): Org-aware sync/vcs surface; reject Org for JSONL-only modes` | 3 | all |
| **P3-11** | `feat(sync): extension-aware history backups` | 3 | all + `e2e_history` |
| **P3-12** | `fix(doctor): Org awareness across export-file checks` | 3 | all |
| **P3-13** | `test: Org e2e coverage, harness helpers, and the JSONL pinning rule` | 3 | all + G-org-roundtrip |
| **P4-01** | `chore: remove stray and foreign artifacts from the upstream tree` | 4 | G-build, G-unit |
| **P4-02** | `build(nix): replace crane+fenix flake with rust-overlay` | 4 | G-build, **G-locked** + `nix build` |
| **P4-03** | `build(nix): add flake checks and extend the dev shell` | 4 | `nix flake check` |
| **P4-04** | `chore: add lefthook pre-commit hooks` | 4 | G-build |
| **P4-05** | `test(fuzz): add org_parse and validation fuzz targets` | 4 | **G-locked + G-aux** |
| **P4-06** | `ci: tighten clippy and benchmark gates` | 4 | G-lint |
| **P4-07** | `style: format shell scripts with shfmt` *(optional; scoped)* | 4 | G-build |
| **P4-08** | *(blocked on §9 D-LICENSE — must not be written)* license disposition | 4 | — |
| **P5-01** | `chore(tracker): import the fork's own issue history` | 5 | G-build + count check |
| **P5-02** | `chore(tracker): convert the tracker to Org` | 5 | G-org-roundtrip |
| **P5-03** | *(conditional on §9 D-PREFIX)* `chore(tracker): re-prefix issue IDs` | 5 | G-build + count + dep-graph check |
| **P6-00** | `chore: delete branded historical apparatus` | 6 | G-build, G-unit |
| **P6-01a** | `refactor!: rename on-disk artifacts, env vars and CLI surface (+ shims)` | 6 | all + **G-unit tier 2** |
| **P6-01b** | `test: mechanical fixture/snapshot update for the artifact rename` | 6 | all |
| **P6-02** | `refactor!: rename crate beads_rust → obr, binary br → obr` | 6 | all + **G-aux** |
| **P6-03** | `refactor: rename internal beads_* identifiers to obr_*` | 6 | all + **G-aux** |
| **P6-04** | `docs: de-brand documentation, packaging and CI` | 6 | all + **G-rename** |
| **P6-05** | `test: regenerate agent_baseline goldens` | 6 | all + G-rename |
| — | **Phase 7 verification sweep** (§6.1) — *not a commit* | 7 | **every gate, serially** |
| **CLEANUP** | `chore: final tree cleanup, CLAUDE.md, fact refresh` | 7 | **every gate** |

**Thirty-two commit rows** (plus the Phase-7 sweep, which produces no commit). The series you
actually write is **30 or 31 commits**, and 29 is the floor:

- **P4-08 must not be written** — it is a placeholder that records what is blocked on D-LICENSE.
- **P5-03 is conditional** on D-PREFIX choosing option (i).
- **P4-07 is optional** (D-SHFMT recommends *not* doing a bulk pass).

*(An earlier draft said "twenty-nine commits" against 32 rows with no explanation. This is the
explanation.)*

Phase 7 is **no longer** "unchanged from the original plan": §6.1 makes it a concrete, numbered,
copy-pasteable sweep with pass criteria, because four of its obligations (the clone-simulation
cycle, the legacy-workspace warning check, the full serial G-unit, and R2's closing write-path
sweep) were owned by no step. Phase 8 (docs / credit / landing) remains outside this document;
`CLEANUP` is its prerequisite and **`CLEANUP` now also creates `CLAUDE.md`**, which does not exist
upstream **[V]**.

---

## 0. Preconditions and the Phase-0 baseline

### 0.1 The baseline is RED in five places. This is not your fault and you must not chase it.

> **This section was rewritten to match the durable Phase-0 record.** An earlier draft was wired to a
> superseded first-pass ("v1") baseline that reported 9 unit failures and a third-party clippy abort.
> **Both numbers were artifacts of the wrong toolchain and a truncated test run.** The authoritative
> record is `docs/research/upgrade/BASELINE.md` plus the inventories in
> `docs/research/upgrade/baseline/`. Read BASELINE.md before your first gate. Anything that still
> cites `scratchpad/baseline/*` or a 9-name unit list is stale — the scratchpad is ephemeral and the
> v1 numbers are wrong.

#### 0.1.0 The canonical gate toolchain — use this exact pin for every gate

```bash
nix shell 'github:nix-community/fenix/a7c3ef79859bf024cdc865a0ed8f14a4e3f31099#complete.toolchain' \
  --command <cmd>
# rustc 1.99.0-nightly (7608eb7b0 2026-08-05), clippy 0.1.99
```

Three host facts, all **[V]** in the Phase-0 record, that make this non-negotiable:

1. **`nix develop` cannot build this tree.** The fork's dev shell pins nightly 1.95.0 (2026-02-17),
   which cannot compile `sysinfo 0.39.6` (it needs then-unstable `cfg_select`). Upstream's own flake
   devShell is broken on Darwin twice over: it references removed `darwin.apple_sdk.frameworks.*`
   stubs (`flake.nix:110-112`) and crane's eval-time `vendorCargoDeps` cannot find `Cargo.lock`
   because `combinedSrc` nests sources under `beads_rust/` while `src` points at the root. Both
   defects feed **P4-02**.
2. **`cargo clippy` and `cargo fmt` are unusable machine-wide.** `$CARGO_HOME/bin/cargo-clippy` and
   `cargo-fmt` are rustup shims that resolve *before* PATH and dispatch to rustup's stale Feb-2026
   nightly. **Always invoke the binary `cargo-clippy` / `cargo-fmt` directly inside the nix shell.**
   Never write `cargo clippy` (with a space) anywhere in this port — in a gate, in CI, or in a hook.
3. The host's ambient stable 1.97.1 builds and tests the tree and produces **identical e2e failure
   name-lists** to nightly 1.99.0 (VERIFIED by diff), so the failures below are deterministic, not
   toolchain artifacts. Lint results are *not* toolchain-stable; only the pin above counts.

#### 0.1.1 Gate results at baseline

| Gate | Command (inside the nix shell above) | Baseline result | Durable inventory |
|---|---|---|---|
| G-build | `cargo build` | **PASS**, ~145 s cold | — |
| G-locked | `cargo build --locked` | **PASS** — upstream's committed `Cargo.lock` is valid | — |
| G-lint | `cargo-clippy --all-targets -- -D warnings` | **FAIL: 85 errors, all FIRST-PARTY** in the `beads_rust` lib | `baseline/v3-clippy-lints-baseline.txt` |
| G-unit | `cargo test --lib --bins` | **FAIL: 266 failed / 2440 passed / 9 ignored**, plus **4 process-aborting tests** | `baseline/v3-unit-failures-complete.txt` (266 names), `baseline/v3-unit-aborters.txt` (4 names) |
| G-e2e `e2e_basic_lifecycle` | `cargo test --test e2e_basic_lifecycle` | 193 passed, **3 failed** | `baseline/v3-e2e_basic_lifecycle-failures-baseline.txt` |
| G-e2e `e2e_sync_git_safety` | `cargo test --test e2e_sync_git_safety` | 166 passed, **4 failed** | `baseline/v3-e2e_sync_git_safety-failures-baseline.txt` |
| G-e2e `jsonl_import_export` | `cargo test --test jsonl_import_export` | 158 passed, **23 failed** | `baseline/v3-jsonl_import_export-failures-baseline.txt` |

All paths above are relative to `docs/research/upgrade/` **in this repository** — they are tracked
files, not scratchpad droppings. Use them, and nothing else, as the comparison set.

#### 0.1.2 The four process-aborting unit tests

All in `cli::commands::doctor::tests`, all `execute_repair_indexes_*`; each kills the harness with
exit status 5, **silently truncating the run at whatever point it is reached**:

```
execute_repair_indexes_clears_stale_sidecar_snapshots_from_previous_run
execute_repair_indexes_dry_run_skips_mutation
execute_repair_indexes_quotes_names_that_need_quoting
execute_repair_indexes_succeeds_against_healthy_db_and_retains_snapshot
```

**This is why the v1 baseline said "9 failures".** A parallel run dies early and reports only what
finished first. Diffing against a truncated list is worthless in both directions: a genuine new
failure can hide behind the abort, and a *shift* in where the abort lands surfaces ~257 pre-existing
failures that read as NEW. §1 G-unit gives the two-tier procedure that avoids both.

**Five baseline facts that change how you work:**

1. **The clippy failure is FIRST-party lint debt, not a third-party build abort.** Under the
   canonical fenix pin, `sysinfo 0.39.6` compiles and clippy reaches the workspace's own code, where
   it finds **85 errors** in the `beads_rust` lib: 24 `clippy::pedantic`, 6 `clippy::nursery`, 6
   plain `-D warnings`, plus ~19 individually-named lints (`too-many-lines`, `large-enum-variant`,
   `incompatible-msrv`, `significant-drop-tightening`, `manual-let-else`, …). Kind/count inventory:
   `baseline/v3-clippy-lints-baseline.txt`.
   → G-lint is therefore a **no-NEW-warnings** gate against that inventory (§1), *not* a
   "substitute gate that avoids `--all-targets`". `--all-targets` works fine; it is simply red.
   → This answers **R3's open question 1**: upstream is **not** green under
   `clippy --all-targets -- -D warnings`, and the reason is upstream's own lint hygiene against a
   newer clippy, not a dependency incompatibility. Commit **P4-06** must be written accordingly.

2. **`e2e_sync_git_safety` already fires the project's own SAFETY VIOLATION alarm at baseline.**
   `integration_sync_only_touches_allowed_files` fails with
   `CREATED: .beads/.br-jsonl-write-<24hex>.lock` — the JSONL-family lock sidecar
   (`src/sync/mod.rs:1006`) is not in the test's hand-copied allowlist. **[V]**
   → This **closes R1's open question 2 in the negative**: the sidecar already trips the test on
   upstream/main. Adding `org`/`org.tmp` to `ALLOWED_EXTENSIONS` is *not* what breaks it, and
   **P3-03** must not be blamed for these four failures. It should, however, fix the drift (see
   P3-03 done-criteria).

3. **Most `jsonl_import_export` failures are macOS-environmental**, e.g.
   `Config("JSONL parent component for <external-path …> must not be a symlink and must be a
   directory")` — the `/tmp` → `/private/tmp` symlink on Darwin. **[V]** Do not "fix" these.

4. **266, not 9.** The complete unit-failure inventory is 266 names, obtained by a serial run with
   the four aborters skipped. It spans `audit`, `capacity`, `changelog`, `close`, `defer`,
   `doctor`, `doctor_subsystems::schema_migration`, and much more. **Never diff against a parallel
   run's visible names alone**; see §1 G-unit.

5. **The four red suites are the whole comparison set, and they are red *before you start*.** A
   green series does not mean a green tree. It means the failure-name sets did not grow. Every
   done-criterion in §2–§6 is written on that understanding.

### 0.2 Pre-flight checklist before P3-01

**Environment**

- [ ] Read `docs/research/upgrade/BASELINE.md` end to end and confirm
      `docs/research/upgrade/baseline/` contains all six inventory files.
- [ ] Confirm the canonical toolchain resolves:
      `nix shell 'github:nix-community/fenix/a7c3ef79859bf024cdc865a0ed8f14a4e3f31099#complete.toolchain' --command rustc --version`
      → `1.99.0-nightly (7608eb7b0 2026-08-05)`.
      **Do NOT use `nix develop`** — neither flake's dev shell can build this tree (§0.1.0).
- [ ] Confirm the clippy shim hazard is understood: `which -a cargo-clippy` shows a rustup shim
      ahead of any toolchain binary. Every lint gate in this document invokes `cargo-clippy`
      **inside** the nix shell. `cargo clippy` (with a space) is banned for the duration of the port.
- [ ] Re-run one cheap baseline gate (`cargo test --test e2e_basic_lifecycle`) and confirm it
      reproduces 3 failures with the recorded names. If it does not, re-baseline before proceeding.

**Repository**

- [ ] `git -C /Users/johnw/src/obr fetch upstream --prune`; confirm `upstream/main` = `d1fb0d3a`.
- [ ] Branch exists: `git checkout -b org-forward-port upstream/main`. **`main` is never touched.**
- [ ] `/Users/johnw/src/org2jsonl` is clean at `59521f99a490703d4d02f9b0f312a92ec9135ba8` and that
      rev is pushed to the public `github.com/jwiegley/org2jsonl` [R2 §5.1].

**Decisions (§9) that must be answered before the commit that consumes them**

- [ ] **U1** (`agent_context`), **U2** (description fidelity), **U3** (colon labels),
      **U-DRAFT** (Draft keyword), **U5** (`:BEADS_SCHEMA_VERSION:` rename timing) — all **P3-02**
      inputs.
- [ ] **U-EXTERNAL** (does `--allow-external-jsonl` accept `.org`?) — a **P3-03** input. This one
      previously carried *no recommendation*; §9 now offers one, but it still needs your sign-off,
      and P3-03 cannot be written without it.
- [ ] **Q-LINES** (import error shape) — a **P3-06** input, and it shapes P3-02's parser errors.
- [ ] **U6** (accept the Org auto-flush perf regression) — a **P3-07** input.
- [ ] **U-MERGEART** (do the merge artifacts stay JSONL?) — a **P3-08** input.
- [ ] **U4** (`init` seeding) — a **P3-09** input.
- [ ] **U-TESTPOLICY** — a P3-13 input, but it shapes every commit's done-criteria.

**Naming note that applies to every done-criterion below.** Commands are written as `obr <cmd>`
throughout for readability, but **the binary is literally `br` until P6-02 renames it.** Before
P6-02, read every `obr` in a done-criterion as `br` (or `target/debug/br`). From P6-02 onward the
spelling is literal. G-rename part 4 is the only place the distinction is load-bearing.

### 0.3 How to exercise Org **before** the default flips (P3-09)

Commits P3-03 … P3-08 need to run Org code paths while `DEFAULT_JSONL_FILENAME` is still
`"issues.jsonl"`. The switch is `metadata.json`'s `jsonl_export` field: `resolve_jsonl_path`
priority 2 (`src/config/mod.rs:4219-4262`) treats any value **different from the default** as an
explicit override. **[R1 §4.3]**, mechanism **[V]** at `src/config/mod.rs:62`.

```bash
# bring-up helper — the mirror image of the pin_jsonl test helper
jq '.jsonl_export = "issues.org"' "$WS/.beads/metadata.json" > tmp && mv tmp "$WS/.beads/metadata.json"
```

After P3-09 flips the default this same field, set to `"issues.jsonl"`, is what pins a workspace
back to JSONL — which is exactly the `pin_jsonl(&ws)` test helper in P3-13. One mechanism, both
directions.

> **The same mechanism is a trap in Phase 5, and it is the reason P5-02 exists as a separate
> commit.** This repository's own `.beads/metadata.json` already says
> `"jsonl_export": "issues.jsonl"` **[V]** — identically in `main` and in `upstream/main`. After
> P3-09, that stops being "the default" and becomes an **explicit JSONL override**, so a bare
> `sync --flush-only` on the project's own tracker writes `issues.jsonl` and every done-criterion
> about field fidelity still passes. Read P5-02 before you touch the tracker.

Note that `validate_sync_paths` (`src/cli/commands/sync.rs:1522`) runs on the *resolved*
path for every `sync` invocation, so its extension gate at `:1553` must be fixed in **P3-03**, not
later, or nothing can be tested. **[V]**

---

## 1. Gate definitions (bound to this plan)

Run gates from the branch worktree. "No new failures" always means: the failure-name set is a
subset of the corresponding durable inventory in `docs/research/upgrade/baseline/`.

**Every gate command below runs inside the canonical toolchain (§0.1.0).** Define this once per
shell and use it verbatim:

```bash
NIXRUN="nix shell github:nix-community/fenix/a7c3ef79859bf024cdc865a0ed8f14a4e3f31099#complete.toolchain --command"
BASE="$(git rev-parse --show-toplevel)/docs/research/upgrade/baseline"
```

**"all" means: G-build + G-locked + G-lint + G-unit + G-e2e-subset.** G-locked is *inside* "all"
(see M9 rationale in §1.1); the master table names it separately only where a commit is especially
likely to move a lockfile.

### G-build
```bash
$NIXRUN cargo build
```
Exit 0. Baseline: exit 0 (~145 s cold). **Hard gate — a red G-build is never acceptable.**

### G-locked  *(the plan §7.6 asks for it but never named it)*
```bash
$NIXRUN cargo build --locked
```
Exit 0. Baseline: exit 0 — upstream's committed `Cargo.lock` is valid, and org2jsonl must be added
without breaking that. The fork fails this today (dossier R14 / R30); the port must not inherit it.

**G-locked is part of "all" and therefore runs on every commit.** It is *load-bearing* on the
commits that can move a lockfile or a manifest identity:

| Commit | Why |
|---|---|
| **P3-01** | adds the only git dependency; the only Phase-3 commit allowed to change `Cargo.lock` |
| **P4-02** | rewrites `flake.nix` and commits the first `flake.lock` |
| **P4-05** | edits `fuzz/Cargo.toml` (adds two `[[bin]]` stanzas) |
| **P6-02** | **renames the package** — this necessarily rewrites `Cargo.lock`'s root entry (`Cargo.lock:347` is `name = "beads_rust"` **[V]**) and `fuzz/Cargo.toml`'s `beads_rust = { path = ".." }` dependency. `fuzz/Cargo.lock` is **untracked** (`.gitignore:21` **[V]**) so it simply regenerates, but a stale local copy will fail `cargo fuzz build` until deleted. |

### G-lint
```bash
$NIXRUN cargo-clippy --all-targets -- -D warnings 2>&1 | tee /tmp/clippy.log
grep -oE '^error(\[[A-Z0-9]+\])?' /tmp/clippy.log | wc -l          # baseline: 85
grep -oE '`-D clippy::[a-z-]+`|`-D warnings`' /tmp/clippy.log \
  | sort | uniq -c | sort -rn > /tmp/clippy-lints.txt
diff "$BASE/v3-clippy-lints-baseline.txt" /tmp/clippy-lints.txt
```

**Note the binary name: `cargo-clippy`, not `cargo clippy`.** The space form dispatches through a
rustup shim to a stale nightly and its output is meaningless here (§0.1.0 fact 2).

`--all-targets` **works** under the canonical pin — the earlier claim that it aborts inside `sysinfo`
was an artifact of the wrong toolchain. It is simply **red at baseline with 85 first-party errors**
(24 pedantic, 6 nursery, 6 plain `-D warnings`, ~19 named lints). The gate is therefore
**no-NEW-warnings** against `baseline/v3-clippy-lints-baseline.txt`, which is a lint-kind/count
inventory rather than a message list (messages carry line numbers that shift under every commit).

**Rule: the per-kind counts must not increase, and zero new warnings may be attributable to a file
this commit touched.** If a commit legitimately removes a lint (e.g. P6-03's `cargo fmt` pass), update
the inventory in the same commit and say so in the commit message.

Upstream already denies `pedantic` + `nursery` in `Cargo.toml [lints.clippy]` [R3 §2.6], so
`-D warnings` adds only rustc lints on top.

### G-unit

Two tiers, both from `BASELINE.md`. **The fast tier is not sufficient on its own** — the four
aborters (§0.1.2) truncate a parallel run at an unpredictable point.

```bash
# Tier 1 — fast check, run after every commit
$NIXRUN cargo test --lib --bins 2>&1 | tee /tmp/unit.log
grep -E '^test .* FAILED$' /tmp/unit.log | sed 's/^test //; s/ \.\.\. FAILED$//' \
  | sort -u > /tmp/unit-failures.txt
# Any name here that is NOT in the 266-name inventory is a NEW failure = red gate:
comm -23 /tmp/unit-failures.txt <(sort -u "$BASE/v3-unit-failures-complete.txt")   # EXPECT: empty
```

```bash
# Tier 2 — full serial check. MANDATORY at P3-09, P3-13, P6-01a, Phase 7 and CLEANUP;
# cheap insurance after any commit that touches doctor, sync or storage.
SKIP=$(sed 's/^/--skip /' "$BASE/v3-unit-aborters.txt" | tr '\n' ' ')
$NIXRUN cargo test --lib --bins -- --test-threads=1 $SKIP 2>&1 | tee /tmp/unit-serial.log
grep -E '^test .* FAILED$' /tmp/unit-serial.log | sed 's/^test //; s/ \.\.\. FAILED$//' \
  | sort -u > /tmp/unit-failures-serial.txt
diff <(sort -u "$BASE/v3-unit-failures-complete.txt") /tmp/unit-failures-serial.txt
```

**Red-gate conditions, all three:**
1. A FAILED name that is not in `v3-unit-failures-complete.txt` (266 names).
2. The tier-2 run aborts anyway — i.e. **a new aborter** appeared somewhere outside the four known
   `execute_repair_indexes_*` tests. Detect it: tier-2 must print a normal `test result:` summary
   line. Exit status 5 with no summary = a new aborter = red.
3. Tier-2's diff shows *removals* you did not intend. Fixing a pre-existing failure is welcome, but
   it must be deliberate and the inventory must be updated in the same commit.

**Do not** re-run tier 1 and conclude "only 9 failures, we are fine". That number came from a
truncated parallel run and is wrong.

### G-e2e-subset
Three targets, fixed for the whole port:
```bash
for t in e2e_basic_lifecycle e2e_sync_git_safety jsonl_import_export; do
  $NIXRUN cargo test --test "$t" 2>&1 | tee "/tmp/$t.log"
  grep -E '^ +[a-z0-9_:]+$|^test .* FAILED$' "/tmp/$t.log" | sort -u > "/tmp/$t-failures.txt"
  diff "$BASE/v3-$t-failures-baseline.txt" "/tmp/$t-failures.txt"
done
```
Baselines: 3, 4, 23 failures respectively (name lists in the three
`v3-*-failures-baseline.txt` files). **A new name in any list stops the series.**
From P3-11 onward, add `e2e_history`; from P3-12, add `e2e_doctor_fixture_suite` (establish their
baselines on first run and commit them into `baseline/` beside the others, named
`v3-<target>-failures-baseline.txt`).

### G-aux  *(new; closes the fuzz/benches blind spot)*

```bash
$NIXRUN cargo build --benches            # benches/{benchmarks,storage_perf}.rs
$NIXRUN cargo fuzz build                 # the fuzz/ crate and all its targets
```

**Why this exists.** `fuzz/` is a *separate crate* (`fuzz/Cargo.toml` declares package
`beads_rust-fuzz` with `beads_rust = { path = "..", default-features = false }`, and all 7 targets
`use beads_rust::` **[V]**), and `benches/storage_perf.rs` has 11 `beads_rust::` paths **[V]**.
Neither is compiled by G-build (`cargo build`), G-lint (`--all-targets` covers benches for *lints*
but the fuzz crate is outside the workspace), G-unit, or G-e2e. Without G-aux, **P6-02 and P6-03 can
ship an uncompilable fuzz crate and broken benches while every other gate stays green** — silently
destroying P4-05's own `org_parse` deliverable.

Required at **P4-05**, **P6-02**, **P6-03**, **Phase 7** and **CLEANUP**. `cargo fuzz build` needs
a nightly (the canonical pin is one) and `cargo-fuzz` on PATH; if it is unavailable, `cargo build
--manifest-path fuzz/Cargo.toml` is an acceptable substitute for the compile check but does **not**
substitute at P4-05, whose whole point is a working fuzz target.

### 1.1 Which gate runs where

"all" = G-build + G-locked + G-lint + G-unit(tier 1) + G-e2e-subset. On top of that:

| Extra gate | Commits |
|---|---|
| G-unit **tier 2** (serial) | P3-09, P3-13, P6-01a, Phase 7, CLEANUP |
| G-org-roundtrip | P3-09 onward, every commit |
| G-aux | P4-05, P6-02, P6-03, Phase 7, CLEANUP |
| G-rename | P6-04, P6-05, Phase 7, CLEANUP |
| `nix build` | P4-02 |
| `nix flake check` | P4-03 |
| `cargo test --test e2e_history` | P3-11 onward |
| `cargo test --test e2e_doctor_fixture_suite` | P3-12 onward |

### G-org-roundtrip
A **real test** (`tests/e2e_org_roundtrip.rs`, delivered by P3-13), not a manual script.
Concretely, per [R2 §7]:

*Setup* — throwaway workspace; `init`; ≥12 issues covering every `Status` (**including `Draft`**),
every `Priority` 0–4, a colon label (`provides:auth`) and a hyphen label, unicode + emoji + ZWJ in
title and description, a description containing `,*` and a `*`-leading line and a list and a table
and a `#+begin_src` block, a multi-paragraph description, `agent_context` JSON, ≥3 dependencies of
≥2 types, ≥5 comments, an `external_ref`, a tombstone, and one minimal issue.

*Cycle* — `sync --flush-only` → G1; copy G1 into a fresh workspace, `sync --import-only`,
`sync --flush-only` → G2; repeat → G3.

*Assertions*
1. G1 parses; `doctor` reports the export-file parse check OK with the right count.
2. Field-level equality between source DB and re-imported DB via `list --json` / `show --json`,
   with an explicit known-lossy allow-list that **must be empty** for: `id`, `title`, `status`,
   `priority`, `issue_type`, all timestamps, `assignee`, `owner`, all `source_*`, all tombstone
   fields, all compaction fields, `labels`, `dependencies`, `comments`, `agent_context`.
3. **G1 == G2 byte-for-byte** if §9 U2 resolves to option (a); otherwise **G2 == G3** and the
   one-cycle churn is recorded as an accepted defect with a tracker issue.
4. Repeated `sync --flush-only` in the same workspace is byte-identical (determinism).
5. `metadata.jsonl_content_hash` after export equals `compute_jsonl_hash(issues.org)`.
6. A JSONL-pinned workspace with the same issues produces a `.jsonl` whose re-import is
   field-identical (the JSONL path must not regress).

First required at **P3-09**; re-run at every subsequent commit.

### G-rename
The plan's §7.8 gate **does not work**: `git grep -E` is POSIX ERE where `\b` is a literal `b`
[R4 §10.1]. Use this, verbatim, from the repo root:

```bash
# part 1 — no beads branding outside the allowlist
git grep -Pn -i 'beads' -- \
  ':!docs/research' ':!CHANGELOG.md' ':!docs/operations/UPGRADE_LOG*' ':!UPGRADE_LOG.md' \
  ':!Cargo.lock' ':!*.db' ':!*.db-wal' ':!*.db-shm' ':!fuzz/corpus' \
| grep -vE '^(README\.md|LICENSE(\.md)?|docs/ORIGINS\.md):' \
| grep -vE 'LEGACY_|legacy_|_legacy|legacy '                    \
| grep -vE '^\.obr/issues\.org:'            # only if D-PREFIX chose "keep historical IDs"
# EXPECT: empty
#
# NOTE on the two UPGRADE_LOG files: they are DISTINCT documents, not a duplicate.
#   UPGRADE_LOG.md               6,355 bytes, dated 2026-05-14 (v0.2.10 upgrades)  [V]
#   docs/operations/UPGRADE_LOG.md  8,121 bytes, dated 2026-01-18 (earlier)        [V]
# `diff` reports them different; the root copy has 6 branded lines. Both are historical
# records (L8). P6-04 SHOULD `git mv UPGRADE_LOG.md docs/operations/UPGRADE_LOG-2026-05-14.md`
# so one pathspec covers both; until it does, the ':!UPGRADE_LOG.md' exclusion above is
# what keeps part 1 from failing on it.
#
# NOTE on docs/ORIGINS.md: it does NOT exist upstream [V] and is created by Phase 8
# (the README/Origins rewrite). The grep -vE above is inert until then; that is fine.

# part 2 — no br/bd binary or artifact names
git grep -Pn '(^|[^a-zA-Z0-9_/.-])(br|bd)([^a-zA-Z0-9_-]|$)' -- \
  ':!docs/research' ':!CHANGELOG.md' ':!Cargo.lock' ':!*.db*' ':!fuzz/corpus' \
| grep -vE '(^|[^a-z])(br|bd)-[0-9a-z]'     # opaque test-fixture issue IDs (L10)
| grep -vE '\.br_history|\.br_recovery|BR_|BD_'
# EXPECT: only lines inside legacy-compat shim functions

# part 3 — hard artifact names
git grep -Pn '\.beads\b|_beads\b|beads\.db|beads\.jsonl|BEADS_[A-Z]|beads://|"br\.[a-z]|beads-rust' -- \
  ':!docs/research' ':!CHANGELOG.md' ':!Cargo.lock' ':!*.db*' ':!fuzz/corpus'
# EXPECT: only legacy-compat shim code + tests/legacy_compat.rs

# part 3b — the repository's OWN tracker directory must have moved (B3)
test ! -e .beads && test -d .obr                       # the git mv landed
git ls-files .beads | grep . && echo 'FAIL: .beads still tracked'
git grep -Pn -i 'beads|(^|[^a-z])(br|bd)([^a-z]|$)' -- .obr \
  | grep -vE '^\.obr/issues\.org:'                     # only under D-PREFIX option (ii)
# EXPECT: empty. Upstream tracks TEN files under .beads/ [V]; every one is renamed,
# de-branded or deleted by P6-01a. See §5.2a.

# part 4 — the binary itself
test -x target/debug/obr && ! test -e target/debug/br
target/debug/obr --help    | grep -Piv 'obr' | grep -Pi '\bbeads\b|\bbr\b|\bbd\b'   # empty
target/debug/obr --version | grep -Pi 'beads|\bbr\b'                                # empty
target/debug/obr capabilities --format json | grep -Pi 'beads|"br\.|BD_|BR_'        # empty
target/debug/obr robot-docs guide | grep -Pi '\bbeads\b|\bbd\b'                     # empty
cd "$(mktemp -d)" && obr init && { find . -name '*beads*' -o -name '.br_*'; } | grep . && echo FAIL
```

**The allowlist — the only permitted residual hits** [R4 §9]:

| ID | Where | What is allowed |
|---|---|---|
| L1 | `README.md` § *Origins* | Credit to Steve Yegge's beads (concept) and Dicklesworthstone's beads_rust (codebase), with links. The one sanctioned prose mention. |
| L2 | `LICENSE` / `LICENSE.md` | Upstream copyright + rider text as required attribution. |
| L3 | `src/config/mod.rs` `mod legacy_compat` | `is_legacy_workspace_dir_name`, `LEGACY_DB_FILENAME`, legacy JSONL names, legacy user-config paths, legacy merge-artifact names. **Must all sit in one clearly-marked region.** |
| L4 | `src/config/mod.rs` / `src/cli/mod.rs` legacy-env block | `BEADS_*`, `BD_*`, `BR_*` accepted-with-warning names + the doctor env-scrub list. |
| L5 | `src/cli/commands/agents.rs` | `"<!-- br-agent-instructions-v"` legacy marker prefix. |
| L6 | `src/sync/org_bridge.rs` | `"BEADS_SCHEMA_VERSION"` accepted as a legacy drawer key. |
| L7 | `tests/legacy_compat.rs` | The single file exercising L3–L6. **This file does not exist upstream [V] and is created by P6-01a** — see M3 in §5.2a. It is not a pre-existing file you merely allowlist. |
| L8 | `CHANGELOG.md`, **both** `UPGRADE_LOG.md` (root, 2026-05-14) and `docs/operations/UPGRADE_LOG.md` (2026-01-18) — two distinct files **[V]** — and `docs/research/**` | Historical records; excluded by pathspec. |
| L9 | `.obr/issues.org` | **Only if D-PREFIX chooses "keep historical IDs".** Covers issue-ID literals of the form `beads_rust-<n>` in the tracker corpus. **It does NOT cover `.obr/config.yaml`'s `issue_prefix` key** — that key mints *future* IDs and must be de-branded regardless (see L11). |
| L10 | `tests/**` issue-ID literals matching `(br\|bd)-[0-9a-z]` | Opaque fixture IDs; `id` is excluded from `content_hash`, so renaming them is safe but touches ~3,400 lines for no user-visible gain. |
| L11 | *(nothing)* — **explicitly NOT allowlisted:** `.obr/config.yaml`, `.obr/metadata.json`, `.obr/.gitignore`, `.obr/*.md`, `.obr/interactions.jsonl` | Upstream tracks ten files under `.beads/` **[V]**. Every one is renamed, de-branded or deleted by **P6-01a**; none of them gets a pass. Listed here so the reader knows the omission is deliberate. |

**The `.obr/` tracker directory is the single largest thing part 1 and part 3 will trip over.**
Verified inventory of what upstream tracks under `.beads/` and the branding in each
(`grep -oi beads` + `grep -oPi '(^|[^a-z])(br|bd)([^a-z]|$)'`, occurrences not lines, **[V]**):

| File | beads | br/bd | Disposition (P6-01a) |
|---|---|---|---|
| `SYNC_CLI_FLAG_SEMANTICS.md` (336 ln) | 15 | 50 | de-brand: it documents **this** tool's sync flags |
| `SYNC_THREAT_MODEL.md` (188 ln) | 18 | 15 | de-brand |
| `SYNC_SAFETY_INVARIANTS.md` (238 ln) | 16 | 11 | de-brand |
| `README.md` (81 ln) | 20 | 9 | de-brand |
| `MCP_AGENT_MAIL_PATTERNS.md` (310 ln) | 0 | 20 | de-brand (`br` command examples) |
| `.gitignore` | — | — | rewrite per map row A16 |
| `config.yaml` | 2 | — | `issue_prefix`/`issue-prefix` → D-PREFIX answer |
| `metadata.json` | 1 | — | `"database": "beads.db"` → `"obr.db"` (row A3) |
| `interactions.jsonl` | 0 | 0 | 3 synthetic upstream test records (`actor: ubuntu`, `"test prompt"`) — **delete**, or carry; D-DELETIONS |
| `issues.jsonl` | — | — | replaced by `issues.org` in **P5-02**; `git rm` there, not here |

First required at **P6-04**; must be clean at **Phase 7** and again at **CLEANUP**.

---

## 2. Phase 3 — the Org forward-port

Sequential. Gate after every commit. Do not proceed on a red gate.

> **The four things that will kill a naive port**, all absent at the fork point, all handled below:
> P3-04 (canonical hash — *breaks loudly on the first flush*), P3-07 (line-splice incremental
> auto-flush — *breaks loudly*), P3-08 (byte-copy merge anchor — *breaks silently on every merge*),
> P3-10 (reviewed-additive reconcile — *needs explicit rejection*). [R2 §C5]

---

### P3-01 · `build: add org2jsonl git dependency pinned by rev`

**Scope.** Introduce the parser dependency as a *public, reproducible* git pin. Nothing else.

**Files.** `Cargo.toml`, `Cargo.lock`.

**Source of truth.** fork `33bb2e82`; [R2 §5]; plan §7.1 (whose description of the current pin is
wrong — see §10.C7).

**Do.**
```toml
# Cargo.toml, under [dependencies] (upstream's block starts at Cargo.toml:38)
# Org-mode parsing (sibling project; see docs/ORG_FORMAT.md)
org2jsonl = { git = "https://github.com/jwiegley/org2jsonl", rev = "59521f99a490703d4d02f9b0f312a92ec9135ba8" }
```
Full 40-hex rev, not the short form. Do **not** use a `path =` dep and do **not** add a flake input
(the flake work is P4-02, and upstream's crane-based flake needs no `outputHashes` [R2 §5.3]).

**Why this rev.** `org_to_entries_with_keywords` has a byte-identical signature at `5ea7586` and
`59521f9`; `orgize` stays pinned at `0.10.0-alpha.10`; the model types change only additively
(`OrgEntry` gains `file`/`char_begin`/`char_end`/`line_begin`/`line_end`). It is the rev the fork
has actually been compiling all along, it is public, and its license is BSD-3-Clause. [R2 §5.2]

**Done-criteria.**
- `cargo build --locked` exits 0.
- `cargo tree -p org2jsonl` reports the pinned rev.
- `git diff --stat` touches exactly `Cargo.toml` and `Cargo.lock`.
- Upstream has **zero** other git dependencies (`toon_rust` is the crates.io package `tru` at
  `Cargo.toml:92`) [R3 §2.1] — after this commit there is exactly one.

**Gate.** G-build, G-locked.

---

### P3-02 · `feat(sync): add org_bridge module (emit + parse), no callers`

**Scope.** The whole Org serializer/parser as a self-contained module with its own unit tests. No
call sites anywhere else in the tree; the binary's behaviour is unchanged by this commit.

**Files.** new `src/sync/org_bridge.rs`; one `pub mod org_bridge;` line in `src/sync/mod.rs`
(beside `pub mod history;` / `pub mod path;`).

**Source of truth.** `fork:src/sync/org_bridge.rs` (900 lines, **[V]**), fork commits `afe1351e`,
`03c5cd2c`, `56e4a0fc`; the normative spec is [R2 §1] in full.

**Port faithfully (behaviour, not code shape).**

*File layout* [R2 §1.3] — two header lines then a blank line, even for an empty issue set:
```
#+TITLE: Beads Issues
#+SEQ_TODO: TODO DOING DRAFT WAIT DEFER NOTE | DONE CANCELED
```
The `#+SEQ_TODO:` line is written but never read; the parser uses the compiled-in keyword arrays.

*Heading* [R2 §1.4] — `"* " KEYWORD " " COOKIE " " sanitized_title` then, only when labels are
non-empty, exactly four spaces and `:l1:l2:`. Level 1 always. Cookie always emitted (`[#C]` for
MEDIUM). Labels sorted ASCII-ascending. (Superseded by 4afe14c2: four spaces is still the
default, but a surface declaring `# -*- org-tags-column: N -*-` is aligned by Org's own rule.)

*Status ↔ keyword* [R2 §1.5] — the **exact** map (the plan's §6.3 example is wrong, §10.C1):

| Export | | Import accepts |
|---|---|---|
| `Open` → `TODO` | | `TODO`, `OPEN` → `Open` |
| `InProgress` → `DOING` | | `DOING`, `IN_PROGRESS`, `INPROGRESS` → `InProgress` |
| `Blocked` → `WAIT` | | `WAIT`, `BLOCKED` → `Blocked` |
| `Deferred` → `DEFER` | | `DEFER`, `DEFERRED` → `Deferred` |
| **`Draft` → `DRAFT`** *(new; §9 U-DRAFT)* | | **`DRAFT`, `DRAFT`→`Draft`** |
| `Closed` → `DONE` | | `DONE`, `CLOSED` → `Closed` |
| `Tombstone` → `CANCELED` | | `CANCELED`, `TOMBSTONE` → `Tombstone` |
| `Pinned` → `NOTE` | | `NOTE`, `PINNED` → `Pinned` |
| `Custom(s)` → **refuse to export** (see below) | | anything else → `Status::from_str` |

Comparison on `keyword.to_uppercase()`. `Status::Draft` is new upstream (`src/model/mod.rs:64`,
wire string `draft`) **[V]** and both arrays must gain it:
`ORG_TODO_KEYWORDS = ["TODO","DOING","DRAFT","WAIT","DEFER","NOTE"]`,
`ORG_DONE_KEYWORDS = ["DONE","CANCELED"]`.

*Priority ↔ cookie* [R2 §1.6] — `0..4` → `[#A]..[#E]`; anything else collapses to `[#C]`/`2`. Keep
the collapse, but add `tracing::warn!` naming the issue id when it fires (today it is silent).

*Drawer* [R2 §1.7] — **29 properties in a fixed emission order**, not the three the plan mentions
(§10.C2). Non-obvious guards: `COMPACTION_LEVEL` only when `Some(l) && l > 0`;
`EPHEMERAL`/`PINNED`/`IS_TEMPLATE` only when `true`. `:ID:` is the only property required on read.
`:ID:` uses the format string `":ID:       {}\n"` (seven spaces).

*New upstream fields* — the model gained **exactly two** since `e805fe98` [R1 §6.2] **[V]**:
- `source_repo_path: Option<String>` → drawer property `:SOURCE_REPO_PATH:`, inserted immediately
  after `:SOURCE_REPO:`, sanitized, `Some`-guarded.
- `agent_context: Option<String>` (a JSON document in a TEXT column) → **§9 U1**. Recommended: a
  level-2 child `** Agent Context` with a `#+begin_src json` block, stored **verbatim** (do not
  re-serialize — that would rewrite the DB's stored text and change `content_hash` for untouched
  issues). A drawer property is *wrong*: `sanitize_property_value` flattens newlines to spaces,
  producing lossy-but-still-parsing JSON.

**Fix these fork defects while porting** (each is cheap and each is a "silent drop", the codebase's
#1 historical failure mode):

| ID | Defect | Fix |
|---|---|---|
| B10 | deps/comments serialization failure writes the literal `"[]"` (`fork:org_bridge.rs:216,228`) | propagate the error |
| — | `ISSUE_TYPE` emitted **unsanitized** (`fork:org_bridge.rs:82`) | route through `sanitize_property_value` |
| B4 | `sanitize_org_text`/`unsanitize_org_text` are not inverses — a literal `,*` loses its comma forever | symmetric pair: on write prefix a comma to any line starting `*` **or** `,`; on read strip exactly one leading comma from `,*` or `,,`. Add a proptest. |
| B5 | `Status::Custom(s)` exports to an unrecognised keyword that reads back into the *title* with status silently `Open` | **refuse to export** an Org file containing a `Custom` status; error names the issue id and the status |
| B1/R9 | a label containing `:` becomes two labels on round-trip — and `provides:<cap>` is the live wire format of upstream's capability-dependency feature (`src/storage/sqlite.rs:16187` builds it, `:16214` strips it) | **§9 U3.** Recommended: emit a canonical `:LABELS:` drawer property (compact JSON array) as authoritative, *plus* Org tags for ergonomics; on read `:LABELS:` wins, else fall back to `heading.tags` |
| B8 | a title ending `:foo:` is truncated and injects a phantom label | refuse to emit a title matching `:\w+(:\w+)*:$` without escaping; proptest |
| R23/B6 | round-trip is not a fixpoint (9.4 % of descriptions lose content on pass 1) | **§9 U2.** Recommended (a): preserve-by-raw-slice using `char_begin`/`char_end` whenever a body contains any non-`Paragraph` element |

**Mandatory new test — the single highest-leverage line in this whole document** [R2 §2.3]:
```rust
#[test]
fn org_emission_covers_every_issue_field() {
    let Issue { id, content_hash, title, description, /* … all 41 fields … */ } = Issue::default();
    // touch each binding; a new upstream field then breaks the BUILD instead of vanishing silently
}
```
Two upstream fields would have disappeared without this recon. Nothing in the fork enforced coverage.

**Shape the API record-oriented, not whole-file.** The fork built one `String` for the entire file;
upstream streams, batches at 1,024 issues and prepares records on up to 64 threads. Export exactly:
```rust
pub enum ExportFormat { Jsonl, Org }
impl ExportFormat {
    pub fn for_path(path: &Path) -> Self;          // extension == Some("org"), lowercase
    pub const fn temp_extension(self) -> &'static str;   // "org.tmp" | "jsonl.tmp"
}
pub fn org_file_header() -> &'static [u8];         // the two header lines + blank
pub fn emit_issue_record(issue: &Issue) -> Result<Vec<u8>>;   // pure, no cross-issue state
pub fn org_text_to_issues(text: &str) -> Result<Vec<Issue>>;
```
`emit_issue_record` **must be pure** — no shared hasher, no comment renumbering — or the parallel
export path (`prepare_export_issues_jsonl_parallel`, `:9506`) becomes nondeterministic. **[V]**

**One helper, not four probes.** The fork had three subtly different spellings of
`extension == Some("org")`; upstream needs ~15 more sites. Every one of them dispatches on
`ExportFormat::for_path`. Never re-derive the extension inline.

**Done-criteria.**
- Module compiles; `cargo test --lib org_bridge` green, including: the field-coverage
  destructure, a `Draft` round-trip, a colon-label round-trip (if U3 = fix), a `,*` proptest, a
  `Custom`-status export refusal, and an `agent_context` round-trip (if U1 = carry).
- `grep -rn "org_bridge" src/ --include=*.rs | grep -v org_bridge.rs` → only the `pub mod` line.
- Binary behaviour is bit-identical to P3-01 (the module is unreachable).
- `org_bridge.rs` has **zero** `rusqlite` references — verified true of the fork's file **[V]**, so
  the `fsqlite` swap (§10.C6) costs nothing here. (This closes R3 open question 4.)

**Gate.** G-build, G-lint, G-unit.

---

### P3-03 · `feat(sync): admit .org through every path gate`

**Scope.** Make `.org` a legal name everywhere the safety layer decides what sync may touch. No
Org bytes are written yet; this commit only unlocks the doors. **This is plan landmine §7.5, and
the plan looked in the wrong place** (§10.C3).

**Files.** `src/sync/path.rs`, `src/sync/mod.rs` (temp-name generator only),
`src/cli/commands/sync.rs`, `tests/e2e_sync_git_safety.rs`.

**Source of truth.** [R1 §3.4], [R2 §3.3, §3.16], fork `afe1351e`.

**Do — each anchor verified [V] this session:**

| Anchor | Current | Change |
|---|---|---|
| `src/sync/path.rs:85-94` `ALLOWED_EXTENSIONS` | 8 entries, no `org` | add `"org"`, `"org.tmp"` |
| `src/sync/path.rs:16-27` module doc table | repeats the list in prose | update |
| `src/sync/path.rs:546-559` `is_allowed_jsonl_temp_name` | accepts `*.jsonl.tmp` and `<base>.jsonl.<digits>.tmp` only | generalize to both extensions; **rename to `is_allowed_export_temp_name`** |
| `src/sync/path.rs:679-690` `validate_external_jsonl_path` | `must be a .jsonl file` hard-reject | **§9 U-EXTERNAL**; if accepted, allow `.org` |
| `src/sync/mod.rs:1818` `export_temp_path_for_attempt` | `output_path.with_extension("jsonl.{pid}.tmp")` | dispatch on `ExportFormat::for_path(output_path).temp_extension()` |
| `src/cli/commands/sync.rs:1553` extension gate | `!= Some("jsonl")` → `Config("JSONL path must end with .jsonl")` | `!matches!(ext, Some("jsonl" \| "org"))`, message `"Sync path must end with .jsonl or .org"` |
| `tests/e2e_sync_git_safety.rs:1274-1283` | a **second, hand-copied** `ALLOWED_EXTENSIONS` | **import `beads_rust::sync::path::ALLOWED_EXTENSIONS`** instead of re-declaring |

**The silent-wrongness trap.** `Path::with_extension` *replaces* the final extension, so an
`issues.org` target currently yields `issues.jsonl.<pid>.tmp` holding Org bytes, which then gets
`RENAME_EXCHANGE`d onto `issues.org`. It would work and be wrong and no test would catch it
[R1 §2.3] **[V]**. And the moment you fix it, the fixed name is *rejected* by
`is_allowed_export_temp_name` unless both land in this same commit.

**Do not touch:** `validate_pinned_jsonl_leaf` (`:949-979`, extension-agnostic),
`validate_jsonl_fd_metadata` (`:2548`, regular-file check only), `scan_conflict_markers`
(`sync/mod.rs:8932`, textual and works on Org unchanged). [R1 §3.4e], [R2 §3.3]

**Done-criteria.**
- Unit tests: `export_temp_path(".../issues.org")` ends `.org.<pid>.tmp` and is accepted by the
  temp-name validator; `.jsonl` behaviour unchanged.
- `grep -rn "ALLOWED_EXTENSIONS" tests/` returns **zero** re-declarations.
- `sync --flush-only` against an `issues.org`-pinned workspace (§0.3) now fails *later* than the
  extension gate (it will still fail on the content hash — that is P3-04).
- **G-e2e-subset `e2e_sync_git_safety` shows no new failures beyond the four baselined in §0.1.**
  The `.br-jsonl-write-*.lock` violations are pre-existing; you are not expected to fix them, but
  note the fix (add the lock-sidecar glob to the shared allowlist) is now a one-line option since
  the test imports the constant.

**Gate.** G-build, G-unit, G-e2e-subset.

---

### P3-04 · `fix(sync): make the canonical content hash format-aware`

**Scope.** The single change without which **every `sync --flush-only` and every auto-flush in
every Org workspace errors on the first export.** Fix it before writing a single Org byte.

**Files.** `src/sync/path.rs`, `src/sync/mod.rs`.

**Source of truth.** [R2 §3.5] (the critical finding); [R2 §C9] correcting `map-sync.md` §1.3.

**The mechanism [V].** Upstream computes two digests per file: `raw_sha256` (bytes as-is) and
`content_sha256` (canonical). The canonical one, identically at `src/sync/path.rs:2293-2299` and
`src/sync/mod.rs:12783-12787`, is:
```rust
let trimmed = line.trim_ascii();
if !trimmed.is_empty() { hasher.update(trimmed); hasher.update(b"\n"); }
```
— **blank lines skipped, every line trimmed on both sides.** For JSONL this equals a straight byte
hash. For Org it does not: the format has blank lines by design and `to_string_pretty` indents.
The exporter then hashes the bytes it writes and asserts equality against the staged file's
canonical hash (`src/sync/mod.rs:9951-9957`). That assertion fails on the very first export.
The fork never hit it because its own `compute_jsonl_hash` (`fork:src/sync/mod.rs:2551-2569`) only
stripped trailing newlines.

**Do — define Org's canonical form as the raw bytes.**

| Anchor | Change |
|---|---|
| `src/sync/path.rs:2274` `compute_snapshot_content_sha256(backing: &File, deadline)` | **the signature carries no path** **[V]** — add an `ExportFormat` parameter. For `Org`, return the already-computed `raw_sha256` rather than re-hashing line-canonically. |
| `src/sync/path.rs:~2407` (the sole call site, inside `capture_jsonl_source_snapshot`) | `path` **is** in scope here **[V]** — pass `ExportFormat::for_path(path)` |
| `src/sync/mod.rs:12769` `compute_jsonl_hash_from_reader` | leave as the JSONL implementation; add `compute_org_hash_from_reader` (raw bytes) |
| `src/sync/mod.rs:12805` `compute_jsonl_hash(path)` | dispatch on the path's extension |

**Consequence, and it is in the safe direction [I].** Org staleness detection becomes byte-exact
rather than whitespace-insensitive. An edit that only changes indentation inside a src block is now
*detected* instead of ignored — the ignore case is a data-loss path, because the DB would believe
it was in sync and overwrite the edit on the next flush.

**Done-criteria.**
- New unit test: an Org text containing blank lines, indented JSON and a trailing newline hashes
  identically through the export-side path and the file-side path.
- JSONL digests are bit-identical to before (regression test on a fixture).
- `metadata.jsonl_content_hash` written on import (`src/sync/mod.rs:12641`) and
  `ExportResult.content_hash` are demonstrably the same function for both formats.

**Gate.** G-build, G-unit.

---

### P3-05 · `feat(sync): Org record emission on the export path`

**Scope.** Teach the four emission sites and the two verification helpers to produce and read Org.

**Files.** `src/sync/mod.rs` (export region `:8246`–`:10200`).

**Source of truth.** [R2 §3.4, §3.8], [R1 §2.1, §2.6].

**Do — all anchors verified [V]:**

| Anchor | Function | Change |
|---|---|---|
| `:9421` | `struct PreparedExportIssue` | rename field `jsonl_line: Vec<u8>` → `record_bytes: Vec<u8>` |
| `:9386` | `write_export_issue_jsonl` | take `ExportFormat`; for Org emit `org_bridge::emit_issue_record` |
| `:9463` | `prepare_export_issue_jsonl` (parallel worker) | same |
| `:9493` / `:9506` | `prepare_export_issue_chunk` / `..._parallel` | thread the parameter |
| `:9555` | `write_prepared_export_entries` | no change beyond the field rename |
| `:10150` | `export_to_writer_with_policy_and_retention_at` (**stdout export — the third site**) | same |
| `~:9894` | inside `export_to_jsonl_with_policy_expected_authority`, right after `let mut hasher = Sha256::new();` | when `format == Org`, write `org_file_header()` to **both** `writer` and `hasher`. **This is the only cross-record state in Org emission.** |
| `:9081` | `verify_exported_jsonl_snapshot_integrity` | Org branch: parse the staged text, assert the id **set** equals `exported_ids` and the count matches. Keep the `"Export verification failed: …"` prefix — doctor string-matches on it. |
| `:9022` | `analyze_jsonl_from_reader` | Org branch returning `(count, ids)`; **must reproduce the duplicate-id hard error** (see below). Prefer counting `:ID:` on level-1 headings without constructing `Issue`s, as the fork did. |
| `:8246` | `validate_jsonl_issue_records_from_reader` | Org branch: parse, `normalize_issue`, validate, collect failures with heading ordinals |

**Duplicate-id parity is mandatory (dossier R21 / B9).** Upstream now **errors** on duplicate ids in
every JSONL reader (`:9036`, `:11784`, `:12119`) **[V]**. The fork's `count_issues_in_org` did not,
and 100 headings sharing one `:ID:` import silently, last-one-wins. Without parity the Org path is
strictly less safe than the format it replaces.

**Do not touch:** tombstone skipping, progress reporting, `issue_hashes`, `report.*` counters,
batching thresholds, the parallel fan-out shape, `publish_staged_jsonl_conditionally` and the whole
CAS publication layer (`:1464`–`:1778`) — all format-agnostic. [R1 §2.2]

**Done-criteria.**
- In a workspace pinned to `issues.org` (§0.3), `sync --flush-only` produces a well-formed Org file
  containing every issue exactly once, in id-sorted order.
- A second `sync --flush-only` is **byte-identical** to the first.
- `BR_DISABLE_PARALLEL_JSONL_EXPORT=1` and the parallel path produce **byte-identical** output
  (`src/sync/mod.rs:9452` is the escape hatch; useful during bring-up). [R1 §4.6]
- A JSONL-pinned workspace's export is byte-identical to pre-commit output.

**Gate.** G-build, G-lint, G-unit, G-e2e-subset.

---

### P3-06 · `feat(sync): Org import through one format-aware chokepoint`

**Scope.** One function, four callers, three subsystems fixed at once.

**Files.** `src/sync/mod.rs` (import region `:11744`–`:12660`, plus `:13008`).

**Source of truth.** [R2 §3.6], [R1 §2.4].

**The chokepoint [V].** `for_each_jsonl_import_issue` (`src/sync/mod.rs:12077`) is the **only**
line→`Issue` iterator, called from `collect_import_validation_plan` (`:12106`),
`scan_import_collision_renames` (`:12284`), `stream_import_actions_in_tx` (`:12434`) and
`for_each_reconcile_classified_row` (`:13008`). Making it format-aware fixes import, `--no-db`
seeding, and `sync --reconcile` classification together.

```rust
fn for_each_import_issue(source: &JsonlSourceSnapshot,
                         mut handle: impl FnMut(usize, Issue) -> Result<()>) -> Result<()> {
    match ExportFormat::for_path(source.display_path()) {
        ExportFormat::Jsonl => { /* existing line loop, untouched */ }
        ExportFormat::Org => {
            for (idx, mut issue) in cached_org_issues(source)?.into_iter().enumerate() {
                normalize_issue(&mut issue);
                validate_or_err(&issue, idx + 1)?;   // same shape as :12061-12072
                handle(idx + 1, issue)?;
            }
        }
    }
}
```

**Three details that matter.**
1. **Do not skip normalization/validation.** `parse_normalized_import_issue` (`:12056`) applies
   `normalize_issue` + `IssueValidator::validate` per record. Org must too, or doctor results
   diverge between formats.
2. **Do not lie about line numbers.** The `line_num` argument feeds `"Prefix mismatch at line N"`,
   `"Duplicate issue id … at line N"`. Until the org2jsonl `line_begin` plumbing lands (§9 Q-LINES),
   pass the heading ordinal and say **"heading #N"**.
3. **Cache the parse.** Three passes = three orgize parses, on top of an O(n²) worst case.
   Parse once into a `Vec<Issue>` hoisted into `import_from_jsonl_snapshot` (`:12561`) and pass it
   down, or memoize on the snapshot.

**Also update:** `read_issues_from_jsonl` (`:11744`) and `..._snapshot` (`:11752`) — used by merge
and base-snapshot loading. **Do not touch** `ensure_no_conflict_markers*` (`:8962`) — textual, works
on Org, and the fork's `c046b35e` migrated exactly this test to Org and it passed. [R2 §3.6]

**Done-criteria.**
- `sync --import-only` on a hand-written `issues.org` populates the DB; `list --json` matches.
- Duplicate `:ID:` is a hard error with a heading ordinal.
- A prefix-mismatch and a malformed-heading case each produce an actionable message naming the
  heading, not a bare parse failure.
- Import of a `.jsonl` file is byte-for-byte behaviourally unchanged (the whole
  `jsonl_import_export` baseline set is unmoved).
- The three-pass import parses the Org text **once** (assert with a counter in a test build, or
  measure).

**Gate.** all (G-build, G-lint, G-unit, G-e2e-subset).

---

### P3-07 · `fix(sync): decline incremental auto-flush for non-JSONL exports`

**Scope.** Three lines of code, one of the two loudest failure modes.

**Files.** `src/sync/mod.rs`.

**Source of truth.** [R1 §2.5] (the hazard), [R2 §3.7] (the fix).

**The hazard [V].** `auto_flush` (`src/sync/mod.rs:11620`) branches at `:11678`: when the DB is
dirty but not `needs_flush` — **the common path for ordinary mutations** — it calls
`try_incremental_auto_flush` (`:11492`), which splices individual JSONL *lines* in place:
`scan_existing_jsonl_replacements` (`:11073`) parses every line as `PartialId`;
`write_existing_jsonl_replacements_atomically` (`:11225`) substitutes byte ranges; the fallback path
is `read_jsonl_lines_by_id` (`:9183`) + `write_jsonl_lines_atomically` (`:11320`). Every step
assumes one issue = one line. Against an Org file it either errors inside `serde_json::from_str` on
`#+TITLE: …` or produces a garbage file.

**Do.** At the top of `try_incremental_auto_flush` (`:11492`), immediately after the
`!jsonl_path.exists()` guard **[V]**:
```rust
if ExportFormat::for_path(jsonl_path) != ExportFormat::Jsonl {
    return Ok(None);   // decline; the caller falls through to the full exporter
}
```
`Ok(None)` is the existing "declined, use the full exporter" contract (`:11562`). **Do not** attempt
a block-splicing Org incremental path — that is a separate project and it would have to interact
with `emit_issue_record`'s blank-line accounting.

**Accepted regression (§9 U6).** Every mutating command in an Org workspace does a full re-export.
Upstream has a dedicated guard for the JSONL fast path (`tests/repro_auto_flush_inefficiency.rs`) —
pin it to JSONL (Class A) rather than weakening it.

**Done-criteria.**
- In an Org workspace, `create`/`update`/`close` each produce a correct, complete, id-sorted
  `issues.org`; no partial-line artefacts.
- In a JSONL workspace, the incremental path still fires (assert via the existing repro test).
- **Both** auto-flush failure reporters name the resolved path, not a literal. There are two, not
  one, and the second was previously unassigned:

  | Anchor | String | Change |
  |---|---|---|
  | `src/cli/commands/mod.rs:86-133` `report_auto_flush_failure` | remediation sentence hardcodes `.beads/issues.jsonl` [R1 §7.2] **[V]** | resolved path |
  | **`src/mcp/mod.rs:120`** **[V]** | `"recovery": "Run br sync --flush-only after fixing the export problem before committing .beads/issues.jsonl"` — the **MCP** flavour of the same remediation, emitted to agent clients | resolved path; `jsonl_path` is already in scope two lines above at `:118` |

  The `.beads` and `br` halves of both strings are Phase 6 (P6-01a / P6-02); the `issues.jsonl` half
  is this commit's, because this is the commit that makes the filename format-dependent.

**Gate.** all.

---

### P3-08 · `fix(sync): re-serialize the merge base anchor for Org workspaces`

**Scope.** The one failure in this list that is **silent**. Skip it and every three-way merge in an
Org workspace gets an empty or erroring base, forever, with no error message.

**Files.** `src/sync/mod.rs`.

**Source of truth.** [R2 §3.9].

**The mechanism [V].** `refresh_base_snapshot_from_flushed_jsonl_snapshot`
(`src/sync/mod.rs:13961`) deliberately **byte-copies** the live export into `.beads/beads.base.jsonl`
(the doc comment at `:13936-13940` says the byte-copy is intentional). `load_base_snapshot`
(`:14018`) then parses that file. With an Org live file and extension-keyed dispatch (P3-06), the
anchor is Org text in a `.jsonl`-named file and is parsed as JSONL.

**Do.** For `ExportFormat::Org`, call `save_base_snapshot_from_jsonl_snapshot` (`:13923`) instead of
the byte copy: it parses and re-serializes into canonical JSONL. The anchor stays machine-only,
stays `.jsonl`, stays parseable, and **no new filename enters `ALLOWED_EXACT_NAMES`, the doctor's
base-anchor checks (`doctor.rs:7959`, `:10202`), or the Phase-6 rename map.**

Rejected alternative: writing `beads.base.org` and making the loader extension-aware. More faithful,
but it touches path validation, two hardcoded doctor sites, `legacy_backup_target_path`, and the
rename map. Not worth it. The property lost — "the anchor is byte-identical to the export" — is one
the Org pipeline cannot honour anyway.

**Done-criteria.**
- After an Org flush, `.beads/beads.base.jsonl` is valid JSONL whose id set equals the DB's.
- `sync --merge` smoke test in an Org workspace reaches its normal outcome rather than an empty
  base. (Note: dossier R10 says merge was structurally non-functional in the fork; upstream
  rewrote it. This commit is a *prerequisite* for merge working at all under Org, not a claim that
  merge is correct — see §8 R10.)
- **§9 U-MERGEART** decides whether the other merge artifacts (`beads.left/right.jsonl`,
  `sync_base.jsonl`, `deletions.jsonl`, `interactions.jsonl`) stay JSONL. This commit assumes yes.

**Gate.** all.

---

### P3-09 · `feat: make issues.org the default export artifact`

**Scope.** The switch. Everything before this was reachable only by pinning; after this, Org is what
a fresh workspace gets. **Nothing after this point is testable until this lands, and nothing before
it is safe to land after.**

**Files.** `src/config/mod.rs`, `src/cli/commands/init.rs`, `src/cli/commands/stats.rs`.

**Source of truth.** fork `afe1351e` + `a0c54428`; [R2 §1.2, §3.2, §3.14, §3.15], [R1 §4].

**Do — anchors verified [V]:**

| Anchor | Change |
|---|---|
| `src/config/mod.rs:62` | `pub const DEFAULT_JSONL_FILENAME: &str = "issues.org";` (upstream's is private; the fork makes it `pub` because `history.rs`, `stats.rs` and `sync/mod.rs` need it) |
| `src/config/mod.rs:64` | `LEGACY_JSONL_FILENAME = "issues.jsonl"`; add `OLDER_LEGACY_JSONL_FILENAME = "beads.jsonl"` |
| `src/config/mod.rs:147-162` `discover_jsonl` | three tiers: `issues.org` → `issues.jsonl` → `beads.jsonl` |
| `src/config/mod.rs:4219-4262` `resolve_jsonl_path` | **no logic change** — but read the trap below and write the test |
| `src/config/mod.rs:3352` | `join("issues.jsonl")` → `join(DEFAULT_JSONL_FILENAME)` (the `--no-db` `is_default_path` check) |
| `src/config/mod.rs:4146-4186` `first_prefix_from_jsonl_snapshot` / `_reader` | **needs an Org branch.** It raw-parses JSON to infer the issue prefix during `--no-db` bootstrap. Without it, a fresh `--no-db` open of an Org workspace infers the prefix from the directory name and every new issue gets the wrong prefix. Port **upstream's** semantics (first non-tombstone id wins), not the fork's (all ids must agree). |
| `src/cli/commands/init.rs:171-174` | metadata template `"jsonl_export": "issues.org"` — build it from the constant with `format!`, not a literal |
| `src/cli/commands/init.rs:252-257` | the empty seed file → `issues.org`, content `""` (not the header: a pristine `init` must hash the same as an empty export). **§9 U4** on the `bv` viewer contract. |
| `src/cli/commands/init.rs:360, :406, :464, :638` | step label, ASCII tree art, two unit tests |
| `src/cli/commands/stats.rs:92` | `\|\| beads_dir.join("issues.jsonl")` → `DEFAULT_JSONL_FILENAME`. **One line.** The fork's `a0c54428` stats fix is superseded: `compute_recent_activity` (`:644-656`) already takes an explicit path and computes a cached repo-relative git pathspec [R2 §C3] **[V]**. |

**The trap, and it must be a deliberate tested decision, not an accident** [R1 §4.3]. Flipping the
default silently reclassifies every existing `"jsonl_export": "issues.jsonl"` — which `br init`
writes today and every workspace on disk contains — from "the default" to "an explicit override".
That is **correct and desirable**: legacy workspaces keep reading their JSONL and do not silently
change format. Do not "fix" it. Write two tests: (a) a workspace whose `metadata.json` says
`issues.jsonl` keeps using JSONL after the flip; (b) a fresh `init` produces `issues.org`.

**Done-criteria.**
- Fresh `obr init` creates `.beads/issues.org` and a `metadata.json` naming it.
- A workspace created before the flip is unchanged in behaviour.
- `--no-db list` on a hand-written `issues.org` works, and `--no-db create` mints an id with the
  **correct prefix**.
- **G-org-roundtrip runs for the first time and passes** (as a manual script here if P3-13 has not
  landed; it becomes a real test there).

**Gate.** all + G-org-roundtrip.

---

### P3-10 · `feat(cli): Org-aware sync/vcs surface; reject Org for JSONL-only modes`

**Scope.** Make the commands that did not exist at the fork point either work or refuse honestly.

**Files.** `src/cli/commands/vcs.rs`, `src/cli/commands/sync.rs`, `src/sync/mod.rs`, `src/cli/mod.rs`.

**Source of truth.** [R2 §3.10, §3.11, §3.17], [R1 §3.1].

| Anchor | Current | Change |
|---|---|---|
| `src/cli/commands/vcs.rs:259-263` **[V]** | `if anchored.extension() != Some(OsStr::new("jsonl")) { Err("the diagnostic target must have a .jsonl extension") }` | accept `org`. This command did not exist at the fork point and its default target is the resolved export path — **left alone, `obr vcs-status` dies on every Org workspace.** |
| `src/sync/witness.rs` + `src/cli/commands/sync.rs:1994, 2045, 2086` | Merkle witness is line-**chunk**-oriented (`chunk_size_lines`, `witness.rs:29`) and reachable only from `sync --witness` | **Reject `--witness` on a `.org` target** with `"chunk witnesses are only defined for line-oriented JSONL exports"`. Leave `witness.rs` itself untouched — it is byte-oriented and would produce correct-but-meaningless digests. Update the help text at `src/cli/mod.rs:2915`. |
| `src/sync/mod.rs:3694` / `:3759` (`plan_/apply_reviewed_additive_reconcile`), `:4565` `reject_unknown_additive_fields`, `:4683` `parse_strict_additive_issue` | semantics defined in terms of *unknown JSON object keys* | **Reject a non-`.jsonl` source** with `"reviewed-additive reconcile requires a JSONL source; pass --jsonl <path.jsonl>"`. The Org drawer ignores unknown properties **by design** — the exact opposite of "strict". A strict mode that silently isn't strict is worse than an absent feature. |
| `src/sync/mod.rs:8321` `preflight_export`, `:8548/:8566` `preflight_import` | consume the same snapshot/validation helpers, so they inherit format-awareness | **audit their user-facing strings for hardcoded "JSONL"** [R2 §3.19] |

**Also in this commit — the published agent-facing strings that say `issues.jsonl` out loud.**
These are not internal messages; they are the contract agents and users read. Each was previously
unowned. All five anchors **[V]** in the upstream worktree:

| Anchor | Text | Change |
|---|---|---|
| `src/cli/mod.rs:962` | `br sync --flush-only    Export database to .beads/issues.jsonl` — the `sync` long-help EXAMPLES block | say `.beads/issues.org` (Org is the default after P3-09); mention that `.jsonl` is available by pinning |
| `src/cli/mod.rs:2865` | `/// Export database to JSONL (DB → .beads/issues.jsonl)` — the `--flush-only` doc comment, which clap renders into `--help` | "Export database to the workspace export file (DB → `.beads/issues.org`)" |
| `src/cli/mod.rs:2915` | `--witness` doc comment: "Reads the resolved issues.jsonl bytes …" | keep the JSONL wording — after this commit `--witness` **only** accepts JSONL — but state the restriction explicitly so the help matches the new rejection |
| `src/cli/commands/robot_docs.rs:17` | the `GUIDE` const: "exports .beads/issues.jsonl for git-friendly handoff" — the **agent guide**, and `robot-docs guide` is executed by G-rename part 4 | `.beads/issues.org` |
| `src/cli/commands/capabilities.rs:703` | published example `"br dep import .beads/issues.jsonl --robot"` in the `dep import` `CommandContract` | `.obr/issues.org`… but see note |

> **Note on the two-phase edit.** `capabilities.rs`'s `ENV_VARS` block (`:213-237`) is assigned to
> **P6-01a** as map row E18, and rename row M9 / section C rewrite `br` → `obr` in P6-02. This
> commit changes only the **filename** half (`issues.jsonl` → `issues.org`); the `.beads` and `br`
> halves follow in Phase 6. Do not try to do both here — you would be writing `.obr` paths into a
> binary that still creates `.beads`.

**Done-criteria.**
- `obr vcs-status` returns a normal report in an Org workspace.
- `obr sync --witness` and `obr sync --reconcile` on an Org workspace each produce a one-line,
  actionable error naming the flag and the reason — and exit non-zero.
- `obr sync --witness` on a JSONL-pinned workspace is unchanged.
- No "JSONL" appears in a message that an Org user can trigger without asking for JSONL.
- `obr sync --help`, `obr robot-docs guide` and `obr capabilities --format json` contain **zero**
  occurrences of `issues.jsonl` (`obr … | grep -c 'issues\.jsonl'` → 0 for each).

**Gate.** all.

---

### P3-11 · `feat(sync): extension-aware history backups`

**Scope.** `.br_history/` snapshots of the export file. Upstream's version gained collision indices,
nanosecond timestamps and per-backup `.meta.json` sidecars since the fork.

**Files.** `src/sync/history.rs`, **`src/cli/commands/history.rs`**.

**Source of truth.** fork `afe1351e`; [R2 §3.12].

#### `src/sync/history.rs` — the backup naming layer

| Anchor | Change |
|---|---|
| `:171-184` `parse_backup_filename` | try `.org` then `.jsonl`; return the extension alongside `(stem, timestamp)` |
| `:192-223` `create_backup_file_for_timestamp` | take the target extension: `{stem}.{timestamp}.{ext}` and `{stem}.{timestamp}.{idx}.{ext}` |
| `:227` `backup_metadata_path` | `with_extension("jsonl.meta.json")` → `with_extension(format!("{ext}.meta.json"))`. **Easy to miss; silently orphans every sidecar.** |
| `:384` `legacy_backup_target_path` | use the parsed extension instead of `format!("{stem}.jsonl")` |
| `:644-648` listing filter | accept `org` as well as `jsonl` |
| `:166-169` `BACKUP_FILENAME_REGEX` | unchanged (operates on the extension-stripped name) |

#### `src/cli/commands/history.rs` — the `history restore` default target *(previously unassigned)*

| Anchor | Current | Change |
|---|---|---|
| **`:878-885`** `is_default_jsonl_target` **[V]** | `let default_target = beads_dir.join("issues.jsonl");` at **`:879`** | `beads_dir.join(DEFAULT_JSONL_FILENAME)` — and, because P3-09 makes `issues.jsonl` a *legacy* name rather than an illegal one, also accept the legacy default so a restore into a JSONL-pinned workspace is still recognised as "the default target" |
| `:897` | `if !is_default_jsonl_target(beads_dir, target_path)` — the sole caller, which gates the extra confirmation/safety path around restoring over the live export | no change once the helper is correct |

**Why this matters and why it is not cosmetic.** `history restore` decides whether the restore
target *is* the workspace's live export file. Left as a literal, an Org workspace's `issues.org` is
never recognised as the default target, so the restore takes the "arbitrary external path" branch —
different validation, different confirmation, and a real chance of writing the wrong file. This is
the same class of defect as the `with_extension` trap in P3-03: it would work, be wrong, and no
existing test would catch it. `src/cli/commands/history.rs` has **18** `issues.jsonl` hits total
**[V]**; audit them all in this commit, not just `:879`.

**Done-criteria.** In an Org workspace: a flush creates `issues.<ts>.org` **and**
`issues.<ts>.org.meta.json`; `obr history list` shows it; `obr history restore` round-trips **and
takes the default-target branch** (assert the confirmation/validation path the JSONL default takes).
JSONL behaviour unchanged, including in a JSONL-pinned workspace after the P3-09 flip.
`grep -c 'issues\.jsonl' src/cli/commands/history.rs src/sync/history.rs` → only legacy-fallback
sites remain, each commented as such.

**Gate.** all + `cargo test --test e2e_history` (establish its baseline on first run and commit it
into `docs/research/upgrade/baseline/`).

---

### P3-12 · `fix(doctor): Org awareness across export-file checks`

**Scope.** ~12 sites in a 24,752-line file. The plan's §6.5 *understates* doctor and *overstates*
stats (§10.C4).

**Files.** `src/cli/commands/doctor.rs` (142 `issues.jsonl` hits **[V]**),
**`src/cli/commands/doctor_subsystems/surface.rs`** (4 hits **[V]**).

**Source of truth.** fork `a0c54428` (intent only — the code shape is superseded); [R2 §3.13].

| Anchor | Function | Change |
|---|---|---|
| `:10439-10449` | doctor's **own copy** of `discover_jsonl` | make 3-tier — or better, delete it and call `config::discover_jsonl` |
| `:10451-10457` | `should_fallback_to_workspace_jsonl` | compares against the literal `"issues.jsonl"`; must accept the Org default **and** the legacy JSONL default |
| `:10580-10626` | `check_jsonl` | free once `validate_jsonl_issue_records` is format-aware (P3-05); update `"Parsed {n} records"` → issues for Org, and add `"format": "org"` to the JSON details as `a0c54428` did |
| `:4012-4059` | `check_export_hash_cache_divergence` | free once P3-04 lands |
| `:5201` | `check_jsonl_duplicate_ids` | **needs an Org branch** — this is the check that catches B9 |
| `:7959`, `:10202` | base-anchor checks | hardcode `issues.jsonl`; use the resolved path |
| `:2904`, `:3234`, `:3250` | `doctor.jsonl_rebuild` fixer | rebuilds the DB from the export file — must go through the format-aware reader |
| `:3202` | error text `"Fix the offending records in .beads/issues.jsonl"` | use the resolved path |
| `:4672`, `:4885`, `:4939`, `:5143`, `:5371` | BOM / CRLF / trailing-newline / oversized / world-writable | **no change** — byte-level and correct for Org. CRLF matters *more* for Org, since `sanitize_org_text` silently normalizes it. |

**`doctor_subsystems/` — the concrete anchor, no longer "possibly".** The whole `doctor_subsystems/`
tree has exactly **one** file with `issues.jsonl` hits, and it is load-bearing **[V]**:

| Anchor | Current | Change |
|---|---|---|
| **`src/cli/commands/doctor_subsystems/surface.rs:413`** | `let jsonl_present = beads.join("issues.jsonl").is_file() \|\| beads.join("beads.jsonl").is_file();` | make it three-tier: `issues.org` → `issues.jsonl` → `beads.jsonl`, i.e. exactly `config::discover_jsonl`'s chain (P3-09). Better: **call `config::discover_jsonl`** rather than adding a third hand-rolled copy. |
| `surface.rs:411` (adjacent) | `let db = beads.join("beads.db");` | leave for **P6-01a** map row A3 — it is a *branding* fix, not a format fix. Note it in the commit message so P6-01a does not miss it. |

**Why `surface.rs:413` cannot wait.** `jsonl_present` feeds the doctor's top-level workspace-surface
classification. An Org workspace with a perfectly healthy `issues.org` reports `jsonl_present:
false` — i.e. "this workspace has no export file" — which changes which checks run and which
remediations doctor offers. It is a silent mis-diagnosis, not a hard failure, so no gate catches it.

**Also check (open question Q4, cheap):** `rg -l 'issues\.jsonl|\{"id"' tests/doctor_fixtures/` —
enumerate whether the fixture shell scripts embed the filename or raw JSONL. (Disk reality **[V]**:
`tests/doctor_fixtures/` holds **206 files** in 68 top-level entries, **171** of which contain a
`beads`/`br ` hit.) Whatever they do, they are Class A (pin to JSONL) under the P3-13 rule.

**Done-criteria.** `obr doctor` in an Org workspace reports the export-file checks as OK with the
right issue count and `"format": "org"`; no check hard-fails merely because the file is not JSONL;
`obr doctor --quick --json` exits 0 on a healthy Org workspace; **`obr doctor --json` on an Org
workspace reports the export file as present** (the `surface.rs:413` regression test);
`rg -c 'issues\.jsonl' src/cli/commands/doctor_subsystems/` → only legacy-fallback sites.

**Gate.** all + `cargo test --test e2e_doctor_fixture_suite` (baseline it first and commit the
inventory into `docs/research/upgrade/baseline/`).

---

### P3-13 · `test: Org e2e coverage, harness helpers, and the JSONL pinning rule`

**Scope.** The test estate. **59 of upstream's 165 `tests/*.rs` files mention `issues.jsonl` [V];
the fork migrated 5.** This needs a *rule*, not per-file judgement (§9 U-TESTPOLICY).

**Files.** `tests/common/cli.rs`, new `tests/e2e_org_roundtrip.rs`, and the Class-A/B/C pins across
the estate.

**Source of truth.** [R2 §6] in full; fork `c046b35e` as an **anti-pattern**, not a model.

**Two harness helpers first** (`tests/common/cli.rs`, next to `BrWorkspace` at `:95`):
```rust
/// The workspace's default export artifact (Org, post-P3-09).
pub fn export_path(ws: &BrWorkspace) -> PathBuf;
/// Pin this workspace to JSONL. Call immediately after init, before the first mutating command.
pub fn pin_jsonl(ws: &BrWorkspace) -> PathBuf;   // rewrites metadata.json jsonl_export
```
`pin_jsonl` works because `resolve_jsonl_path` priority 2 treats a non-default value as an explicit
override. **No new CLI flag is needed.**

**The rule — apply per test FUNCTION, not per file:**

> **Default is Org. JSONL requires an explicit, one-line pin.**
>
> **Class A — pin to JSONL** (add `pin_jsonl(&ws)` after setup; leave every assertion untouched) if
> any of: the test writes raw JSON into the export path or reads it and calls
> `serde_json::from_str`/`from_value`/`.lines()`; the test asserts a *line* property; the test
> exercises JSONL-only machinery (reviewed-additive reconcile, incremental line auto-flush, Merkle
> witness reuse, `beads.base.jsonl` anchors); the test is a **conformance** test (out of scope per
> plan §7.3 — needs the Go `bd` binary); the test is a benchmark/dataset test whose numbers must
> stay comparable to upstream's.
>
> **Class B — convert to Org** (swap the literal path for `export_path(&ws)`; translate content
> assertions) otherwise. Class B is everything treating the export as *the durable artifact*:
> existence, size, mtime/hash staleness, backup/restore/prune, conflict-marker rejection,
> path-traversal and git-safety allowlists, doctor counts, determinism, field round-trip.
>
> **Class C — duplicate** when one test asserts both a format-agnostic behaviour and a JSON-shaped
> payload: split into `..._org()` and `..._jsonl()`. Prefer this over weakening an assertion.

**Mechanical triage, per file:**
```bash
rg -n 'issues\.jsonl' tests/<file>.rs
rg -n 'from_str|from_value|\.lines\(\)|line \{|"line"' tests/<file>.rs
```
Second grep hits inside a block that reads the export file → Class A or C. Otherwise → Class B.

**The anti-pattern to refuse.** The fork's `c046b35e` wrote
`if org_path.exists() { …weak assertion… } else { …original JSONL assertion… }` — e.g. in
`tests/e2e_labels.rs` the Org branch degenerates to `org_content.contains(":persisted:")`, which
cannot distinguish a label from a title. **Never branch on file existence in a test.** Pin the
format, then assert unconditionally. [R2 §6.2]

**Pre-classified inventory** (verify each; starting point from [R2 §6.3]):

| Files | Class |
|---|---|
| `conformance*.rs` (6), `jsonl_import_export.rs`, `repro_auto_flush_inefficiency.rs`, `e2e_sync_reconcile.rs`, `e2e_sync_preflight_integration.rs`, `bench_*.rs`, `benchmark_*.rs`, `tests/common/dataset_registry.rs` | **A** |
| `storage_export_atomic.rs`, `e2e_vcs_status.rs` | **C** |
| `e2e_basic_lifecycle.rs`, `e2e_comments.rs`, `e2e_labels.rs`, `e2e_relations.rs`, `e2e_history*.rs`, `storage_history.rs`, `repro_history_collision.rs`, `e2e_sync_artifacts.rs`, `e2e_sync_fuzz_edge_cases.rs`, `e2e_sync_failure_injection.rs`, `e2e_sync_git_safety.rs`, `proptest_sync_path.rs`, `repro_create_path_traversal_check.rs`, `e2e_errors.rs`, `e2e_global_flags.rs`, `e2e_env_overrides.rs`, `e2e_workspace_commands.rs`, `e2e_routing.rs`, `e2e_orphans.rs`, `e2e_ready.rs`, `e2e_stats.rs`, `golden_beads_init.rs`, `snapshots.rs`, `e2e_sync_status_health.rs`, `e2e_sync_needs_flush_guard.rs` | **B** |

**New Org-only tests this commit must add** [R2 §6.4]:
1. `tests/e2e_org_roundtrip.rs` — the G-org-roundtrip gate, as a real test.
2. A proptest in `org_bridge.rs`: `issues → text → issues == issues` over colon labels, unicode,
   `,*` bodies, empty strings, every status **including `Draft`**, every priority, deps, comments,
   `agent_context`. This is the ~10-line test dossier R9 says would have caught R9.
3. `Status::Custom` export refusal.
4. Duplicate-`:ID:` rejection parity with JSONL.
5. Content-hash agreement export-side vs file-side for Org with blank lines and indented JSON.
6. `--no-db` create-one-issue-changes-one-block (the direct regression test for phantom changes).
7. The field-coverage destructuring test (moved here if not already in P3-02).

**Done-criteria.** G-e2e-subset failure sets unchanged from baseline; G-org-roundtrip is a real
green test; `golden_*`/insta snapshots regenerated and **reviewed by hand** (a regenerated golden
that nobody read is not a test).

**Gate.** all + G-org-roundtrip.

---

## 3. Phase 4 — productization re-application

Derived from [R3 §8]. Only **P4-05** depends on Phase 3 (it needs `org_bridge`); the rest could in
principle run earlier, but running them after Phase 3 keeps the Org series free of infra churn.

### P4-01 · `chore: remove stray and foreign artifacts from the upstream tree`

**Scope.** ~450 files, ~178 MB, ~4,400 branding hits removed before any renaming happens.

**Files.** The paths below, all tracked **[V]**. Nothing else.

```
git rm -r temp_test temp_test_2 custom.db \
          sample_beads_db_files refactor .ntm .rchignore .ubsignore .agent-mail.yaml \
          .claude .claude-plugin skills
```

**`.ubsignore` is new to this list.** It is tracked **[V]**, it is the exact sibling of `.rchignore`
(a foreign tool's ignore file), and it contains `legacy_beads/` and `.beads/` — so if it is *kept*
it becomes a P6-01a rename target (map row A19) instead. Deleting `.rchignore` while keeping
`.ubsignore` is the one option that makes no sense. **All twelve paths are now enumerated in §9
D-DELETIONS**, which previously folded `.agent-mail.yaml`, `.rchignore` and `custom.db` into this
command line without listing them as decisions (governing plan §8.4 asks about exactly these).
**Rationale, each [R3 §5.1]:** `sample_beads_db_files/` is 178 MB of third-party issue data
referenced by **nothing** (`Cargo.toml:14` publish-exclude and `.gitignore:148-149` only);
`refactor/` is 63 dated session artefacts from upstream's own campaigns; `.ntm/` is three log
droppings; `.rchignore` serves upstream's `rch` tool; `.agent-mail.yaml` makes an `obr` checkout
announce itself as upstream's project (and is already in `.gitignore:125` yet tracked);
`.claude-plugin/` declares `"name": "beads"` on an official marketplace the fork does not run;
`.claude/skills/br/` is that plugin's payload; `skills/bd-to-br-migration/` documents a Go-`bd` →
Rust-`br` migration meaningless for obr. `temp_test*`/`custom.db` are tracked-but-ignored accidents
in **both** trees (plan §5 Phase 0.5 is correct about this).

**Read before deleting:** `skills/bd-to-br-migration/scripts/find-bd-refs.sh` is a decent template
for the G-rename audit script.

**Subject to §9 D-PLUGIN** (`.claude/`, `.claude-plugin/`) and **D-DELETIONS**.

**Done.** Tree shrinks by ~178 MB; `cargo build` and `cargo test --lib --bins` unaffected;
`git grep -l sample_beads_db_files` returns only files this commit also edits.
**Gate.** G-build, G-unit.

### P4-02 · `build(nix): replace crane+fenix flake with rust-overlay`

**Scope.** The fork's flake is the base; **upstream's contributes nothing** — it is byte-identical
to the fork point except `version = "0.1.9"` → `"0.1.20"`, and there is no `flake.lock` [R3 §2.1].

**Source of truth.** fork `f826ac4e` + `33bb2e82`.

**Do:** start from `fork:flake.nix` (rust-overlay, `makeRustPlatform`, `apple-sdk_15`,
`stdenv.hostPlatform.isDarwin`, mkShell devShell); **delete** `cargoLock.outputHashes."toon_rust-0.1.1"`
(toon is now crates.io `tru`); **delete** `nativeBuildInputs = [ rustPlatform.bindgenHook ]` and
`buildInputs = [ sqlite ]` (those existed for rusqlite's bundled C SQLite; `fsqlite` is pure Rust —
**verify with one `nix build`**, R3 open question 6); **delete** the `org2jsonl` flake input and the
`postUnpack: cp -r ${inputs.org2jsonl} org2jsonl` hack — P3-01's git dep makes them unnecessary;
bump `version`; **commit a `flake.lock`**; set `meta.license` per §9 D-LICENSE (the fork's
`licenses.bsd3` is not a free choice on the new base).

If `nix build` needs an `outputHashes` entry for the git dep, add exactly one:
`"org2jsonl-0.1.0" = "sha256-<from the first build failure>";`

**Files.** `flake.nix`, new `flake.lock`, `Cargo.toml` (only if an `outputHashes` entry forces one).

**Done-criteria.**
- `nix build` succeeds and produces a binary that runs `--version`.
- `nix develop --command cargo build` succeeds — **and this is a real deliverable, not a formality.**
  Per §0.1.0, *neither* tree's dev shell can build the tree today: the fork's pins nightly 1.95.0
  (too old for `sysinfo 0.39.6`'s `cfg_select`) and upstream's is broken on Darwin twice
  (`flake.nix:110-112` references removed `darwin.apple_sdk.frameworks.*` stubs; crane's
  `vendorCargoDeps` cannot find `Cargo.lock` because `combinedSrc` nests under `beads_rust/`).
  **The new dev shell must pin a toolchain at least as new as the canonical fenix pin** and must
  build the tree, or the port permanently depends on an ad-hoc `nix shell` line.
- `flake.lock` is committed and `nix flake metadata` resolves every input.
- G-locked still passes (the flake work must not perturb `Cargo.lock`).
- `git diff --stat` touches only `flake.nix`, `flake.lock` (and `Cargo.toml` if `outputHashes`).

**Gate.** G-build, **G-locked** + `nix build` + `nix develop --command cargo build`.

### P4-03 · `build(nix): add flake checks and extend the dev shell`

**Scope.** The `checks` outputs and the dev-shell tool list. Nothing else.

**Files.** `flake.nix`, `flake.lock` (only if inputs change).

**Source of truth.** fork `1c32e5e5`. Separate commit so it can be reverted independently if
`nix flake check` (which runs `cargo test --lib --bins` in a sandbox) proves too slow on a
34,865-line `sqlite.rs` [R3 §2.1].

**Do.** Note that the fork's flake check ran a *green* test suite. **Upstream's is red at baseline
(266 unit failures, §0.1.1).** A `nix flake check` that runs `cargo test --lib --bins` will
therefore fail unconditionally. Either (a) restrict the check to `cargo build` + `cargo fmt --check`
+ `cargo-clippy`, or (b) make the test check apply the same no-NEW-failures comparison G-unit uses,
which is awkward inside a sandbox. **(a) is recommended.** Do not ship a check you know is red.

**Done-criteria.**
- `nix flake check` **exits 0**, or the commit is not written.
- Its wall-clock runtime is measured and recorded in the commit message (input to the
  P4-04 decision about pre-commit vs pre-push).
- The dev shell provides, at minimum: the pinned toolchain, `cargo-fuzz`, `lefthook`, `shfmt`,
  `jq` (the §0.3 bring-up helper uses it).

**Gate.** `nix flake check` + G-build.

### P4-04 · `chore: add lefthook pre-commit hooks`

**Scope.** One new `lefthook.yml`, one `.gitattributes` line, and the D-HOOKS disposition of
upstream's `.githooks/`.

**Files.** new `lefthook.yml`; `.gitattributes`; `.gitignore` (**minus** one line — see trap 1);
`.githooks/` (deleted or kept per D-HOOKS).

**Source of truth.** fork `4cbf6348`. `grep -rn lefthook` over the whole upstream tree returns
**nothing** [R3 §2.3].

**Two traps.**
1. **Do NOT re-apply the `.gitignore` hunk's `fuzz/corpus/` line** — upstream **tracks** 2,747
   corpus files there. Apply the commit minus that one line. (Upstream already has
   `fuzz/artifacts/` at `.gitignore:62` and `fuzz/Cargo.lock` at `:21`.)
2. **Pick one hook mechanism.** Upstream added `.githooks/pre-commit` (opt-in via
   `core.hooksPath`), which runs `br doctor --quick --json` against `.beads/` with a
   `BR_DOCTOR_SKIP_PRECOMMIT` bypass. **§9 D-HOOKS**: fold its intent into `lefthook.yml` as a
   seventh command and `git rm -r .githooks`, or keep `.githooks/` renamed and drop the lefthook
   doctor command. Shipping both invites a `core.hooksPath` / lefthook-install conflict.
3. Consider moving `nix-check` from pre-commit to pre-push (see P4-03).
4. **Every lint/format command in `lefthook.yml` must use the `cargo-clippy` / `cargo-fmt` binaries,
   not `cargo clippy` / `cargo fmt`** (§0.1.0 fact 2) — otherwise the hook silently runs a stale
   rustup nightly and its verdict is meaningless. The fork's copy of this file predates that
   discovery; fix it while porting.

**Also adopt:** `.gitattributes` (`/fuzz/corpus/** binary`) — needed if the corpora stay.

**Done-criteria.**
- `lefthook install` succeeds and `lefthook run pre-commit` on a clean tree **exits 0** (a hook that
  is red on an unmodified tree gets disabled by the first person who hits it).
- `git diff HEAD~1 -- .gitignore` does **not** contain `fuzz/corpus/` (trap 1), and
  `git ls-files fuzz/corpus | wc -l` is still 2,747.
- Exactly one pre-commit mechanism is active: either `.githooks/` is gone, or `lefthook.yml` has no
  doctor command (D-HOOKS). `git config core.hooksPath` and a `lefthook install` cannot both be
  meaningful — assert whichever one you chose and delete the other.
- G-build unaffected.

**Gate.** G-build.

### P4-05 · `test(fuzz): add org_parse and validation fuzz targets`

**Depends on P3-02.** Upstream has 7 targets (`config_yaml`, `content_hash`, `jsonl_import`,
`markdown_import`, `merge_issue`, `search_issues`, `sync_cycle`) plus corpora and a shared
`common.rs` [R3 §2.2].

**Files.** `fuzz/Cargo.toml` (append two `[[bin]]` stanzas), new
`fuzz/fuzz_targets/{org_parse,validation}.rs`, new `fuzz/corpus/org_parse/**`, `fuzz/README.md`.
**Not** `fuzz/Cargo.lock` — it is untracked (`.gitignore:21` **[V]**).

**Source of truth.** fork `5312cb5a`; upstream's `fuzz/` layout [R3 §2.2].

**Verified manifest facts [V].** `fuzz/Cargo.toml` declares `name = "beads_rust-fuzz"` and depends
on `beads_rust = { path = "..", default-features = false }`; all 7 existing targets `use
beads_rust::`. This is why **P6-02 must carry G-aux** — the fuzz crate is outside the workspace and
nothing else compiles it.

- **DROP** the fork's `fuzz_jsonl_parse` — subsumed by upstream's `jsonl_import`, which drives the
  real `import_from_jsonl` against a temp workspace with sentinel invariants.
- **KEEP** `fuzz_validation` → rename `validation`. Genuine upstream gap; all five APIs it calls are
  still public (`src/validation/mod.rs:451`, `:375`, `:383`; `src/model/mod.rs:137`, `:172`, `:259`),
  so it should compile unmodified.
- **KEEP** `fuzz_org_parse` → rename `org_parse`. Mandatory: the fork's only fuzz coverage of its own
  new input boundary. Seed `fuzz/corpus/org_parse/` from the P3-13 round-trip test's outputs.

Take **upstream's** `fuzz/Cargo.toml` and append two `[[bin]]` stanzas. Note upstream's `fuzz/` has
no `[workspace]` table while the fork's does — verify `cargo fuzz build` works with upstream's
arrangement **before** adding the stanzas (§9.4 Q-FUZZWS, was R3 open question 5). Add both targets
to `fuzz/README.md`.

**Done-criteria.**
- `cargo fuzz list` shows **9** targets.
- `cargo fuzz build` exits 0 for all 9 (this is G-aux).
- `cargo fuzz run org_parse -- -runs=10000` completes without a crash against the seeded corpus.
- `fuzz/corpus/org_parse/` is seeded from the P3-13 round-trip test's G1/G2 outputs plus at least:
  an empty file, a heading with no `:ID:`, two headings sharing one `:ID:`, a `,*` body, a title
  ending `:foo:`, 100 KB of `[[`, and a `#+begin_src` block containing `:PROPERTIES:`.
- `git diff --stat` shows no change under `fuzz/corpus/{config_yaml,jsonl_import,…}` — you are
  adding a corpus, not perturbing upstream's 2,747 files.
- **G-locked**: appending `[[bin]]` stanzas must not move the root `Cargo.lock`.

**Gate.** **G-locked + G-aux** (`cargo build --benches` + `cargo fuzz build`).

### P4-06 · `ci: tighten clippy and benchmark gates`

**Files.** `.github/workflows/ci.yml`; plus the supply-chain adoptions below.

**Source of truth.** the gate half of fork `6a0518d0`. Upstream never added `-- --deny warnings` to
either clippy step (`.github/workflows/ci.yml:44`, `:47` **[V]**: `cargo clippy --all-targets
--all-features` and `--no-default-features`, both bare) and still uses the 10 % bench threshold
(`:356`, `:364`, `:396`) [R3 §2.6] **[V]**.

**§0.1 changes the shape of this commit — but not the way an earlier draft said.** The earlier claim
was that `--all-targets` aborts inside `sysinfo` and so a `--lib --bins --tests` substitute was
needed. That was a stale-toolchain artifact. The real situation:

- `--all-targets` works. It is **red with 85 first-party errors** under clippy 0.1.99 (§0.1.1).
- So `-- --deny warnings` **cannot** be added as a hard gate today. Adding it turns CI permanently
  red on upstream's own pre-existing lint debt.
- **Write CI to match G-lint**: run `cargo-clippy --all-targets -- -D warnings`, capture the
  lint-kind histogram, and **fail only when a count exceeds the committed baseline**
  (`docs/research/upgrade/baseline/v3-clippy-lints-baseline.txt`). That is a real, tightening gate
  that is green on day one and cannot silently absorb new debt.
- Alternatively, land the 85 fixes first as their own commit and *then* make `-D warnings` hard.
  That is strictly better but it is a separate, non-trivial piece of work on the hottest files in
  the tree; **§9.4 Q-LINTDEBT** records the choice.
- **Use `cargo-clippy` / `cargo-fmt`, not `cargo clippy` / `cargo fmt`, in the workflow** — the
  shim hazard (§0.1.0) applies to any runner that has rustup installed, which GitHub's `ubuntu-*`
  and `macos-*` images do.

**Also revisit here, because this is the CI commit (§7.1 raises it and nothing else owns it):**
`perform_conditional_namespace_change` (`src/sync/mod.rs:1464`) is the `renameat2`/`RENAME_EXCHANGE`
CAS primitive, and it is **cfg-gated to Linux / Android / Apple with a hard error elsewhere**
(`:1521`). Every job that runs the sync tests must therefore be on `ubuntu-*` or `macos-*`. Audit
the workflow matrices for `windows-*` runners in any job that exercises sync/export, and either
drop them or mark those tests `#[cfg]`-skipped with an explicit reason. A Windows job that reaches
`:1521` fails with an error that reads like a port bug and is not one.

**Also adopt** upstream's CI supply-chain machinery the fork lacks (`.github/action-pins.jsonl`,
`.github/action-pin-upstreams.jsonl`, `scripts/verify-workflow-action-pins.sh`,
`scripts/audit-workflow-action-pins.sh`, `docs/CI_SUPPLY_CHAIN.md`) — real productization, all
actions sha-pinned, and a standing maintenance commitment (§9 D-SUPPLYCHAIN).

**`.cargo/audit.toml` (§9 D-AUDIT).** The file upstream ships is byte-identical to the one fork
`fcbbce29` deleted, and the RUSTSEC-2025-0140 suppression is still live (`vergen-gix = "10.0"` at
`Cargo.toml:131`, 606 `gix` lines in `Cargo.lock`). The commit message's premise is only half true:
CI's advisory step is `continue-on-error: true` (`audit.yml:39-42`), so the ignore file cannot
affect CI redness either way, and `cargo audit --deny yanked` is unaffected by `[advisories].ignore`.
**R3 recommends keeping the file and making the advisory step blocking** — strictly better than
deleting the suppression while leaving the step non-blocking.

**Done-criteria.**
- The workflow is **green on the branch as it stands** (push the branch and read the run — a CI
  commit whose CI was never run is not a deliverable).
- The clippy step fails a deliberately-introduced new pedantic warning (test it once, then revert).
- The bench threshold is the fork's tightened value, and the bench job still runs
  `cargo build --benches` successfully (see G-aux).
- No job that exercises sync/export runs on a `windows-*` runner (the `renameat2` constraint above).
- `scripts/verify-workflow-action-pins.sh` passes if D-SUPPLYCHAIN adopts it.
- Every `cargo clippy`/`cargo fmt` in `.github/` is spelled `cargo-clippy`/`cargo-fmt`
  (`git grep -n 'cargo clippy\|cargo fmt' .github/` → empty).

**Gate.** G-lint.

### P4-07 · `style: format shell scripts with shfmt` *(optional)*

**Files.** *(only if the pass happens)* `install.sh`, `scripts/*.sh` (~14 files).
**Explicitly not:** `tests/doctor_fixtures/**`.

**Source of truth.** fork `5eb13fb7` (as intent only — all 17 of its files are stale).

Upstream tracks **164** `.sh` files; **160** are non-conforming under shfmt defaults, 155 under
`-i 4` [R3 §2.4]. Upstream's house style is 4-space; the fork chose tabs. Every one of the fork's 17
reformatted files is stale (`install.sh` grew 40,327 → 61,370 bytes; `scripts/` gained 9 files).

**Recommended: do not bulk-reformat.** Keep the `shfmt -d {staged_files}` lefthook command and let
the repo converge. If a one-shot pass is wanted, scope it to `install.sh` + `scripts/*.sh` (~14
files) and **exclude `tests/doctor_fixtures/**`** (the `assert.sh`/`corrupt.sh` scripts consumed by
the doctor harness — pure risk, zero benefit). §9 D-SHFMT decides tabs vs `-i 4`.

**Done-criteria.**
- `git diff --stat` touches **zero** files under `tests/`.
- `shfmt -d <the chosen flags> install.sh scripts/*.sh` is silent.
- `bash -n` parses every touched file.
- `install.sh --help` (or its dry-run path) behaves identically before and after — a formatter that
  breaks a heredoc in a 61 KB installer is a real risk.
- G-build and `cargo test --test e2e_doctor_fixture_suite` unmoved.

**Gate.** G-build (and the doctor fixture suite if fixtures are touched — do not touch them).

### P4-08 · license disposition — **BLOCKED on §9 D-LICENSE. Do not write this commit.**

**Do not let Phase 4 apply fork `b3415739`.** Its literal action (delete `LICENSE`, ship only BSD-3)
is not available on a base that contains upstream's code under MIT + the OpenAI/Anthropic Rider.
Whichever way it lands, four packaging sites are wrong **today in both trees** and need fixing:
`packaging/homebrew/*.rb:16`, `packaging/scoop/br.json:6`, `packaging/aur/PKGBUILD:13` (+ the full
MIT text embedded inline at `:28-44`), and `packaging/aur/PKGBUILD-git:42`, which installs a
`LICENSE` file the fork deleted — so the `-git` AUR package silently installs no license. [R3 §3]

**Explicitly not in Phase 4:** `aa129a2c` (nothing to do — see §8 rows and §10.C6), the README
(`a2ba738e` → Phase 8), the rename half of `6a0518d0` (→ P6-04), `agent_baseline` regeneration
(→ P6-05).

---

## 4. Phase 5 — the fork's own tracker data

**Loss of this data is failure** (plan §7.4). 549 issues, prefix `beads_rust-`.

> **Read this before P5-01.** The branch already carries **upstream's** `.beads/` directory —
> **ten tracked files [V]**: five branded `.md` specs, `.gitignore`, `config.yaml`,
> `metadata.json`, `interactions.jsonl`, `issues.jsonl`. Phase 5 is not "add a tracker to an empty
> directory"; it is "replace the *contents* of an existing, tracked, branded directory". Two
> consequences that an earlier draft got wrong:
>
> 1. **`metadata.json` is byte-identical in both trees** — `{"database": "beads.db",
>    "jsonl_export": "issues.jsonl"}` **[V]** in `main` *and* in `upstream/main`. There is nothing
>    to copy, and copying it is actively harmful (see the trap in P5-02).
> 2. **`beads.db` is not tracked** (`.beads/.gitignore` line `*.db` **[V]**), so there is no fork
>    database to carry and nothing to migrate. The 1 → 17 schema ladder is real but it is not this
>    commit's business.

### P5-01 · `chore(tracker): import the fork's own issue history`

**Scope.** Put the fork's 549 issues on the branch as JSONL. Format conversion is P5-02; the
directory rename is P6-01a. Do all three in one commit and you will not be able to tell which one
broke.

**Files.** `.beads/issues.jsonl` (replaced wholesale from `main`). **Nothing else.**

**Source of truth.** `git show main:.beads/issues.jsonl`; plan §7.4; [R4 §6.4]; §10.C25.

**Do.**
```bash
git show main:.beads/issues.jsonl > .beads/issues.jsonl
git add .beads/issues.jsonl
```

**Do NOT copy, and why — each verified [V]:**

| File | Why not |
|---|---|
| `main`'s `.beads/metadata.json` | **byte-identical to upstream's.** After P3-09 its `"jsonl_export": "issues.jsonl"` is an **explicit override** (`resolve_jsonl_path` priority 2, `src/config/mod.rs:4219-4262`) — the exact mechanism P3-09 calls "the trap" and P3-13 uses as `pin_jsonl`. Leave it alone here: for *this* commit, pinning to JSONL is precisely what you want. P5-02 flips it deliberately. |
| `main`'s `.beads/config.yaml` | 41 lines with ~20 branded comment lines and `issue_prefix: "beads_rust"`. Upstream's is **4 lines** with the same two prefix keys. Keeping upstream's is strictly less to de-brand in P6-01a. Both name the same prefix, so nothing is lost. |
| the five tracked `.beads/.br_history/issues.*.jsonl` snapshots | they duplicate git history of `issues.jsonl` and would be the only tracked-file hits on the history path [R4 §6.4] |
| the `.beads/*.md` spec docs | they are **upstream's**, not the fork's (§10.C25) — they arrive from `upstream/main` for free, in their newer form, and P6-01a de-brands them |
| `main`'s `.beads/interactions.jsonl` | upstream's copy is 3 synthetic test records **[V]** (`actor: ubuntu`, `"test prompt"`). Neither copy is real data. **D-DELETIONS** decides; recommended: `git rm .beads/interactions.jsonl` in **P6-01a**. |

**Done-criteria.**
- `obr sync --import-only --rebuild` (or `--no-db list --json | jq length`) reports **549 issues,
  3 open**, matching `main`.
- The freshly created `.beads/beads.db` is at `CURRENT_SCHEMA_VERSION = 17`
  (`src/storage/schema.rs:11` **[V]**) — it is *created* at 17, not migrated to it, because no fork
  DB is carried. *(If you separately choose to open a copy of `main`'s untracked `beads.db` with
  the new binary, that one does migrate 1 → 17; verify it once out-of-band, but it is not part of
  this commit and must not be committed.)*
- `git status --porcelain` shows exactly one changed path.
- `git ls-files .beads | wc -l` is still **10**.
- The 549 issue IDs still carry the `beads_rust-` prefix (P5-03 / D-PREFIX changes that, not this).

**Gate.** G-build + the issue-count check above.

### P5-02 · `chore(tracker): convert the tracker to Org`

**Scope.** Flip the project's own tracker from JSONL to Org. This is the commit where the fork's
reason to exist starts applying to the fork itself.

**Files.** `.beads/metadata.json` (edited), `.beads/issues.org` (added), `.beads/issues.jsonl`
(removed).

**Source of truth.** P3-09's `resolve_jsonl_path` analysis; [R2 §7].

> ### The trap that silently defeats this entire commit
>
> `.beads/metadata.json` says `"jsonl_export": "issues.jsonl"` **[V]**. After P3-09 flipped
> `DEFAULT_JSONL_FILENAME` to `"issues.org"`, that value is no longer "the default" — it is an
> **explicit override** at `resolve_jsonl_path` priority 2 (`src/config/mod.rs:4219-4262`).
>
> So a bare `obr sync --flush-only` here writes **`issues.jsonl`**, not `issues.org`. And the
> obvious done-criterion — "re-import reproduces every field" — **passes anyway**, because the
> JSONL round-trip is fine. The project's own tracker would silently stay in the format the fork
> exists to replace, and every gate would be green.
>
> **You must edit `metadata.json` first.** This is not optional and it is not automatic.

**Do — in this order:**
```bash
# 1. Retire the explicit JSONL pin. Either drop the key entirely (falls through to the default,
#    priority 4) or set it to the new default. Setting it explicitly is clearer:
jq '.jsonl_export = "issues.org"' .beads/metadata.json > .beads/metadata.json.tmp \
  && mv .beads/metadata.json.tmp .beads/metadata.json

# 2. Confirm the resolution BEFORE flushing — this is the check that catches the trap:
obr where --json | jq -r '.jsonl // .jsonl_path'      # must end in issues.org

# 3. Flush.
obr sync --flush-only

# 4. Stage the swap.
git add .beads/metadata.json .beads/issues.org
git rm .beads/issues.jsonl
```

**Done-criteria — every one of these, and note that four of them exist specifically to defeat the
trap above:**
- **`.beads/issues.org` exists and is tracked; `.beads/issues.jsonl` is gone from the index**
  (`git ls-files .beads/issues.jsonl` → empty). *This is the criterion an earlier draft lacked.*
- **`.beads/metadata.json` contains `"jsonl_export": "issues.org"`** (or no `jsonl_export` key).
- **`obr where --json` resolves the export path to `issues.org`.**
- **`.beads/issues.org` opens as valid Org** and its level-1 heading count equals 549.
- Issue count survives: 549 total, 3 open, byte-for-byte the same id set as P5-01.
- `obr sync --import-only` into a scratch DB reproduces every issue field-for-field against the
  P5-01 DB (`list --json` diff is empty modulo `updated_at` if U2 forces it).
- G-org-roundtrip passes on the **real** corpus, not just the synthetic one — this is the first
  time the Org pipeline meets 549 real issues with real descriptions, and it is the most likely
  place for U2's description-fidelity decision to show up as a diff.
- A second `obr sync --flush-only` is byte-identical (determinism on the real corpus).
- `git ls-files .beads | wc -l` is **10** again (issues.jsonl out, issues.org in).

**Gate.** G-org-roundtrip on the tracker corpus + G-build + the count check.

### P5-03 · `chore(tracker): re-prefix issue IDs` — **conditional on §9 D-PREFIX**

**Scope.** `beads_rust-*` → `obr-*` across the tracker corpus, **and** the config key that mints
future IDs.

**Files.** `.beads/issues.org`, **`.beads/config.yaml`**.

**Source of truth.** `src/cli/commands/sync.rs:3252-3254`; `build_prefix_renames`
(`src/sync/mod.rs:12150`); [R4 §11 D3] (whose `:12586` anchor is wrong — the function is at
`:12150`).

Mechanism verified to exist: `obr sync --import-only --rebuild --rename-prefix`
(`src/cli/commands/sync.rs:3252-3254` → `ImportConfig{rename_on_import: true}` →
`build_prefix_renames` at `src/sync/mod.rs:12150`), and dependency refs are remapped through the
rename map [R4 §11 D3]. **Not verified:** whether IDs embedded in *prose* (title, description,
design, notes, acceptance_criteria, comment text) are rewritten. **Run a dry-run on a copy first.**
If prose refs are not rewritten, fall back to keeping historical IDs and allowlist row L9.

**The half the mechanism does not cover.** `.beads/config.yaml` carries **two** keys — `issue_prefix:
beads_rust` and `issue-prefix: beads_rust` **[V]** (upstream writes both spellings). `--rename-prefix`
rewrites existing IDs; it does **not** touch this file. Left alone, every issue created after the
port is minted as `beads_rust-<n>` again, and **G-rename part 3b fails on the config file** —
exactly what [R4 §5.8] warned would be "the very first thing the G-rename audit trips over".
*(The rename map's row A14 lists `config.yaml` as "KEEP — already unbranded". That is wrong about
its contents; A14 is right that the *filename* needs no change. §5.1 is corrected accordingly.)*

**Both keys must be rewritten**, in this commit if it runs, and in **P6-01a** if D-PREFIX chooses
option (ii) — because option (ii) keeps *historical* IDs, not the *minting* prefix.

**Done-criteria.**
- Issue count unchanged: 549 / 3 open.
- Dependency graph unchanged: the edge count and the set of `(from, to, type)` triples are
  identical modulo the prefix substitution. `obr dep list --json` before/after, diffed.
- **Zero** `beads_rust-` occurrences remain in `.obr`/`.beads` outside prose that the dry-run showed
  is not rewritten (and if any remain, allowlist row L9 applies and D-PREFIX has effectively
  chosen (ii) for those).
- `.beads/config.yaml` has **no** `beads_rust` in either prefix key.
- A fresh `obr create --title=x` mints an `obr-` id.
- G-build; G-org-roundtrip still passes on the re-prefixed corpus.

**Gate.** G-build + count check + dep-graph check + G-org-roundtrip.

---

## 5. Phase 6 — the de-beads rename

Apply the finalized R4 map below. Commit order is deliberate: deletions first (shrinks everything),
then surface-with-shims (the only part needing real review), then mechanical fixtures, then
identity, then internals, then docs, then goldens.

**§5.2 is the commit table; §5.2a gives each commit its source-of-truth and done-criteria** — read
both. In particular, **P6-01a owns three things the table cannot express**: `git mv .beads .obr`
plus the de-branding of all ten tracked files in it, the root `.gitignore`, and the creation of
`tests/legacy_compat.rs` with one test per LR row.

### 5.1 Finalized rename map (this replaces plan §7.6 in its entirety)

**Compat legend:** **LR** = legacy-read with a one-line deprecation warning, never write the old
name; **HARD** = no compat; **AUTO** = follows from another rename; **KEEP** = deliberately unchanged.

**There is no deprecation machinery upstream** — `git grep -in deprecat -- src/` returns zero hits
[R4 §10.10]. Every LR row is new code, and it **must** follow the JSON-safe stderr pattern of
`report_auto_flush_failure` (`src/cli/commands/mod.rs:81-132`): honour `ctx.is_quiet()`, emit a
`{"warning":{...}}` envelope on stderr under `--json`/`--toon`, sanitized text otherwise. Anything
else corrupts `--json`/`--robot` stdout.

#### A. On-disk artifacts

| # | Old | New | Compat | Anchor |
|---|---|---|---|---|
| A1 | `.beads/` | `.obr/` | **LR** — open `.beads` if `.obr` absent | `src/config/mod.rs:55-57` `is_beads_dir_name` **[V]** |
| A2 | `_beads/` | `_obr/` | **LR** (§9 D-MONOREPO: carry it at all?) | same |
| A3 | `beads.db` | `obr.db` | **LR** — only needed when `metadata.json` is absent/blank AND `obr.db` missing AND `beads.db` present (~8 lines in `resolve_db_path`) | `src/config/mod.rs:60` **[V]**, `init.rs:171-174` |
| A4 | `beads.jsonl` legacy export | *drop the name* | **LR only** — keep reading, never write | `src/config/mod.rs:64`, `:155` |
| A5 | `issues.jsonl` default | `issues.org` | chain `issues.org` → `issues.jsonl` → `beads.jsonl` (**done in P3-09**) | `src/config/mod.rs:62`, `:147-162` |
| A6 | `beads.{base,left,right}.jsonl` + `.meta.json` | `merge.{base,left,right}.jsonl` | **LR** — recognise both in `EXCLUDED_JSONL_FILES` **and** in doctor's merge-artifact detection | `src/config/mod.rs:71-79` **[V]**, `init.rs:229-234` |
| A7 | `.beads/.br_history/` | **`.obr/history/`** | **LR + migrate-on-first-write** | `src/sync/history.rs:502` |
| A8 | `.beads/.br_recovery/` | **`.obr/recovery/`** | **LR** | `src/config/mod.rs:66` **[V]** |
| A9 | `.beads/redirect` | `.obr/redirect` | AUTO from A1 | `src/config/routing.rs:190` |
| A10 | `bd.sock` (gitignore entry only) | `obr.sock` — or **delete the line** if vestigial (R4 open q. 5) | HARD | `init.rs:240` |
| A11 | `.bv.lock` | **KEEP** | KEEP — owned by the foreign `bv` viewer | `init.rs:246-247` |
| A12 | `~/.cache/beads/startup`, `$XDG_CACHE_HOME/beads/startup`, `$TMPDIR/beads-startup-cache` | `…/obr/startup`, `$TMPDIR/obr-startup-cache` | HARD (pure cache) | `src/config/mod.rs:4979,4985,4987` |
| A13 | `.beads.snapshot_*`, `.beads_snap*` detection | add `.obr.snapshot_*`, `.obr_snap*` | **LR** — keep detecting the old names as foreign debris | `doctor.rs:2584`, `:17523` |
| A14 | *filenames only:* `metadata.json`, `config.yaml`, `routes.jsonl`, `.write.lock`, `.sync.lock`, `last-touched`, `sync_base.jsonl`, `deletions.jsonl`, `interactions.jsonl`, `sync-state.json`, `daemon.{lock,log,pid}` | **KEEP the names** | KEEP | — |
| **A14b** | **the *contents* of the repo's own `.beads/config.yaml` and `.beads/metadata.json`** — `issue_prefix: beads_rust`, `issue-prefix: beads_rust`, `"database": "beads.db"` **[V]** | de-branded values | HARD | **This is a correction.** A14 said "already unbranded", which is true of the *filenames* and false of the *contents*. [R4 §5.8] flagged `issue_prefix: beads_rust` as "the very first thing the G-rename audit will trip over". Owned by **P5-03** (prefix keys) and **P6-01a** (`database`). |
| A15 | `# Beads Project Configuration` header written into `config.yaml` | `# obr project configuration` | HARD | `init.rs:186` |
| A16 | the `.beads/.gitignore` **template** (what `init` writes) **and the repo's own tracked `.obr/.gitignore`** | rewrite both identically: `.br_history/`→`history/`, `.br_recovery/`→`recovery/`, `beads.*.jsonl`→`merge.*.jsonl`, `bd.sock`→`obr.sock`, legacy `bd.db`→drop, keep `.bv.lock` (row A11) | HARD | `init.rs:199-248`; the tracked copy's current body is verified **[V]**: `bd.sock`, `beads.{base,left,right}.jsonl` + `.meta.json`, `.br_history/`, `.bv.lock`, `bd.db` |
| **A18** | **root `.gitignore` — 18 branded lines, owned by no commit until now [V]:** `:6` (`beads_rust is a binary CLI`), `:34` `/legacy_beads/`, `:39-40` `bv (beads viewer)`, `:42-46` (`.beads/last-touched`, `.beads/.bv.lock`, `.beads/*.lock`, `.beads/.br_history/`), `:131-142` (`.beads/*.corrupt*` … `.beads/recovery*/`), `:144`, `:148-149` `sample_beads_db_files/**`, `:167-168` `.beads.snapshot_*/`, `:215`, `:223`, `:229`, `:232-234` | rewrite every `.beads/` path to `.obr/`; **tighten `:142` `.beads/recovery*/` to the exact `.obr/recovery/`** (that is what **D-GITIGNORE** is about); drop `:148-149` and `:229` if P4-01/P6-00 deleted their targets | HARD | **P6-01a** owns it. A `.gitignore` that still ignores `.beads/*` while the tool writes `.obr/*` silently un-ignores every runtime artifact. |
| **A19** | **root `.ubsignore`** (tracked **[V]**) — contains `legacy_beads/` and `.beads/` | delete alongside `.rchignore`, or rewrite `.beads/`→`.obr/` | HARD | Its sibling `.rchignore` is deleted by **P4-01**; `.ubsignore` was in no list. Same class (a foreign tool's ignore file). **D-DELETIONS**; recommended: delete both in P4-01. |
| A17 | `<!-- br-agent-instructions-vN -->` / `<!-- end-br-agent-instructions -->` markers **written into the user's AGENTS.md/CLAUDE.md** | `<!-- obr-agent-instructions-v1 -->` / `<!-- end-obr-agent-instructions -->` | **LR** — `find_marker_block_range` must match the legacy marker or `obr agents --update` cannot replace an old block | `agents.rs:19,22,183,203,211,422` |

> **`.br_history` and `.br_recovery` are already *inside* `.beads/`** (`beads_dir.join(...)`), so the
> plan's "`.obr/history/` vs `.obr_history/`" question is mis-framed (§10.C9). Recommended
> `.obr/history/` and `.obr/recovery/`: the parent is already hidden, the gitignore template reads
> better, and `src/sync/path.rs`'s "Refusing to {operation} outside .beads" allowlist would reject a
> sibling location. **Collision check:** root `.gitignore:142` already has `.beads/recovery*/` —
> after A8 it must become an exact path, or it silently swallows siblings (§9 D-GITIGNORE).

#### B. Environment variables

| # | Old | New | Compat |
|---|---|---|---|
| E1–E7 | `BEADS_{DIR,JSONL,CACHE_DIR,FLUSH_DEBOUNCE,IDENTITY,REMOTE_SYNC_INTERVAL,AUTO_START_DAEMON}` | `OBR_*` | **LR** |
| E8 | `BEADS_ACTOR`, `BD_ACTOR` | `OBR_ACTOR` | **LR** (both) |
| E9 | `BD_DB`, `BD_DATABASE` | `OBR_DB`, `OBR_DATABASE` | **LR** (both) |
| E10 | **`BD_<KEY>` dynamic namespace** (`src/config/mod.rs:4345-4351`; any `BD_FOO_BAR` becomes a config key under **three** spellings via `env_key_variants`) | `OBR_<KEY>` | **LR** — keep stripping `BD_` too; warn once per process |
| E11 | `BR_OUTPUT_FORMAT` | `OBR_OUTPUT_FORMAT` | **LR** |
| E12 | `BR_{AGENT_NAME,HARNESS,MODEL,SESSION,INHERITED_CONTEXT}` | `OBR_*` | **LR** — clap `env=` takes exactly one name, so add a manual pre-`Cli::parse` pass copying legacy→new (R4 open q. 4: unvalidated against clap 4.6 when both are set) |
| E13 | `BR_NO_AUTOFLUSH`, `BD_NO_AUTOFLUSH` | `OBR_NO_AUTOFLUSH` | **LR** |
| E14 | `BR_STARTUP_CACHE{,_DIR}` | `OBR_STARTUP_CACHE{,_DIR}` | LR (cheap) |
| E15 | `BR_{DISABLE_READ_ONLY_FAST_OPEN,DISABLE_PARALLEL_JSONL_EXPORT,HISTORY_MIN_INTERVAL_SECS,DOCTOR_STALE_LOCK_THRESHOLD_SECS,DOCTOR_RUNS_DIR,MCP_READ_SNAPSHOT}` | `OBR_*` | HARD (undocumented kill-switches) |
| E16 | the ~78 test/CI/installer tokens | `OBR_*` | HARD |
| E17 | scrub lists `doctor.rs:13665-13675`, `config/mod.rs:5110-5121` | add `OBR_`, keep `BD_`/`BEADS_`/`BR_` | — |
| E18 | `capabilities.rs:213-237` `const ENV_VARS` — the **published machine contract** | rewrite to the new names **in the same commit** or `doctor capabilities` self-check drifts | — |

#### C. Crate / binary identity

| # | Old | New | Compat |
|---|---|---|---|
| I1 | package `beads_rust` | `obr` | HARD — 86 files `use beads_rust::` |
| I2 | bin `br` (`Cargo.toml:34-36`) | `obr` | HARD |
| I3 | `#[command(name = "br")]` (`src/cli/mod.rs:692`) | `"obr"` | HARD → **drives completions automatically** (`_clap_complete_obr`) |
| I4 | `upgrade.rs:25 BIN_NAME = "br"` | `"obr"` | HARD |
| I5 | `GITHUB_REPO_OWNER="Dicklesworthstone"`, `GITHUB_REPO_NAME="beads_rust"` (`cli/commands/mod.rs:61-62`) | `"jwiegley"`, `"obr"` — **or remove self-update entirely** | §9 **D-SELFUPDATE** |
| I6 | release assets `br-{ver}-{plat}.tar.gz` | `obr-…` | HARD |
| I7 | tracing filter `beads_rust=…` (`src/logging.rs:88-98`, `release.yml:109`) | `obr=…` | HARD |
| I8 | MCP `Server::new("br", …)` + description `"beads_rust (br) issue tracker MCP server."` (`src/mcp/mod.rs:1225-1227`) | `"obr"` | HARD |
| I9 | `flake.nix` `pname`/`mainProgram`/app name | `obr` | HARD |
| I10 | `Cargo.toml` `description`, `repository`, `keywords`, `exclude` | de-branded | HARD |

#### D. User-config paths

Precedence becomes `~/.config/obr/` → `~/.config/beads/` → `~/.config/bd/` → `~/.beads/`
(warn on any of the last three). Anchors: `src/config/mod.rs:206-216`, `:221-225`, `:4514-4541`,
`src/cli/commands/config.rs:1384-1399`, `:1413` (asserts on the substrings), watch-path list
`config/mod.rs:5145-5148`.

#### E. Machine-readable identifiers (entirely absent from the plan)

| # | Old | New | Compat |
|---|---|---|---|
| M1 | **26 versioned `br.*.vN` JSON schema IDs** (full list in [R4 §5.1]; anchors `src/sync/witness.rs:18`, `src/write_combining.rs:18`, `doctor_subsystems/schema_migration.rs:49`) | `obr.*.vN` | **LR for the 6 read back from disk** (`br.doctor.schema_migration.{plan,prepared,applied,failed,undo}.v1`, `br.doctor.undo.v1`); HARD for the rest. *R4 open q. 2: the remaining ~20 are INFERRED write-only — verify before shipping.* |
| M2 | `beads://…` MCP resource URIs (12 URIs, 115 hits, 10 files) | `obr://…` | HARD (MCP clients rebind per session) |
| M3 | JSON output key `"beads_dir"` (12 emit sites) | `"obr_dir"` | HARD — **this is surface, not internals: the 10 `Serialize`-able `pub beads_dir` fields must be renamed in commit P6-01a, not P6-03** |
| M4 | doctor check names `gitignore.beads_inner{,_present}`, `permissions.beads_dir` | `…obr…` | HARD |
| M5 | failure-mode IDs `fm-configs-gitignore-leaking-beads`, `fm-permissions-beads-dir-readonly` | `…-obr` | HARD |
| M6 | **9 hash domain separators**: `b"beads-rust-database-write-authority-v1\0"` (`sync/mod.rs:988`), `b"beads-rust-jsonl-write-authority-v1\0"` (`:1003`), `b"br:jsonl-witness:{root,chunk}:v1\0"` (`witness.rs:20-21`), `b"br-startup-cache-v2"` (`config/mod.rs:4992`), 4 × `beads-rust.sync-merge-*.v1` (`sync/mod.rs:2602-2606`) | `obr…` | §9 **D-DOMAINSEP**. Renaming invalidates startup caches once and breaks digest verification of an in-flight `sync --merge` receipt. **Do not bump the `.vN` suffixes** — that would imply a format change that did not happen. |
| M7 | `:BEADS_SCHEMA_VERSION:` Org drawer property | `:OBR_SCHEMA_VERSION:` | **LR** — accept both on read. §9 **U5**: recommended to do this in **P3-02** (Phase 3), not here, since nothing reads the key today and the accept-both reader is two match arms either way. Its value must be **17**, not 1 (§10.C10). |
| M8 | `#+TITLE: Beads Issues` | `#+TITLE: Obr Issues` | HARD (write-only, never read) |
| M9 | default issue-ID prefix fallback `"br"` (`src/util/id.rs:749`, `init.rs:149-163`, `config.rs:534`) — **not `"bd"`** | `"obr"` | HARD |
| M10 | `Cargo.toml [lints]`, error codes, DB table/column names | **KEEP** — verified unbranded | KEEP |

#### F. Deletions instead of de-branding (P6-00)

`docs/porting/` (3 files, 342 hits — specs for the *Go* implementation),
`docs/audit_bd_to_br_2026_05_09.md`, `tests/artifacts/perf/` (335 historical perf captures with
branded directory names). Together with P4-01's removals this takes ~450 files and ~4,400 hits out
of the audit surface. §9 D-DELETIONS.

**Deliberately NOT renamed** (historical records; G-rename allowlist row L8, excluded by pathspec):

- `CHANGELOG.md` — 341 hits of upstream release history. §9 D-CHANGELOG, recommended
  keep-verbatim-and-exclude.
- `docs/operations/UPGRADE_LOG.md` — 8,121 bytes, dated 2026-01-18 **[V]**.
- **root `UPGRADE_LOG.md`** — 6,355 bytes, dated 2026-05-14, **a different document** with 6 branded
  lines **[V]** (`diff` confirms; `git log` shows it was created at the root *after* `df0a5ff7`
  moved the earlier one into `docs/operations/`). An earlier draft's "Deliberately NOT renamed" list
  and G-rename part 1's pathspec both named only the `docs/operations/` copy, so **the root file
  failed part 1 and no commit owned it.** Fixed two ways: the pathspec now covers both (§1), and
  **P6-04 should `git mv UPGRADE_LOG.md docs/operations/UPGRADE_LOG-2026-05-14.md`** so a single
  `:!docs/operations/UPGRADE_LOG*` pathspec suffices and the repo root stops carrying a stray log.

### 5.2 Phase 6 commit series

| ID | Commit | Files | Notes |
|---|---|---|---|
| **P6-00** | `chore: delete branded historical apparatus` | ~340 | §F above. Gate: G-build, G-unit. |
| **P6-01a** | `refactor!: rename on-disk artifacts, env vars and CLI surface (+ compat shims)` | **54 `src/` files**, ~1,957 hits | Map sections A, B, D, and M3/M4/M5/M9. **The only part of Phase 6 that needs real review.** All LR shims go in one `mod legacy_compat` region (allowlist L3/L4). Hot spots: `config/mod.rs`, `cli/commands/doctor.rs`, `sync/mod.rs`, `sync/path.rs`, `sync/history.rs`, `cli/commands/init.rs`, `util/mod.rs`, `mcp/*`, `cli/commands/{agents,capabilities,upgrade,where,info,config}.rs`. Gate: all. |
| **P6-01b** | `test: mechanical fixture/snapshot update for the artifact rename` | ~380 | `tests/doctor_fixtures/`, `tests/fixtures/`, `tests/snapshots/` (**`golden_beads_init.rs` must itself be renamed** — insta derives snapshot filenames from the test module path), `tests/common/`, `tests/e2e_scripts/`. Near-zero judgement. Gate: all. |
| **P6-02** | `refactor!: rename crate beads_rust → obr, binary br → obr` | 266 **+ `fuzz/` + `benches/`** | Map section C. Only ~30 need thought; the rest is `s/beads_rust::/obr::/`. Also `tests/common/binary_discovery.rs:105-129` (hardcodes `BR_BINARY`, `cargo_bin!("br")`, `which("br")`, asserts `version.binary == "br"`) and `tests/e2e_completions.rs:31,62-63,134`. Gate: all + **G-aux**. |
| **P6-03** | `refactor: rename internal beads_* identifiers to obr_*` | 149 files, **11,812 tokens** | Pure sed + `cargo-fmt`. Suggested longest-first order: `setup_test_beads_dir` → `discover_beads_dir_*` → `*_beads_dir` → `BeadsError` → `BeadsState` → `beads_to_mcp` → `beads_dir` → bare `beads`. **One review pass to confirm no string literal changed** and that the 10 `Serialize`-able `pub beads_dir` fields were already handled in P6-01a. Gate: all + **G-aux**. |
| **P6-04** | `docs: de-brand documentation, packaging and CI` | ~55 after P6-00 | `docs/`, `AGENTS.md`, `packaging/` (all four manifests: filename, `pkgname`, binary name, URLs, license), `install.sh` (**discard the fork's copy entirely** — re-derive from upstream's 61 KB file), 8 workflows, `.github/workflows/doctor.yml`, `scripts/`, and the `git mv` of root `UPGRADE_LOG.md`. **§9 D-ACFS**: `notify-acfs.yml` notifies *upstream's* installer registry — recommend drop, not rename. README **and CLAUDE.md** are deferred (README → Phase 8, CLAUDE.md → CLEANUP). Gate: all + **G-rename**. |
| **P6-05** | `test: regenerate agent_baseline goldens` | 18 + 1 | **The plan never mentions this and it is rename-blocking.** `tests/e2e_schema.rs:920-928` asserts live CLI help/schema/example output against 18 goldens, failing with `"agent_baseline/{path} is stale; rerun with UPDATE_AGENT_BASELINE=1"` (`:1129`, `:1156`, `:1185`; env const `:19`); `scripts/verify-agent-contracts.sh:30` runs it. **Feature-gating hazard: the test is `#[cfg(feature = "self_update")]` (`tests/e2e_schema.rs:918` **[V]**) and `default = ["self_update"]` (`Cargo.toml:167` **[V]**) — see D-SELFUPDATE.** Gate: all + G-rename. |

### 5.2a Phase 6 — per-commit source-of-truth and done-criteria

The table above gives scope and file counts. This subsection gives the **source of truth** and the
**done-criteria** each Phase-6 commit was missing. P6-01a is flagged as "the only part of Phase 6
that needs real review" and it had no done-criteria at all; that is fixed first.

---

#### P6-00 · `chore: delete branded historical apparatus`

**Source of truth.** §5.1 section F; [R3 §5.1]; §9 D-DELETIONS.

**Done-criteria.**
- `docs/porting/`, `docs/audit_bd_to_br_2026_05_09.md`, `tests/artifacts/perf/` are gone.
- `git grep -l 'docs/porting\|audit_bd_to_br\|tests/artifacts/perf'` returns only files this commit
  also edits (root `.gitignore:229` references `tests/artifacts/perf/**/*.beads/` — remove that line
  here or in P6-01a's A18 pass, but do not leave a dangling rule).
- Branded-hit count drops measurably: record `git grep -Pci beads | awk -F: '{s+=$2} END {print s}'`
  before and after in the commit message.
- G-build, G-unit unchanged.

---

#### P6-01a · `refactor!: rename on-disk artifacts, env vars and CLI surface (+ compat shims)`

**Source of truth.** §5.1 map sections **A** (incl. new rows A14b, A18, A19), **B**, **D**, and
**M3/M4/M5/M9**; [R4 §5–§9]; the LR-warning template at `src/cli/commands/mod.rs:81-132`.

**Files.** 54 `src/` files (~1,957 hits) — hot spots `config/mod.rs`, `cli/commands/doctor.rs`,
`sync/mod.rs`, `sync/path.rs`, `sync/history.rs`, `cli/commands/init.rs`, `util/mod.rs`, `mcp/*`,
`cli/commands/{agents,capabilities,upgrade,where,info,config}.rs` — **plus**, and these were
previously unowned:

| Path | Why it belongs here |
|---|---|
| **the repository's own `.beads/` → `.obr/`** (`git mv .beads .obr`) | **B3.** No commit in the series owned this. Upstream tracks **ten** files there **[V]**; P6-01a is the commit that renames the directory the tool creates, so it is the commit that must rename the one this repo uses. Without it G-rename parts 1, 3 and 3b all fail. |
| `.obr/metadata.json` | `"database": "beads.db"` → `"obr.db"` (row A3). Tracked, so it trips part 3. |
| `.obr/.gitignore` | rewrite per row A16 to match the new `init` template exactly |
| `.obr/{SYNC_CLI_FLAG_SEMANTICS,SYNC_THREAT_MODEL,SYNC_SAFETY_INVARIANTS,README,MCP_AGENT_MAIL_PATTERNS}.md` | five upstream specs describing **this** tool; per plan Phase 5 they "migrate to the renamed directory with names/content de-branded where they describe THIS tool". Verified occurrence counts in §1's allowlist table. |
| `.obr/interactions.jsonl` | 3 synthetic upstream test records **[V]** — delete (D-DELETIONS) |
| **root `.gitignore`** | row A18 — 18 branded lines, incl. the `.beads/recovery*/` at `:142` that **D-GITIGNORE** is about |
| **new `tests/legacy_compat.rs`** | see below |

**`tests/legacy_compat.rs` is a deliverable of this commit, not a pre-existing file.** It does not
exist upstream **[V]** and `git grep -in deprecat -- src/` returns **zero** hits **[V]** — *every*
LR row is new code. Twenty new legacy-read code paths with no tests is the largest untested surface
this port introduces. **Done-criterion: one `#[test]` per LR row, twenty in total**, each asserting
the same three things — (1) the legacy name still works, (2) exactly one deprecation warning is
emitted on **stderr**, (3) the new name is what gets **written**:

| LR row | Test asserts |
|---|---|
| A1 `.beads/` | a `.beads/` workspace opens; warns once; a fresh `init` creates `.obr/`; no `.obr/` is created inside the legacy workspace |
| A2 `_beads/` | same, monorepo variant (skip if D-MONOREPO drops it) |
| A3 `beads.db` | opens when `metadata.json` is absent/blank AND `obr.db` missing AND `beads.db` present; warns; does **not** rename the file behind the user's back |
| A4 `beads.jsonl` | read-only fallback; a flush writes `issues.org`, never `beads.jsonl` |
| A6 `beads.{base,left,right}.jsonl` | recognised in `EXCLUDED_JSONL_FILES` **and** in doctor's merge-artifact detection; new merges write `merge.*.jsonl` |
| A7 `.br_history/` | existing backups are listed and restorable; the next backup lands in `.obr/history/`; migrate-on-first-write leaves no duplicates |
| A8 `.br_recovery/` | recognised; new recovery artifacts go to `.obr/recovery/` |
| A13 `.beads.snapshot_*` / `.beads_snap*` | still detected as foreign debris; `.obr.snapshot_*` also detected |
| A17 `<!-- br-agent-instructions-vN -->` | `agents --update` **finds and replaces** an existing legacy block rather than appending a second one (this is R29's added-destruction-mode risk) |
| E1–E7 `BEADS_{DIR,JSONL,CACHE_DIR,FLUSH_DEBOUNCE,IDENTITY,REMOTE_SYNC_INTERVAL,AUTO_START_DAEMON}` | each legacy name still takes effect; warns once per process, not once per read |
| E8 `BEADS_ACTOR`/`BD_ACTOR` | both accepted; `OBR_ACTOR` wins when both set |
| E9 `BD_DB`/`BD_DATABASE` | accepted; precedence vs `OBR_DB` asserted |
| E10 `BD_<KEY>` dynamic namespace | a `BD_FOO_BAR` still becomes a config key under all three `env_key_variants` spellings; warns once |
| E11 `BR_OUTPUT_FORMAT` | accepted |
| E12 `BR_{AGENT_NAME,HARNESS,MODEL,SESSION,INHERITED_CONTEXT}` | the manual pre-`Cli::parse` copy works, **and the both-set case is asserted** (R4 open q. 4 / §9.4 Q-CLAPENV) |
| E13 `BR_NO_AUTOFLUSH`/`BD_NO_AUTOFLUSH` | accepted |
| E14 `BR_STARTUP_CACHE{,_DIR}` | accepted |
| D user-config chain | `~/.config/obr/` → `~/.config/beads/` → `~/.config/bd/` → `~/.beads/`, each of the last three warning; first hit wins |
| M1 the 6 read-back `br.*.v1` schema IDs | a doctor migration plan/prepared/applied/failed/undo artifact written under the old ID is still read; new ones are written under `obr.*.v1` |
| M7 `:BEADS_SCHEMA_VERSION:` | accepted on read; `:OBR_SCHEMA_VERSION:` written. **If U5 chose Phase 3, this test lives in P3-02 and this row is satisfied by a pointer, not a duplicate.** |

**Every warning test must also assert JSON-safety**: under `--json`, the warning is a
`{"warning":{...}}` envelope on **stderr** and stdout remains parseable JSON. That is the whole
point of the `report_auto_flush_failure` template, and a shim that prints to stdout breaks
`--robot` for every downstream agent.

**Other done-criteria.**
- All LR shims sit in **one** clearly-marked `mod legacy_compat` region per file (allowlist L3/L4);
  `git grep -n 'legacy_compat' src/ | wc -l` shows a small, reviewable set.
- `test ! -e .beads && test -d .obr`; `git ls-files .beads` is empty.
- A fresh `obr init` in a temp dir creates **only** `.obr/` — `find . -name '*beads*' -o -name '.br_*'`
  is empty. (This is G-rename part 4's last line, run early.)
- **G-unit tier 2 (serial)** — this commit touches `config/mod.rs`, `doctor.rs` and `sync/mod.rs`,
  the three files most likely to shift where the aborters land.
- G-rename **part 3b** passes (the tracker directory), even though full G-rename is not required
  until P6-04.

---

#### P6-01b · `test: mechanical fixture/snapshot update for the artifact rename`

**Source of truth.** the P6-01a diff; insta's module-path snapshot naming.

**Fixture counts — restated, because the earlier numbers were "files with hits" presented as
totals.** Disk reality **[V]**:

| Tree | Total files | Files containing a `beads`/`br ` hit |
|---|---|---|
| `tests/doctor_fixtures/` | 206 (68 top-level entries) | **171** |
| `tests/fixtures/` | 205 | **47** |
| `tests/snapshots/` | 97 | **35** |

**Done-criteria.**
- `git grep -Pn -i 'beads|(^\|[^a-z])(br\|bd)([^a-z]\|$)' -- tests/doctor_fixtures tests/fixtures tests/snapshots`
  returns only allowlist L10 opaque issue IDs.
- `golden_beads_init.rs` is renamed **and** its snapshot files moved to the matching module path —
  `cargo test --test golden_obr_init` passes without `INSTA_FORCE_UPDATE`.
- Every snapshot change is *reviewed*, not just regenerated. State in the commit message how many
  snapshots changed and that each diff was read.
- G-e2e-subset and `e2e_doctor_fixture_suite` unmoved from their baselines.

---

#### P6-02 · `refactor!: rename crate beads_rust → obr, binary br → obr`

**Source of truth.** §5.1 map section **C** (I1–I10); [R4 §7].

**Files — the previously-missing half.** In addition to the 266 already listed:

| Path | What changes | Verified |
|---|---|---|
| `fuzz/Cargo.toml` | `name = "beads_rust-fuzz"` → `"obr-fuzz"`; `beads_rust = { path = "..", … }` → `obr = { … }` | **[V]** |
| `fuzz/fuzz_targets/*.rs` (all 9 after P4-05) | `use beads_rust::` → `use obr::` | **[V]** all 7 upstream targets do this |
| `benches/storage_perf.rs` | 11 `beads_rust::` paths at `:22`, `:23`, `:92`, `:628`, `:654`, `:673-674`, `:895`, `:956`, `:998`, `:1028`; 22 total `beads`/`jsonl` hits | **[V]** |
| `benches/benchmarks.rs` | zero hits — no change | **[V]** |
| `Cargo.lock` | root entry `name = "beads_rust"` at `:347` | **[V]** |

`fuzz/Cargo.lock` is **untracked** (`.gitignore:21` **[V]**) — nothing to commit, but delete any
stale local copy or `cargo fuzz build` fails against the old package name.

**Done-criteria.**
- **G-aux is mandatory here.** `cargo build --benches` and `cargo fuzz build` both exit 0. Without
  it, this commit can ship an uncompilable fuzz crate and broken benches with every other gate green.
- **G-locked**: `Cargo.lock`'s root entry is `obr` and `cargo build --locked` exits 0.
- `target/debug/obr` exists; `target/debug/br` does not.
- `obr --version` reports `binary == "obr"`; `tests/common/binary_discovery.rs` asserts it.
- `obr completions bash | head -1` names `_clap_complete_obr` (drives from `#[command(name)]`, I3).
- `git grep -c 'beads_rust' -- ':!docs/research' ':!CHANGELOG.md' ':!Cargo.lock'` → 0 outside L2.
- D-SELFUPDATE has been answered, because I5 (`GITHUB_REPO_{OWNER,NAME}`) is decided here.

---

#### P6-03 · `refactor: rename internal beads_* identifiers to obr_*`

**Source of truth.** the token inventory in [R4 §8]; the P6-01a diff (for what must *already* be
done).

**Files.** 149 files, 11,812 tokens — **plus `benches/storage_perf.rs`** (whose `beads`/`jsonl`
identifiers were not in the 149-file count) and the fuzz targets.

**Done-criteria.**
- **G-aux** again: benches and fuzz still compile after the sed pass.
- `git diff -U0 | grep -E '^[+-].*"' | ...` — **no string literal changed.** State in the commit
  message that this was checked; that is the single review obligation of this commit.
- The 10 `Serialize`-able `pub beads_dir` fields (map row M3) were **already** renamed in P6-01a —
  assert `git grep -n 'pub beads_dir' src/` is empty *before* this commit runs.
- `cargo-fmt --check` clean (note the binary spelling, §0.1.0).
- G-lint's histogram is unchanged or smaller; if `cargo-fmt` removed a lint, update
  `baseline/v3-clippy-lints-baseline.txt` in this same commit.

---

#### P6-04 · `docs: de-brand documentation, packaging and CI`

**Source of truth.** [R3 §3] (packaging); [R4 §6]; §9 D-ACFS, D-LICENSE, D-SUPPLYCHAIN.

**Done-criteria.**
- **G-rename runs in full for the first time and passes** — all four parts plus 3b.
- All four packaging manifests build-name, `pkgname`, URL and license are consistent with each other
  and with the D-LICENSE answer: `packaging/homebrew/*.rb:16`, `packaging/scoop/br.json:6` (**the
  file itself must be renamed to `obr.json`**), `packaging/aur/PKGBUILD:13` + the inline license at
  `:28-44`, `packaging/aur/PKGBUILD-git:42` (which today installs a `LICENSE` the fork deleted).
- `git mv UPGRADE_LOG.md docs/operations/UPGRADE_LOG-2026-05-14.md` done, root is clean.
- `notify-acfs.yml` dropped (D-ACFS) or repointed — not left aimed at upstream's registry.
- `install.sh` is upstream's 61 KB file de-branded, not the fork's stale 40 KB copy.
- Every workflow still parses: `for f in .github/workflows/*.yml; do python3 -c 'import sys,yaml;yaml.safe_load(open(sys.argv[1]))' "$f"; done`.
- CLAUDE.md is **not** created here (it lands in CLEANUP, §6.2).

---

#### P6-05 · `test: regenerate agent_baseline goldens`

**Source of truth.** `tests/e2e_schema.rs:19, 918-928, 1129, 1156, 1185, 1204`;
`scripts/verify-agent-contracts.sh:30`; §10.C20.

**Do.** `git mv agent_baseline/help/br_{help,list_help,schema_help}.txt → obr_*.txt`, edit the path
strings in `tests/e2e_schema.rs`, then
`UPDATE_AGENT_BASELINE=1 cargo test --test e2e_schema agent_baseline_snapshots_match_current_binary`.

> **The feature-gate hazard, which interacts with D-SELFUPDATE and is noted nowhere else.**
> `agent_baseline_snapshots_match_current_binary` is `#[cfg(feature = "self_update")]`
> (`tests/e2e_schema.rs:918` **[V]**) and `self_update` is in `default` (`Cargo.toml:167` **[V]**).
> **D-SELFUPDATE's recommended option (b) — "drop `self_update` from default features" — compiles
> this gate out of existence while it still reports green.** If (b) is chosen, either move the test
> out from under the cfg, or run this gate explicitly with
> `cargo test --features self_update --test e2e_schema`, and say which in the commit message.

**Done-criteria.**
- `cargo test --test e2e_schema` passes **without** `UPDATE_AGENT_BASELINE` set — and the run
  actually executed the golden test (assert it appears in the `running N tests` output, not skipped).
- `scripts/verify-agent-contracts.sh` exits 0.
- `git grep -Pn -i 'beads|(^\|[^a-z])(br\|bd)([^a-z]\|$)' agent_baseline/` → empty.
- Every regenerated golden was **read**. `README_first_80_lines.md` is also a golden, so record that
  Phase 8's README rewrite will invalidate this a second time.
- G-rename still clean.

---

## 6. Phase 7 verification sweep, then the `CLEANUP` commit

### 6.1 Phase 7 — the verification sweep, made concrete

The governing plan §5 Phase 7 asks for six things. **Four of them were owned by no step in an
earlier draft of this document, and Phase 7 was declared "unchanged from the original plan" —
i.e. never made concrete, contrary to the Phase-2 remit.** This subsection is the fix. It produces
**no commit**; it is a checklist you run on the branch tip after P6-05 and before CLEANUP, and any
red step sends you back to the commit that owns it.

Plan §5 Phase 7 verbatim, mapped to the steps below:

| Plan asks for | Step |
|---|---|
| "Full gates" | 1, 2 |
| "the rename-completeness audit" | 3 |
| "Org round-trip test" | 4 |
| "a fresh `obr init` → create → flush → **clone-simulation** → import cycle in a throwaway dir" | 6 |
| "confirming: only `.obr/` artifacts are created … and the file written is `issues.org`" | 6 |
| "**legacy `.beads` workspaces still open with a warning**" | 7 |

Plus three obligations this document adds because nothing else covers them: the serial G-unit
BASELINE.md mandates "at Phase 7 minimum" (step 1), R2's closing write-path sweep (step 5), and the
R18 SIGPIPE smoke that §8 mentions but no step owned (step 8).

---

**Step 1 — full serial G-unit.** Not the fast tier.

```bash
SKIP=$(sed 's/^/--skip /' "$BASE/v3-unit-aborters.txt" | tr '\n' ' ')
$NIXRUN cargo test --lib --bins -- --test-threads=1 $SKIP 2>&1 | tee /tmp/p7-unit.log
grep -E '^test .* FAILED$' /tmp/p7-unit.log | sed 's/^test //; s/ \.\.\. FAILED$//' | sort -u \
  > /tmp/p7-unit-failures.txt
diff <(sort -u "$BASE/v3-unit-failures-complete.txt") /tmp/p7-unit-failures.txt
```
**Pass:** diff empty, and the run printed a `test result:` summary (no new aborter).
BASELINE.md mandates this "at Phase 7 minimum". Do not substitute the parallel run.

**Step 2 — every other gate, in order.**
```bash
$NIXRUN cargo build && $NIXRUN cargo build --locked
$NIXRUN cargo-clippy --all-targets -- -D warnings          # histogram vs baseline (§1 G-lint)
for t in e2e_basic_lifecycle e2e_sync_git_safety jsonl_import_export e2e_history \
         e2e_doctor_fixture_suite e2e_org_roundtrip e2e_schema legacy_compat; do
  $NIXRUN cargo test --test "$t"; done
$NIXRUN cargo build --benches && $NIXRUN cargo fuzz build   # G-aux
```
**Pass:** every failure-name set is a subset of its committed baseline; `e2e_org_roundtrip`,
`legacy_compat` and `e2e_schema` are **fully green** (they are the port's own tests — they get no
baseline allowance).

**Step 3 — G-rename, all five parts** (§1), from a clean `git status`.
**Pass:** parts 1, 2, 3, 3b produce only allowlisted hits; part 4's five binary checks are all empty
and the fresh-`init` `find` is empty.

**Step 4 — G-org-roundtrip** on both the synthetic fixture set *and* the real 549-issue tracker
corpus (`.obr/issues.org`).
**Pass:** all six assertions of §1 G-org-roundtrip hold on both.

**Step 5 — R2 §9 Q5's closing write-path sweep.** *(Previously in no step.)* R2 could not close, read-only,
whether anything writes the export file outside the paths it audited.

```bash
rg -n 'durable_rename|write_all|fs::write|write_atomic|persist\(' src/ | rg -i 'jsonl|org|export'
```
**Pass:** every hit is one of — the export path (P3-05), auto-flush (both variants, P3-07), `--no-db`
flush, reconcile apply, base-snapshot refresh (P3-08), history backup (P3-11), or a doctor fixer
(P3-12). **Any hit outside that list is an unaudited writer of the durable artifact and blocks the
port**; add it to the P3-05/P3-07 anchor tables and re-run the phase that owns it. Record the sweep's
output in the CLEANUP known-residual doc so the next person does not have to re-derive the list.

**Step 6 — the fresh-init → clone-simulation cycle.** *(Previously in no step; the plan asks for it
explicitly.)* The point is to prove the artifacts a **git clone** carries are sufficient — i.e. that
nothing essential lives only in the untracked `obr.db`.

```bash
set -e
A=$(mktemp -d); B=$(mktemp -d)
cd "$A"
git init -q .
obr init
obr create --title="unicode 🎉 and a colon label" --type=task --priority=2
obr create --title="second" --type=bug --priority=0
obr dep add <id2> <id1> --type=blocks         # ids from the create output
obr comment add <id1> "a comment"
obr sync --flush-only

#  (a) only .obr/ artifacts exist
find . -name '*beads*' -o -name '.br_*' -o -name 'bd.sock' | grep . && echo 'FAIL: legacy artifact'
test -f .obr/issues.org                                  # (b) the file written is issues.org
test ! -f .obr/issues.jsonl
grep -c '^\* ' .obr/issues.org                           # == 2

#  (c) CLONE SIMULATION: carry only what git tracks. No obr.db, no history, no locks.
cd "$A" && git add -A && git -c user.email=t@t -c user.name=t commit -qm seed
git clone -q "$A" "$B/clone" && cd "$B/clone"
test ! -e .obr/obr.db                                    # the DB really is absent
obr sync --import-only
obr list --json | jq 'length'                            # == 2
diff <(cd "$A" && obr list --json | jq -S .) <(obr list --json | jq -S .)
obr sync --flush-only
diff "$A/.obr/issues.org" .obr/issues.org                # byte-identical after a clone round-trip
```
**Pass:** every command exits 0, both `diff`s are empty, no legacy-named file is ever created.
**This is the single strongest end-to-end statement the port can make**, because it exercises the
default format, the default directory, the import chokepoint, the prefix inference on a DB-less
open, and determinism — in the configuration a real collaborator would hit.

**Step 7 — the legacy-workspace warning check.** *(Previously in no step. Also the acceptance test
for M3's `tests/legacy_compat.rs`.)*

```bash
C=$(mktemp -d); cd "$C"
mkdir -p .beads
printf '{"database":"beads.db","jsonl_export":"issues.jsonl"}\n' > .beads/metadata.json
printf 'issue_prefix: leg\n' > .beads/config.yaml
cp <a small valid issues.jsonl> .beads/issues.jsonl

obr list                > /tmp/leg.out 2> /tmp/leg.err ;  echo "exit=$?"
obr list --json         > /tmp/leg.json 2> /tmp/leg.jerr
```
**Pass, all six:**
1. `exit=0` — the legacy workspace **opens**.
2. `/tmp/leg.out` lists the issues; the data is real, not empty.
3. `/tmp/leg.err` contains **exactly one** deprecation warning naming `.beads` and pointing at
   `.obr` (`grep -c -i 'deprecat\|legacy' /tmp/leg.err` == 1). Not zero — silence here means the LR
   shim is missing and users will never learn to migrate. Not many — once per process.
4. `jq . /tmp/leg.json` **parses**, and the warning is in `/tmp/leg.jerr` as a `{"warning":{...}}`
   envelope, not on stdout. This is the `--robot` contract.
5. `find "$C" -name '.obr*'` is **empty** — opening a legacy workspace must not silently create the
   new one beside it.
6. The legacy artifacts are unmodified: `md5sum` of `.beads/issues.jsonl` before and after.

Repeat with `.beads/beads.db` present and `metadata.json` absent (map row A3's narrow LR condition)
and with a `.beads/.br_history/` containing one backup (row A7).

**Step 8 — the R18 SIGPIPE smoke.** *(§8's R18 row asks for it; no step owned it.)*
```bash
obr list | head -1; echo "exit=$?"
obr list --json | head -1 >/dev/null; echo "exit=$?"
```
**Pass:** exit 0 (or at worst a clean 141), **not 134**. A 134 is the dossier's R18 panic and must be
recorded in the CLEANUP known-residual doc even if it is not fixed here.

**Step 9 — the org2jsonl supply-chain check** (the other half of R30).
```bash
git -C /tmp rm -rf org2jsonl-probe 2>/dev/null; \
  git clone -q --depth 1 https://github.com/jwiegley/org2jsonl /tmp/org2jsonl-probe
grep -n 'org2jsonl' Cargo.toml            # full 40-hex rev, https URL, no path=, no file://
$NIXRUN cargo tree -p org2jsonl
```
**Pass:** the pinned rev is fetchable anonymously and matches `Cargo.lock`. This is what makes CI
buildable by someone other than the author (dossier R14/R30) and it must be re-checked after the
flake work, not just at P3-01.

**Step 10 — record the sweep.** Write the outputs of steps 1–9 into the CLEANUP known-residual doc
(§6.2). A sweep whose results nobody wrote down cannot be compared against next time.

---

### 6.2 `CLEANUP` · `chore: final tree cleanup, CLAUDE.md, fact refresh`

Run after §6.1's sweep is fully green, before Phase 8's docs work.

**Files.** new `CLAUDE.md`; `AGENTS.md`; new `docs/KNOWN_RESIDUALS.md`; `src/cache.rs` (deleted or
annotated); whatever `git status --ignored` turns up.

**Source of truth.** `git -C /Users/johnw/src/obr show 22adc72b:CLAUDE.md` (75 lines) for the
CLAUDE.md skeleton; §10 for every fact that must change; §8 and §9 for the residual list.

**Scope.**
- Remove any residual stray artifacts the earlier deletions missed (`git status --ignored` review).
- **`src/cache.rs` is dead code** — 641 lines of S3-FIFO policy kernel with **zero callers**
  (`grep -rn "crate::cache\|cache::" src/` outside itself → 0) [R1 §3.3]. Delete it or leave it with
  a one-line note; do not silently inherit it as "used".
- **Create `CLAUDE.md`. It does not exist on the branch.** *(Previously this commit said "refresh
  CLAUDE.md", on a file that would not be there.)* Upstream has **no** `CLAUDE.md`
  (`git ls-files | grep -i '^CLAUDE'` is empty **[V]**); it is fork commit **`22adc72b`**
  (`docs: add CLAUDE.md for Claude Code session context`, 75 lines, one file **[V]**), which the
  governing plan §3.1 classifies "keep; update facts after sync" — and which was the **only one of
  the fork's 20 commits with no home in the commit series.** It lands here rather than in P6-04 so
  that it is written de-branded from the start and never has to pass G-rename twice.
- **Write both `CLAUDE.md` and `AGENTS.md` against current facts.** Every one of these is wrong in
  the fork's copy:

  | Fork CLAUDE.md says | Truth on the branch |
  |---|---|
  | Binary `br` (beads_rust) | binary **`obr`**, crate **`obr`** |
  | `.beads/` workspace | **`.obr/`** |
  | JSONL storage | **Org** (`issues.org`), JSONL by explicit pin |
  | `#![forbid(unsafe_code)]` | **`unsafe_code = "deny"`** in `Cargo.toml [lints.rust]` + one sanctioned `#[allow(unsafe_code)]` in `sync::db_inode_lock` (`src/lib.rs:20-27`). **Re-imposing `forbid` will not compile** (§10.C5). |
  | edition 2024 / nightly | still true, but `rust-version = "1.88"`, not 1.85 |
  | `src/storage/sqlite.rs` ~5K lines | **34,865** |
  | `src/sync/mod.rs` ~5K lines | **21,609** |
  | 781 unit tests | ~2,700 collected; **266 fail at baseline** (§0.1) — say so, with a pointer to `docs/research/upgrade/BASELINE.md` |
  | rusqlite | **`fsqlite`** (pure-Rust) — §10.C6 |
  | `cargo clippy --all-targets -- --deny warnings` | **`cargo-clippy`**, inside the pinned nix shell; the space form is a rustup shim (§0.1.0) |
  | `nix develop` | the canonical `nix shell` fenix pin, unless **P4-02** actually fixed the dev shell — check before writing it down |
  | schema version | `CURRENT_SCHEMA_VERSION = 17` |
  | 35+ subcommands | the `Commands` enum is **47** (§10.C17) |

- **Write the known-residual list** as a maintained doc (`docs/KNOWN_RESIDUALS.md`), not a report
  file: the accepted Org perf regression (§9 U6), the R4-residual and R6-residual upstream defects
  (§8), the known-lossy Org fields if U2 resolves to (c), the four pre-existing red baseline suites
  with their counts, the 85 first-party clippy errors, the §6.1 step-5 sweep output, and the
  follow-up audits §8 flags (R10 merge correctness, R12, R13, R17, R24, R28 redirect-target
  validation, R32 `history restore` path traversal).
- Final `agent_baseline` regeneration if Phase 8 has already rewritten the README.

**Done-criteria.**
- `CLAUDE.md` exists, is de-branded, and **every fact in the table above is correct** — verify each
  one against the tree rather than copying this table.
- `AGENTS.md` agrees with `CLAUDE.md`; no contradiction between them.
- `docs/KNOWN_RESIDUALS.md` exists and covers all eight bullet categories above.
- `git status --ignored --porcelain | grep -v '^!! target/'` is reviewed and empty of surprises.
- **Every gate, including §6.1's serial G-unit and G-aux.**

**Gate.** Every gate: G-build, G-locked, G-lint, **G-unit tier 2**, G-e2e-subset, G-org-roundtrip,
**G-aux**, G-rename.

---

## 7. Master anchor index

One table. Every integration point, the upstream function and line the implementer edits, and the
commit that owns it. All anchors are `/Users/johnw/src/obr-org-forward-port` @ `d1fb0d3a`; the ones
marked **[V]** I re-verified in this session, the rest are inherited from R1/R2 with the brief's own
verification.

### 7.1 `src/sync/mod.rs` (21,609 lines)

| Line | Function | Role | Commit |
|---|---|---|---|
| `988`, `1003` | write-authority domain separators | branded hash inputs | P6-01a (M6) |
| `1006` | `.br-jsonl-write-<24hex>.lock` sidecar name | trips the git-safety allowlist **at baseline** **[V]** | — (note only) |
| `1010` | `blocking_jsonl_family_write_lock_with_timeout` | format-agnostic | — |
| `1367` | `try_sync_lock` | `.sync.lock` around auto-flush | — |
| `1464` / `1521` | `perform_conditional_namespace_change` | `renameat2` CAS; **cfg-gated Linux/Android/Apple, hard error elsewhere** | **P4-06** (audit the workflow matrices — no `windows-*` runner may reach `:1521`) |
| `1544`–`1778` | `publish_staged_jsonl_conditionally*` | atomic publication; **format-agnostic, do not touch** | — |
| **`1818`** | **`export_temp_path_for_attempt`** | `with_extension("jsonl.{pid}.tmp")` — **silent-wrongness trap** **[V]** | **P3-03** |
| `1830`, `1877`, `1912`, `1940` | temp-file creators | callers of the above | P3-03 |
| `2350`–`2380` | `ImportResult` | now carries `skipped_count` + 9 more counters | — (R4 stale) |
| `2602-2606` | 4 × sync-merge domain separators | branded hash inputs | P6-01a (M6) |
| `3694` / `3759` / `4565` / `4683` | reviewed-additive reconcile | **reject `.org`** | **P3-10** |
| **`8246`** | `validate_jsonl_issue_records_from_reader` | doctor's record validator | **P3-05** |
| `8321` / `8548` / `8566` | `preflight_export` / `preflight_import[_impl]` | inherit format-awareness; audit strings | P3-10 |
| `8932` / `8944` | `scan_conflict_markers` / `detect_conflict_marker` | textual; **no change** | — |
| `8962`–`9010` | `ensure_no_conflict_markers*` | **no change** | — |
| **`9022`** | **`analyze_jsonl_from_reader`** | id harvest + **duplicate-id hard error** | **P3-05** |
| **`9081`** | **`verify_exported_jsonl_snapshot_integrity`** | staged-file id-set check | **P3-05** |
| `9183` | `read_jsonl_lines_by_id` | incremental fallback | P3-07 (bypassed) |
| `9222` | `export_issue_ids` | canonical id-sorted order **[V]** | — |
| `9237` / `9291` | `hydrate_export_issue_batch` / `_full_scan` | batching | — |
| **`9386`** | **`write_export_issue_jsonl`** | serial emission | **P3-05** |
| `9421` | `struct PreparedExportIssue` | `jsonl_line` → `record_bytes` **[V]** | P3-05 |
| **`9463`** | **`prepare_export_issue_jsonl`** | parallel-worker emission | **P3-05** |
| `9493` / `9506` | `prepare_export_issue_chunk` / `_parallel` | thread the format | P3-05 |
| `9555` | `write_prepared_export_entries` | field rename only | P3-05 |
| `9642` | `export_to_jsonl_with_policy` | public entry, name unchanged **[V]** | — |
| `9704` | `export_to_jsonl_with_policy_expected_authority` | the ~400-line body **[V]** | P3-05 |
| `9720` | `validate_sync_path_with_external` call | allowlist gate | P3-03 |
| `9817`–`9871` | the two data-loss guards | call `analyze_jsonl_snapshot` | P3-05 |
| `~9894` | after `let mut hasher = Sha256::new();` **[V]** | **write the Org file header here** | **P3-05** |
| `9951`–`9957` | staged-bytes-vs-hash assertion | **fails on every Org export until P3-04** | P3-04 |
| `10150` | `export_to_writer_with_policy_and_retention_at` | **stdout emission — third site** | P3-05 |
| `10656` / `10666` | `finalize_export*` | clears dirty flags | — |
| `10821` | `normalize_issue_for_export` | canonical relation order | — |
| `11073` / `11225` / `11320` / `11356` / `11377` / `11405` / `11475` | incremental line-splice machinery | **structurally incompatible with Org** | **P3-07** |
| **`11492`** | **`try_incremental_auto_flush`** | early `Ok(None)` for non-JSONL **[V]** | **P3-07** |
| **`11620`** | **`auto_flush`** | branches at `:11678` **[V]** | P3-07 |
| `11744` / `11752` | `read_issues_from_jsonl[_snapshot]` | merge + base-snapshot reads | P3-06 |
| `11839` | `detect_collision` | phases: ext-ref → **id** → hash → new | — (R6-residual) |
| `11880` | `determine_action` | wall-clock LWW | — (R4-residual) |
| `11931` | `normalize_issue` | label dedupe, hash recompute | P3-06 (must run for Org) |
| `12056` | `parse_normalized_import_issue` | normalize + validate per record | P3-06 |
| **`12077`** | **`for_each_jsonl_import_issue`** | **the import chokepoint** **[V]** | **P3-06** |
| `12106` / `12284` / `12434` / `13008` | its four callers | free once the chokepoint dispatches | P3-06 |
| `12150` | `build_prefix_renames` | `--rename-prefix` mechanism | P5-03 |
| `12228` / `12232` | hash-map preload + tie-break | scan-order-dependent | — (R6-residual) |
| `12375` / `12393` / `12496` | certified-skip machinery + `needs_flush` | R4 mitigation | — |
| **`12539`** | **`import_from_jsonl`** | public entry, name unchanged **[V]** | — |
| `12561` | `import_from_jsonl_snapshot` | three streaming passes | P3-06 (cache the parse) |
| `12641` | `set_metadata_in_tx(JSONL_CONTENT_HASH)` | must agree with export | P3-04 |
| **`12769`** | **`compute_jsonl_hash_from_reader`** | canonical: **skip blank, trim both sides** **[V]** | **P3-04** |
| `12805` | `compute_jsonl_hash(path)` **[V]** | dispatch by extension | P3-04 |
| `13923` | `save_base_snapshot_from_jsonl_snapshot` | parses + re-serializes — **use this for Org** **[V]** | P3-08 |
| **`13961`** | **`refresh_base_snapshot_from_flushed_jsonl_snapshot`** | **byte copy — silent breakage** **[V]** | **P3-08** |
| `14018` | `load_base_snapshot` **[V]** | parses the anchor | P3-08 |
| `14469` | `scan_jsonl_for_tombstone_filter_from_reader` | 17th (de)serialization site | P3-06 |

### 7.2 `src/sync/path.rs`

| Line | Item | Commit |
|---|---|---|
| `16-27` | module doc table repeating the allowlist | P3-03 |
| **`85-94`** | **`ALLOWED_EXTENSIONS`** — 8 entries, no `org` **[V]** | **P3-03** |
| `512-544` | `validate_extension_and_name` | P3-03 |
| **`546-559`** | **`is_allowed_jsonl_temp_name`** — `*.jsonl.tmp` and `<base>.jsonl.<digits>.tmp` only **[V]** | **P3-03** |
| **`679-690`** | **`validate_external_jsonl_path`** — `"must be a .jsonl file"` **[V]** | P3-03 (§9 U-EXTERNAL) |
| `949-979` | `validate_pinned_jsonl_leaf` — extension-agnostic, **no change** | — |
| **`2274-2301`** | **`compute_snapshot_content_sha256(backing: &File, deadline)`** — no path param **[V]** | **P3-04** |
| `~2407` | its sole call site inside `capture_jsonl_source_snapshot`; **`path` is in scope** **[V]** | P3-04 |
| `2548-2550` | `validate_jsonl_fd_metadata` — regular-file check, **no change** | — |

### 7.3 `src/config/mod.rs` (9,513 lines)

| Line | Item | Commit |
|---|---|---|
| `55-57` | `is_beads_dir_name` **[V]** | P6-01a (A1/A2) |
| `60` | `DEFAULT_DB_FILENAME = "beads.db"` **[V]** | P6-01a (A3) |
| **`62`** | **`DEFAULT_JSONL_FILENAME = "issues.jsonl"`, private** **[V]** | **P3-09** |
| `64` | `LEGACY_JSONL_FILENAME = "beads.jsonl"` **[V]** | P3-09 |
| `66` | `RECOVERY_DIR_NAME = ".br_recovery"` **[V]** | P6-01a (A8) |
| `71-79` | `EXCLUDED_JSONL_FILES` (6 names) **[V]** | P6-01a (A6) |
| `147-162` | `discover_jsonl` — 2-tier | **P3-09** |
| `241`–`324`–`394` | workspace discovery incl. `--db` handling | P6-01a |
| `3324-3389` | `flush_no_db_if_dirty`; **`:3352` hardcoded default path** | P3-09 |
| `3700`/`3728-3796` | `--no-db` open + seeding | free after P3-06 |
| `4146-4186` | `first_prefix_from_jsonl_{snapshot,reader}` — **needs an Org branch** | **P3-09** |
| `4219-4262` | `resolve_jsonl_path` — 5 priorities; **priority 2 is the migration trap** | P3-09 (test only) |
| `4345-4351` | `BD_<KEY>` dynamic namespace + `env_key_variants` | P6-01a (E10) |
| `4975-4992` | startup-cache root + `b"br-startup-cache-v2"` | P6-01a (A12/M6) |
| `5110-5131` | `startup_cache_env_witness` scrub list | P6-01a (E17) |
| `5145-5148` | user-config watch-path list | P6-01a (D) |

### 7.4 Elsewhere

| Anchor | Item | Commit |
|---|---|---|
| `src/model/mod.rs:64` | `Status::Draft` **[V]** | P3-02 |
| `src/model/mod.rs:557-566`, `:568-585` | `source_repo_path`, `agent_context` | P3-02 |
| `src/storage/schema.rs:11` | `CURRENT_SCHEMA_VERSION = 17` **[V]** | CLEANUP (docs) |
| `src/storage/sqlite.rs:16187`, `:16214` | `provides:<cap>` capability label wire format | P3-02 (B1) |
| `src/cli/commands/sync.rs:1522`/`:1553` | `validate_sync_paths` extension gate **[V]** | **P3-03** |
| `src/cli/commands/sync.rs:1994, 2045, 2086` | witness call sites | P3-10 |
| `src/cli/commands/sync.rs:3252-3254` | `--rename-prefix` | P5-03 |
| `src/cli/commands/vcs.rs:259-263` | `.jsonl`-only diagnostic gate **[V]** | **P3-10** |
| `src/cli/commands/stats.rs:92` | the one hardcoded path **[V]** | P3-09 |
| `src/cli/commands/stats.rs:644-656` | `compute_recent_activity` — already path-aware | — |
| `src/cli/commands/init.rs:171-174, 199-257, 360, 406` | metadata, gitignore template, seed, art | P3-09 / P6-01a |
| `src/cli/commands/mod.rs:61-62` | `GITHUB_REPO_{OWNER,NAME}` | P6-02 (§9 D-SELFUPDATE) |
| `src/cli/commands/mod.rs:81-133` | `report_auto_flush_failure` — **the LR-warning template** | P3-07 / P6-01a |
| **`src/mcp/mod.rs:120`** **[V]** | the **MCP** auto-flush remediation string, hardcoding `.beads/issues.jsonl` | **P3-07** (filename half) / P6-01a (`.beads`) / P6-02 (`br`) |
| `src/cli/commands/agents.rs:19,22,183,203,211,422` | AGENT_BLURB markers written into user files | P6-01a (A17) |
| `src/cli/commands/capabilities.rs:213-237` | published `ENV_VARS` contract | P6-01a (E18) |
| **`src/cli/commands/capabilities.rs:703`** **[V]** | published example `"br dep import .beads/issues.jsonl --robot"` | **P3-10** (filename) / P6-01a + P6-02 (rest) |
| **`src/cli/commands/robot_docs.rs:17`** **[V]** | the agent `GUIDE` const: "exports .beads/issues.jsonl" — executed by G-rename part 4 | **P3-10** (filename) / P6-04 (rest) |
| **`src/cli/mod.rs:962`** **[V]** | `sync` long-help EXAMPLES: `Export database to .beads/issues.jsonl` | **P3-10** |
| **`src/cli/mod.rs:2865`** **[V]** | `--flush-only` doc comment: `(DB → .beads/issues.jsonl)` | **P3-10** |
| `src/cli/mod.rs:2915` **[V]** | `--witness` doc comment ("the resolved issues.jsonl bytes") | P3-10 |
| `src/cli/commands/doctor.rs` (12 sites, §2 P3-12; 142 `issues.jsonl` hits **[V]**) | export-file checks | P3-12 |
| **`src/cli/commands/doctor_subsystems/surface.rs:413`** **[V]** | `jsonl_present = issues.jsonl \|\| beads.jsonl` — the only `doctor_subsystems/` file with hits | **P3-12** |
| `src/cli/commands/doctor_subsystems/surface.rs:411` **[V]** | `beads.join("beads.db")` | P6-01a (A3) |
| `src/sync/history.rs:166-227, 384, 502, 644-648` | backup naming + `.br_history` | P3-11 / P6-01a |
| **`src/cli/commands/history.rs:878-885`** **[V]** | `is_default_jsonl_target` — `beads_dir.join("issues.jsonl")` at `:879`; the **`history restore` default-target** decision; 18 `issues.jsonl` hits in the file | **P3-11** |
| `src/sync/witness.rs:18-21, 218, 322` | line-chunk Merkle witness | P3-10 (reject) |
| `src/sync/db_inode_lock.rs` | **zero JSONL assumptions** — the plan's guess is wrong | — |
| `src/cache.rs` | **dead code, zero callers** | CLEANUP |
| `src/util/id.rs:216-227`, `:749` | fallible `generate`; default prefix `"br"` | P6-01a (M9) |
| `src/mcp/mod.rs:1225-1227`, `resources.rs`, `tools.rs` | server name + `beads://` URIs | P6-02 / P6-01a |
| `tests/e2e_sync_git_safety.rs:1274-1283` | **hand-copied second allowlist** | **P3-03** |
| `tests/e2e_schema.rs:19, 918, 920-928, 1129, 1156, 1185, 1204` | agent_baseline goldens; **`:918` is `#[cfg(feature = "self_update")]`** **[V]** | **P6-05** (+ D-SELFUPDATE) |
| `tests/common/binary_discovery.rs:105-129` | hardcoded `br` discovery | P6-02 |
| `tests/common/cli.rs:95` | `BrWorkspace` — add `export_path` / `pin_jsonl` | P3-13 |
| **`tests/legacy_compat.rs`** | **does not exist upstream [V]** — created by P6-01a, one test per LR row | **P6-01a** |
| **`fuzz/Cargo.toml`** **[V]** | package `beads_rust-fuzz`, dep `beads_rust = { path = ".." }`; 7 targets all `use beads_rust::` | P4-05 (stanzas) / **P6-02** (identity) — gated by **G-aux** |
| **`benches/storage_perf.rs:22,23,92,628,654,673-674,895,956,998,1028`** **[V]** | 11 `beads_rust::` paths, 22 `beads`/`jsonl` hits | **P6-02 / P6-03** — gated by **G-aux** |
| **root `.gitignore`** (18 branded lines **[V]**) | artifact-name rules incl. `:142` `.beads/recovery*/` | **P6-01a** (map row A18, §9 D-GITIGNORE) |
| **root `.ubsignore`** **[V]** (tracked; `legacy_beads/`, `.beads/`) | foreign ignore file, sibling of `.rchignore` | **P4-01** (delete) or P6-01a (rewrite) — map row A19 |
| **root `UPGRADE_LOG.md`** **[V]** | distinct from `docs/operations/UPGRADE_LOG.md`; 6 branded lines | **P6-04** (`git mv`); allowlist L8 |
| **`.beads/` — ten tracked files** **[V]** | the repository's own tracker directory | **P5-01/P5-02** (contents) / **P6-01a** (`git mv .beads .obr` + de-brand) |

---

## 8. Risk register — every dossier §5 risk, R1 through R32

**Status legend.** **FIXED-UPSTREAM** = independently fixed on `upstream/main`, cited.
**CARRIES-FORWARD** = still live and the port must act. **PARTIAL** = half fixed; a named residual
survives. **OUT-OF-SCOPE** = real but not this port's job. **RE-ASSESS** = the fork-era description
no longer applies and no brief closed it; do not carry the old wording forward.

| # | Risk (dossier §5 title, abbreviated) | Status | Evidence | What the port must do |
|---|---|---|---|---|
| **R1** | Concurrent auto-flush silently drops committed writes | **FIXED-UPSTREAM** | Three layers: process `DatabaseFamilyWriteLock` (`src/main.rs:141-148`); `.sync.lock` flock around auto-flush (`main.rs:833`, `sync/mod.rs:1367`) that reports skips loudly; JSONL-family lock (`sync/mod.rs:1010`, taken `:9736`) + CAS publication verifying the *displaced* file (`:1685-1697`). [R1 §7.1] | **Nothing.** Do not re-port the fork's workaround. Note the primitive is cfg-gated Linux/Android/Apple (`:1464` vs the hard-erroring `:1521`) — constrains CI runner choice. |
| **R2** | `run_auto_flush` swallows every error at `debug!` | **FIXED-UPSTREAM** | `main.rs:838/849/871` → `report_auto_flush_failure` (`cli/commands/mod.rs:86-133`): `tracing::warn!`, stderr remediation unless `--quiet`, `{"warning":{"code":"AUTO_FLUSH_FAILED",…}}` under JSON/TOON. Exit stays 0 (the mutation did succeed). [R1 §7.2] | **P3-07** must de-hardcode the `.beads/issues.jsonl` literal in the remediation sentence; **P6-01a** de-brands the rest. This function is also the LR-warning template for every Phase-6 shim. |
| **R3** | Routine import destroys an issue's entire audit history | **FIXED-UPSTREAM** | `upsert_issue_for_import_in_tx` (`sqlite.rs:17299-17325`) probes then UPDATEs in place — no DELETE, so `ON DELETE CASCADE` never fires. Pinned by a regression test naming the defect (`sqlite.rs:28437-28480`, issue #263a). [R1 §7.3] | Nothing. |
| **R4** | Import is selective, export is total → silent divergence | **PARTIAL** | Fixed: `ImportResult` now carries `skipped_count` + 9 counters (`:2350`); sync prints a skip breakdown (`cli/commands/sync.rs:3776`); uncertified skips set `needs_flush=true` (`:12496`), forcing a full re-export. Still true: LWW skip (`:11880`), unconditional hash stamp after skips (`:12639`), **no DL-2b guard** (`:9817-9871` only computes `jsonl_ids − db_ids`). [R1 §7.4] | **Carry forward as R4-residual:** repair is always DB-wins, so a hand-resolved git merge is now reverted *promptly and reliably* rather than accidentally, and the trigger is logged only at `debug!`. **§9 D-RESIDUALS** decides whether to fix during the port. **Dossier's "no skipped_count" complaint is stale** (§10.C13). |
| **R5** | Fixed temp filename corrupts export / bricks the workspace | **FIXED-UPSTREAM** — with an inverse hazard | pid in the name (`:1820`), `create_new(true)` + mode 0600 + retry + hard error on a symlink at the temp path (`:1848-1873`), CAS publication. [R1 §7.5] | **Do not re-apply the fork's fix — you would be re-fixing a fixed bug.** But upstream's fix hardcodes `jsonl` in the extension, so **P3-03** must re-introduce the `is_org_format` branch *inside* upstream's function, and patch the allowlist in the same commit. |
| **R6** | Content-hash dedup annihilates independently created issues | **PARTIAL** | Fixed: collision phases reordered to ext-ref → **id** → hash (`:11839-11876`, was hash-before-id at the fork point); tombstones excluded from the hash map (`:12228`). Still broken: `find_by_content_hash` has **no `ORDER BY`** (`sqlite.rs:17063-17079`); `idx_issues_content_hash` still non-unique (`schema.rs:269`); the preload tie-break reads `get_all_issues_metadata` (`sqlite.rs:7390`) which has no `ORDER BY`; dedup still unconditional on every import. [R1 §7.6] | **Carry forward as R6-residual.** **§9 D-RESIDUALS.** Note the dossier's proposed fix is half-done *and not where the dossier looked* (§10.C14). |
| **R7** | Hard ceiling at exactly 32,767 exportable issues | **FIXED-UPSTREAM** | `clear_dirty_issues_in_tx` (`sqlite.rs:14284-14298`) issues one parameterized DELETE per issue, no `IN (?,?,…)`; the duplicate `clear_dirty_flags` (`:16687`) forwards to `clear_dirty_issue_ids_in_tx` (`:14312`) which chunks at 900 *and* deletes row-by-row. Bonus: clearing is now `marked_at`-aware. [R1 §7.7] | Nothing. |
| **R8** | `--db` + auto-flush writes into another workspace's file | **FIXED-UPSTREAM** | Four-way path resolution collapsed to one: `StartupContext::init` (`main.rs:906`) uses `discover_beads_dir_with_cli`; auto-flush (`main.rs:829-876`) passes that same `ConfigPaths`; `auto_flush` **takes** `jsonl_path` as a parameter (`sync/mod.rs:11620`). The `let (_, _paths)` discard antipattern is gone. [R1 §7.8] | Nothing. |
| **R9** | Org round-trip destroys any label containing `:`, including `provides:` capabilities | **CARRIES-FORWARD — and worse** | Fork-only bug; upstream has no Org code. But `LabelValidator` **explicitly permits** `:` (`src/validation/mod.rs:392-400`, with a test named `label_validation_allows_namespaced_labels`) and `provides:<cap>` is the live wire format of the capability-dependency feature (`sqlite.rs:16187` builds, `:16214` strips). A flush+import turns `provides:auth` into two labels and the feature stops resolving, silently. [R2 §4/B1] | **P3-02**, §9 **U3**. ~25 lines: canonical `:LABELS:` drawer property (compact JSON) as authoritative + Org tags for ergonomics + tag fallback for old files. **The dossier's fix ("escape or reject colons") is partly harmful** (§10.C12). |
| **R10** | `sync --merge` structurally non-functional, force-overwrites | **RE-ASSESS** (likely superseded) | Upstream's 2026-07-27 rescue merge (`251b501b`/`dbdb1296`) rebuilt write-authority, conditional publication and `sync --merge` sagas; `a3a1243f` added `sync --reconcile`. No brief audited merge correctness end to end. [R1 §1.3] | **P3-08 is a prerequisite** (without it the Org base anchor is poisoned) but is not a claim that merge is correct. Do not port the fork's merge workarounds. Flag a dedicated merge audit as follow-up. |
| **R11** | Wall-clock LWW destroys work with no merge involved; a future timestamp pins an issue | **CARRIES-FORWARD** (upstream defect) | `determine_action` (`sync/mod.rs:11880`) is still pure wall-clock LWW on `updated_at`; no logical clock. Same root cause as R4-residual. [R1 §7.4] | Out of the port's scope; record in the CLEANUP known-residual doc. §9 D-RESIDUALS covers the same code if the user wants it fixed. |
| **R12** | Dependency cycle check runs outside the transaction (44 % failure rate) | **RE-ASSESS** | Not audited by any brief. Upstream reworked mutation into `with_write_transaction` with app-level retry (`sqlite.rs:2285-2290`), which plausibly changes this. | Out of scope. **Do not** state "fixed" without an audit; add to the follow-up list. |
| **R13** | `waits-for` cycles creatable with zero concurrency (pure logic bug) | **RE-ASSESS** | Not audited. | Out of scope; follow-up audit. |
| **R14** | The test pyramid is not executed; CI cannot build the tree | **PARTIAL — materially improved** | `cargo build --locked` **passes at baseline** (exit 0) **[V]**, and **P3-01**'s public git rev removes the unpublished-path-dep blocker that made the fork structurally unbuildable in CI. Residual: **four baselined red suites carrying 266 + 3 + 4 + 23 failures plus 4 process-aborting tests** (§0.1), and **85 first-party clippy errors** — the clippy gate is blocked by upstream's own lint debt, **not** by a third-party crate (that earlier claim was a stale-toolchain artifact). | **P3-01**; **G-locked is part of "all" and therefore runs on every commit** (§1, and see the four commits where it is load-bearing); **P3-13** makes the Org path actually covered; **G-aux** stops fuzz/benches rotting silently; **P4-06** writes an honest CI gate against the committed lint histogram. Never let any failure-name set grow. |
| **R15** | Conformance-to-Go: skipped equals passed, no version pin, divergence silenced | **OUT-OF-SCOPE** | Requires the Go `bd` binary; plan §7.3 excludes it. | **P3-13 Class A**: pin all 6 `conformance*.rs` files to JSONL (`bd` has no Org format). Do not attempt to make them green. |
| **R16** | Performance: auto-flush rewrites the whole file after every mutation | **FIXED-UPSTREAM for JSONL; REINTRODUCED for Org by this port** | Upstream's incremental line-splice auto-flush (`:11073-11585`) is the fast path; **P3-07** declines it for `.org`. [R1 §2.5], [R2 §3.7] | **Accepted regression, §9 U6.** Pin `tests/repro_auto_flush_inefficiency.rs` to JSONL (Class A) rather than weakening it. Record the acceptance in the CLEANUP doc. |
| **R17** | Per-connection performance pragmas revert on every reopen | **RE-ASSESS** (likely addressed) | Upstream's open path has a three-way `apply_runtime_pragmas` / `apply_runtime_compatible_schema` / `apply_schema` decision (`sqlite.rs:2294-2311`). Not audited against the dossier's specific claim. | Out of scope; follow-up. |
| **R18** | `obr list \| head` panics and exits 134 (SIGPIPE) | **RE-ASSESS** | Not audited. | Out of scope, but it is a two-second Phase-7 smoke: `obr list \| head -1; echo $?`. |
| **R19** | Import is non-transactional; comment ids collide across issues | **PARTIAL** | Import now runs inside `with_write_transaction` with FK restore (`sync/mod.rs:12614-12646`) — the transactionality half is addressed. Comment-id collision was not audited. | Out of scope. **P3-02 constraint:** the Org emitter must not introduce a comment-id renumbering pass (it would break emission purity and the parallel export path). |
| **R20** | Import bypasses dependency validation; remapping can mint self-blocking edges | **PARTIAL / RE-ASSESS** | `restore_foreign_keys_after_import` (`:12646`) now validates FKs after the `PRAGMA foreign_keys = OFF` bulk window. Semantic dependency validation was not audited. | Out of scope; relevant to **P5-03** if `--rename-prefix` runs — check the dep graph after the re-prefix. |
| **R21** | Duplicate ids in the import file resolve by last-LINE-wins | **FIXED-UPSTREAM for JSONL; CARRIES-FORWARD for Org** | Every JSONL reader now **errors** on duplicate ids (`:9036-9043`, `:11784-11791`, `:12119-12126`). The fork's `count_issues_in_org` does not, and 100 headings sharing one `:ID:` import silently, last-one-wins. [R1 §6.1], [R2 §3.8/B9] | **Mandatory parity in P3-05 + P3-06**, plus the doctor branch in **P3-12** (`doctor.rs:5201`). Without it Org is strictly less safe than the format it replaces. |
| **R22** | Tombstone/deletion semantics do not converge | **PARTIAL / RE-ASSESS** | `determine_action` has tombstone protection; tombstones are excluded from the content-hash map (`:12228`); `ImportResult.tombstone_skipped` exists. Convergence was not audited. | Out of scope. **G-org-roundtrip must include a tombstone** in its fixture set so the Org path at least preserves them. |
| **R23** | Org round-trip is lossy and is not a fixpoint | **CARRIES-FORWARD** | Measured in the fork: 9.4 % of descriptions lose content on pass 1, 1.06 % change again on pass 2, convergence at generation 3. Root cause: 1 of 23 `Element` variants handled (`Paragraph`), 21 dropped, plus an `i > 0` separator artifact. [R2 §1.13, §4/B6] | **§9 U2** decides. Recommended (a): preserve-by-raw-slice via `char_begin`/`char_end` (~40 lines, now possible at the pinned rev). Determines **G-org-roundtrip assertion 3** (`G1 == G2` vs `G2 == G3`). |
| **R24** | `--robot` broken, exit codes collide, no envelope | **RE-ASSESS** (likely improved) | Upstream added `RobotDocs`, a structured warning envelope (`cli/commands/mod.rs:86-133`), and `doctor_subsystems/exit_codes.rs`. Not audited. | Out of scope, but **G-rename part 4** runs `obr robot-docs guide`, so it must at least execute. |
| **R25** | `obr doctor` cannot see the failures that matter, exits 0 on all of them | **SUPERSEDED / RE-ASSESS** | 663 commits (v0.2.0–v0.2.8) were almost entirely `br doctor`: a detector/fixer registry, `FixerFilter`, refuse-gates, fingerprinted recovery artifacts, a fixture suite, `doctor_subsystems/` (8 modules). The fork-era description does not apply. [R1 §1.3] | **P3-12** must not *reduce* what doctor sees in an Org workspace. Beyond that, out of scope. |
| **R26** | Structure: a god object whose file size defeats its own invariant | **CARRIES-FORWARD — worse** | `sqlite.rs` 5,157 → 34,865 lines; `sync/mod.rs` 5,139 → 21,609 **[V]**. | Out of scope. Consequence for the port: **keep Org changes minimal and obviously separable**, because the subsystem you are integrating with is 10 days old and its last three releases were emergency fixes to it. |
| **R27** | Large dead surface | **CARRIES-FORWARD** | `src/cache.rs` (641 lines, S3-FIFO) has **zero callers** anywhere in `src/`. [R1 §3.3] | **CLEANUP**: delete it or annotate it. Do not silently inherit it as "used". |
| **R28** | A git-committable `.beads/redirect` steers every read and write, unvalidated | **RE-ASSESS — the fork-era description no longer applies** | `routing::read_redirect` (`src/config/routing.rs:189-247`) now has a size cap (`MAX_REDIRECT_BYTES`), UTF-8 validation, an empty-file warning, and relative resolution against the `.beads` dir itself; `follow_redirects` (`:256`) has loop detection and a depth bound (10 from the discovery caller). **Whether the redirect *target* is validated (absolute paths, `..`, symlinks) was NOT verified.** [R1 §4.4] | Out of the port's scope, but **do not repeat the dossier's wording**. Flag the target-validation question as a follow-up audit. `.beads/redirect` → `.obr/redirect` is AUTO from rename row A9. |
| **R29** | `obr agents` rewrites files outside the repo and destroys them five ways | **RE-ASSESS** | Not audited. | Out of scope **except** where Phase 6 touches it: rename row **A17** must add a legacy-read for `<!-- br-agent-instructions-v* -->` or `obr agents --update` cannot find and replace an old block — which would *add* a destruction mode. |
| **R30** | The parser of the durable artifact is unversioned, unbuildable by anyone else, and the pinned build differs from the local one | **CARRIES-FORWARD — fixed by this port** | `59521f9` is pushed to the **public** `github.com/jwiegley/org2jsonl` (`api.github.com/repos/...` → 200, `"private": false`; anonymous `ls-remote` succeeds); pinning it collapses the two-different-parsers-both-called-0.1.0 divergence toward the one people actually use. [R2 §5.1] | **P3-01** (public git rev pin, full 40-hex) + **P3-02** should stamp parser identity into the file: one `#+OBR_PARSER: org2jsonl@<short-rev>` header line plus a doctor warning when the file's stamp differs from the binary's compiled-in rev [R2 §2.4]. Today an `issues.org` written by one build and read by another silently produces different issues, and the degradation lands as a legitimate-looking git diff. |
| **R31** | Import is all-or-nothing with no location information, and has an O(n²) blowup | **CARRIES-FORWARD** | One `:ID:`-less heading aborts the whole parse (`fork:org_bridge.rs:742-748`, propagated with `?` at `:347`), bricking every command in the workspace with no line number. 100 KB of adversarial `[[` costs 12.7 s per invocation; the cost is inside `orgize`'s link scanning. [R2 §4/B2, B3] | **P3-06**: cache the parse so three passes are not 3×. **P3-12**: keep `check_jsonl_oversized` (`doctor.rs:5143`) active for Org as a size cap. **§9 Q-LINES** decides abort vs skip-with-warning vs abort-with-line-number (now possible: `OrgEntry` carries `line_begin`/`line_end` at the pinned rev, and one ~8-line public function in a repo the user owns would expose it). |
| **R32** | The backup mechanism is inert on the agent-mutation path; `history restore` is a path-traversal primitive | **RE-ASSESS / PARTIAL** | `.br_history/` is now a real 1,465-line subsystem (`src/sync/history.rs`) with a `sync.history_enabled` flag (#293), nanosecond timestamps, collision indices, per-backup `.meta.json` sidecars, and a doctor detector for unbounded growth; `backup_before_export_snapshot` is called from the export path at `sync/mod.rs:9801`. **The `history restore` path-traversal claim was NOT re-verified.** [R1 §7.9], [R2 §3.12] | **P3-11** must make backups extension-aware without reintroducing inertness (assert a backup is actually created on an Org flush). Flag the restore path-traversal question as a follow-up audit. Rename rows A7/A8 move the directories. |

**Summary — all 32 risks, each in exactly one bucket.** *(An earlier summary bucketed only 26
labels across 32 rows and silently omitted R11, R15, R19, R20, R22, R24, R26 and R27, so a reader of
the summary alone would have believed eight risks were dropped. The 32 table rows themselves were
and are complete and correctly titled, verified against DOSSIER.md's 32 `### R<n>` headings.)*

| Bucket | Count | Risks | What the port does |
|---|---|---|---|
| **Fixed upstream, wholly** | 6 | R1, R2, R3, R5, R7, R8 | nothing — and specifically, **do not re-port the fork's fixes** (R5 in particular would be re-fixing a fixed bug) |
| **Fixed upstream for JSONL only; the Org path needs work** | 2 | **R16** (incremental auto-flush — *reintroduced for Org by deliberate trade*, §9 U6, P3-07), **R21** (duplicate ids — *carries forward for Org*; mandatory parity in P3-05/P3-06/P3-12) | the two rows that are green in one format and red in the other; neither may be reported as simply "fixed" |
| **Partial — a named residual survives** | 7 | R4, R6, R14, R19, R20, R22, R32 | R4/R6 → §9 **D-RESIDUALS**; R14 → gates; R19 → a P3-02 constraint (no comment-id renumbering); R20 → check the dep graph if P5-03 runs; R22 → a tombstone in the G-org-roundtrip fixture set; R32 → P3-11 must not reintroduce inertness. R20/R22/R32 are **also** partly RE-ASSESS: the unaudited half of each is a follow-up. |
| **Carries forward; the port must act** | 4 | R9 (colon labels, §9 U3, P3-02), R23 (round-trip fidelity, §9 U2, P3-02), R30 (parser supply chain, P3-01 + P3-02 + §6.1 step 9), R31 (import recovery + O(n²), §9 Q-LINES, P3-06/P3-12) | these four *are* the Org half of the port |
| **Carries forward; out of scope — record only** | 3 | R11 (wall-clock LWW, same root cause as R4-residual), R26 (god objects — `sqlite.rs` 34,865 lines, `sync/mod.rs` 21,609), R27 (`src/cache.rs`, 641 dead lines) | R27 is CLEANUP's; R11 and R26 go in `docs/KNOWN_RESIDUALS.md`. R26 has a live consequence: **keep Org changes minimal and obviously separable.** |
| **Out of scope by plan mandate** | 1 | R15 (conformance-to-Go) | plan §7.3 excludes it; **P3-13 Class A** pins all 6 `conformance*.rs` to JSONL |
| **Re-assess — the fork-era description no longer applies and no brief closed it** | 9 | R10, R12, R13, R17, R18, R24, R25, R28, R29 | **do not repeat the dossier's wording for any of these.** Two have concrete Phase-7 hooks: **R18** is §6.1 step 8, **R24** is exercised by G-rename part 4's `robot-docs guide`. **R29** has a hard requirement: map row **A17**'s legacy marker read, or `agents --update` gains a destruction mode. The rest are follow-up audits listed in `docs/KNOWN_RESIDUALS.md`. |

6 + 2 + 7 + 4 + 3 + 1 + 9 = **32**.

---

## 9. User decisions — surfaced, **not resolved** — and open technical questions

Get answers before the commit named in each row. Recommendations are the briefs'; they are not
decisions.

**Coverage check against the governing plan §8's four ASK-THE-USER items:** §8.1 (license) →
**D-LICENSE**; §8.2 (`obr.db`, `.obr/history/`) → **D-ARTIFACTNAMES**, newly added below;
§8.3 (landing) → **D-LANDING**, newly added below; §8.4 (upstream root adoptions) → **D-PLUGIN** +
**D-DELETIONS** (which now enumerates `.agent-mail.yaml`, `.rchignore`, `.ubsignore` and `custom.db`
explicitly, instead of folding them silently into P4-01's command line). All four are present.

### 9.1 Blocking Phase 3

| ID | Decision | Options | Brief's recommendation | Blocks |
|---|---|---|---|---|
| **U1** | `agent_context` in Org | (a) carry as a `** Agent Context` level-2 child with a JSON src block, stored verbatim (~30 lines); (b) refuse Org export when any issue has one. **A drawer property is not an option** — `sanitize_property_value` flattens newlines into spaces, producing lossy-but-still-parsing JSON. | (a) carry | P3-02 |
| **U2** | Description fidelity — **the decision that determines whether R23/B6 is fixed or inherited** | (a) preserve-by-raw-slice using `char_begin`/`char_end` whenever a body contains any non-`Paragraph` element (~40 lines; converts 21 silent drops into 0; likely restores the one-cycle fixpoint); (b) explicitly reject descriptions containing structure; (c) keep the 21-of-23-element drop and merely document it | (a) | P3-02, and G-org-roundtrip assertion 3 |
| **U3** | Fix R9 (colon labels) now or defer | fix (~25 lines, `:LABELS:` drawer property) vs defer. It currently **breaks a shipped upstream feature** (capability dependencies via `provides:<cap>`). | fix, in the same commit as the emitter | P3-02 |
| **U-DRAFT** | Which Org keyword for the new `Status::Draft` | `DRAFT`, `SOMEDAY`, other. **User-visible**: it goes into the emitted `#+SEQ_TODO:` line and every `issues.org`, and into `ORG_TODO_KEYWORDS`. | `DRAFT` — unambiguous, not a standard keyword with conflicting meaning, keeps the legacy-alias table trivial | P3-02 |
| **U5** | Timing of `:BEADS_SCHEMA_VERSION:` → `:OBR_SCHEMA_VERSION:` | Phase 3 (one format, one migration) vs Phase 6 (with the rest of the rename, requiring a dual-key reader for everything written in between). This is a **file-format** change, not cosmetic. | Phase 3 — nothing reads the key today and the accept-both reader is two match arms either way | P3-02 |
| **U-EXTERNAL** | Should `BEADS_JSONL` / `--allow-external-jsonl` accept a `.org` path at all? | `validate_external_jsonl_path` (`src/sync/path.rs:679-690`) currently hard-rejects anything not ending `.jsonl`. Allowing it widens the safety boundary; refusing it means an Org workspace cannot use the external-export escape hatch. | **Accept `.org`, changing nothing else in the function.** *(This row previously offered no recommendation while blocking P3-03, which §0.3 says must land before anything Org is testable — an unanswerable blocker.)* Rationale: every **other** check in `validate_external_jsonl_path` (symlink rejection, parent-directory checks, traversal) is extension-independent and stays; the extension test is the *only* thing being widened, and after P3-09 `.org` is the **default** format — refusing it means the escape hatch works for the legacy format and not the current one, which is the wrong asymmetry to ship. **Counter-consideration the user should weigh:** an external `.org` path is a file the tool will *parse with orgize*, and R31's O(n²) blowup on adversarial input applies to attacker-supplied paths in a way it does not apply to JSONL. Mitigation: keep `check_jsonl_oversized` (`doctor.rs:5143`) active for Org (already required by P3-12) and enforce the same size cap on the external path. | P3-03 |
| **U-MERGEART** | Do `beads.base/left/right.jsonl`, `sync_base.jsonl`, `deletions.jsonl`, `interactions.jsonl` stay JSONL inside an Org workspace? | They are internal, never hand-edited, and `sync --merge` plus a whole doctor detector/fixer family depend on their names and format. Keeping them JSONL is far cheaper but makes the workspace mixed-format, which arguably contradicts "Org-mode is the fork's reason to exist". | keep JSONL (P3-08 assumes this) | P3-08 |
| **U4** | `init` seeding | `issues.org` only (fork behaviour; breaks the documented `bv`/beads_viewer contract that an `issues.jsonl` exists) vs seed both. **Do not seed both with content** — discovery would always pick `.org` and the `.jsonl` would rot. | `issues.org` only, documented in the README | P3-09 |
| **U-TESTPOLICY** | The rule for 160 test files and ~130 doctor fixture scripts | "all existing tests pin JSONL explicitly + a new parallel Org suite covers the default path" vs "convert the sync/e2e core to Org, pin the rest". [R2 §6.2] proposes a per-**function** Class A/B/C rule; confirm it. | the Class A/B/C rule as written in P3-13 | P3-13 (but shapes every commit's done-criteria) |
| **U6** | Accept the Org perf regression from disabling incremental auto-flush? | Every mutating command in an Org workspace does a full re-export. The alternative — a block-splicing incremental Org flush — is a separate project. | accept | P3-07 |
| **Q-LINES** | Per-heading import recovery for B2 | (a) abort (current; safest for a sync format, since a partial import that then flushes would delete the skipped issues); (b) skip-with-warning (makes hand-editing survivable); (c) abort **with a line number and the offending heading title** — now possible via one ~8-line addition to org2jsonl, a repo the user owns | needs whoever owns the human-editing workflow | P3-06 (message shape), P3-02 |
| **D-RESIDUALS** | Fix the two residual **upstream** defects during the port? | R4-residual (LWW + DB-wins repair silently reverts hand-resolved git merges; no DL-2b guard) and R6-residual (unconditional content-hash dedup with non-deterministic tie-break) are now upstream's bugs, not the fork's. Fixing means carrying local deltas in the hottest file in the tree; not fixing means knowingly shipping data-loss paths. | *(genuine maintainer call; no recommendation)* | — (would add commits after P3-13) |

### 9.2 Blocking Phase 4

| ID | Decision | Notes |
|---|---|---|
| **D-LICENSE** | **plan §8.1.** Upstream's LICENSE is MIT **plus an OpenAI/Anthropic Rider** (commit `b91c42b5`, `CHANGELOG.md:661`). The rider defines Restricted Parties as OpenAI, Anthropic, their Affiliates, and anyone "acting directly or indirectly on behalf of, for the benefit of, or under the direction of" them including contractors and agents; grants them no rights; forbids providing the Software or any Derivative Work to or for them; defines "use" to include benchmarking, testing, analyzing, and incorporation into any ML pipeline; requires that "any distribution of the Software or any Derivative Works must include this rider provision unmodified"; and auto-terminates on breach. **New timing datum:** the rider landed in v0.1.15 (2026-02-22), **one week after the fork point** — so the fork's BSD-3 relicense replaced *plain* MIT, and the fork has never carried the rider. Forward-porting means adopting a term the fork has never been subject to. Because the port branch contains upstream's code, `b3415739`'s literal action (delete LICENSE, ship only BSD-3) is not available. **Raised neutrally, not answered:** this port is being executed with Claude Code (Anthropic); whether an AI coding agent operated by the licensee is a "Restricted Party … acting under the direction of" Anthropic, and whether this work constitutes "analyzing"/"testing" the Software, is an interpretive legal question that may warrant counsel before the branch is landed or published. **Whichever way it lands, four packaging license sites are wrong today in both trees** (see P4-08). |
| **D-HOOKS** | `.githooks/` vs `lefthook.yml` — fold upstream's `br doctor --quick` guard into lefthook and delete `.githooks/`, or keep `.githooks/` renamed and drop the lefthook doctor command. Shipping both risks a `core.hooksPath` / lefthook-install conflict. |
| **D-AUDIT** | `.cargo/audit.toml` — re-apply fork `fcbbce29`'s deletion, or keep upstream's file and make CI's advisory step blocking (currently `continue-on-error: true`, so the file cannot affect CI either way)? **Recommend the latter.** |
| **D-SHFMT** | tabs (shfmt default, the fork's choice) or `-i 4` (what upstream's 164 scripts already look like, smaller diff)? And bulk one-shot, scoped pass, or converge-via-lefthook only? **Recommend: converge only, or scope to `install.sh` + `scripts/*.sh`.** |
| **D-PLUGIN** | `.claude-plugin/` (declares `"name": "beads"`, author Dicklesworthstone, an official marketplace), `.claude/skills/br/` (32 KB of `br`/`.beads` prose, the plugin's payload), `skills/bd-to-br-migration/` (8 files, a Go-`bd`→Rust-`br` guide). Adopt de-branded (implies maintaining an obr Claude plugin + marketplace) or drop? **`skills/bd-to-br-migration/` is unambiguously droppable; `.claude/skills/br/` → `.claude/skills/obr/` is cheap.** |
| **D-SUPPLYCHAIN** | Adopting upstream's `.github/action-pins.jsonl` + `action-pin-upstreams.jsonl` + `scripts/verify-workflow-action-pins.sh` + `docs/CI_SUPPLY_CHAIN.md` implies maintaining sha-pinned actions going forward. **Recommended, but a standing commitment.** |
| **D-ACFS** | `.github/workflows/notify-acfs.yml` notifies `Dicklesworthstone/agentic_coding_flywheel_setup` about `install.sh` changes. The fork renamed `TOOL_NAME` to `obr` but kept pointing at upstream's registry — almost certainly wrong. **Recommend drop.** |
| **D-CHANGELOG** | `CHANGELOG.md` (69 KB, 341 `beads` hits; fork has none) — keep verbatim as labelled upstream history with a new obr section on top, truncate, or delete. **Recommend keep-and-exclude**; the G-rename allowlist already excludes it (L8). |

### 9.3 Blocking Phases 5 and 6

| ID | Decision | Notes |
|---|---|---|
| **D-PREFIX** | The fork's own tracker issue-ID prefix. All 549 issues are `beads_rust-*`. (i) re-prefix via `obr sync --import-only --rebuild --rename-prefix` — mechanism **verified to exist**, dependency refs verified to be remapped, but **whether IDs embedded in prose are rewritten is NOT verified**; or (ii) keep historical IDs and allowlist `.obr/issues.org` (G-rename row L9). **Recommend (i) with a verified dry-run on a copy, falling back to (ii).** **Two halves, and the second is not optional under either branch:** (1) the *historical* IDs in `issues.org` — that is what (i)/(ii) decides; (2) **the `issue_prefix` / `issue-prefix` keys in `.beads/config.yaml`, both literally `beads_rust` [V]**, which mint *future* IDs and which `--rename-prefix` does not touch. Even under (ii), key (2) must be de-branded or every issue created after the port is minted `beads_rust-<n>` again and G-rename part 3b fails on the config file. [R4 §5.8] called this "the very first thing the G-rename audit will trip over"; map row A14b now covers it. Owned by **P5-03** (if it runs) or **P6-01a** (if it does not). |
| **D-SELFUPDATE** | `obr upgrade` points at `Dicklesworthstone/beads_rust` releases and `self_update` is **on by default** (`Cargo.toml:167` `default = ["self_update"]` **[V]**). After the rename: (a) point at `jwiegley/obr` (requires publishing minisign-signed releases via `release.yml`), or (b) disable — drop `self_update` from default features and delete `upgrade`, `packaging/`, `install.sh`. **(b) removes ~13 files and ~230 branding hits and matches an unpublished personal fork. Recommend (b).** **⚠ Interaction nobody flagged: `tests/e2e_schema.rs:918` marks `agent_baseline_snapshots_match_current_binary` `#[cfg(feature = "self_update")]` **[V]**. Choosing (b) COMPILES P6-05's entire gate out of existence, and the test run still reports green because a cfg'd-out test is simply absent.** If (b) is chosen, P6-05 must either move that test out from under the cfg or run under `--features self_update`. Say which, in the commit message. |
| **D-DOMAINSEP** | Rename the 9 branded hash domain separators? They are hasher inputs only — the strings never reach disk, only their digests do. Renaming invalidates startup caches once (they are caches) and breaks digest verification of an **in-flight** `sync --merge` receipt. Keeping them violates the letter of the mandate invisibly. **Low stakes either way. Do not bump the `.vN` suffixes.** |
| **D-MONOREPO** | Carry the `_beads` monorepo variant forward as `_obr` at all? It is accepted but never created (`src/config/mod.rs:55-57`). Keeping it doubles every directory-name check for a feature the fork may not need. |
| **D-GITIGNORE** | Root `.gitignore:142` already has `.beads/recovery*/`. After rename row A8 the recovery dir *is* `recovery/`, so the glob must be tightened to an exact path or it silently swallows siblings. Cosmetic but easy to get wrong. |
| **D-DELETIONS** | The bulk deletions (P4-01 + P6-00), **now enumerated in full** — the earlier list silently omitted four items that P4-01's command line already deletes, which is exactly how a deletion gets made without a decision. **P4-01:** `temp_test/`, `temp_test_2/`, `custom.db` (tracked-but-ignored in **both** trees **[V]**), `sample_beads_db_files/` (~178 MB), `refactor/`, `.ntm/`, **`.rchignore`** (upstream's `rch` tool), **`.ubsignore`** (**[V]** tracked, contains `legacy_beads/` and `.beads/` — its sibling `.rchignore` was in the list and it was not; map row A19), **`.agent-mail.yaml`** (plan §8.4; makes an `obr` checkout announce itself as upstream's project; already in `.gitignore:125` yet tracked), `.claude/`, `.claude-plugin/`, `skills/` (all three → **D-PLUGIN**). **P6-00:** `docs/porting/` (3 files, 342 hits), `docs/audit_bd_to_br_2026_05_09.md`, `tests/artifacts/perf/` (335 captures). **P6-01a:** `.obr/interactions.jsonl` (3 synthetic upstream test records **[V]**). Together ~450 files and ~4,400 branding hits *before any renaming happens*. This is a judgement about how much of upstream's historical apparatus the fork wants to carry. |
| **D-UPSTREAMTRACKER** | Plan Phase 5 says to copy the fork's 549 issues onto the branch, which **implicitly discards upstream's own 953-issue tracker**. That is probably right but is not stated as a decision anywhere — and upstream's issue corpus is a useful source of context on the new subsystems (`witness.rs`, `db_inode_lock.rs`, `doctor_subsystems/`). The same decision covers `.beads/interactions.jsonl`, which is tracked in **both** trees. |
| **D-ARTIFACTNAMES** | **Governing plan §8.2, verbatim: "confirm `obr.db` (vs keeping `beads.db` hidden inside `.obr/`), and `.obr/history/` vs `.obr_history/`." This was an ASK-THE-USER item and it has NOT been answered — it was re-framed into map rows A3/A7 and correction C9 with recommendations attached, and re-framing a question is not answering it. [R4 §7.7/§7.8] explicitly labelled both as RECOMMENDATIONS.** What is settled (VERIFIED, so the question is narrower than the plan thought): `.br_history` is created at `beads_dir.join(".br_history")` (`src/sync/history.rs:502` **[V]**) and `.br_recovery` at `src/config/mod.rs:66` **[V]** — both are *already inside* the workspace dir, and moving either outside it is not viable because `src/sync/path.rs`'s "refuse to write outside `.beads`" allowlist would reject it. So the real question is only **the name inside `.obr/`**. Recommendations (still just recommendations): `obr.db`; `.obr/history/`; `.obr/recovery/`. **Blocks P6-01a.** |
| **D-LANDING** | **Governing plan §8.3, verbatim: "Landing: replace `main` (force-push; clean history: upstream + port series) vs merge commit vs long-lived branch." This is the highest-stakes decision in the project and it was absent from this section entirely.** It does not block any commit in §2–§6, but it must be answered **before Phase 8** and it changes what CLEANUP should do: a force-push replacement wants a clean, self-contained series and an honest `docs/KNOWN_RESIDUALS.md`; a merge wants the fork's 20 commits to remain reachable; a long-lived branch wants the branch name and CI wiring settled. Plan §5 Phase 8 says explicitly **"Do not land."** — present options, do not choose. |

### 9.4 Open **technical** questions — nobody's decision, but nobody has closed them either

These are not user decisions; they are facts nobody could establish read-only. **This document
previously had no open-questions section at all, so unclosed technical questions simply vanished
between the briefs and the plan.** Each row names who can close it and how cheaply. Close them in
the commit that consumes them, and record the answer here.

| ID | Question | Why it matters | How to close | Consumed by |
|---|---|---|---|---|
| **Q-TAGHYPHEN** *(was [R2 §9 Q1])* | **Does `orgize 0.10.0-alpha.10` accept `-` in Org tags?** Emacs' `org-tag-re` excludes `-`; the fork's hyphenated labels *appear* to survive, but that was never tested. **[I]** | This decides whether the `:LABELS:` drawer property (§9 U3) is a **nicety or a necessity**. If orgize drops or splits hyphenated tags, then tags alone cannot represent the label set at all and `:LABELS:` is mandatory regardless of how U3 resolves. | A 5-line test against the pinned rev: emit an issue with labels `["a-b", "c"]`, parse it back, assert both survive. Do it **before** writing the emitter. | **P3-02** |
| **Q-WITNESS** *(was [R2 §9 Q2])* | **What exactly does the `sync --witness` CLI surface gate on?** R2 confirmed `witness.rs` itself is byte-oriented and format-agnostic, but read only the three call sites (`src/cli/commands/sync.rs:1994, 2045, 2086`) — not the surrounding flag handling, nor `src/cli/mod.rs:2915`'s stated contract. | P3-10 must **reject** `--witness` on a `.org` target. If the flag is reachable by some path other than those three call sites (e.g. via `--status`, or a witness recorded as a side effect of a normal flush), the rejection is incomplete and an Org workspace can still produce correct-but-meaningless chunk digests. | Read `src/cli/commands/sync.rs` around each call site plus the `SyncArgs` group/conflict declarations in `src/cli/mod.rs:2860-2930`; grep `witness` across `src/`. | **P3-10** |
| **Q-LINTDEBT** | Fix the **85 first-party clippy errors** (§0.1.1) during the port, or gate against the committed histogram? | Fixing them makes `-D warnings` a real hard gate in CI (P4-06's natural end state). Not fixing them means CI can only assert "no worse", forever. The 85 sit in the hottest files in the tree, so fixing them conflicts directly with R26's "keep Org changes minimal and obviously separable". | Read `baseline/v3-clippy-lints-baseline.txt`; count how many are in `sync/mod.rs` / `sqlite.rs` / `doctor.rs`. If most are elsewhere, a separate pre-Phase-6 commit is cheap. | **P4-06** |
| **Q-FUZZWS** *(was R3 open q. 5)* | Does `cargo fuzz build` work with upstream's `fuzz/` arrangement, which has **no `[workspace]` table** (the fork's does)? | If it does not, P4-05 must add the table, which is a manifest change that G-locked must then tolerate. | One `cargo fuzz build` on the branch **before** adding the two `[[bin]]` stanzas. | **P4-05** |
| **Q-BINDGEN** *(was R3 open q. 6)* | Can `nativeBuildInputs = [ rustPlatform.bindgenHook ]` and `buildInputs = [ sqlite ]` really be dropped now that the engine is pure-Rust `fsqlite` (§10.C6)? | Dropping them shrinks the flake and removes a C toolchain dependency; keeping them is harmless but misleading. | One `nix build` with them removed. | **P4-02** |
| **Q-SCHEMAIDS** *(was R4 open q. 2)* | Of the **26** versioned `br.*.vN` JSON schema identifiers, only 6 are known to be read back from disk; the remaining ~20 are **INFERRED write-only**. | Map row M1 renames the write-only ones HARD. If any of the ~20 is actually read back, a HARD rename orphans on-disk artifacts silently. | `git grep -n 'br\.[a-z_.]*\.v[0-9]'` and, for each, check whether the literal appears in a *deserialization* path as well as a serialization one. | **P6-01a** |
| **Q-CLAPENV** *(was R4 open q. 4)* | With clap 4.6, what happens when **both** the legacy `BR_*` and the new `OBR_*` env var are set, given `env =` accepts exactly one name and map row E12 adds a manual pre-`Cli::parse` copy? | Wrong precedence means a user who sets the new name gets the old value, silently, with no warning. | Write the E12 test first (it is one of the twenty in `tests/legacy_compat.rs`), assert `OBR_*` wins. | **P6-01a** |
| **Q-SOCKET** *(was R4 open q. 5)* | Is `bd.sock` (map row A10) vestigial? It appears only as a `.gitignore` entry in the `init` template (`init.rs:240`) and in the tracked `.beads/.gitignore` **[V]**. | If nothing creates it, the row should be a deletion, not a rename. | `git grep -n 'bd\.sock\|\.sock' src/` — if the only hits are the two gitignore templates, delete the line. | **P6-01a** |
| **Q-PROSEIDS** | Does `--rename-prefix` rewrite issue IDs embedded in **prose** (title, description, design, notes, acceptance_criteria, comment text)? Dependency refs are verified to be remapped; prose is not. | Decides D-PREFIX (i) vs (ii) and therefore whether allowlist row L9 is needed. | A dry-run of `obr sync --import-only --rebuild --rename-prefix` on a **copy** of the 549-issue corpus, then `grep -c beads_rust-` on the result. | **P5-03** |

---

## 10. Corrections to the original plan

Consolidated from all four briefs plus this session's verification. **Read this before trusting
`UPSTREAM_SYNC_AND_RENAME_PLAN.md`.**

**C1 — §6.3's status map is wrong on two counts.** The plan's example says
"in_progress→STRT/NEXT, deferred→WAIT". The actual map (`03c5cd2c`) is **`InProgress→DOING`** (never
STRT or NEXT) and **`Deferred→DEFER`**; **`WAIT` is `Blocked`**, not `Deferred`. Full 8-row table
with the legacy import aliases in §2/P3-02. The plan's own instruction to read the diff rather than
trust the line was correct. **And the map is now incomplete** — `Status::Draft` is new upstream and
unmapped. [R1, R2]

**C2 — §6.2 describes the property drawer as three items.** It is **29 properties in a fixed
emission order**, three with non-obvious guards (`COMPACTION_LEVEL` only when `> 0`;
`EPHEMERAL`/`PINNED`/`IS_TEMPLATE` only when true) and one (`ISSUE_TYPE`) emitted **unsanitized**.
[R2 §1.7]

**C3 — §7.5 is half wrong about where the JSONL assumptions live.** It warns that "witness.rs,
db_inode_lock.rs may assume JSONL in its safety checks". **`db_inode_lock.rs` contains zero JSONL
references** — it only ever locks the database inode. `witness.rs` *is* line-oriented but sits on no
safety path (only `sync --witness`). The **actual** JSONL-assuming safety layer, which the plan never
names, is: (a) `src/sync/path.rs` `ALLOWED_EXTENSIONS` `:85-94` + `validate_extension_and_name`
`:512-544` + `is_allowed_jsonl_temp_name` `:546-559` + `validate_external_jsonl_path` `:679-690`;
(b) the temp-name generator `sync/mod.rs:1818`; (c) the entire incremental line-flush machinery
`sync/mod.rs:11073-11620`; (d) a hand-copied allowlist at `tests/e2e_sync_git_safety.rs:1274-1283`.
All verified **[V]** this session. [R1 §8.2]

**C4 — §6.5 understates doctor and overstates stats.** Upstream's `stats` needs **one line**
(`src/cli/commands/stats.rs:92` **[V]**) because `compute_recent_activity` (`:644-656`) already
takes a path and computes a cached repo-relative git pathspec — the fork's `a0c54428` stats fix is
**superseded**. Upstream's `doctor` needs ~12 sites. [R2 §C3]

**C5 — the plan (via CLAUDE.md) assumes `#![forbid(unsafe_code)]`. It is false upstream.**
`Cargo.toml [lints.rust] unsafe_code = "deny"` with one sanctioned `#[allow(unsafe_code)]` in
`sync::db_inode_lock` (`src/lib.rs:20-27`, GitHub #412). **Re-imposing `forbid` will not compile.**
Also `rust-version` moved 1.85 → 1.88. [R3 §7.2]

**C6 — the plan never mentions that upstream replaced `rusqlite` with `fsqlite` (frankensqlite), a
pure-Rust SQLite.** `Cargo.toml:43-59` lists `fsqlite` + 14 `fsqlite-*` crates at 0.1.18; zero
`rusqlite` in `Cargo.lock`. This belongs in §3's fact table and §7's landmines: it invalidates
`aa129a2c`'s patch text, changes the row-access API across all 34,865 lines of `sqlite.rs`
(`conn.query_row(sql) -> Row`, `row.get(n) -> Option<&SqliteValue>`), changes concurrency
assumptions (`sqlite.rs:2285-2290`: the busy handler hot-spins, so there is app-level retry plus a
`.write.lock` flock), and likely removes the need for `bindgenHook`/bundled-SQLite plumbing in the
flake. **Good news for the port: `fork:src/sync/org_bridge.rs` has zero `rusqlite` references**
**[V]**, so the Org module is unaffected. [R3 §1.4, §7.1]

**C7 — §7.1 misdescribes the org2jsonl pin.** "Fork's `flake.nix:22` pins … at rev `5ea7586`" is
wrong: `fork:flake.nix:21-24` contains **no rev**, only `url = "git+file:///Users/johnw/src/org2jsonl"`.
The rev lives in `fork:flake.lock`. The prescribed remediation is right; the description of current
state is not, and the difference matters because a bare `git+file://` URL re-resolves on any
`nix flake update`. [R3 §7.3]

**C8 — §7.1 and Phase 3 say to "mirror how the fork pins `toon_rust` (`Cargo.toml:54`)". Upstream
retired that pattern.** Upstream has **zero git dependencies**: `toon_rust` is the crates.io package
`tru 0.2.3` (`Cargo.toml:92`) and `rich_rust` is crates.io `0.2.2` (`:89`). There is no in-tree
exemplar. Also, upstream's crane-based flake needs **no `outputHashes`** (crane vendors git deps
from `Cargo.lock`), unlike the fork's `rustPlatform.buildRustPackage`, which does. [R2 §C4]

**C9 — the `.br_history/` rename row is mis-framed, and `.br_recovery/` is missing entirely.**
*(Read with §9 **D-ARTIFACTNAMES**: this correction narrows plan §8.2's question but **does not
answer it**. Plan §8.2 is an ASK-THE-USER item; a recommendation is not a decision.)*
`.br_history` is **already** created inside the workspace dir (`beads_dir.join(".br_history")`,
`src/sync/history.rs:502`), and so is `.br_recovery` (`src/config/mod.rs:66` **[V]**). Both of the
plan's options describe the same location; the real choice is the *name inside* `.obr/`. Moving
history outside `.obr/` is not viable — `src/sync/path.rs`'s "refuse to write outside `.beads`"
allowlist would reject it. And `.br_recovery/` (32 files, 107 hits, its own gitignore entry, a
doctor age-check, `.rchignore` rules) appears nowhere in the plan. [R4 §10.2, §10.4]

**C10 — the `:BEADS_SCHEMA_VERSION:` value is stale.** `CURRENT_SCHEMA_VERSION` is **17**
(`src/storage/schema.rs:11` **[V]**), not 1. The 1→17 migration ladder also means a fork
`beads.db` will be migrated on first open by the new binary — a Phase-5 note. [R1 §8.9]

**C11 — the plan's rename map has no row for** the 26 versioned `br.*.vN` JSON schema identifiers,
the `beads://` MCP URI scheme (12 URIs, 115 hits), the `"beads_dir"` JSON output key (12 emit
sites), branded doctor check names and failure-mode IDs, `bd.sock`, `.bv.lock`,
`~/.cache/beads/startup`, `.beads.snapshot_*`, `beads.{base,left,right}.jsonl`, `beads.jsonl`
legacy, the `<!-- br-agent-instructions-vN -->` markers written into user files, the `BD_<KEY>`
dynamic env namespace, the **`"br"` default issue-ID prefix** (not `"bd"`), or the 9 hash domain
separators. §5.1 above supplies all of them. [R4 §10.6]

**C12 — dossier R9's suggested fix is incomplete and partly harmful.** "Escape or reject colons in
the Org tag emission" would break the `provides:<cap>` capability wire format that upstream still
builds and parses (`sqlite.rs:16187`, `:16214`). The fix must be **lossless** — a canonical
`:LABELS:` drawer property. [R2 §C10]

**C13 — dossier R4's observability complaint is stale.** `ImportResult` (`sync/mod.rs:2350-2380`)
carries `skipped_count`, `tombstone_skipped`, `orphans_removed`, `orphan_cleaned_count`,
`labels_imported`, `dependencies_imported`, `comments_imported`, `export_hashes_recorded`,
`blocked_cache_entries`, `child_counter_entries`. The structural LWW problem remains; the counter
complaint does not. [R1 §8.4]

**C14 — dossier R6's fix is half-done, and the done half is not where the dossier pointed.**
Upstream did **not** add `ORDER BY` to `find_by_content_hash` (still absent, `sqlite.rs:17063`); it
swapped collision phases 2 and 3 so id-match precedes hash-match. Anyone auditing only
`find_by_content_hash` would wrongly conclude R6 is entirely untouched. [R1 §8.5]

**C15 — dossier R5's central claim is obsolete against upstream.** The fixed temp filename is gone
(pid + `create_new(true)` + retry + symlink rejection + CAS publication). "Re-applying the fork's
temp-file fix" would be re-fixing a fixed bug. The **inverse** hazard is real: upstream's fix
hardcodes `jsonl` in the extension. [R1 §8.3]

**C16 — the R1 brief's own premise, that `CHANGELOG.md` can carry the commit clustering, is
wrong.** The changelog has **no section at all** for v0.1.34–v0.1.45, v0.2.0–v0.2.7,
v0.2.12–v0.2.18, or v0.2.20 — roughly 1,000 of the 1,843 commits. Its preamble says it is
"organized by capability rather than diff order": a curated narrative, not an audit trail. [R1 §8.7]

**C17 — §3's new-command list overstates.** `audit`, `changelog`, `delete` and `defer` **all existed
at the fork point**. The exact new set is **eight**: `Capabilities`, `Capacity`, `Coordination`,
`Gate`, `RobotDocs`, `Scheduler`, `Serve`, `VcsStatus`. The `Commands` enum went 39 → 47. [R1 §8.1]

**C18 — §6.1's "format selection: chosen per-file by extension" is accurate about the fork but
misleading about upstream.** There is **no format-abstraction seam of any kind** upstream — no
trait, no enum, no dispatch point. `grep -rn "org_bridge|is_org_format|orgize|org2jsonl" src/`
returns zero hits; the only `*Format` enums are `OutputFormat`, `OutputFormatBasic` and
`GitObjectFormat`, all about terminal output. The word `jsonl` appears **1,310 times** in `src/`
across **17 distinct `Issue`↔file (de)serialization sites**. Every touchpoint is a fresh `if`; the
fork's 4-probe version badly understates the size. [R1 §2.6, R2 §C6]

**C19 — Phase 3's commit list omits four upstream subsystems that did not exist at the fork
point**: the line-splice incremental auto-flush (breaks loudly), the canonical content-hash
normalizer (breaks loudly on the first flush), the byte-copy 3-way-merge base anchor (breaks
quietly on every merge), and reviewed-additive reconcile (needs explicit rejection). Each now has
its own commit: P3-07, P3-04, P3-08, P3-10. [R2 §C5]

**C20 — `agent_baseline/` is a rename-blocking golden-snapshot dependency the plan never
mentions.** `tests/e2e_schema.rs:920-928` + `scripts/verify-agent-contracts.sh:30`. Phase 6 must
`git mv` three `br_*.txt` goldens, edit the path strings in the test, and regenerate with
`UPDATE_AGENT_BASELINE=1`. `README_first_80_lines.md` is also a golden, so the Phase-8 README
rewrite invalidates it a second time. Now commit **P6-05**. [R3 §7.5]

**C21 — Phase 4's "merge fuzz target sets" and "re-run shfmt" both understate traps.** Re-applying
`4cbf6348`'s `.gitignore` hunk verbatim adds `fuzz/corpus/`, but upstream **tracks** 2,747 corpus
files there — apply the commit minus one line. And shfmt was a 17-file operation on the fork; on
the new base it is ~160 files across 164 tracked `.sh`, including ~130
`tests/doctor_fixtures/{assert,corrupt}.sh` consumed by the doctor harness. [R3 §7.6]

**C22 — Phase 4's "keep lefthook (upstream has none)" is correct but omits `.githooks/`**, a second
overlapping pre-commit mechanism upstream added. Shipping both is a footgun. [R3 §7.7]

**C23 — §7.6's "man pages" row has no referent.** Neither tree contains any man pages (no
`clap_mangen`, no roff, no `.1` generation). Completions are runtime-generated via
`obr completions <shell>` and derive from the clap command name, so renaming
`#[command(name)]` fixes them automatically. [R3 §7.8, R4 §10.11]

**C24 — §3's neutral "Upstream DOES have (own versions) flake.nix" understates reality.** Upstream's
flake is byte-identical to the fork point except a version bump, has no `flake.lock`, and no
upstream doc/workflow/script references nix at all. Phase 4's open question resolves definitively:
**the fork's flake is the base; upstream's contributes nothing.** [R3 §7.9]

**C25 — the `.beads/*.md` tracker docs are UPSTREAM's, not the fork's.** `SYNC_SAFETY_INVARIANTS.md`,
`SYNC_THREAT_MODEL.md`, `SYNC_CLI_FLAG_SEMANTICS.md`, `MCP_AGENT_MAIL_PATTERNS.md`, `.beads/README.md`
all existed at the fork point and the fork never touched them; upstream has since advanced three
(blob-hash proof in [R4 §6.2]). In the forward-port they arrive from `upstream/main` for free, in
their **newer** form. Plan §7.4 and Phase 5 imply otherwise.

**C26 — the G-rename gate as written cannot work.** `git grep -inE '(beads|\bbd\b)'` uses POSIX ERE,
where `\b` is a literal `b`. Must be `-P`. Confirmed empirically: `git grep -Ec '\bbeads\b'`
returned 0 files, `-P` returned 549. §1 G-rename supplies a working four-part gate plus a 10-row
allowlist. [R4 §10.1]

**C27 — several plan line references are the FORK's, not upstream's.** Plan §6.1 and Phase 3 step 3
cite `src/config/mod.rs:62` and `:100-116`. Upstream's constants are at `:60-64`, `discover_jsonl`
is at `:147-162`, `is_beads_dir_name` at `:55-57`, `RECOVERY_DIR_NAME` at `:66` — all **[V]** this
session. [R4 §10.7]

**C28 — `fork:CLAUDE.md:7` is stale** ("Binary name: `br` (beads_rust)") while `fork:Cargo.toml:13`
says `name = "obr"`. And **the fork's own rename was binary-only**: `fork:Cargo.toml:2` still says
`name = "beads_rust"` **[V]**, `fork:flake.nix:53,97,186,199,212` still say `pname = "beads_rust"`,
`packaging/scoop/br.json` was never renamed, and no `.beads`/env/config-path surface was touched.
[R4 §6.5, §10.9]

**C29 — a plan claim I re-verified as CORRECT, recorded so nobody redoes it:** 108 `src/*.rs` files;
`src/sync/mod.rs` 21,609 lines; `src/storage/sqlite.rs` 34,865; 1,843 commits `e805fe98..upstream/main`;
upstream still crate `beads_rust` / binary `br` / `.beads`+`_beads` / `issues.jsonl` default;
`fork:src/config/mod.rs:33` is indeed `DEFAULT_JSONL_FILENAME = "issues.org"` **[V]**; and
`temp_test/`, `temp_test_2/`, `custom.db` really do exist tracked-but-ignored in **both** trees.

**C30 — plan §5 Phase 0 step 1 says "`nix develop` (or ensure nightly per `rust-toolchain.toml`)".
Neither works.** The fork's dev shell pins nightly 1.95.0 (2026-02-17), too old for `sysinfo
0.39.6`'s `cfg_select`; upstream's flake devShell is broken on Darwin twice over (`flake.nix:110-112`
references removed `darwin.apple_sdk.frameworks.*` stubs, and crane's `vendorCargoDeps` cannot find
`Cargo.lock` because `combinedSrc` nests sources under `beads_rust/`). Both are recorded in
`BASELINE.md`. Use the canonical fenix `nix shell` pin (§0.1.0) until **P4-02** fixes the dev shell,
and treat "`nix develop --command cargo build` succeeds" as a P4-02 **deliverable**, not a formality.

**C31 — `cargo clippy` and `cargo fmt` are unusable on this host and the plan uses both.** Plan
§7.8's G-lint says `cargo clippy --all-targets -- -D warnings`; plan §5 Phase 0 step 3 logs the same
command. `$CARGO_HOME/bin/cargo-clippy` and `cargo-fmt` are rustup shims that resolve **before**
PATH and dispatch to a stale Feb-2026 nightly, so the space-separated form silently lints a
different toolchain. **Always invoke the `cargo-clippy` / `cargo-fmt` binaries directly, inside the
nix shell.** This propagates into P4-04's `lefthook.yml` and P4-06's workflows, both of which the
fork wrote with the space form.

**C32 — plan §5 Phase 0 step 3's baseline is not reproducible as written, and its first execution
produced wrong numbers.** `cargo test --lib --bins` run in parallel is killed at exit status 5 by
four `cli::commands::doctor::tests::execute_repair_indexes_*` tests, truncating the run. The first
baseline pass recorded **9** unit failures; a serial run with those four skipped records **266**
(`baseline/v3-unit-failures-complete.txt`). Any gate that diffs against the 9-name list will either
hide a real regression behind the abort or, if the abort point shifts, surface ~257 pre-existing
failures as "new". §1 G-unit's two-tier procedure is the fix.

**C33 — plan §5 Phase 4 says "keep CLAUDE.md (updated)". There is nothing to update.** Upstream has
no `CLAUDE.md` (`git ls-files | grep -i '^CLAUDE'` is empty **[V]**); it is fork commit `22adc72b`,
75 lines, one file **[V]** — and it was the **only one of the fork's 20 §3.1 commits with no home in
this document's commit series**. It is now **created** by CLEANUP (§6.2), de-branded from the start,
with a fact-by-fact correction table.

**C34 — the plan's own §5 Phase 7 was never made concrete, and this document made that worse by
declaring it "unchanged from the original plan".** Plan §5 Phase 7 asks for six things; only two of
them (full gates, rename audit) were owned by anything. The clone-simulation import cycle, the
"legacy `.beads` workspaces still open with a warning" check, BASELINE.md's mandated serial G-unit,
R2 §9 Q5's write-path sweep, and §8's own R18 SIGPIPE smoke had no owner at all. §6.1 now supplies
ten numbered, copy-pasteable steps with pass criteria.

**C35 — plan §5 Phase 5's "copy `main`'s `.beads/issues.jsonl` (+ `config.yaml`, `metadata.json`)"
is wrong on two of the three files.** `metadata.json` is **byte-identical** in `main` and
`upstream/main` (`{"database":"beads.db","jsonl_export":"issues.jsonl"}` **[V]**), so copying it
achieves nothing — and *keeping* that `jsonl_export` value after P3-09 makes it an **explicit
JSONL override** (`resolve_jsonl_path` priority 2), which is why P5-02 must rewrite it explicitly or
the tracker silently stays JSONL. `config.yaml` should **not** be copied either: the fork's is 41
lines with ~20 branded comments, upstream's is 4 lines with the same two prefix keys **[V]**.
Only `issues.jsonl` is actually carried. See P5-01/P5-02.

---

## 11. What was verified directly

Two passes. §11 below is revision 1's (the Phase-2 synthesis); **§11.1 is revision 2's** (the
completeness repair). Both read-only: no builds anywhere, no writes to the upstream worktree, no
writes outside this file. Rows marked **SUPERSEDED** in §11 were correct observations of the wrong
run and are retained so the provenance of the discarded numbers is legible.

### 11.0 Revision 1 (Phase-2 synthesis)

Read-only. No builds anywhere; no writes outside this file.

| What | How | Result |
|---|---|---|
| All four recon briefs | `Read`, in full (1,196 + 937 + 438 + 678 lines) | consumed |
| The governing plan | `Read`, in full | consumed |
| ~~Phase-0 baseline~~ **SUPERSEDED** | read the v1 scratchpad logs `summary.txt`, `*-failures-baseline.txt`, `clippy.log`, `unit.log`, … | **These were the wrong run.** The v1 pass used an ambient toolchain and a parallel test run that aborts. Superseded by §11.1 row 1 and by `BASELINE.md`. Kept here as a record of how the 9-failure figure arose. |
| ~~The clippy blocker is third-party~~ **SUPERSEDED / FALSE** | `grep -oE '^error.*' clippy.log \| sort \| uniq -c` on the v1 log | 28 × `E0658 cfg_select` in `sysinfo` — **true only of the stale toolchain.** Under the canonical fenix pin `sysinfo` compiles and clippy reaches first-party code: **85 first-party errors**. See §0.1.1 and §10.C31. |
| `sysinfo`'s provenance | `Cargo.lock:258` + reverse-dep scan | `asupersync` ← `fsqlite-*`/`fastmcp-*`; **not** a direct dep |
| The git-safety violation is pre-existing | read the failing test's stdout in `e2e_sync_git_safety.log` | `CREATED: .beads/.br-jsonl-write-<hex>.lock` — closes R1 open question 2 |
| `jsonl_import_export` failures are environmental | read the failing test's stdout | macOS `/tmp` symlink rejection |
| Export/import anchors | `grep -n` + `sed -n` on `src/sync/mod.rs` for 20 named functions | all within ±2 lines of the briefs' citations |
| The Org header insertion point | `sed -n '9884,9900p' src/sync/mod.rs` | right after `let mut hasher = Sha256::new();` |
| `ALLOWED_EXTENSIONS`, temp-name grammar, external-path gate | `sed -n` on `src/sync/path.rs:85-95, 546-560, 679-692` | verbatim as reported; no `org` |
| The canonical hash normalizer | `sed -n` on `path.rs:2274-2302` and `sync/mod.rs:12769-12812` | identical logic in both; **`compute_snapshot_content_sha256` takes no path** — an extra detail R2 did not state |
| That `path` is in scope at the hash call site | `sed -n '2390,2420p' src/sync/path.rs` | yes, inside `capture_jsonl_source_snapshot` |
| Config constants | `sed -n '55,80p' src/config/mod.rs` | `:60/:62/:64/:66` and the 6 `EXCLUDED_JSONL_FILES` exactly as reported |
| `Status::Draft` | `sed -n '55,72p' src/model/mod.rs` | present, between `Deferred` and `Closed` |
| `CURRENT_SCHEMA_VERSION` | `src/storage/schema.rs:11` | `17` |
| The `.jsonl`-only gates in `vcs.rs`, `sync.rs`, `stats.rs` | `sed -n` on each | `vcs.rs:259`, `sync.rs:1553`, `stats.rs:92` — all as reported |
| `validate_sync_paths` runs on the **resolved** path for every sync | read `sync.rs:1522-1546` | confirmed — the extension gate must land in P3-03, not later |
| The fork's Org module has no `rusqlite` | `grep -rn rusqlite fork:src/sync/org_bridge.rs` | **none** — closes R3 open question 4 |
| Fork constants and deps | `sed -n '28,40p' fork:src/config/mod.rs`; `grep -n` on `fork:Cargo.toml` | `DEFAULT_JSONL_FILENAME = "issues.org"` at `:33`; `org2jsonl = { path = "../org2jsonl" }` at `:64`; package still `beads_rust` at `:2`, bin `obr` at `:13` |
| Dossier risk inventory | `grep -n '^### R[0-9]' docs/research/DOSSIER.md` | 32 headings, R1–R32, all mapped in §8 |

### 11.1 Additionally verified for revision 2

Read-only. No builds anywhere, no writes to the upstream worktree, no writes outside this file.

| What | How | Result |
|---|---|---|
| The durable Phase-0 record | `Read docs/research/upgrade/BASELINE.md`; `cat` all six `baseline/v3-*.txt` | 266 unit failures (`wc -l v3-unit-failures-complete.txt` = 266), 4 aborters, 85 first-party clippy errors, e2e 3/4/23; canonical fenix pin; `cargo clippy`/`cargo fmt` shim hazard; both dev shells broken |
| Fork tracker metadata | `git show main:.beads/metadata.json` | `{"database":"beads.db","jsonl_export":"issues.jsonl"}` — **byte-identical to upstream's** (`cat .beads/metadata.json`). B2 confirmed. |
| Fork vs upstream `.beads/config.yaml` | `git show main:.beads/config.yaml`; `cat .beads/config.yaml` | fork 41 lines with ~20 branded comments; upstream 4 lines. **Both** carry `issue_prefix`/`issue-prefix` = `beads_rust`. |
| The repository's own `.beads/` inventory | `git -C <upstream worktree> ls-files .beads`; `git ls-tree -r --name-only main -- .beads` | upstream **10** tracked files; fork 15 (the 5 extra are `.br_history/issues.*.jsonl`). B3 confirmed. |
| Branding in the five `.beads/*.md` | `grep -oi beads`, `grep -oPi '(^\|[^a-z])(br\|bd)([^a-z]\|$)'` per file | SYNC_CLI_FLAG_SEMANTICS 15+50, SYNC_THREAT_MODEL 18+15, README 20+9, SYNC_SAFETY_INVARIANTS 16+11, MCP_AGENT_MAIL_PATTERNS 0+20 |
| `.beads/interactions.jsonl` | `cut -c1-200 .beads/interactions.jsonl` | 3 synthetic records, `actor: ubuntu`, `"test prompt"` — not real data |
| `.beads/.gitignore` body | `cat` | `bd.sock`, `beads.{base,left,right}.jsonl` + `.meta.json`, `.br_history/`, `.bv.lock`, legacy `bd.db` |
| `issues.jsonl` surface in `src/` | `grep -rn 'issues\.jsonl' src --include='*.rs' \| wc -l`; per-file counts | **515 hits / 25 files**; doctor.rs 142, sync/mod.rs 98, config/mod.rs 57, path.rs 56, stats.rs 25, sync.rs 21, cli/commands/history.rs 18, sync/history.rs 15, … |
| The six previously-unassigned anchors | `sed -n` on each | `cli/commands/history.rs:879`, `doctor_subsystems/surface.rs:413`, `mcp/mod.rs:120`, `cli/mod.rs:962`, `cli/mod.rs:2865`, `robot_docs.rs:17`, `capabilities.rs:703` — **all seven exist exactly as claimed** |
| `tests/legacy_compat.rs` | `ls tests/ \| grep -i legacy` | **absent**. And `git grep -in deprecat -- src/` = **0** — every LR row is new code. M3 confirmed. |
| `fuzz/` identity | `cat fuzz/Cargo.toml` | package `beads_rust-fuzz`; `beads_rust = { path = "..", default-features = false }`; 7 `[[bin]]` targets |
| `benches/` identity | `grep -n 'beads_rust' benches/*.rs` | `storage_perf.rs` 11 paths (`:22,23,92,628,654,673,674,895,956,998,1028`), 22 `beads`/`jsonl` hits; `benchmarks.rs` **0** |
| `fuzz/Cargo.lock` | `ls fuzz/Cargo.lock`; `grep -n fuzz .gitignore` | **does not exist**; `.gitignore:21` ignores it. M9's "fuzz/Cargo.lock path dependency" is a local-only concern. |
| `Cargo.lock` root entry | `grep -n 'name = "beads_rust"' Cargo.lock` | `:347` — P6-02 necessarily moves it, hence G-locked |
| The two `UPGRADE_LOG.md` files | `ls -la`, `diff -q`, `git log --oneline -3 --` on each | 6,355 B (2026-05-14, v0.2.10) vs 8,121 B (2026-01-18); **different**; root copy has 6 branded lines. M6(a) confirmed. |
| Root `.gitignore` branding | `grep -niE 'beads\|[^a-z]br[^a-z]\|bd\|bv' .gitignore` | 18 branded lines incl. `:34`, `:39-46`, `:131-142` (the `.beads/recovery*/` at `:142`), `:148-149`, `:167-168`, `:229`, `:232-234`. M6(b) confirmed. |
| `.ubsignore` | `git ls-files .rchignore .ubsignore .agent-mail.yaml custom.db`; `cat .ubsignore` | all four tracked; `.ubsignore` contains `legacy_beads/` and `.beads/`. M6(c) confirmed. |
| CLAUDE.md upstream | `git ls-files \| grep -i '^CLAUDE'` | **empty**. Fork `22adc72b` = 75 lines, 1 file. M7 confirmed. |
| `docs/ORIGINS.md` upstream | `ls docs/ORIGINS.md` | **absent** — created by Phase 8; the allowlist grep is inert until then |
| `agent_baseline` feature gate | `sed -n '910,930p' tests/e2e_schema.rs`; `grep -n -A6 '^\[features\]' Cargo.toml` | `:918` is `#[cfg(feature = "self_update")]`; `Cargo.toml:167` `default = ["self_update"]`. m1 confirmed. |
| Fixture counts | `find … \| wc -l` and `xargs grep -lI` per tree | doctor_fixtures 206 files / 171 with hits; tests/fixtures 205 / 47; tests/snapshots 97 / 35. m3's "presumably files-with-hits" — now stated explicitly. |
| Test-estate JSONL exposure | `grep -rl 'issues\.jsonl' tests --include='*.rs' \| wc -l`; `find tests -name '*.rs' \| wc -l` | **59 of 165** — P3-13's figure confirmed |
| R2's open questions | `awk '/^## 9\./,0' R2-org-port-spec.md` | **Q1–Q5 only; there is no Q6.** P3-10's `[R2 §Q6]` citation was dangling and is now `[R2 §3.19]`. Q1/Q2 were unsurfaced and are now §9.4 Q-TAGHYPHEN / Q-WITNESS; Q5 is §6.1 step 5. |
| Governing plan §5 Phase 7 and §8 | `sed -n '94,200p' UPSTREAM_SYNC_AND_RENAME_PLAN.md` | Phase 7's six obligations and §8's four ASK items transcribed verbatim into §6.1 and §9 |
| Upstream CI clippy steps | `sed -n '40,50p' .github/workflows/ci.yml` | `:44` `cargo clippy --all-targets --all-features`, `:47` `--no-default-features`, both **bare** — P4-06's premise confirmed |
| Upstream `Cargo.toml` identity | `sed -n '1,40p' Cargo.toml` | `name = "beads_rust"`, `version = "0.2.22"`, `rust-version = "1.88"`, `license = "MIT"`, `[[bin]] name = "br"` |
