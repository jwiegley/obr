# Web research: sync / merge design for structured data through git

Agent: `sync-merge` (web research). Target project: `/Users/johnw/src/obr` (crate `beads_rust`, binary `br`)
— a Rust port of Steve Yegge's Go `beads`/`bd`: SQLite for speed + JSONL-in-git for sync, content-addressed
SHA-256 dedup, 4-step mutation protocol, never runs git itself.

Every nontrivial claim below carries a source URL. Claims labelled **[EXPERIMENT]** were reproduced locally
by me with `git version 2.55.0` in a throwaway repo under the scratchpad
(`/private/tmp/claude-501/-Users-johnw-src-obr/f80d1967-0fc0-44fa-a53f-3054d39727e2/scratchpad/uniontest*`);
exact repro commands are inlined. The `obr` repo itself was only read, never modified.

---

## 0. Grounding: what `obr` actually does today (read-only inspection)

Just enough context so the recommendations land. Other agents own the deep repo analysis.

- No `.gitattributes` exists in the repo root (`/Users/johnw/src/obr`), so **no merge driver and no
  `merge=union` is in effect today**; `.beads/issues.jsonl` merges with git's default 3-way text driver.
- `/Users/johnw/src/obr/.beads/issues.jsonl` is one JSON object per line, **sorted by `id`**
  (`beads_rust-07b`, `beads_rust-0a5`, `beads_rust-0ol`, …) and **ends with `\n`**. Both properties matter a
  great deal — see §3.
- `/Users/johnw/src/obr/src/sync/mod.rs:2120-2121` resolves import conflicts with wall-clock
  **last-write-wins**: `// Last-write-wins: compare updated_at` / `match incoming.updated_at.cmp(&existing.updated_at)`.
- Tombstones exist with a retention window: `/Users/johnw/src/obr/src/sync/mod.rs:44` ("Retention period for
  tombstones in days (None = keep forever)"), `:1455-1457` skips expired tombstones on export,
  `:2098-2099` gives tombstones protection that "even force doesn't override".
- A 3-way merge exists with base = `last_sync_hash`, left = SQLite, right = JSONL — described in
  `/Users/johnw/src/obr/.beads/issues.jsonl` issue `beads_rust-07b` ("3-Way Merge Algorithm Implementation").
- Conflict-marker defence exists: `/Users/johnw/src/obr/src/sync/mod.rs:1053` `scan_conflict_markers`,
  `:1091` `ensure_no_conflict_markers`, called at `:2221` before import.
- Upstream Go beads shipped a **custom** merge driver, not union:
  `/Users/johnw/src/obr/EXISTING_BEADS_STRUCTURE_AND_ARCHITECTURE.md:5825` — "`.gitattributes` gets:
  `.beads/issues.jsonl merge=beads`", with install/uninstall wiring in `bd init` (`:99`, `:6003`, `:7562`).
  Grepping `src/**/*.rs` for `merge_driver|merge=beads|merge=br` returns **nothing** — the Rust port does not
  currently install a merge driver.

So `obr` is squarely in the design space this note covers: line-oriented JSONL in git, LWW on wall clocks,
tombstones with TTL, SQLite as the fast index.

---

## 1. Git's `merge=union`: exact semantics, and where it silently eats data

### 1.1 What the manual actually says

From the official `gitattributes(5)` page, <https://git-scm.com/docs/gitattributes>:

> The attribute `merge` affects how three versions of a file are merged when a file-level merge is necessary
> during `git merge`, and other commands such as `git revert` and `git cherry-pick`.

> **union** — Run 3-way file level merge for text files, but take lines from both versions, instead of
> leaving conflict markers. This tends to leave the added lines in the resulting file in random order and the
> user should verify the result. **Do not use this if you do not understand the implications.**

Two details that matter for a tracker:

1. The driver is invoked for `merge`, `revert`, **and `cherry-pick`** — and, as I confirmed empirically, for
   `rebase` too (§1.3 Case R). So it is not a "merges only" opt-in; it silently governs rebase-heavy agent
   workflows.
2. Custom drivers are defined in `.git/config`, not `.gitattributes`
   (<https://git-scm.com/docs/gitattributes>): `driver = filfre %O %A %B %L %P`, where `%O`/`%A`/`%B` are the
   base/ours/theirs temp files, the driver **must overwrite `%A`**, and must "exit with zero status if it
   managed to merge them cleanly, or non-zero if there were conflicts". Exit status **>128** is treated as a
   *failure*, "which is different from producing a conflict". `merge.*.recursive` names the driver used for
   internal merges of multiple merge bases; if unspecified the driver is used for those too. `%P` is the real
   pathname; `%S`/`%X`/`%Y` are conflict labels.

`union` is *built in*, so unlike a custom driver it requires no `.git/config` entry — which is exactly why
projects reach for it, and exactly why it is dangerous by default.

### 1.2 Documented field reports of union going wrong

- Vladimir Kiselev, "On reducing 'Changelog' merge conflicts",
  <https://medium.com/@nettsundere/on-reducing-changelog-merge-conflicts-1eb23552630b> — union is
  "bad idea when your changelog has some not-so-unique endings" and "All similar stirngs [sic] can be
  silently merged into one during conflict resolution." This is the post cited in
  <https://github.com/gitattributes/gitattributes/issues/21> ("According to this blog, `merge=union` causes
  silent troubles"). *Caveat: I could not reproduce whole-line dedup when the union driver actually runs —
  see §1.3 Case G. The observed dedup case is narrower (identical resulting blobs, §1.3 Case D).*
- Generic guidance: "the union strategy can produce logically incorrect results if lines from both sides
  depend on each other, so it should only be used for files where the content is genuinely additive and
  order-independent" — <https://tenthirtyam.org/dispatches/2026/04/11/controlling-git-repository-behavior-with-gitattributes/>
- scikit-learn revisited and questioned their union setting:
  <https://github.com/scikit-learn/scikit-learn/issues/21516>
- keep-a-changelog discussion on union's tradeoffs:
  <https://github.com/olivierlacan/keep-a-changelog/discussions/478> and
  <https://github.com/olivierlacan/keep-a-changelog/issues/56>

### 1.3 **[EXPERIMENT]** Union merge behaviour on JSONL, measured

Setup for every case (git 2.55.0):

```sh
git init -q -b main .
printf 'x.jsonl merge=union\n' > .gitattributes
printf '{"id":"1"}\n{"id":"2"}\n{"id":"3"}\n' > x.jsonl
git add -A && git commit -qm base
# then branch A and branch B, edit, commit, and `git merge A` from B
```

| Case | Setup | Result | Verdict |
|---|---|---|---|
| **A** | A appends `{"id":"4"}`; B appends `{"id":"5"}` | rc=0, file ends `…,{"id":"5"},{"id":"4"}` — **ours first, then theirs** | works; ordering is not append-time order |
| **B** | A deletes record `2`; B appends record `4` (disjoint hunks) | rc=0, record 2 stays deleted, record 4 present | correct |
| **C** | A edits record `2` in place; B appends record `4` | rc=0, both changes kept | correct |
| **D** | Both sides append the **identical** line `{"id":"9","t":"nine"}` and nothing else | rc=0, line appears **once**; git printed no "Auto-merging" — the two blobs were identical so the merge resolved trivially without ever calling the union driver | dedup here is *blob identity*, not union |
| **G** | Both sides append the same `{"id":"9","t":"dup"}` **plus** a distinct line each | rc=0; `{"id":"9","t":"dup"}` appears **TWICE** | **union duplicates; it does not dedupe** |
| **E** | Both sides edit record `2` differently (`from-A` vs `from-B`) | rc=0, output contains **both** `{"id":"2","t":"from-B"}` and `{"id":"2","t":"from-A"}` | **duplicate primary keys, zero conflict signal** |
| **F** | A deletes record `3` (last line); B edits record `3` | rc=0, `{"id":"3","t":"THREE-EDITED"}` survives | **delete silently loses to edit (resurrection)** |
| **I** | A deletes record `2` (middle line); B edits record `2` | rc=0, `{"id":"2","t":"EDIT"}` survives | same resurrection, mid-file |
| **H** | Base file has **no trailing newline**; both sides append | rc=0, `{"id":"3"}` appears **TWICE** | **missing final `\n` turns an append into a last-line edit → duplicated record** |
| **J** | File sorted by id; A inserts `b`, B inserts `d` | rc=0, result is `a,b,c,d,e` — still sorted | **sorted JSONL merges beautifully** |
| **K** | A reorders the whole file (`3,2,1`); B appends `4` | rc=0, result `3,4,2,1` | no loss here but ordering is scrambled; a non-deterministic exporter is a landmine |
| **Adj** | A inserts `{"id":"1.5"}` after line 1; B inserts `{"id":"1.6"}` at the same spot | rc=0, result `1, 1.6, 1.5, 2, 3` | both survive, order arbitrary — the git analogue of the CRDT *interleaving anomaly* (§5.3) |
| **R** | Same as A but via `git rebase main` | rc=0, both lines present | **union applies during rebase** |
| **CP** | Same as A but via `git cherry-pick` | rc=0, both lines present | **union applies during cherry-pick** |
| **NUL** | Any line contains a raw `\0`; both sides append | **rc=1**, `git status` shows `UU`, worktree holds **only "ours"** — theirs is gone, and there are **no conflict markers** to reveal it | **worst failure mode**: union degrades to the `binary` driver ("keep the version from your branch in the work tree, but leave the path in the conflicted state" — <https://git-scm.com/docs/gitattributes>). Adding an explicit `text` attribute (`x.jsonl text merge=union`) **does not** fix it — I retested and got the same `UU` + ours-only result |

**Distilled failure taxonomy for union + JSONL:**

1. **Duplicate keys from concurrent edits** (Case E). Union's "correct" output for a keyed record file is a
   file containing the same `id` twice with different bodies. The merge exits 0. Any downstream importer
   *must* be prepared for duplicate ids and must have a deterministic tiebreak, or the outcome depends on
   iteration order.
2. **Deletes lose to concurrent edits** (Cases F, I) — an implicit, undocumented *add-wins/update-wins*
   policy chosen by git's line algebra rather than by you.
3. **Trailing-newline sensitivity** (Case H). `jsonlines.org` says the trailing newline is "strongly
   recommended but not required" (<https://jsonlines.org/>). For union merge it is effectively **required**:
   without it, every append rewrites the final line, and union duplicates that record.
4. **Binary-heuristic cliff** (Case NUL). One stray control byte and union stops applying, keeping only your
   side, with no markers. This is a plausible mechanism for the kind of mass-loss incidents Yegge reported
   ("We lost bd-100 through bd-179 (80 issues!)" during a rebase — reported in
   <https://ianbull.com/posts/beads/> and Yegge's Medium posts, e.g.
   <https://steve-yegge.medium.com/beads-best-practices-2db636b9760c>).
5. **Non-determinism of record order** (Cases A, K, Adj). Union output order is "random" by git's own
   admission. Anything that depends on file order is broken by construction.

### 1.4 Forges do not honour it

This is the single biggest operational gotcha and it is well documented:

- GitHub support, in <https://github.com/orgs/community/discussions/9288>: "GitHub doesn't consider
  user-defined .gitattributes files (normally, we use our own .gitattributes file which you can't change)."
  Their suggested workaround is "merge pull requests in your local clone (and not via the web UI)".
- Kubernetes **removed** their union driver precisely for this reason:
  <https://github.com/kubernetes/kubernetes/pull/70576> — GitHub doesn't support the union merge driver, and
  Prow merges via the GitHub API rather than `git merge`; GitHub also refuses to merge PRs that have
  conflicts, so union "doesn't make much sense in that context".
- GitLab: <https://gitlab.com/gitlab-org/gitlab/-/issues/18830> — ".gitattributes … custom merge drivers are
  not used during merge requests (since they are defined in the gitconfig)"; also
  <https://gitlab.com/gitlab-org/gitlab-foss/-/issues/17325>.
- Bitbucket: <https://community.atlassian.com/forums/Bitbucket-questions/Merge-strategy-in-gitattributes-doesn-t-work-on-Bitbucket/qaq-p/1773507>
  — works locally, not in the web PR flow.
- Older GitHub asks: <https://github.com/isaacs/github/issues/487>, <https://github.com/isaacs/github/issues/560>.

**Consequence:** a merge strategy expressed in `.gitattributes` is a *client-side convention only*. The
authoritative resolution must live in the tool (`br import`/`br sync`), not in the merge driver, because half
your merges will happen on a server that ignores the driver entirely.

---

## 2. Prior art: how other systems store an issue tracker in a VCS

### 2.1 Fossil — the strongest architectural precedent, and it says "don't put tickets in the tree"

<https://fossil-scm.org/home/doc/tip/www/bugtheory.wiki>

Fossil explicitly rejects storing tickets as files in the source tree, for three reasons: check-ins in fossil
are immutable so a check-in cannot gain new tickets afterwards; "thousands and thousands" of ticket files
would clutter the tree; and ticket permissions must be distinct from check-in permissions so that
"Casual passers-by on the internet" can file tickets.

Instead, tickets are **ticket change artifacts**: each artifact carries "a timestamp", "a ticket ID", and
"one or more name/value pairs". Current state is computed by replaying "all ticket change artifacts with the
same ticket ID in timestamp order", and "As each ticket change artifact is encountered, values are either
replaced or appended, according to a flag on the name/value pair". Only changed fields appear in an artifact.
Ticket IDs are randomly generated 40-hex-char values. Critically: **"Tickets do not branch."**

Fossil also names the wall-clock hazard directly: a clock off "by a few minutes or even a few hours" is
fine, but drift of "months or years" seriously disrupts the replay algorithm's ability to determine state.

Sync-wise: "only the low-level ticket change artifacts are synced, and the content of the ticket tables can
always be reconstructed from these artifacts" — the SQL tables are a *derived cache*
(<https://fossil-scm.org/home/doc/trunk/www/tech_overview.wiki>,
<https://fossil-scm.org/home/doc/trunk/www/sync.wiki>). Artifacts are transferred content-addressed by
artifact ID via "file" cards, with delta encoding.

**This is the same shape as a SQLite+JSONL tracker, with one difference that matters: Fossil's on-disk unit
is a per-change artifact, not a per-issue state record.** That is what buys it conflict-free sync.

### 2.2 git-bug — operation log + Lamport clocks + DAG, inside git refs

<https://raw.githubusercontent.com/git-bug/git-bug/trunk/doc/design/data-model.md> (repo:
<https://github.com/git-bug/git-bug>)

- "Instead of storing the final bug data directly, we store a series of edit `Operation`s." Explicitly framed
  as operation-based CRDTs.
- "To get the final state of an entity, we apply these `Operation`s in the correct order on an empty state,
  to compute (aka 'compile') our view."
- Each `Operation` carries a type, author ref, timestamp, Lamport clock(s), payload, and **a random nonce for
  hash entropy** (so that two semantically identical operations get distinct ids).
- Operations are grouped into an `OperationPack` (a JSON array = one edit session), stored as a git blob,
  referenced by a tree, wrapped in a commit; the entity is a ref at `refs/<namespace>/<id>`.
- On wall clocks: "you can't rely on the time provided by other people (their clock might be off) for
  anything other than just display."
- Lamport clocks are encoded **into tree entry names** (e.g. `create-clock-14`) pointing at an empty blob —
  a cute trick to make logical time part of the git object graph.
- Identity: "This identifier is computed from the `Operation`'s data itself, with a hash of that data:
  `id = hash(json(op))`", and the entity id is "the hash of the first `Operation` of the entity, as
  serialized on disk". Displayed truncated to 7 chars, any unambiguous prefix accepted.
- Concurrent edits: "`git-bug` creates the equivalent of a merge commit to merge both branches into a DAG."
  Ordering during merge: (1) load all commits/packs; (2) **validate that Lamport clocks respect the DAG** — a
  parent's clock may not be "higher or equal than its direct child", else the commit is rejected;
  (3) order operations by Lamport clock, falling back to "the lexicographic order of the `OperationPack`'s
  identifier". "we inherit the implicit ordering given by the DAG". On the tiebreak: "This secondary ordering
  doesn't carry much meaning, but it's unbiased and hard to abuse."
- Architecture doc (<https://raw.githubusercontent.com/git-bug/git-bug/trunk/doc/design/architecture.md>): a
  `cache` layer keeps a "pre-digested excerpt" per bug for fast querying, guarantees a single loaded instance
  per bug "to avoid data loss from divergent copies", and uses a lock file "to protect on-disk data from
  concurrent git-bug (not general git) operations".

**Takeaways for `obr`:** the hash-of-canonical-JSON id, the nonce-for-entropy trick, the
Lamport-then-lexicographic total order, the DAG-consistency validation, and the "cache is derived, ops are
truth" split are all directly transplantable ideas.

### 2.3 Radicle Collaborative Objects (COBs)

- LWN overview: <https://lwn.net/Articles/966869/> — issues and patches are stored as a CRDT called a
  "collaborative object"; any node can append to a COB and changes from different nodes merge without
  conflict.
- Refs live under `refs/cobs/<namespace>` with reverse-DNS type names (`xyz.radicle.issue`,
  `xyz.radicle.patch`), extensible to user-defined types without network coordination —
  <https://radicle.dev/guides/protocol>, <https://hackmd.io/@radicle/rJ2UH54P6>.
- "Each modification is stored as a separate Git commit object to ensure that the CRDT change graph is
  compatible with Git's fetch protocol", and state is obtained by replaying "all changes in a deterministic
  and causally-consistent order" — <https://deepwiki.com/radicle-dev/heartwood/6.1-collaborative-objects-(cobs)>.
- Original RFC assumed Automerge:
  <https://github.com/radicle-dev/radicle-link/blob/master/docs/rfc/0662-collaborative-objects.adoc>
  — refs at `refs/namespaces/<namespace>/cob/<typename>/<object ID>`. Heartwood later implemented custom
  Rust CRDTs "inspired by Ink & Switch's Automerge" instead (<https://radicle.dev/guides/protocol>).
- All operations are cryptographically signed; unsigned/improperly signed changes are rejected.
- Crate docs: <https://docs.rs/radicle-cob> — COBs are "graphs of CRDTs"; `ObjectId` is a content address.

### 2.4 The graveyard, and what killed each one

<https://remenos.codes/building-on-gits-primitives> and
<https://matej.ceplovi.cz/blog/current-state-of-the-distributed-issue-tracking.html> and
<https://github.com/npryce/deft/wiki/Other-Distributed-Issue-Trackers>:

- **Bugs Everywhere** — "stored issues in the working tree, which caused merge conflicts". The canonical
  cautionary tale; users had to hand-resolve issue-file conflicts.
- **ticgit** — "the granddaddy of all issues-in-separate-branch systems"; declared dead; "its creator went on
  to build GitHub instead".
- **git-bug** — "a real, working distributed issue tracker", but critiqued for adding CRDTs as "a new
  conceptual layer on top of git rather than staying inside git's native model".
- **Deft** — its stated design requirement was that "the feature database must be easy to deal with when
  merge conflicts occur — plain text or readable structured text, not XML"; uses Markdown + YAML.
- **git-native-issue** (<https://github.com/remenoscodes/git-native-issue>) — the modern reaction: issues at
  `refs/issues/<uuid>`, events as chained commits ("Append-only, content-addressed, cryptographically
  verified"), fields as **git trailers**, history = commit ancestry, sync = plain `git fetch`/`push` with
  "Zero custom protocol". Conflict policy: "label changes use three-way set merging; scalar fields (like
  priority) use last-writer-wins by timestamp", which the author calls "the simplest correct thing I could
  find" while admitting uncertainty about scale.
- The recurring meta-lesson from that post: "Every previous attempt at distributed issue tracking failed to
  produce a format specification." Without a spec, interop means reverse-engineering an implementation.

### 2.5 Upstream beads has already moved off the JSONL-as-truth model

- <https://ianbull.com/posts/beads/> and <https://ai.miraheze.org/wiki/Beads>: classic architecture was
  "bd create writes to SQLite immediately, then exports to JSONL … git IS the database", with
  "hash-based issue IDs (e.g., bd-a3f2)" chosen to "prevent collisions when multiple agents create issues
  simultaneously".
- Same sources: newer versions sync via `bd dolt push` / `bd dolt pull` against `refs/dolt/data`, with
  "`.beads/issues.jsonl` serving as an export for viewers and interchange rather than the source of truth or
  a backup."
- Yegge's own reports of loss under the classic model: "We lost bd-180, bd-181, and bd-182 when we accepted
  the remote's cleaned JSONL" and "We lost bd-100 through bd-179 (80 issues!)" during a rebase — surfaced in
  <https://ianbull.com/posts/beads/> summarising <https://steve-yegge.medium.com/beads-best-practices-2db636b9760c>.
- Best-practice guidance from Yegge: `bd cleanup` aggressively (~2 days), because "issues remain in git
  history even after deletion"; and a practical ceiling — agents grep `issues.jsonl` directly, which "fails
  if the file is larger than 25k tokens (roughly 500-ish Beads issues)"
  (<https://steve-yegge.medium.com/beads-best-practices-2db636b9760c>).
- `obr` is explicitly the frozen-classic-architecture fork:
  <https://github.com/Dicklesworthstone/beads_rust>.

**This is the most important strategic datum in this note: the upstream project concluded that
JSONL-as-source-of-truth-in-git did not hold, and replaced it with a versioned database (Dolt) synced over a
git ref.** A port that freezes the classic architecture must therefore harden the JSONL path deliberately,
because upstream's answer to its weaknesses was "stop using it as truth".

---

## 3. Line-oriented text databases in git: what makes them merge well

### 3.1 The format contract

<https://jsonlines.org/>:
- UTF-8; a BOM (U+FEFF) "must NOT be included".
- Each line is "a valid JSON value"; a blank line is not valid.
- Line terminator `\n` (`\r\n` tolerated since JSON ignores surrounding whitespace).
- Trailing newline "strongly recommended but not required"; if present "must be the last byte in the file".
- Consistent newline termination "makes generating and concatenating JSON Lines files easier".
- Suggested extension `.jsonl`; candidate MIME `application/jsonl` (not standardised).

Because JSON strings must escape control characters, a *valid* JSONL record can never contain a raw newline —
which is the whole reason line-granular merging is even defensible here.

### 3.2 The general tradeoff

Open Knowledge Foundation, "Git (and Github) for Data",
<https://blog.okfn.org/2013/07/02/git-and-github-for-data/>:
- "fundamentally a revision system is built around a diff format and a merge protocol — get these right and
  much of the rest follows".
- Three options: (1) serialize to line-oriented text and use git; (2) diff at an atomic structural level
  (CouchDB-style documents, RDBMS row-level COW); (3) record transforms (OpenRefine-style).
- Line-oriented works "best for data made up of lots of similar records, ideally tabular", and "best for
  smallish data (e.g. < 100k rows, < 50mb files, optimally < 5mb files)".
- The killer caveat: "swapping the order of two fields (columns) leads to a change in every single line",
  and the pattern degrades "if your CSV is not very line-oriented (e.g. you have a lot of fields with line
  breaks in them)".

`obr`'s `.beads/issues.jsonl` is 1.7 MB today — inside the "optimal" band's outer edge, over the 5 MB
"optimal" line if it triples.

### 3.3 Determinism is the whole ballgame

Serialization instability turns a one-record change into a whole-file diff, and (per Case K) scrambles union
merges. The canonicalization literature is directly applicable:

RFC 8785, JSON Canonicalization Scheme, <https://datatracker.ietf.org/doc/html/rfc8785> (info page:
<https://www.rfc-editor.org/info/rfc8785/>):
- Motivation: "Cryptographic operations like hashing and signing need the data to be expressed in an
  invariant format so that the operations are reliably repeatable."
- Mechanism: I-JSON subset, **lexicographic property sorting by UTF-16 code unit**, ECMAScript-compatible
  number serialization, whitespace stripped. Output is a "hashable" representation.
- It is **Informational**, not Standards Track — "the RFC Editor makes no statement about its value for
  implementation or deployment".
- Known weak spot: decimal/number handling. "Handling decimals in JSON is a known challenge, and certain
  corner cases may not behave as expected", because JCS inherits ECMAScript number semantics.
- Practitioner failure classes worth stealing as a test matrix
  (<https://dev.to/lenny321/json-canon-a-strict-rfc-8785-implementation-in-go-for-deterministic-json-3mfg>):
  `INVALID_UTF8`, `INVALID_GRAMMAR`, `DUPLICATE_KEY`, `LONE_SURROGATE`, `NONCHARACTER`, `NUMBER_OVERFLOW`,
  `NUMBER_NEGZERO`, `NUMBER_UNDERFLOW`, `BOUND_EXCEEDED`, `NOT_CANONICAL`.
- The same source's sharp distinction: "a canonicalizer that accepts invalid input is a normalizer, and
  normalizers do not provide the determinism guarantees that signing and hashing pipelines require."
- Practical consequence for content-addressed dedup: **if the hash is over serialized JSON, the serializer is
  part of your data format**. Any change to key order, float formatting, or escaping silently changes every
  hash and defeats dedup.

Also relevant: `git config merge.renormalize` exists so that clean/smudge filter changes don't produce
spurious conflicts, and works only if "a 'smudge→clean' results in the same output as a 'clean'"
(<https://git-scm.com/docs/gitattributes>) — the same idempotence requirement, phrased in git's terms.

### 3.4 Sorted beats append-ordered

**[EXPERIMENT] Case J** above: a file sorted by `id` where two branches insert different records in the
middle merges cleanly and *stays sorted*, under union and (I also observed clean auto-merge behaviour for
disjoint hunks) under the default text driver. Append-ordered files, by contrast, put every concurrent write
in the same hunk at EOF, which is precisely where union's arbitrary ordering and duplicate output bite.

`obr` already sorts by id — this is a load-bearing property that should be asserted in tests, not left as an
emergent behaviour of the exporter.

---

## 4. Three-way merge of structured records

### 4.1 The two IETF patch formats and why neither is a merge algorithm

- **RFC 7386 / 7396 JSON Merge Patch**, <https://www.rfc-editor.org/rfc/rfc7386.html>:
  - "if the patch is anything other than an object, the result will always be to replace the entire target
    with the entire patch, and it is not possible to patch part of a target that is not an object, such as
    to replace just some of the values in an array" — **arrays are replaced wholesale**.
  - `null` is overloaded as the delete sentinel: "if a value in the patch is null and the name exists in the
    target, the name/value pair is removed from the target". Therefore **you cannot set a field to `null`**.
  - Self-assessed scope: "The merge patch format is not appropriate for all JSON syntaxes."
- **RFC 6902 JSON Patch** — an explicit op list (`add`/`remove`/`replace`/`move`/`copy`/`test`), needed
  whenever you must address array elements or set `null`
  (<https://erosb.github.io/json-patch-vs-merge-patch/>, <https://zuplo.com/learning-center/what-is-json-merge-patch>).
- Neither is a *merge*: both are one-directional patch application. A three-way merge still needs a policy
  for "both sides changed the same field".

### 4.2 Kubernetes strategic merge patch — the best-documented "merge lists by key" design

<https://github.com/kubernetes/community/blob/main/contributors/devel/sig-api-machinery/strategic-merge-patch.md>,
<https://deepwiki.com/kubernetes/apimachinery/6.1-strategic-merge-patch>,
<https://itnext.io/kubernetes-strategic-merge-patch-4bdd19b48789>:

- Motivation: "neither of the other patch strategies have a good way to deal with lists".
- Schema annotations drive behaviour: **`patchStrategy`** (`merge` | `replace`) and **`patchMergeKey`**
  (the field that identifies list elements). With `merge`, "a `patchMergeKey` can be specified to identify
  the field of each element to use as the equivalent of a map key so that elements can be matched
  irrespective of their array positions."
- Lists **without** a merge strategy are replaced entirely. Deletions are expressed as explicit `null`s;
  primitive lists get `$deleteFromPrimitiveList`.
- Ordering is not preserved by key-merging, so a separate **`$setElementOrder`** directive was added, because
  "the Kubernetes API may apply semantic meaning to the ordering of items within a list"
  (<https://github.com/kubernetes/design-proposals-archive/blob/main/cli/preserve-order-in-strategic-merge-patch.md>).
- Three-way variant used by `kubectl apply` compares last-applied-config, desired config, and live state so
  that out-of-band changes survive.

**Direct lesson:** merging a JSONL tracker is exactly "merge a list of objects by `patchMergeKey = id`, with
per-field strategies". `labels` want set-merge; `status`/`priority` want a scalar policy; `comments`/`events`
want append-merge; `description` wants either a text 3-way merge or an explicit conflict.

### 4.3 Dolt — a versioned SQL database with git semantics

- Cell-level 3-way merge: <https://www.dolthub.com/blog/2020-07-15-three-way-merge/> and
  <https://www.dolthub.com/blog/2024-06-19-threeway-merge/> — "three-way merge in a SQL database is a
  six-step process whereas three-way merge of files is a three-step process".
- Implementation shape: diff left and right against base, then "patches from right are applied directly to
  left, reducing write work"; a `CollisionFn` handles key-value pairs modified on both sides, attempting "a
  cell-wise merge of the tuples, or registers a conflict if such a merge is not possible"
  (<https://pkg.go.dev/github.com/dolthub/dolt/go/store/prolly/tree>).
- Storage: Prolly Trees give **history independence** — "content-addressing a database requires the same
  database to produce the same content-address no matter what order it was built in"
  (<https://www.dolthub.com/blog/2024-09-18-database-branches/>); ~4 KiB chunks, so identical chunks are
  stored once, and diffs cost "proportional to the size of the differences"
  (<https://www.dolthub.com/blog/2020-06-16-efficient-diff-on-prolly-trees/>).
- Conflicts are **first-class rows** in Dolt, not text markers — surfaced via system tables
  (<https://docs.dolthub.com/concepts/dolt/git/diff>).

Since upstream beads migrated to Dolt, this is also a description of where the ecosystem went.

### 4.4 SQLite's own answer: the session extension

<https://sqlite.org/sessionintro.html>, <https://sqlite.org/session.html>,
<https://sqlite.org/session/sqlite3changeset_apply.html>, <https://sqlite.org/session/rebaser.html>:

- Changesets record row-level changes; "the complications in applying a changeset lie in conflict resolution".
- Conflict cases enumerated by SQLite: no row with the PK exists; the row exists but "current field values
  that don't match the original values stored within the changeset"; or a UNIQUE/CHECK violation.
- `xConflict` callback options: OMIT the change, ABORT the whole changeset, or REPLACE. "It is safe to
  execute SQL statements … from within the xConflict callback."
- **Rebaser**: `sqlite3changeset_apply_v2()` can emit a rebase buffer when conflicts occurred; the local
  changeset is then rebased against remote resolutions. Crucially, "conflict resolutions from multiple remote
  changesets are combined on a per-field basis rather than per-row" — i.e. SQLite converged on the same
  field-granularity answer as Kubernetes and Dolt.
- Known sharp edges from the SQLite forum:
  <https://sqlite.org/forum/info/b538d66337d541ddd4673903bee2e3aa060c1574e2142c580f52bf77032adab8> —
  returning `SQLITE_CHANGESET_REPLACE` can yield `SQLITE_CONSTRAINT` with no changes, and
  `SQLITE_CHANGESET_OMIT` can result in the row being deleted, contrary to intuition.
- Ecosystem: SQLiteChangesetSync (<https://github.com/gerdemb/SQLiteChangesetSync>) detects "a merge conflict
  … not when branches are merged, but rather when attempting to apply (pull) a changeset"; sqlite-sync
  (<https://github.com/sqliteai/sqlite-sync-dev>) layers CRDTs on SQLite with block-level LWW for text.

---

## 5. CRDTs: what's transplantable and what is a trap

### 5.1 The core promise and the coordination fine print

- Definition and convergence guarantee: <https://simongui.github.io/distributed-systems/crdt.html>,
  glossary at <https://crdt.tech/glossary>, resources at <https://crdt.tech/resources>.
- **State-based (CvRDT) vs op-based (CmRDT):** "State-based CRDTs (CvRDTs) transmit full state and need only
  eventual delivery, while operation-based CRDTs (CmRDTs) are bandwidth-efficient but need exactly-once
  delivery"; op-based "typically require a reliable, causally-ordered broadcast mechanism, which is fragile
  in the face of long network partitions or dynamic membership"
  (<https://www.iankduncan.com/engineering/2025-11-27-crdt-dictionary/>,
  <https://interjectedfuture.com/trade-offs-between-different-crdts/>).
- **Delta-state CRDTs** are the pragmatic middle: <https://arxiv.org/pdf/1410.2803> ("Efficient State-based
  CRDTs by Delta-Mutation"); a field study found "the State-Based CRDT system was not viable due to flooding
  the network with too many and too large messages, while the Delta-State CRDT system … is a viable
  replacement" (<https://odr.chalmers.se/items/325cb377-97b9-407a-8218-66593e442e54>). See also
  <https://blog.helsing.ai/posts/dson-a-delta-state-crdt-for-resilient-peer-to-peer-communication/> and
  bandwidth-optimised sync in <https://arxiv.org/html/2505.01144> (ConflictSync).

Git is a *reliable, eventually-delivered, causally-ordered* transport when you sync whole refs — which is
exactly why git-bug and Radicle can get away with op-based designs on top of it.

### 5.2 Sets: add-wins vs remove-wins, and why naive tombstones fail

<https://arxiv.org/pdf/1210.3368> ("An optimized conflict-free replicated set"), <https://crdt.tech/glossary>,
<https://arxiv.org/pdf/1710.04469> ("Pure Operation-Based Replicated Data Types"), <https://www.gatlin.io/content/crdts>:

- **OR-Set / Add-Wins Set**: every `add` mints a unique tag; a `remove` records the tags it has *observed*.
  An element is present iff some add-tag exists that no remove has seen. Merge:
  `E := (E \ B.T) ∪ (B.E \ T); T := T ∪ B.T`.
- Without unique tags, "a simple removed-set approach makes it impossible to add an element back after a
  concurrent remove has been received".
- **Remove-Wins Set** is the dual: "in the presence of a concurrent add and remove of the same element, the
  remove wins".
- The choice is *policy*, not correctness: pick one, document it, and be consistent.

Note what §1.3 Case F/I showed: git's union driver implicitly implements **update-wins-over-delete** with no
tags at all — the weakest possible version of add-wins, and one that cannot be re-deleted deterministically.

### 5.3 The hard parts (Kleppmann)

<https://martin.kleppmann.com/2020/07/06/crdt-hard-parts-hydra.html> (slides
<https://speakerdeck.com/ept/crdts-the-hard-parts>):

- "CRDTs are easy to implement badly. Many published algorithms have anomalies that cause them to behave
  strangely in some situations. Simple implementations often have terrible performance."
- **Interleaving anomalies** — Kleppmann/Gomes/Mulligan/Beresford, PaPoC 2019
  (<https://martin.kleppmann.com/papers/interleaving-papoc19.pdf>). Classic example
  (<https://arxiv.org/pdf/2310.18220>): concurrent inserts of " Alice" and " Charlie" into "Hello!" can
  converge to "Hello Al Ciharcliee!" in Logoot, LSEQ, Treedoc and WOOT; a lesser anomaly occurs in RGA when
  typing backwards. Follow-up work (Fugue, <https://arxiv.org/pdf/2305.00583>) shows Kleppmann's original
  non-interleaving definition "cannot be satisfied by any algorithm" and that the proposed algorithm "does
  not converge".
- **Move operations** are their own research problem —
  <https://martin.kleppmann.com/papers/list-move-papoc20.pdf>,
  <https://martin.kleppmann.com/papers/move-op.pdf>.
- **Metadata/tombstone overhead** motivated columnar encoding for Automerge
  (<https://github.com/automerge/automerge-perf/blob/master/columnar/README.md>,
  <https://github.com/automerge/automerge/pull/253>).
- Theory backdrop: <https://arxiv.org/abs/1805.04263> (OpSets), and Attiya et al. PODC 2016
  "Specification and Complexity of Collaborative Text Editing".

### 5.4 Automerge and Yjs: history growth and GC in practice

- Automerge preserves full operation history, so "document size grows with the number of operations, not just
  current content size"; deletions leave tombstones; on concurrent delete-vs-update "the update takes
  precedence, with the updated value retained"
  (<https://posit-dev.github.io/automerge-r/articles/crdt-concepts.html>).
- Automerge's mitigation is columnar compression in the binary document format, reaching "4-6 bytes per
  character — still 40-60% overhead versus raw text"
  (<https://zylos.ai/research/2026-01-29-crdt-real-time-collaboration/>).
- Yjs internals (<https://github.com/yjs/yjs/blob/main/INTERNALS.md>): every item has a `(clientID, clock)`
  id; the clock is incremented **only by inserts**; Yjs "does not record metadata about a deletion — no data
  is kept on when an item was deleted or who deleted it". With GC enabled, deleted content is discarded and
  replaced by a lightweight GC object holding only the length. But: "Yjs can't garbage collect deleted
  structs (tombstones) while ensuring a unique order of the structs" — GC is only fully safe "if order no
  longer matters (e.g. if the parent was deleted)".
- Yjs state vectors enable differential sync; `Y.mergeUpdates` compresses but "doesn't garbage-collect
  deleted content" (<https://docs.yjs.dev/api/document-updates>).
- Comparative: <https://www.pkgpulse.com/guides/yjs-vs-automerge-vs-loro-crdt-libraries-2026>,
  <https://loro.dev/docs/performance>.
- Archagon's ORDT framing of GC (<http://archagon.net/blog/2018/03/24/data-laced-with-history/>): GC "isn't
  just about removing tombstone operations — it's also an opportunity to drop redundant operations, coalesce
  operations of the same kind, reduce excess metadata"; split the structure into a *live* part and a
  *compacted* part with a "baseline weft" (version vector) marking the boundary.
- Figma's pragmatic hack: remove tombstones older than 24 h, accepting that late-joining clients may need a
  resync (<https://zylos.ai/research/2026-01-29-crdt-real-time-collaboration/>).

### 5.5 Where CRDTs are overkill

Ink & Switch, "Local-first software", <https://www.inkandswitch.com/essay/local-first/>:
- Git is "perhaps the closest thing we have to a true local-first software package", but limited by lack of
  real-time merging and by three-way merge being line-oriented — other formats are "binary blobs that cannot
  meaningfully be edited or merged".
- Their empirical finding, which is the key permission slip for a simpler design: conflicts were **less
  problematic than anticipated**, because fine-grained type-aware merging handles most cases and because
  "users naturally tend to avoid stepping on each other's edits". True conflicts arise only when "multiple
  users concurrently update the same property of the same object", and default resolution policies sufficed
  across all their prototypes.

For an issue tracker where each agent mostly touches its own issues, the concurrency profile is far milder
than collaborative text editing. Full CRDT machinery is not obviously warranted; *deterministic, documented,
field-level policy* is.

---

## 6. Event sourcing vs state-based sync

- Event sourcing stores "the state of a system as a sequence of events rather than as a current snapshot";
  costs are storage growth and replay latency, mitigated by **snapshots** ("loads the latest snapshot and
  replays only events that occurred afterward") and by **log compaction** (Kafka "retains the latest record
  with a specific key and discards older records with the same key")
  (<https://www.conduktor.io/glossary/event-sourcing-patterns-with-kafka>,
  <https://medium.com/towardsdev/event-sourcing-and-log-compaction-3959cba0cda4>,
  <https://blog.n8n.io/event-sourcing/>).
- Compaction's cost: "log files often serve as valuable sources for debugging … and log compaction may remove
  some log entries useful for diagnosing issues"
  (<https://www.conduktor.io/glossary/event-sourcing-patterns-with-kafka>).
- The parallel to CRDTs is exact: full-history/full-state gives recoverability at the price of size;
  snapshots and delta-states buy size back by shifting complexity into the sync protocol
  (<https://interjectedfuture.com/trade-offs-between-different-crdts/>).
- **Every successful git-native tracker in §2 is event-sourced**: Fossil (ticket change artifacts), git-bug
  (Operations in OperationPacks), Radicle (one commit per modification), git-native-issue (chained commits).
  Each also keeps a **derived** query index — Fossil's SQL ticket tables, git-bug's cache/excerpts.
- **The state-based exception proves the rule**: `obr`'s JSONL is a *state* file (current issue snapshot per
  line). State-based sync is what forces you into LWW, because a state record carries no information about
  *which fields* changed and no causal metadata about what the writer had observed.

---

## 7. Last-writer-wins and clocks

### 7.1 Why wall-clock LWW loses data

Riak, "Clocks Are Bad, Or, Welcome to Distributed Systems",
<https://riak.com/clocks-are-bad-or-welcome-to-distributed-systems/>:
- "The problem is simple: there is no reliable definition of 'last write'"; "system clocks across multiple
  servers are going to drift"; "If two updates to the same object occur within 30 seconds in such an
  environment, the end result is unpredictable."
- Alternatives named: supply vector clocks ("Giving Riak more information to track causal history is never a
  bad thing"); keep siblings (`allow_mult=true`) "and write your application to deal with them".
- On forcing LWW: `last_write_wins=true` "will instruct Riak to **always** overwrite existing objects,
  ignoring the timestamps" — but "we've just punted the problem down the road a bit".
- The line worth putting on a wall: **"If your distributed system isn't explicitly dealing with data
  conflicts, any correct behavior it exhibits is more a matter of good luck than of good design."**

Corroborating material:
- Concrete loss scenario: a node 50 ms ahead writes `x=1`, a correct-clock node writes `x=2` slightly later
  in real time, LWW picks the higher timestamp and `x=2` is silently lost
  (<https://mehedees.medium.com/when-quorum-isnt-enough-how-distributed-clock-skew-silently-discards-data-mehedee-siddique-56b22d0dbd31>).
- Causes of skew: quartz oscillator drift ("a server clock can drift by several seconds per day"), NTP sync
  intervals (~64 s), unreachable NTP servers, CPU load delaying adjustments, and **VM live migration causing
  clock jumps** (<https://medium.com/@truongtud90/the-hidden-pitfall-of-timestamps-in-distributed-systems-a-spring-boot-guide-fba77b4cf798>).
- Skew of ~150 ms already breaks creation-time sorting, time-window queries, audit-log ordering, LWW, and
  TTL locks (same source).
- Production postmortem: `chronyd` silently killed, clock drifted, "valid updates were silently discarded"
  until it was restarted (<https://mehedees.medium.com/when-quorum-isnt-enough-how-distributed-clock-skew-silently-discards-data-mehedee-siddique-56b22d0dbd31>).
- General framing: <https://www.numberanalytics.com/blog/last-writer-wins-distributed-systems>,
  <https://oneuptime.com/blog/post/2026-01-30-last-write-wins/view>,
  <https://manthanguptaa.in/posts/replication_in_distributed_systems_part_3/>.

### 7.2 Logical time options

<https://snormore.dev/blog/logical-clocks-in-distributed-systems/>,
<https://dev.to/santosh_koti/distributed-systems-lamport-clock-vs-hybrid-logical-clocks-aj1>,
<https://sergeiturukin.com/2017/06/26/hybrid-logical-clocks.html>,
<https://www.geeksforgeeks.org/distributed-systems/logical-clock-in-distributed-system/>:

| | Guarantee | Size | Wall-clock meaning | Detects concurrency? |
|---|---|---|---|---|
| **Lamport** | `a → b ⟹ L(a) < L(b)` (one direction only) | O(1) | none | no |
| **Vector clock** | `a → b ⟺ V(a) < V(b)` (biconditional) | O(n) nodes | none | **yes** |
| **HLC** | same one-directional guarantee as Lamport, plus `pt(e)+ε < pt(f) ⟹ hlc(e) < hlc(f)` | O(1) | close to real time | no |

- Lamport: "a Lamport timestamp tells you nothing about wall-clock time … You get ordering, not timing."
- Vector clocks: "high computational overhead as they must trace dependencies"; use them "when your
  application needs to know about conflicts rather than silently resolving them".
- HLC: tuple `(physical_time, logical_counter, node_id)`; on merge, physical time = max of local/remote, and
  when equal, counter = max+1 — "ensures the HLC timestamp preserves causality even when system clocks drift
  or move backward". Used by CockroachDB to avoid needing TrueTime.
- Information-theoretic floor: exact causality tracking needs O(n) space per timestamp (Dilworth's theorem
  argument, <https://snormore.dev/blog/logical-clocks-in-distributed-systems/>) — which is precisely why
  Lamport/HLC cannot detect concurrency.
- git-bug's choice: Lamport clocks + DAG-derived ordering + lexicographic tiebreak, with wall-clock
  timestamps kept "for anything other than just display" explicitly disclaimed
  (<https://raw.githubusercontent.com/git-bug/git-bug/trunk/doc/design/data-model.md>).
- Fossil's choice: wall-clock replay ordering, with an explicit caveat that drift of "months or years"
  breaks it (<https://fossil-scm.org/home/doc/tip/www/bugtheory.wiki>).

**Key insight for a git-based tracker:** git *already gives you a causal DAG for free*. The merge-base and
commit ancestry of `.beads/issues.jsonl` are a happens-before relation. A tool that never runs git can still
*read* `git merge-base`-equivalent information — or, more cheaply, can store a per-writer Lamport counter in
each record and use `(lamport, writer_id)` as the ordering key, keeping `updated_at` for display only.

---

## 8. Content-addressed storage and hash-based dedup

- Git as CAS: every object keyed by its hash, giving automatic dedup ("if 10,000 commits all include an
  unchanged README.md, there's exactly one blob object") and integrity ("flip one bit … and `git fsck` will
  catch it immediately")
  (<https://runbook.fyi/Git/object-model/>, <https://dev.to/__whyd_rf/a-deep-dive-into-git-internals-blobs-trees-and-commits-1doc>).
- General CAS lineage: Merkle trees and content-derived identifiers from the 1990s; IPFS CIDs are
  multihash-based and Merkle-link file blocks; Perkeep uses BLAKE2
  (<https://grokipedia.com/page/Content-addressable_storage>,
  <https://lab.abilian.com/Tech/Databases%20&%20Persistence/Content%20Addressable%20Storage%20(CAS)/>).
- Git is migrating SHA-1 → SHA-256 with a compatibility layer, and hardens against known SHA-1 collision
  attacks meanwhile (<https://runbook.fyi/Git/object-model/>).
- Dolt's Prolly Trees add **history independence** — the same logical database content-addresses identically
  regardless of insertion order (<https://www.dolthub.com/blog/2024-09-18-database-branches/>). If `obr` ever
  hashes a *collection* rather than a record, this property is the thing to aim for.

### 8.1 Short ids are a birthday problem, and it bites early

Beads uses short hash-derived ids like `bd-a3f2` (<https://ianbull.com/posts/beads/>); `obr` uses
`beads_rust-07b`, `beads_rust-0a5` — 3 base-36-ish characters.

- Git's own experience: git "checks for collisions when abbreviating object names, starting at … seven by
  default … and keeps adding digits until the result names a unique object", and warns that "a name that's
  unique one day may not be the next"
  (<https://blog.cuviper.com/2013/11/10/how-short-can-git-abbreviate/>).
- Measured on a real repo: 4 hex chars is "totally saturated — every object is ambiguous"; the default 7
  leaves 1.2% ambiguous; 8 still has triplicates; 12 was needed to disambiguate everything (same source).
- The Linux kernel actually hit an abbreviated-hash collision, prompting a proposal to raise the minimum to
  16 chars; Torvalds' counterargument and data:
  <https://lwn.net/Articles/1001526/>, <https://lkml.iu.edu/hypermail/linux/kernel/2412.0/07364.html>,
  <https://news.ycombinator.com/item?id=42554420>.
- Birthday math for a 3-char base-36 id space (46,656 values): expected first collision at ≈ √(π·N/2) ≈ **270
  issues**. `obr`'s own `.beads/issues.jsonl` is 1.7 MB and clearly holds far more than 270 issues — meaning
  the id space must already be doing something beyond 3 random chars (suffixes like `-0v1.1.1` suggest
  hierarchical ids). **This deserves an explicit audit**: the id length must be sized to the *lifetime* issue
  count including tombstones, and collision handling must be deterministic across replicas (two agents
  offline must not both mint `beads_rust-0v1` for different issues, or a union merge will produce two lines
  with the same id and LWW will destroy one). See <https://getsolid.io/blog/birthday-paradox-coding-solid.html>
  for the generic version of this mistake.

---

## 9. Tombstones and deletion GC

- Tombstone definition and origin: a lightweight timestamped deletion marker; the concept originated in
  Google's Bigtable, where deletions write entries that suppress older versions until a major compaction
  removes them (<https://en.wikipedia.org/wiki/Tombstone_(data_store)>,
  <https://grokipedia.com/page/Tombstone_(data_store)>).
- Why immutable/append-only stores need them: Cassandra/ScyllaDB use immutable SSTables, so "rather than
  alter a database record itself, you leave a marker"
  (<https://www.scylladb.com/2022/06/30/preventing-data-resurrection-with-repair-based-tombstone-garbage-collection/>).
- **Resurrection**: the classic 3-node scenario — a QUORUM delete succeeds on two nodes while a third is
  down; if that node rejoins after the hinted-handoff window and the tombstones are GC'd before repair, the
  deleted data comes back (<https://www.compactionstrategy.org/cassandra-architecture-compaction-fundamentals/tombstone-management-garbage-collection/>,
  <https://oneuptime.com/blog/post/2026-01-26-cassandra-tombstones/view>).
- **The TTL is a safety parameter, not a storage knob.** Cassandra's `gc_grace_seconds` is only safe "if you
  run regular repairs more frequently than gc_grace_seconds and all nodes are consistently available"
  (same sources).
- ScyllaDB's improvement: GC tombstones **based on repair execution** rather than a fixed duration, using
  Merkle-tree anti-entropy to confirm every replica saw the delete
  (<https://www.scylladb.com/2022/06/30/preventing-data-resurrection-with-repair-based-tombstone-garbage-collection/>).
- In CRDT-land the same constraint: "CRDTs can garbage collect tombstones when all replicas have seen the
  deletion, but this requires coordination, which partially negates the 'no coordination needed' benefit"
  (<https://zylos.ai/research/2026-01-29-crdt-real-time-collaboration/>). Automerge caps undo history at 100
  operations to bound metadata growth (same source).
- Cost of keeping them: tombstones consume storage, sync-query processing, and bandwidth
  (<https://tdwi.org/blogs/data-101/2026/05/what-is-a-tombstone.aspx>); at scale "files with 10 million
  tombstones can balloon to gigabytes" (<https://zylos.ai/research/2026-01-29-crdt-real-time-collaboration/>).

**Applied to `obr`**: `/Users/johnw/src/obr/src/sync/mod.rs:44` has "Retention period for tombstones in days
(None = keep forever)" and `:1455-1457` drops expired tombstones from export. In a git world there is **no
repair protocol and no membership list** — a clone can sit on a laptop for months. A time-based tombstone TTL
is therefore *exactly* the Cassandra `gc_grace_seconds` footgun without the repair discipline that makes it
safe. A collaborator who was offline longer than `retention_days` and had the issue locally will re-introduce
it on their next export: **classic zombie resurrection**. Yegge's advice to run `bd cleanup` with a 2-day
window (<https://steve-yegge.medium.com/beads-best-practices-2db636b9760c>) makes this *more* likely, not
less; his mitigation is that "issues remain in git history even after deletion" — recovery by archaeology,
not prevention.

The git-native alternative: GC a tombstone only when it is provably present in the merge-base shared by all
active branches — i.e. tie retention to *ancestry reachability*, not to elapsed time. That is the direct
analogue of ScyllaDB's repair-based GC.

---

## 10. Synthesis: what a SQLite + JSONL tracker should and should not rely on

### 10.1 Do not rely on

1. **`merge=union` as a correctness mechanism.** It exits 0 on genuine conflicts (duplicate ids, §1.3 Case E),
   silently resurrects deletes (Cases F, I), duplicates records when the trailing newline is missing (Case
   H), degrades to ours-only on any binary-looking byte (Case NUL), and is ignored by GitHub, GitLab and
   Bitbucket web merges (§1.4). Git's own manual says "Do not use this if you do not understand the
   implications" (<https://git-scm.com/docs/gitattributes>).
2. **A custom merge driver as the only line of defence.** Same forge problem (§1.4), plus it must be
   installed per-clone in `.git/config` — which a fresh CI checkout or a new contributor will not have.
3. **Wall-clock `updated_at` as the arbiter of truth.** §7.1. Two agents on one machine can tie; two machines
   can invert; a VM migration can jump. `/Users/johnw/src/obr/src/sync/mod.rs:2120` is exactly the pattern
   Riak calls "more a matter of good luck than of good design".
4. **Time-based tombstone expiry as a substitute for propagation proof.** §9.
5. **Short random ids without a lifetime-sized space and a deterministic collision rule.** §8.1.
6. **File order carrying meaning.** Union output order is explicitly "random"; a re-export that reorders
   produces whole-file diffs (§3.2) and scrambled merges (Case K).
7. **A serializer that is not byte-deterministic.** If SHA-256 dedup hashes serialized JSON, the serializer
   is part of the wire format (§3.3).
8. **"Conflicts are rare so it's fine."** The upstream project shipped on that assumption and reported losing
   80 issues in one rebase (<https://ianbull.com/posts/beads/>), then moved off the architecture entirely.

### 10.2 Do rely on

1. **JSONL's structural invariants, enforced mechanically**: exactly one canonical JSON value per line, UTF-8,
   no BOM, no raw control bytes anywhere (this both satisfies JSON and dodges git's binary heuristic), and a
   mandatory trailing `\n` (<https://jsonlines.org/>, §1.3 Case H/NUL).
2. **A total, deterministic record order — sort by id.** Case J shows sorted files merge cleanly and stay
   sorted; `obr` already does this and should test-lock it.
3. **Canonical serialization** — RFC 8785-style key sorting and number formatting, with the failure classes
   from §3.3 as a test matrix (<https://datatracker.ietf.org/doc/html/rfc8785>).
4. **The importer as the real merge engine.** Since forge merges bypass drivers, `br import` must be able to
   ingest a *post-union, possibly duplicate-keyed, possibly conflict-markered* file and reconcile it
   deterministically. `obr` already scans for conflict markers
   (`/Users/johnw/src/obr/src/sync/mod.rs:1053,1091,2221`); it needs the duplicate-id path to be equally
   explicit.
5. **Field-level merge policy, not record-level LWW.** All three mature systems converged here:
   Kubernetes `patchMergeKey` + per-field `patchStrategy` (§4.2), Dolt's cell-level merge with a `CollisionFn`
   (§4.3), SQLite's rebaser combining resolutions "on a per-field basis rather than per-row" (§4.4).
6. **Logical time.** Lamport counter per writer, plus writer id as tiebreak, exactly as git-bug does
   (§2.2, §7.2). Keep wall clocks for display. HLC if human-meaningful timestamps are needed (§7.2).
7. **Content-addressed, append-only event records where the semantics are naturally additive** — comments,
   status transitions, dependency edges. Fossil, git-bug, Radicle and git-native-issue all landed on this
   (§2), and it converts most concurrent edits from conflicts into commutative appends.
8. **SQLite as a derived, rebuildable cache.** Fossil: "the content of the ticket tables can always be
   reconstructed from these artifacts" (<https://fossil-scm.org/home/doc/tip/www/bugtheory.wiki>). git-bug
   maintains excerpts and a lock file for the same reason. If SQLite can always be rebuilt from JSONL, a bad
   merge is recoverable; if SQLite holds state JSONL cannot express, it is not.
9. **Loud, non-silent conflicts.** Ink & Switch found true conflicts are rare in practice
   (<https://www.inkandswitch.com/essay/local-first/>) — which means surfacing them costs little and buys
   correctness. Dolt's model of conflicts-as-rows (§4.3) is a good template: `br conflicts` listing unresolved
   field-level divergences beats silent LWW.
10. **A written format specification.** The recurring post-mortem across two decades of git trackers:
    "Every previous attempt at distributed issue tracking failed to produce a format specification"
    (<https://remenos.codes/building-on-gits-primitives>).

### 10.3 A concrete, incremental hardening path for `obr`

Ordered by (value ÷ disruption). None of these require abandoning the classic architecture.

1. **Ship a `.gitattributes` that makes the file conflict *loudly*, not quietly.** Do **not** set
   `merge=union`. Either leave the default text driver (conflicts get markers, and
   `ensure_no_conflict_markers` already refuses to import them — the safe failure) or set a custom
   `merge=br` driver *and* keep the marker check as the backstop for forge-side merges.
2. **Add a `br doctor` / import-time check for duplicate ids in the JSONL** with an explicit, deterministic
   resolution (highest `(lamport, writer_id)`, or "keep both and flag"). Today a post-union duplicate-id file
   is resolved by whatever `updated_at` comparison at `src/sync/mod.rs:2120-2121` happens to see first.
3. **Assert the file invariants in the exporter and in a test**: sorted by id, trailing newline present, no
   byte < 0x20 outside JSON escapes, canonical key order. Each of these is a distinct measured failure mode
   from §1.3.
4. **Add a monotonic per-writer counter to each record** (`lamport`, `writer_id`) and switch the import
   tiebreak from `updated_at` to `(lamport, writer_id, updated_at)`. This is a purely additive JSONL field;
   old records default to 0 and fall back to today's behaviour.
5. **Change tombstone GC from time-based to ancestry-based**, or at minimum make `retention_days` default to
   `None` (keep forever) and document the resurrection risk. §9.
6. **Audit the id space against the birthday bound** for the expected lifetime issue count *including
   tombstones*, and define what two offline agents do when they mint the same id. §8.1.
7. **Consider a `.beads/events.jsonl` append-only companion** for comments/status/dependency events, keeping
   `issues.jsonl` as a derived materialized view. This is the Fossil/git-bug split, and it makes the
   *append-only* claim actually true — at which point union-style merging becomes defensible for that file
   specifically, because concurrent appends genuinely commute.
8. **Document the merge policy as a spec** (fields, strategies, ordering rule, conflict semantics), per §10.2
   item 10.

---

## 11. Open questions / things I could not verify

- I could **not** reproduce the "union silently merges similar lines into one" claim from
  <https://medium.com/@nettsundere/on-reducing-changelog-merge-conflicts-1eb23552630b> in the case where the
  union driver actually runs (§1.3 Case G showed *duplication*, not dedup). The dedup I did observe (Case D)
  came from both branches producing byte-identical blobs, which resolves before any driver is consulted. The
  blog's claim may reflect an older git, or a scenario I did not construct. Treat "union dedupes" as
  unverified; treat "union duplicates" as verified.
- `https://github.com/git-bug/git-bug/blob/master/doc/design.md` → **404**; the design docs live under
  `doc/design/` on branch `trunk` (URLs used above).
- `https://github.com/steveyegge/beads/blob/main/docs/PROTECTED_BRANCHES.md` → **404** at fetch time (the
  repo appears to have moved/renamed; search results referenced `gastownhall/beads`). The beads architecture
  claims above therefore rest on secondary sources (<https://ianbull.com/posts/beads/>,
  <https://ai.miraheze.org/wiki/Beads>, Yegge's Medium posts) rather than the primary repo docs.
- `https://www.inkandswitch.com/local-first.html` returned empty; the canonical URL is
  <https://www.inkandswitch.com/essay/local-first/>.
- I did not find any published tool that is specifically a **JSONL/NDJSON merge driver keyed by record id**.
  The closest published analogues are JSON-object drivers
  (<https://github.com/rmedaer/git-merge-drivers>, <https://github.com/jonatanpedersen/git-json-merge>,
  <https://github.com/fcostin/jsonmerge_git_merge_driver>, <https://github.com/kettek/git-json-intellimerge>,
  <https://github.com/balbuf/composer-git-merge-driver>) and `merge-drivers-cli` for installation
  (<https://github.com/charpeni/merge-drivers-cli>). The upstream Go beads `merge=beads` driver
  (`/Users/johnw/src/obr/EXISTING_BEADS_STRUCTURE_AND_ARCHITECTURE.md:5825`) may be the only one of its kind;
  worth reading its source if reachable.
- Whether GitLab's web merges now honour built-in `union` (as opposed to custom drivers) is contested:
  <https://gitlab.com/gitlab-org/gitlab-foss/-/issues/17325> has a comment speculating rugged ≥ 0.25 gives
  support, but no authoritative confirmation. GitHub's support statement (§1.4) is explicit that it does not.

---

## 12. Source index

**Git mechanics**
- <https://git-scm.com/docs/gitattributes> — union/text/binary drivers, `%O %A %B %L %P %S %X %Y`, exit-code
  semantics, `merge.*.recursive`, `merge.renormalize`, `conflict-marker-size`.
- <https://github.com/orgs/community/discussions/9288> — GitHub ignores user `.gitattributes` on PR merge.
- <https://github.com/kubernetes/kubernetes/pull/70576> — Kubernetes removed union because GitHub/Prow ignore it.
- <https://gitlab.com/gitlab-org/gitlab/-/issues/18830>, <https://gitlab.com/gitlab-org/gitlab-foss/-/issues/17325> — GitLab.
- <https://community.atlassian.com/forums/Bitbucket-questions/Merge-strategy-in-gitattributes-doesn-t-work-on-Bitbucket/qaq-p/1773507> — Bitbucket.
- <https://medium.com/@nettsundere/on-reducing-changelog-merge-conflicts-1eb23552630b> — union "silent troubles".
- <https://github.com/gitattributes/gitattributes/issues/21>, <https://github.com/scikit-learn/scikit-learn/issues/21516>,
  <https://github.com/olivierlacan/keep-a-changelog/discussions/478> — union skepticism.
- <https://tenthirtyam.org/dispatches/2026/04/11/controlling-git-repository-behavior-with-gitattributes/> — union caveat.
- <https://en.wikipedia.org/wiki/Merge_(version_control)> — unstructured line-based merge, weave merge and deletion tracking.

**Prior-art trackers**
- <https://fossil-scm.org/home/doc/tip/www/bugtheory.wiki>, <https://fossil-scm.org/home/doc/trunk/www/tech_overview.wiki>,
  <https://fossil-scm.org/home/doc/trunk/www/sync.wiki>, <https://fossil-scm.org/home/doc/tip/www/tickets.wiki>.
- <https://raw.githubusercontent.com/git-bug/git-bug/trunk/doc/design/data-model.md>,
  <https://raw.githubusercontent.com/git-bug/git-bug/trunk/doc/design/architecture.md>,
  <https://github.com/git-bug/git-bug>, <https://news.ycombinator.com/item?id=43971620>.
- <https://lwn.net/Articles/966869/>, <https://radicle.dev/guides/protocol>, <https://hackmd.io/@radicle/rJ2UH54P6>,
  <https://deepwiki.com/radicle-dev/heartwood/6.1-collaborative-objects-(cobs)>,
  <https://github.com/radicle-dev/radicle-link/blob/master/docs/rfc/0662-collaborative-objects.adoc>,
  <https://docs.rs/radicle-cob>.
- <https://remenos.codes/building-on-gits-primitives>, <https://github.com/remenoscodes/git-native-issue>,
  <https://matej.ceplovi.cz/blog/current-state-of-the-distributed-issue-tracking.html>,
  <https://github.com/npryce/deft/wiki/Other-Distributed-Issue-Trackers>,
  <https://github.com/dspinellis/git-issue>.
- <https://ianbull.com/posts/beads/>, <https://ai.miraheze.org/wiki/Beads>,
  <https://steve-yegge.medium.com/beads-best-practices-2db636b9760c>,
  <https://steve-yegge.medium.com/introducing-beads-a-coding-agent-memory-system-637d7d92514a>,
  <https://github.com/Dicklesworthstone/beads_rust>.

**Structured merge**
- <https://www.rfc-editor.org/rfc/rfc7386.html>, <https://erosb.github.io/json-patch-vs-merge-patch/>,
  <https://zuplo.com/learning-center/what-is-json-merge-patch>.
- <https://github.com/kubernetes/community/blob/main/contributors/devel/sig-api-machinery/strategic-merge-patch.md>,
  <https://deepwiki.com/kubernetes/apimachinery/6.1-strategic-merge-patch>,
  <https://github.com/kubernetes/design-proposals-archive/blob/main/cli/preserve-order-in-strategic-merge-patch.md>,
  <https://itnext.io/kubernetes-strategic-merge-patch-4bdd19b48789>.
- <https://www.dolthub.com/blog/2020-07-15-three-way-merge/>, <https://www.dolthub.com/blog/2024-06-19-threeway-merge/>,
  <https://www.dolthub.com/blog/2020-06-16-efficient-diff-on-prolly-trees/>,
  <https://www.dolthub.com/blog/2024-09-18-database-branches/>,
  <https://pkg.go.dev/github.com/dolthub/dolt/go/store/prolly/tree>, <https://docs.dolthub.com/concepts/dolt/git/diff>.
- <https://sqlite.org/sessionintro.html>, <https://sqlite.org/session.html>,
  <https://sqlite.org/session/sqlite3changeset_apply.html>, <https://sqlite.org/session/rebaser.html>,
  <https://sqlite.org/forum/info/b538d66337d541ddd4673903bee2e3aa060c1574e2142c580f52bf77032adab8>,
  <https://github.com/gerdemb/SQLiteChangesetSync>, <https://github.com/sqliteai/sqlite-sync-dev>.

**CRDTs**
- <https://martin.kleppmann.com/2020/07/06/crdt-hard-parts-hydra.html>, <https://speakerdeck.com/ept/crdts-the-hard-parts>,
  <https://martin.kleppmann.com/papers/interleaving-papoc19.pdf>, <https://martin.kleppmann.com/papers/list-move-papoc20.pdf>,
  <https://martin.kleppmann.com/papers/move-op.pdf>, <https://arxiv.org/abs/1805.04263>.
- <https://arxiv.org/pdf/2310.18220>, <https://arxiv.org/pdf/2305.00583> (Fugue), <https://arxiv.org/pdf/1210.3368>,
  <https://arxiv.org/pdf/1710.04469>, <https://arxiv.org/pdf/1410.2803>, <https://arxiv.org/html/2505.01144>.
- <https://github.com/yjs/yjs/blob/main/INTERNALS.md>, <https://docs.yjs.dev/api/document-updates>,
  <https://posit-dev.github.io/automerge-r/articles/crdt-concepts.html>,
  <https://github.com/automerge/automerge-perf/blob/master/columnar/README.md>,
  <https://github.com/automerge/automerge/pull/253>, <https://loro.dev/docs/performance>,
  <https://www.pkgpulse.com/guides/yjs-vs-automerge-vs-loro-crdt-libraries-2026>.
- <http://archagon.net/blog/2018/03/24/data-laced-with-history/>,
  <https://zylos.ai/research/2026-01-29-crdt-real-time-collaboration/>,
  <https://www.iankduncan.com/engineering/2025-11-27-crdt-dictionary/>,
  <https://interjectedfuture.com/trade-offs-between-different-crdts/>, <https://crdt.tech/glossary>,
  <https://crdt.tech/resources>, <https://simongui.github.io/distributed-systems/crdt.html>,
  <https://www.inkandswitch.com/essay/local-first/>.

**Clocks & LWW**
- <https://riak.com/clocks-are-bad-or-welcome-to-distributed-systems/>,
  <https://mehedees.medium.com/when-quorum-isnt-enough-how-distributed-clock-skew-silently-discards-data-mehedee-siddique-56b22d0dbd31>,
  <https://medium.com/@truongtud90/the-hidden-pitfall-of-timestamps-in-distributed-systems-a-spring-boot-guide-fba77b4cf798>,
  <https://www.numberanalytics.com/blog/last-writer-wins-distributed-systems>,
  <https://oneuptime.com/blog/post/2026-01-30-last-write-wins/view>,
  <https://manthanguptaa.in/posts/replication_in_distributed_systems_part_3/>.
- <https://snormore.dev/blog/logical-clocks-in-distributed-systems/>,
  <https://dev.to/santosh_koti/distributed-systems-lamport-clock-vs-hybrid-logical-clocks-aj1>,
  <https://sergeiturukin.com/2017/06/26/hybrid-logical-clocks.html>,
  <https://www.geeksforgeeks.org/distributed-systems/logical-clock-in-distributed-system/>.

**Tombstones & GC**
- <https://en.wikipedia.org/wiki/Tombstone_(data_store)>, <https://grokipedia.com/page/Tombstone_(data_store)>,
  <https://www.scylladb.com/2022/06/30/preventing-data-resurrection-with-repair-based-tombstone-garbage-collection/>,
  <https://www.compactionstrategy.org/cassandra-architecture-compaction-fundamentals/tombstone-management-garbage-collection/>,
  <https://oneuptime.com/blog/post/2026-01-26-cassandra-tombstones/view>,
  <https://tdwi.org/blogs/data-101/2026/05/what-is-a-tombstone.aspx>.

**Serialization, CAS, ids**
- <https://datatracker.ietf.org/doc/html/rfc8785>, <https://www.rfc-editor.org/info/rfc8785/>,
  <https://dev.to/lenny321/json-canon-a-strict-rfc-8785-implementation-in-go-for-deterministic-json-3mfg>,
  <https://connect2id.com/blog/how-to-secure-json-objects-with-hmac>.
- <https://jsonlines.org/>, <https://blog.okfn.org/2013/07/02/git-and-github-for-data/>,
  <https://trenta3.gitlab.io/note:storing-sqlite-databases-under-git/>, <https://lobste.rs/s/gnv9ho/tracking_sqlite_database_changes_git>.
- <https://runbook.fyi/Git/object-model/>, <https://grokipedia.com/page/Content-addressable_storage>,
  <https://lab.abilian.com/Tech/Databases%20&%20Persistence/Content%20Addressable%20Storage%20(CAS)/>.
- <https://blog.cuviper.com/2013/11/10/how-short-can-git-abbreviate/>, <https://lwn.net/Articles/1001526/>,
  <https://lkml.iu.edu/hypermail/linux/kernel/2412.0/07364.html>, <https://news.ycombinator.com/item?id=42554420>,
  <https://getsolid.io/blog/birthday-paradox-coding-solid.html>.

**Event sourcing**
- <https://www.conduktor.io/glossary/event-sourcing-patterns-with-kafka>,
  <https://medium.com/towardsdev/event-sourcing-and-log-compaction-3959cba0cda4>, <https://blog.n8n.io/event-sourcing/>,
  <https://www.graphapp.ai/blog/understanding-the-event-sourcing-pattern-a-comprehensive-guide>.

**JSON merge driver implementations**
- <https://github.com/rmedaer/git-merge-drivers>, <https://github.com/jonatanpedersen/git-json-merge>,
  <https://github.com/fcostin/jsonmerge_git_merge_driver>, <https://github.com/kettek/git-json-intellimerge>,
  <https://github.com/balbuf/composer-git-merge-driver/blob/master/README.md>,
  <https://github.com/charpeni/merge-drivers-cli>, <https://www.gregmicek.com/software-coding/2020/01/13/how-to-write-a-custom-git-merge-driver/>,
  <https://www.julianburr.de/til/custom-git-merge-drivers>, <https://graphite.com/guides/git-merge-driver>.
