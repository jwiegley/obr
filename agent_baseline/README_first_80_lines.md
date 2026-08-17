# obr

An issue tracker whose working surface is a single Org file.

## What it is

`obr` keeps your project's issues in one place a human actually reads:
`PLAN.org`, an Org-mode file tracked in git. Underneath, it is a thin layer
over a fast SQLite issue engine — dependencies, priorities, ready-work
queries, dedup, sync — inherited from
[beads_rust](https://github.com/Dicklesworthstone/beads_rust). The Org file
is bridged to that engine through the same line-oriented JSONL mechanism the
engine already understands, so obr adds a surface, not a second brain.

Two stores, with a clear split of responsibility:

- **`PLAN.org`** is the tracked surface. It lives under `doc/` if your project
  has one, else `docs/` if it has that, else at the project root — in that
  order of precedence; obr never creates either directory. This is the file
  git sees, the file you edit, and the file a fresh clone bootstraps from.
- **`.obr/`** is a per-machine cache — a SQLite database for fast queries,
  plus config, metadata, history and lock files. It ignores itself wholesale
  via its own `.gitignore`, and nothing inside it is ever committed.

Nothing invasive, by design: `obr` never runs git for you, never uses a
tracking branch, and never writes tracked metadata into a dot-directory.
Exports and imports are explicit; committing is yours to do.

## What PLAN.org looks like

```org
#+TITLE: Obr Issues
#+SEQ_TODO: TODO DOING DRAFT WAIT DEFER NOTE | DONE CANCELED
#+ISSUE_PREFIX: myproj

* TODO [#B] Implement auth
:PROPERTIES:
:ID:       myproj-4gs
:ISSUE_TYPE: feature
:CREATED:  [2026-08-08 Sat 02:10]
:END:

Blocked on the schema work; see the dependency below.
```

Headings are issues; TODO keywords are status; properties carry the
machine fields. Descriptions round-trip byte-exactly — content the Org
grammar cannot carry natively is preserved verbatim in example blocks, so
re-exports are a fixpoint: flush, import, and flush again produce the
identical file.

Times are Org-native inactive timestamps in your machine's local zone —
`:CREATED:`, `:MODIFIED:`, `:FINISHED:`, `:DUE:`, `:DEFERRED:`, `:DELETED:`,
`:COMPACTED:` — so Org's own tooling reads them: `org-sort-entries`,
`org-entry-get`, `org-ql`, column view, `C-c .`. Closing time is `:FINISHED:`
rather than `:CLOSED:`, which `org-special-properties` reserves and shadows.
Inactive is a choice, not a limit: an *active* `<…>` timestamp in a drawer
does reach the day agenda, which is no place for hundreds of issues. Org has
nowhere to put a UTC offset or a seconds field, so — deliberately — the
stored precision is one minute and two machines in different zones write the
same issue as different bytes; `docs/RESIDUALS.md` states the cost. Files
written before this change spelled these `:CREATED_AT:` and so on with
RFC3339 values; those are still read, and re-exported in the current form.

Editing it by hand is expected, within one contract: obr rewrites the file
from its database on every flush, so an issue may only carry the level-2
sections obr stores (`Design`, `Acceptance Criteria`, `Notes`, `Close
Reason`, `Delete Reason`, `Agent Context`, `Dependencies`, `Comments`).
Any other section under an issue warns on import and is dropped on the next
rewrite; a `Dependencies`, `Comments`, or `Agent Context` section whose
`#+begin_src json` block has been broken fails the import outright rather
than silently importing as empty.

## Status

Unreleased. There is no published binary, no package-manager tap, and no
upgrade command. The packaging manifests under `packaging/` are repointed at
this fork but carry placeholder checksums until a first release is cut.

The version is **`0.3.2+1`**. It has two halves:
