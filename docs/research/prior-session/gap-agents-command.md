# GAP-FILL: `obr agents` — the command that writes outside `.beads/`

**Subject:** `/Users/johnw/src/obr/src/cli/commands/agents.rs` (1,054 lines)
**Binary tested:** `/etc/profiles/per-user/johnw/bin/obr`, `obr 0.1.14`
**Method:** full source read + clap wiring + test-corpus grep + 14 live experiments against the
installed binary in throwaway directories under
`/private/tmp/claude-501/.../scratchpad/lab/`.
**Repo treated as read-only.** (See §9 — one accidental write occurred and was fully reverted;
the incident is itself evidence and is disclosed in full.)

---

## 0. Executive answer

`obr agents` is the **only** obr subcommand that reads, creates, overwrites, or truncates files
outside `.beads/`. It:

- resolves its target by walking **up to 3 parent directories** from `$PWD`
  (`agents.rs:366`, `agents.rs:225-244`), so it routinely leaves the current git repository;
- applies **zero** of the `src/sync/path.rs` safety layer — no allowlist, no `.git` rejection,
  no traversal check, no atomic temp+rename (verified: `agents.rs:1-12` imports only
  `crate::error`, `crate::output`, `regex`, `rich_rust`, `std::fs`, `std::io`, `std::path`);
- writes with a plain `fs::write` whole-file replacement (`agents.rs:549`, `:633`, `:718`);
- protects that write with a single fixed-name `*.md.bak` copy that is **clobbered on the second
  invocation** and whose failure is downgraded to an `eprintln!` warning before writing anyway
  (`agents.rs:535-547`, `:621-631`, `:706-716`);
- **prompts on stdin** in three places (`:525`, `:611`, `:696`), which for a non-interactive
  agent is either a silent no-op (stdin at EOF) or an **indefinite hang** (stdin an open pipe);
- silently ignores `--add/--remove/--update` entirely under `--json` (`:368-370`, `:397`);
- injects a 2,076-byte blurb whose **19 command examples all invoke a binary named `br` that does
  not exist**, that describes a **JSONL** artifact this fork no longer produces, and that tells
  agents to run `git add` / `git commit` / `git push` — the exact automation this project's
  headline safety promise disclaims;
- has **no integration test of any kind**. The `FileTreeSnapshot` allowlist assertion — the
  repo's strongest safety mechanism — has never been pointed at it.

Two of the six behaviours I reproduced are **unrecoverable data loss with exit code 0**.

---

## 1. Wiring and entry points

| Concern | Location |
|---|---|
| clap subcommand | `src/cli/mod.rs:889-890` — `/// Manage AGENTS.md workflow instructions` / `Agents(AgentsArgs)` |
| clap args struct | `src/cli/mod.rs:2365-2391` |
| dispatch | `src/main.rs:114-124` |
| auto-import predicate | `src/main.rs:205` — `\| Commands::Agents(_) => false` (grouped with `Doctor`, `Info`, `Where`, `Version` as non-DB) |
| error handler | `src/main.rs:304-324` (`handle_error`; emits JSON to **stderr** when stdout is not a TTY) |

`AgentsArgs` (`src/cli/mod.rs:2367-2391`) — six bools, **none with `conflicts_with`**:

```rust
pub struct AgentsArgs {
    #[arg(long)] pub add: bool,        // 2368-2370
    #[arg(long)] pub remove: bool,     // 2372-2374
    #[arg(long)] pub update: bool,     // 2376-2378
    #[arg(long)] pub check: bool,      // 2380-2382
    #[arg(long)] pub dry_run: bool,    // 2384-2386
    #[arg(long, short = 'f')] pub force: bool,  // 2388-2390
}
```

Notably `agents` does **not** accept `--robot`, unlike its sibling commands:

```
$ obr agents --remove --robot --force
error: unexpected argument '--robot' found
Usage: obr agents --remove
```

`execute()` (`agents.rs:364-392`) never touches config, never calls
`config::discover_beads_dir`, never opens the database. Its only input is
`std::env::current_dir()` (`:365`). **The global `--db` flag (`src/cli/mod.rs:658-660`) is
ignored** — there is no way to redirect `obr agents` at a directory other than `$PWD`.

### 1.1 Flag precedence is silently lossy

`agents.rs:373-389`:

```rust
let is_check = !args.add && !args.remove && !args.update;   // 373
if is_check || args.check { return execute_check(...); }    // 375-377
if args.add    { return execute_add(...); }                 // 379-381
if args.remove { return execute_remove(...); }              // 383-385
if args.update { return execute_update(...); }              // 387-389
```

Verified live:

```
=== --add --check  (does add happen?) ===
Found: AGENTS.md at .../flags/AGENTS.md
Status: No beads workflow instructions found
  bytes=7        <-- unchanged; --add was silently swallowed by --check
=== --add --remove (which wins?) ===
Added beads workflow instructions to: .../flags/AGENTS.md
  bytes=2082     <-- --add won; --remove silently ignored, exit 0
```

`obr agents --add --check` reports "To add: br agents --add" and exits 0 having done nothing.
`obr agents --add --remove` performs the add and never mentions that `--remove` was dropped.

### 1.2 `--json` silently swallows every mutating flag

`agents.rs:368-370` short-circuits before the action dispatch, and `execute_json` takes
`_args: &AgentsArgs` — underscore-prefixed, **never read** (`agents.rs:395-413`).

```
=== --json --remove --force on a blurbed file (does it act?) ===
  NO-OP: --json silently swallows --remove         [sha before == sha after]
```

Consequence for the JSON contract: `obr --json agents --add` emits
`"needs_blurb": true` and exit 0 while doing nothing. An agent that drives obr in `--json`
mode — the documented agent-first mode — can never mutate an agent file and can never learn
that it failed to. Conversely the non-JSON error path *does* signal failure:

```
=== truly-no-agent-file (isolated dir, no ancestor has AGENTS.md) ===
$ obr agents --remove --force            -> VALIDATION_FAILED, EXIT=4
$ obr --json agents --remove --force     -> {"found": false, ...}, EXIT=0
```

---

## 2. Q1 — The full write surface

### 2.1 Target resolution

```
detect_agent_file_in_parents(work_dir, 3)        agents.rs:366
  └─ for _ in 0..=max_levels                     agents.rs:228   <-- INCLUSIVE range
       └─ detect_agent_file(current_dir)         agents.rs:169-193
            ├─ AGENTS.md, CLAUDE.md   (uppercase pass, :171-179)
            └─ agents.md, claude.md   (lowercase pass, :182-190)
                 └─ check_agent_file  agents.rs:196-221
       └─ current_dir = current_dir.parent()     agents.rs:235-240
```

`SUPPORTED_AGENT_FILES` = `["AGENTS.md", "CLAUDE.md", "agents.md", "claude.md"]`
(`agents.rs:25`). `get_preferred_agent_file_path` (`agents.rs:337-339`) is
`work_dir.join("AGENTS.md")` — the creation target when nothing is found.

**`max_levels = 3` with an inclusive `0..=3` means 4 directories are searched: `$PWD` plus 3
ancestors.** Verified exactly:

```
  marker 0 levels up -> "found": true
  marker 1 levels up -> "found": true
  marker 2 levels up -> "found": true
  marker 3 levels up -> "found": true
  marker 4 levels up -> "found": false
  marker 5 levels up -> "found": false
```

### 2.2 The target can be outside the git repository — CONFIRMED

Layout: `work/AGENTS.md` (belongs to a different project), `work/myrepo/` is the git repo,
`$PWD = work/myrepo/src/deep` (exactly 3 levels below `work/`).

```
sha before: e2b84fa38d9236c833528cc4a9b7056a036d4dee  .../esc2/work/AGENTS.md
$ obr agents --add --force < /dev/null
Backup created: .../esc2/work/AGENTS.md.bak
Added beads workflow instructions to: .../esc2/work/AGENTS.md
EXIT=0
sha after:  b8bc66de4653558eea46f1092ccef5d71762d2b7   (57 -> 2132 bytes)
$ git -C .../esc2/work/myrepo rev-parse --show-toplevel
.../esc2/work/myrepo                                   <-- the repo does NOT contain work/
```

A single `obr agents --add` from inside a project modified a file in a **sibling project's
parent directory** and dropped a stray `AGENTS.md.bak` there. If your repo lives at
`~/src/proj` and you run from `~/src/proj/sub`, the walk reaches `~/src`; from `~/src/proj` it
reaches `~`. **`~/AGENTS.md` and `~/CLAUDE.md` are in range from any repo ≤3 levels below
`$HOME`** — which is the normal layout for `~/src/foo`.

The same escape applies to `--remove`. From an *empty* subdirectory:

```
=== --remove where NO agent file exists in cwd ===
Backup created: .../flags/AGENTS.md.bak
Removed beads workflow instructions from: .../flags/AGENTS.md   <-- the PARENT's file
  EXIT=0
```

### 2.3 The target can be inside `.git` — CONFIRMED

```
=== A: run from inside .git/hooks ===
$ cd repo/.git/hooks && obr agents --add --force
Added beads workflow instructions to: .../repo/.git/hooks/AGENTS.md
EXIT=0
$ find repo/.git -name "AGENTS.md*"
  /gitdir/repo/.git/hooks/AGENTS.md

=== B: pre-existing AGENTS.md inside .git ===
$ cd repo/.git && obr agents --add --force
Backup created: .../repo/.git/AGENTS.md.bak
Added beads workflow instructions to: .../repo/.git/AGENTS.md
EXIT=0
-rw-r--r--  AGENTS.md      2088
-rw-r--r--  AGENTS.md.bak    13
```

Contrast with `src/sync/path.rs:140-175` `validate_no_git_path`, which rejects any path with a
`.git` component *and* re-checks the canonicalised path to catch symlinks — documented at
`src/sync/path.rs:130-133` as safety invariant **"NGI-3: br sync NEVER modifies .git/
directory."** That invariant is enforced for `sync` and consulted by `doctor`
(`src/cli/commands/doctor.rs:508-509`). It is not enforced here. `obr sync --help` still prints:

```
SAFETY GUARANTEES:
  • br sync NEVER executes git commands or auto-commits
  • br sync NEVER modifies files outside .beads/ (unless --allow-external-jsonl)
  • All writes use atomic temp-file-then-rename pattern
```

All three sentences are scoped to `sync`; none of the three properties holds for `agents`.

### 2.4 Symlinks are followed to arbitrary destinations — CONFIRMED

`proj/AGENTS.md` → `secret/other.md` (a different directory entirely):

```
$ ls -la proj
lrwxr-xr-x AGENTS.md -> .../sym/secret/other.md
$ obr agents --add --force
Backup created: .../sym/proj/AGENTS.md.bak
Added beads workflow instructions to: .../sym/proj/AGENTS.md
EXIT=0
$ head -3 .../sym/secret/other.md
# Victim
important

<!-- br-agent-instructions-v1 -->     <-- TARGET was rewritten, 27 -> 2094 bytes
$ ls .../sym/proj
AGENTS.md -> ...   AGENTS.md.bak (19 bytes)
```

Two facts compound here: (a) `fs::write` follows the symlink, so the write lands on the
destination; (b) the `.bak` is created next to the **link**, not next to the file that was
actually modified — so recovery requires the operator to notice the indirection. Combined with
the 3-level parent walk, a symlink named `AGENTS.md` anywhere in the ancestor chain redirects
the write to any writable path on the filesystem, and nothing in obr checks.

There is no `dunce::canonicalize` call anywhere in `agents.rs` (contrast `sync/path.rs:167`).

### 2.5 Path-safety application: **NONE**

Grep for every consumer of the safety layer:

```
src/cli/commands/doctor.rs:9      use ... validate_no_git_path, validate_sync_path
src/cli/commands/doctor.rs:508-509  validate_no_git_path(jsonl_path)
src/sync/path.rs:*                (self-references)
```

`src/cli/commands/agents.rs` appears nowhere. Confirmed by its import block
(`agents.rs:6-12`) — it imports no module from `crate::sync`.

The allowlist that `agents` bypasses is `src/sync/path.rs:48-56`
(`db`, `db-wal`, `db-shm`, `jsonl`, `jsonl.tmp`, `org`, `org.tmp`) plus
`src/sync/path.rs:59` (`.manifest.json`, `metadata.json`). Note `md` and `md.bak` are not on it
— i.e. if the sync layer were ever pointed at these writes it would reject every one of them.

### 2.6 Complete enumeration of the write surface

For `$PWD = D`, ancestors `D₁ D₂ D₃`:

| Path | Op | Trigger |
|---|---|---|
| `Dₙ/{AGENTS,CLAUDE,agents,claude}.md` for n∈{0,1,2,3}, first match wins | whole-file `fs::write` | `--add` (:549), `--remove` (:633), `--update` (:718) |
| same path with `.md` → `.md.bak` | `fs::copy` overwrite | every non-dry-run mutation with `detection.found()` |
| `D/AGENTS.md` (created) | `fs::write` | `--add` when nothing found in the 4-dir window |
| **any path a matched name symlinks to** | `fs::write` follows link | as above |
| **paths inside `.git/`** | `fs::write` | when `$PWD` or an ancestor is inside `.git` |
| **paths outside the git repo** | `fs::write` | whenever the match is above the repo root |

Non-targets: `check_agent_file:197` skips directories; `check_agent_file:201-208` returns a
detection with `content: None` for unreadable/non-UTF-8 files — see §3.4, this is the worst bug.

`obr agents` needs **no beads project at all**. Every experiment above ran in bare directories
with no `.beads/`, and all succeeded.

---

## 3. Q2 — Backup and data-loss behaviour

### 3.1 The backup mechanism

Identical code appears three times (`:535-547` add, `:621-631` remove, `:706-716` update):

```rust
let backup_path = file_path.with_extension("md.bak");     // 537 / 622 / 707
if let Err(e) = fs::copy(&file_path, &backup_path) {      // 538 / 623 / 708
    eprintln!("Warning: Could not create backup at {}: {}", ...);   // 539-543
} else if !matches!(ctx.mode(), OutputMode::Rich) {
    println!("Backup created: {}", backup_path.display());
}
fs::write(&file_path, &new_content)?;                      // 549 / 633 / 718
```

Three defects, all confirmed live:

1. **Fixed name, one generation.** `AGENTS.md` → `AGENTS.md.bak` always. The second mutation
   overwrites the only copy of the pristine original.
2. **Failure is non-fatal.** A `fs::copy` failure prints a warning and the destructive write
   proceeds, exit 0.
3. **Not gitignored.** `obr init` writes `.beads/.gitignore` covering only `*.db`, `*.db-shm`,
   `*.db-wal`, `*.lock`, `last-touched`, `*.tmp`. `*.md.bak` is ignored nowhere:

```
$ git status --porcelain
?? .beads/
?? AGENTS.md
?? AGENTS.md.bak        <-- will be swept up by `git add -A`
```

Demonstration of (2) — backup target made un-writable by placing a directory at `AGENTS.md.bak`:

```
BEFORE: 36 bytes  ("# IRREPLACEABLE CONTENT\nline2\nline3\n")
$ obr agents --add --force
Warning: Could not create backup at .../AGENTS.md.bak: Is a directory (os error 21)
Added beads workflow instructions to: .../AGENTS.md
  EXIT=0
AFTER:  2111 bytes            <-- write proceeded with no backup, success exit
```

The same run against an adversarial file (§3.3) destroys content with no recovery path at all:

```
BEFORE: 140
Warning: Could not create backup at .../AGENTS.md.bak: Is a directory (os error 21)
Removed beads workflow instructions from: .../AGENTS.md
  EXIT=0
AFTER:  149  -- no recoverable backup exists
```

### 3.2 `remove_blurb` is string surgery with an independent end-marker search

`agents.rs:261-289`:

```rust
let start_marker = "<!-- br-agent-instructions-v";
let Some(start_idx) = content.find(start_marker) else { return content.to_string(); };  // 263
let Some(end_pos)   = content.find(BLURB_END_MARKER) else { return content.to_string(); };  // 267
let end_idx = end_pos + BLURB_END_MARKER.len();                                        // 270
...
while end < content.len() && content[end..].starts_with('\n') { end += 1; }             // 277-279
let mut removed_leading = 0;
while start > 0 && content[..start].ends_with('\n') && removed_leading < 2 {            // 283-286
    start -= 1; removed_leading += 1;
}
format!("{}{}", &content[..start], &content[end..])                                     // 288
```

The end marker is located with `content.find(...)` over the **whole string**, not from
`start_idx`. There is no ordering check, no `start < end` assertion, no scoping to a single
blurb, and no bound on how much of the user's file falls between the two indices.
`remove_legacy_blurb` (`:293-325`) is the same algorithm with `bv` markers.
`update_blurb` (`:329-333`) = `remove_legacy_blurb` → `remove_blurb` → `append_blurb`, so every
defect below is also reachable via `--update` and via `--add` on a legacy/outdated file
(`agents.rs:499-503` routes `--add` into `execute_update`).

Detection is equally loose: `contains_blurb` (`:137-139`) matches the **prefix**
`"<!-- br-agent-instructions-v"` with no closing `-->`, so any prose containing that substring
marks the file as blurbed.

### 3.3 Five adversarial `AGENTS.md` files, byte-level before/after

All runs used `obr agents --remove --force < /dev/null`. All exited 0. All printed
"Removed beads workflow instructions from: …".

---

**R1 — end marker mentioned in prose above the real blurb → content DUPLICATION, unbounded**

Input (308 bytes):
```
# Team Agent Rules

## SECRET DEPLOY KEYS - DO NOT DELETE
prod-token: hunter2
staging-token: swordfish

The end marker looks like this: <!-- end-br-agent-instructions -->

## More critical rules
Never force-push to main.

<!-- br-agent-instructions-v1 -->
beads blurb body
<!-- end-br-agent-instructions -->
```

After one run (**356 bytes — the file GREW**):
```
...
The end marker looks like this: <!-- end-br-agent-instructions -->

## More critical rules
Never force-push to main.## More critical rules
Never force-push to main.

<!-- br-agent-instructions-v1 -->
beads blurb body
<!-- end-br-agent-instructions -->
```

The blurb was **not removed**, a chunk of the user's file was **duplicated**, and the duplicate
was glued on with no separator (`main.## More critical rules`). Mechanism: `end_idx` lands in
the prose line at ~155, `start_idx` at ~250; `&content[..start] + &content[end..]` re-emits the
region `[end, start)` twice.

Because the blurb survives, the command is non-idempotent and the duplicated region **doubles
every run**:

```
run 1 -> 356 b   (+48)
run 2 -> 452 b   (+96)
run 3 -> 644 b   (+192)
run 4 -> 1028 b  (+384)
```

After 4 runs the file contained 16 copies of `## More critical rules\nNever force-push to main.`
concatenated without newlines. This is exponential — a retry loop or a `for` loop over repos
fills the disk. And the backup follows the corruption:

```
after run 1: AGENTS.md.bak sha = a4ef7c56... (308 b)  == pristine original, recoverable
after run 2: AGENTS.md.bak      = 356 b               == the CORRUPTED run-1 output
                                                          original now unrecoverable
```

---

**R2 — user prose mentions the start marker → SILENT MASS DELETION (292 → 64 bytes)**

Input (292 bytes):
```
# Docs for our tooling

To document beads we mention the marker <!-- br-agent-instructions-v1 --> inline.

## PAYROLL RUNBOOK
Step 1: do the thing
Step 2: do the other thing
## ONCALL ROTATION
alice, bob, carol

<!-- br-agent-instructions-v1 -->
real blurb
<!-- end-br-agent-instructions -->
```

Output (64 bytes, no trailing newline):
```
# Docs for our tooling

To document beads we mention the marker
```

**The PAYROLL RUNBOOK and ONCALL ROTATION sections were destroyed**, the sentence was truncated
mid-clause, and obr reported success. `start_idx` matched the *documentation* mention, `end_pos`
matched the real blurb's end marker, and everything between — 228 bytes of the user's content —
was deleted. Any repo whose AGENTS.md documents the beads integration for humans is vulnerable.

---

**R3 — two blurbs (e.g. after a merge) → first removed, second left, adjacent line joined**

Input (233 b) → output (151 b):
```
# HeaderMIDDLE USER CONTENT - KEEP ME
<!-- br-agent-instructions-v1 -->
SECOND blurb (from a merge)
<!-- end-br-agent-instructions -->
Trailer content
```

`# Header` and `MIDDLE USER CONTENT` were **welded into one line** (heading destroyed), and the
file still contains a blurb despite the "Removed …" success message. Nested markers behave the
same way — only the outermost start and the innermost-first end are used.

---

**R4 — start marker present, end marker hand-deleted → FALSE success, useless backup**

Input 101 bytes → output 101 bytes, byte-identical. `remove_blurb:267-269` returns early. But
execute_remove had already written a `.bak` and then printed:

```
Backup created: .../AGENTS.md.bak
Removed beads workflow instructions from: .../AGENTS.md
  EXIT=0
```

Nothing was removed. A caller that trusts the exit code and the message is wrong, and a
subsequent `--check` still reports the blurb present.

---

**R5 — legacy `bv` blurb + current `br` blurb both present → only legacy removed**

`execute_remove:587-591` branches on `has_legacy_blurb` first, so `remove_legacy_blurb` runs and
`remove_blurb` never does. 162 → 85 bytes:

```
# Header<!-- br-agent-instructions-v1 -->
current
<!-- end-br-agent-instructions -->
```

Again `# Header` was welded to the following line, and the current blurb survives a command
whose entire purpose is to remove it.

---

### 3.4 Non-UTF-8 AGENTS.md → **TOTAL SILENT CONTENT LOSS**

This is the most severe finding and does not require any adversarial marker.

`check_agent_file:201-208`:
```rust
let Ok(content) = fs::read_to_string(file_path) else {
    // File exists but not readable
    return Some(AgentFileDetection {
        file_path: Some(file_path.to_path_buf()),
        file_type: Some(file_type.to_string()),
        ..Default::default()          // content: None, has_blurb: false
    });
};
```

`execute_add:489`:
```rust
let content = detection.content.clone().unwrap_or_default();   // "" when read failed
```

So a file obr cannot decode is reported as *found but empty*, and `append_blurb("")` +
`fs::write` replaces the whole file with just the blurb. `fs::read_to_string` fails on **any**
non-UTF-8 byte — a Latin-1 accented character, a stray CP-1252 smart quote, a pasted binary
fragment.

Live transcript. Input: 114 bytes, one `\xe9` byte in an author's name:

```
BEFORE: 114 bytes
0000000   #   C R I T I C A L   T E A M   R U L E S \n \n A u t h o r :   J
0000040   o s   ?     G a r c i a \n \n # #   D e p l o y   c h e c k l i s t
          1. run migrations / 2. notify oncall / 3. rotate keys

--- obr sees it as: ---
{ "found": true, "has_blurb": false, "blurb_version": 0, "needs_blurb": true }

$ obr agents --add --force < /dev/null
Backup created: .../utf/AGENTS.md.bak
Added beads workflow instructions to: .../utf/AGENTS.md
EXIT=0

AFTER: 2076 bytes
======== FILE CONTENT NOW ========


<!-- br-agent-instructions-v1 -->
---
## Beads Workflow Integration
======== END ========
```

**Every byte of the user's file is gone**, replaced by the blurb. No warning about the read
failure. Exit 0. The `.bak` holds the original — for exactly one more command:

```
=== an agent then runs --remove --force ===
bak BEFORE remove: 114 bytes   (the original)
bak AFTER remove:  2076 bytes  (the blurb-only file)
AGENTS.md:         0 bytes
--- is the original 114-byte content anywhere? ---
  GONE - unrecoverable
```

Two commands, both exit 0, both "successful": a 114-byte file becomes a 0-byte file and the
backup holds nothing but obr's own boilerplate.

### 3.5 Even the happy path is lossy

Clean file, no adversarial content, `--add` then `--remove`:

```
ORIGINAL: 28 b   "# My Project\n\nSome content.\n"
AFTER ADD: 2103 b
AFTER REMOVE: 27 b
cmp: EOF on AGENTS.md after byte 27 -> DIFFERS
od: ... S o m e   c o n t e n t .        <-- trailing \n eaten
```

`remove_blurb:277-279` consumes *all* newlines after the end marker while `:283-286` restores at
most 2 before it, so a round trip strips the POSIX trailing newline. It converges (27→27 over 3
cycles), so it is a one-time single-byte loss — but it dirties every git diff and trips
end-of-file-newline linters on files obr merely visited.

### 3.6 Error paths that behave correctly

For completeness — these two do fail safe:

```
=== AGENTS.md is a DIRECTORY ===
{"code":"IO_ERROR","message":"I/O error: Is a directory (os error 21)"}   EXIT=8
=== AGENTS.md mode 000 ===
Warning: Could not create backup at .../AGENTS.md.bak: Permission denied (os error 13)
{"code":"IO_ERROR","message":"I/O error: Permission denied (os error 13)"} EXIT=8
  (file preserved: "secret rules")
```

The unreadable-by-permissions case survives only because the *write* also fails. The
non-UTF-8 case (§3.4) is the one where the read fails and the write succeeds.

---

## 4. Q3 — The interactive prompt

### 4.1 The three prompt sites

| Line | Command | Gate |
|---|---|---|
| `agents.rs:521-533` | `--add` | `if !force && !detection.found()` — only when *creating* a new file |
| `agents.rs:605-619` | `--remove` | `if !force` — **always**, including on existing files |
| `agents.rs:690-704` | `--update` | `if !force` — **always** |

All three are the same four lines:
```rust
print!("Continue? [y/N] ");   // 525 / 611 / 696  -- STDOUT
io::stdout().flush()?;        // 526 / 612 / 697
let mut input = String::new();
io::stdin().read_line(&mut input)?;   // 528 / 614 / 699
if !input.trim().eq_ignore_ascii_case("y") { println!("Aborted."); return Ok(()); }
```

`--dry-run` returns before the prompt in all three (`:507-519`, `:593-603`, `:678-688`), so
dry-run never blocks — confirmed (`EXIT=0`, no file created).

### 4.2 stdin closed (`< /dev/null`) — silent no-op with exit 0

```
$ obr agents --add < /dev/null
This will create a new AGENTS.md with beads workflow instructions.
File: .../proj/AGENTS.md
Continue? [y/N] Aborted.
EXIT=0
--- files ---
total 0                      <-- nothing created
```

`read_line` returns `Ok(0)`, `input` is empty, `"".trim() != "y"` → "Aborted.", `Ok(())`.
**An agent gets exit code 0 and no file.** There is no distinguishable signal — the same exit
code as success. Same for `--remove`:

```
$ obr agents --remove < /dev/null
This will remove beads workflow instructions from: .../flags/AGENTS.md
Continue? [y/N] Aborted.
  EXIT=0  bytes=2082        <-- unchanged
```

### 4.3 stdin an open pipe that never writes — INDEFINITE HANG

The realistic agent case: the harness gives the child an open pipe and never writes to it.

```
$ mkfifo fifo; sleep 30 > fifo &          # holder keeps the write end open
$ timeout 5 obr agents --add < fifo
EXIT=124 (124 == timed out == HUNG)
--- stdout ---
This will create a new AGENTS.md with beads workflow instructions.
File: .../proj/AGENTS.md
Continue? [y/N]
--- stderr ---
(empty)
```

Confirmed for `--remove` as well: `--remove exit=124 (124=HUNG)`.

There is no timeout, no `is_terminal()` check, and no `--yes`/`--assume-no` fallback.
Compare `src/main.rs:309`, where the error handler *does* check
`!io::stdout().is_terminal()` to decide output format — the codebase knows how to detect
non-interactivity; `agents.rs` simply never asks.

### 4.4 The prompt lands on stdout and corrupts machine-readable output

`print!` and `println!` — stdout, not stderr (`:523-525`, `:607-611`, `:692-696`). The
"Continue? [y/N] " string is not newline-terminated and is explicitly flushed, so it
interleaves into whatever the caller is parsing. Under `--json` the prompt is unreachable only
because `--json` disables the actions entirely (§1.2) — so the two failure modes are
complementary, not mutually mitigating: **either you get a hang, or you get a silent no-op.**
There is no configuration of flags under which an agent gets a machine-readable, non-blocking,
mutating `obr agents`. The closest is `--force`, which removes the prompt but also removes every
confirmation of the destructive behaviours in §3.

### 4.5 Does "destructive commands never prompt" hold elsewhere? — NO, one more site

```
$ grep -rn '\[y/N\]\|\[Y/n\]\|Continue?' --include="*.rs" src/
src/cli/commands/agents.rs:525:        print!("Continue? [y/N] ");
src/cli/commands/agents.rs:611:        print!("Continue? [y/N] ");
src/cli/commands/agents.rs:696:        print!("Continue? [y/N] ");
src/cli/commands/orphans.rs:203:            print!("Close {} ({})? [y/N] ", orphan.issue_id, orphan.title);
```

`obr orphans --fix` (`src/cli/commands/orphans.rs:199-226`) loops over every orphan issue and
prompts for each. It calls `close::execute_with_args` on "y" (`:211-221`) — a **database
mutation** driven by an stdin prompt. And critically, `OrphansArgs` (`src/cli/mod.rs:2267-2279`)
has **no `force` and no `yes` flag at all** — only `details`, `fix`, `robot`. The prompt is
unbypassable; `obr orphans --fix` cannot be automated.

Other stdin readers are legitimate input channels, not prompts, and are opt-in:
- `src/cli/commands/audit.rs:266` — `read_to_string` gated on `args.stdin`
- `src/cli/commands/comments.rs:304` — gated on `--file -`
- `src/sync/mod.rs:1154`, `:2558` — `BufRead::read_line` over files, unrelated

**Verdict on §4.11 of the dossier:** the claim "Destructive commands never prompt … Exactly
right for agents" is false. Four prompt sites exist across two modules; three of them guard
file mutation and one guards database mutation, and the database one cannot be forced past.

---

## 5. Q4 — Blurb content and doc drift

### 5.1 Constants

```rust
pub const BLURB_VERSION: u8 = 1;                                          // agents.rs:16
pub const BLURB_START_MARKER: &str = "<!-- br-agent-instructions-v1 -->";  // agents.rs:19
pub const BLURB_END_MARKER:   &str = "<!-- end-br-agent-instructions -->"; // agents.rs:22
pub const AGENT_BLURB: &str = ...                                         // agents.rs:28-93
// legacy markers, matched but never emitted:
"<!-- bv-agent-instructions-v"      // agents.rs:145, :298
"<!-- end-bv-agent-instructions -->" // agents.rs:299
```

There is no `LEGACY_BLURB` constant — the legacy format exists only as two marker strings used
for detection and stripping. `BLURB_VERSION` has never been incremented despite the JSONL→Org
migration (2026-02-18) and the `br`→`obr` rename (2026-02-19), so **no existing installation
will ever be told its blurb is stale**: `needs_upgrade()` (`agents.rs:127-132`) returns false
for any v1 blurb, and `execute_update` short-circuits with "already up to date (v1)". The drift
is therefore permanent and self-perpetuating until someone bumps the constant.

### 5.2 The exact 2,076 bytes obr injects into other repositories

Reproduced by running `obr agents --add --force` in an empty directory:

```markdown
<!-- br-agent-instructions-v1 -->

---

## Beads Workflow Integration

This project uses [beads_rust](https://github.com/Dicklesworthstone/beads_rust) (`br`/`bd`)
for issue tracking. Issues are stored in `.beads/` and tracked in git.

### Essential Commands

```bash
# View ready issues (unblocked, not deferred)
br ready              # or: bd ready

# List and search
br list --status=open # All open issues
br show <id>          # Full issue details with dependencies
br search "keyword"   # Full-text search

# Create and update
br create --title="..." --description="..." --type=task --priority=2
br update <id> --status=in_progress
br close <id> --reason="Completed"
br close <id1> <id2>  # Close multiple issues at once

# Sync with git
br sync --flush-only  # Export DB to JSONL
br sync --status      # Check sync status
```

### Workflow Pattern

1. **Start**: Run `br ready` to find actionable work
2. **Claim**: Use `br update <id> --status=in_progress`
3. **Work**: Implement the task
4. **Complete**: Use `br close <id>`
5. **Sync**: Always run `br sync --flush-only` at session end

### Key Concepts

- **Dependencies**: Issues can block other issues. `br ready` shows only unblocked work.
- **Priority**: P0=critical, P1=high, P2=medium, P3=low, P4=backlog (use numbers 0-4, not words)
- **Types**: task, bug, feature, epic, chore, docs, question
- **Blocking**: `br dep add <issue> <depends-on>` to add dependencies

### Session Protocol

**Before ending any session, run this checklist:**

```bash
git status              # Check what changed
git add <files>         # Stage code changes
br sync --flush-only    # Export beads changes to JSONL
git commit -m "..."     # Commit everything
git push                # Push to remote
```

### Best Practices

- Check `br ready` at session start to find available work
- Update status as you work (in_progress → closed)
- Create new issues with `br create` when you discover tasks
- Use descriptive titles and set appropriate priority/type
- Always sync before ending session

<!-- end-br-agent-instructions -->
```

### 5.3 Drift quantification

```
occurrences of 'br ' (wrong binary name): 19
occurrences of 'bd ' (Go binary):          1
occurrences of 'obr':                      0
occurrences of 'JSONL':                    2
occurrences of 'Org-mode' / 'issues.org':  0
git commands recommended:  git add  git commit  git push  git status
```

**Binary name — every example is broken.** Verified against the installed binary in a real
initialised project:

```
br ready                                   FAIL(127): command not found: br
obr ready                                  OK
obr list --status=open                     OK
obr search "keyword"                       OK
obr create --title="x" --description="y" --type=task --priority=2   OK
obr sync --flush-only                      OK
obr sync --status                          OK
```

So **19 of 19 `br` invocations fail with exit 127**, and the one `bd` invocation fails too
(`bd not found`). The flag *shapes* are all still correct (`--title`, `--reason`
(`obr close -r/--reason`), `obr dep add` exists), so the blurb is wrong in exactly one
mechanical way — but that one way makes every single copy-pasteable line fail. An agent that
obeys this file will emit `command not found` on its first action, on its last action, and on
the mandatory end-of-session checklist.

**Storage format — factually wrong.** The blurb says `br sync --flush-only  # Export DB to
JSONL` (`agents.rs:54`) and `br sync --flush-only    # Export beads changes to JSONL`
(`agents.rs:80`). Ground truth from a fresh project:

```
$ obr init && ls .beads/
.gitignore  beads.db  config.yaml  issues.org  metadata.json
$ obr sync --flush-only && ls .beads/
... issues.org (561 bytes)
$ find . -name "*.jsonl" | wc -l
0
$ head -2 .beads/issues.org
#+TITLE: Beads Issues
#+SEQ_TODO: TODO DOING WAIT DEFER NOTE | DONE CANCELED
```

The artifact is Org-mode `.beads/issues.org`. No JSONL file is produced at all. The blurb names
a file that does not exist and never mentions the one that does — so a human or agent told to
"commit the JSONL" will commit nothing and lose the durable artifact.

**Project identity — wrong upstream.** `agents.rs:34` points at
`github.com/Dicklesworthstone/beads_rust`, and calls the tool "beads_rust". This fork is `obr`.

**Workflow the fork disclaims.** `agents.rs:73-83` instructs every agent, before ending every
session, to run `git status`, `git add <files>`, `git commit -m "..."`, `git push`. The project's
headline safety property — restated in `obr sync --help` and `src/sync/path.rs:130-131`
("NGI-1: br sync NEVER executes git subprocess commands") — is that obr does not run git for
you. The blurb's response is to hand that job to the agent as a mandatory, unreviewed,
`git push`-terminated checklist, injected into repositories the obr maintainers do not own. The
non-invasiveness promise is preserved at the process level and abandoned at the instruction
level.

### 5.4 Blast radius

`obr agents --add` writes **2,076 bytes containing 20 non-functional command invocations, a
wrong project URL, a wrong storage format named twice, and a mandatory auto-`git push` ritual**
into a file whose entire purpose is to be read and obeyed by an AI agent. Every one of the four
supported filenames (`AGENTS.md`, `CLAUDE.md`, `agents.md`, `claude.md`) is a file that agent
harnesses load automatically as system-level instruction. This is the highest-leverage
documentation surface in the repository and it is 100% stale.

The staleness is not confined to the blurb. The command's own help strings still say `br`:

```
agents.rs:432  println!("  br agents --add");
agents.rs:443  println!("  br agents --update");   // legacy-blurb branch
agents.rs:449  println!("  br agents --update");   // outdated-version branch
agents.rs:452  println!("  br agents --update");
agents.rs:459  println!("  br agents --add");
agents.rs:757  content.append_styled("  br agents --update", ...)   // Rich mode
agents.rs:766  content.append_styled("  br agents --update", ...)
agents.rs:776  content.append_styled("  br agents --add", ...)
agents.rs:786  content.append_styled("  br agents --add", ...)
```

Nine user-facing suggestions, all naming a binary that does not exist. Verified live —
`obr agents` on an un-blurbed file prints `To add:\n  br agents --add`.

And the fork does not dogfood the command: `/Users/johnw/src/obr/AGENTS.md` contains **zero**
occurrences of `br-agent-instructions`. The command has never been run against this repository.

Adjacent known drift, confirmed: `/Users/johnw/src/obr/.beads/README.md` is unmodified upstream
Go `bd` boilerplate — `bd create`, `bd list`, `bd show`, `bd update`, `bd sync`, a link to
`github.com/steveyegge/beads`, and the claims *"Stored in `.beads/issues.jsonl`"* and
*"**Always in sync**: Auto-syncs with your commits"* (the latter directly contradicting NGI-1).
`/Users/johnw/src/obr/.beads/` still contains `issues.jsonl` and `interactions.jsonl` and **no**
`issues.org`, while a fresh `obr init` produces `issues.org` and no JSONL — so the repo's own
`.beads/` predates its own migration.

There is **no documentation of the `agents` command anywhere**:

```
$ grep -rn "agents --add\|agents --remove\|obr agents\|br agents" --include="*.md" .
(no matches outside target/)
```

Its only appearance in the whole doc corpus is one line of clap-generated help
(`tests/snapshots/.../help_output.snap:48` — `agents  Manage AGENTS.md workflow instructions`).

---

## 6. Q5 — Assurance coverage

### 6.1 Integration/e2e tests: ZERO

```
$ grep -rln "agents" tests/
tests/snapshots/snapshots/snapshots__snapshots__cli_output__help_output.snap
$ grep -rn "AGENTS.md\|CLAUDE.md\|AGENT_BLURB\|agent_file\|blurb" tests/
tests/conformance_schema.rs:695:  /// ...excluded from the br port per AGENTS.md.   (a comment)
tests/snapshots/.../help_output.snap:48:  agents  Manage AGENTS.md workflow instructions
```

The only test-tree evidence that this command exists is one line of a help snapshot. **No test
in `tests/` ever invokes `obr agents`.**

### 6.2 `FileTreeSnapshot` has never been aimed at it

`tests/e2e_sync_git_safety.rs:675-721` defines `FileTreeSnapshot`, which SHA-256-hashes every
file under a workspace root and diffs two snapshots; `check_allowed_changes()` asserts that only
allowlisted paths changed (allowlist at `tests/e2e_sync_git_safety.rs:~650-670`, extensions
`db`, `db-wal`, `db-shm`, `jsonl`, `jsonl.tmp`).

Every one of its five uses targets `sync`:

| Line | Command under test |
|---|---|
| `:1000` / `:1016` | `run_br(&workspace, ["sync", "--flush-only"], ...)` |
| `:1095` / `:1115` | `sync --import-only` |
| `:1186` / `:1207` | full export/import cycle |
| `:1412` / `:1427` | `run_br(&workspace, ["sync", "--flush-only", "--manifest"], ...)` |

**Stated plainly: the repository's strongest safety assertion — "only allowlisted paths changed"
— has never been pointed at the one command that writes outside the sandbox.** Pointing it at
`obr agents --add --force` would fail immediately, because `AGENTS.md` and `AGENTS.md.bak` are
not on the allowlist (`md` and `md.bak` are absent from `ALLOWED_EXTENSIONS`, both in the test
file and in `src/sync/path.rs:48-56`).

### 6.3 Unit tests: 9, all pure functions, none of the mutating paths

`agents.rs:952-1054`:

| Line | Test | Covers |
|---|---|---|
| `:958` | `test_contains_blurb` | `contains_blurb` |
| `:965` | `test_contains_legacy_blurb` | `contains_legacy_blurb` |
| `:973` | `test_get_blurb_version` | `get_blurb_version` |
| `:980` | `test_detect_agent_file` | `detect_agent_file` |
| `:998` | `test_detect_agent_file_with_blurb` | detection + `needs_upgrade` |
| `:1013` | `test_append_blurb` | `append_blurb` |
| `:1022` | `test_remove_blurb` | `remove_blurb`, happy path only |
| `:1031` | `test_update_blurb` | `update_blurb`, legacy→current |
| `:1040` | `test_detect_in_parents` | one-level parent walk |

`grep -c execute` over the whole file: 12 occurrences; over the `#[cfg(test)]` block
(`:952-1054`): **0**. `execute`, `execute_add`, `execute_remove`, `execute_update`,
`execute_json`, `execute_check` are entirely untested — meaning **`fs::write`, `fs::copy`, and
`io::stdin().read_line` are never exercised by any test at any level.**

`test_remove_blurb` (`:1022-1028`) asserts only three things:
```rust
assert!(!result.contains(BLURB_START_MARKER));
assert!(result.contains("# Agents"));
assert!(result.contains("More content."));
```
No byte-equality assertion, no length assertion, no round-trip assertion. That is why the
trailing-newline loss (§3.5) and all five corruption classes (§3.3) pass unnoticed.

`test_detect_in_parents` (`:1040-1053`) passes `max_levels = 3` but only ever creates one level
of nesting, so the 4-directory reach and the repo-escape it enables are untested.

---

## 7. Q6 — Verdict

### 7.1 How the sentence must be rewritten

> ~~"obr only writes inside `.beads/`."~~

Accurate version:

> **"`obr sync` never writes outside `.beads/`: its paths are checked against an explicit
> extension allowlist and rejected if they contain a `.git` component, and its writes are atomic
> temp-file-then-rename (`src/sync/path.rs:48-59`, `:140-175`). This guarantee is scoped to the
> sync layer and does not extend to `obr agents`, which is the sole exception: it locates
> `AGENTS.md`/`CLAUDE.md`/`agents.md`/`claude.md` by searching `$PWD` and up to three parent
> directories (`agents.rs:366`, `:225-244`) — a window that routinely reaches outside the
> current git repository and can reach inside `.git/` — follows symlinks, applies none of the
> sync validation, and replaces the whole file with a non-atomic `fs::write` behind a
> single-generation `.md.bak` copy whose failure is only a warning (`agents.rs:535-549`,
> `:621-633`, `:706-718`). It also creates `*.md.bak` files that no generated `.gitignore`
> covers. Separately, `obr agents` and `obr orphans --fix` read from stdin
> (`agents.rs:525`, `:611`, `:696`; `orphans.rs:203`); the first three are bypassable with
> `--force`, the last is not bypassable at all."**

The narrower "obr never runs git itself" claim survives at the process level — I found no
subprocess invocation in `agents.rs` — but is undercut in spirit by §5.3: the blurb obr writes
into third-party repositories instructs the *agent* to run `git add`/`git commit`/`git push` at
the end of every session.

### 7.2 Risk ranking (all confirmed by live execution)

| # | Risk | Severity | Trigger | Recoverable? |
|---|---|---|---|---|
| **1** | **Non-UTF-8 file → entire content replaced by the blurb, exit 0, no warning.** One stray byte in `AGENTS.md`. `--remove` afterwards destroys the backup and leaves a 0-byte file. (§3.4) | **Critical** | `obr agents --add --force` on any file with a non-UTF-8 byte | Only from the single `.bak`, only until the next mutation |
| **2** | **`remove_blurb` deletes everything between an unrelated marker mention and the first end marker.** R2: 292 → 64 bytes, two whole sections silently destroyed, "Removed …", exit 0. Triggered by an AGENTS.md that merely *documents* the beads integration. (§3.3-R2) | **Critical** | `obr agents --remove` / `--update` | One `.bak` generation |
| **3** | **Writes outside the git repo and inside `.git/`.** 3-ancestor walk with no boundary check; `~/AGENTS.md` and `~/CLAUDE.md` in range from any repo ≤3 levels under `$HOME`. `--remove` from an empty subdir silently edited the parent's file. (§2.2, §2.3) | **High** | any `obr agents` mutation from a nested directory | Yes, but the operator must first notice |
| **4** | **Indefinite hang for non-interactive agents.** `--remove`/`--update` prompt unconditionally without `--force`; stdin as an open pipe → `exit 124` after timeout, prompt stranded on stdout. `obr orphans --fix` has the same hang and **no force flag** to escape it. (§4.3, §4.5) | **High** | `obr agents --remove` from any agent harness | n/a (liveness) |
| **5** | **Exponential file growth / non-idempotent corruption.** R1: 308 → 356 → 452 → 644 → 1028 bytes across four identical `--remove` runs, with the blurb never removed so the loop never terminates. (§3.3-R1) | **High** | end-marker text appearing above the blurb | `.bak` destroyed after run 2 |
| **6** | **Silent no-op with exit 0.** stdin at EOF → "Aborted.", exit 0, indistinguishable from success. `--json` swallows `--add/--remove/--update` outright (`:368-370`, `:397`) and reports exit 0. `--add --check` and `--add --remove` silently drop a flag. (§1.1, §1.2, §4.2) | **Medium-High** | every non-interactive invocation without `--force`; every `--json` invocation | n/a |
| **7** | **Symlink following to arbitrary paths**, with the `.bak` written next to the link rather than the modified file. (§2.4) | **Medium-High** | `AGENTS.md` symlink anywhere in the 4-dir window | Yes, if the operator finds the `.bak` |
| **8** | **Backup failure is non-fatal.** `fs::copy` error → `eprintln!` warning → destructive write proceeds → exit 0. Demonstrated with a directory at `AGENTS.md.bak`. (§3.1) | **Medium** | any un-writable backup path | No |
| **9** | **Doc-drift injection into third-party repos.** 19 broken `br` invocations + 1 broken `bd`, wrong upstream URL, "JSONL" named twice for an Org-mode store, mandatory `git add/commit/push` checklist. `BLURB_VERSION` frozen at 1 so no deployed copy will ever be flagged stale. Nine more `br agents …` strings in the command's own help. (§5) | **Medium** (High reputationally) | every `obr agents --add` | Trivially, by editing the constant |
| **10** | **False success reports.** R4 (no end marker) reports "Removed" while changing nothing; R3/R5 report success while leaving a blurb behind; every mutation reports success after eating a trailing newline or welding two lines together. (§3.3, §3.5) | **Medium** | common | n/a |
| **11** | **`*.md.bak` not gitignored** → `?? AGENTS.md.bak` swept into commits by `git add -A`. (§3.1) | **Low-Medium** | every mutation on an existing file | Trivially |
| **12** | **Zero test coverage of every mutating path**, and `FileTreeSnapshot` never aimed here. (§6) | **Meta** | — | — |

### 7.3 Smallest set of fixes that would retire ranks 1–8

1. `execute_add:487-496` — if `detection.found() && detection.content.is_none()`, **error out**;
   never treat an unreadable file as empty. (Kills #1.)
2. `remove_blurb:267` — search for the end marker **from `start_idx`**, and assert
   `end_idx > start_idx`; refuse and error if the markers are unbalanced or appear more than
   once. (Kills #2 and #5.)
3. `execute:366` — pass `max_levels = 0`, or bound the walk at the git-repo root, or require an
   explicit `--path`. (Kills #3.)
4. `agents.rs:521/605/690` — treat `!io::stdin().is_terminal()` as an explicit refusal with a
   non-zero exit and a hint to pass `--force`, exactly as `src/main.rs:309` already does for
   output formatting. Add `--yes` to `OrphansArgs`. (Kills #4 and most of #6.)
5. `execute_json:395` — honour `_args`; perform the action and report it, or exit non-zero
   stating that mutations are unavailable in JSON mode. Add `conflicts_with` to the clap args.
   (Kills the rest of #6.)
6. Route the target path through `validate_no_git_path` and `dunce::canonicalize`, and write via
   temp+rename. (Kills #7 and part of #3.)
7. `execute_add:538` etc. — make backup failure fatal, and use a timestamped or numbered backup
   name. (Kills #8 and the "one generation" caveat under #1, #2, #5.)

---

## 8. Complete list of citations

**`src/cli/commands/agents.rs`** — `:16` BLURB_VERSION=1 · `:19` start marker · `:22` end
marker · `:25` SUPPORTED_AGENT_FILES · `:28-93` AGENT_BLURB · `:34` wrong upstream URL and
`br`/`bd` naming · `:54` "Export DB to JSONL" · `:64` mandatory flush · `:73-83` git
add/commit/push checklist · `:80` "Export beads changes to JSONL" · `:115-117` `found()` ·
`:127-132` `needs_upgrade` · `:137-139` `contains_blurb` (prefix-only) · `:143-146`
`contains_legacy_blurb` · `:157-165` `get_blurb_version` (`unwrap_or(0)`, u8 overflow→0) ·
`:169-193` `detect_agent_file` · `:196-221` `check_agent_file` (`:197` dir skip, `:201-208`
unreadable→empty) · `:225-244` parent walk · `:228` `0..=max_levels` · `:248-257`
`append_blurb` · `:261-289` `remove_blurb` (`:263` start, `:267` independent end, `:277-279`
trailing, `:283-286` leading≤2, `:288` splice) · `:293-325` `remove_legacy_blurb` · `:329-333`
`update_blurb` · `:337-339` preferred path · `:364-392` `execute` (`:365` cwd, `:366`
max_levels=3, `:368-370` json short-circuit, `:373-389` precedence) · `:395-413` `execute_json`
(`:397` `_args` unused) · `:432/443/449/452/459` `br agents …` help strings · `:465-560`
`execute_add` (`:489` `unwrap_or_default`, `:507-519` dry-run, `:521-533` prompt, `:535-547`
backup, `:549` write) · `:499-503` add→update delegation · `:562-644` `execute_remove`
(`:587-591` legacy-first branch, `:605-619` prompt, `:621-631` backup, `:633` write) · `:646-730`
`execute_update` (`:690-704` prompt, `:706-716` backup, `:718` write) · `:757/766/776/786` Rich
`br agents …` · `:952-1054` tests.

**`src/cli/mod.rs`** — `:654-693` global flags (`:658-660` `--db`, ignored by agents) ·
`:889-890` subcommand · `:2267-2279` `OrphansArgs` (no force) · `:2365-2391` `AgentsArgs`.

**`src/main.rs`** — `:114-124` dispatch · `:160-210` `should_auto_import` (`:205`
`Agents(_) => false`) · `:304-324` `handle_error` (`:309` `is_terminal` check).

**`src/sync/path.rs`** — `:48-56` `ALLOWED_EXTENSIONS` · `:59` `ALLOWED_EXACT_NAMES` ·
`:130-133` NGI-1/NGI-3 · `:140-175` `validate_no_git_path` (`:167` canonicalize).

**`src/cli/commands/orphans.rs`** — `:199-226` `--fix` interactive close (`:203` prompt,
`:207` `read_line`, `:211-221` DB mutation).

**`src/cli/commands/doctor.rs`** — `:9`, `:508-509` the only other consumer of path validation.

**`tests/e2e_sync_git_safety.rs`** — `:675-721` `FileTreeSnapshot` · `:1000/1016/1095/1115/1186/1207/1412/1427` all five uses, all `sync`.

**`tests/snapshots/snapshots/snapshots__snapshots__cli_output__help_output.snap:48`** — the only
doc mention.

**`/Users/johnw/src/obr/.beads/README.md`** — upstream Go `bd` boilerplate: `bd create`,
`bd sync`, `github.com/steveyegge/beads`, "Stored in `.beads/issues.jsonl`", "Auto-syncs with
your commits".

---

## 9. Incident disclosure

During experiment batching, one `cd "$SCRATCH/live"` failed because the harness had reaped that
scratchpad subdirectory between calls. Because the command chain used `;` rather than `&&`, the
shell remained in its default working directory — `/Users/johnw/src/obr` — and the next two
statements ran there:

```
printf 'x\n' > AGENTS.md
obr agents --add --force
```

This overwrote `/Users/johnw/src/obr/AGENTS.md` and created
`/Users/johnw/src/obr/AGENTS.md.bak`. I detected it immediately from `git status --porcelain`
showing `?? docs/superpowers/` (the repo's pre-existing untracked entry) and reverted:

```
$ git checkout -- AGENTS.md && rm -f AGENTS.md.bak
$ git status --porcelain
?? docs/superpowers/          <-- identical to the session-start snapshot
$ git diff --stat
                              <-- empty
```

The repository is byte-identical to its starting state. No other file was touched.

The incident is worth recording as evidence rather than merely as an apology: a single failed
`cd` — no flag error, no wrong argument, the command was exactly as intended — caused
`obr agents --add --force` to rewrite a protected file in a protected repository and **exit 0
reporting success**. That is precisely risk #3 and risk #6 from §7.2 firing together in the
wild, under an operator who had read the source and was actively trying to avoid them. It also
demonstrates the value of the `.md.bak` — and its limits: the `.bak` here captured the already
destroyed `x\n`, not the original file, because the `printf` preceded the `obr` call. Recovery
came from git, which is the only durable backup this command's design actually relies on, and
which it never checks for.
