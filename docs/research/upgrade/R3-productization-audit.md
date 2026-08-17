# R3 — Productization Audit

**Brief:** Phase 1 recon (R3) of `/Users/johnw/src/obr/UPSTREAM_SYNC_AND_RENAME_PLAN.md` §5.
**Written:** 2026-08-06. **Author:** read-only recon subagent.
**Fork:** `/Users/johnw/src/obr` @ `main` (fork point `e805fe98`, 2026-02-15).
**Upstream:** `/Users/johnw/src/obr-org-forward-port` = worktree at `upstream/main` = `d1fb0d3a` (beads_rust v0.2.22, 2026-08-06). Read-only; no builds were run there.

Evidence convention: `worktree-relative-path:line` means the **upstream** worktree unless prefixed `FORK:`. Every claim is marked **VERIFIED** (I read/ran/grepped it) or **INFERRED**.

---

## 0. Summary table

### 0.1 Fork productization commits — disposition

| Commit | Subject | Verdict | One-line reason |
|---|---|---|---|
| `aa129a2c` | fix(storage): propagate errors instead of silent fallbacks in SQLite layer | **SUPERSEDED** (all 3 sub-fixes; ⚠ 1 caveat) | Upstream independently made `parse_datetime` fallible and `IdGenerator::generate` fallible-closure; the `user_version` path was redesigned differently and *deliberately* still falls back. §1.1 |
| `f826ac4e` | build(nix): modernize flake with rust-overlay replacing crane+fenix | **STILL-NEEDED → SEMANTIC-MERGE** | Upstream's `flake.nix` is byte-identical to the fork point except a version bump; it is bit-rotted (crane+fenix, `toon_rust` GitHub input that no longer matches `Cargo.toml`, openssl, `stdenv.isDarwin`, `darwin.apple_sdk.frameworks`) and has **no `flake.lock`**. §2.1 |
| `1c32e5e5` | Add nix flake checks and extend dev shell | **STILL-NEEDED → SEMANTIC-MERGE** | No upstream equivalent. Rides on the `f826ac4e` rewrite; `license = licenses.bsd3` line is a §8.1 user decision. §2.1 |
| `4cbf6348` | Add lefthook pre-commit hooks | **STILL-NEEDED** (re-apply nearly as-is) | Zero `lefthook` references anywhere upstream. **BUT: drop the `fuzz/corpus/` .gitignore line** — upstream tracks 2,747 corpus files. §2.3 |
| `5312cb5a` | Add cargo-fuzz targets for input boundaries | **SEMANTIC-MERGE (merge target sets)** | Upstream has 7 far richer targets + corpora; fork's `fuzz_jsonl_parse` is subsumed, `fuzz_validation` is a genuine gap to keep, `fuzz_org_parse` is Org-port-mandatory. §2.2 |
| `5eb13fb7` | Format shell scripts with shfmt | **SEMANTIC-MERGE (re-run, scoped)** | The fork's reformatted files are worthless (upstream `install.sh` grew 40 KB→61 KB, 12 new scripts). 160 of 164 upstream `.sh` files are non-conforming; full reformat is a ~160-file diff touching `tests/doctor_fixtures/**`. Recommend scoping. §2.4 |
| `fcbbce29` | Remove unused .cargo/audit.toml | **STILL-NEEDED (trivially) — but re-examine the rationale** | Upstream's `.cargo/audit.toml` is byte-identical to the one the fork deleted, and `vergen-gix` is still a dependency, so the RUSTSEC-2025-0140 suppression is still live. The commit message's premise is only half true. §2.5 |
| `b3415739` | Replace MIT license with BSD 3-Clause | **USER DECISION (§8.1) — do not decide** | Upstream's LICENSE is now MIT **+ an OpenAI/Anthropic Rider** with explicit derivative-work and unmodified-propagation clauses. Also: the fork's own relicense was *incomplete* (packaging still says MIT). §3 |
| `a2ba738e` | Rewrite README in maintainer voice | **DEFERRED to Phase 8** (plan is right) — with two corrections | The fork README is already factually stale (documents `.beads/issues.jsonl`, not `issues.org`) and contains **no attribution to beads/beads_rust at all**. §4 |
| `6a0518d0` | Update CI workflows for obr rename and tighten gates | **SPLIT: rename→Phase 6; gate-tightening→STILL-NEEDED** | Upstream never added `-- --deny warnings` to either clippy step, and never tightened the 10% bench threshold. §2.6 |
| `33bb2e82` | Add org2jsonl flake input (listed under Org-core but touches `flake.nix`) | **STILL-NEEDED, folds into the flake merge** | §2.1 |

### 0.2 Upstream root artifacts — disposition

| Artifact | Size / files | New since fork? | Beads branding? | Disposition |
|---|---|---|---|---|
| `.claude-plugin/` | 8 KB, 2 files | **yes** | heavy (`"name": "beads"`, `displayName: "Beads (br)"`, author Dicklesworthstone, MIT, marketplace URLs) | **DROP** |
| `.claude/skills/br/` | 32 KB, 5 files | **yes** | heavy (skill literally named `br`) | **DROP** (or adapt only if you want an `obr` agent skill) |
| `skills/bd-to-br-migration/` | 52 KB, 8 files | no (existed at fork point) | total — the skill's *subject* is bd→br migration | **DROP** |
| `.agent-mail.yaml` | 62 B | **yes** | none (opaque `project_uid`) | **DROP** — and note it is already in upstream `.gitignore:125` yet still tracked |
| `.ntm/` | 12 KB, 3 log files | no | filenames `*-beads_rust.log` | **DROP** (log droppings) |
| `refactor/` | 268 KB, 63 files | **yes** | yes (session artifacts referencing br/beads) | **DROP** |
| `sample_beads_db_files/` | **178 MB**, 32 files | **yes** | yes (dir name, `beads.db` filenames, real third-party issue data) | **DROP** — nothing in `src/ tests/ scripts/ .github/ docs/` references it |
| `.githooks/` | 8 KB, 2 files | **yes** | yes (`br doctor`, `.beads/`, `BR_DOCTOR_SKIP_PRECOMMIT`) | **ADAPT** — fold its *intent* into `lefthook.yml` as one extra command |
| `agent_baseline/` | 204 KB, 18 files | no (16 files at fork point) | yes, and **load-bearing** | **ADOPT-RENAMED, MANDATORY** — `tests/e2e_schema.rs:920` asserts CLI output matches these goldens; the rename breaks them |
| `temp_test/`, `temp_test_2/`, `custom.db` | 20 K + 12 K + 172 K | no (in **both** trees) | yes | **DROP** in the Phase-0.5 cleanup commit |
| `.cargo/audit.toml` | 558 B | no (fork deleted it) | none | see §2.5 |
| `.gitattributes` | 23 B (`/fuzz/corpus/** binary`) | **yes** | none | **ADOPT** (needed if you keep the corpora) |
| `.rchignore` | 1.4 KB | **yes** | none directly | **DROP** (tooling for upstream's `rch` sync workflow) |
| `.ubsignore` | 295 B | no (in both) | mentions `legacy_beads/` | keep as-is / de-brand in Phase 6 |
| `CHANGELOG.md` | 69 KB | **yes** (fork has none) | heavy | **ADOPT-RENAMED** with a truncation/preamble strategy — §5.3 |
| `.github/action-pins.jsonl`, `action-pin-upstreams.jsonl`, `scripts/verify-workflow-action-pins.sh`, `docs/CI_SUPPLY_CHAIN.md` | — | **yes** | none | **ADOPT** — real upstream productization the fork lacks |
| `.github/workflows/doctor.yml` | 164 lines | **yes** | yes (`br doctor`, `.beads/`, `BR_NO_AUTOFLUSH`) | **ADOPT-RENAMED** |
| `packaging/` (aur, homebrew, scoop) | 16 KB, 4 files | no | yes + **stale MIT** in all three | **ADOPT-RENAMED**; license fields blocked on §8.1 |

---

## 1. `aa129a2c` — silent-fallback fixes vs upstream's 34,865-line `sqlite.rs`

The fork's commit had three independent sub-fixes. I traced each to its upstream analogue. **VERIFIED** by reading both trees.

### 1.1 Sub-fix 1 — `PRAGMA user_version` `unwrap_or(0)` → `?`

**Fork's change** (`FORK: git show aa129a2c`, `src/storage/sqlite.rs:104-107`):

```rust
-            .query_row("PRAGMA user_version", [], |row| row.get(0))
-            .unwrap_or(0);
+            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
```

**Upstream today** — the open path was rewritten around a *deliberately fallible-tolerant* two-source read (`src/storage/sqlite.rs:2294-2311`):

```rust
        let schema_current = connection_user_version(&conn)
            .or_else(|| database_header_user_version(path))
            .is_some_and(|version| version >= u32::try_from(CURRENT_SCHEMA_VERSION).unwrap_or(0));
        let runtime_compatible = runtime_schema_compatible(&conn);

        if schema_current && runtime_compatible {
            crate::storage::schema::apply_runtime_pragmas(&conn)?;
        } else if runtime_compatible {
            apply_runtime_compatible_schema(&conn)?;
        } else {
            apply_schema(&conn)?;
        }
```

`connection_user_version` returns `Option<u32>` and swallows the pragma error with `.ok()?` (`src/storage/sqlite.rs:15688-15693`). `database_header_user_version` also returns `Option`.

**Verdict: SUPERSEDED-BY-DIFFERENT-DESIGN, with a caveat.** The *literal* fallback the fork removed is gone (there is no longer an `unwrap_or(0)` on a pragma read), and upstream added strictly more machinery: a WAL-aware pragma read that beats the raw header peek (documented at `src/storage/sqlite.rs:15679-15687`, "issue #373"), a raw-header fallback, a three-way schema decision (`apply_runtime_pragmas` / `apply_runtime_compatible_schema` / `apply_schema`), and a *strict* accessor `SqliteStorage::schema_user_version()` that returns `Result` and errors with `"Could not read PRAGMA user_version for reconciliation provenance"` (`src/storage/sqlite.rs:2733-2739`).

**Caveat (INFERRED, worth a decision in Phase 4):** if *both* the pragma read and the header peek fail, `schema_current` is `false` and upstream re-applies schema — exactly the outcome the fork's commit message called out ("silently re-applying schema"). Upstream's inline comment at `src/storage/sqlite.rs:2293-2299` shows this is intentional ("a database behind `CURRENT_SCHEMA_VERSION` is migrated in place (legacy fleets depend on this)"). **Recommendation: do NOT re-apply the fork's strictness here.** It would regress upstream's WAL-tail handling and the legacy auto-migration contract, and it is now covered by an explicit `br doctor migrate-schema` lifecycle. If you want the fork's paranoia back, the correct shape is a `doctor` detector, not a hard error in `open()`.

### 1.2 Sub-fix 2 — `parse_datetime` falling back to `Utc::now()`

**Fork's change:** made `parse_datetime` return `rusqlite::Result<DateTime<Utc>>`, removed the `warn!` + `Utc::now()` fallback, and threaded `?` / `.transpose()?` through 12 call sites.

**Upstream today** (`src/storage/sqlite.rs:16284-16303`):

```rust
fn parse_datetime(s: &str) -> Result<DateTime<Utc>> {
    if s.is_empty() {
        // NULL/empty datetime columns (common when migrating from bd/Go beads)
        // default to epoch rather than crashing the import.
        return Ok(DateTime::<Utc>::UNIX_EPOCH);
    }
    if let Some(dt) = parse_canonical_utc_datetime(s) { return Ok(dt); }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) { return Ok(dt.with_timezone(&Utc)); }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") { return Ok(Utc.from_utc_datetime(&naive)); }
    Err(BeadsError::Config(format!("unparseable datetime: {s}")))
}
```

**Verdict: SUPERSEDED, strictly better.** Upstream reached the same conclusion (fallible, no `Utc::now()` fabrication) and then went further:

- a hand-rolled fast path `parse_canonical_utc_datetime` (`:16306-16358`) — a performance win the fork does not have;
- `parse_datetime_value(Option<&SqliteValue>) -> Result<DateTime<Utc>>` (`:16387-16397`) which correctly decodes INTEGER/REAL/NULL/BLOB storage classes. Its doc comment (`:16378-16386`) documents a *worse* bug than the one the fork fixed: `as_text().unwrap_or("")` → `parse_datetime` silently mapped integer-epoch datetimes to `UNIX_EPOCH`, "corrupting the value on export";
- `parse_opt_datetime_value` (`:16399-16409`) for the `Option<DateTime>` columns the fork handled with `.map(parse_datetime).transpose()?`;
- ~20 call sites all use `parse_datetime_value(row.get(N))?` (e.g. `:14955`, `:14957`, `:15025`, `:16591`);
- unit tests for every branch (`:31596-31793`), including `test_parse_datetime_garbage_returns_error` and `test_parse_datetime_value_rejects_blob`.

Semantic differences vs the fork worth knowing: **empty string is `Ok(UNIX_EPOCH)`, not an error**, and NULL/missing is `Ok(UNIX_EPOCH)` for the required-column helper. That is deliberate (`bd`/Go-beads migration compatibility). Do not "fix" it.

### 1.3 Sub-fix 3 — `id_exists(...).unwrap_or(false)` inside an infallible closure

**Fork's change:** pre-fetched `storage.get_all_ids()?` into a `HashSet<String>` before the loop in `import_from_jsonl`, because `IdGenerator::generate` took `F: Fn(&str) -> bool` (`FORK: src/util/id.rs:113-124`) and the call site had to swallow the DB error.

**Upstream today:** `IdGenerator::generate` is now itself fallible (`src/util/id.rs:216-227`):

```rust
    pub fn generate<F>(
        &self, title: &str, description: Option<&str>, creator: Option<&str>,
        created_at: DateTime<Utc>, issue_count: usize, mut exists: F,
    ) -> Result<String>
    where
        F: FnMut(&str) -> Result<bool>,
```

with a doc comment (`:210-215`) stating it returns `BeadsError::IdCollision` "instead of fabricating an unchecked ID". **And** the import path pre-fetches anyway: `build_prefix_renames` (`src/sync/mod.rs:12150-12183`) does `occupied_ids.extend(storage.get_all_ids()?)` at `:12165` and passes `|candidate| Ok(occupied_ids.contains(candidate) || generated_ids.contains(candidate))` at `:12174`. It is reached from `import_from_jsonl` at `src/sync/mod.rs:12587`.

**Verdict: SUPERSEDED, strictly better** — upstream applied *both* remedies (fallible closure type + prefetch) plus an explicit collision error.

I grepped every `id_exists` call site (`src/mcp/tools.rs:1624,1633`, `src/cli/commands/create.rs:629,640,645,672,680`, `src/cli/commands/mod.rs:144,158`, `src/cli/commands/show.rs:474`, `src/cli/commands/q.rs:82,91`, `src/cli/commands/sync.rs:5266`) — **none** uses `.unwrap_or(false)`. **VERIFIED.**

### 1.4 ⚠️ The big one: `rusqlite` is gone

**VERIFIED, and this is not in the plan.** Upstream replaced `rusqlite` with **`fsqlite` (frankensqlite), a pure-Rust SQLite** — `Cargo.toml:43-59` lists `fsqlite` plus 14 `fsqlite-*` transitive crates at `0.1.18`, and `grep -n 'name = "rusqlite"' Cargo.lock` returns nothing. The only `rusqlite` strings left in `src/` are inside a *test fixture string* in `src/util/markdown_import.rs:897,905,913` ("Constraints: no rusqlite; fsqlite only").

Consequences for R3 (and for R1/R2/Phase 3 generally):

- `aa129a2c`'s patch text is dead on arrival — it uses `rusqlite::Result`, `rusqlite::Error::FromSqlConversionFailure`, `rusqlite::types::Type::Text`, and the `conn.query_row(sql, params, |row| …)` 3-arg shape. Upstream's shape is `conn.query_row(sql) -> Result<Row>` with `row.get(n) -> Option<&SqliteValue>` and `SqliteValue::{Null,Text,Integer,Float,Blob}`.
- Any fork code (including `org_bridge.rs` if it touches storage types) that names `rusqlite` must be re-expressed against `fsqlite`.
- Concurrency semantics changed with it: `src/storage/sqlite.rs:2285-2290` notes "frankensqlite's busy handler hot-spins, so we rely on application-level retry (see `with_write_transaction`). The `.write.lock` flock serializes concurrent mutating processes."
- `src/lib.rs:20-27` + `Cargo.toml:[lints.rust]` downgraded `unsafe_code` from `forbid` to **`deny`**, with one sanctioned `#[allow(unsafe_code)]` in `sync::db_inode_lock` (GitHub #412). The fork still has `unsafe_code = "forbid"` (`FORK: Cargo.toml:104-105`) and `CLAUDE.md` still claims `#![forbid(unsafe_code)]`.

---

## 2. Infra pairs, compared concretely

### 2.1 `flake.nix` — fork vs upstream

**VERIFIED.** `git diff e805fe98:flake.nix upstream/main:flake.nix` produces exactly one hunk: `version = "0.1.9"` → `"0.1.20"`. Upstream's flake history is `d5e9821a` (add) → `419350f1` (version bump) → `e4a6fce0`, `4abf01a6` (incidental). **Upstream has done essentially nothing to its flake in 1,843 commits, and there is no `flake.lock` in the tree** (`ls flake.lock` → no such file). No upstream doc, workflow, or script references `nix develop` or `flake.nix` (grep over `AGENTS.md README.md .github/workflows/*.yml scripts/*.sh` → empty).

Upstream's flake is provably stale in at least six ways (**VERIFIED** against upstream's own `Cargo.toml`):

| Upstream `flake.nix` says | Reality at `d1fb0d3a` |
|---|---|
| `toon_rust` flake input from `github:Dicklesworthstone/toon_rust`, copied into `combinedSrc` because "Cargo.toml references `path = "../toon_rust"`" (`flake.nix:33-37,71-85`) | `Cargo.toml:92`: `toon_rust = { version = "0.2.3", package = "tru" }` — a crates.io dep. The whole `combinedSrc` contraption is dead weight. |
| `buildInputs = [ openssl ]; OPENSSL_NO_VENDOR = "1"` (`flake.nix:107-117`) | `self_update` is configured `rustls`-only (`Cargo.toml:112`) |
| `lib.optionals stdenv.isDarwin [ darwin.apple_sdk.frameworks.… ]` | `stdenv.isDarwin` is deprecated; `darwin.apple_sdk.frameworks` was removed from nixpkgs-unstable |
| `version = "0.1.20"` | package version is `0.2.22` |
| crane + fenix | the fork already migrated to `rust-overlay` + `makeRustPlatform` |
| no `flake.lock` | non-reproducible; `nix flake check` re-resolves every input |

**Verdict for `f826ac4e` + `1c32e5e5` + `33bb2e82`: STILL-NEEDED, applied as a SEMANTIC-MERGE — i.e. take the fork's flake as the base and re-derive the parts that reference the old dependency graph.** Concretely, the port's flake commit must:

1. Start from `FORK: flake.nix` (rust-overlay, `makeRustPlatform`, `apple-sdk_15`, `stdenv.hostPlatform.isDarwin`, `mkShell`-based devShell independent of the package build, the four `checks` outputs).
2. **Delete `cargoLock.outputHashes."toon_rust-0.1.1"`** (`FORK: flake.nix:63-66` and again at `:105-108`) — obsolete now that toon is `tru` from crates.io. Keep the `outputHashes` map only if some git dep survives.
3. **Delete `nativeBuildInputs = [ rustPlatform.bindgenHook ]` and `buildInputs = [ sqlite ]`** — those exist for `rusqlite`'s bundled C SQLite. `fsqlite` is pure Rust. (**INFERRED** from the dep swap; verify with one `nix build` in Phase 4.)
4. Keep the `org2jsonl` input + `postUnpack` copy from `33bb2e82`, but **pin it by explicit rev** in the flake URL (`git+file:///Users/johnw/src/org2jsonl?rev=…`) rather than relying on `flake.lock` alone. Plan §7.1 says "Fork's `flake.nix:22` pins … at rev `5ea7586`" — that is imprecise: `flake.nix:21-24` has **no rev**, only `url = "git+file:///Users/johnw/src/org2jsonl"`; the rev `5ea75860bdcca8ffe3989ae877b63328e34c6270` (`revCount: 15`) lives in `FORK: flake.lock`. The distinction matters because a bare `git+file://` URL re-resolves to whatever `main` points at whenever the lock is regenerated.
5. Bump `version` and `pname`/`mainProgram` per Phase 6.
6. Set `meta.license` per the §8.1 decision — the fork's `licenses.bsd3` (`FORK: flake.nix:88`, from `1c32e5e5`) is **not** a free choice on the new base.
7. Commit a `flake.lock`.

**Suggested commit split for Phase 4:** one commit "build(nix): replace crane+fenix flake with rust-overlay" (items 1-3, 5, 7) and one "build(nix): add flake checks + dev shell tooling" (the `checks` block and the lefthook/cargo-audit/shellcheck/shfmt devShell packages from `1c32e5e5`), so the checks commit can be reverted independently if `nix flake check` proves too slow on a 34 K-line `sqlite.rs`.

### 2.2 `fuzz/` — fork (3 targets) vs upstream (7 targets + 2,747 corpus files)

**VERIFIED.**

| Target | Fork | Upstream | Disposition |
|---|---|---|---|
| `fuzz_jsonl_parse` (fork) | `serde_json::from_str::<Issue>(data)` — 9 lines | — | **DROP.** Subsumed by upstream `jsonl_import`, which drives the real `import_from_jsonl` against a temp workspace with sentinel-preservation invariants (`fuzz/fuzz_targets/jsonl_import.rs:1-40`). |
| `fuzz_org_parse` (fork) | `org_bridge::org_text_to_issues(data)` | — | **KEEP, mandatory.** Rename to `org_parse` for house style. This is the fork's only fuzz coverage of its own new input boundary. Add a `fuzz/corpus/org_parse/` seeded from the Org round-trip test's outputs. |
| `fuzz_validation` (fork) | `is_valid_id_format`, `LabelValidator::validate`, `Status/Priority/IssueType::from_str` | — | **KEEP.** Genuine coverage gap upstream. All five APIs still exist and are public: `src/validation/mod.rs:451`, `:375,383`, `src/model/mod.rs:137,172,259`. **VERIFIED** — it should compile unmodified. |
| `config_yaml`, `content_hash`, `jsonl_import`, `markdown_import`, `merge_issue`, `search_issues`, `sync_cycle` | — | yes, with corpora | **ADOPT unchanged.** |

Mechanics of the merge:
- Upstream `fuzz/Cargo.toml` declares `beads_rust = { path = "..", default-features = false }` and has **no `[workspace]` table**; the fork's has `[workspace] members = ["."]`. Take upstream's file and append two `[[bin]]` stanzas (`org_parse`, `validation`) plus `test = false / doc = false / bench = false` to match house style.
- Upstream `fuzz/fuzz_targets/common.rs` is a shared helper module (`ByteCursor`) — the fork's targets don't use it; no conflict.
- Upstream's `fuzz/README.md` documents per-target `cargo fuzz run … -runs=N -max_len=M` invocations; add the two new targets there.
- `.gitattributes` (`/fuzz/corpus/** binary`) must be adopted alongside the corpora.

### 2.3 `lefthook.yml` (fork) vs `.githooks/` (upstream)

**VERIFIED.** They are orthogonal, not competing:

- **Fork `lefthook.yml`** (21 lines, `FORK: lefthook.yml`) runs six *developer-hygiene* checks in parallel on pre-commit: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- --deny warnings`, `cargo test --lib --bins`, `shellcheck {staged_files}`, `shfmt -d {staged_files}`, `nix flake check --no-warn-dirty`.
- **Upstream `.githooks/pre-commit`** (opt-in via `git config core.hooksPath .githooks`) runs exactly one *data-integrity* check: `br doctor --quick --json` against the repo's own `.beads/`, blocking the commit when `workspace_health` is non-healthy, with `BR_DOCTOR_SKIP_PRECOMMIT=1` as a bypass and fail-open on missing `br` / missing `.beads/`.

`grep -rn lefthook` over the entire upstream tree (excluding `target/`, `.git`) returns **nothing**.

**Verdict: `4cbf6348` STILL-NEEDED, re-apply nearly as-is**, with three adjustments:

1. **Do NOT re-apply the `fuzz/corpus/` line** of the commit's `.gitignore` hunk. Upstream tracks 2,747 files under `fuzz/corpus/` (`git ls-files fuzz/corpus | wc -l`). Upstream's `.gitignore` already has `fuzz/artifacts/` at line 62 and `fuzz/Cargo.lock` at line 21 with a comment explaining why. The fork's hunk is therefore a net-negative.
2. Fold `.githooks/pre-commit`'s intent in as a seventh lefthook command (e.g. `doctor: run: ./target/debug/obr doctor --quick --json || …`) and then **delete `.githooks/`** so there is one hook mechanism, not two. Alternatively keep `.githooks/` renamed and drop the lefthook doctor command — either is fine, but shipping both invites the `core.hooksPath` / lefthook-install conflict.
3. Re-check the `nix-check` command's runtime. On the fork it gated a 5 K-line `sqlite.rs`; on a 34 K-line one with `checks.tests = cargo test --lib --bins` in a sandbox it may be minutes. Consider moving `nix-check` to `pre-push`.

### 2.4 `shfmt` (`5eb13fb7`)

**VERIFIED with `shfmt` 's own output** (read-only `-l`, run against the upstream worktree; no files written):

- Upstream tracks **164** `.sh` files (`git ls-files '*.sh' | wc -l`).
- With shfmt defaults (tabs), **160** are non-conforming.
- With `-i 4` (matching upstream's actual house style — `install.sh`, `scripts/*.sh` all use 4-space indents), **155** are non-conforming; with `-i 4 -ci`, **152**.

So upstream shell code has never been shfmt-formatted under any setting, and the fork's tab-based reformat of 17 files is now stale in every one of them (upstream's `install.sh` is 61,370 bytes vs the fork's 40,327; `scripts/` gained 9 new shell scripts incl. `verify-agent-contracts.sh`, `audit-workflow-action-pins.sh`, `activate-dev-local-patch.sh`).

**Verdict: SEMANTIC-MERGE — re-run shfmt, but scope it.** Recommendation, in order of preference:

1. **Preferred (least risk):** do **not** bulk-reformat. Keep the `shfmt -d {staged_files}` lefthook command so only scripts you actually touch get formatted, and let the repo converge. This keeps the diff reviewable and avoids touching test fixtures.
2. If you want a one-shot pass, scope it to `install.sh` + `scripts/*.sh` (~14 files) and **explicitly exclude `tests/doctor_fixtures/**` (≈130 `assert.sh`/`corrupt.sh` files) and `skills/**`** (being dropped anyway). The doctor fixtures are consumed by the doctor test harness; reformatting them is pure risk for zero benefit. Add a `.shfmtignore`-equivalent glob to the lefthook command if you take this route.
3. Whichever you pick, decide tabs (`shfmt` default, what the fork chose) vs `-i 4` (what upstream's code already looks like). `-i 4` yields a smaller diff and matches the surrounding style; the fork's choice of tabs was arbitrary. **This is a taste call, not a correctness one.**

### 2.5 `.cargo/audit.toml` (`fcbbce29`)

**VERIFIED.** `diff <(git show e805fe98:.cargo/audit.toml) upstream/main's .cargo/audit.toml` → **identical**. The file suppresses `RUSTSEC-2025-0140` (gix-date, reached via `vergen-gix`).

The commit message's rationale — "Cargo audit configuration is handled in CI workflows directly" — is only half right. What CI does (`.github/workflows/audit.yml:39-45`, mirrored at `.github/workflows/ci.yml:63-69`):

```yaml
      - name: Security audit (advisories)
        run: cargo audit --deny warnings
        continue-on-error: true          # ← advisory-only
      - name: Check for yanked dependencies
        run: cargo audit --deny yanked   # ← hard gate
```

So the advisory run is non-blocking anyway (removing the ignore cannot redden CI), and the yanked check is unaffected by `[advisories].ignore`. **But** `vergen-gix = "10.0"` is still a dependency (`Cargo.toml:131`) and `Cargo.lock` still carries 606 `gix`-related lines, so a developer running `cargo audit` locally (the fork's dev shell ships `cargo-audit`, `FORK: flake.nix:141`) *will* see RUSTSEC-2025-0140 once the file is gone.

**Verdict: STILL-NEEDED (the deletion is harmless and re-applies as a one-line `git rm`) — but reconsider.** My recommendation is to **keep upstream's `.cargo/audit.toml`** and instead make the CI advisory step blocking (`continue-on-error: false`), which is a strictly better productization outcome than deleting the suppression file and leaving the step non-blocking. Either way this is a two-minute decision; flagging it so the Phase-4 executor doesn't re-apply the deletion on autopilot.

### 2.6 CI workflows (`6a0518d0`)

**VERIFIED.**

| | fork `main` | upstream `d1fb0d3a` |
|---|---|---|
| workflows | 7 (`audit, ci, conformance, e2e-full, notify-acfs, release, update-package-manifests`) | 8 (+ `doctor.yml`) |
| total lines | 1,314 | 1,907 |
| `ci.yml` | 363 | 477 |
| `release.yml` | 388 | 660 |
| diff vs fork point | +64 / −64 across 6 files (pure rename + gates) | +835 / −190 across 10 files |

The two *substantive* (non-rename) things `6a0518d0` did — **VERIFIED that upstream did neither**:

1. `cargo clippy --all-targets --all-features -- --deny warnings` and the same for `--no-default-features`. Upstream `.github/workflows/ci.yml:44,47` still runs bare `cargo clippy --all-targets --all-features` / `--no-default-features`. (`git diff e805fe98 upstream/main -- .github/workflows/ci.yml | grep -iE 'clippy|deny|fmt'` → empty.) Note: upstream's `Cargo.toml [lints.clippy]` already sets `pedantic`/`nursery` to `deny` (`Cargo.toml:198-216`), so `-- --deny warnings` adds *rustc* warnings (dead_code, unused, etc.) on top. Whether upstream is currently green under that is a **Phase-0 baseline question** — record it before deciding whether the gate goes in as a hard fail or as no-NEW-warnings.
2. Bench regression threshold `1.10` → `1.05`. Upstream `.github/workflows/ci.yml:356,364,396` still says 10%.

**Verdict: SPLIT the commit.**
- The `br`→`obr` half (artifact names, binary paths, archive names, release titles, `TOOL_NAME`, job names) belongs in **Phase 6(d)**, re-derived against upstream's much larger `release.yml` (660 lines) — not cherry-picked. Note the fork's own commit left a bug: `conformance.yml` prints `=== obr version ===` and runs `./target/release/obr`, but the job name and echo text were only partially updated, and `ci.yml`'s grep still accepts `"obr\|beads"`.
- The gate-tightening half is **STILL-NEEDED** and should be one small Phase-4 commit touching 3 lines in `ci.yml`.
- Additionally **ADOPT** upstream's new CI productization the fork lacks: `doctor.yml`, sha-pinned actions with a machine-checkable pin manifest (`.github/action-pins.jsonl`, `.github/action-pin-upstreams.jsonl`, `scripts/verify-workflow-action-pins.sh`, `scripts/audit-workflow-action-pins.sh`, `docs/CI_SUPPLY_CHAIN.md`), and `scripts/verify-release-workflow-fragments.sh`.

---

## 3. `b3415739` — license. **USER DECISION (plan §8.1). Not decided here.**

Facts only, all **VERIFIED**:

1. **At the fork point** (`e805fe98:LICENSE`) upstream's license was plain **MIT**, © 2026 Jeffrey Emanuel. The fork deleted it and added `LICENSE.md` (BSD 3-Clause, © 2026 John Wiegley, New Artisans LLC clause) and set `Cargo.toml: license = "BSD-3-Clause"`.
2. **Upstream changed its license after the fork**: `git log upstream/main -- LICENSE` → `8858ab79 Add MIT License` then **`b91c42b5 chore: update license to MIT with OpenAI/Anthropic Rider`** (CHANGELOG.md:661). The current `LICENSE` is MIT plus a ~50-line rider.
3. **What the rider says** (quoting `LICENSE`, upstream worktree):
   - "Restricted Parties" = OpenAI, L.L.C.; Anthropic, PBC; their Affiliates; "and any person or entity acting directly or indirectly on behalf of, for the benefit of, or under the direction of any of the foregoing (including any officer, director, employee, contractor, agent, consultant, service provider, or representative)."
   - "Notwithstanding any other provision of this License, no rights are granted to any Restricted Party."
   - "You may not provide, disclose, distribute, sublicense, sell, lease, lend, host, make available, or otherwise permit access to the Software or any derivative work … to or for any Restricted Party."
   - "'use' includes … benchmarking, testing, analyzing, indexing, or incorporating the Software or any Derivative Works into any dataset, training corpus, evaluation harness, or pipeline for machine learning or other automated systems."
   - "This rider applies to the Software and all Derivative Works. … any distribution of the Software or any Derivative Works must include this rider provision unmodified."
   - Breach "automatically and immediately terminates the permissions granted by this License."
4. **Mechanical consequence for the forward-port:** the port branch will contain upstream's code, so it is a derivative work. `b3415739`'s literal action (delete `LICENSE`, ship only BSD-3) is not available on the new base if the rider's "must include this rider provision unmodified" clause is honored. The plan's §8.1 options (a)/(b) both keep upstream's LICENSE text present.
5. **The fork's relicense was also incomplete** — worth fixing whichever way §8.1 lands:
   - `FORK: packaging/homebrew/obr.rb:16` → `license "MIT"`
   - `FORK: packaging/scoop/br.json:6` → `"license": "MIT"`
   - `FORK: packaging/aur/PKGBUILD:13` → `license=('MIT')`, and `:28-44` **embeds the full MIT text inline** via a heredoc
   - `FORK: packaging/aur/PKGBUILD-git:42` → `install -Dm644 "LICENSE" …  || true` — the fork deleted `LICENSE` (renamed to `LICENSE.md`), so the `-git` AUR package silently installs **no** license file. Same four sites exist upstream unchanged.
   - `FORK: README.md:145-147` says only "BSD 3-Clause -- see LICENSE.md" and contains **zero** attribution to beads or beads_rust — even plain MIT requires the copyright notice be "included in all copies or substantial portions of the Software."
6. **One thing to raise with the user, neutrally:** this port is being executed with Claude Code (Anthropic). Whether an AI coding agent operated by the licensee constitutes a "Restricted Party … acting … under the direction of" Anthropic, and whether the work performed constitutes "analyzing"/"testing" the Software, is an interpretive legal question I am not qualified to answer and will not answer. It is squarely inside §8.1 and may warrant counsel before the branch is landed or published.

**Do not let Phase 4 apply `b3415739`.** Gate it behind an explicit user answer.

---

## 4. `a2ba738e` — README. Deferred to Phase 8 (plan is correct) — with corrections

**VERIFIED.** The plan says README is "deferred to Phase 8 (it must reflect the rename too)". Correct, and two further reasons make a *rewrite from scratch* the right call rather than re-applying the fork's 93-line README:

1. **The fork's README is already factually wrong about the fork's own headline feature.** `FORK: README.md:33` says `obr init  # creates .beads/ directory`; `:43` "`.beads/beads.db`"; `:46` "exports the database to `.beads/issues.jsonl` -- one JSON object per line"; `:131` "`.beads/beads.db   .beads/issues.jsonl`". The Org-mode format — the fork's entire reason to exist — is not mentioned once. It was written before / independently of the Org work and never updated.
2. **It has no Origins/attribution section.** Plan §8 already requires one (crediting Steve Yegge's beads and Dicklesworthstone's beads_rust); note that under either §8.1 outcome, attribution is a *license obligation*, not just courtesy.
3. Upstream's README is now 45 KB / 1,240+ lines with 18 H2 sections (`Why This Project Exists`, `TL;DR`, `Design Philosophy`, `Comparison vs Alternatives`, `Installation`, `Commands`, `Configuration`, `Architecture`, `Troubleshooting`, `Limitations`, `FAQ`, `AI Agent Integration`, `VCS Integration`, `Community Projects`, `About Contributions`, `License`). The fork's rewrite deleted 734 lines of it. On the new base that ratio is worse; treat `a2ba738e` as a *style specification* (concise, maintainer voice, no badges, no curl-pipe-bash, no comparison tables) and write fresh content against the post-rename tree.

Use it as spec, not as a patch. **Verdict: SEMANTIC-MERGE, Phase 8.**

---

## 5. Upstream root artifacts — disposition rationales

Sizes from `du -sh`; tracked counts from `git ls-files`. All **VERIFIED**.

### 5.1 Drop

**`.claude-plugin/` (2 files, 8 KB) — DROP.** `plugin.json` declares `"name": "beads"`, `"displayName": "Beads (br)"`, `"version": "0.2.19"`, `"author": {"name": "Dicklesworthstone"}`, `"license": "MIT"`, `homepage`/`repository` pointing at `github.com/Dicklesworthstone/beads_rust`, and `"skills": ["./.claude/skills/"]`. `marketplace.json` declares an *official marketplace* for the upstream project. Every field is an identity claim about someone else's project; "adopt-renamed" would mean publishing an `obr` plugin to a marketplace the fork does not run, and maintaining a second distribution channel indefinitely. The fork does not use Claude Code plugins today. **Drop; revisit only if the user later wants to publish an `obr` agent plugin, which is a product decision, not a port decision.** Feeds R4/Phase 6 (heavy branding) only if adopted.

**`.claude/skills/br/` (5 files, 32 KB) — DROP.** The skill directory is named `br` and its `SKILL.md` + four reference files (`COMMANDS.md`, `CONFIG.md`, `INTEGRATION.md`, `TROUBLESHOOTING.md`) are wall-to-wall `br`/`.beads` instructions. It is the payload of `.claude-plugin/`; dropping one without the other leaves a dangling `"skills"` pointer. If the user *does* want an agent skill for `obr`, write it fresh in Phase 8 against the renamed CLI — mechanically de-branding 32 KB of prose is more error-prone than regenerating it. Heavy branding → R4.

**`skills/bd-to-br-migration/` (8 files, 52 KB) — DROP.** Not new (present at fork point; the fork's `5eb13fb7` even shfmt'd its two scripts). Its entire purpose is migrating documentation from Go `bd` to Rust `br` — a transition that is irrelevant to `obr` and whose content ("`bd sync` → `br sync --flush-only`", "Issue IDs: bd-### → br-###") would have to be rewritten wholesale to mean anything post-rename. Its two scripts (`find-bd-refs.sh`, `verify-migration.sh`) are, however, a decent *template* for the Phase-7 G-rename audit script — worth reading once, then deleting. Total branding → R4 would otherwise inherit ~52 KB of noise.

**`.agent-mail.yaml` (62 B) — DROP.** Contents in full: a comment plus `project_uid: cc95740c87f7e565c386`. It is a discovery token for upstream's `mcp_agent_mail_rust` coordination tooling, tied to *their* project identity. Carrying it forward would make an `obr` checkout announce itself as upstream's project to that service. Note the oddity: upstream's `.gitignore:125` lists `.agent-mail.yaml`, yet the file is tracked — it was committed before the ignore rule. No branding, so R4 is unaffected.

**`.ntm/` (3 files, 12 KB) — DROP.** `.ntm/logs/{am,bd,cm}-beads_rust.log`. Present at fork point too (the fork inherited them). These are log droppings from upstream's `ntm` tooling; nothing reads them. Filenames carry `beads_rust`, so they would otherwise show up in the R4 grep as false positives.

**`refactor/` (63 files, 268 KB) — DROP.** Contents are dated session artifacts from upstream's own refactoring campaigns: `refactor/artifacts/2026-04-25-codex-simplify-loop/{SKILL_LOOP_PROGRESS.md, golden_workspace.env, loc_before.txt, head_before.txt, baseline.md, git_status_before.txt, …}` and `refactor/artifacts/2026-04-24-shrink-pass-1/{clippy_before.txt, REJECTIONS.md, fmt_before.txt, …}`. They are point-in-time scratch state referencing upstream commit SHAs and upstream's clippy baseline; they have no meaning on the fork and no consumer in the build. Some contain `br`/`beads` strings → R4 noise if kept.

**`sample_beads_db_files/` (32 files, 178 MB) — DROP, highest-value deletion in this list.** Thirteen subdirectories of real `beads.db` + `issues.jsonl` snapshots from upstream author's other projects (`asupersync` 31 MB, `frankenterm` 23 MB, `flywheel_connectors` 21 MB, `frankensqlite` 19 MB, plus `franken_whisper`, `frankentui`, `mcp_agent_mail_rust`, `ntm`, `remote_compilation_helper`, and two `repro_*_import_write.*` reproduction dumps). **Nothing references them**: `grep -rn sample_beads_db_files src tests scripts .github docs Cargo.toml AGENTS.md README.md .gitignore` returns exactly two hits — `Cargo.toml:14` (publish-exclude) and `.gitignore:148-149` (ignore the `-shm`/`-wal` sidecars). They are not fixtures for any test. Keeping them means 178 MB of third-party issue data in every clone of a repo the user intends to de-brand. Branded by directory name and by `beads.db` filenames → R4.

**`temp_test/`, `temp_test_2/`, `custom.db` — DROP in the Phase-0.5 cleanup commit.** `temp_test/.beads/{metadata.json,.local_version,config.yaml,daemon.log,.gitignore}` and `temp_test_2/.beads/{metadata.json,config.yaml,.gitignore}`; `custom.db` is a 172 KB SQLite file at the repo root. **The plan is right that all three exist in both trees** (fork: 5 + 3 + 1 tracked; upstream: 5 + 3 + 1 tracked). Both trees *also* already ignore them (`FORK: .gitignore:43` and upstream `.gitignore:60`, both `temp_test*/`) — they were committed before the ignore rule landed, which is why `git clean` never removed them. Pure accident; delete.

**`.rchignore` (1.4 KB) — DROP.** Ignore-list for upstream's `rch` (remote-compilation-helper) sync tool; its comments reference "the local frankensqlite/asupersync patch configuration for proof runs". No `rch` in the fork's workflow.

### 5.2 Adopt / adapt

**`agent_baseline/` (18 files, 204 KB) — ADOPT-RENAMED, and it is LOAD-BEARING.** This is the most important entry in this table and the plan does not mention it. `tests/e2e_schema.rs:920-928` defines `agent_baseline_snapshots_match_current_binary()`, which compares live CLI output against golden files and fails with `"agent_baseline/{relative_path} is stale; rerun with UPDATE_AGENT_BASELINE=1"` (`tests/e2e_schema.rs:1129,1156,1185`; env const at `:19`; root resolved at `:1204`). `scripts/verify-agent-contracts.sh:30` runs exactly this test. The goldens are:

```
agent_baseline/help/br_help.txt          agent_baseline/schemas/cli_schema.json
agent_baseline/help/br_list_help.txt     agent_baseline/schemas/schema_all.json
agent_baseline/help/br_schema_help.txt   agent_baseline/schemas/schema_error.json
agent_baseline/examples/{ready,show_one,list_limit3,version}.json (+ .toon)
agent_baseline/examples/robot_mode_examples.jsonl
agent_baseline/errors/show_not_found.json
agent_baseline/{README_first_80_lines.md, AGENT_JOURNEY_NOTES.md}
agent_baseline/schemas/schema_issue_details.json
```

Every filename prefixed `br_`, and `grep -ric 'beads|\bbr\b'` finds hits in 12 of the 18 (`schema_all.json` 18, `README_first_80_lines.md` 17, `br_help.txt` 7). **Therefore: the Phase-6 rename WILL turn this test red, and the fix is not a hand-edit — it is `UPDATE_AGENT_BASELINE=1 cargo test --test e2e_schema agent_baseline_snapshots_match_current_binary` plus `git mv` of the three `br_*.txt` files to `obr_*.txt` (the path strings are built in `tests/e2e_schema.rs`, so the test source changes too).** Also note `README_first_80_lines.md` is a golden copy of the README's first 80 lines — so the Phase-8 README rewrite *also* invalidates this baseline. Sequence Phase 8 before the final baseline regeneration, or regenerate twice. **Feeds R4 directly.**

**`.githooks/` (2 files, 8 KB) — ADAPT (fold into lefthook), then delete.** See §2.3. If instead you keep it: rename `br doctor` → `obr doctor`, `.beads` → `.obr`, `BR_DOCTOR_SKIP_PRECOMMIT` → `OBR_DOCTOR_SKIP_PRECOMMIT`, and the README's "Per AGENTS.md, `br` is non-invasive: it never installs git hooks on its own" line. Heavy branding → R4 either way (the file will be either de-branded or deleted).

**`CHANGELOG.md` (69 KB) — ADOPT-RENAMED, with a strategy.** The fork has none (`git ls-files CHANGELOG.md` on `main` → 0). Upstream's spans v0.1.x → v0.2.22 and is dense with `br`/`beads`/`.beads` and links to `github.com/Dicklesworthstone/beads_rust/commit/…`. Two defensible approaches: (a) keep it verbatim as *upstream history* under a heading that says so, and start a new `## obr` section on top — historical entries describing what upstream's `br` did are arguably the one legitimate place for the old name (same category as the README Origins section); or (b) truncate to the last few releases. **Recommend (a)** — it is honest, it is cheap, and it keeps the R4 allowlist small and principled. The plan's G-rename gate already excludes `CHANGELOG.md` (`§7.8 G-rename`), which is consistent with (a). Note upstream's `CHANGELOG.md:661` is where the license change is recorded.

**`.gitattributes` (`/fuzz/corpus/** binary`) — ADOPT verbatim** if the corpora are kept. Trivial, no branding.

**`.github/` supply-chain machinery — ADOPT.** `action-pins.jsonl` (38 lines) + `action-pin-upstreams.jsonl` (14) + `scripts/verify-workflow-action-pins.sh` + `scripts/audit-workflow-action-pins.sh` + `docs/CI_SUPPLY_CHAIN.md`. All workflows are sha-pinned (e.g. `actions/checkout@3d3c42e5…  # v7.0.1`). This is *better* productization than the fork has and costs nothing to keep. No branding.

**`.github/workflows/doctor.yml` (164 lines) — ADOPT-RENAMED.** Runs `br doctor health --json` against the repo's own `.beads/` and fails on non-healthy, plus `br doctor capabilities --format json` to lock the contract shape. Env `BR_NO_AUTOFLUSH: 1`. Rename `br`→`obr`, `.beads`→`.obr`, `BR_NO_AUTOFLUSH`→`OBR_NO_AUTOFLUSH` in Phase 6(d).

**`packaging/` — ADOPT-RENAMED.** `packaging/scoop/br.json`, `packaging/homebrew/br.rb`, `packaging/aur/PKGBUILD`, `packaging/aur/PKGBUILD-git`. Present in both trees. The fork already did `git mv packaging/homebrew/br.rb packaging/homebrew/obr.rb` (`a2760e61`/`6a0518d0`) but left `packaging/scoop/br.json` un-renamed and all four license fields at MIT (§3.5). Phase 6(d) must handle all four files: filename, `pkgname`, binary name, download URLs, and license.

**`.ubsignore` (295 B) — keep.** Present in both trees. Mentions `legacy_beads/` — trivial R4 hit, de-brand or leave (it names a directory that no longer exists).

---

## 6. Item 4 of the brief: tarpaulin, toolchain, CHANGELOG, install.sh, completions, man pages

**All VERIFIED.**

| Artifact | Fork vs upstream | Productization overlap |
|---|---|---|
| `tarpaulin.toml` | **byte-identical** (`diff` clean; 1,048 B both). Fork never touched it. | **None.** Take upstream's. Note nothing in CI runs tarpaulin — only `scripts/coverage.sh` does, manually. If the user wants a coverage gate (`fail-under = 50.0` is already configured), that is *new* work, not a re-application. |
| `rust-toolchain.toml` | **byte-identical**: `[toolchain] channel = "nightly"` + `components = ["rustfmt", "clippy"]`. | **None.** But `Cargo.toml:5` now says `rust-version = "1.88"` (was `1.85` at fork point) — worth a line in the updated `CLAUDE.md`. |
| `CHANGELOG.md` | fork: absent. upstream: 69 KB. | See §5.2. |
| `install.sh` | fork 40,327 B (shfmt'd to tabs, `br`→`obr`); upstream 61,370 B (4-space, `br`). | **Discard the fork's file entirely.** Re-derive: take upstream's, apply the Phase-6 rename, then shfmt per §2.4. `notify-acfs.yml` watches `install.sh` and posts to `Dicklesworthstone/agentic_coding_flywheel_setup` — that workflow should probably be **dropped**, not renamed, since it notifies *upstream's* installer registry. |
| completions | Generated at **runtime** by `br completions <shell>` (`src/main.rs:709` → `src/cli/commands/completions.rs`); `clap_complete` with `unstable-dynamic` (`Cargo.toml:33`). There is a `CompletionIndex` with workspace-aware dynamic completion (`src/cli/mod.rs:212-265`, incl. `resolve_completion_paths_for_beads_dir` at `:225`). No checked-in completion files in either tree. | **No fork overlap.** But `src/cli/mod.rs:225` is a named R4 target, and the emitted completion script's command name changes with the rename — Phase 7 should smoke `obr completions bash \| grep -c '\bbr\b'`. |
| man pages | **None in either tree** (`find . -name '*.1' -o -name man -type d` → empty). | **None.** The plan's §7.6 row "man pages" has no referent; drop it from the rename map (or treat it as "if we ever add them"). |

---

## 7. Corrections to the plan

Loudly, as instructed. Each is **VERIFIED**.

1. **`rusqlite` → `fsqlite` is missing from the plan entirely.** The plan's §3 fact table, §4 strategy, §6 spec, and §7 landmines never mention that upstream swapped its SQL engine for a pure-Rust one (`Cargo.toml:43-59`; zero `rusqlite` in `Cargo.lock`). This invalidates the literal text of `aa129a2c`, changes the row-access API used everywhere in `sqlite.rs`, changes concurrency assumptions (`src/storage/sqlite.rs:2285-2290`), and probably removes the need for `bindgenHook`/bundled-SQLite plumbing in the flake. It deserves a first-class line in §3 and a landmine in §7.

2. **`#![forbid(unsafe_code)]` is no longer true upstream.** `Cargo.toml [lints.rust] unsafe_code = "deny"` and `src/lib.rs:20-27` document one sanctioned `#[allow(unsafe_code)]` in `sync::db_inode_lock` (GitHub #412). The fork is still `forbid` (`FORK: Cargo.toml:104-105`) and `CLAUDE.md` asserts `forbid`. Re-imposing `forbid` on the new base **will not compile**. §7 / Phase 8 must say so.

3. **§7.1 misdescribes the org2jsonl pin.** "Fork's `flake.nix:22` pins `git+file:///Users/johnw/src/org2jsonl` at rev `5ea7586`" — `FORK: flake.nix:21-24` contains **no rev**; the rev lives in `FORK: flake.lock` (`rev: 5ea75860bdcca8ffe3989ae877b63328e34c6270`, `revCount: 15`). The remediation the plan prescribes (pin explicitly) is right; the description of the current state is wrong, and the difference matters because a bare `git+file://` URL silently re-resolves on any `nix flake update`.

4. **The R3 brief calls `agent_baseline/` (and implicitly `skills/`, `.ntm/`) "new upstream root artifacts". They are not new** — all three existed at the fork point (`git ls-tree -r e805fe98`: `agent_baseline` 16 files, `skills` 8, `.ntm` 3). Genuinely new since the fork: `.claude/`, `.claude-plugin/`, `.agent-mail.yaml`, `.githooks/`, `refactor/`, `sample_beads_db_files/`, `.gitattributes`, `.rchignore`, `CHANGELOG.md`, `.github/action-pins.jsonl`, `.github/action-pin-upstreams.jsonl`, `.github/workflows/doctor.yml`. Conversely `.cargo/audit.toml` is *not* an upstream addition — it existed at the fork point and the fork deleted it.

5. **`agent_baseline/` is a rename-blocking test dependency and the plan never mentions it.** `tests/e2e_schema.rs:920` + `scripts/verify-agent-contracts.sh:30`. Phase 6 and Phase 7 both need an explicit step: regenerate with `UPDATE_AGENT_BASELINE=1`, and `git mv agent_baseline/help/br_*.txt` → `obr_*.txt` with matching edits in `tests/e2e_schema.rs`. Add it to the §7.8 gate list.

6. **Phase 4's one-liner "merge fuzz target sets" understates the trap.** Re-applying `4cbf6348`'s `.gitignore` hunk verbatim adds `fuzz/corpus/`, which upstream **tracks** (2,747 files). Phase 4 must apply that commit *minus* one line. Similarly, "re-run shfmt" is a 160-file operation on the new base, not the 17-file operation it was on the fork (§2.4).

7. **Phase 4's "keep lefthook (upstream has none)" is right, but the plan omits `.githooks/`**, which is a *second*, overlapping pre-commit mechanism upstream added. Shipping both is a footgun (`core.hooksPath` vs lefthook's installed hooks). Decide one.

8. **The plan's §7.6 rename-map row for "man pages" has no referent** — neither tree has any. Minor, but it will send a Phase-6 executor looking for files that do not exist.

9. **Upstream's `flake.nix` being *identical to the fork point except a version bump*, and having no `flake.lock`, is stronger than §3's neutral "Upstream DOES have (own versions) `flake.nix`".** The plan's Phase 4 phrasing "graft fork's flake improvements onto upstream's flake (or vice versa — R3 decides)" can be resolved now: **the fork's flake is the base; upstream's contributes nothing.**

10. **Plan §5 Phase 0.5's claim about `temp_test/`, `temp_test_2/`, `custom.db` existing in both trees is CORRECT** (I checked; they are tracked in both, and ignored in both). No correction — recording it because a partial `ls` can easily suggest otherwise.

---

## 8. Recommended Phase-4 commit series (derived from the above)

Ordered, each independently gated per plan §7.8. Rename-coupled work is deliberately deferred to Phase 6.

| # | Commit | Contents | Source |
|---|---|---|---|
| 1 | `chore: remove stray artifacts from upstream tree` | `git rm -r temp_test temp_test_2 custom.db sample_beads_db_files refactor .ntm .rchignore .agent-mail.yaml .claude .claude-plugin skills` | plan §5 Phase 0.5 + §5.1 above |
| 2 | `build(nix): replace crane+fenix flake with rust-overlay` | fork `flake.nix` as base; drop `toon_rust-0.1.1` outputHashes, drop `bindgenHook`+`sqlite` (verify), add `org2jsonl` input pinned by rev, commit `flake.lock` | `f826ac4e` + `33bb2e82`, §2.1 |
| 3 | `build(nix): add flake checks and extend dev shell` | `checks.{build,formatting,clippy,tests}`; devShell += lefthook, cargo-audit, shellcheck, shfmt | `1c32e5e5`, §2.1 |
| 4 | `chore: add lefthook pre-commit hooks` | `lefthook.yml` verbatim + `.gitignore` hunk **minus** `fuzz/corpus/`; fold the `.githooks/pre-commit` doctor check in and `git rm -r .githooks` | `4cbf6348`, §2.3 |
| 5 | `test(fuzz): add org and validation fuzz targets` | two new `[[bin]]` + two `fuzz_targets/*.rs` on top of upstream's `fuzz/`; README entries; seed `fuzz/corpus/org_parse/`. **Depends on Phase 3** (needs `org_bridge`) | `5312cb5a`, §2.2 |
| 6 | `ci: tighten clippy and benchmark gates` | 3 lines in `.github/workflows/ci.yml` (two `-- --deny warnings`, `1.10`→`1.05`). Condition on the Phase-0 baseline being green under `-D warnings` | `6a0518d0` (gate half), §2.6 |
| 7 | `style: format shell scripts with shfmt` | **Optional.** If taken, scope to `install.sh` + `scripts/*.sh`, exclude `tests/doctor_fixtures/**`. Decide tabs vs `-i 4` first | `5eb13fb7`, §2.4 |
| 8 | *(blocked)* license | Do not write until §8.1 is answered | `b3415739`, §3 |

Not in Phase 4: `aa129a2c` (nothing to do — §1), README (`a2ba738e` → Phase 8 — §4), the rename half of `6a0518d0` (→ Phase 6d), `agent_baseline` regeneration (→ Phase 6/7 — §5.2), `.cargo/audit.toml` (→ one-line decision, §2.5).

---

## 9. Open questions for the integration planner

1. **Is upstream green under `cargo clippy --all-targets --all-features -- --deny warnings`?** Phase-0 baseline must answer this before commit #6 above is written as a hard gate. If red, the gate becomes no-NEW-warnings per plan §7.8 G-lint.
2. **shfmt: tabs or `-i 4`, and full-repo or scoped?** (§2.4) Pure taste, but it determines whether a ~160-file formatting commit lands in the series.
3. **`.cargo/audit.toml`: delete it (re-apply `fcbbce29`) or keep it and make the CI advisory step blocking?** (§2.5) I recommend the latter; the user may disagree.
4. **`.githooks/` vs `lefthook.yml`: one mechanism or two?** (§2.3)
5. **`CHANGELOG.md`: keep verbatim-as-upstream-history plus a new `obr` section, or truncate?** (§5.2) Affects the R4 allowlist.
6. **Does `fsqlite` still need `bindgenHook` / a `sqlite` buildInput in the flake?** INFERRED "no"; needs one `nix build` in Phase 4 to confirm.
7. **`notify-acfs.yml`: rename or drop?** It notifies upstream's installer registry (`Dicklesworthstone/agentic_coding_flywheel_setup`) about `install.sh` changes. The fork renamed `TOOL_NAME` to `obr` but kept pointing at upstream's repo — almost certainly wrong. Recommend drop; flagging rather than deciding.
