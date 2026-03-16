# obr

I've been tracking issues with various tools for years -- GitHub Issues,
Jira, plain TODO comments in code -- and none of them quite fit the way I
work. What I really wanted was something that lives in the repo itself,
works offline, never touches git behind my back, and gives me structured
data I can pipe through `jq` or hand to an AI agent.

`obr` is a local-first issue tracker built in Rust. It stores everything in
SQLite for fast queries, with JSONL export for git-friendly collaboration.
It's non-invasive by design: it won't run git commands, install hooks, or
modify anything outside `.beads/`.

## Getting started

You'll need Rust nightly (the project uses edition 2024). If you have Nix:

```bash
nix develop            # enter dev shell with nightly Rust
nix build              # build the obr binary
```

Without Nix:

```bash
cargo build --release
```

Then initialize in your project:

```bash
cd my-project
obr init               # creates .beads/ directory
obr create "Fix the login timeout" --type bug --priority 1
obr ready              # shows what's actionable
```

## How it works

There are really just three ideas here:

**SQLite for speed.** All queries hit a local SQLite database in
`.beads/beads.db`. Listing 10,000 issues takes under 100ms.

**JSONL for git.** When you're ready to share, `obr sync --flush-only`
exports the database to `.beads/issues.jsonl` -- one JSON object per line.
You `git add` and commit it yourself. Pulling changes from others is just
`obr sync --import-only` after a `git pull`.

**Content-addressed dedup.** Every issue gets a SHA-256 hash, so syncing
across repos won't create duplicates.

## Common commands

```bash
obr create "Title" -p 1 --type bug    # create an issue
obr list --status open --priority 0-1  # filter issues
obr ready                              # unblocked, actionable work
obr show bd-abc123                     # issue details
obr update bd-abc123 --status in_progress
obr close bd-abc123 --reason "Done"
obr dep add bd-child bd-parent         # dependency tracking
obr label add bd-abc123 backend auth   # labels
obr comments add bd-abc123 "Root cause found"
obr sync --flush-only                  # export to JSONL
obr doctor                             # diagnostics
obr stats                              # project statistics
```

Every command supports `--json` for machine-readable output, which is what
makes it handy for AI agent workflows. There's also `--quiet` for scripts
and `--no-color` for piped output.

## Configuration

`obr` uses layered config: CLI flags override environment variables, which
override `.beads/config.yaml`, which overrides `~/.config/beads/config.yaml`.

```yaml
# .beads/config.yaml
id:
  prefix: "proj"       # issue ID prefix (default: "bd")
defaults:
  priority: 2
  type: "task"
sync:
  auto_import: false
  auto_flush: false
```

## Dependencies

Issues can depend on other issues:

```bash
obr dep add bd-auth bd-schema    # auth blocked until schema is done
obr ready                        # only shows bd-schema
obr close bd-schema
obr ready                        # now bd-auth appears
obr dep tree bd-auth             # visualize the dependency graph
obr dep cycles                   # find circular dependencies
```

## Building and testing

```bash
nix develop                                    # dev shell
cargo build                                    # build
cargo test --lib --bins                        # unit tests
cargo clippy --all-targets -- --deny warnings  # lint
cargo fmt --check                              # format check
nix flake check                                # all of the above via Nix
```

Pre-commit hooks run automatically via [lefthook](https://github.com/evilmartians/lefthook)
if you have it installed (it's included in the Nix dev shell).

## Architecture

```
CLI (obr)
    │
    ▼
Storage Layer
    ├── SqliteStorage ◄──► JSONL Export/Import
    │   (WAL mode,          (atomic writes,
    │    dirty tracking,     content hashing,
    │    blocked cache)      merge support)
    │                        │
    ▼                        ▼
.beads/beads.db          .beads/issues.jsonl
```

All writes go through a 4-step mutation protocol: every modification passes
through `mutate()` with `FnOnce(&Transaction, &mut MutationContext)`. Unsafe
code is forbidden via `#![forbid(unsafe_code)]`.

## Safety guarantees

- Never executes git commands
- Only writes inside `.beads/`
- Atomic file writes (write to temp, then rename)
- Guards prevent overwriting non-empty JSONL with an empty database

## License

BSD 3-Clause -- see [LICENSE.md](LICENSE.md).
