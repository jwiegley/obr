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

The version is **`0.5.7+1`**. It has two halves:

- `0.5.7` — upstream `beads_rust` release this fork tracks.
- `+1` — fork generation since that upstream release. It increments on each obr
  release and resets to 1 whenever tracked upstream version changes.

`+N` is semver build metadata, only place a fourth component can live — Cargo
rejects `0.5.7.1` outright. The semver spec ignores build metadata for
precedence, while some version comparators order it. Never rely on that ordering
across tools. See `docs/RESIDUALS.md`.

Release tags carry version verbatim (`v0.5.7+1`); release asset names flatten
`+` to `.` (`obr-0.5.7.1-linux_amd64.tar.gz`).

Build from source:

```bash
cargo build --release      # binary lands at target/release/obr
```

```bash
obr --version
# obr 0.5.7+1
```

The repository pins a nightly toolchain via `rust-toolchain.toml`;
`nix develop` provides a matching shell if you use Nix.

## Quick start

```bash
obr init --prefix myproj        # creates .obr/ and seeds PLAN.org
obr create "Set up the schema" --type task --priority 1
obr create "Implement auth" --type feature --priority 1
obr dep add <auth-id> <schema-id>   # auth is blocked by schema
obr ready                       # what is unblocked right now
obr close <schema-id>
obr sync --flush-only           # write PLAN.org, then commit it yourself
```

A fresh clone has `PLAN.org` but no `.obr/`. Rebuild the cache from the file:

```bash
obr init && obr sync --import-only --rebuild
```

The `#+ISSUE_PREFIX:` keyword in `PLAN.org` is authoritative, so issue
identity survives the round trip.

## MCP server

Built with `--features mcp`, `obr serve` exposes the tracker to MCP-capable
agents over stdio. It opens no network listener and runs no git.

Tools: `list_issues`, `show_issue`, `create_issue`, `update_issue`,
`close_issue`, `manage_dependencies`, `project_overview`.

Resources: `obr://project/info`, `obr://issues/{id}`, `obr://schema`,
`obr://labels`, `obr://issues/ready`, `obr://issues/blocked`,
`obr://issues/in_progress`, `obr://issues/deferred`,
`obr://issues/bottlenecks`, `obr://coordination/status`,
`obr://graph/health`, `obr://events/recent`.

See [`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md) for the full `serve`
contract.

## Origins

`obr` stands on two projects. [beads](https://github.com/steveyegge/beads),
by Steve Yegge, defined the model: a dependency-aware issue tracker whose
data lives in your repository and whose primary consumer is a coding agent.
[beads_rust](https://github.com/Dicklesworthstone/beads_rust), by Jeffrey
Emanuel, reimplemented that model in Rust with a SQLite engine and a hardened
sync layer; obr is a fork of it at v0.5.7. What obr changes is the surface:
Org-mode instead of JSONL as the file you live in, one tracked `PLAN.org`
instead of a tracker directory, and a strict nothing-invasive posture. The
engine, and most of the credit, is theirs.

## Documentation

- [`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md) — every command and flag
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — how the two stores fit together
- [`docs/SYNC_SAFETY.md`](docs/SYNC_SAFETY.md) — what sync will and will not touch
- [`docs/TROUBLESHOOTING.md`](docs/TROUBLESHOOTING.md) — when something looks wrong
- [`docs/INSTALLING.md`](docs/INSTALLING.md) — installation paths
- [`AGENTS.md`](AGENTS.md) — rules for agents working in this repository
- [`docs/RESIDUALS.md`](docs/RESIDUALS.md) — known gaps, carried upstream
  breakage, and deliberate decisions

## License

The upstream code is MIT with an additional rider that restricts use by
specific parties; the rider is part of the license conditions and must
accompany any redistribution — read [`LICENSE`](LICENSE) in full before
depending on this project. New contributions made in this fork are MIT.

Copyright for the upstream work remains with Jeffrey Emanuel;
`CHANGELOG.md` is retained verbatim as that project's history.
