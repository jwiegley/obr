# GAP-FILL: `org2jsonl` — the unvendored parser of obr's durable artifact

Research date: 2026-08-06. Both repos treated strictly read-only; all experiments ran in
`/private/tmp/claude-501/-Users-johnw-src-obr/f80d1967-0fc0-44fa-a53f-3054d39727e2/scratchpad/`
(copies at `scratchpad/work/obr` and `scratchpad/work/org2jsonl`).

Binaries used:
- `obr 0.1.14` installed via home-manager at `/etc/profiles/per-user/johnw/bin/obr`
  (release profile → `panic = "abort"`, built by Nix against the *flake-pinned* org2jsonl rev).
- `scratchpad/work/obr/target/debug/obr` — built from the current obr worktree against the
  current org2jsonl worktree, using the nightly toolchain from `nix develop /Users/johnw/src/obr`.
- `scratchpad/work/org2jsonl/target/debug/{org2jsonl,jsonl2org}` — built from the current
  org2jsonl worktree.

---

## 0. Executive summary

1. `org2jsonl` is a **general-purpose Org⇄JSONL converter**, not purpose-built for obr. It has one
   obr-specific concession: `org_to_entries_with_keywords` (org2jsonl `src/org_to_json.rs:61-75`),
   added in commit `52f873c` *"Add `org_to_entries_with_keywords()` for custom TODO keyword
   parsing"* — that is the single entry point obr uses.
2. **obr uses ~52% of the crate.** It uses `model` (312 lines) + `org_to_json` (2,274 lines).
   It never touches `json_to_org` (2,231 lines) or the two binaries (165 lines). The crate's
   headline claim (lossless adjunction) is a property of the *writer*, which obr does not use.
3. **The round-trip loss is overwhelmingly obr's, not the parser's.** The parser models 23
   `Element` variants and 22 `InlineContent` variants; obr's `extract_body_text`
   (`src/sync/org_bridge.rs:540-554`) consumes exactly **1 of 23** Element variants, and
   `flatten_inline_to_text` (`src/sync/org_bridge.rs:451-537`) handles **14 of 22** inline
   variants. Lists, tables, src blocks, quote blocks, drawers, keywords, comments, fixed-width,
   clocks, footnote definitions and 12 more block types are parsed correctly and then thrown away
   by obr.
4. **The parser also has its own first-round-trip infidelity**, including a data-corrupting
   `CLOCK: ` duplication bug and an **unbounded-growth** non-idempotency (6-byte input grows by
   1 byte per round-trip, forever), which directly falsifies the README's central claim and which
   the crate's own `fuzz/fuzz_targets/fuzz_roundtrip.rs` asserts against. Nobody ran it.
5. **Empirically confirmed non-fixpoint in obr's own pipeline**: 318/30,000 random descriptions
   differ between generation 1 and generation 2 of flush→import; 2,807/30,000 (9.4%) lose content
   on the *first* pass. Convergence takes **two** cycles, not one — so a git-tracked `issues.org`
   can churn across two independent commits from two different clones.
6. **Emission is deterministic** (byte-identical across 10+ runs and processes); ordering of
   issues, labels, dependencies and comments is stable. This is the one thing that works.
7. **No panic found** in 100,000 mutation-fuzzed parses (+writes +reparses), so the
   `panic = "abort"` → exit 134 hazard is real-but-unrealized. But there **is** an
   O(n²) parse blowup reachable from the git-tracked file, and a single heading missing `:ID:`
   **bricks every obr command in the workspace**.
8. **Supply/reproducibility verdict: broken in three independent ways** — `path` dep with no
   version, flake pinned to a `file://` URL on one machine, and the flake pin is **15 commits
   behind the worktree** so `nix build` and `cargo build` compile *different parsers both called
   0.1.0*. Additionally obr's committed `Cargo.lock` is **stale** against the org2jsonl worktree,
   so `cargo build --locked` fails outright.

---

## 1. What the crate is

### 1.1 Identity, dependencies, layout

`/Users/johnw/src/org2jsonl/Cargo.toml:1-7`:
```toml
[package]
name = "org2jsonl"
version = "0.1.0"
edition = "2021"
description = "Convert Emacs Org-mode files to/from JSONL for manipulation with standard JSON tools"
license = "BSD-3-Clause"
authors = ["John Wiegley <johnw@newartisans.com>"]
```
No `publish = false`, no `repository`, no `readme` key — it is simply unpublished.
Note `edition = "2021"` while obr is `edition = "2024"` (`/Users/johnw/src/obr/Cargo.toml:4`).

Dependencies (`Cargo.toml:9-15`): `clap 4` (derive), **`orgize 0.10.0-alpha.10`**, `serde 1`,
`serde_json 1`, `anyhow 1`, `walkdir 2`. Dev-deps (`:17-21`): `proptest 1`,
`pretty_assertions 1`, `tempfile 3`, `criterion 0.5`.

The load-bearing fact: **the actual Org parsing is `orgize 0.10.0-alpha.10`** — an *alpha*
release of a third-party crate. `org2jsonl` is a CST→model converter on top of it
(`src/org_to_json.rs:8-18` imports 30+ `orgize::ast` node types). So obr's durable format is
parsed by a pre-1.0 alpha, wrapped by an unpublished path dep.

Source layout (7,024 Rust lines total, `target/` excluded):

| File | Lines | obr uses? |
|---|---|---|
| `src/org_to_json.rs` | 2,274 | **yes** (one fn) |
| `src/json_to_org.rs` | 2,231 | **no** |
| `tests/integration_tests.rs` | 1,924 | n/a |
| `src/model.rs` | 312 | **yes** (types) |
| `benches/bench_roundtrip.rs` | 89 | no |
| `src/bin/jsonl2org.rs` | 86 | no |
| `src/bin/org2jsonl.rs` | 79 | no |
| `fuzz/fuzz_targets/fuzz_roundtrip.rs` | 15 | no |
| `fuzz/fuzz_targets/fuzz_parse.rs` | 9 | no |
| `src/lib.rs` | 5 | — |

`src/lib.rs` in full:
```rust
pub mod json_to_org;
pub mod model;
pub mod org_to_json;

pub const SCHEMA_VERSION: u32 = 1;
```

### 1.2 Binaries — correction to the task premise

The task states "the crate ships a `roundtrip` binary". It does **not**. `Cargo.toml:23-31`
declares exactly two `[[bin]]` targets, `org2jsonl` and `jsonl2org`. What exists is:

- `Cargo.toml:33-36` — a **criterion bench** named `roundtrip` (`benches/bench_roundtrip.rs`),
  which measures parse/write/round-trip *speed*, not fidelity.
- `/Users/johnw/src/org2jsonl/roundtrip.sh` (5 lines, mode 755) — the actual "oracle":
  ```bash
  #!/usr/bin/env bash
  diff "$1" <(cargo run --bin org2jsonl -- "$1" | cargo run --bin jsonl2org)
  ```

So the built-in oracle is a two-line shell script, not a binary. I used it (§3.1) and it does
find real bugs immediately, including on the crate's own fixtures.

### 1.3 Model types

`src/model.rs` defines:

- `OrgEntry` (`:9-34`) — `schema_version`, optional `file`/`char_begin`/`char_end`/
  `line_begin`/`line_end` location metadata, flattened `content: EntryContent`, `post_blank`.
- `EntryContent` (`:36-49`) — `Section { elements, body_spacing }` or `Heading(Box<Heading>)`.
- `Heading` (`:51-93`) — `level`, `keyword`, `priority`, `title: Vec<InlineContent>`, `tags`,
  `planning`, `properties: Vec<Property>`, `pre_body_blank`, `body: Vec<Element>`,
  `body_spacing: Vec<bool>`, `post_body_blank`, `children: Vec<Heading>`, `post_blank`.
- `Planning` (`:95-103`) — `closed`/`deadline`/`scheduled`, raw strings.
- `Property` (`:105-109`) — `{ key: String, value: String }`.
- `Element` (`:112-186`) — **23 variants**, enumerated in §2.3.
- `ListKind` (`:188-194`), `ListItem` (`:196-219`), `CheckboxState` (`:221-227`),
  `TableRow` (`:229-237`) with `cell_widths` for width-faithful re-emission,
  `TableRowKind` (`:239-246`).
- `InlineContent` (`:249-312`) — **22 variants**, enumerated in §2.4.

The model is genuinely comprehensive: it carries blank-line accounting (`post_blank`,
`pre_body_blank`, `body_spacing`, `post_body_blank`, `content_spacing`) and per-row table cell
widths purely so the writer can reproduce byte-level formatting. That is a *general Org tool*
design, not an issue-tracker design.

### 1.4 Git history / maturity

`git -C /Users/johnw/src/org2jsonl log --oneline | wc -l` → **30 commits**, all authored
2026-02-18 through 2026-03-18 (one month). Remote: `git@github.com:jwiegley/org2jsonl.git`
(private/unpublished as far as obr's build is concerned — obr does not reference it).
Branches: `main` plus `origin/dependabot/cargo/rand-0.9.4`. **No tags** — hence nothing to pin
to semantically.

History (newest first), with the flake pin marked:

```
59521f9 Add tests                                                       <- worktree HEAD
7202b96 Add end-to-end integration tests for multi-file location metadata
360b573 Support multiple files and directories in jsonl2org with FILE property
8fefe2d Add inject_file_properties for FILE property in multi-file output
8401de3 Support multiple files and directories in org2jsonl
bb99c7b Add org_to_entries_with_source for location metadata
9b91599 Add optional location fields to OrgEntry
b8fc44d Add walkdir dependency for directory traversal                  <- adds the dep that
bf44e27 Rewrite README in project voice                                    breaks obr's Cargo.lock
01af2fc Expand CI and Makefile build targets
fccdc6b Add lefthook pre-commit hooks
ec0f85a Add test and doc checks to flake, update flake.lock
5a31542 Add criterion benchmarks and fuzz testing targets
3cde81c Add BSD-3-Clause license
d804c86 Fix clippy warnings and apply rustfmt
5ea7586 Add roundtrip test script and proptest regression seeds         <- obr flake.lock pin
ebe4111 Fix 6 categories of roundtrip fidelity failures                    (revCount 15)
52f873c Add org_to_entries_with_keywords() for custom TODO keyword parsing  <- the obr hook
5170a5c Fix CI coverage: add 26 unit tests and exclude bin/ from threshold
5f3f9bd Add README.md
f55cf8b Preserve keyword spacing and fix nested blank line detection
a90904c Fix blank line preservation for headings with children, add CI workflow
568465a Improve round-trip fidelity with per-row table widths, precise list spacing, ...
8f80b32 Track inter-element spacing for faithful round-trip blank line preservation
f81ad13 Preserve blank line structure and fix formatting for round-trip fidelity
c0b9bd8 Add llvm-cov coverage testing infrastructure
6126c1f Include .org fixture files in Nix source filter
1292ef4 Fix Darwin SDK reference in flake.nix for modern nixpkgs
5a38b40 Add flake.lock for reproducible Nix builds
53786ad Initial implementation of org2jsonl/jsonl2org bidirectional converter
```

Six of the thirty commits are round-trip-fidelity firefighting (`f81ad13`, `8f80b32`, `568465a`,
`a90904c`, `f55cf8b`, `ebe4111`). That is the crate's dominant engineering theme, and §3
shows it is still not finished.

Tests: 153 tests, all passing.
```
$ cargo test   # in scratchpad/work/org2jsonl
test result: ok. 153 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```
Distribution: `tests/integration_tests.rs` 103 `#[test]`, `src/json_to_org.rs` 66,
`src/org_to_json.rs` 30. Plus 5 proptest blocks
(`tests/integration_tests.rs:1316,1365,1398,1433,1471`) at 30–100 cases each.
`docs/COVERAGE.md` states a 89% line-coverage floor enforced by
`.github/workflows/coverage.yml` and `lefthook.yml`. **Coverage is high and fidelity is still
broken** — see §1.5 for why.

Also present: `fuzz/` with two cargo-fuzz targets, `.github/workflows/{ci,coverage}.yml`,
`lefthook.yml` running fmt/clippy/test/doc/coverage/bench/`nix flake check`,
`tests/integration_tests.proptest-regressions` with exactly one saved seed
(`shrinks to input = "* A\n\n** A\n"`).

### 1.5 Why 153 tests + 89% coverage miss the bugs

The fixture round-trip tests (`tests/integration_tests.rs:81-140`) generate five tests per
fixture and **not one of them asserts `round_trip(FIXTURE) == FIXTURE`**:

- `round_trip_produces_valid_output` (`:93-98`) asserts only `output.ends_with('\n')`.
- `json_round_trip_preserves_entries` (`:100-102`) asserts `entries == recovered` — a
  serde round-trip of the *model*, which cannot detect parser or writer loss.
- `idempotency` (`:104-106`) asserts `round_trip(x) == round_trip(round_trip(x))` — the
  **second** round-trip, by construction blind to first-pass loss.
- `schema_version_present_in_all_entries` (`:108-119`), `jsonl_format_valid` (`:121-…`) —
  structural only.

So the whole fixture suite is compatible with the parser deleting arbitrary content, as long as
it deletes it consistently. Section 3.1 shows 5 of 11 shipped fixtures do in fact fail a true
`input == round_trip(input)` check.

The proptest strategies are similarly narrow:
`tests/integration_tests.rs:1309-1314`
```rust
fn maybe_body() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        "[A-Za-z][A-Za-z0-9 .]{0,50}".prop_map(|s| format!("\n{}\n", s.trim_end())),
    ]
}
```
The alphabet is `[A-Za-z0-9 .]` — it contains no `-`, `|`, `:`, `#`, `*` or blank lines, so it
can never generate the 6-byte growth reproducer in §3.2, nor a list, table, or clock line.

---

## 2. The obr ⇄ org2jsonl seam

### 2.1 Exact API surface used

Every reference in obr (`grep -rn 'org2jsonl'`):

| obr location | Symbol |
|---|---|
| `Cargo.toml:64` | `org2jsonl = { path = "../org2jsonl" }` |
| `flake.nix:21-23` | flake input `url = "git+file:///Users/johnw/src/org2jsonl"` |
| `flake.nix:59-60`, `:102-103` | `postUnpack: cp -r ${inputs.org2jsonl} org2jsonl` |
| `src/sync/org_bridge.rs:12` | `use org2jsonl::model::{Element, EntryContent, Heading, InlineContent};` |
| `src/sync/org_bridge.rs:336-340` | `org2jsonl::org_to_json::org_to_entries_with_keywords(...)` |
| `src/sync/org_bridge.rs:585` | `fn parse_properties(properties: &[org2jsonl::model::Property], ...)` |
| `src/sync/org_bridge.rs:838` | `use org2jsonl::model::InlineContent;` (unit test) |
| `src/sync/mod.rs:1178-1182` | `org_to_entries_with_keywords(...)` in `count_issues_in_org` |
| `src/sync/mod.rs:1188` | `org2jsonl::model::EntryContent::Heading(heading)` |

That is the **entire** surface: **one function** and **five types**
(`Element`, `EntryContent`, `Heading`, `InlineContent`, `Property`).
`json_to_org::entries_to_org` — 2,231 lines, the half of the crate the README is about — is
never called by obr. obr writes Org itself, byte by byte, in
`emit_issue_heading` (`src/sync/org_bridge.rs:45-237`).

Consequences of using only the parser:
- The parser's `post_blank` / `body_spacing` / `pre_body_blank` / `post_body_blank` /
  `cell_widths` machinery — the crate's whole fidelity apparatus — is **dead weight** for obr.
- The `json_to_org` bugs (§3.2, §3.3) do **not** affect obr directly. But they are the crate's
  only fidelity oracle, so their existence means the crate's fidelity is unmeasured.

### 2.2 What obr does with the parse tree

`org_text_to_issues` (`src/sync/org_bridge.rs:335-355`):
```rust
let entries = org2jsonl::org_to_json::org_to_entries_with_keywords(
    org_text, BEADS_TODO_KEYWORDS, BEADS_DONE_KEYWORDS,
);
for entry in entries {
    if let EntryContent::Heading(heading) = entry.content {
        if heading.level == 1 { issues.push(parse_heading_to_issue(&heading)?); }
    }
    // Skip Section entries (file header)
}
```

`parse_heading_to_issue` (`:358-434`) maps:
- `heading.keyword` → `Status` via `keyword_to_status` (`:276-287`).
- `heading.priority` → `Priority` via `org_to_priority` (`:301-309`).
- `heading.title` → `issue.title` via `extract_title_text` → `flatten_inline_to_text`.
- `heading.tags` → `issue.labels` (sorted).
- `heading.properties` → 31 scalar fields via `parse_properties` (`:585-751`).
- `heading.body` → `issue.description` via `extract_body_text` (`:540-554`).
- `heading.children` where `child.level == 2`, dispatched on the **plain-text title string**:
  `"Design"`, `"Acceptance Criteria"`, `"Notes"` → `extract_body_text`;
  `"Dependencies"`, `"Comments"` → `extract_json_from_body` (`:557-566`), which returns the
  first `Element::SrcBlock` whose `language == "json"`. Any other child title is
  **silently discarded** (`:426-428`).

Everything else the parser produced is dropped on the floor:

| Parser output obr ignores | Where |
|---|---|
| `OrgEntry::post_blank` | never read |
| `OrgEntry::file/char_begin/char_end/line_begin/line_end` | never read — and *cannot* be, see below |
| `EntryContent::Section` (the `#+TITLE:` / `#+SEQ_TODO:` header obr itself wrote) | `org_bridge.rs:351` |
| `Heading::planning` (SCHEDULED / DEADLINE / CLOSED) | never read |
| `Heading::pre_body_blank`, `body_spacing`, `post_body_blank`, `post_blank` | never read |
| `Heading::children` at level ≥ 3 | `org_bridge.rs:392` (`if child.level == 2`) |
| level-2 children with unrecognised titles | `org_bridge.rs:426-428` |
| headings at level ≥ 2 that are direct entries | `org_bridge.rs:346` (`if heading.level == 1`) |

The location-metadata point is worth its own note: `org_to_entries_with_keywords`
(org2jsonl `src/org_to_json.rs:61-75`) hard-codes `file: None`:
```rust
let org = config.parse(input);
org_to_entries_from_parsed(org, input, None)   // <- src/org_to_json.rs:74
```
and `org_to_entries_from_parsed:208-224` only populates `line_begin`/`line_end` when `file` is
`Some`. There is no `org_to_entries_with_keywords_and_source`. So obr **structurally cannot**
report a line number for a bad heading — which is exactly what §5.4 shows it failing to do.

### 2.3 Element variants: parser emits vs obr handles

`extract_body_text` in full (`src/sync/org_bridge.rs:540-554`):
```rust
fn extract_body_text(body: &[Element]) -> String {
    let mut text = String::new();
    for (i, element) in body.iter().enumerate() {
        if let Element::Paragraph { contents } = element {
            if i > 0 { text.push_str("\n\n"); }
            let para_text = extract_title_text(contents);
            text.push_str(&para_text);
        }
    }
    text
}
```

Note the second bug hiding in this function beyond the obvious one: the `"\n\n"` separator is
keyed on `i > 0` — the *index in `body`*, not on whether a paragraph was actually emitted. So
when a non-paragraph element sits between two paragraphs, the separator is still inserted, and
because orgize's paragraph TEXT token retains its own trailing `\n`, the result is the
signature `"Before.\n" + "\n\n" + "After."` = **`Before.\n\n\nAfter.`** seen throughout §3.4.

All 23 `Element` variants (`org2jsonl/src/model.rs:112-186`) and obr's treatment:

| # | Variant | model.rs | obr treatment |
|---|---|---|---|
| 1 | `Paragraph` | `:115-116` | **handled** — inline-flattened |
| 2 | `PlainList` | `:117-121` | **dropped** (lists, checkboxes, nesting all lost) |
| 3 | `SrcBlock` | `:122-128` | **dropped from body text**; consumed *only* by `extract_json_from_body` when `language == "json"` under a `Dependencies`/`Comments` child |
| 4 | `ExampleBlock` | `:129-130` | dropped |
| 5 | `QuoteBlock` | `:131-132` | dropped |
| 6 | `CenterBlock` | `:133-134` | dropped |
| 7 | `VerseBlock` | `:135-136` | dropped |
| 8 | `CommentBlock` | `:137-138` | dropped |
| 9 | `ExportBlock` | `:139-140` | dropped |
| 10 | `SpecialBlock` | `:141-147` | dropped |
| 11 | `Drawer` | `:148-149` | dropped (silently deletes user `:PROPERTIES:`-style text) |
| 12 | `Table` | `:150-151` | dropped |
| 13 | `HorizontalRule` | `:152-156` | dropped |
| 14 | `Keyword` | `:157-158` | dropped (`#+TITLE:`, `#+SEQ_TODO:` in a body vanish) |
| 15 | `Comment` | `:159-160` | dropped (`# ...` lines vanish) |
| 16 | `FixedWidth` | `:161-162` | dropped (`: ...` lines vanish) |
| 17 | `Clock` | `:163-164` | dropped |
| 18 | `DiarySexp` | `:165-166` | dropped |
| 19 | `FootnoteDefinition` | `:167-171` | dropped |
| 20 | `AffiliatedKeyword` | `:172-173` | dropped |
| 21 | `LatexEnvironment` | `:174-175` | dropped |
| 22 | `DynamicBlock` | `:176-182` | dropped |
| 23 | `Raw` (fallback) | `:183-185` | dropped |

**1 handled, 1 conditionally consumed, 21 dropped.** No variant is *mangled* by obr — the
mangling (`Before.\n\n\nAfter.`) is the collateral of the `i > 0` separator, not per-variant
misinterpretation.

### 2.4 InlineContent variants: parser emits vs obr handles

`flatten_inline_to_text` (`src/sync/org_bridge.rs:451-537`) ends with
`_ => {} // Skip unknown variants` at `:535`. All 22 variants
(`org2jsonl/src/model.rs:249-312`):

| # | Variant | model.rs | obr treatment (`org_bridge.rs`) |
|---|---|---|---|
| 1 | `Text` | `:252-253` | handled `:453-455` |
| 2 | `Bold` | `:254-255` | handled — re-wraps in `*…*` `:480-486` |
| 3 | `Italic` | `:256-257` | handled — `/…/` `:487-493` |
| 4 | `Underline` | `:258-259` | handled — `_…_` `:494-500` |
| 5 | `StrikeThrough` | `:260-261` | handled — `+…+` `:501-507` |
| 6 | `Code` | `:262-263` | handled — `~…~` `:508-512` |
| 7 | `Verbatim` | `:264-265` | handled — `=…=` `:513-517` |
| 8 | `Link` | `:266-271` | handled — `[[path][desc]]` `:524-534` |
| 9 | `Timestamp` | `:272-273` | handled `:453-455` (raw value) |
| 10 | `FootnoteReference` | `:274-279` | **dropped** `:535` |
| 11 | `LineBreak` | `:280-281` | handled — emits `\n` `:518` |
| 12 | `Entity` | `:282-283` | **mangled** `:519-523` — emits `\NAME{}`, adding `{}` that was not in the source |
| 13 | `LatexFragment` | `:284-285` | handled `:453-455` |
| 14 | `ExportSnippet` | `:286-287` | **dropped** `:535` |
| 15 | `InlineBabel` | `:288-289` | **dropped** `:535` |
| 16 | `InlineSrc` | `:290-291` | **dropped** `:535` |
| 17 | `Macro` | `:292-293` | **dropped** `:535` (`{{{m(a)}}}` vanishes) |
| 18 | `Target` | `:294-295` | **dropped** `:535` (`<<tgt>>` vanishes) |
| 19 | `RadioTarget` | `:296-297` | **dropped** `:535` |
| 20 | `StatisticsCookie` | `:298-299` | **dropped** `:535` (`[1/2]`, `[50%]` vanish) |
| 21 | `Subscript` | `:300-305` | handled `:456-467` |
| 22 | `Superscript` | `:306-311` | handled `:468-479` |

**14 handled, 1 mangled (`Entity`), 7 dropped.**

### 2.5 Attribution: how much loss is obr's vs the parser's

Measured directly (§3.1): running the parser's own `roundtrip.sh` oracle over the exact
`issues.org` files obr produces shows the parser reproduces lists, tables, src blocks, quote
blocks, drawers, keywords, comments, fixed-width lines and footnote definitions **byte for byte**
except for four normalizations. Verbatim, on a 400-line obr-generated `issues.org`, the *only*
parser-side differences were:

```
@@ -267,8 +267,6 @@
 para1
-
-
 para2                       <- 3 blank lines collapsed to 1
@@ -392,6 +390,6 @@
-line with trailing
+line with trailing          <- trailing whitespace stripped (documented canonical form)
```

Everything else in that file survives the parser and is destroyed by obr.

**Attribution verdict:** of the loss classes prior research attributed to `extract_body_text`,
essentially **all of the structural loss is obr's**. The parser's independent contributions to
infidelity are narrow: blank-run collapse, trailing-whitespace stripping, `Entity` brace
normalization, `CLOCK: ` duplication, property-value column alignment, planning-keyword
reordering, and table numeric-alignment loss (§3.1). None of these except blank-run collapse
show up in obr's pipeline, because obr never runs the writer.

The one place obr's loss is *caused* by the parser's model rather than obr's laziness:
`Entity` (`\alpha` → `\alpha{}`) is obr adding braces the source didn't have, because the
parser's `Entity { name }` variant discards whether the source used `{}`. That is a genuine
model-level lossiness (`model.rs:282-283` carries only `name`).

---

## 3. Round-trip fidelity, empirically

### 3.1 The parser's own oracle (`roundtrip.sh`), on obr output and on the crate's fixtures

```
$ for f in <obr-generated>.org /Users/johnw/src/org2jsonl/tests/fixtures/*.org; do
      org2jsonl "$f" | jsonl2org | diff -q "$f" -   ; done

DIFFERS org_gen1.org  (4 changed lines)     <- obr-generated, 400 lines
DIFFERS org_gen2.org  (13 changed lines)
DIFFERS c2_gen1.org   (2 changed lines)
OK      complex_lists.org
OK      deep_nesting.org
DIFFERS edge_cases.org  (6 changed lines)
DIFFERS full_document.org  (14 changed lines)
DIFFERS inline_objects.org  (2 changed lines)
OK      links.org
OK      minimal.org
OK      no_headings.org
DIFFERS simple.org  (4 changed lines)
DIFFERS tables.org  (6 changed lines)
OK      timestamps.org
```

**5 of the crate's 11 own fixtures fail its own oracle.** Verbatim samples:

`tests/fixtures/edge_cases.org` — property alignment changed and planning keywords **reordered**:
```diff
 * Heading with only properties
 :PROPERTIES:
-:ID: abc123
+:ID:       abc123
 :END:
 * Heading with planning and properties
-DEADLINE: <2024-06-01 Sat> SCHEDULED: <2024-05-15 Wed>
+SCHEDULED: <2024-05-15 Wed> DEADLINE: <2024-06-01 Sat>
```
(The reorder is inherent to `Planning` being a struct with fixed field order —
`model.rs:95-103` — plus the writer's fixed emission order.)

`tests/fixtures/tables.org` — numeric right-alignment destroyed despite `cell_widths`:
```diff
-| Apple |  1.50 |   3 |  4.50 |
-| Bread |  2.00 |   1 |  2.00 |
+| Apple | 1.50  | 3   | 4.50  |
+| Bread | 2.00  | 1   | 2.00  |
```

`tests/fixtures/inline_objects.org` — entity braces stripped (the exact inverse of what obr does):
```diff
-Entity: \alpha{} \beta{} \rightarrow{}
+Entity: \alpha \beta \rightarrow
```

### 3.2 Unbounded-growth non-idempotency — 6-byte reproducer

The README claims: *"the first round-trip through JSONL may normalize things … but every
round-trip after that is byte-identical."* This is false. Minimal reproducer, file
`grow0.org` containing exactly `-\n* h\n` (6 bytes):

```
$ printf -- '-\n* h\n' > grow0.org
$ for i in 1..6; do org2jsonl cur.org | jsonl2org > $i.org; cp $i.org cur.org; done

iter 1:  7 bytes :: -$|$|* h$|
iter 2:  8 bytes :: -$|$|$|* h$|
iter 3:  9 bytes :: -$|$|$|$|* h$|
iter 4: 10 bytes :: -$|$|$|$|$|* h$|
iter 5: 11 bytes :: -$|$|$|$|$|$|* h$|
iter 6: 12 bytes :: -$|$|$|$|$|$|$|* h$|
```
(one `$` per newline, from `cat -A`). It grows by exactly one blank line per round-trip,
without bound. A second reproducer with the same behaviour: `* h\n- a\n-\n\n** c\n`.

This is precisely what `fuzz/fuzz_targets/fuzz_roundtrip.rs:6-15` asserts:
```rust
let entries = org_to_entries(s);
let org1 = entries_to_org(&entries);
let entries2 = org_to_entries(&org1);
let org2 = entries_to_org(&entries2);
assert_eq!(org1, org2, "idempotency violation on fuzzed input");
```
A 6-byte input from the crate's own domain trips it. The fuzz target has never been run
(`.gitignore` lists `fuzz/artifacts/` and `fuzz/corpus/`, both absent; `lefthook.yml` and
`.github/workflows/ci.yml` never invoke `cargo fuzz`).

I ran a 100,000-iteration mutation fuzzer against `org_to_entries_with_keywords` →
`entries_to_org` → reparse → rewrite (driver at
`scratchpad/drv/src/main.rs`, seeded with obr-shaped and Org-feature-rich inputs).
Result: **`panics=0`**, but idempotency violations were found in the thousands, e.g.

```
NON-IDEMPOTENT
 input="…\n** Design\n- a\n- \n\n** Dependencies\n…"
 t1   ="…\n** Design\n- a\n-\n\n\n** Dependencies\n…"
 t2   ="…\n** Design\n- a\n-\n\n\n\n** Dependencies\n…"     <- still growing at gen 3
```

### 3.3 `CLOCK:` prefix duplication — a first-pass data-corruption bug

Parser (`org2jsonl/src/org_to_json.rs:680-685`):
```rust
if let Some(clock) = Clock::cast(node.clone()) {
    return Some(Element::Clock { value: clock.raw().trim().to_string() });
}
```
`clock.raw()` **includes** the literal `CLOCK: ` prefix — verified:
```
$ printf '* Head\nCLOCK: [2026-01-01 Wed 10:00]--[2026-01-01 Wed 11:00] =>  1:00\n' | org2jsonl --pretty
  "body": [ { "type": "clock",
              "value": "CLOCK: [2026-01-01 Wed 10:00]--[2026-01-01 Wed 11:00] =>  1:00" } ]
```
Writer (`org2jsonl/src/json_to_org.rs:465-470`):
```rust
Element::Clock { value } => {
    buf.push_str(&prefix);
    buf.push_str("CLOCK: ");
    buf.push_str(value);
    buf.push('\n');
}
```
Result — one round-trip permanently corrupts the line, then stabilises:
```
input : CLOCK: [2026-01-01 Wed 10:00]--[2026-01-01 Wed 11:00] =>  1:00
iter 1: CLOCK: CLOCK: [2026-01-01 Wed 10:00]--[2026-01-01 Wed 11:00] =>  1:00
iter 5: CLOCK: CLOCK: [2026-01-01 Wed 10:00]--[2026-01-01 Wed 11:00] =>  1:00
```
The unit test that "covers" `Element::Clock` (`json_to_org.rs:1636-1642`) hand-constructs
`Element::Clock { value: "[2024-01-15 Mon 10:00]--… =>  1:30" }` — **without** the prefix, i.e.
a value the parser can never produce. The test masks the bug and contributes to the 89%
coverage figure. This is a textbook example of coverage without fidelity.

### 3.4 obr end-to-end: what does not survive a flush → import cycle

Method: `obr init` in a throwaway dir, `obr create` 25 issues with adversarial descriptions,
`obr sync --flush-only`, snapshot `obr list --json` (= A); copy `issues.org` to a *fresh*
workspace, `obr sync --import-only`, snapshot again (= B); diff per field.

```
A ids: 25 B ids: 25   missing in B: []   extra in B: []
total field diffs: 12
```

| Payload | In DB (A) | After flush→import (B) | Loss class |
|---|---|---|---|
| `list` | `Intro line.\n- alpha\n- beta\n  - nested\n1. one\n2. two\nTail line.` | `Intro line.\n\n\nTail line.` | `PlainList` dropped |
| `table` | `Before.\n\| a \| b \|\n\|---+---\|\n\| 1 \| 2 \|\nAfter.` | `Before.\n\n\nAfter.` | `Table` dropped |
| `src` | `Before.\n#+begin_src python\nprint('hi')\n#+end_src\nAfter.` | `Before.\n\n\nAfter.` | `SrcBlock` dropped |
| `quote` | `Before.\n#+begin_quote\nQuoted text.\n#+end_quote\nAfter.` | `Before.\n\n\nAfter.` | `QuoteBlock` dropped |
| `props_drawer` | `Para.\n:PROPERTIES:\n:FOO: bar\n:END:\nAfter.` | `Para.\n\n\nAfter.` | `Drawer` dropped |
| `comment` | `before\n# a comment\nafter` | `before\n\n\nafter` | `Comment` dropped |
| `fixed` | `before\n: fixed width line\nafter` | `before\n\n\nafter` | `FixedWidth` dropped |
| `horiz` | `before\n-----\nafter` | `before\n\n\nafter` | `HorizontalRule` dropped |
| `footnote` | `text with fn[fn:1]\n\n[fn:1] the definition` | `text with fn` | `FootnoteReference` inline **and** `FootnoteDefinition` element both dropped |
| `crlf` | `line1\r\nline2\r\n` | `line1\nline2` | CR stripped, trailing newline lost |
| `blank_runs` | `para1\n\n\n\npara2` | `para1\n\n\npara2` | blank-run collapse |
| `latex` | `inline $x^2$ and \alpha here` | `inline $x^2$ and \alpha{} here` | **content growth** — `Entity` mangling |

Second batch (22 more payloads), field diffs after one cycle:

| Payload | In DB (A) | After (B) |
|---|---|---|
| `title_kw` `#+TITLE: hijack` | present | **`None` — entire description erased** |
| `seq_todo` `#+SEQ_TODO: X Y \| Z` | present | **`None`** |
| `clock` `CLOCK: [..]--[..] =>  1:00` | present | **`None`** |
| `properties_only` `:PROPERTIES:\n:X: 1\n:END:` | present | **`None`** |
| `example` `#+begin_example\nex\n#+end_example` | present | **`None`** |
| `verse` `#+begin_verse\nline1\nline2\n#+end_verse` | present | **`None`** |
| `macro` `{{{macro(arg)}}}` | present | **`None`** |
| `brackets` `[[link-ish]] and [fn::inline] and <<target>>` | — | `[[link-ish]] and  and ` |
| `cookie_stat` `progress [1/2] and [50%]` | — | `progress  and ` |
| `backslash` `back\slash and \\ double` | — | `back\slash{} and \\ double` (growth) |
| `heading_like` `* TODO …\n:PROPERTIES:\n:ID: bd-EVIL\n:END:\nbody` | — | `* TODO …\n\n\nbody` (heading escape works; drawer destroyed) |
| **`comma_star`** `,* already escaped line\nnext` | — | **`* already escaped line\nnext` — silent one-way corruption** |

The `,*` case is a genuine escaping bug in obr, not the parser.
`sanitize_org_text` (`src/sync/org_bridge.rs:248-259`) only prefixes a comma when a line
`starts_with('*')`, but `unsanitize_org_text` (`:569-580`) strips a comma from **any** line
starting with `,*`. A user's literal `,*` (a legitimate Org escape, or just a line beginning
with a comma and an asterisk) loses its comma on the first import.

**What survived** (worth recording, since it bounds the damage): `nested_heading`
(`* Level one star` — the `,*` escape works), `leading_star`, `[#A]` cookies in body text,
TODO-like words at line start in body text, tabs, emoji (incl. ZWJ sequences and combining
marks), NUL-adjacent control bytes `\x01\x02\x03`, `\x07\x0b\x0c\x1b[31m` ANSI, non-breaking /
ideographic / zero-width spaces, `:colons:` and a bare `:END:` inside body text, `[[url][desc]]`
links, `*bold* /italic/ _under_ +strike+ ~code~ =verb=`, `H_2O`/`a_{bc}` subscripts,
`<2026-01-01 Wed>` timestamps, unterminated `#+begin_src`, unterminated drawers, trailing
whitespace, 5,000-char lines, 500-line descriptions.

### 3.5 Tags and titles

| Case | Written to `issues.org` | Read back |
|---|---|---|
| label `a:b` + label `plain` | `* TODO [#C] colon label test    :a:b:plain:` | labels `['a','b','plain']` — **one label became two** |
| title `title ending in colon tag    :fake:` | `* TODO [#C] title ending in colon tag    :fake:` | title `'title ending in colon tag    '`, labels `['fake']` — **title truncated, phantom label injected** |
| title `title with :tags: in it` | verbatim | round-trips (Org only reads trailing tags) |
| title with `\n` | `sanitize_property_value` (`:60`, `:241-245`) → space | round-trips |

Labels are emitted **without** sanitization (`src/sync/org_bridge.rs:63-74` pushes `label`
raw), unlike every property value. A label containing `:` is therefore a split; a label
containing a space is rejected at creation time by validation (verified: `obr create -l
'has space'` → `{"error": …}`), so the space case is closed but the colon case is not.

### 3.6 Is it a fixpoint? No — it takes **two** cycles

Generation chain, each generation = fresh workspace + `sync --import-only` + `sync --flush-only`:

```
gen1 → gen2 : 122 diff lines (all 12 loss classes above materialise)
gen2 → gen3 : IDENTICAL
gen3 → gen4 : IDENTICAL
```

So for this corpus, convergence happens at gen2 — i.e. the file is **not** a fixpoint after one
cycle, which is the operationally relevant statement: a clone that imports a healthy
`issues.org` and flushes it back produces a different file.

At scale the two-cycle behaviour is measurable. Driver
`scratchpad/work/obr/src/bin/rtfuzz.rs` (added to the *copy* only) calls obr's own
`issues_to_org_text` / `org_text_to_issues` on 30,000 random descriptions drawn from an
Org-hostile alphabet:

```
iters done: nonfixpoint(g1!=g2)=318  gen3_moves=0  gen4_moves=0  grew_by_gen4=0
            lossy_first_pass=2807  parse_errors=0
```

- **9.4 % (2,807/30,000)** of descriptions lose or change content on the **first** pass.
- **1.06 % (318/30,000)** change *again* on the **second** pass — the file is not stable until
  generation 3.
- Nothing moves at gen 3 or gen 4, and nothing grows without bound. So obr's pipeline is
  eventually-convergent, unlike `json_to_org` (§3.2).

Typical gen1→gen2 mover (verbatim from the run):
```
  in ="|c😀abc<0 \"é}\na8\téé%!\n*_\n5/ 4 ,*1=59:0<333=9^\"8😀O5"
  d1 =Some("\n\na8\téé%!\n*_\n5/ 4 ,*1=59:0<333=9^\"8😀O5")
  d2 =Some("a8\téé%!\n*_\n5/ 4 ,*1=59:0<333=9^\"8😀O5")
```
A leading `|`-line becomes a `Table` element (dropped) leaving a `"\n\n"` artifact at gen1;
that leading blank is itself eaten at gen2.

### 3.7 The phantom change, demonstrated end-to-end

The mechanism that makes this a *git* problem, not just a data problem — verbatim transcript:

```
$ obr init && cp healthy_issues.org .beads/issues.org
$ md5 -q .beads/issues.org               # dd9828db…
$ obr list                               # plain list, no sync flags
○ bd-wkf [● P2] [task] - payload-trailing_ws …
$ md5 -q .beads/issues.org               # dd9828db…  UNCHANGED (import is DB-only)
$ obr ready ; md5 -q .beads/issues.org   # dd9828db…  UNCHANGED
$ obr create "unrelated new issue" -t task -p 2 --silent
bd-1w8
2026-08-06T19:04:39Z INFO beads_rust::sync: Auto-flush complete exported=26
$ md5 -q .beads/issues.org               # 720e6b54…  CHANGED
$ diff healthy_issues.org .beads/issues.org
… 12 unrelated issues' descriptions destroyed, plus the one intended addition …
```

Creating **one** issue in a fresh clone silently rewrites and destroys the descriptions of
**twelve** unrelated issues in the git-tracked file. The auto-import that caused the damage
(`obr list`) leaves no trace on disk, so the destruction appears in the commit of whoever next
mutates anything. This is R23's propagation mechanism, reproduced.

---

## 4. Determinism of emission

`issues_to_org_text` (`src/sync/org_bridge.rs:28-41`) is deterministic for a fixed DB state.

Test 1 — 25-issue DB, 8 consecutive exports, then 3 more in separate processes:
```
dd9828dbc780d154f53912fb6da8247f   x11 (identical every time)
```

Test 2 — 4 issues, 10 labels on one, 3 dependencies, 5 comments; 10 exports:
```
$ for i in $(seq 1 10); do obr sync --flush-only; md5 -q .beads/issues.org; done | sort -u
498eda625f806e1d7d5689d48b4aaa6c        # single line ⇒ byte-identical
```

Ordering audit of the produced file:
- **Issues**: lexicographic by ID (`bd-15n`, `bd-39q`, `bd-3cm`, `bd-3oz`). The export path
  collects into a `Vec` (`src/sync/mod.rs:1454-1476`) from an already-sorted query.
- **Labels**: explicitly sorted in `emit_issue_heading` — `src/sync/org_bridge.rs:65-66`
  (`let mut sorted_labels = issue.labels.clone(); sorted_labels.sort();`). ASCII order, so
  `Mid` precedes `alpha`; stable but case-sensitive. Import re-sorts at
  `src/sync/org_bridge.rs:378-379`.
- **Dependencies**: emitted as `serde_json::to_string_pretty(&issue.dependencies)`
  (`src/sync/org_bridge.rs:216-217`); observed sorted by `depends_on_id`.
- **Comments**: `serde_json::to_string_pretty(&issue.comments)` (`:228-229`); observed sorted
  by integer `id`.
- **No `HashMap`/`HashSet` iteration reaches the output.** Properties are emitted by an explicit
  hand-written sequence of `push_str` calls (`org_bridge.rs:79-168`), not by iterating a map.
- **Timestamps**: `DateTime<Utc>::to_rfc3339()` throughout (`:83`, `:84`, `:100`, `:109`,
  `:113`, `:124`, `:143`) — fixed format, no locale/TZ dependence.

**Verdict: determinism holds.** The git-tracked file will not churn from emission
nondeterminism. It churns from §3.6/§3.7 content degradation instead.

One caveat: `sanitize_org_text` (`:248-259`) uses `str::lines()` and rejoins with `"\n"`,
which **normalizes CRLF to LF and drops a trailing newline** on every write. That is stable
(idempotent) but lossy on the first write, and it is why the `crlf` payload in §3.4 changed.

---

## 5. Panic and robustness

### 5.1 What `fuzz_org_parse.rs` actually asserts

`/Users/johnw/src/obr/fuzz/fuzz_targets/fuzz_org_parse.rs` in full:
```rust
#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &str| {
    // Fuzz the Org-mode text to issues parser.
    // This is the primary input boundary for Org-mode import.
    let _ = beads_rust::sync::org_bridge::org_text_to_issues(data);
});
```
It asserts **nothing**. The `Result` is discarded with `let _ =`. Its only oracle is the
libFuzzer default: *don't crash, don't OOM, don't time out*. It cannot detect content loss,
non-idempotency, or mis-parse. Note also `|data: &str|` — libFuzzer's `Arbitrary for &str`
only yields valid UTF-8, so the invalid-UTF-8 path (which obr hits via
`fs::read_to_string` at `src/sync/mod.rs:2230`) is never fuzzed here.

The task's framing is correct that the *real* target is `org2jsonl`: `org_text_to_issues`
is 20 lines of dispatch (`org_bridge.rs:335-355`) over 2,274 lines of parser plus
`orgize 0.10.0-alpha.10`. A panic anywhere in that stack is a panic in obr.

There is no evidence the target was ever run: `/Users/johnw/src/obr/fuzz/` contains no
`corpus/` or `artifacts/`, and `.github/workflows/*.yml` never invokes `cargo fuzz`.

### 5.2 Hostile-input sweep through obr's auto-import path

32 adversarial `.org` files placed at `.beads/issues.org`, then plain `obr list`
(which triggers auto-import), 30 s timeout, `ulimit -v 4000000`:

```
file                        rc     time    first line of output
bad_date.org               RC=4    0.1s   {   (VALIDATION_FAILED)
bom.org                    RC=0    0.0s
comments_bad.org           RC=8    0.0s   {
conflict.org               RC=7    0.0s   {   (conflict-marker guard fired)
crlf.org                   RC=0    0.0s   ○ bd-1 …
deep_nest.org              RC=4    0.0s   {   (200 nested headings; level-1 has no :ID:)
dup_end.org                RC=0    0.0s   ○ bd-1 …
dup_ids.org                RC=0    0.5s   ○ bd-same …    (100 headings sharing one ID: accepted)
empty.org                  RC=0    0.0s
footnote_cycle.org         RC=0    0.0s   ○ bd-1 …
huge_prop.org              RC=0    0.1s   ○ bd-1 …       (2 MB single property value)
invalid_utf8.bin           RC=8    0.0s   {
json_bomb.org              RC=8    0.0s   {   (100 000 '[' in a json src block)
just_star.org              RC=0    0.0s
keyword_only.org           RC=7    0.0s   {
lone_cr.org                RC=0    0.0s   ○ bd-1 …
long_line.org              RC=7    0.1s   {   (10 MB single line)
many_headings.org          RC=0    9.6s   ○ bd-19999 …   (20 000 issues)
many_lists.org             RC=0    0.3s   ○ bd-1 …
many_tags.org              RC=0    3.0s   ○ bd-1 …       (50 000 tags on one heading)
nested_braces.org          RC=0    0.9s   ○ bd-1 …
nested_brackets.org        RC=0   12.7s   ○ bd-1 …       <-- see §5.3
nested_emph.org            RC=0    0.1s   ○ bd-1 …
no_id.org                  RC=4    0.0s   {
null_bytes.org             RC=0    0.0s   ○ bd-1 …       (NUL survives into the title!)
star_nl.org                RC=0    0.0s
surrogate_ish.org          RC=8    0.0s   {
table_ragged.org           RC=0    0.0s   ○ bd-1 …
unterminated_drawer.org    RC=4    0.0s   {
unterminated_src.org       RC=0    0.0s   ○ bd-1 …
very_deep.org              RC=0    0.0s                  (100 000 leading stars)
weird_prio.org             RC=0    0.0s   ○ bd-1 … [#ZZZZ] h
```

**No panic, no SIGABRT (no exit 134), no hang, no OOM.** Every failure produced obr's JSON
error envelope with a clean exit code. Invalid UTF-8 and lone surrogates are rejected at
`fs::read_to_string` before the parser ever sees them (rc 8), which is why the
`panic = "abort"` hazard is contained on that axis.

Two behavioural notes:
- `null_bytes.org` — a literal NUL inside a heading title is accepted and stored
  (`title = "h\0nul"`), then re-emitted. SQLite tolerates it; downstream consumers may not.
- `dup_ids.org` — 100 level-1 headings sharing `:ID: bd-same` import without complaint;
  last one wins.

### 5.3 Quadratic parse blowup — the real DoS

`nested_brackets.org` is `"[[" * n` in a description. Timing, release `obr list`
(i.e. the auto-import path a user hits by typing any read command):

| input size | wall time |
|---|---|
| 25 KB | 0.83 s |
| 50 KB | 3.22 s |
| 100 KB | 12.73 s |
| 200 KB | 50.96 s (then rc=7 — obr's own guard tripped) |

Debug `org2jsonl` alone shows the same shape (0.30 / 0.75 / 2.87 / 11.35 / 45.36 s for
4/8/16/32/64 KB): **doubling the input quadruples the time — clean O(n²)**, inside
`orgize`'s link scanning. Extrapolating the release numbers, a 1 MB `issues.org` of this shape
costs ~20 minutes *per obr invocation*. `.beads/issues.org` is git-tracked and
human/agent-editable, and every read command auto-imports it, so this is reachable by
committing a large paste containing unbalanced `[[`.

### 5.4 One bad heading bricks the workspace

`parse_properties` (`src/sync/org_bridge.rs:742-748`) returns `Err` when `:ID:` is absent, and
`org_text_to_issues` propagates with `?` (`:347`). There is no per-heading recovery.

```
$ cp healthy_issues.org .beads/issues.org
$ printf '* TODO [#C] orphan heading with no ID\n:PROPERTIES:\n:ISSUE_TYPE: task\n:END:\n\n' >> .beads/issues.org
$ obr list
{ "error": { "code": "VALIDATION_FAILED",
             "message": "Validation failed: id: Missing required :ID: property", … } }
$ obr show bd-ll2          # a perfectly healthy, unrelated issue
{ "error": { "code": "VALIDATION_FAILED", … } }
```

Every command in the workspace fails, including `show` of an unrelated healthy issue. The error
names **no line number, no heading title, no file offset** — and as established in §2.2 it
cannot, because `org_to_entries_with_keywords` discards location metadata. For a format whose
entire selling point is that humans edit it in Emacs, adding a `* Notes to self` heading is a
workspace-wide outage with an unactionable error.

### 5.5 `#+SEQ_TODO:` in the file is ignored

obr writes `#+SEQ_TODO: TODO DOING WAIT DEFER NOTE | DONE CANCELED`
(`src/sync/org_bridge.rs:33`) and then ignores it on read: the Section entry is skipped
(`:351`) and the parse always uses the hardcoded `BEADS_TODO_KEYWORDS` / `BEADS_DONE_KEYWORDS`
(`:312-324`, `:336-340`). A hand-edited or foreign heading with any other keyword is absorbed
into the title:

```
input :  * REVIEWING [#B] hand written        (with #+SEQ_TODO: TODO REVIEWING | DONE)
parsed:  title  = "REVIEWING [#B] hand written"
         status = "open"       (priority [#B] swallowed too → default 2)
```

Related latent bug: `status_to_keyword` emits `Status::Custom(s) => s.to_uppercase()`
(`org_bridge.rs:271`), which is by construction *not* in `BEADS_TODO_KEYWORDS`. Any
`Status::Custom` that reaches the exporter round-trips into the title and silently becomes
`Open`. The CLI currently blocks custom statuses (`obr update -s reviewing` →
`{"error":{"code":"INVALID_STATUS"…}}`), so this is reachable only via import from another
beads implementation or hand-editing — both supported workflows.

### 5.6 100 k-iteration mutation fuzz of the parser: zero panics

Driver `scratchpad/drv/src/main.rs` — xorshift-seeded, 1–24 byte-level mutations per iteration
over two Org-feature-rich seeds (obr-shaped headings, property drawers, tables, src blocks,
drawers, clock lines, footnotes, entities, LaTeX, targets, macros, cookies), alphabet
`*:[]{}|+-#=~/_^\<>,.\n\t %$@!()"'0123456789abcTODNE `, each iteration doing
parse → write → parse → write inside `catch_unwind`:

```
done, panics=0
```

Combined with §5.2, my finding is: **no panic reachable through obr's Org import in this
sample**. The `panic = "abort"` risk (`/Users/johnw/src/obr/Cargo.toml:94`) is real in kind —
a parser panic *would* become exit 134 with a Rust backtrace instead of obr's JSON envelope,
bypassing the entire 3-layer error system — but unrealized in practice. The **O(n²)** blowup
(§5.3) and the **all-or-nothing import** (§5.4) are the robustness defects that actually bite.

---

## 6. Supply chain / reproducibility verdict

### 6.1 The `path` dependency

`/Users/johnw/src/obr/Cargo.toml:63-64`:
```toml
# Org-mode integration
org2jsonl = { path = "../org2jsonl" }
```
A `path` dependency with no `version` key means:
- **No version requirement is recorded anywhere.** `Cargo.lock:2673-2683` records
  `name = "org2jsonl", version = "0.1.0"` with **no `source`, no `checksum`** — it is a
  local-path package, so Cargo has nothing to verify.
- **`cargo build` fails for anyone who is not the author**, in any checkout that does not have
  a sibling `../org2jsonl` directory. CI does exactly that: `.github/workflows/ci.yml` runs
  `cargo fmt`, `cargo clippy --all-targets --all-features -- --deny warnings`,
  `cargo check --all-targets --all-features` after a bare `actions/checkout`. Nothing in
  `.github/` mentions `org2jsonl` (`grep -rn 'org2jsonl' .github/` → no hits). Every job that
  compiles is unbuildable.
- **`cargo audit` / `cargo deny` / SBOM tooling cannot see it.** Path dependencies have no
  registry identity, so `.github/workflows/audit.yml` cannot report on org2jsonl or on its
  transitive `orgize 0.10.0-alpha.10`.
- **`cargo publish` is impossible** while the path dep exists.
- Anyone auditing obr's parse of the durable format has to be told the crate exists and where
  it lives; it is not in the dependency graph any tool would surface.

### 6.2 obr's `Cargo.lock` is already stale against the worktree

`Cargo.lock:2673-2683` records org2jsonl's dependencies as:
```
name = "org2jsonl"
version = "0.1.0"
dependencies = [ "anyhow", "clap", "orgize", "serde", "serde_json" ]
```
The org2jsonl worktree declares `walkdir = "2"` (`org2jsonl/Cargo.toml:15`, added in commit
`b8fc44d`). Therefore:
```
$ cargo build --offline --locked        # in /Users/johnw/src/obr
error: cannot update the lock file /Users/johnw/src/obr/Cargo.lock
       because --locked was passed to prevent this
```
Building without `--locked` mutates the committed lockfile (verified in the copy: the
`org2jsonl` block gains `"walkdir"`). So the checked-in lockfile does not describe the crate
that would actually compile, and any reproducible-build flag rejects the tree.

### 6.3 The flake pins something — but not the thing on disk

`/Users/johnw/src/obr/flake.nix:21-23`:
```nix
org2jsonl = {
  url = "git+file:///Users/johnw/src/org2jsonl";
  inputs.nixpkgs.follows = "nixpkgs";
};
```
`flake.lock` node `org2jsonl`:
```json
{"lastModified":1771539100,
 "narHash":"sha256-SWSuqmfqSHc7T1pPn/D0O5FOw5OlGQB7UM0tVniFtAA=",
 "ref":"refs/heads/main",
 "rev":"5ea75860bdcca8ffe3989ae877b63328e34c6270",
 "revCount":15,
 "type":"git",
 "url":"file:///Users/johnw/src/org2jsonl"}
```

Two independent problems:

1. **The URL is a local absolute path on one machine.** `git+file:///Users/johnw/src/org2jsonl`
   resolves for exactly one user on exactly one host. `nix build` / `nix develop` /
   `nix flake check` fail for everyone else, regardless of the rev being pinned. The `narHash`
   makes the *content* verifiable but the *location* unobtainable.
2. **The pinned rev is 15 commits behind the worktree.** `rev 5ea7586` = `revCount 15`; the
   worktree HEAD is `59521f9`, commit 30 of 30. So:

| build method | org2jsonl actually compiled | walkdir? | Cargo.lock consistent? |
|---|---|---|---|
| `nix build` (flake) | `5ea7586` (Mar 16) | no | **yes** |
| `cargo build` (path dep) | `59521f9` (Mar 18) | yes | **no** — lock is rewritten |

Both are labelled `org2jsonl 0.1.0`. Nothing anywhere detects that they are different parsers.
The flake "works" only because it happens to pin a rev predating the `walkdir` addition —
a coincidence, not a mechanism. Whichever `obr` binary a user has, they cannot tell which
parser it contains.

### 6.4 Zero version compatibility between an `issues.org` and the parser reading it

- `issues_to_org_text` emits `:BEADS_SCHEMA_VERSION: 1` for every issue
  (`src/sync/org_bridge.rs:80`).
- `parse_properties` **explicitly discards it** (`src/sync/org_bridge.rs:735-738`):
  ```rust
  "BEADS_SCHEMA_VERSION" | _ => {
      // BEADS_SCHEMA_VERSION is validated elsewhere if needed
      // Unknown properties are silently ignored
  }
  ```
  Grepping obr for a reader of that key finds none. The comment's "validated elsewhere" is not
  true.
- `org2jsonl::SCHEMA_VERSION` (`org2jsonl/src/lib.rs:5`) is stamped into every `OrgEntry`
  (`org_to_json.rs:122`, `:146`) but obr never reads `entry.schema_version` either.
- There is no parser-version marker in the file, no `#+` header recording the writer, and no
  content hash over the Org text that a reader could check. (`content_hash` exists on `Issue`
  and is used for JSONL dedup — `src/sync/mod.rs:1470-1476` — but it is computed *from* the
  parsed issue, so a lossy parse simply produces a different, self-consistent hash. It detects
  nothing.)

**Verdict: no compatibility is enforced at any layer.** An `issues.org` written by a build with
one parser and read by a build with another will silently produce different issues, and the
degradation will be committed as a legitimate-looking diff (§3.7).

### 6.5 Minimum change to make the durable format's parser reproducible

Ranked by effort, all of them strictly better than the status quo:

1. **Git-pin with a rev (smallest real fix).** Push `org2jsonl` to a reachable remote
   (`github.com/jwiegley/org2jsonl` already exists as the crate's `origin`) and change
   `Cargo.toml:64` to
   `org2jsonl = { git = "https://github.com/jwiegley/org2jsonl", rev = "<40-hex>" }`,
   and `flake.nix:22` to the same `github:`/`https://` URL + `rev`. This is precisely the
   pattern obr already uses for its other sibling project — `Cargo.toml:54`:
   `toon_rust = { git = "https://github.com/Dicklesworthstone/toon_rust.git", rev = "788589d" }`,
   with the matching `outputHashes` entry at `flake.nix:65-67`. Doing the same for org2jsonl
   costs one line each and immediately fixes CI, `cargo audit` visibility, third-party builds,
   and the flake/cargo divergence.
   *This is the recommended minimum.*
2. **Vendor.** `cargo vendor` or copy `src/{lib,model,org_to_json}.rs` into
   `obr/vendor/org2jsonl/` as a workspace member. Removes the network and the sibling-checkout
   requirement entirely; obr only needs 2,586 of the 4,987 lines (§2.1), so a vendored subset
   would drop `json_to_org.rs` and both binaries.
3. **Publish to crates.io** with a real semver requirement (`org2jsonl = "0.1"`). Best for
   auditability, but it locks a `0.1.0` API that is still churning (six fidelity-fix commits
   in one month) and pins a public dependency on `orgize 0.10.0-alpha.10`.
4. **Inline.** obr only needs `orgize` CST → 5 model types. Given that obr consumes 1 of 23
   `Element` variants and 14 of 22 `InlineContent` variants (§2.3–2.4), a purpose-built
   ~300-line `orgize`→`Issue` reader inside `src/sync/` would cover obr's needs, delete the
   dependency, and make the loss surface explicit instead of accidental. This is the correct
   long-term answer if obr's Org subset is not going to grow.

Orthogonal but required regardless of which option is chosen:
- Regenerate and commit `Cargo.lock` so `cargo build --locked` passes (§6.2).
- Add a CI job that actually builds — today none can.
- Record the parser identity in the file (e.g. a `#+BEADS_PARSER:` keyword or an
  `:ORG2JSONL_REV:` property) so a mismatched read is detectable rather than silent (§6.4).

---

## 7. Cross-cutting: what this means for R23

Prior research attributed the round-trip loss "entirely to obr's `extract_body_text`
(`org_bridge.rs:540-554`)". That attribution is **essentially correct but incomplete**:

- **Correct**: the parser preserves lists, tables, src blocks, quote blocks, drawers, keywords,
  comments, fixed-width lines and footnote definitions faithfully (§2.5, §3.1). obr discards
  them. 21 of 23 `Element` variants and 7 of 22 `InlineContent` variants die in obr's code.
- **Incomplete in three ways**:
  1. `extract_body_text`'s `i > 0` separator bug (§2.3) is a *second*, independent defect in
     the same 15-line function — it is what turns silent deletion into the visible
     `Before.\n\n\nAfter.` artifact, and it is what makes the pipeline take two cycles to
     converge (§3.6).
  2. `flatten_inline_to_text`'s `Entity` arm (`org_bridge.rs:519-523`) **adds** `{}` that was
     not in the source. That is content *growth*, in the opposite direction from all the other
     losses, and it lives outside `extract_body_text`.
  3. `unsanitize_org_text` (`:569-580`) is not the inverse of `sanitize_org_text` (`:248-259`),
     producing one-way `,*` → `*` corruption (§3.4) — also outside `extract_body_text`.
- **And the parser's own fidelity is not clean**, contrary to the implicit assumption: 5 of its
  11 fixtures fail its own oracle, it has a `CLOCK:` duplication bug, and it has an
  unbounded-growth non-idempotency its own unrun fuzz target would catch (§3.1–3.3). None of
  these reach obr today — only because obr never calls the writer — but they are exactly the
  code that would run if anyone used `jsonl2org` on `.beads/issues.org`, which is the advertised
  workflow for the format.

The deepest structural point: obr has **zero tests** exercising `org_text_to_issues`.
`grep -rn 'org_text_to_issues\|issues_to_org_text' tests/` returns nothing, and the seven
`#[test]`s inside `src/sync/org_bridge.rs:753-900` test `sanitize_org_text`,
`sanitize_property_value`, `unsanitize_org_text`, status/priority keyword mapping,
`issues_to_org_text` string-contains assertions, and `flatten_inline_to_text` on
hand-constructed values. **Not one of them parses an Org file.** The 15-line function
responsible for the entire loss surface of the durable artifact has no test at all, in a
codebase with 781 unit tests.

---

## Appendix A — artifacts produced

All under `/private/tmp/claude-501/-Users-johnw-src-obr/f80d1967-0fc0-44fa-a53f-3054d39727e2/scratchpad/`:

| Path | What |
|---|---|
| `work/obr`, `work/org2jsonl` | rsync copies (no `.git`, no `target`) used for all builds |
| `work/obr/src/bin/rtfuzz.rs` | obr round-trip fixpoint driver (added to the **copy** only) |
| `drv/src/main.rs` | 100 k-iteration org2jsonl mutation fuzzer |
| `org_gen1..4.org`, `c2_gen1.org` | generation chain from obr flush/import cycles |
| `hostile/*.org` | 32-file adversarial corpus |
| `e2e/`, `e2e2/`, `e2e3/`, `e2e4/`, `clone/`, `c2/`, `c2b/`, `lab/`, `lab2/`, `inj/`, `inj2/`, `det/`, `cust*/`, `hz/`, `hz_mixed/`, `scaleobr/` | throwaway obr workspaces |

Nothing was written to `/Users/johnw/src/obr` or `/Users/johnw/src/org2jsonl`; both remain
`git status` clean apart from the pre-existing untracked `docs/superpowers/` in obr.

## Appendix B — commands that reproduce the headline findings

```bash
# 1. flake pin is 15 commits behind the worktree
python3 -c "import json;d=json.load(open('/Users/johnw/src/obr/flake.lock'));print(d['nodes']['org2jsonl']['locked'])"
git -C /Users/johnw/src/org2jsonl log --oneline | wc -l          # 30
git -C /Users/johnw/src/org2jsonl log --oneline | sed -n 16p     # 5ea7586

# 2. committed Cargo.lock is stale
cd /Users/johnw/src/obr && cargo build --offline --locked        # errors

# 3. unbounded growth in org2jsonl's round-trip (6-byte input)
printf -- '-\n* h\n' > /tmp/g.org
org2jsonl /tmp/g.org | jsonl2org | org2jsonl | jsonl2org | cat -A

# 4. CLOCK duplication
printf '* H\nCLOCK: [2026-01-01 Wed 10:00]--[2026-01-01 Wed 11:00] =>  1:00\n' \
  | org2jsonl | jsonl2org

# 5. obr content destruction + phantom change
cd $(mktemp -d) && obr init && obr create x -d 'a
- l1
- l2
b' --silent && obr sync --flush-only && cp .beads/issues.org /tmp/g1.org
cd $(mktemp -d) && obr init && cp /tmp/g1.org .beads/issues.org \
  && obr create "unrelated" --silent && diff /tmp/g1.org .beads/issues.org

# 6. one bad heading bricks the workspace
printf '* TODO [#C] no id\n:PROPERTIES:\n:ISSUE_TYPE: task\n:END:\n' >> .beads/issues.org
obr list        # VALIDATION_FAILED, no line number, every command dead

# 7. quadratic parse
python3 -c "print('* TODO [#C] h\n:PROPERTIES:\n:ID: bd-1\n:END:\n\n' + '[['*50000)" > .beads/issues.org
time obr list   # ~13 s for 100 KB; 4x per doubling
```
