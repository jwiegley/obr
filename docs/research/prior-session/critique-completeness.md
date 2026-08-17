# Completeness critique of the obr research dossier

**Role:** completeness critic. **Date:** 2026-08-06.
**Inputs read:** `DOSSIER.md` (1,604 lines) in full for §1–§4, §5 R1–R27, §6, §7 Q1–Q10, §8, §9;
plus targeted reads of the 17 contributing notes files, plus first-hand verification against
`/Users/johnw/src/obr` (treated read-only).

**Method.** Two passes.
1. *Mechanical coverage diff.* For every `.rs` file under `src/`, counted occurrences of its
   path and of its basename across all 18 notes files, to find files nobody cited.
2. *Claim spot-check.* Took the dossier's most load-bearing negative claims (dead code, "no
   recovery story", "no injection surface", the git-safety boundary) and re-derived them from
   source. Two are wrong; two are unexamined.

The dossier is, on the whole, unusually good: the empirical reproductions (R1, R3, R5, R6, R8,
R9, R12, R13) are first-hand and the §9 contradiction resolutions are genuinely adjudicated.
What follows is only what is *missing*.

---

## Coverage diff — files no agent cited

Path-string citation counts across all notes (`grep -ohF "<path>" *.md | wc -l`), lowest first:

| citations | file | lines |
|---:|---|---:|
| 0 | `src/cli/commands/agents.rs` | 1054 |
| 0 | `src/cli/commands/audit.rs` | 712 |
| 0 | `src/cli/commands/blocked.rs` | 697 |
| 0 | `src/cli/commands/close.rs` | 678 |
| 0 | `src/cli/commands/defer.rs` | 663 |
| 0 | `src/cli/commands/epic.rs` | 550 |
| 0 | `src/cli/commands/reopen.rs` | — |
| 0 | `src/cli/commands/stale.rs` | — |
| 0 | `src/cli/commands/version.rs` | — |
| 0 | `src/cli/commands/where.rs` | — |
| 1 | `src/cli/commands/query.rs` (907), `q.rs` (165), `src/util/markdown_import.rs` (527) | |
| 2 | `src/config/routing.rs` (510), `src/cli/commands/graph.rs` (1136), `upgrade.rs` | |

Basename mentions raise several of these to 3–8 (`blocked.rs` 8, `close.rs` 5), i.e. they were
*named* in command inventories but not *read*. The two whose absence is materially load-bearing
are `agents.rs` (Gap 2) and `config/routing.rs` (Gap 1).

Two things I checked and am satisfied are NOT gaps, so a gap-fill agent should not spend time on
them:

- **SQL injection.** Every dynamically assembled `WHERE`/`ORDER BY` in `src/storage/sqlite.rs`
  appends only literal fragments with `?` placeholders (`:812-902`, `:988-1031`, `:1082-1204`);
  the one `format!`-built UPDATE (`:622`) interpolates `set_clauses` drawn from a fixed internal
  list, and `:3448` interpolates only a generated `?,?,…` placeholder string. `query.rs`'s
  `format!` calls build config-table *keys*, not SQL. No injection surface. The dossier's silence
  here is correct.
- **`obr upgrade` signature verification.** `upgrade.rs:249,268` pass `.verifying_keys(...)`, so
  the self-updater is not an unsigned-download hole. The dossier's actual finding — that it
  points at `Dicklesworthstone/beads_rust`, a *different tool with a different storage format*
  (§6) — is the real defect and is already recorded.

---

## Gap 1 — `config/routing.rs` is live on every command's hot path, and the dossier calls it dead

**The dossier's claim (R27, "Large dead surface"):**
> "~400 lines of `config/routing.rs` (the Gastown mayor/town routing, explicitly excluded by the
> porting plan, ported anyway and abandoned): 8 of 10 public items have zero external references."

**This is false for the item that matters.** `routing::follow_redirects` is called from
`discover_beads_dir_with_env` at **three** sites — `src/config/mod.rs:214`, `:220`, `:233` —
i.e. on the `BEADS_DIR` env path, and on the upward-walk path that terminates every ordinary
workspace discovery. `src/cli/commands/where.rs:4` imports it directly. Workspace discovery is
the first thing essentially every one of the 39 commands does, and it is also what
`run_auto_flush` does (`src/main.rs:261`, the R8 mechanism). So routing is not dead code; it is
in front of every read and every write.

**What that means, and what nobody traced:**

`follow_redirects` (`src/config/routing.rs:200-232`) reads a `.beads/redirect` file, treats its
contents as a path, and iterates up to `max_depth = 10`, with a `visited` list that catches
loops (`:208-214`) and an `is_dir()` existence check at the end (`:224-229`). There is **no other
constraint on the target**: not that it is under the repo, not that it is not absolute, not that
it is not `../..`, not that it is not somewhere else on the filesystem entirely. `read_redirect`
was not read by any agent.

`.beads/redirect` is an ordinary file in the working tree, therefore *git-trackable*. Combine
that with auto-flush-after-every-mutation (R16) and the export path derivation (R8) and the
untested hypothesis is: **cloning a repository that carries a checked-in `.beads/redirect` makes
every subsequent `obr` mutation write into whatever directory that file names.** That is exactly
the class of thing the entire `src/sync/path.rs` safety layer exists to prevent, and
`validate_sync_path` cannot see it, because by the time sync runs, the redirected directory *is*
the beads directory — the allowlist checks the file's *extension and name*, not whether the
directory is the local one.

The sibling mechanism, `routes.jsonl` prefix routing (`resolve_route`, `:234-319`, plus
`find_town_root`'s unbounded upward walk for `mayor/town.json` at `:82-100`), has the same
property one level up: a route entry maps an id prefix to an arbitrary path, `follow_redirects`
is applied to the result (`:309`), and `is_external` is computed but the dossier never asks what
*consumes* that flag or whether an external target is written to.

**Why the dossier would fail without this.** It asserts (§4.1, §2.3) that the git-safety layer is
"the best-designed part of the project" and "mechanically enforced", and it lists routing as dead
weight to delete. Both statements are affected. If a redirect can steer writes, the safety layer
has an unguarded upstream input; if routing is live, R27 is over-counting the dead surface and a
reader acting on R27 would delete a load-bearing module.

**Corroborating evidence the dossier missed:** `tests/e2e_routing.rs` is **595 lines** and its
header (`:1-8`) lists "Prefix-based route lookup (routes.jsonl) / Redirect file following /
Redirect loop detection / **External DB reference safety and path normalization** / Clear errors
for missing/invalid routes". Somebody wrote a dedicated safety suite for this feature. No notes
file cites that test file's contents. That suite is also inside the 97 targets R14 says are never
executed — so a safety property has a test that is written, compiled, and never run.

---

## Gap 2 — `obr agents` (1,054 lines): the one command that writes files outside `.beads/`, unread

Zero citations anywhere in the corpus. It is the third-largest command module and the only one
that mutates files in the user's source tree.

What it does, from a first read of `src/cli/commands/agents.rs`:

- `detect_agent_file(work_dir)` (`:169`) and `detect_agent_file_in_parents(work_dir, max_levels)`
  (`:225`) locate an `AGENTS.md` / `CLAUDE.md`, **walking up parent directories**. So the target
  can be outside the repository.
- `execute_add` (`:465`) does `fs::write(&file_path, &new_content)` (`:545`) — a whole-file
  rewrite — after copying the original to `file_path.with_extension("md.bak")` (`:533-535`). The
  backup is a fixed name, so a second run **clobbers the only backup**, and a `fs::copy` failure
  is downgraded to `eprintln!("Warning: …")` and the write proceeds anyway (`:536-543`).
- `execute_remove` (`:562`) performs string surgery on the user's file via `remove_blurb` /
  `remove_legacy_blurb` (`:261`, `:293`) and writes the result back.
- `execute_add` **prompts on stdin**: `print!("Continue? [y/N] "); io::stdin().read_line(...)`
  (`:521-531`), gated only on `!force && !detection.found()`.
- The emitted help text still says `br agents --update` (`:443,449`), and `AGENT_BLURB` is
  versioned by a `BLURB_VERSION` constant with legacy-`bv`-format migration logic (`:143-167`).

**Four dossier claims this contradicts or leaves untested:**

1. **§4.11 "Destructive commands never prompt … Exactly right for agents."** `obr agents --add`
   prompts. For the agent audience this is not a UX wart, it is a hang: a non-interactive child
   process with stdin closed hits `read_line`, and the `[y/N]` prompt goes to *stdout*, which
   means it also corrupts any structured stdout an agent is parsing.
2. **§2.3 / §4.1 git-safety framing.** The write allowlist (`path.rs:47-59`) governs `.beads/`.
   `agents.rs` writes `AGENTS.md`, `CLAUDE.md`, and `*.md.bak` at arbitrary ancestor paths with
   no `validate_no_git_path`, no allowlist, no temp+rename, no fsync. The invariant "obr only
   writes inside `.beads/`" is narrower than the dossier states — exactly parallel to the
   §6 correction it *did* make about "never executes git commands".
3. **R14's git-safety test failures.** `FileTreeSnapshot` (§4.13) asserts on the complement of an
   allowlist. Nobody checked whether any test exercises `obr agents` under that snapshot; if not,
   the strongest assurance mechanism in the repo has never been pointed at the one command that
   writes outside the sandbox.
4. **§6 doc-drift catalogue.** The catalogue is thorough about `br`→`obr` in docs, but
   `AGENT_BLURB` is the text obr *injects into every user's agent instructions*. If it says `br`,
   obr is actively propagating the wrong binary name into other people's repos — a drift vector
   the dossier's model of "stale docs" does not cover, because this doc is generated at runtime.

---

## Gap 3 — `org2jsonl`, the parser for the durable artifact, is an unread, unvendored, unpinned local crate

`Cargo.toml:64`: `org2jsonl = { path = "../org2jsonl" }`. `flake.nix:21-22`:
`url = "git+file:///Users/johnw/src/org2jsonl"`.

The dossier mentions this dependency exactly twice, both times as the *build* problem —
"CI cannot build the tree at all" (R14). Nobody asked the prior question: **what is it, and is
the Org story sound?**

First-hand: the crate exists at `/Users/johnw/src/org2jsonl`, is ~4,987 lines of Rust,
`version = "0.1.0"`, ships three binaries (`org2jsonl`, `jsonl2org`, `roundtrip`), and its most
recent commits are "Add tests" / "Add end-to-end integration tests for multi-file location
metadata". obr calls into it at `src/sync/org_bridge.rs:12, 336, 585, 838` and
`src/sync/mod.rs:1178, 1188` — `org_to_entries_with_keywords` is the entry point for parsing the
git-tracked source of truth.

Consequences the dossier does not draw:

- **A `path` dependency carries no version requirement.** Cargo ignores `version` for path deps;
  `flake.nix` pins a `file://` URL on one machine. So the parser for the durable artifact is
  unversioned, unpublished, unvendored, and unbuildable by anyone but the author — not merely
  "CI is red", but *nobody else can build obr at all*, and there is no lockstep between an
  `issues.org` written by one build and parsed by another.
- **R23 (Org round-trip is lossy) is attributed entirely to obr's `extract_body_text`
  (`org_bridge.rs:540-554`).** That may be only half the loss. Nobody read the parser to
  establish what `Element` variants it produces, what it silently drops, whether it round-trips
  (it ships a `roundtrip` binary — a built-in oracle nobody used), or whether obr's
  `extract_body_text` is discarding structure the parser correctly preserved.
- **Panic-safety of the import path.** `fuzz/fuzz_targets/fuzz_org_parse.rs` exists and the
  assurance agent flagged it as having "no oracle". But the fuzzer's real target is *this crate*,
  which is where a panic on hostile input would originate — and a panic in the parser, under
  `panic = "abort"` (`Cargo.toml:94`, the R18 mechanism), is a SIGABRT on a plain `obr list`
  after a bad merge.
- **The one merge property that matters.** §3.2 argues obr's file "is not append-only, is not
  field-decomposed, and is not canonically ordered per record". Whether Org *emission* is
  byte-deterministic for a fixed DB state — the prerequisite for the file not churning under
  git, and for Q1's "hybrid" answer to be viable — is a property of this crate plus
  `issues_to_org_text`, and is never tested or stated.

---

## Gap 4 — the recovery mechanism that already exists was never evaluated, so §7 Q9 answers the wrong question

**Q9 ("What is the recovery story?")** concludes that several confirmed defects "leave a
workspace in a state a user cannot diagnose or exit", and proposes `doctor --repair` as
"probably the single highest-value *new* feature in this dossier".

But obr already ships a snapshot-and-restore mechanism, and no agent evaluated it as a recovery
path. `grep -c "br_history"` across all 18 notes: 3 in `map-sync.md`, 2 in `analysis-performance.md`,
1 each in `map-cli.md` / `map-docs.md` / `analysis-assurance.md`, 2 in `DOSSIER.md`.
`grep -c "history restore\|--restore\|execute_restore"`: **zero in every file, including the
dossier.** `map-sync.md:521-536` describes the backup writer structurally (rotation, dedup,
filename grammar) and stops; the performance notes count it as a cost (§R16: "one full file
copy"). Nobody opened `src/cli/commands/history.rs`, where `HistoryCommands::Restore { file,
force }` → `restore_backup` (`:25-26, :232-290`) copies a backup back over
`DEFAULT_JSONL_FILENAME` and prints the correct follow-up
(`"Next: br sync --import-only --force"`, `:278` — note `br`).

This changes conclusions in both directions and the dossier states neither:

- **Downward on severity.** R5 ("the workspace is unusable until a human hand-edits the file"),
  R1 and R6 all corrupt or diverge the *file*, and `backup_before_export`
  (`src/sync/history.rs:43-97`) copies the file **before** each overwrite. If the pre-corruption
  snapshot is sitting in `.beads/.br_history/`, then R5's "brick" is a two-command recovery, not
  a hand-edit — provided the user knows the command exists, which is the actual defect.
- **Upward, via three unexamined limits, all of which R16 makes worse:**
  1. **One-second filename granularity.** `backup_name = format!("{file_stem}.{timestamp}.{ext}")`
     with `%Y%m%d_%H%M%S` (`history.rs:71-76`), written by `fs::copy` (`:93`), which
     *overwrites*. Two flushes in the same second collide, and R16 establishes that obr flushes
     after **every** mutation — so an agent doing rapid mutations, or the R5 concurrency
     scenario itself, silently destroys its own backups.
  2. **Rotation window.** `HistoryConfig::default()` (`:22-28`) is `max_count = 100` /
     `max_age_days = 30`, pruned per stem by `rotate_history` (`:104-133`) after every backup.
     Under flush-per-mutation, 100 backups is ~100 mutations — possibly one working session.
     Nobody measured how far back the window actually reaches in wall-clock time.
  3. **Dedup-against-latest.** `:80-91` skips the backup entirely when the file is byte-identical
     to the newest existing backup. Interaction with R1 (a losing rename writes an *older*
     snapshot) and with the corruption cases is untraced.
- **`--force` semantics and path safety of restore.** `restore_backup` takes a user-supplied
  `file` argument. Whether it is validated against `validate_sync_path` / `validate_no_git_path`
  before being copied over the working file — i.e. whether `obr history restore ../../evil` is
  possible — was not checked by anyone, including me.

Q9's proposed `doctor --repair` (diff hashes, run `detect_all_cycles`, check orphans, exit
nonzero) is still right about *detection*. What it is missing is that the *remediation* half may
already exist and only need to be surfaced, hardened (unique backup names), and pointed at by the
error messages — which is a far cheaper answer than the one the dossier gives, and the reader
cannot choose between them from the dossier as written.

---

## Lower-priority observations (not returned as gaps)

- `src/cli/commands/epic.rs` (550), the `.N.M` child-id grammar, `next_child_number`'s `LIKE`
  scan (`sqlite.rs:2504-2534`), the dead `child_counters` table, and parent-child blocking
  propagation to `MAX_DEPTH = 50` are each mentioned in isolation, but the *epic/hierarchy
  feature* is never traced end-to-end (missing parent on import, `--rename-prefix` over
  hierarchical ids, cascade close, depth-cap behaviour on a 51-deep tree).
- `src/util/markdown_import.rs` (527) and `obr q` / quick-capture (`tests/e2e_quick_capture.rs`,
  642 lines) are bulk-ingestion paths for the agent audience with one citation between them.
- `src/cli/commands/graph.rs` (1136) is cited only for the R13 fixture trap.
- `tests/e2e_workspace_commands.rs` (612) and `obr where` / `obr info` — the "which workspace am
  I in" surface — are unexamined; this is Gap 1's user-facing half.
