# Related Trackers: Distributed / Git-Native / Embedded Issue Tracking as Prior Art

Research target: comparative prior art for **obr** (`beads_rust`, binary `br`) — a Rust port of Steve
Yegge's Go tracker `beads`/`bd`, using SQLite for speed + JSONL-in-git for sync, content-addressed
SHA-256 dedup, a 4-step mutation protocol, and a strict "never runs git itself" rule.

Every nontrivial claim below carries its source URL. Where a source was unreachable or silent on a
point, that is stated explicitly rather than guessed.

**Date of research:** 2026-08-06. Repo liveness figures came from the GitHub REST API on that date.

---

## 0. Liveness snapshot (hard data, GitHub API, 2026-08-06)

Queried via `gh api repos/<owner>/<repo>`:

| Repo | Last push | Stars | Lang | Read |
|---|---|---|---|---|
| `git-bug/git-bug` | 2026-07-06 | 9964 | Go | **alive, dominant** |
| `jj-vcs/jj` | 2026-08-06 | 30842 | Rust | alive (VCS, not tracker) |
| `radicle-dev/heartwood` | 2026-08-06 | 255 | Rust | alive |
| `schacon/ticgit` | 2026-08-04 | 515 | **Rust** | **revived** (was Ruby, now git-meta) |
| `dspinellis/git-issue` | 2025-10-17 | 883 | Shell | alive |
| `neithernut/git-dit` | 2025-06-02 | 464 | Rust | low activity |
| `google/git-appraise` | 2023-08-12 | 5304 | Go | dormant |
| `jeffWelling/ticgit` | 2014-02-09 | 267 | Ruby | **dead** |
| `sit-fyi/sit` | 2018-12-20 | 556 | Rust | **dead** |

The shape of this table is itself a finding: of ~15 serious attempts over 18 years, exactly one
(git-bug) reached meaningful adoption, and it is the one that shipped **bridges to centralized
trackers**. See §11.

---

## 1. git-bug — operation-log CRDT in git refs

Sources:
- https://github.com/git-bug/git-bug
- https://raw.githubusercontent.com/git-bug/git-bug/master/doc/design/data-model.md
- https://deepwiki.com/git-bug/git-bug
- https://deepwiki.com/git-bug/git-bug/2.3-caching-system
- https://github.com/git-bug/git-bug/releases/tag/v0.8.0
- https://github.com/git-bug/git-bug/issues/178

### 1.1 Storage model

git-bug does **not** store entity snapshots. It stores an ordered series of edit `Operation`s and
"compiles" the current state by replaying them onto an empty state — the design doc explicitly
compares this to **operation-based CRDTs**
(https://raw.githubusercontent.com/git-bug/git-bug/master/doc/design/data-model.md).

Each `Operation` carries:
- a type identifier,
- an author reference (pointing at *another entity* — identity is itself an entity),
- timestamp(s) plus Lamport clock(s),
- operation-specific payload,
- **a random nonce**, present purely to supply entropy because the operation's ID is a hash of its
  content (ibid.).

### 1.2 Serialization into git

- Operations are grouped into an `OperationPack` — "a simple array" representing one edit session,
  with the author stored **once for the whole pack**.
- The pack is serialized as **JSON and stored as a git blob**.
- A git **tree** references that blob under `/ops`, and optionally media blobs under `/media`.
- A git **commit** wraps the tree; successive edits form a commit chain.
- The chain is exposed at `refs/<namespace>/<id>` so ordinary `git push`/`pull` transfers it, and git
  automatically carries any referenced blobs (attachments) along.
  (all from https://raw.githubusercontent.com/git-bug/git-bug/master/doc/design/data-model.md)

DeepWiki confirms the concrete namespace `refs/bugs/` and notes git-bug deliberately keeps issues,
comments and identities out of the working directory
(https://deepwiki.com/git-bug/git-bug).

### 1.3 Logical time — Lamport clocks encoded in tree entry names

The doc states real timestamps cannot be trusted for ordering, because clocks may be inaccurate or
deliberately manipulated. git-bug therefore uses **Lamport logical clocks**:
- `L1 < L2` means L1 causally preceded L2; **equal values mean the edits were concurrent**.
- Each new entry takes a clock one higher than the highest known.
- The first commit of an entity stores both a `create-clock` and an `edit-clock`; later commits carry
  only an edit-clock.
- **The clock values are encoded directly into git tree entry names** (e.g. `create-clock-14`), each
  pointing at an *empty blob* that is reused across entities so no additional data is transferred.
  (https://raw.githubusercontent.com/git-bug/git-bug/master/doc/design/data-model.md)

That last trick is elegant: it smuggles a scalar into git's object graph at zero transfer cost by
abusing tree entry *names* as the payload.

### 1.4 ID scheme

- Operation ID = `hash(json(op))`.
- **Entity ID = hash of its first serialized operation.**
- Displayed truncated to 7 characters, git-style; users may type any unambiguous prefix, and git-bug
  reports ambiguity when a prefix matches several entities.
  (ibid.)

v0.8.0 added `CombinedId`, which combines two partial identifiers so a sub-item (e.g. a specific
comment) inside an entity can be addressed quickly
(https://github.com/git-bug/git-bug/releases/tag/v0.8.0).

### 1.5 Merge / conflict strategy

- Fast-forward pull/push are trivial: the ref just advances.
- Concurrent edits produce a **merge commit**, giving a DAG with one root and possibly several
  branches converging on a single head.
- Deterministic replay is then obtained by:
  1. loading all commits and their `OperationPack`s;
  2. **validating that Lamport clocks respect the DAG** — a parent's clock must be strictly lower
     than its child's; violating commits are **refused/discarded**;
  3. ordering operations by Lamport clock, falling back to **lexicographic order of the
     `OperationPack` ID** when clocks tie (i.e. for genuinely concurrent edits).
- The doc argues the DAG supplies an implicit base ordering refined by logical clocks, that this
  combined with signed commits "limit[s] how this data model can be abused," and that the
  lexicographic tiebreak carries "not much meaning, but... unbiased and hard to abuse."
  (https://raw.githubusercontent.com/git-bug/git-bug/master/doc/design/data-model.md)

**Note the crucial property: there is no such thing as a merge conflict here.** Concurrency is always
resolvable because the merge function is a total, deterministic ordering over an append-only op set.

### 1.6 The v0.8.0 rewrite — why the linear model broke

This is the single most transferable lesson in the whole survey. Per the release notes
(https://github.com/git-bug/git-bug/releases/tag/v0.8.0):

- The old model **enforced a linear operation structure**. It "was working OK with a star topology of
  git remotes (that is, a central repository) but would break down with anything more complicated."
- The new model represents operations as a **DAG** supporting "concurrent editions across machines
  while still supporting proper merge conflict."
- The model became **fully immutable / append-only**, which the notes say is "much more conductive
  with cryptographic signatures and encryption."
- The DAG/merge/signature/integrity logic was generalized into a **reusable `entity` package** so
  that adding a new entity type (bug, board, config) is "nearly as simple as for a CRUD application."
- It was a **breaking change requiring a separate migration tool**, and the legacy identity system
  embedded in bugs was removed.
- Git access moved from shelling out to the `git` binary to the **`go-git` library**, and the
  `Repository` layer was reworked for modularity and pluggable storage backends.

### 1.7 The cache layer — closest structural analogue to obr's SQLite

Per https://deepwiki.com/git-bug/git-bug/2.3-caching-system:

- Cached **excerpts** (lightweight metadata) are persisted as Go **`.gob` binary blobs** under
  `cache/[namespace]` in local storage — i.e. *derived, disposable, never synced*.
- A **`lock` file containing the PID** prevents concurrent git-bug processes on one repo.
- Staleness is detected two ways: (a) a **`formatVersion` constant** (currently 4) mismatch, and
  (b) matching the **Bleve search index document count against the excerpt count**.
- On "missing, broken, or outdated," `buildCache` **reads all entities from git and regenerates
  excerpts and search indexes** from scratch.
- Two-tier: excerpts always in memory for listing/filtering; full entities loaded on demand behind an
  **LRU capped at a default of 1000 bugs**.
- Filtering runs against excerpts only via a compiled `Matcher`, deferring full loads.
- Cache-layer wrappers around `Pull`, `Push`, `MergeAll` keep cache consistent, with **identity
  merges sequenced before bug merges** due to dependency ordering.

### 1.8 Known limitation

`git-bug` refuses to run in a bare repository — "git-bug must be run from within a git repo" — even
though `git log` and `tig` work fine against bare clones
(https://github.com/git-bug/git-bug/issues/178). Since essentially all server-side git hosting uses
bare repos, this blocks server-side integration.

---

## 2. Fossil SCM tickets — artifact replay, no branching, clock-dependent

Source: https://fossil-scm.org/home/doc/trunk/www/bugtheory.wiki (and the tip mirror
https://fossil-scm.org/home/doc/tip/www/bugtheory.wiki)

### 2.1 Storage model

A Fossil repository is "an unordered collection of *artifacts*." Tickets are a dedicated artifact
type — **Ticket Change Artifacts** — not ordinary versioned files. A ticket is never one object:
"A ticket is created by a ticket change artifact" and "each subsequent modification of the ticket is
a separate artifact."

Each artifact carries a timestamp, the ticket ID, and one or more **name/value pairs**.

### 2.2 ID scheme

"The ticket ID is a random 40-character lower-case hexadecimal number," generated from "a good source
of randomness to insure uniqueness." Random rather than sequential precisely because multiple people
may create tickets offline simultaneously.

### 2.3 State reconstruction & merge strategy

- "The current state of a ticket is found by replaying all ticket change artifacts with the same
  ticket ID **in timestamp order**."
- Fields start NULL and are **replaced or appended** per a flag on each name/value pair.
- Amending writes a **new artifact containing only the changed fields**; untouched fields are omitted.
- Sync treats ticket artifacts like any other artifact — "the sync algorithm has no knowledge of the
  meaning of the artifacts it is syncing."
- Consequently "independently entered changes are automatically merged together when artifacts are
  shared between repositories," and — the key line — **"Tickets do not branch."**

### 2.4 The Achilles heel: wall-clock ordering

Because replay is timestamp-ordered, a timestamp "off by months or years... can seriously confuse the
replay algorithm." Fossil's answer is (a) shunning — "there are techniques available to cause rogue
artifacts to be ignored by fossil" — and (b) "take steps to insure that timestamps are approximately
correct in the first place" (ibid.).

Operationally, Fossil warns when client/server clocks disagree by more than 10 seconds and there is
no way to relax it; Richard Hipp defended this bluntly: "Timestamps do matter, a lot"
(https://fossil-scm.org/forum/forumpost/cff69242dad3e384e2dec6f3493b558de90c80d241bd254aed01c7447520ef5a).

**This is the exact failure mode git-bug's Lamport clocks were designed to avoid.** Fossil is the
control group proving wall-clock ordering is fragile.

### 2.5 Global vs local state — direct analogue of obr's JSONL vs SQLite split

Fossil explicitly separates:
- **Global state (synced):** the raw ticket change artifacts and their name/value pairs.
- **Local state (not synced):** the `TICKET` **database table schema** and the TH1-scripted HTML
  pages that create/view/edit tickets.

Because of this split, "columns can be added or removed from the TICKET table at any time,"
triggering **an automatic replay to repopulate data under the new schema** (ibid.).

That is precisely the property obr wants: the JSONL is the record of truth; the SQLite schema is a
local projection that can be changed freely and rebuilt by replay.

### 2.6 Rationale for NOT storing tickets as versioned files

The doc explicitly rejects the file-based approach for three reasons (ibid.):
1. **Immutability conflict** — "check-ins in fossil are immutable," so a bug found after a check-in
   could never be attached to it.
2. **Clutter** — large projects have "thousands and thousands of tickets," which would pollute the
   source tree.
3. **Permission separation** — ticket creation/editing is meant to be governed by a *different*
   permission model from code commits, so that "casual passers-by on the internet" can file tickets
   without check-in access or a local Fossil install.

Reason 3 is the drive-by-contributor problem (§11) solved at the design level rather than lamented.

---

## 3. SIT / Serverless Information Tracker — content-addressed records, Rust, dead

Sources:
- https://raw.githubusercontent.com/sit-fyi/sit/master/README.md
- https://raw.githubusercontent.com/sit-fyi/sit/master/doc/architecture_overview.md
- https://github.com/sit-fyi/issue-tracking
- https://www.linuxjournal.com/content/foss-project-spotlight-sit-serverless-information-tracker

By Yurii Rashkovskii. **Rust**, and the closest philosophical ancestor to obr's content-addressed
dedup. Last push 2018-12-20 (GitHub API) — dead.

### 3.1 Model

- **Repository** = "a collection of items," in a `.sit` directory found by scanning upward. Config in
  `config.json`; the stated philosophy is that all configurable items should be explicit "to avoid
  potential breakage of behaviour."
- **Item** = "a uniquely named directory within a repository," named by a generated GUID (**UUID by
  default**), manual naming permitted. "Each item is comprised of zero or more records."
- **Record** = "an immutable collection of files," identified by a **deterministic hash of its
  content**, computed as: "for each file, hash relative file name and then hash its content to get a
  cumulative hash." On disk a record is "a directory named after its deterministic hash"
  (Base32-encoded by default).
  (https://raw.githubusercontent.com/sit-fyi/sit/master/doc/architecture_overview.md)

Record contents:
- `.type/TYPE` — one or more marker files; "unknown types must be ignored" and "more than one entry
  is allowed."
- `.prev/<record-id>` — **empty marker files** forming the parent links.
- `.timestamp` (ISO-8601 UTC), `.authors` (one per line), `.signature` (PGP over the record hash).

### 3.2 The DAG and merge

"A record can be linked to more than one previous record, effectively 'joining' them," which "allows
to establish non-exclusive ordering of records and allow records to be prepared independently."

Merge is **structural, not content-based**: "By convention, if there is more than one of the last
records, when a new record needs to be added, it is appended to all of them." Concurrent branches are
joined by the *next* event rather than by any merge algorithm.

### 3.3 The direct critique of flat append-only files (**most important quote for obr**)

The architecture doc states: "The directory layout described is chosen in favour of **plain text
append-only files** for two reasons":
1. "It's far more merge-friendly (one wouldn't incur merge conflicts)"
2. "It's an easier mechanism for managing record's supplemental files"
(https://raw.githubusercontent.com/sit-fyi/sit/master/doc/architecture_overview.md)

This is a deliberate, reasoned rejection of exactly the JSONL-in-git shape obr uses. It should be
read as the strongest available argument against obr's storage choice, and answered rather than
ignored — see §12.2 for why obr can plausibly answer it.

### 3.4 Reducers — fold to current state

`Reducer(State, Item) -> State1`. Each reducer takes an accumulated JSON state plus a record and
returns updated JSON. Iterating records in order (optionally stopping early) reconstructs state at any
point in history; `sit reduce <item id>` gives the present. Custom reducers live at
`.sit/reducers/FILENAME.js` exporting `function(state, record) { return newState; }`, executed with a
bound `this` allowing state to persist across invocations per item. (ibid.)

### 3.5 Append-only enforced at the transport layer

The issue-tracking module dogfooded SIT with GitHub Issues turned off. Contribution flow: create a
branch, add new files under `.sit`, commit, and send via `git send-email` to an Inbox. "**If the
commit only contains new records (nothing else is permitted!) the Inbox will accept the push**" and
forwards it; otherwise the push is rejected
(https://github.com/sit-fyi/issue-tracking).

A machine-checkable append-only invariant enforced at ingress is a cheap, powerful idea directly
portable to obr.

### 3.6 Why it died

No post-mortem was published that I could find. Evidence available: the README self-describes as "in
the early adopter stage. It's usable but not everything is done yet and some things will change"
(https://raw.githubusercontent.com/sit-fyi/sit/master/README.md), and the repo stopped in Dec 2018
with 556 stars and 0 open issues. The scope also drifted — "originally 'IT' stood for issue tracking"
but it "grew to become a generalized information tracking tool" (ibid.), which is a classic
generality-before-adoption failure pattern. I did not find a source stating the cause explicitly, so
this is inference, flagged as such.

---

## 4. git-appraise — JSON-lines in git-notes, merged by `cat_sort_uniq`

Sources:
- https://github.com/google/git-appraise
- https://raw.githubusercontent.com/google/git-appraise/master/README.md
- https://deepwiki.com/google/git-appraise

**This is the closest prior art to "JSONL in git" and the single most directly actionable source in
this survey.**

### 4.1 The format and why it is shaped that way

"Each item stored is written as a single line of JSON, and is written with at most one such item per
line." The stated reason is explicit: "This allows the git notes to be automatically merged using the
**`cat_sort_uniq`** strategy"
(https://raw.githubusercontent.com/google/git-appraise/master/README.md).

`cat_sort_uniq` is git-notes' built-in merge strategy: concatenate both sides, sort, deduplicate.
Because each record is one self-contained, order-independent, byte-identical-when-equal line, the
merge is a **set union with dedup** and is therefore commutative, associative and idempotent — a
join-semilattice, i.e. a state-based CRDT obtained for free from git's own tooling. No custom merge
driver, no conflict markers, no manual resolution.

**The line format is not a serialization convenience; it is the merge algorithm.**

### 4.2 Namespaces

All refs are prefixed `refs/notes/devtools` because "these are meant to be read and written by
automated tools" and "these notes are not in a human-friendly form" (ibid.). Four refs:

| Ref | Content | Annotates |
|---|---|---|
| `refs/notes/devtools/reviews` | review requests (request schema) | first revision in review |
| `refs/notes/devtools/discuss` | human-written comments (comment schema) | first revision in review |
| `refs/notes/devtools/ci` | CI build/test results (ci schema: `timestamp`, `url`, `status` ∈ {null, success, failure}, `agent`, `v`) | the revision built/tested |
| `refs/notes/devtools/analyses` | robot comments from static analysis (analysis schema) | the revision |

(README + https://deepwiki.com/google/git-appraise)

### 4.3 Versioning

"When a field named `v` appears in one of these notes, it is used to denote the version of the
metadata format being used. If that field is missing, then it defaults to the value 0, which
corresponds to this initial version of the formats." Every JSON structure includes `v` to support
future format changes (ibid.).

A per-record format version with an implicit default of 0 is exactly the right cheap forward-compat
mechanism for a line-oriented log.

### 4.4 Resolving "last writer" semantics on top of a set

Since union-merge means multiple review requests accumulate for one commit, git-appraise defines a
deterministic reduction: requests "are sorted by timestamp and the final request is treated as the
current one," done **stably**, so "if there are multiple requests with the same timestamp, then the
last such request in the note is treated as the current one." This is what lets re-running
`git appraise request` update a review rather than create ambiguity (README).

Note the layering, which is the key architectural idea:
**union-merge the log (conflict-free) → deterministically reduce the multiset to current state.**
Exactly the pattern obr needs between JSONL and SQLite.

### 4.5 Status

Last push 2023-08-12 (GitHub API); 5304 stars. Dormant but not archived. Google-internal tooling of
this shape survived; the public tool stalled.

---

## 5. Radicle Collaborative Objects (COBs) — signed CRDT DAG + SQLite materialization

Sources:
- https://deepwiki.com/radicle-dev/heartwood/6.1-collaborative-objects-(cobs)
- https://hackmd.io/@radicle/rJ2UH54P6 (Radicle Protocol Overview, Heartwood)
- https://hackmd.io/@radicle/SJGhA3eUT

Note: `https://radicle.xyz/guides/protocol` 307-redirects to `https://radicle.dev/guides/protocol`,
which returned **HTTP 403** to WebFetch. That page was not read; the DeepWiki and HackMD sources were
used instead.

### 5.1 Model

Each COB is "a cryptographically signed, append-only log represented as a **directed acyclic graph
(DAG)** that uses CRDT... semantics to merge concurrent updates"
(https://deepwiki.com/radicle-dev/heartwood/6.1-collaborative-objects-(cobs)).

Stated properties: decentralized (no central authority, each peer keeps its own view), conflict-free
(CRDT semantics ensure deterministic merging), signed (all operations cryptographically signed by
their authors).

The CRDTs are "inspired by Ink & Switch's Automerge JavaScript library yet implemented in Rust"
(https://hackmd.io/@radicle/rJ2UH54P6).

### 5.2 Git layout and IDs

- Refs: `refs/cobs/<type>/<object-id>`, with types in **reverse-DNS notation**: `xyz.radicle.issue`,
  `xyz.radicle.patch`, `xyz.radicle.id`. Custom COB types are supported.
- "Each operation is a **git commit whose tree contains the operation's manifest and signature**."
- `ObjectId` is "the unique hash identifying a COB, **derived from the root operation**" — same
  first-op-hash-as-entity-id scheme as git-bug and git-dit.
- "Each modification is stored as a separate Git object to ensure that the CRDT change graph is
  compatible with Git's synchronization processes."
  (DeepWiki + HackMD)

### 5.3 Mutation lifecycle — compare to obr's 4-step mutation protocol

Three steps (DeepWiki):
1. **Open** a `Transaction` against a target COB.
2. **Push** one or more `Action`s into it.
3. **Commit**, which: "verifies that only one action produces an identifier (to avoid ambiguity),"
   serializes the actions, **signs** the payload with the peer's `Signer`, and writes the commit.

Core entities: `ObjectId`, `Store` (git persistence), `Op` (one DAG node), `Action` (domain payload),
`Transaction` (staging area batching actions into one signed operation).

The "only one action per transaction may mint an identifier" invariant is a neat, cheap guard against
ID ambiguity in a batched mutation — directly relevant to obr's batched `mutate()`.

### 5.4 SQLite materialization — the strongest validation of obr's hybrid

Per DeepWiki, "to avoid reprocessing full Git history on each access, **a SQLite-based cache
materializes current COB state**," with this data flow:
1. Node fetches new refs from a peer.
2. Storage layer detects changes under `refs/cobs/*`.
3. COB state is reconstructed from git history via a `from_history` function.
4. Materialized state is persisted to SQLite (schema defined via a migration file).

So a serious, actively-developed, funded protocol independently arrived at **git-as-truth +
SQLite-as-materialized-view + replay-on-ref-change**. That is obr's architecture with JSONL swapped
for git objects.

### 5.5 Concrete COB types

- **Issue**: actions `Comment`, `Edit`, `Label`, `Assign`, `Lifecycle` (open/close); state accumulates
  into a discussion `Thread`.
- **Patch**: `RevisionId` for code versions, `ReviewId` for verdicts/comments; states Open, Draft,
  Archived, Merged.
- **Identity**: manages delegate lists and repo metadata with **threshold/quorum-based** acceptance
  (`Revision`, `RevisionAccept`, `RevisionReject`).
  (DeepWiki)

### 5.6 Gaps in the sources

DeepWiki explicitly does **not** cover: the exact serialization format of actions (CBOR vs JSON), the
concrete CRDT merge/ordering algorithm for concurrent branches (e.g. whether there is a timestamp or
lexical tiebreak analogous to git-bug's), the signature scheme beyond a generic `Signer`, or the full
manifest structure. Those would require reading
`crates/radicle-cob/src/change_graph.rs` and `crates/radicle/src/cob/op.rs`. **Not established.**

---

## 6. Jujutsu — first-class conflicts as an algebra (the deepest idea here)

Sources:
- https://docs.jj-vcs.dev/latest/conflicts/
- https://docs.jj-vcs.dev/latest/technical/conflicts/
- https://github.com/martinvonz/jj/blob/main/docs/conflicts.md
- https://v5.chriskrycho.com/journal/deferred-conflict-resolution-in-jujutsu/
- https://neugierig.org/software/blog/2024/12/jujutsu.html

jj is not a tracker, but its conflict model is the most transferable *idea* in this survey.

### 6.1 Conflicts are recorded, not raised

"Unlike most other VCSs, Jujutsu can record conflicted states in commits... if you rebase a commit and
it results in a conflict, the conflict will be recorded in the rebased commit and the rebase operation
will **succeed**. You can then resolve the conflict whenever you want"
(https://github.com/martinvonz/jj/blob/main/docs/conflicts.md).

Conflicted states can themselves be further rebased, merged, or backed out.

### 6.2 Storage vs materialization

Commits store "a **logical** representation of the conflict, not conflict *markers*." Markers are
generated only when a conflict is **materialized** — on checkout (`jj new`/`jj edit`) or in diff
output (`jj show`) (https://docs.jj-vcs.dev/latest/conflicts/).

### 6.3 The algebra (from the technical doc)

A conflicted commit links **an ordered list of tree objects, always odd in length**. The first tree is
a base; subsequent trees form add/subtract pairs. For trees A, B, C, D, E the content is:

```
A + (C − B) + (E − D)
```

A plain three-way merge is just the minimal case: "A three-way merge between A and C with B as base
can be represented as a commit with trees A, B, and C, also known as **A+(C−B)**." Ordinary merges
and N-way conflicts share one representation
(https://docs.jj-vcs.dev/latest/technical/conflicts/).

### 6.4 Why rebasing a conflict does not nest

Conflict expressions **compose and then cancel**. Implemented as `Merge::flatten()` and
`Merge::simplify()`: "if one term in a 3-way merge expression is itself a conflict, we simply insert
the conflict expression there. Then we simplify by removing canceling terms."

Worked examples from the doc:
- Rebase-of-a-rebase: B (based on A) rebased onto C gives `C+(B−A)`; rebasing that onto D gives
  `D+((C+(B−A))−C)`, which **simplifies to `D+(B−A)`** — "a regular 3-way merge between D and B with
  A as base (no trace of C)."
- Reverting a conflict: given `E = C+(B−A)`, reverting is `E+(C−E)` =
  `(C+(B−A))+(C−(C+(B−A)))`, which **simplifies to plain `C`** — no conflict at all.

This is why conflicts don't compound: the representation is closed under composition and admits
term cancellation.

### 6.5 Lazy evaluation

Merges are computed on demand and only where needed: on checkout only the differing portions of the
tree need merging; when listing conflicted paths, sub-trees that can be "trivially resolved" by
comparing tree IDs are skipped entirely; the same recursion applies down to file hunks (ibid.).

### 6.6 The one deliberate unsoundness

"When all sides of a conflict make the same change, we automatically consider it resolved to that
value by default." The doc admits this rule "is lossy in terms of conflict algebra" — applying it and
later reversing the rebase can lose information, with a linked real bug as evidence — but it is kept
because it matches user expectations "in the vast majority of cases." Git and Mercurial behave the
same way; **Darcs** treats identical concurrent changes as a conflict requiring explicit resolution
(ibid.).

An explicitly documented, deliberately-chosen lossy shortcut with a linked counterexample is a model
of engineering honesty worth imitating in obr's own docs.

### 6.7 Materialization details

Default "diff" marker style: `<<<<<<<`/`>>>>>>>` delimit, `%%%%%%%` begins a diff section, `+++++++`
begins a snapshot section, `\\\\\\\` continues long labels. Configurable via
`ui.conflict-marker-style` to "snapshot" (full content per side) or "git" (diff3-style, but **only
supports 2-sided conflicts**, falling back to snapshot for 3+ sides). If file content contains lines
resembling markers, jj **lengthens the markers** (e.g. `<<<<<<<<<<<<<<<`) to stay unambiguous. Files
missing a trailing newline are handled by adding a newline internally per term and omitting it from
the final `>>>>>>>` marker (https://docs.jj-vcs.dev/latest/conflicts/).

---

## 7. git-dit — issues as commits, threads by first-parent

Sources:
- https://github.com/neithernut/git-dit
- https://raw.githubusercontent.com/neithernut/git-dit/master/doc/datamodel.md
- https://news.ycombinator.com/item?id=13732598

Rust. "An issue/comment **is** a commit"; the tool avoids "checking files into the repository," and
needs "no additional software... on the server-side" (README).

### 7.1 Layout and IDs

- `refs/dit/<issue-hash>/head`
- `refs/dit/<issue-hash>/leaves/`
- "An issue is identified by the **hash of its initial commit**."
  (https://raw.githubusercontent.com/neithernut/git-dit/master/doc/datamodel.md)

### 7.2 Threading

Modeled like mail threads: each message is its own commit; "other messages refer to the replied
message via their **first parent**." *Additional* parents are allowed for other purposes — bug reports
or cross-references to messages in other issues. Because an issue's initial commit is recognizable by
having a head ref, it "can safely refer to arbitrary commits as parents," including commits in the
project's own history (e.g. a patch set rooted in the codebase, or a bug report pointing at the commit
where the bug was observed). (ibid.)

Using *first-parent for threading, other parents for cross-references* is a genuinely clever
overloading of git's DAG.

### 7.3 Heads, leaves, status

- **Leaves** span the message tree, exist to prevent GC of messages, and "may be removed" when no
  longer needed for that purpose.
- **Head** marks a maintainer-agreed state/status, since "status is also altered through commit
  messages." Issue metadata is assembled by walking from head back to the initial message via first
  parents, accumulating metadata.
- Maintainers may either move the head ref directly, or maintain an independent sequence of
  status-change commits linking to discussion messages via the **second parent**. (ibid.)

### 7.4 Metadata

Git trailers (`man git-interpret-trailers`), currently only `Dit-status` and `Dit-type`; more "may be
provided in the future" (README + datamodel.md).

### 7.5 Stated limitations

From datamodel.md: no garbage collector yet for pruning leaf refs; no tooling yet for managing head
refs ("we do not yet provide tools for managing the head reference of an issue"), meaning **no
automated merge/conflict resolution for concurrent status updates is described**; tools must never
assume an issue's initial message is parentless; trailer tags are minimal. README adds: "WARNING:
This is pre-1.0! Expect bugs and incompatibilities!" and "this is alpha quality — there are bugs,
missing things and rough edges."

Licensing is split: library MPL-2.0, binary GPLv2 (README).

---

## 8. ticgit — died, then came back in Rust with a SQLite index

Sources:
- https://github.com/schacon/ticgit
- https://github.com/jeffWelling/ticgit
- https://matej.ceplovi.cz/blog/current-state-of-the-distributed-issue-tracking.html
- https://lwn.net/Articles/281849/

### 8.1 The original (dead)

Called "the granddaddy of all issues-in-separate-branch systems." Per the 2013 survey, the original
project "declares project as dead," pointing to the jeffWelling fork
(https://matej.ceplovi.cz/blog/current-state-of-the-distributed-issue-tracking.html) — and that fork's
last push was **2014-02-09** (GitHub API). Both are dead.

LWN in 2008 described ticgit as git-specific, adding "a new branch to the SCM and stor[ing] the bug
information there," operating directly on git's object database, and noted the tradeoff: tighter git
integration **sacrifices per-branch bug views**, weakening the bug/fix linkage that Bugs Everywhere
offered (https://lwn.net/Articles/281849/).

### 8.2 The 2026 revival (**most relevant living analogue to obr**)

`schacon/ticgit` is now **Rust**, described as "built on git-meta," last push 2026-08-04, 515 stars
(GitHub API). Per the README (https://github.com/schacon/ticgit):

- "TicGit delegates storage and transfer to **`git-meta-lib`**."
- Exchange between clones happens "through **`refs/meta/*`** using normal Git transfer."
- **Locally, a query database is kept in `.git/git-meta.sqlite`.**
- All data lives on the git-meta `project` target under the `ticgit:` namespace.
- Storage is **typed key paths**, not files: `ticgit:schema-version` (string), `ticgit:owners` (set),
  `ticgit:views:<name>` (set of ticket UUIDs), and per ticket
  `ticgit:tickets:<uuid>:{title,description,status,state,assigned,points,milestone,tags(set),comments(list),created-at,created-by}`.
- **"Ticket existence is implied by the presence of fields under `ticgit:tickets:<uuid>:*`"** — there
  is no separate ticket index.
- IDs are **UUIDs**; "most commands accept a full UUID or any unique UUID prefix," and ambiguous
  prefixes fail with a nonzero exit and a stderr message.
- Sync commands: `ti pull`, `ti push`, `ti sync` (pull then push).

The README does **not** describe a merge/conflict-resolution algorithm — presumably it lives in
git-meta-lib. **Not established from the sources read.**

Two things matter here for obr: (1) an independent 2026 design landed on *git refs for truth + local
SQLite for queries*, same as obr and Radicle; (2) the typed-key-path model with existence implied by
field presence is a CRDT-friendly encoding (a map of LWW registers and grow-sets) that sidesteps
whole-record conflicts entirely.

---

## 9. Bugs Everywhere — files in `.be`, UUIDs, killed by friction

Sources:
- https://lwn.net/Articles/281849/
- https://github.com/aaiyer/bugseverywhere
- https://matej.ceplovi.cz/blog/current-state-of-the-distributed-issue-tracking.html

### 9.1 Model

Creates a `.be` directory in the project root storing bugs as "directories full of text files"
versioned by the underlying SCM (LWN). "Bugs have globally unique ids" (UUIDs), so references stay
consistent across distributed branches without collisions. Works with "Arch, Bazaar, Darcs, Git,
Mercurial, and Monotone... but is easily extensible," and can run "with no VCS at all"
(https://github.com/aaiyer/bugseverywhere).

Its distinguishing benefit: "bugs can be marked 'fixed' in the branches that fix them" (ibid.) — the
per-branch bug view that ticgit gave up.

### 9.2 Why it stalled — LWN's diagnosis (1 May 2008)

All from https://lwn.net/Articles/281849/:
- **Commit tax**: bug changes only become real once committed to the SCM. Fine when pairing a fix with
  a bug update, but for adding comments in an ongoing conversation "the required commit is just more
  work to do."
- **UUID file clutter**: new files get names like
  `12968ab9-5344-4f08-9985-ef31153e504f/comments/...`, which in git must be manually `git add`ed.
- **Merge conflicts on structured fields**: changing severity in two branches and merging produced a
  conflict "resolvable only by hand-editing plain text files" — "less comforting than one might
  think."
- **Split metadata streams**: bug data and SCM commit metadata stay disconnected, with "no way of
  tying those two metadata streams together."

Corbet's conclusion: without better tooling, distributed bug tracking risks becoming "a source of more
work for developers, which is not the path to world domination"; the field was "few of these projects
have achieved a high level of usability"; and any distributed system still needs a **web-facing,
centralized-feeling interface** because most people "are unlikely to be impressed by" being told to
run git commands to file a bug.

The 2013 survey lists Bugs Everywhere as a rare survivor that "shows some signs of life"
(https://matej.ceplovi.cz/blog/current-state-of-the-distributed-issue-tracking.html). By 2026 the
mirror shows minimal activity.

---

## 10. git-issue — plain files, SHA IDs, native git merge, still alive

Source: https://github.com/dspinellis/git-issue

- Everything under `.issues/`, which contains its **own `.git`**, plus `config`, `imports`, `issues`,
  `templates`.
- Each issue is "a directory named `issues/xx/xxxxxxx...`, where the x's are the **SHA of the issue's
  initial commit**."
- Attributes are separate files: `description`, `duedate`, `weight`, `timespent`, `timeestimate`
  (ISO-8601 or seconds), `tags`, `milestone`, `watchers`, `assignee` (often one value per line), plus
  `comments/` and `attachments/`.
- IDs: issues and comments are identified by "the SHA hash associated with the parent of the commit
  that opened them," from which author and date can be derived. Comments are tracked by the SHA of a
  commit containing a marker text tied to the issue's SHA.
- **Merging is entirely delegated to git's native machinery** — no custom conflict resolution.
  `git issue push/pull/clone`, and `git issue git <cmd>` to run arbitrary git commands on the issues
  repo.
- `imports` tracks external data as `import/<provider>/<user>/<repo>/<number>` mapping external issue
  numbers to internal SHAs, plus a **`checkpoint` file recording "the SHA of the last imported or
  updated issue"** for incremental future merges.

Stated advantages: no backend/dependencies ("install and use git issue with a single shell command"),
decentralized/asynchronous, transparent plain-text format ("viewable/editable/backup-able with any
tool you like"), and reuse of existing git credentials/infrastructure with "a solid audit trail."

The README situates git-issue among bug, Bugs Everywhere, deft, Fossil, git-appraise, git-bug, GitHub
CLI, Perceval, and SD, but **lists only its advantages, no disadvantages**.

The per-attribute-file decomposition is notable: it makes git's line-level merge coincidentally
correct for independent field edits, because two people editing `assignee` and `milestone` touch
different files. Field-level granularity is the poor man's CRDT.

---

## 11. What killed the dead ones — synthesized post-mortem

### 11.1 The 2013 field report

Matěj Cepl surveyed the field and opened with: the landscape resembles "the landscape after the
Battle of Verdun," noting "most of the projects mentioned below are defunct and abandoned even by its
original authors" (https://matej.ceplovi.cz/blog/current-state-of-the-distributed-issue-tracking.html).

He identifies two storage camps:
1. **Data as regular repo files** (VCS-independent, hidden directory). Pro: simple; can link tickets
   to specific commits. Example: Bugs Everywhere.
2. **Data in a separate git branch**. Con: not DVCS-independent; "hidden writing to the git branch...
   makes it way more complex to debug." Example: gitissius, ticgit.

He grew "rather unhappy" with camp 1 after people "filed bugs to incorrect branches and completely
loose them" and because "most DITs use plenty of UUIDs and plenty of small files, which then clog all
commit messages."

His status table: Ditz mailing list "(dead)"; ditz-trac "Not working ATM"; ticgit "declares project as
dead"; git-case "dead (last commit is four years old)"; stick's code link "gives 404"; Artemis is a
Mercurial extension that "doesn't work with git"; DisTract filed under "Completely dead projects."

His diagnosis of common causes:
- Poor or absent **import/export with centralized trackers** — "not many DIT deal with well if at all"
- Messy git history from UUID-heavy file storage
- Fragile, hacky integration (Firefox-JS-only tools, 404s)
- Plain abandonment

And a design principle: "issue tracking is something that doesn't have to bother everybody, it should
be **optional** — if I want to track issues I should be able to share my tickets with others, but if a
project has its own upstream tracker, I shouldn't bother them with my mess."

### 11.2 Eric Sink's structural critique

https://ericsink.com/entries/dbts_fossil.html

- The obvious first instinct — bugs as mergeable text files that branch along with the code — is
  likely **wrong**, because "there are just too many scenarios where the bug-tracking info is being
  updated without changing anything in the tree."
- Bug records need "their own place, living in **their own DAG**," separate from the code.
- The **QA scenario**: should a QA member marking a bug "fix verified" really require "a commit to the
  version control tree"? Bug tracking and code likely warrant "a completely different set of access
  permissions."
- Structured bug data deserves **purpose-built merge logic**, not reused text-diff merging.
- Bug databases need **much more frequent syncing** than code — push comments "as soon as my network
  connectivity allows" — unlike the deliberate, sandboxed workflow of code commits.
- On Fossil: credits separating tickets from the version tree as partially right, but critiques
  newest-timestamp-wins as risking user frustration from **unexpected data loss**, and notes Fossil's
  ticket system "isn't DAG-based at all."

### 11.3 The social cause: the drive-by contributor

The recurring, decisive theme across LWN 2008, the 2013 survey, HN threads
(https://news.ycombinator.com/item?id=10905482,
https://news.ycombinator.com/item?id=22831604,
https://news.ycombinator.com/item?id=33730417,
https://news.ycombinator.com/item?id=43971620) and Fossil's own rationale:

A centralized web tracker lets **anyone** file a bug from a browser with zero setup. A distributed
tracker requires cloning the repo, installing a CLI, and pushing a commit. The majority of bug reports
come from casual first-time reporters, so the asymmetry in *reporting friction* — not any technical
deficiency — is what let centralized trackers keep the network effect. LWN said it in 2008: most
people "are unlikely to be impressed by" being told to run git commands to file a bug
(https://lwn.net/Articles/281849/).

Fossil is the only system in this survey that treated this as a **first-class design constraint**,
explicitly separating ticket permissions from check-in permissions so "casual passers-by on the
internet" can file tickets "without requiring check-in access or a local Fossil install"
(https://fossil-scm.org/home/doc/trunk/www/bugtheory.wiki). Fossil's tickets are also the only ones in
this survey still in daily production use by their host project.

git-bug is the only *git-native* survivor, and it is the one that shipped **bridges** to GitHub and
GitLab (https://github.com/git-bug/git-bug). That is not a coincidence — it directly answers Cepl's
"import/export with centralized trackers" failure mode.

### 11.4 Note on obr's position

obr's context differs in one decisive way from every project above: it is an **agent-facing** tracker.
The drive-by human reporter is not its primary user; coding agents are, and agents already have a CLI,
already have the repo cloned, and pay no friction cost for a commit. **The single social force that
killed this entire category does not apply with the same force to obr.** But its corollary does: obr
still needs a story for humans who want to look at issues without installing anything (a rendered
view, a bridge, or a static export), and it needs import/export with GitHub Issues if it is ever to
coexist with an upstream tracker.

---

## 12. Lessons for a SQLite + JSONL-in-git hybrid

### 12.1 The prevailing architecture is exactly obr's — three independent confirmations

| System | Truth | Materialized view | Rebuild trigger |
|---|---|---|---|
| Radicle | git objects under `refs/cobs/*` | **SQLite** (migration-defined schema) | ref change → `from_history` |
| ticgit (2026) | `refs/meta/*` via git-meta-lib | **`.git/git-meta.sqlite`** query DB | not documented in README |
| git-bug | git objects under `refs/bugs/` | `.gob` excerpts + Bleve index in `cache/<ns>` | formatVersion mismatch or index/excerpt count mismatch → `buildCache` |
| Fossil | ticket change artifacts | **SQLite `TICKET` table** | schema change → automatic replay |
| **obr** | **JSONL in git** | **SQLite** | ? |

Sources: §5.4, §8.2, §1.7, §2.5. This convergence is strong evidence the hybrid is right. The
differentiator is what obr uses for truth: **JSONL files in the working tree** rather than git objects
in a side ref namespace. That is the one axis where obr is unusual, and it carries both the biggest
advantage (human/agent-readable, diffable, greppable, no custom git plumbing, honors the "never runs
git" rule) and the biggest risk (§12.2).

### 12.2 JSONL-in-git: how to make union-merge actually work

git-appraise proves line-per-record works, but only because it satisfies specific invariants
(§4.1–4.4). The checklist obr must satisfy for `merge=union` (or `cat_sort_uniq`) semantics to be
correct:

1. **Every line must be a self-contained, order-independent record.** Union merge "can produce
   logically incorrect results if lines from both sides depend on each other"
   (https://tenthirtyam.org/dispatches/2026/04/11/controlling-git-repository-behavior-with-gitattributes/,
   https://github.com/orgs/community/discussions/9288).
2. **Byte-identical serialization for identical records** — canonical key order, no float formatting
   drift, stable whitespace. Union dedup is *textual*: "if the same logical record was added
   independently on both branches with slightly different formatting, you'll get duplicate entries."
   obr's SHA-256 content-addressed dedup is the right second line of defense here, but the
   serializer must be canonical or the git-level dedup silently fails and pushes all the work onto
   the importer.
3. **Never rewrite or delete a line.** Union merge over a mutable file is unsound. Append-only is the
   precondition, and SIT enforced it *at ingress* — "only new records permitted, otherwise the push is
   rejected" (https://github.com/sit-fyi/issue-tracking).
4. **Order must not be load-bearing.** "The resulting order of JSONL records after a merge may not
   match either branch's original order."
5. **Union merge is silent.** It "never surfaces true conflicts for manual review," so semantic
   conflicts must be detected at import time, not merge time.
6. **Hosted platforms may ignore `.gitattributes` merge config.** GitHub "doesn't consider
   user-defined .gitattributes files, normally using its own"
   (https://github.com/isaacs/github/issues/487); GitLab reads `union`/`binary` but "custom merge
   drivers are not used during merge requests since they are defined in the gitconfig"
   (https://gitlab.com/gitlab-org/gitlab-foss/-/issues/17325,
   https://gitlab.com/gitlab-org/gitlab/-/issues/18830). **obr cannot rely on the merge button
   behaving like the CLI.** The importer must be able to repair a badly-merged file (duplicate lines,
   conflict markers) rather than assuming the merge was clean.
7. **`.gitattributes` only applies to merges after it is committed**
   (https://tenthirtyam.org/dispatches/2026/04/11/controlling-git-repository-behavior-with-gitattributes/).

SIT's counter-argument stands and must be answered honestly: it rejected "plain text append-only
files" for directory-per-record because the latter is "far more merge-friendly (one wouldn't incur
merge conflicts)" (§3.3). obr's answer, if it holds, is that a *canonically serialized, append-only,
union-merged, hash-deduplicated* JSONL is merge-conflict-free too — and buys human diffability that
a tree of hash-named directories destroys. But that answer is only true if invariants 1–3 hold
mechanically, not by convention.

### 12.3 Logical clocks beat wall clocks — the Fossil/git-bug natural experiment

Fossil replays "in timestamp order" and its own docs admit a timestamp "off by months or years... can
seriously confuse the replay algorithm," mitigated only by shunning rogue artifacts and by telling
people to fix their clocks (§2.4). git-bug explicitly rejected wall clocks as untrustworthy and used
Lamport clocks with a DAG-causality validation that **discards commits whose clocks violate the
parent<child invariant** (§1.3, §1.5).

For obr: if records carry only wall-clock timestamps and the importer orders by them, obr has
Fossil's failure mode. A per-record monotone counter (Lamport-style), plus a deterministic tiebreak
on content hash for concurrent records, gives total determinism without trusting clocks. git-bug's
tiebreak choice — lexicographic order of the pack ID — is justified as "unbiased and hard to abuse,"
which is exactly the property a content hash has.

### 12.4 Separate the conflict-free log from the deterministic reduction

git-appraise's two-layer design is the cleanest statement of the pattern (§4.4):
**union-merge the log (guaranteed conflict-free) → deterministically reduce the multiset to current
state (timestamp-sorted, stable, last-wins).** SIT's reducers (§3.4), Fossil's field replay (§2.3),
git-bug's operation replay (§1.1) and Radicle's `from_history` (§5.4) are all the same shape.

The corollary for obr: **the SQLite database must be 100% derivable from the JSONL and must be
disposable.** git-bug treats its cache as disposable and rebuilds from git on any version or
consistency mismatch (§1.7). Fossil rebuilds the `TICKET` table by replay whenever the schema changes
(§2.5). If obr's SQLite ever holds state not present in the JSONL, the hybrid is broken and every
merge is a data-loss risk.

Concretely worth stealing from git-bug's cache: a **`formatVersion` constant** that forces a rebuild
on mismatch, and a **cheap consistency check** (git-bug compares search-index doc count against
excerpt count) so corruption is caught rather than served.

### 12.5 Per-field granularity is the poor man's CRDT

Every survivor decomposes state below the record level:
- Fossil: name/value pairs with a replace-or-append flag; unchanged fields are simply **omitted** from
  the amending artifact (§2.3).
- ticgit 2026: typed key paths `ticgit:tickets:<uuid>:<field>`, with sets and lists as distinct types,
  and existence implied by field presence (§8.2).
- git-issue: one file per attribute, so independent field edits touch disjoint files (§10).
- git-bug: one operation per semantic change (`SetTitle`, `AddComment`) (§1.1).

The anti-pattern is Bugs Everywhere, where editing a structured field in two branches produced a text
conflict "resolvable only by hand-editing plain text files" (§9.2).

**For obr: a JSONL line should represent a delta to specific fields, not a full snapshot of the issue.**
Full-snapshot lines make every concurrent edit a semantic conflict that union-merge will happily and
silently resolve wrong (you get two complete, divergent snapshots and must pick one, losing the other
side's field edits). Field-level deltas make concurrent edits to different fields commute for free.
Set-valued fields (labels, assignees, watchers) should use explicit add/remove ops, not
replace-the-whole-list — that is a 2P-set/OR-set and it merges; a replaced list does not.

### 12.6 Entity ID = hash of the creating record

Three independent systems converged: git-bug ("an entity's ID is the hash of its first serialized
operation," §1.4), git-dit ("an issue is identified by the hash of its initial commit," §7.1),
Radicle (`ObjectId` "derived from the root operation," §5.2), and git-issue (directory named by "the
SHA of the issue's initial commit," §10). Fossil instead uses a random 40-hex ID (§2.2); SIT and
ticgit use UUIDs (§3.1, §8.2).

Both families work, and both avoid the sequential-counter trap that cannot survive offline creation.
The hash family gets verification and dedup for free; the random family gets a stable ID before the
content is finalized. **Since obr already does SHA-256 content-addressed dedup, the hash family is the
natural fit** — but note git-bug had to add a **random nonce to every operation** specifically because
the op ID is a content hash and identical operations would otherwise collide
(§1.1). Any content-addressed scheme needs an entropy source, or a deliberate decision that collision
*is* dedup.

Also worth copying: **short-prefix addressing with explicit ambiguity errors.** git-bug displays
7-char prefixes and "will list matches if a prefix is ambiguous" (§1.4); ticgit accepts "any unique
UUID prefix" and fails "with a nonzero exit and stderr message" on ambiguity (§8.2). The nonzero-exit
detail matters for agent consumers.

### 12.7 Make conflicts representable rather than fatal (jujutsu)

jj's insight (§6) is that an operation that *can* conflict should **record the conflict and succeed**,
deferring resolution. For obr this suggests: an import that encounters a semantically irreconcilable
state should not fail the whole sync — it should materialize a *conflicted* issue state that is
queryable (`br list --conflicted`), keep both sides, and let resolution happen later as an ordinary
mutation appended to the log.

The deeper structural lesson is jj's **simplification property**: because conflicts are represented
algebraically and redundant terms cancel (`D+((C+(B−A))−C)` → `D+(B−A)`, §6.4), repeated
rebases/merges do not compound complexity. Any conflict representation obr adopts should have the
analogous property — resyncing a conflicted issue repeatedly must not accumulate nested conflict
records.

And jj's honesty is worth imitating: the same-change auto-resolution rule is documented as "lossy in
terms of conflict algebra" with a linked real bug, kept anyway because it matches expectations (§6.6).

### 12.8 Never trust the transport to preserve invariants — validate at ingest

git-bug validates Lamport clocks against DAG structure and **discards** violating commits (§1.5).
Radicle's transaction commit "verifies that only one action produces an identifier (to avoid
ambiguity)" (§5.3). SIT's inbox rejects any push containing anything but new records (§3.5). Fossil
provides shunning for rogue artifacts (§2.4).

Given §12.2's finding that hosted merge buttons may not honor obr's merge configuration, **the JSONL
importer is obr's only real enforcement point** and should be treated as a validating parser:
reject/quarantine malformed lines, detect and drop exact duplicates by hash, detect conflict markers
left by a bad merge, and verify any ordering invariants — reporting rather than silently absorbing.

### 12.9 Survival requires bridges, not purity

git-bug is the only git-native tracker with real adoption and the only one with GitHub/GitLab bridges
(§1, §11.3). Cepl named absent import/export with centralized trackers as a leading cause of death
(§11.1). git-issue ships an `imports/` directory with a **`checkpoint` file recording the SHA of the
last imported issue** for incremental re-merges (§10) — a small, concrete, copyable design.

### 12.10 Miscellaneous concrete borrowings

- **Per-record format version field with implicit default.** git-appraise's `v`, absent ⇒ 0 (§4.3).
  Cheap forward-compat for an append-only log where old lines must remain parseable forever.
- **Machine-facing namespaces should say so.** git-appraise prefixes everything `refs/notes/devtools`
  because "these are meant to be read and written by automated tools" (§4.2).
- **Bounded memory with LRU over a lightweight always-resident index.** git-bug keeps excerpts in
  memory for filtering and LRU-caches at most ~1000 full entities (§1.7). For obr, SQLite handles this
  natively, but the principle — filter on a narrow projection, hydrate lazily — applies to the JSON
  output path.
- **PID lock file to prevent concurrent processes** on one repo (§1.7). Relevant given obr's agent
  users may run several `br` invocations concurrently.
- **Ordering dependency between entity kinds during merge**: git-bug merges identities *before* bugs
  (§1.7). obr likely has an analogous ordering (issues before dependencies before comments/events).
- **Bare-repo support.** git-bug's inability to run in a bare repo blocks all server-side use
  (§1.8). If obr ever wants a server-side render or a CI check, working against a bare clone should be
  designed in rather than retrofitted.
- **Don't over-generalize before adoption.** SIT drifted from "issue tracking" to "generalized
  information tracking" and died (§3.6). git-bug generalized to an `entity` framework only *after*
  the bug use case worked (§1.6). Sequence matters.

---

## 13. Sources that could not be read

- `https://radicle.dev/guides/protocol` — **HTTP 403**. Radicle COB details were taken from DeepWiki
  and HackMD instead; the precise CRDT ordering algorithm and serialization format remain
  **unestablished** (§5.6).
- `https://raw.githubusercontent.com/google/git-appraise/master/schema/README.md` and
  `https://github.com/google/git-appraise/blob/master/docs/metadata.md` — **HTTP 404**. Confirmed via
  `gh api` that the repo contains only `README.md`, `CONTRIBUTING.md` and `docs/tutorial.md`; the
  metadata/schema documentation lives in the README, which was read (§4).
- git-bug's `doc/design/data-model.md` was fetched successfully but the fetcher declined to reproduce
  it verbatim; the content above is a detailed paraphrase with short quotes, from two independent
  fetches (github.com blob view and raw.githubusercontent.com) that agreed.
- No published post-mortem was found for SIT; §3.6's causal reasoning is explicitly labeled inference.
- ticgit 2026's merge/conflict algorithm is not documented in its README and lives in `git-meta-lib`,
  which was not read. **Unestablished** (§8.2).
