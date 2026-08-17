# R4 — Branding Surface Inventory (upstream/main + fork Org additions)

**Brief:** Phase 1 (R4) of `/Users/johnw/src/obr/UPSTREAM_SYNC_AND_RENAME_PLAN.md`.
**Written:** 2026-08-06. **Author:** read-only recon subagent.
**Upstream tree read:** `/Users/johnw/src/obr-org-forward-port` (worktree at `upstream/main` = beads_rust v0.2.22, `d1fb0d3a`). Nothing was modified there.
**Fork tree read:** `/Users/johnw/src/obr` @ `main` (fork point `e805fe98`).
**Evidence rule:** every claim below is either **VERIFIED** (I ran the grep / read the lines cited) or **INFERRED** (deduced, marked inline). Paths are worktree-relative unless prefixed with `fork:`.

---

## 0. Summary table

Counts are over **git-tracked files only**, excluding `Cargo.lock`, `*.db*`, and `fuzz/corpus/` (2,747 generated corpus files) unless noted. All counts VERIFIED by the commands in §1.

| # | Category | Distinct names | Files | Hits | Renameable mechanically? | Needs compat shim? |
|---|---|---|---|---|---|---|
| a | On-disk artifact names | 17 | 457 | 4,736 | Mostly (literals) | **Yes** — 8 of them |
| b | Environment variables | **103 tokens** (24 product-read + 1 dynamic namespace `BD_*`; rest test/CI/installer) | 138 | 863 | Yes | **Yes** — 25 |
| c | Crate / binary identity | 2 (`beads_rust`, `br`) | 266 | 2,146 | Yes (`beads_rust::` in 86 files) | No (hard rename) |
| d | User-config paths | 5 | 21 | 45 | Yes | **Yes** — legacy-read chain |
| e | Help / error / log strings | n/a | 215 (`` `br ``) + 99 (`"br `) | 2,281 | Semi (prose) | No |
| f | Shell completions / man pages | 0 man pages; completions derive from clap name | 3 | ~15 | **Automatic** | No |
| g | Packaging (AUR/Homebrew/Scoop/install.sh/CI) | 6 files + 8 workflows | 13 | ~230 | Yes | No |
| h | Docs | 41 doc files (+README 45 KB, AGENTS 42 KB, CHANGELOG 69 KB) | 43 (`docs/`+README+AGENTS+UPGRADE_LOG); 94 incl. skills/plugin/packaging/CI/scripts | 2,579 (+341 CHANGELOG) | Semi (prose) | No |
| i | `.claude-plugin/` + skills + agent-emitted blurb | 4 manifests + 13 skill files + `AGENT_BLURB` | 16 | 688 | Yes | No |
| j | Internal identifiers | 87+ distinct; `beads_dir` (3,791), `BeadsError` (2,300) | 149 | **11,812 tokens** | Yes (sed-able) | No |
| k | Test fixtures / snapshots with embedded literals | — | **506** under `tests/` (**393** excluding `tests/artifacts/`) | ~4,000 | Mechanical breakage | No |
| **NEW** | `br.*` machine-readable schema IDs | **26** | 20 | ~70 | Yes | **Judgement** (§5.7) |
| **NEW** | `beads://` MCP resource URIs | 12 | 10 | 115 | Yes | **Judgement** (§5.8) |
| **NEW** | Hash domain separators | 5 | 3 | 5 | Yes | **Judgement** (§5.9) |
| **NEW** | Fork's own tracker issue-ID prefix `beads_rust-` | 1 | fork `.beads/issues.jsonl` (549 issues) | 549 IDs | Needs `--rename-prefix` run | **USER DECISION** (§8) |

**Headline:** the plan's draft §7.6 map covers roughly **half** the real surface. The four biggest omissions are (1) 26 `br.*` versioned JSON schema identifiers that are a machine contract, (2) the `beads://` MCP resource URI scheme, (3) the `"br"` default issue-ID prefix fallback plus the fork's own `beads_rust-*` issue IDs, and (4) `.br_recovery/`, `bd.sock`, `.bv.lock`, `~/.cache/beads/startup`, `.beads.snapshot_*` — five artifact names the plan never lists. Also: `.br_history` is **already inside** `.beads/`, so the plan's `.obr/history/` vs `.obr_history/` question is mis-framed (§6.1).

---

## 1. Method (reproducible)

All greps run from `/Users/johnw/src/obr-org-forward-port`.

```bash
# Authoritative: tracked files only. Note git grep -E is POSIX ERE (no \b);
# use -P for word boundaries. rg silently skips gitignored-but-tracked files
# (sample_beads_db_files/, temp_test/), so rg undercounts by ~2,300 hits.
EX=(':!Cargo.lock' ':!*.db' ':!*.db-wal' ':!*.db-shm' ':!fuzz/corpus')
git grep -Pc -e '<PATTERN>' -- "${EX[@]}" | awk -F: '{s+=$NF;n++} END {print n" files, "s" hits"}'
```

Whole-repo baseline (VERIFIED):

| Pattern | Files | Hits |
|---|---|---|
| `(?i)beads` (all tracked, ex-lock/db) | 748 | 16,906 |
| `\bbeads\b` | 549 | 6,981 |
| `\bbd\b` | 1,050 | 18,042 |
| `\bbr\b` | 628 | 9,118 |

`\bbd\b` and `\bbr\b` are dominated by **test-fixture issue IDs** (`bd-001`, `br-mcp-…`) — see §4.k. That is why the naive G-rename grep in plan §7.8 will produce thousands of false positives unless the allowlist in §7 below is used.

Distribution of `(?i)beads` by top-level path (VERIFIED):

```
src 86f/7819h   sample_beads_db_files 13f/3168h   tests 471f/3165h   .beads 8f/1026h
docs 33f/737h   CHANGELOG.md 1f/341h   skills 8f/96h   fuzz 10f/88h   README.md 1f/68h
AGENTS.md 1f/44h  .claude 4f/37h  refactor 13f/35h  .github 6f/32h  .gitignore 1f/28h
flake.nix 1f/27h  agent_baseline 7f/26h  packaging 4f/17h  .rchignore 1f/17h
scripts 8f/15h  .claude-plugin 2f/14h  benches 1f/11h  install.sh 1f/10h
.githooks 2f/8h  UPGRADE_LOG.md 1f/6h  Cargo.toml 1f/6h  temp_test* 5f/6h
.ubsignore 1f/2h  tarpaulin.toml 1f/1h  build.rs 1f/1h
```

---

## 2. (a) On-disk artifact names — COMPLETE inventory

Every name below is a file or directory this tool **creates or reads on disk**. This is the mandate-critical category ("no beads naming in created/managed artifacts").

| Artifact | Where defined | Files | Hits | Notes |
|---|---|---|---|---|
| `.beads/` workspace dir | `src/config/mod.rs:55-57` (`is_beads_dir_name`) | 398 | 3,331 | VERIFIED |
| `_beads/` monorepo variant | `src/config/mod.rs:56` | 46 | 276 | VERIFIED — accepted, never created |
| `beads.db` (+`-wal`,`-shm`,`-journal`,`-fsqlite-ns-*`) | `src/config/mod.rs:60` `DEFAULT_DB_FILENAME`; written into `metadata.json` by `src/cli/commands/init.rs:171-174` | 190 | 918 | VERIFIED |
| `issues.jsonl` default export | `src/config/mod.rs:62` `DEFAULT_JSONL_FILENAME` | 243 | 1,438 | Unbranded name; fork changes default to `issues.org` |
| `beads.jsonl` legacy export | `src/config/mod.rs:64` `LEGACY_JSONL_FILENAME`, read in `discover_jsonl` `:147-162` | 10 | 46 | VERIFIED |
| `beads.base.jsonl` / `.left.` / `.right.` (+ `.meta.json`) | `src/config/mod.rs:72-79` `EXCLUDED_JSONL_FILES`; `.beads/.gitignore` template `init.rs:229-234` | 57 | 311 | VERIFIED — 3-way-merge artifacts |
| `.br_history/` | `src/sync/history.rs:502` (`beads_dir.join(".br_history")`) | 49 | 137 | **Inside `.beads/`**, not a sibling |
| `.br_recovery/` | `src/config/mod.rs:66` `RECOVERY_DIR_NAME` | 32 | 107 | **MISSING FROM PLAN.** Inside `.beads/` |
| `redirect` (worktree redirect) | `src/config/routing.rs:190`; gitignored `init.rs:243-244` | 51 | 188 | VERIFIED |
| `routes.jsonl` (town routing) | `src/config/mod.rs:5137`, `:5156` | 21 | 123 | Unbranded name |
| `.write.lock` / `.sync.lock` | `init.rs:208`,`:225`; `src/main.rs` (`.beads/.write.lock`) | 50 | 268 | Unbranded names |
| `bd.sock` (daemon socket) | `init.rs:240` (gitignore template) | 25 | 25 | **MISSING FROM PLAN** |
| `.bv.lock` (beads-viewer lock) | `init.rs:246-247`; root `.gitignore:44` | 24 | 24 | **MISSING FROM PLAN** — foreign-tool integration |
| `last-touched` | `src/util/mod.rs:29` `LAST_TOUCHED_FILE`; root `.gitignore:43` | — | — | Unbranded |
| `sync_base.jsonl`, `deletions.jsonl`, `interactions.jsonl`, `sync-state.json`, `daemon.{lock,log,pid}` | `init.rs:226`,`:236-241`; `config/mod.rs:72-79` | 37 | 82 | Unbranded |
| `~/.cache/beads/startup/` and `$XDG_CACHE_HOME/beads/startup` and `$TMPDIR/beads-startup-cache` | `src/config/mod.rs:4975-4987` | 11 | 20 | **MISSING FROM PLAN** |
| `.beads.snapshot_*/`, `.beads_snap*` (foreign recovery debris) | `src/cli/commands/doctor.rs:2584`, `:17523`; root `.gitignore:168`; `.rchignore:32` | 4 | 8 | **MISSING FROM PLAN** — detection-only, never created |

**Key structural fact (VERIFIED, corrects the plan):** `.br_history` and `.br_recovery` are created as `beads_dir.join(".br_history")` / `.join(".br_recovery")` — i.e. **subdirectories of `.beads/`**, not siblings of it. See `src/sync/history.rs:502` and `src/config/mod.rs:66` + `src/cli/commands/init.rs:216,222` (the `.beads/.gitignore` template ignores `.br_history/` and `.br_recovery/` as relative paths) and root `.gitignore:46` (`.beads/.br_history/`) and `.rchignore:28-31`.

**Shim insertion points (VERIFIED, both exist and are single-function):**
- DB name: `fn resolve_db_path(beads_dir, metadata, db_override)` in `src/config/mod.rs` — reads `metadata.database`, which defaults to `DEFAULT_DB_FILENAME` only when `metadata.json` is absent/blank (`Metadata::load` `:119-135`). Legacy workspaces therefore keep working **without any shim** because their `metadata.json` names `beads.db` explicitly. A shim is only needed for a `.obr/` (or `.beads/`) directory with **no** `metadata.json`.
- JSONL name: `pub fn discover_jsonl(beads_dir)` `src/config/mod.rs:147-162` — already a two-step preference chain (`issues.jsonl` → `beads.jsonl`). Extending it to `issues.org` → `issues.jsonl` → `beads.jsonl` is the Org port's job (Phase 3), and the de-beads rename just drops nothing.

**Deprecation-warning precedent:** upstream has **no** deprecation machinery — `git grep -in deprecat -- src/` returns **zero** hits (VERIFIED). But there is a clean template for a stderr-only, JSON-safe warning: `report_auto_flush_failure` at `src/cli/commands/mod.rs:81-132` (respects `ctx.is_quiet()`, emits a JSON envelope on stderr when `is_json()`/`is_toon()`, otherwise a sanitized text line). The legacy-read shim warning must follow exactly this shape or it will corrupt `--json`/`--robot` stdout.

---

## 3. (b) Environment variables — COMPLETE inventory

**103 distinct `(BR|BD|BEADS)_*` tokens** exist repo-wide (VERIFIED: `git grep -Pho '\b(BR|BD|BEADS)_[A-Z0-9_]+\b' | sort -u | wc -l` = 103). They split three ways.

### 3.1 Read by the product binary (must rename **with** legacy-accept shim) — 24 named + 1 namespace

| Env var | Read at | Semantics |
|---|---|---|
| `BEADS_DIR` | `src/config/mod.rs:236`, `:505` (`beads_dir_override_from_env`) | Workspace override |
| `BEADS_JSONL` | `src/config/mod.rs` `resolve_jsonl_path` (Priority 1) | JSONL path override |
| `BEADS_CACHE_DIR` | `src/util/mod.rs:39` `BEADS_CACHE_DIR_ENV`, used `:48`; `resolve_db_path` comment `src/config/mod.rs:4212` | Move DB off slow FS |
| `BEADS_FLUSH_DEBOUNCE` | `src/config/mod.rs:4354` | → config key `flush-debounce` |
| `BEADS_IDENTITY` | `src/config/mod.rs:4357` | → `identity` |
| `BEADS_REMOTE_SYNC_INTERVAL` | `src/config/mod.rs:4360` | → `remote-sync-interval` |
| `BEADS_AUTO_START_DAEMON` | `src/config/mod.rs:4363` | inverted → `no-daemon` |
| `BEADS_ACTOR` | `src/cli/commands/comments.rs:468` | actor chain #2 |
| `BD_ACTOR` | `src/cli/commands/comments.rs:468`; `src/close_policy.rs:325` | actor chain #1 |
| `BD_DB`, `BD_DATABASE` | `src/config/mod.rs:497` (`startup_db_override_from_env`) | DB path override |
| **`BD_<ANYTHING>`** | `src/config/mod.rs:4345-4351` | **Dynamic namespace.** Any `BD_FOO_BAR` → config key inserted under **three** spellings (`foo_bar`, `foo.bar`, `foo-bar`) via `env_key_variants` |
| `BR_OUTPUT_FORMAT` | `src/cli/mod.rs`; scrubbed at `doctor.rs:13671` | text/json/toon |
| `BR_AGENT_NAME`, `BR_HARNESS`, `BR_MODEL` | clap `env=` attrs, `src/cli/mod.rs:1154,1158,1162` (+5 more arg structs) | Agent identity |
| `BR_SESSION` | `src/cli/…` | Session id |
| `BR_INHERITED_CONTEXT` | `src/…` | Inheritance |
| `BR_NO_AUTOFLUSH` | doctor capability list `capabilities_doctor.rs:152` | Suppress auto-flush |
| `BD_NO_AUTOFLUSH` | `capabilities_doctor.rs:153` | legacy spelling of same |
| `BR_STARTUP_CACHE` | `src/config/mod.rs:4689` `STARTUP_CACHE_ENABLE_ENV` | Enable startup cache |
| `BR_STARTUP_CACHE_DIR` | `src/config/mod.rs:4690` `STARTUP_CACHE_DIR_ENV` | Cache dir override |
| `BR_DISABLE_READ_ONLY_FAST_OPEN` | `src/main.rs:23` | Fast-open kill switch |
| `BR_DISABLE_PARALLEL_JSONL_EXPORT` | `src/sync/…` | Export parallelism |
| `BR_HISTORY_MIN_INTERVAL_SECS` | `src/sync/history.rs:26` | History throttle |
| `BR_DOCTOR_STALE_LOCK_THRESHOLD_SECS` | `src/cli/commands/doctor.rs` | Doctor threshold |
| `BR_DOCTOR_RUNS_DIR` | `src/cli/commands/doctor_subsystems/run_dir.rs:52` `ENV_RUNS_DIR` | Doctor run-dir |
| `BR_MCP_READ_SNAPSHOT` | `src/mcp/mod.rs:30` `MCP_READ_SNAPSHOT_ENV` | MCP read mode |

Non-branded env vars the product also reads (leave alone): `HOME`, `XDG_CACHE_HOME`, `NO_COLOR`, `RUST_LOG`, `COLUMNS`, `EDITOR`/`VISUAL`, `USER`/`LOGNAME`/`USERNAME`, `PATH`, `GITHUB_TOKEN`/`GH_TOKEN`, `TOON_DEFAULT_FORMAT`, `TOON_STATS`.

### 3.2 Declared in the agent-facing capabilities contract

`src/cli/commands/capabilities.rs:213-237` (`const ENV_VARS`) publishes exactly `BD_DB / BD_DATABASE`, `BEADS_JSONL`, `BR_OUTPUT_FORMAT`, `TOON_DEFAULT_FORMAT`, `NO_COLOR`, `RUST_LOG` in `br capabilities --format json` output. VERIFIED. This is a machine-readable contract: renaming the env vars must update this table in the same commit or `br doctor capabilities` self-check drifts.

### 3.3 Scrubbing / witness lists that hardcode the prefixes

- `src/cli/commands/doctor.rs:13665-13675` — scrubs every `BD_*` and `BEADS_*` var plus `BR_OUTPUT_FORMAT` from spawned subprocesses. Must learn `OBR_*` while keeping the legacy prefixes for hygiene.
- `src/config/mod.rs:5110-5131` (`startup_cache_env_witness`) — hardcodes 7 `BEADS_*` names + all `BD_*` vars as cache-invalidation inputs.

### 3.4 Test / CI / installer only (rename freely, **no** compat shim)

~78 tokens, e.g. `BD_BINARY` (59), `BR_E2E_STRESS` (49), `BR_BINARY` (26), `BD_PATH` (21), `BR_BIN` (24), `BR_SYNTHETIC_*`, `BR_NUMA_PROFILE_*`, `BR_BENCH_*`, `BR_CAPACITY_*`, `BD_CONFORMANCE_PATH`, `BD_3PLOP*`/`BD_2DDL_*` (frankensqlite stress knobs), `BR_INSTALL_DIR`, `BR_INSTALLER_{URL,BRANCH,SELF_REEXEC}`.

---

## 4. Remaining categories

### (c) Crate / binary identity

| Site | Value | Evidence |
|---|---|---|
| Cargo package name | `beads_rust` | `Cargo.toml:2` |
| Cargo bin name | `br` | `Cargo.toml:34-36` (`[[bin]] name = "br"`) |
| Clap command name | `br` | `src/cli/mod.rs:692` `#[command(name = "br", …)]` |
| Self-update binary name | `br` | `src/cli/commands/upgrade.rs:25` `const BIN_NAME: &str = "br"` |
| Release repo coords | `Dicklesworthstone` / `beads_rust` | `src/cli/commands/mod.rs:61-62`; used to build 3 URLs `:66,:72,:78` |
| Release asset pattern | `br-{VERSION}-{platform}_{arch}.tar.gz` | `src/cli/commands/upgrade.rs:318` |
| Tracing filter target | `beads_rust=…` | `src/logging.rs:88,93,95,98` (and CI `release.yml:109` `RUST_LOG: beads_rust=debug`) |
| MCP server name | `"br"` | `src/mcp/mod.rs:1225` `fastmcp_rust::Server::new("br", …)` |
| MCP server description | `"beads_rust (br) issue tracker MCP server."` | `src/mcp/mod.rs:1227` |
| `--version` text | `"br version {version} ({build})"` | `src/cli/commands/version.rs:118`, `:157`, `:207`, `:268-270` |
| Nix package | `pname = "beads_rust"`, `mainProgram = "br"` | `flake.nix:92,133,205` |
| `use beads_rust::` (crate path) | 86 files / 411 hits | VERIFIED: 64 in `tests/`, 8 in `fuzz/`, 3 in `src/`, 1 `benches/`, 4 docs, 1 `.beads/issues.jsonl` prose, 4 `sample_beads_db_files/*.jsonl` prose |
| `cargo_bin!("br")` / `CARGO_BIN_EXE_br` / `Command::new("br")` | 18 files / 130 hits | incl. `tests/common/binary_discovery.rs:105-129` |

**Default issue-ID prefix fallback is `"br"` — NOT `"bd"`** (VERIFIED, and **missing from the plan**):
- `src/util/id.rs:749` — `default_prefix: "br".to_string()`
- `src/cli/commands/init.rs:149-163` — `br init` derives the prefix from the sanitized directory name, falling back to `"br"` only if the directory name yields nothing.
- `src/cli/commands/config.rs:534` — `# issue_prefix: br` in the config template comment.
This means a fresh `obr init` in a degenerate directory would mint `br-*` IDs. Must become `obr`.

### (d) User-config paths (precedence documented at `src/config/mod.rs:3-10`)

1. CLI overrides
2. Env
3. Project `.beads/config.yaml` — `src/config/mod.rs:229-231`
4. User `~/.config/beads/config.yaml`, falling back to `~/.config/bd/config.yaml` — `src/config/mod.rs:206-216`, `:4514-4529`, duplicated at `src/cli/commands/config.rs:1384-1388`
5. Legacy user `~/.beads/config.yaml` — `src/config/mod.rs:221-225`, `:4532-4541`, `src/cli/commands/config.rs:1399`
6. DB config table
7. Defaults

Also watched for startup-cache invalidation: `src/config/mod.rs:5145-5148` lists all three user-config paths. And `src/cli/commands/config.rs:1413` asserts on the substrings `".config/beads"` / `".config/bd"`.

Cache root: `$XDG_CACHE_HOME/beads/startup` → `~/.cache/beads/startup` → `$TMPDIR/beads-startup-cache` (`src/config/mod.rs:4975-4987`).

### (e) Help / error / log strings

- `` `br `` (backticked command in doc-comments and help text): **215 files / 1,336 hits** repo-wide; **50 files / 370 hits** inside `src/`. Densest: `src/cli/commands/doctor.rs` (68), `src/cli/mod.rs` (34), `doctor_subsystems/surface.rs` (30), `src/storage/sqlite.rs` (27), `src/cli/commands/sync.rs` (20), `src/main.rs` (19).
- `"br ` (inside double-quoted runtime strings — actual user-visible text): **99 files / 945 hits**; 35 files / 310 hits in `src/`.
- Branded prose strings that are *written to disk or shown to users*: `"Initialized beads workspace in …"` (`init.rs:280`), `".beads/ directory"` step label (`init.rs:312`), `"# Beads Project Configuration"` written into `config.yaml` (`init.rs:186`), `"Tip: Set a custom prefix with \`br init --prefix <name>\`"` (`init.rs:416`), `"Run 'br init' to initialize the beads directory."` (`src/sync/mod.rs`), `"…commit the updated .beads/issues.jsonl."` (`src/cli/commands/mod.rs:104-106`).
- Doctor `--json` **check names** that embed branding (machine contract): `gitignore.beads_inner` (13), `gitignore.beads_inner_present` (14), `permissions.beads_dir` (13); failure-mode IDs `fm-configs-gitignore-leaking-beads` (7), `fm-permissions-beads-dir-readonly`. VERIFIED via `git grep -Pho '"[a-z0-9_.]*beads[a-z0-9_.]*"' -- src/cli/commands/doctor*`.
- JSON **output key** `"beads_dir"` emitted in 12 places in `src/` (`doctor.rs:8640,8663,8862,10819,10832,10852,10867`; `cli/commands/mod.rs:113`; `mcp/mod.rs:117,138`; `mcp/resources.rs:173`; `mcp/tools.rs:2893`) and asserted in 4 test files + 2 baseline JSON fixtures. **This is a surface change, not an internals change** — see §8 commit partitioning.

### (f) Shell completions and man pages

- **No man pages exist.** VERIFIED: no `clap_mangen` dependency in `Cargo.toml`; no `.1`/roff generation anywhere.
- Completions are **dynamic** (`clap_complete::env::Shells`): `src/cli/commands/completions.rs:54-68` calls `env_shell.write_registration("COMPLETE", cmd.get_name(), bin, bin, out)`. The emitted function name (`_clap_complete_br`) and compdef target derive entirely from the clap command name, so **renaming `#[command(name)]` fixes completions automatically**.
- What breaks: `tests/e2e_completions.rs:62-63` asserts `_clap_complete_br`; `:134` asserts `#compdef` or `_br`; `:31` asserts the output contains `"br"`. Doc references to `~/.zsh/completions/_br` and `~/.config/fish/completions/br.fish` at `src/cli/commands/completions.rs:11-14` and `docs/INSTALLING.md`.

### (g) Packaging

| File | Branding |
|---|---|
| `install.sh` (61 KB) | 80 branded lines. `REPO="${REPO:-beads_rust}"` `:110`; `BINARY_NAME="br"` `:112-113`; `OWNER` default `Dicklesworthstone` `:62`; `BR_INSTALL_DIR` `:479,534`; `BR_INSTALLER_{SELF_REEXEC,BRANCH,URL}`; banner text `:278,283`; help text `:452-455`; skills fetch URL `:838` |
| `packaging/aur/PKGBUILD` | `pkgname=br-bin`, `provides=('br')`, `conflicts=('br' 'br-git')`, upstream URL, `install -Dm755 "${srcdir}/br" …/usr/bin/br`, inlined MIT © Jeffrey Emanuel |
| `packaging/aur/PKGBUILD-git` | `pkgname=br-git`, `git+https://github.com/Dicklesworthstone/beads_rust.git`, `target/release/br` |
| `packaging/homebrew/br.rb` | `class Br < Formula`, asset URLs, **test block asserts `testpath/".beads"` and `.beads/beads.db`** |
| `packaging/scoop/br.json` | asset URLs, `br-0.2.10-windows_amd64.zip` |
| `.github/workflows/*` (8) | `release.yml` 50 branded lines (archive names `br-<ver>-<platform>.tar.gz` `:322,329`; `RUST_LOG: beads_rust=debug` `:109`); `ci.yml` 23; `doctor.yml` 17; `update-package-manifests.yml` 14; `conformance.yml` 8; `e2e-full.yml` 2; `notify-acfs.yml` 2; `audit.yml` 0 |
| `.githooks/pre-commit` | 12 branded lines: `.beads/` probe `:36`, `command -v br` `:41`, `BR_DOCTOR_SKIP_PRECOMMIT` `:27`, `BR_NO_AUTOFLUSH=1 br doctor --quick --json` `:50` |
| `flake.nix` | 27 branded lines (see §4c) |
| `tarpaulin.toml`, `build.rs`, `.gitignore` (28), `.rchignore` (17), `.ubsignore` (2) | package-name and artifact-path references |

Fork note: the fork already renamed `packaging/homebrew/br.rb` → `obr.rb` (fork commit `a2760e61`) but left `packaging/scoop/br.json` and both PKGBUILDs untouched. VERIFIED via `git -C /Users/johnw/src/obr diff --name-only e805fe98 main`.

### (h) Docs

41 tracked files under `docs/` plus `README.md` (45 KB, 68 `beads` + 238 `br` hits), `AGENTS.md` (42 KB, 44 + 67), `CHANGELOG.md` (69 KB, 341 + 27), `UPGRADE_LOG.md` (6). Top `docs/` offenders:

```
docs/porting/EXISTING_BEADS_STRUCTURE_AND_ARCHITECTURE.md   190
docs/porting/PROPOSED_ARCHITECTURE_FOR_BR_USING_RUST_…md      83
docs/porting/PLAN_TO_PORT_BEADS_WITH_SQLITE_AND_…md           69
docs/TROUBLESHOOTING.md 58   docs/CLI_REFERENCE.md 48   docs/AGENT_INTEGRATION.md 35
docs/INSTALLING.md 30   docs/ARCHITECTURE.md 29   docs/VCS_INTEGRATION.md 25
docs/SYNC_SAFETY.md 23   docs/E2E_SYNC_TESTS.md 15   docs/SYNC_MAINTENANCE_CHECKLIST.md 14
```

`docs/porting/` (3 files, 342 hits) are **historical porting specs describing the Go `bd` tool** — recommend dropping them wholesale rather than de-branding (they document a foreign product). Likewise `docs/audit_bd_to_br_2026_05_09.md`.

### (i) `.claude-plugin/`, skills, and the agent-emitted blurb

| Artifact | Branding |
|---|---|
| `.claude-plugin/plugin.json` | `"name": "beads"`, `"displayName": "Beads (br)"`, description referencing `br (beads_rust)`, author `Dicklesworthstone`, homepage/repo URLs, keywords `["beads","br",…]`, `"skills": ["./.claude/skills/"]` |
| `.claude-plugin/marketplace.json` | `"name": "beads-rust"`, nested plugin `"name": "beads"`, same URLs/keywords |
| `.claude/skills/br/` (5 files) | **Directory literally named `br`**; 33 branded surface hits, 176 `\bbr\b` hits |
| `skills/bd-to-br-migration/` (8 files) | Whole skill is about migrating Go `bd` → Rust `br`: `SKILL.md`, `references/{BULK,PITFALLS,TRANSFORMS}.md`, `scripts/{find-bd-refs.sh,verify-migration.sh}`, `subagents/batch-migrator.md`. 202 `\bbd\b` + 162 `\bbr\b` hits. **Recommend: delete.** It is upstream-specific migration guidance that has no meaning for `obr`. |
| `src/cli/commands/agents.rs` `AGENT_BLURB` | **The binary writes this into the user's `AGENTS.md`/`CLAUDE.md`.** Markers `<!-- br-agent-instructions-v1 -->` `:19,28` and `<!-- end-br-agent-instructions -->` `:22,93`; version-detect regex `:203` `r"<!-- br-agent-instructions-v(\d+) -->"`; body says "## Beads Workflow Integration", links `github.com/Dicklesworthstone/beads_rust`, "(`br`/`bd`)", "`.beads/`", and 15 `br …` command examples `:28-93`. **This is a created artifact → mandate-critical.** Marker rename needs a legacy-read (`br-agent-instructions-v*`) so `br agents --update` can find and replace an old block. |
| `.agent-mail.yaml`, `.ntm/logs/{am,bd,cm}-beads_rust.log` | Foreign-tool integration files; `.ntm/logs` are stale build logs |
| `agent_baseline/help/{br_help,br_list_help,br_schema_help}.txt` | Captured `--help` golden text (3 files) |

### (j) Internal identifiers

Total identifier tokens containing `beads`: **9,210 in `src/`** + **2,602 in `tests/*.rs`, `tests/common/`, `tests/e2e/`, `benches/`, `fuzz/fuzz_targets/`, `build.rs`** = **11,812**. VERIFIED. Top:

| Identifier | `src/` | tests |
|---|---|---|
| `beads_dir` (local var / field / param) | 3,791 | 697 |
| `BeadsError` | 2,216 | 5 |
| bare `beads` token | 1,991 | 1,107 |
| `beads_rust` (crate path / prose) | 339 | 401 |
| `BeadsState` | 83 | — |
| `local_beads_dir` / `canonical_beads` / `setup_test_beads_dir` / `discover_beads_dir_with_cli` | 66/63/54/42 | — |
| `beads_to_mcp` | 40 | — |

87 distinct compound identifiers of the form `*beads*` exist beyond the bare ones (VERIFIED by `git grep -Pho '\b[A-Za-z_][A-Za-z0-9_]*[Bb]eads[A-Za-z0-9_]*\b' -- src/ | sort -u | wc -l`).

Files touched by internals-only rename: **149** (76 `src/`, 64 `tests/`, 4 `docs/`, 3 `fuzz/`, `AGENTS.md`, `.beads/issues.jsonl`).

**Caveat for the mechanical sed:** `pub beads_dir: PathBuf` appears in 10 public structs (`src/config/mod.rs:178`, `src/config/routing.rs:44,55`, `src/mcp/mod.rs:356`, `src/sync/mod.rs:2000,2312,3503,3518,3535`, `src/cli/commands/doctor_subsystems/surface.rs:360`) — several of them `Serialize`. Renaming the *field* changes JSON output. Those specific renames belong in commit **(a)**, not **(c)**.

### (k) Test fixtures / snapshots that break mechanically

| Set | Files | Branded files | Hits | Break mode |
|---|---|---|---|---|
| `tests/*.rs` (165) | 165 | 137 | 2,049 | `use beads_rust::`, `cargo_bin!("br")`, `.beads` path asserts |
| `tests/common/` (13) | 13 | 12 | 248 | `binary_discovery.rs:105-129` hardcodes `BR_BINARY`, `cargo_bin!("br")`, `which("br")`, asserts `version.binary == "br"` `:437,447` |
| `tests/doctor_fixtures/` (206) | 206 | 157 | 602 | **122 shell scripts** (`corrupt.sh` / `assert.sh`) with 499 hardcoded `.beads`/`beads.db`/`.br_history`/`.br_recovery` paths. Fixture **directory names** also branded: `gitignore_leaking_beads/` |
| `tests/fixtures/` (205: 183 `workspace_failures`, 16 `json_baseline`, 6 `schema_migration`) | 205 | 71 | 195 | Baseline JSON with `"beads_dir":…` keys and `.beads/…` paths |
| `tests/snapshots/` (97, of which 79 `.snap`) | 97 | 27 (24 `.snap`) | 83 | insta goldens. Notably `golden_beads_init__init_directory_listing.snap` (lists `.beads/`, `beads.db`) and `golden_beads_init__init_text_contents.snap` (embeds the whole `.gitignore` + `metadata.json` + `# Beads Project Configuration` templates). **Filenames themselves are branded** — insta derives snapshot filenames from `tests/golden_beads_init.rs`, so the test file must be renamed too |
| `tests/e2e_scripts/` (8) | 8 | 8 | 112 | Shell e2e with `.beads`/`br` literals |
| `tests/artifacts/` (335) | 335 | 79 | 253 | **Historical perf captures** — recommend leave-as-is / delete, not rename |
| `fuzz/corpus/` (2,747) | 2,747 | 53 | — | Generated corpus; leave |
| `sample_beads_db_files/` (32) | 32 | 26 | 3,168 | Foreign-project sample DBs (asupersync, frankensqlite, ntm…). Directory name is branded. **Recommend: delete** (they are upstream's debugging corpus, 26 MB, not needed by the fork) |

**Total files under `tests/` that break mechanically on the full rename: 506** (VERIFIED: `git grep -Pl '<surface∪identity>' -- tests/ | wc -l`), broken down `tests/*.rs` 138, `tests/doctor_fixtures/` 159, `tests/artifacts/` 113, `tests/fixtures/` 71, `tests/snapshots/` 22, `tests/common/` 12, `tests/e2e_scripts/` 8, `tests/e2e/` 1. **393** if `tests/artifacts/` (historical perf captures, recommended for deletion) is excluded. See §8 for the per-commit split.

---

## 5. Findings the draft §7.6 map completely misses

### 5.1 26 versioned `br.*` JSON schema identifiers (machine contract)

VERIFIED, full list (`git grep -Pho '"br\.[a-z0-9_.-]+"' -- src/ | sort -u`):

```
br.capabilities.v1                       br.doctor.robot_docs.v1
br.coordination.v1                       br.doctor.runs_list.v1
br.doctor.capabilities                   br.doctor.schema_migration.applied.v1
br.doctor.capabilities.v1                br.doctor.schema_migration.failed.v1
br.doctor.contract.v1                    br.doctor.schema_migration.plan.v1
br.doctor.db_snapshot.v1                 br.doctor.schema_migration.prepared.v1
br.doctor.explain.v1                     br.doctor.schema_migration.undo.v1
br.doctor.health.v1                      br.doctor.triage.v1
br.graph-projections.v1                  br.doctor.undo.v1
br.jsonl-witness.v1                      br.robot_docs.v1
br.scheduler.v1                          br.sync.additive-reconciliation.v2
br.sync.reconcile.v1                     br.vcs-export-status.v2
br.write-combining.v0                    br.write-combining.v1
```

Anchors: `src/sync/witness.rs:18` (`JSONL_WITNESS_SCHEMA_VERSION`), `src/write_combining.rs:18` (`WRITE_COMBINING_SCHEMA_VERSION`), `src/cli/commands/doctor_subsystems/schema_migration.rs:49` (`PREPARED_SCHEMA`). Several are **written into on-disk JSON** (doctor run dirs, `prepared.json`, `undone.json`, `actions.jsonl`) and read back.

### 5.2 `beads://` MCP resource URI scheme

12 distinct URIs, 115 hits across 10 files (`src/mcp/{mod,resources,tools}.rs`, `src/cli/mod.rs`, `README.md`, `AGENTS.md`, `docs/{AGENT_INTEGRATION,CLI_REFERENCE,SWARM_SCALE_TUNING}.md`, `.beads/issues.jsonl`):

```
beads://schema  beads://labels  beads://project/info  beads://graph/health
beads://coordination/status  beads://events/recent  beads://issues/{id}
beads://issues/{ready,blocked,in_progress,deferred,bottlenecks}
```

Plus MCP server registration `Server::new("br", …)` at `src/mcp/mod.rs:1225` and description `"beads_rust (br) issue tracker MCP server."` at `:1227`.

### 5.3 Default issue-ID prefix fallback `"br"` — see §4c.

### 5.4 `.br_recovery/`, `bd.sock`, `.bv.lock`, `~/.cache/beads/startup`, `.beads.snapshot_*` — see §2.

### 5.5 The `AGENT_BLURB` markers written into user files — see §4i.

### 5.6 Doctor JSON check names and the `"beads_dir"` JSON key — see §4e.

### 5.7 Five hash domain separators (VERIFIED, complete)

| Constant | Value | Site | Persisted? |
|---|---|---|---|
| database write authority | `b"beads-rust-database-write-authority-v1\0"` | `src/sync/mod.rs:988` | digest only |
| jsonl write authority | `b"beads-rust-jsonl-write-authority-v1\0"` | `src/sync/mod.rs:1003` | digest only |
| jsonl witness root | `b"br:jsonl-witness:root:v1\0"` | `src/sync/witness.rs:20` | digest persisted in witness JSON |
| jsonl witness chunk | `b"br:jsonl-witness:chunk:v1\0"` | `src/sync/witness.rs:21` | digest persisted |
| startup cache key | `b"br-startup-cache-v2"` | `src/config/mod.rs:4992` | digest is the cache filename |

Plus four sync-merge domain constants at `src/sync/mod.rs:2602-2606`:
`beads-rust.sync-merge-{intent,kept-issue,receipt.immutable-envelope,receipt.state}.v1` — used only as `hasher.update` inputs (`sync_merge_domain_separated_sha256`), so the **string never lands on disk**; only its digest does.

**Recommendation:** rename all nine. Consequence (INFERRED, low risk): startup caches invalidate once (they are caches), and a `sync --merge` receipt written by a pre-rename binary fails digest verification. Mitigation: document "resolve any in-flight `sync --merge` before upgrading" in the release note. Do **not** bump the `.vN` suffixes — that would imply a format change that did not happen.

### 5.8 Upstream `.beads/config.yaml` sets `issue_prefix: beads_rust`

VERIFIED: upstream's own tracker uses prefix `beads_rust`, so **every one of its 953 issues has an ID like `beads_rust-03c5`**. The fork's tracker (`fork:.beads/issues.jsonl`, 549 issues) has the same prefix (`beads_rust-07b`, …) — see `fork:.beads/config.yaml:7-8`. This is a **created artifact carrying the beads name** and the very first thing the G-rename audit will trip over. See §8 decision.

---

## 6. Fork-side additions that carry branding into the future tree

### 6.1 `src/sync/org_bridge.rs` (fork-only, 900 lines) — VERIFIED

| Line | Branding | Disposition |
|---|---|---|
| `:1` | `//! Bridge module for converting between beads \`Issue\` format and Org-mode format.` | prose |
| `:32` | `output.push_str("#+TITLE: Beads Issues\n")` | **Written into every `issues.org`** → `#+TITLE: Obr Issues` |
| `:80` | `output.push_str(":BEADS_SCHEMA_VERSION: 1\n")` | **Written into every heading's PROPERTIES drawer** → `:OBR_SCHEMA_VERSION:`, with legacy-read of the old key |
| `:312`, `:321` | `pub const BEADS_TODO_KEYWORDS`, `BEADS_DONE_KEYWORDS` | internal → `OBR_*` |
| `:447` | comment about orgize subscript parsing of titles like `"BEADS_DIR"` | prose — update to `OBR_DIR` and re-verify the escaping test still exercises the underscore case |
| `:735-736` | `"BEADS_SCHEMA_VERSION" | _ =>` parse arm | needs to accept **both** keys |
| `:816`, `:832` | test fixture id `"bd-test"`, assert `":ID:       bd-test"` | test-only |
| `:9` and 15 more | `BeadsError` | rides the internals commit |

The `:BEADS_SCHEMA_VERSION:` property is the single highest-value Org-side rename: it appears once per issue in the fork's durable file format. Legacy-read is mandatory (existing `issues.org` files in the wild — at minimum John's own).

### 6.2 CORRECTION: the `.beads/*.md` "tracker docs" are **upstream's**, not the fork's

The brief lists `SYNC_SAFETY_INVARIANTS.md`, `SYNC_THREAT_MODEL.md`, `SYNC_CLI_FLAG_SEMANTICS.md`, `MCP_AGENT_MAIL_PATTERNS.md`, `.beads/README.md` as "fork-side additions". They are not. VERIFIED by blob-hash comparison:

```
                              fork main            upstream/main        fork point e805fe98
README.md                     50f281f0 ...=========== identical ===========... 50f281f0
MCP_AGENT_MAIL_PATTERNS.md    b5229867 ...=========== identical ===========... b5229867
SYNC_SAFETY_INVARIANTS.md     e7edc8e7             181114b2             e7edc8e7  (upstream advanced)
SYNC_THREAT_MODEL.md          ac6918f4             80b017d6             ac6918f4  (upstream advanced)
SYNC_CLI_FLAG_SEMANTICS.md    4a0ae8ab             4fda2baa             4a0ae8ab  (upstream advanced)
```

All five existed at the fork point and the fork never touched them; upstream has since advanced three. In the forward-port they arrive from `upstream/main` for free, in their *newer* form. Their combined branding is 285 surface hits / 7 files (incl. `config.yaml`, `metadata.json`) — de-brand them in the docs commit (d), and note that upstream's newer text is the base to edit, not the fork's.

### 6.3 Fork CI, flake, packaging — superseded

The forward-port branches from `upstream/main`, so `fork:.github/workflows/*` (fork commit `6a0518d0`, 6 files, br→obr in artifact/archive/release names) and `fork:packaging/homebrew/obr.rb` are **superseded**: the same edits must be re-applied to upstream's much-larger workflows. Use them as a checklist, not a patch. Fork `flake.nix` keeps `pname = "beads_rust"` (`fork:flake.nix:53,97,186,199,212`) while setting `mainProgram = "obr"` (`:90`) and `apps.name = "obr"` (`:176`) — i.e. the fork's own rename was **binary-only and incomplete**.

### 6.4 The fork tracks 5 history snapshots in git; upstream does not

VERIFIED: `git -C /Users/johnw/src/obr ls-files .beads/` lists `.beads/.br_history/issues.2026011{7_023942,7_033743,7_034616,7_035254,7_044316}.jsonl` — five tracked snapshots. Upstream's `.beads/` has none (root `.gitignore:46` ignores `.beads/.br_history/`). Phase 5 must decide whether to carry these forward into `.obr/history/` (they are pre-Org-migration JSONL snapshots of the fork's own tracker) or drop them. Recommend **drop** — they are redundant with git history of `issues.jsonl` itself, and they are the only reason the `.obr/history/` path would appear in a `git grep` of tracked files.

### 6.5 CORRECTION: `fork:CLAUDE.md:7` is stale

It says ``- **Binary name:** `br` (beads_rust)`` while `fork:Cargo.toml:13` says `name = "obr"` and `fork:src/cli/mod.rs:652` says `#[command(name = "obr", …)]`. VERIFIED. Fix in Phase 8.

---

## 7. Finalized rename map (replaces plan §7.6)

Legend for **Compat**: **LR** = legacy-read with one-line deprecation warning (never write the old name); **HARD** = no compat, breaking; **AUTO** = follows from another rename; **KEEP** = deliberately unchanged.

### 7.1 On-disk artifacts

| # | Old | New | Compat | Anchor |
|---|---|---|---|---|
| A1 | `.beads/` | `.obr/` | **LR** — open `.beads` if `.obr` absent | `src/config/mod.rs:55-57` |
| A2 | `_beads/` | `_obr/` | **LR** | same |
| A3 | `beads.db` | `obr.db` | **LR** — `resolve_db_path` falls back to `beads.db` when `metadata.json` absent AND `obr.db` missing | `src/config/mod.rs:60`, `init.rs:172` |
| A4 | `beads.jsonl` (legacy export) | *drop the name* | **LR only** — keep reading, never write | `src/config/mod.rs:64,155` |
| A5 | `issues.jsonl` default | `issues.org` (Org port) | LR chain `issues.org` → `issues.jsonl` → `beads.jsonl` | `src/config/mod.rs:62,147-162` |
| A6 | `beads.{base,left,right}.jsonl` + `.meta.json` | `merge.{base,left,right}.jsonl` + `.meta.json` | **LR** — recognise both in `EXCLUDED_JSONL_FILES` and in `doctor` merge-artifact detection | `src/config/mod.rs:72-79`, `init.rs:229-234` |
| A7 | `.beads/.br_history/` | `.obr/history/` | **LR + migrate-on-first-write** (§7.7) | `src/sync/history.rs:502` |
| A8 | `.beads/.br_recovery/` | `.obr/recovery/` | **LR** | `src/config/mod.rs:66` |
| A9 | `.beads/redirect` | `.obr/redirect` | AUTO (relative to A1) | `src/config/routing.rs:190` |
| A10 | `bd.sock` (gitignore entry) | `obr.sock` | HARD (no Rust daemon reads it; entry only) | `init.rs:240` |
| A11 | `.bv.lock` | **KEEP** | KEEP — owned by the foreign `bv` viewer, not by us | `init.rs:246-247` |
| A12 | `~/.cache/beads/startup`, `$XDG_CACHE_HOME/beads/startup`, `$TMPDIR/beads-startup-cache` | `…/obr/startup`, `$TMPDIR/obr-startup-cache` | HARD (pure cache) | `src/config/mod.rs:4979,4985,4987` |
| A13 | `.beads.snapshot_*`, `.beads_snap*` detection | add `.obr.snapshot_*`, `.obr_snap*` | **LR** — keep detecting the old names as foreign debris | `doctor.rs:2584,17523` |
| A14 | `metadata.json`, `config.yaml`, `routes.jsonl`, `.write.lock`, `.sync.lock`, `last-touched`, `sync_base.jsonl`, `deletions.jsonl`, `interactions.jsonl`, `sync-state.json`, `daemon.{lock,log,pid}`, `backups/` | **KEEP** | KEEP — already unbranded | — |
| A15 | `# Beads Project Configuration` header in generated `config.yaml` | `# obr project configuration` | HARD | `init.rs:186` |
| A16 | `.beads/.gitignore` template body | rewrite (`.br_history/`→`history/`, `.br_recovery/`→`recovery/`, `beads.*.jsonl`→`merge.*.jsonl`, `bd.sock`→`obr.sock`) | HARD | `init.rs:199-248` |
| A17 | `<!-- br-agent-instructions-vN -->` / `<!-- end-br-agent-instructions -->` markers in user `AGENTS.md` | `<!-- obr-agent-instructions-v1 -->` / `<!-- end-obr-agent-instructions -->` | **LR** — `find_marker_block_range` must also match the legacy marker so `obr agents --update` can replace an old block | `agents.rs:19,22,183,203,211,422` |

### 7.2 Environment variables

| # | Old | New | Compat |
|---|---|---|---|
| E1 | `BEADS_DIR` | `OBR_DIR` | **LR** |
| E2 | `BEADS_JSONL` | `OBR_JSONL` | **LR** |
| E3 | `BEADS_CACHE_DIR` | `OBR_CACHE_DIR` | **LR** |
| E4 | `BEADS_FLUSH_DEBOUNCE` | `OBR_FLUSH_DEBOUNCE` | **LR** |
| E5 | `BEADS_IDENTITY` | `OBR_IDENTITY` | **LR** |
| E6 | `BEADS_REMOTE_SYNC_INTERVAL` | `OBR_REMOTE_SYNC_INTERVAL` | **LR** |
| E7 | `BEADS_AUTO_START_DAEMON` | `OBR_AUTO_START_DAEMON` | **LR** |
| E8 | `BEADS_ACTOR`, `BD_ACTOR` | `OBR_ACTOR` | **LR** (both) |
| E9 | `BD_DB`, `BD_DATABASE` | `OBR_DB`, `OBR_DATABASE` | **LR** (both) |
| E10 | **`BD_<KEY>` dynamic namespace** | **`OBR_<KEY>`** | **LR** — keep stripping `BD_` too, warn once per process | `src/config/mod.rs:4345-4351` |
| E11 | `BR_OUTPUT_FORMAT` | `OBR_OUTPUT_FORMAT` | **LR** |
| E12 | `BR_AGENT_NAME`, `BR_HARNESS`, `BR_MODEL`, `BR_SESSION`, `BR_INHERITED_CONTEXT` | `OBR_*` | **LR** (clap `env=` supports only one name → add a manual pre-pass that copies legacy→new before `Cli::parse`) |
| E13 | `BR_NO_AUTOFLUSH`, `BD_NO_AUTOFLUSH` | `OBR_NO_AUTOFLUSH` | **LR** |
| E14 | `BR_STARTUP_CACHE`, `BR_STARTUP_CACHE_DIR` | `OBR_STARTUP_CACHE{,_DIR}` | LR (cheap) |
| E15 | `BR_DISABLE_READ_ONLY_FAST_OPEN`, `BR_DISABLE_PARALLEL_JSONL_EXPORT`, `BR_HISTORY_MIN_INTERVAL_SECS`, `BR_DOCTOR_STALE_LOCK_THRESHOLD_SECS`, `BR_DOCTOR_RUNS_DIR`, `BR_MCP_READ_SNAPSHOT` | `OBR_*` | HARD (undocumented internals/kill-switches) |
| E16 | the ~78 test/CI/installer vars (§3.4) | `OBR_*` | HARD |
| E17 | scrub lists at `doctor.rs:13665-13675` and `config/mod.rs:5110-5121` | add `OBR_` prefix, keep `BD_`/`BEADS_`/`BR_` | — |
| E18 | `capabilities.rs:213-237` `ENV_VARS` table | rewrite to the new names | — |

### 7.3 Crate / binary identity

| # | Old | New | Compat |
|---|---|---|---|
| I1 | package `beads_rust` | `obr` | HARD (86 files import `beads_rust::`) |
| I2 | bin `br` | `obr` | HARD |
| I3 | `#[command(name = "br")]` | `"obr"` | HARD → drives completions, help, `_clap_complete_obr` |
| I4 | `upgrade.rs:25 BIN_NAME = "br"` | `"obr"` | HARD |
| I5 | `GITHUB_REPO_OWNER = "Dicklesworthstone"`, `GITHUB_REPO_NAME = "beads_rust"` | `"jwiegley"`, `"obr"` | HARD — **also decide whether `obr upgrade`/self-update stays enabled at all** (§9) |
| I6 | release assets `br-{ver}-{plat}.tar.gz` | `obr-…` | HARD |
| I7 | tracing filter `beads_rust=…` | `obr=…` | HARD; also `release.yml:109` |
| I8 | MCP `Server::new("br", …)` | `"obr"` | HARD |
| I9 | `flake.nix` `pname`/`mainProgram`/app name | `obr` | HARD |
| I10 | `Cargo.toml` `description`, `repository`, `keywords`, `exclude` (`sample_beads_db_files/`, `.beads/`) | de-branded | HARD |

### 7.4 User-config paths

| # | Old | New | Compat |
|---|---|---|---|
| U1 | `~/.config/beads/config.yaml` | `~/.config/obr/config.yaml` | **LR** |
| U2 | `~/.config/bd/config.yaml` | — | **LR only** |
| U3 | `~/.beads/config.yaml` (legacy) | — | **LR only** |
| U4 | `.beads/config.yaml` (project) | `.obr/config.yaml` | AUTO from A1 |
| U5 | watch-path list `config/mod.rs:5145-5148` | add `~/.config/obr/config.yaml` at the head, keep the three legacy entries | — |

Precedence becomes: `~/.config/obr/` → `~/.config/beads/` → `~/.config/bd/` → `~/.beads/` (warn on any of the last three).

### 7.5 Machine-readable identifiers (new rows)

| # | Old | New | Compat |
|---|---|---|---|
| M1 | 26 × `br.*.vN` schema IDs | `obr.*.vN` | **LR for the 6 that are read back from disk** (`br.doctor.schema_migration.{plan,prepared,applied,failed,undo}.v1`, `br.doctor.undo.v1`); HARD for the rest |
| M2 | `beads://…` MCP resource URIs (12) | `obr://…` | HARD (MCP clients rebind per session) |
| M3 | JSON key `"beads_dir"` (12 emit sites) | `"obr_dir"` | HARD |
| M4 | doctor check names `gitignore.beads_inner{,_present}`, `permissions.beads_dir` | `gitignore.obr_inner{,_present}`, `permissions.obr_dir` | HARD |
| M5 | failure-mode IDs `fm-configs-gitignore-leaking-beads`, `fm-permissions-beads-dir-readonly` | `…-obr` | HARD |
| M6 | 9 hash domain separators (§5.7) | `obr…` | HARD (see §5.7 note) |
| M7 | `:BEADS_SCHEMA_VERSION:` Org property | `:OBR_SCHEMA_VERSION:` | **LR** (fork-side, §6.1) |
| M8 | `#+TITLE: Beads Issues` | `#+TITLE: Obr Issues` | HARD (cosmetic) |
| M9 | default ID prefix fallback `"br"` | `"obr"` | HARD |
| M10 | `Cargo.toml` `[lints]`, error codes, DB table/column names | **KEEP** — verified unbranded (`src/error/structured.rs:149-190`, `src/storage/schema.rs`) | KEEP |

### 7.6 Deletions (recommended, rather than de-branding)

| Path | Files | Why |
|---|---|---|
| `skills/bd-to-br-migration/` | 8 | Migration guide for a foreign product (Go `bd` → Rust `br`) |
| `sample_beads_db_files/` | 32 (~26 MB) | Upstream's debugging corpus from unrelated projects |
| `docs/porting/` | 3 (342 hits) | Historical specs describing the Go `bd` implementation |
| `docs/audit_bd_to_br_2026_05_09.md` | 1 | Upstream audit of its own migration |
| `.ntm/logs/{am,bd,cm}-beads_rust.log` | 3 | Stale build logs |
| `temp_test/`, `temp_test_2/`, `custom.db` | 9 | Junk already flagged by plan §5 Phase 0 |
| `refactor/artifacts/` | 63 | Upstream's historical refactor ledgers |
| `tests/artifacts/perf/` | 335 | Historical perf captures with branded directory names |

Deleting these removes **~450 files and ~4,400 branding hits** from the audit surface for free, and shrinks commit (d) dramatically.

### 7.7 `.obr/history/` vs `.obr_history/` — RECOMMENDATION: **`.obr/history/`**

The plan frames this as a choice between a subdirectory and a sibling. That is a mis-framing: `.br_history` is **already** created inside the workspace dir (`beads_dir.join(".br_history")`, `src/sync/history.rs:502`). The genuine choice is the *name inside `.obr/`*.

Recommend **`.obr/history/`** (undotted, unprefixed), and by symmetry **`.obr/recovery/`**:

1. The parent is already a dot-directory; a second dot buys no hiding and makes `ls .obr` output inconsistent (`config.yaml`, `metadata.json`, `issues.org`, `.history/`).
2. The `.obr/.gitignore` template becomes readable: `history/`, `recovery/` instead of `.obr_history/`, `.obr_recovery/`.
3. It removes the redundant tool-name prefix, which is exactly what the rename is for.
4. **Do not** put history outside `.obr/` — 4 files hardcode the workspace-relative location (`src/sync/history.rs`, `src/cli/commands/history.rs`, `doctor.rs` recovery-artifact-age check, `init.rs` gitignore template), and `sync/path.rs`'s "refuse to write outside `.beads`" allowlist (`src/sync/path.rs`, `"Refusing to {operation} outside .beads: {}"`) would reject writes to a sibling directory.

Collision check (VERIFIED): root `.gitignore:142` already has `.beads/recovery*/` — after the rename it must become `.obr/recovery/` **exactly** (not `recovery*/`), or the new history/recovery dirs and any legitimately tracked file starting with `recovery` become ambiguous.

Migration: on first write, if `.obr/.br_history/` (or `.beads/.br_history/` under the LR path) exists and `.obr/history/` does not, `fs::rename` it and emit the one-line deprecation. Same for recovery. Snapshot filenames inside are `issues.<YYYYMMDD_HHMMSS>.jsonl` — unbranded, no change (`src/sync/history.rs:1156` lists with prefix `"issues."`).

### 7.8 `obr.db` vs keeping `beads.db` inside `.obr/` — RECOMMENDATION: **`obr.db`** (plan is right)

Validation (VERIFIED):
- The literal `beads.db` is *only* a fallback. The live value comes from `.beads/metadata.json`'s `database` field (`Metadata::load`, `src/config/mod.rs:119-135`; `resolve_db_path`). Every workspace `br init` ever created has an explicit `metadata.json` naming `beads.db` (`init.rs:171-174`), so **legacy workspaces keep working with zero shim code**.
- Sidecar handling (`database_family_paths`, `remove_db_sidecars`, `copy_database_family_to_directory`, `src/config/mod.rs:2392,2777,2787`) derives everything from `db_path`, so `obr.db-wal`/`-shm`/`-journal` follow automatically.
- `.obr/.gitignore` uses `*.db` globs (`init.rs:200-205`) — unaffected.
- The only shim needed: in `resolve_db_path`, when `metadata.json` is absent/blank **and** `obr.db` does not exist **and** `beads.db` does, use `beads.db` + warn. ~8 lines.
- Counter-argument considered and rejected: "hide `beads.db` inside `.obr/` so nothing breaks" — the file is user-visible in `git status`, in `obr where`/`obr info` output, in `doctor` messages, and in the `.gitignore` template. Keeping it violates the mandate for zero benefit.

**Also rename (same commit):** `sample_beads_db_files/` is excluded in `Cargo.toml:15` — if you keep that directory, rename it; recommended instead to delete it (§7.6).

---

## 8. Blast radius per Phase-6 commit

Sets computed by `git grep -Pl <pattern>` over tracked files excluding `Cargo.lock`, `*.db*`, `fuzz/corpus/`, `sample_beads_db_files/`, `tests/artifacts/`, `refactor/`, `.ntm/`, `CHANGELOG.md`, `temp_test*/`, `agent_baseline/`. Union of all three code sets = **588 files**; pairwise overlaps A∩B=164, A∩C=116, B∩C=97. (VERIFIED.)

### Commit (a) — artifact/env/CLI surface + compat shims

**473 files** total; if you defer the fixture churn to a companion commit, the *production-code* part is **54 `src/` files**.

| Area | Files | Notes |
|---|---|---|
| `src/` | 54 | Hot spots: `config/mod.rs` (9,513 ln), `cli/commands/doctor.rs` (24,752 ln), `sync/mod.rs` (21,609 ln), `sync/path.rs`, `sync/history.rs`, `cli/commands/init.rs`, `util/mod.rs`, `mcp/*`, `cli/commands/{agents,capabilities,upgrade,where,info,config}.rs` |
| `tests/*.rs` | 97 | |
| `tests/doctor_fixtures/` | 157 | **122 shell scripts**, 499 hardcoded paths — the single biggest mechanical chunk |
| `tests/fixtures/` | 67 | JSON baselines with `"beads_dir"` and `.beads/…` |
| `tests/snapshots/` | 22 | 24 `.snap` files + `golden_beads_init.rs` must be renamed (insta derives filenames from the test module path) |
| `tests/common/`, `tests/e2e_scripts/` | 15 | |
| `docs/`, `scripts/`, `.github/`, `.claude/`, `skills/`, `.beads/`, `packaging/`, `fuzz/fuzz_targets/`, root ignore files | 61 | |

**Reviewability advice:** split (a) into **(a1) production shims + surface** (54 `src/` files, ~1,957 hits) and **(a2) fixture/snapshot mechanical update** (~380 files, near-zero judgement). a1 is the only part that needs real review.

### Commit (b) — crate/binary identity + Cargo/flake/CI

**266 files.** Of these, only ~30 need thought; the rest is `s/beads_rust::/obr::/`.

```
tests 138   src 49   docs 30   fuzz 10   scripts 7   skills 5   packaging 4   .github 4
.beads 4   .githooks 2   .claude-plugin 2   .claude 1   benches 1
+ Cargo.toml, Cargo.lock, build.rs, flake.nix, tarpaulin.toml, install.sh,
  README.md, AGENTS.md, UPGRADE_LOG.md, .gitignore
```

Judgement sites: `Cargo.toml` (name/bin/description/repo/exclude), `src/cli/commands/mod.rs:61-62`, `src/cli/commands/upgrade.rs:25`, `src/logging.rs:82-98`, `src/mcp/mod.rs:1225-1227`, `src/cli/mod.rs:692`, `flake.nix`, 8 workflows, `install.sh`, 4 packaging manifests.

### Commit (c) — internal identifiers

**149 files** (`src/` 76, `tests/` 64, `docs/` 4, `fuzz/` 3, `AGENTS.md`, `.beads/issues.jsonl`), **11,812 identifier tokens**. Pure `sed` + `cargo fmt`, one review pass to confirm no *string literal* changed (the surface strings were already handled in (a)) and that the 10 `Serialize`-able `pub beads_dir` fields were **not** re-renamed here (they moved to (a)).

Suggested sed order (longest-first to avoid partial rewrites):
`setup_test_beads_dir` → `discover_beads_dir_*` → `*_beads_dir` → `BeadsError` → `BeadsState` → `beads_to_mcp` → `beads_dir` → bare `beads`.

### Commit (d) — docs / packaging / skills / plugin

**94 files** with any branding across `docs/`, `README.md`, `AGENTS.md`, `skills/`, `.claude/`, `.claude-plugin/`, `packaging/`, `install.sh`, `.githooks/`, `agent_baseline/`, `UPGRADE_LOG.md`, `.github/`, `scripts/`. Drops to **~55** if the §7.6 deletions land first (recommended as commit **(d0)**).

Deliberately **not** renamed in (d): `CHANGELOG.md` (341 hits — upstream's historical release notes; keep verbatim as provenance, or truncate with a note), `docs/operations/UPGRADE_LOG.md`, `refactor/`, `.ntm/`.

---

## 9. G-rename audit — exact gate (replaces plan §7.8 item G-rename)

The plan's proposed gate

```bash
git grep -inE '(beads|\bbd\b)' -- ':!docs/research' ':!CHANGELOG.md'
```

**does not work as written**: `git grep -E` is POSIX ERE, where `\b` is a literal `b`, so `\bbd\b` never matches. Use `-P`. And with `-P` it returns ~4,000 lines of test-fixture issue IDs. Replace with:

```bash
# ---- G-rename, part 1: no beads branding outside the allowlist -------------
git grep -Pn -i 'beads' -- \
  ':!docs/research' ':!CHANGELOG.md' ':!docs/operations/UPGRADE_LOG.md' \
  ':!Cargo.lock' ':!*.db' ':!*.db-wal' ':!*.db-shm' ':!fuzz/corpus' \
| grep -vE '^(README\.md|LICENSE(\.md)?|docs/ORIGINS\.md):' \
| grep -vE 'LEGACY_|legacy_|_legacy|legacy ' \
| grep -vE '^\.obr/issues\.org:'          # tracker data: see decision D3
# EXPECT: empty.

# ---- G-rename, part 2: no br/bd binary or artifact names -------------------
git grep -Pn '(^|[^a-zA-Z0-9_/.-])(br|bd)([^a-zA-Z0-9_-]|$)' -- \
  ':!docs/research' ':!CHANGELOG.md' ':!Cargo.lock' ':!*.db*' ':!fuzz/corpus' \
| grep -vE '(^|[^a-z])(br|bd)-[0-9a-z]'   # test-fixture issue IDs, see A-list
| grep -vE '\.br_history|\.br_recovery|BR_|BD_'   # only if a legacy shim line
# EXPECT: only lines inside the legacy-compat shim functions.

# ---- G-rename, part 3: hard artifact names --------------------------------
git grep -Pn '\.beads\b|_beads\b|beads\.db|beads\.jsonl|BEADS_[A-Z]|beads://|"br\.[a-z]|beads-rust' -- \
  ':!docs/research' ':!CHANGELOG.md' ':!Cargo.lock' ':!*.db*' ':!fuzz/corpus'
# EXPECT: only legacy-compat shim code + its tests (see allowlist L1-L6).

# ---- G-rename, part 4: the binary itself ----------------------------------
test -x target/debug/obr && ! test -e target/debug/br
target/debug/obr --help | grep -Piv 'obr' | grep -Pi '\bbeads\b|\bbr\b|\bbd\b'   # EXPECT empty
target/debug/obr --version | grep -Pi 'beads|\bbr\b'                            # EXPECT empty
target/debug/obr capabilities --format json | grep -Pi 'beads|"br\.|BD_|BR_'    # EXPECT empty
target/debug/obr robot-docs guide | grep -Pi '\bbeads\b|\bbd\b'                 # EXPECT empty
# fresh-workspace artifact check
cd "$(mktemp -d)" && obr init && find . -name '*beads*' -o -name '.br_*' | grep . && echo FAIL
```

### Allowlist (the ONLY permitted residual hits)

| ID | Where | What is allowed |
|---|---|---|
| **L1** | `README.md` § *Origins* | Credit to Steve Yegge's beads (concept) and Dicklesworthstone's beads_rust (codebase), with links. The one sanctioned prose mention. |
| **L2** | `LICENSE` / `LICENSE.md` | Upstream copyright + rider text as required attribution (© Jeffrey Emanuel). |
| **L3** | `src/config/mod.rs` legacy-discovery block | `is_legacy_workspace_dir_name` (`.beads`/`_beads`), `LEGACY_DB_FILENAME = "beads.db"`, `LEGACY_JSONL_FILENAME`s, legacy user-config paths, legacy merge-artifact names. Must all sit in one clearly-marked `mod legacy_compat` region. |
| **L4** | `src/config/mod.rs` / `src/cli/mod.rs` legacy-env block | `BEADS_*`, `BD_*`, `BR_*` accepted-with-warning names, plus the `BD_`/`BEADS_` prefixes in the doctor env-scrub list. |
| **L5** | `src/cli/commands/agents.rs` | `"<!-- br-agent-instructions-v"` legacy marker prefix used by `find_marker_block_range`. |
| **L6** | `src/sync/org_bridge.rs` | `"BEADS_SCHEMA_VERSION"` accepted as a legacy PROPERTIES key. |
| **L7** | tests named `*_legacy_*` / `legacy_compat*` | Tests that exercise L3–L6. Recommend one file, `tests/legacy_compat.rs`, so the allowlist is a single path. |
| **L8** | `CHANGELOG.md`, `docs/operations/UPGRADE_LOG.md`, `docs/research/**`, `UPGRADE_LOG.md` | Historical records; excluded by pathspec. |
| **L9** | `.obr/issues.org` (own tracker) | **Only if decision D3 chooses "keep historical IDs".** Otherwise remove this exemption. |
| **L10** | `tests/**` issue-ID literals matching `(br|bd)-[0-9a-z]` | Opaque fixture IDs. Excluded by the part-2 filter. Note `id` is **excluded from `content_hash`** (`src/util/hash.rs:62-63`), so these could be renamed safely — but doing so touches ~3,400 lines for zero user-visible gain. |

---

## 10. Corrections to the plan / dossier

1. **`git grep -E '\bbd\b'` (plan §7.8 G-rename) is broken.** POSIX ERE has no `\b`; the pattern matches literal `bbdb`. Must be `-P`. VERIFIED.
2. **`.br_history` is inside `.beads/`, not a sibling** — the §7.6 row "`.br_history/` → `.obr/history/` or `.obr_history/`" mis-frames the decision. See §7.7.
3. **`.beads/*.md` tracker docs are upstream's, not the fork's** (brief item 2 and plan §7.4/Phase 5 both imply otherwise). Blob-hash proof in §6.2.
4. **`.br_recovery/` is missing from the plan** and is a live created artifact (`RECOVERY_DIR_NAME`, `src/config/mod.rs:66`), with its own gitignore entry, doctor age-check, and `.rchignore` rules.
5. **The default issue-ID prefix fallback is `"br"`, not `"bd"`** (`src/util/id.rs:749`, `src/cli/commands/init.rs:150`). The plan's env table mentions `BD_*` but never the prefix.
6. **The plan's rename map has no row for**: 26 `br.*` schema IDs, `beads://` MCP URIs, the `"beads_dir"` JSON key, doctor check names, `bd.sock`, `.bv.lock`, `~/.cache/beads/startup`, `.beads.snapshot_*`, `beads.{base,left,right}.jsonl`, `beads.jsonl` legacy, the `AGENT_BLURB` markers, the `BD_<KEY>` dynamic namespace, and the 9 hash domain separators.
7. **Plan §3 row "Upstream identity … `issues.jsonl` default" is right, but the accompanying `src/config/mod.rs:62` line reference in Phase 3 step 3 should be `:62` for `DEFAULT_JSONL_FILENAME` and `:147-162` for `discover_jsonl`** — the plan's "`:62` area" and "`:100-116`" figures are the *fork's* line numbers, not upstream's (upstream `discover_jsonl` is at 147).
8. **`fork:CLAUDE.md:7` is stale** — says binary is `br`; it is `obr`. (§6.5)
9. **The fork's own rename was binary-only.** `fork:Cargo.toml:2` still says `name = "beads_rust"`; `fork:flake.nix:53,97,186,199,212` still says `pname = "beads_rust"`. Plan §3.1 classifies `a2760e61` as "Rename (superseded)" which is right, but the plan should not assume any of the `.beads`/env/crate surface was ever touched. VERIFIED: `a2760e61` touched 55 files, all binary-name only.
10. **No deprecation-warning machinery exists upstream** (`git grep -in deprecat -- src/` → 0 hits). Every "with warning" row in §7 requires new code. Use `report_auto_flush_failure` (`src/cli/commands/mod.rs:81-132`) as the JSON-safe template.
11. **No man pages** — plan §7.6 lists "man pages" as a rename target; there are none, and completions are automatic from the clap name.
12. **Dossier `map-cli.md` env list is accurate for the fork** but incomplete for upstream: upstream added `BR_STARTUP_CACHE{,_DIR}`, `BR_DOCTOR_RUNS_DIR`, `BR_MCP_READ_SNAPSHOT`, `BR_DISABLE_READ_ONLY_FAST_OPEN`, `BR_DISABLE_PARALLEL_JSONL_EXPORT`, `BR_HISTORY_MIN_INTERVAL_SECS`, `BR_DOCTOR_STALE_LOCK_THRESHOLD_SECS`, `BR_{AGENT_NAME,HARNESS,MODEL,SESSION,INHERITED_CONTEXT}`, `BEADS_AUTO_START_DAEMON`.

---

## 11. User decisions this brief surfaces

**D1 — Self-update.** `obr upgrade` currently points at `Dicklesworthstone/beads_rust` releases (`src/cli/commands/mod.rs:61-62`, feature `self_update` is **on by default**, `Cargo.toml`). After the rename it would either (a) point at `jwiegley/obr` (requires publishing signed releases with minisign, `release.yml`), or (b) be **disabled** by dropping `self_update` from `default` features and removing `upgrade`/`packaging/`/`install.sh`. Option (b) removes ~13 files and ~230 branding hits and matches an unpublished personal fork. **Recommend (b); needs the user.**

**D2 — `.claude-plugin/` + `.claude/skills/br/` + `skills/`.** Adopt de-branded (implies maintaining an `obr` Claude plugin/marketplace) or drop? R3 also owns this; from a branding standpoint dropping `skills/bd-to-br-migration/` is unambiguous, while `.claude/skills/br/` → `.claude/skills/obr/` is cheap and useful.

**D3 — The fork's own tracker issue-ID prefix.** `fork:.beads/config.yaml:7-8` sets `issue_prefix: beads_rust`; all 549 issues are `beads_rust-*`. Options:
 - **(i)** Re-prefix during the Phase-5 conversion: set `issue-prefix: obr` in `.obr/config.yaml`, then `obr sync --import-only --rebuild --rename-prefix` (`src/cli/commands/sync.rs:3252-3254` → `ImportConfig{ rename_on_import: true }` → `build_prefix_renames` at `src/sync/mod.rs:12586`). VERIFIED that the mechanism exists and remaps dependency references through the rename map applied in `stream_import_actions_in_tx`. **NOT VERIFIED**: whether it rewrites ID references embedded in *prose* (title/description/notes/comments) — the Go spec says it does (`docs/porting/EXISTING_BEADS_STRUCTURE_AND_ARCHITECTURE.md:3610`) but I did not confirm the Rust implementation. Needs a dry-run.
 - **(ii)** Keep historical IDs, allowlist `.obr/issues.org` (allowlist row L9).
 **Recommend (i) with a verified dry-run**, falling back to (ii) if prose references are not rewritten.

**D4 — `.gitignore` glob narrowing.** Root `.gitignore:142` has `.beads/recovery*/`. After A8 the recovery dir *is* `recovery/`, so the glob must be tightened to an exact path or it silently swallows future files. Cosmetic but easy to get wrong.

**D5 — Hash domain separators (§5.7).** Rename (recommended; invalidates startup caches once and breaks in-flight `sync --merge` receipt verification) vs keep (violates the letter of the mandate for strings that never reach disk). Low stakes either way, but it is a judgement the user may want to make.

**D6 — CHANGELOG.md.** 341 `beads` hits of upstream release history. Keep verbatim as provenance (excluded from the audit), truncate with a "history before the fork lives upstream" note, or delete. **Recommend: keep, excluded.**
