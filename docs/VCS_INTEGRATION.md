# VCS Integration (Git Baseline)

obr never automatically runs version-control commands — all storage lives inside
`.obr/`. User-invoked reporting commands (`obr changelog`, `obr orphans`, and
commit-activity `obr stats`) read git history when explicitly called but never
write to or modify the repository. This document covers the **git baseline**
workflow plus best-effort command equivalents for other VCS (hg, jj, Perforce,
Sapling). Verify details with your VCS docs if your environment differs.

## Core Principles

- **obr never runs git automatically** — VCS operations are explicit and manual.
  Reporting commands (`changelog`, `orphans`, `stats`) read git history only
  when you invoke them.
- **Track `.obr/` in version control** — it is the collaboration surface.
- **Local-only artifacts stay local** — `.obr/.gitignore` excludes dbs, locks,
  and machine-specific files.

## Git Baseline Workflow

### Export before commit

```bash
obr sync --flush-only
git add .obr/
git commit -m "Update issues"
```

### Pull and import after sync

```bash
git pull --rebase
obr sync --import-only
```

## Common Git Commands

| Task | Git command |
|------|-------------|
| Stage `.obr/` changes | `git add .obr/` |
| Check status | `git status .obr/` |
| Inspect JSONL diff | `git diff PLAN.org` |
| Resolve JSONL conflicts | `git add PLAN.org` (after manual edit), then `obr sync --import-only` |

## Conflict Handling (JSONL)

JSONL is line-based, so conflicts are typically straightforward:

```bash
git status PLAN.org
vim PLAN.org   # resolve per-line conflicts
git add PLAN.org
obr sync --import-only
```

For more detail, see the README FAQ on JSONL conflicts.

## Ignore Files (Equivalents)

These are the usual ignore mechanisms for each VCS. Verify in your environment.

| VCS | Ignore file |
|-----|-------------|
| Git | `.gitignore` |
| Mercurial | `.hgignore` |
| Jujutsu | `.gitignore` (git-compatible working copy) |
| Perforce | `.p4ignore` (or `P4IGNORE` env) |
| Sapling | `.gitignore` / `.hgignore` (Sapling honors git/hg ignore files) |

---

## Mercurial (hg)

| Task | Command |
|------|---------|
| Stage `.obr/` changes | `hg add .obr/` |
| Check status | `hg status .obr/` |
| Inspect JSONL diff | `hg diff PLAN.org` |
| Resolve JSONL conflicts | edit file -> `hg resolve -m PLAN.org` |

## Jujutsu (jj)

jj is snapshot-based; there is no explicit "add" step for tracked files.

| Task | Command |
|------|---------|
| Check status | `jj status .obr/` |
| Inspect JSONL diff | `jj diff PLAN.org` |
| Record change | `jj commit -m "Update issues"` |

## Perforce (p4)

| Task | Command |
|------|---------|
| Open/add/edit `.obr/` | `p4 reconcile //.../.obr/...` |
| Check status | `p4 status //.../.obr/...` |
| Inspect JSONL diff | `p4 diff //.../PLAN.org` |
| Resolve JSONL conflicts | edit file -> `p4 resolve //.../PLAN.org` |

## Sapling (sl)

Sapling uses hg-like commands via the `sl` CLI.

| Task | Command |
|------|---------|
| Stage `.obr/` changes | `sl add .obr/` |
| Check status | `sl status .obr/` |
| Inspect JSONL diff | `sl diff PLAN.org` |
| Resolve JSONL conflicts | edit file -> `sl resolve -m PLAN.org` |
