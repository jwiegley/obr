# P6-06 · feat!: relocate the human surface to PLAN.org (D-SURFACE)

**Authority:** user decision 2026-08-07 (DECISIONS.md D-SURFACE). Supersedes U4 and the §8.2 file layout.
**Model:** `.obr/` holds only per-machine cache (SQLite db, config, metadata, history/, recovery/, locks, merge artifacts) and is entirely git-ignored via a self-ignoring `.obr/.gitignore` containing `*`. The tracked human surface is `PLAN.org` in the first existing directory of the ordered probe list **`doc/`, then `docs/`, then the workspace root** (USER RULING 2026-08-07, amending the earlier "doc/ literal" reading). The highest-priority existing PLAN.org wins; a shadowed lower-priority PLAN.org triggers a one-shot JSON-safe warning; never merge.

## Resolution rule (read AND write target)

1. `metadata.json.jsonl_export` set to a NON-default value → explicit override, resolved relative to `.obr/` exactly as today (covers pinned-JSONL tests and power users; external paths keep the `--allow-external-jsonl` policy).
2. Defaulted (`PLAN.org`, the legacy default sentinel `issues.org`, or missing metadata):
   a. If the computed surface exists → use it: `<root>/doc/PLAN.org` if `<root>/doc/` is a directory, else `<root>/PLAN.org`.
   b. LEGACY (LR, warn once per process): if the computed surface does NOT exist but `.obr/issues.org` / `.obr/issues.jsonl` / `.obr/beads.jsonl` does → keep using that in-dir artifact (no forced migration; existing workspaces work unchanged).
   c. Neither exists (fresh) → the computed surface location.
3. If BOTH `doc/PLAN.org` and `./PLAN.org` exist → `doc/PLAN.org` wins; warn about the shadowed root file. Never merge.

`default_jsonl_export()` returns `"PLAN.org"`. `resolve` treats both `"PLAN.org"` and the old default `"issues.org"` as "defaulted" (value == either sentinel → rule 2; anything else → rule 1).

## init

- Creates `.obr/` containing `obr.db`, `config.yaml`, `metadata.json` (`jsonl_export: "PLAN.org"`), and a `.gitignore` whose entire content is `*` (plus a one-line comment) — the directory ignores itself; nothing under `.obr/` is ever tracked.
- Seeds the surface file at the computed location (rule 2) with the Org header; does NOT create `.obr/issues.org`.
- Never creates `doc/` — only uses it if present.

## Prefix bootstrap (fresh clones without .obr)

`org_file_header` gains `#+ISSUE_PREFIX: <prefix>` (written whenever the prefix is known; read on import as the authoritative prefix when `.obr/config.yaml` is absent). Reader accepts files without the keyword (existing corpora).

**Bootstrap flow (RULED 2026-08-07):** `PLAN.org` is NOT a workspace marker — it is far too generic a filename, and auto-creating `.obr/` beside any unrelated `PLAN.org` would be invasive. The documented fresh-clone flow is `obr init && obr sync --import-only --rebuild`; the `#+ISSUE_PREFIX` keyword is authoritative over init's directory-derived prefix, so identity survives the clone. The bootstrap test includes the `init` step.

## Sync path allowlist (src/sync/path.rs)

- The resolved DEFAULT surface path is first-class internal: allowed WITHOUT `--allow-external-jsonl`, but narrowly — exactly basename `PLAN.org` at `<root>` or `<root>/doc/`, with the same symlink/canonicalization hardening applied against the workspace root. Arbitrary out-of-dot-dir paths remain external and opt-in.
- Temp/backup naming (`PLAN.org.<pid>.tmp`) follows the existing tmp-stripping in ExportFormat::for_path (extension-driven, already format-correct).
- Conflict-marker scan, tombstone filter, witness derivation: all operate on the resolved path; no format assumptions change (PLAN.org is `.org` → Org by extension).

## Doctor

- `sync_jsonl_path` check: surface location per the rule is in-allowlist.
- Gitignore checks invert for the new model: `.obr/` must be fully ignored (the self-ignoring `.gitignore` satisfies this; also accept a root `.gitignore` covering `.obr/`), and the SURFACE file must NOT be ignored. Root-gitignore-hides-workspace check (ROOT_GITIGNORE_OFFENDING_PATTERNS) now means "hides PLAN.org", not "hides .obr" — .obr SHOULD be hidden.
- Health/where/info outputs report the surface path.

## Unchanged

Merge artifacts/history/recovery/locks stay inside `.obr/`. Explicit `--jsonl` flag and OBR_JSONL env semantics. Legacy `.beads` workspace compat (all P6-01a LR rows). JSONL-by-extension support.

## Tests

New `tests/e2e_plan_surface.rs`:
- fresh init seeds `./PLAN.org` (no doc/), `doc/PLAN.org` (with doc/); `.obr/.gitignore` == `*` variant; no `.obr/issues.org`.
- create → flush → PLAN.org contains the issue; import round-trip; `--no-db show` works off PLAN.org.
- legacy workspace (`.obr/issues.org` present, metadata defaulted) keeps its in-dir surface, warns once, flush does NOT create PLAN.org.
- pinned metadata (`issues.jsonl`) unaffected (Class A tests stay green).
- shadowing: both files present → doc/PLAN.org used, warning emitted, JSON-safe.
- prefix bootstrap: init --prefix xyz, flush, rm -rf .obr, import --rebuild → new issues get xyz- prefix from `#+ISSUE_PREFIX`.
- doctor: healthy on the new layout; flags a tracked `.obr` file; flags an ignored PLAN.org.
- git-safety: flush writes ONLY PLAN.org outside `.obr/` (extend the allowed-files assertion).

Update in the same commit: legacy_compat rows that assert "flush writes issues.org" (now: legacy workspaces keep issues.org; FRESH workspaces write PLAN.org — split the assertions), golden_obr_init snapshots (new listing), any doctor fixtures asserting the old gitignore template.
