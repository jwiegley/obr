# Web research: Modern Rust CLI engineering norms (benchmark for `obr`)

Agent: `rust-cli-norms` (web research)
Date of research: 2026-08-06
Scope: clig.dev, clap derive practice, error-reporting UX (thiserror/anyhow/miette, exit codes,
machine-readable errors), structured/JSON output conventions for scripts *and AI agents* (2025–2026
discourse), CLI testing practice (assert_cmd / trycmd / snapbox / insta / proptest / cargo-fuzz),
MSRV vs nightly tradeoffs, and `#![forbid(unsafe_code)]` + clippy pedantic/nursery norms.

Method note: every substantive claim below carries the source URL it came from. Where a source was
reachable only as an index page (no substantive body), that is stated explicitly rather than
guessed. Two URLs I guessed for Rain's book (`error-handling-and-exit-codes.html`,
`exit-codes.html`) returned **HTTP 404** and the book's `print.html` served only chapter titles for
several chapters, so **I could not read Rain's "Error handling and exit codes" chapter** — that gap
is called out in §4.5.

A small amount of read-only grounding in `/Users/johnw/src/obr` is included in §11 so the
recommendations are concrete. The repo was not modified.

---

## 1. clig.dev — Command Line Interface Guidelines

Primary source: <https://clig.dev/> (fetched in full). Repo: <https://github.com/cli-guidelines/cli-guidelines>.
Authored by Aanand Prasad and collaborators; CC-BY-SA-4.0; ~3.2k stars; content lives in a single
`content/_index.md` built with Hugo
(<https://github.com/cli-guidelines/cli-guidelines/blob/main/README.md>).

### 1.1 The normative rules that matter most for `obr`

From <https://clig.dev/>:

**Basics**
- "Return zero exit code on success, non-zero on failure." Map non-zero codes to important failure modes.
- "Send output to `stdout`." Machine-readable output belongs on stdout, since piping uses it by default.
- "Send messaging to `stderr`." Logs and errors go here so piped commands don't ingest them.
- Use an argument-parsing library rather than rolling your own.

**Output**
- "Human-readable output is paramount," with TTY detection as the heuristic for who is reading.
- "If human-readable output breaks machine-readable output, use `--plain` to display output in
  plain, tabular text format for integration with tools like `grep` or `awk`."
- "Display output as formatted JSON if `--json` is passed."
- "Display output on success, but keep it brief"; offer `-q` for suppression.
- "If you change state, tell the user."
- "Use color with intention." Disable color when output is not a TTY, when `NO_COLOR` is set, when
  `TERM=dumb`, or via `--no-color`.
- "If `stdout` is not an interactive terminal, don't display any animations."
- "Don't treat `stderr` like a log file, at least not by default" — no `ERR`/`WARN` level labels
  unless verbose.
- Use a pager (`less -FIRX`) for long output, "only if `stdin` or `stdout` is an interactive terminal."

**Errors**
- "Catch errors and rewrite them for humans."
- "Signal-to-noise ratio is crucial" — group repeated errors instead of listing duplicates.
- For unexplainable errors, "provide debug and traceback information, and instructions on how to
  submit a bug."

**Arguments and flags**
- "Prefer flags to args."
- "Have full-length versions of all flags."
- "Only use one-letter flags for commonly used flags."
- "If you've got two or more arguments for different things, you're probably doing something wrong."
- "Use standard names for flags, if there is a standard" — explicitly lists `-f`/`--force`,
  `-n`/`--dry-run`, `--json`.
- "Prompt for user input" when args are missing, but "Never *require* a prompt."
- "Confirm before doing anything dangerous," scaling friction to severity.
- "If input or output is a file, support `-` to read from `stdin` or write to `stdout`."
- "Do not read secrets directly from flags."

**Interactivity**
- "Only use prompts or interactive elements if `stdin` is an interactive terminal (a TTY)."
- "If `--no-input` is passed, don't prompt or do anything interactive."
- "Let the user escape" — Ctrl-C must work during long operations.

**Subcommands**
- "Be consistent across subcommands" in flag names and output formatting.
- "Use consistent names for multiple levels of subcommand," commonly `noun verb`.
- "Don't have ambiguous or similarly-named commands."

**Robustness**
- "Validate user input" early.
- "Responsive is more important than fast" — "Print something to the user in <100ms."
- "Show progress if something takes a long time."
- "Make things time out," with configurable defaults.
- "Make it recoverable"; "Make it crash-only."

**Future-proofing** (directly relevant to a tool whose JSON is consumed by agents)
- "Keep changes additive where you can."
- "Warn before you make a non-additive change."
- "Changing output for humans is usually OK," but push scripts to `--plain`/`--json` for stability.
- "Don't have a catch-all subcommand."
- "Don't allow arbitrary abbreviations of subcommands."
- "Don't create a 'time bomb'."

**Signals**
- "If a user hits Ctrl-C (the INT signal), exit as soon as possible."
- "If a user hits Ctrl-C during clean-up operations that might take a long time, skip them."

**Configuration**
- Flags for per-invocation; flags + env vars for machine-level; version-controlled files for
  project-wide. "Follow the XDG-spec."
- Precedence: flags → env vars → project config → user config → system config.
- "If you automatically modify configuration that is not your program's, ask the user for consent."

**Environment variables**
- Names "must only contain uppercase letters, numbers, and underscores."
- "Avoid commandeering widely used names."
- Check `NO_COLOR`, `DEBUG`, `EDITOR`, proxy vars, `TMPDIR`, `HOME`, `PAGER`.
- "Do not read secrets from environment variables" (leak via `docker inspect`, `systemctl show`,
  process listings).

### 1.2 Status of clig.dev on the agent question (contested / unsettled)

clig.dev has **not** (as of this research) merged a dedicated "AI agents / LLM" prose chapter. What
the maintainers did add is machine-readability infrastructure for the *guidelines site itself*: a
Cloudflare `_headers` rule serving `/llms.txt` as `text/markdown`, plus a URL-rewrite rule serving
`/llms.txt` when `Accept: text/markdown` is sent to `/`
(<https://github.com/cli-guidelines/cli-guidelines>, README build/deploy section). Last repo update
observed around 2025-08-01 (<https://github.com/cli-guidelines/cli-guidelines>).

**Implication:** the canonical human-CLI guideline set is stable and human-first; the agent-specific
norms (§3) are being developed *outside* clig.dev by vendors and independent writers, and they
partially **contradict** clig.dev (see §3.6).

---

## 2. clap derive: current best practice

### 2.1 clap's own position

From clap's FAQ (<https://docs.rs/clap/latest/clap/_faq/index.html>):
- clap's "default answer is to use the Derive API," because it is "Easier to read, write, and
  modify," "Easier to keep the argument declaration and reading of argument in sync," and "Easier to
  reuse, e.g. clap-verbosity-flag."
- The Builder API is "a lower-level API" that buys "Faster compile times if you aren't already using
  other procedural macros" and lets you inspect an "argument's values," their "ordering with other
  arguments," and "what set them" — data the Derive API cannot expose.
- The two APIs interoperate in the same project.
- "clap is structopt" — structopt's derive was merged in v3; structopt is in maintenance mode.
- Performance: despite its features clap "remains about as fast as getopts."
- Self-criticism: clap "is also opinionated about parsing"; "can be very verbose"; "learning
  everything can seem overwhelming."
- YAML config, the docopt-style usage parser, and `clap_app!` are historical and no longer the
  recommended path.

Derive tutorial / reference: <https://docs.rs/clap/latest/clap/_derive/_tutorial/index.html> and
<https://docs.rs/clap/latest/clap/_derive/index.html>. Key mechanics: subcommands derive
`#[derive(Subcommand)]` and attach via `#[command(subcommand)]`; subcommand args can be
struct-variants or auto-flattened tuple-variants; a field without attributes is a positional whose
behavior is inferred from its type; raw attributes forward directly to the builder, so any
`Command`/`Arg`/`PossibleValue` method is usable as an attribute.

### 2.2 Rain's Rust CLI recommendations (the most-cited opinionated Rust-specific source)

Book: <https://rust-cli-recommendations.sunshowers.io/>. Repo:
<https://github.com/sunshowers-code/rust-cli-recommendations>. Prose CC BY 4.0; snippets CC0.
The book positions itself as "more advanced material" and "more opinionated overall" than the Rust
CLI Book, and uses RFC-2119-style *must*/*should* keywords.

Picking a parser (<https://rust-cli-recommendations.sunshowers.io/cli-parser.html>):
- "projects *should* use **clap**" — most popular, follows "standard conventions for Unix CLIs"
  (contrasted with `argh`, which targets Fuchsia conventions).
- Costs: clap "pulls in several dependencies and takes quite a while to build" and "increases binary
  size significantly."
- Derive pros: "Derive-style arguments are significantly easier to read, write, and modify."
  Derive cons: extra deps, "magical" (use `cargo-expand`), "less flexible than the builder API" —
  notably only the builder can report the exact position of each repeated flag occurrence.
- Naming: commands and multi-word args "must be in lowercase" with hyphens, not underscores or
  camelCase (`--example-opt`, never `--example_opt`).
- "You *should not* write your own parser completely by hand"; prefer `pico-args`, or base a custom
  parser on `lexopt`.
- Alternatives with tradeoffs: `argh`, `pico-args` ("Zero dependencies, quick to compile", no derive
  or help generation), `gumdrop`.

Structuring arguments and subcommands
(<https://rust-cli-recommendations.sunshowers.io/handling-arguments.html>):
- "Only the top-level `App` is public."
- "`App` is a struct, one level above the command enum." Rationale: making `App` itself an enum
  causes trouble later when global options need to be added.
- "This option flattens inline options from a struct into the parent struct or enum variant" —
  recommends liberal `#[clap(flatten)]` to break long option lists into reusable components (e.g. a
  separate `OutputOpts`/`GlobalOpts` in its own file) and to pass argument sets as one parameter.
- "Global options are marked with `#[clap(global = true)]`" so e.g. `--color` can appear anywhere.
- "`ArgEnum` simplifies the definition of arguments that take one of a limited number of values"
  (example: `Color { Always, Auto, Never }` with `default_value_t = Color::Auto`).
- Verbosity is a repeatable global flag.
- **Caveat:** the chapter's snippets use pre-clap-4 names (`ArgEnum`, `parse(from_occurrences)`);
  modern equivalents are `ValueEnum` and `action = ArgAction::Count`.

Versioning (<https://rust-cli-recommendations.sunshowers.io/versioning.html>):
- A binary crate "*should* define its public API as consisting of the command-line interface, plus
  anything else related to the interface that the project's maintainers wish to keep stable," so
  "major version changes happen when there are breaking changes to the CLI, not to internal or
  library code."

Colors (<https://rust-cli-recommendations.sunshowers.io/> colors chapter, read via `print.html`):
- "Applications *should* have a global `--color` option, with the values `always`, `auto` (default)
  and `never`."
- "if the output stream ... is a pipe, applications *must* disable colors."
- "The default color schemes in applications *must* be restricted to 12 colors."
- "Applications *must not* use blinking text."
- Libraries "*should not* use termcolor because it targets the deprecated Console APIs on Windows."

Configuration (same source):
- "Configuration *should* be in the TOML format."
- "Applications *should* put repository-scoped configuration in a `.config` directory under the
  repository root."
- "Command-line arguments *must* override environment variables."
- Precedence (high→low): CLI args, env vars, directory/repo config, user config, system config, defaults.

Binaries vs libraries (same source):
- "Binary packages *must not* expose their library functionality within the same package" (for
  crates.io) and "*should not*" internally.
- "you *should* keep lib.rs as minimal as possible, unless your entire library fits in it."
- "`src/bin` makes it harder to accidentally import library code with `mod` statements."

### 2.3 Secondary/derived guidance (weaker consensus, useful as signal)

- Group related flags with `next_help_heading`; "huge flat CLIs create unmaintainable help; the fix
  is subcommands per domain" (<https://rust.codeguides.io/essential-crates/clap/>).
- Keep `Subcommand` enums alphabetically sorted; there is a crate that enforces this at runtime
  (<https://github.com/jdx/clap-sort>).
- Unit-test parsing with `Cli::try_parse_from(["bin", "--env", "prod"])`
  (<https://rust.codeguides.io/essential-crates/clap/>).
- For "truly separate tools," prefer multiple `[[bin]]` targets over one giant CLI (same source).
- clap 4 + edition 2024 + derive is the current default stack for new projects (same source;
  corroborated by <https://oneuptime.com/blog/post/2026-02-03-rust-clap-cli-applications/view>).

### 2.4 Verbosity conventions

`clap-verbosity-flag` (<https://github.com/clap-rs/clap-verbosity-flag>,
<https://docs.rs/clap-verbosity-flag>):
- Idiomatic mapping: `-q` silences, `-v` warnings, `-vv` info, `-vvv` debug, `-vvvv` trace; default
  reports only errors.
- Flatten into the **top-level** `Cli` so `-v`/`-q` apply uniformly across subcommands.
- `tracing` integration requires `--no-default-features --features tracing`, then
  `tracing_subscriber::fmt().with_max_level(args.verbosity)`.
- Change the baseline with `Verbosity<InfoLevel>`.
- `clap_verbosity` is "largely a superset and a drop-in replacement"
  (<https://docs.rs/clap-verbosity/latest/clap_verbosity/>).
- Open design gap acknowledged upstream: verbosity level is orthogonal to *format*; production users
  want ndjson/syslog while devs want pretty — pairing `-v/-q` with `--output <format>` is the common
  extension (<https://github.com/clap-rs/clap-verbosity-flag/issues/5>).

### 2.5 Shell completions

`clap_complete` (<https://docs.rs/clap_complete/latest/clap_complete/>,
<https://docs.rs/clap_complete/latest/clap_complete/env/index.html>):
- AOT generation via the `Shell` enum is the stable path.
- Dynamic completions (`CompleteEnv`) are **available on crate feature `unstable-dynamic` only**.
  The docs warn: these "work by generating shell code that calls into your_program while completing.
  That interface is unstable and a mismatch between the shell code and your_program may result in
  either invalid completions or no completions being generated."
- Recommended practice: **do not** write generated completions to a file; regenerate on shell
  startup so it is "self-correcting," and re-source on upgrade.
- `CompleteEnv::complete()` must run before the rest of app init; calling it outside a completion
  context panics; this "precludes reusing initialization"
  (<https://github.com/clap-rs/clap/discussions/5677>,
  <https://github.com/clap-rs/clap/discussions/5806>).
- Alternative with a different tradeoff: `clap_dyn_complete` moves the completion engine into the
  binary (bigger binary, one small shell adapter instead of per-shell codegen backends)
  (<https://openvmm.dev/rustdoc/linux/clap_dyn_complete/index.html>).
- Background: <https://kbknapp.dev/shell-completions/>.

---

## 3. Machine-readable output and the 2025–2026 "agent-friendly CLI" discourse

This is the fastest-moving area and where consensus is **strong on substance but contested on
defaults**.

### 3.1 The classical (pre-agent) Rust norm — Rain

<https://rust-cli-recommendations.sunshowers.io/machine-readable-output.html>:
- Simple lists: "programs *should* provide list output as newline-delimited items"; if items can
  contain newlines, programs "*must* provide a `-0` flag or similar to list output as
  null-delimited... items."
- For structured data, use an explicit format flag: "`--output-format`, or `--message-format` if
  many lines of structured data are printed out."
- "Programs *should* support at least `json` machine-readable output."
- Self-describing formats only. "Programs *must not* provide their output as bincode or other
  non-self-describing formats." Protobuf only if IDLs ship with releases.
- Streaming: ndjson, modeled on Cargo's `--message-format json`.
- "All machine-readable output *must* be printed to stdout, *not* stderr."
- "Colors *must* be disabled for machine-readable output."
- Stability: within a version series, "output *must* be kept stable and append-only"; breaking
  changes need an explicit opt-in like "`--format-version 2` or `--message-format json-v2`";
  "Adding new keys to a JSON map or equivalent is generally considered stable."

### 3.2 The Rust CLI Book

<https://rust-cli.github.io/book/in-depth/machine-communication.html>:
- Use `std::io::IsTerminal` (`std::io::stdout().is_terminal()`) — not the old `atty` crate — to
  decide human vs machine output.
- `--json` flag pattern; JSON is "simple enough that parsers exist in practically every language."
- Line-delimited JSON for streams: "write one JSON document per message and ... put each JSON
  document on new line," which "can make implementations as simple as using a regular `println!`."
- Cites ripgrep's `--json`, where "each JSON document is an object (map) containing a `type` field"
  (`begin`/`match`/`end`/`summary`) — the basis of VS Code's search integration.
- Reading piped stdin via `-`; if stdin is a TTY when piped input is required, print help and exit 2.
- The book does **not** cover JSON schema versioning (gap; Rain covers it — §3.1).

### 3.3 Arcjet — the most rigorous vendor writeup on agent CLIs

<https://blog.arcjet.com/designing-a-cli-for-ai-agents/>. Concrete decisions:

1. **Frozen API contract.** After 1.0, additive-only: new commands/flags/JSON fields allowed, nothing
   renamed or removed, because "An agent may have learned the old flag from a previous run, a local
   skill, a copied prompt, or stale context." Explicitly stricter than human-first practice because
   agents "cache patterns" and "replay examples."
2. **Fuzzy suggestions disabled** (Cobra `DisableSuggestions`): "Fuzzy suggestions create ambiguity";
   an agent might treat a suggestion as confirmation. Unknown commands/flags are hard failures.
   Discovery happens via `--help`, completions, and skills. *(This directly contradicts clig.dev's
   "if you guess what a user meant, ask" guidance — see §3.6.)*
3. **Structured errors** on stderr in JSON mode, with `error`, `code`, and `remediation` fields:
   "An agent should not have to grep stderr for 'not logged in' to decide whether authentication
   failed."
4. **Distinct exit codes**: 0 success, 1 general error, 2 authentication error, 3 input validation
   error, 4 confirmation required.
5. **Pre-network input validation** (TypeID prefixes like `site_...`): "the fastest and safest answer
   is a local validation error."
6. **Confirmation as a protocol, not a prompt.** Mutating commands without `--confirm` exit 4 and
   print a JSON confirmation envelope (status, command, `changes` array, exact `confirmCommand` to
   rerun). Interactive "are you sure?" prompts are rejected because they "require a live stdin
   conversation and often degrade into brittle text automation." Works in CI.
7. **TTY detection drives defaults**: text when stdout is a TTY, JSON when it is not, so "agents,
   scripts, and subprocess calls get structured output without remembering to pass `--output json`."
8. **`--fields`** for context-window efficiency: "Context windows are finite - if an agent only needs
   `id,name`, it should not have to ingest a full response."
9. **Self-documenting help**: full contract in the usage line plus multiple realistic examples;
   generated completions; a `skills` command pointing at an external canonical skill package rather
   than embedding all docs in the binary.
10. **Non-interactive auth**: `ARCJET_TOKEN` env var takes priority over stored credentials.
11. **Shared domain model** between CLI and MCP server — "peer clients for the same platform."

Acknowledged costs: "No fuzzy suggestions means a typo stays a typo"; more implementation code;
confirmation flows are defense-in-depth, not a substitute for backend authorization. They validated
with the open-source `cli-agent-lint`, which "found a number of things we'd overlooked."

### 3.4 Agent Surface — the most schema-like formalization

<https://agentsurface.dev/docs/cli-design> (index) defines agent-ready as: a command can be
"discovered, invoked non-interactively, parsed structurally, retried safely, and diagnosed from exit
status plus error body." Nine subtopics: command structure, machine-readable output, raw payload
input, schema introspection, context-window discipline, input hardening, safety rails, agent
knowledge packaging, CLI scale.

**Machine-readable output** (<https://agentsurface.dev/docs/cli-design/machine-readable-output>):
- "stdout carries data — the structured result of the command"; "stderr carries everything else —
  spinners, progress indicators, warnings, informational messages, debug output." Separation is
  "non-optional when agents are consumers."
- "An agent reading stdout to parse a created resource ID will break if a progress spinner is
  interleaved with the JSON."
- Both mechanisms are needed: TTY detection via `isatty(1)` so "agents get structured output by
  default without needing to pass any flags," **and** an explicit `--json` that "overrides TTY
  detection and always outputs structured JSON regardless of context."
- JSON must be a parallel path, not a serialization of the display object: "serialize the raw data
  object — the same one you would return from an API endpoint — not a derivative of the
  display-formatted version."
- Determinism, four properties: identical inputs → identical structure; "Optional fields are always
  present in the response, set to `null` rather than omitted"; "Array responses are always arrays,
  never `null` when empty"; field names stable across versions, treated "like API fields" with major
  version bumps for breaking changes.
- NDJSON for paginated/large results, flushing each record immediately.
- "Always exit with a non-zero code when outputting a structured error"; "the exit code and the
  structured error body are complementary signals — do not rely on just one."
- `NO_COLOR` must be honored *in addition to* TTY detection (CI environments can have a TTY attached).
- Rubric: score 3 requires NDJSON streaming for paginated results and structured output as the
  default in non-TTY contexts.

**CLI errors** (<https://agentsurface.dev/docs/error-handling/cli-errors>):
- A bare stderr message with exit 1 "communicates exactly one bit of information: something failed."
- Recommended stable exit-code taxonomy: 0 success; 1 general error ("safe to retry with exponential
  backoff"); 2 usage error; 3 not found; 4 auth error ("do not retry until credentials are
  refreshed"); 5 conflict/precondition. Exit codes "should never be omitted or always set to 1
  regardless of failure type."
- JSON error body on stderr with four fields: `error` — "stable snake_case error code that agents can
  branch on," explicitly never a human message; `message` — human-readable, allowed to "change
  between releases"; `suggestions` — "ordered list of concrete next steps" with runnable commands;
  `failing_input`.
- Error codes must be domain-prefixed snake_case and durable: `invoice_not_found`,
  `payment_card_declined`, `auth_token_expired`, `rate_limit_exceeded`, `service_unavailable`.
  "treat it like an API field."
- Retryability is encoded in the exit code, not a separate field.
- Stack traces are "a debug artifact that discloses internal implementation details and is unusable
  by agents"; convert unhandled exceptions to a generic `internal_error` with a trace ID.
- Validation errors: `invalid_args` array of `{arg, reason, received, expected}` plus `suggestions`,
  mirroring HTTP validation error shape.
- JSON error mode auto-activates on `isatty(stderr)`, with `--json` as override.

**Schema introspection** (<https://agentsurface.dev/docs/cli-design/schema-introspection>):
- "An agent operating without pre-loaded documentation must be able to discover what a CLI accepts at
  runtime." "Human-readable `--help` text does not satisfy this. It is written for scanning, not
  parsing."
- Three problems solved: doc staleness, upfront context cost, version mismatch — "Hardcoded
  documentation will silently mismatch."
- Layers: (1) `--help --json` as the minimum; (2) a `describe` command at every level of the tree,
  emitting the command tree, parameter details, and return-type `$ref`s; (3) live runtime-resolved
  schemas (e.g. from an OpenAPI doc) — "Static embedded schemas have the same staleness problem as
  pre-loaded documentation."
- A complete schema includes scopes/permissions, enums *with per-value descriptions*, nested `$ref`
  types, and a top-level `"version"` field.
- Skill metadata can point at the entrypoint: `schema_command: mytool describe --json`, framed as "a
  CLI extension to the public Agent Skills format, not as a separate incompatible format."
- "Auto-generate or validate CLI skill metadata from your CLI's describe output as part of your
  release pipeline."
- Rubric 0–3: 0 = help text only; 1 = partial; 2 = full JSON schema for all commands; 3 = live
  runtime-resolved schemas with scopes/enums/nested types.

**Context-window discipline** (<https://agentsurface.dev/docs/cli-design/context-window-discipline>):
- "An agent's context window is a finite, shared resource"; the practice is "giving agents tools to
  control what comes back — and building defaults that protect them when they forget to ask."
- Motivating example: 100 records × 30+ fields ≈ 50 KB JSON per page.
- `--fields` with comma-separated names on **every read command**, including single-resource reads;
  dot notation for nested (`billing.plan`); implement masking client-side if the API lacks it
  ("projecting the response after it is received").
- `--page-all`/`--all` so agents don't hand-track cursors; default responses carry pagination
  metadata (total, per-page, cursor, `has_more`).
- NDJSON streaming pagination lets agents "begin processing immediately" and stop early.
- Ship a **default field set smaller than the full record** (example: 5 of 16 fields), with `--full`
  or `--fields *` as opt-in; document `default_fields` and `all_fields` in the introspection output.
- "the most durable context window protection is not a flag — it is documentation that teaches
  agents to use the flags correctly before they need to" (SKILL.md/AGENTS.md invariants).
- Rubric 0–3: 3 = NDJSON streaming pagination + explicit skill-file guidance; "the CLI actively
  protects the agent from token waste."

### 3.5 Other agent-CLI sources (converging content, varying rigor)

- **Cursor's `cli-for-agents` skill**
  (<https://github.com/cursor/plugins/blob/main/cli-for-agent/skills/cli-for-agents/SKILL.md>):
  "Human-oriented CLIs often block agents: interactive prompts, huge upfront docs, and help text
  without copy-pasteable examples."
  - "Every input should be expressible as a flag or flag value. Do not require arrow keys, menus, or
    timed prompts." "If flags are missing, **then** fall back to interactive mode—not the other way
    around."
  - Layered discovery: "Let each subcommand own its documentation so unused commands stay out of
    context."
  - "Every subcommand has `--help`." "Every `--help` includes **Examples** with real invocations" —
    "Examples do more than prose for pattern-matching."
  - stdin/pipelines; "Avoid odd positional ordering."
  - Missing required flags → "exit immediately with a clear message and a **correct example
    invocation**, not a hang."
  - "Agents retry often. The same successful command run twice should be safe (no-op or explicit
    'already done'), not duplicate side effects."
  - "Add `--dry-run` (or equivalent)"; "Offer `--yes` / `--force` to skip confirmations while keeping
    the safe default for humans."
  - Consistent `resource` + `verb` naming; success output should be "machine-useful data: IDs, URLs,
    durations."
  - Review checklist: "non-interactive path, layered help, examples on `--help`, stdin/pipeline
    story, error messages with invocations, idempotency, dry-run, confirmation bypass flags,
    consistent command structure, structured success output."

- **Gibil, "Designing CLIs for AI Agents: The `--json` Pattern"**
  (<https://www.gibil.dev/blog/cli-json-pattern>): `--json` on *every* command because "An agent
  doesn't know which commands are 'important.'"; meaningful exit codes ("Don't exit `0` when the
  operation failed but the CLI process 'ran successfully.'"); stdout = only the JSON result, stderr =
  progress/warnings/debug; idempotent commands returning e.g.
  `{"destroyed": true, "was_already_destroyed": false}`; structured error objects
  (`{"error": true, "code": "SERVER_NOT_FOUND", "message": "..."}`) — "The agent switches on `code`.
  A human reads `message`." Closing claim: "None of this is hard to implement. It just needs to be
  intentional from the start."

- **JoelClaw, "CLI Design for AI Agents"** (<https://joelclaw.com/cli-design-for-ai-agents>) — the
  maximalist position: "No plain text. No tables. No color codes. No `--json` flag to opt into
  structured output." JSON is "the default and only format." Adds HATEOAS-style `next_actions`
  command templates with typed placeholders (`params` with `value`/`default`/`enum`/`description`),
  an error `fix` field in plain language, a self-documenting root command returning the whole command
  tree as JSON, truncation by default ("show last 30 lines, not all of them") with a file path
  pointer for retrieval, `--count`-capped lists, and NDJSON streaming with six event types (`start`,
  `step`, `progress`, `log`, `event`, terminal `result`/`error`) where "the last line is always the
  standard HATEOAS envelope." Notably does **not** discuss exit codes.

- **ComposioHQ `awesome-agent-clis`** (<https://github.com/ComposioHQ/awesome-agent-clis>) — badge
  criteria for "agent-ready": structured output (`--json`) since "agents parse JSON, not ASCII
  tables"; non-interactive mode (`--no-interactive`/`--yes`) since "agents can't answer prompts"; API
  key/env-var auth avoiding the "browser OAuth dance"; **idempotency keys** so "agents can safely
  retry on failure"; piped-output detection; meaningful exit codes to "branch on success/failure."
  Also defines a **SKILL.md packaging standard**: YAML frontmatter `name` (≤64 chars) + `description`
  (≤1024 chars), markdown body covering "installation, auth, key commands, output modes," with
  progressive disclosure — metadata ~100 tokens → full instructions <5k tokens → execution.

- **cli-agent-lint** (<https://github.com/Camil-H/cli-agent-lint>) — an actual linter you can run
  against a binary: 34 checks in 5 categories, letter-graded.
  - `FS-*` Flow Safety: "An agent that gets stuck on an interactive prompt, can't authenticate, or
    can't tell success from failure is a dead agent."
  - `TE-*` Token Efficiency: "Every byte of CLI output eats into the agent's context window." Checks
    JSON output, `--quiet`/`--no-color`, list pagination, field filtering.
  - `SD-*` Self-Describing: "Agents learn your CLI by reading `--help`."
  - `SA-*` Automation Safety: `--yes`/`--force` on destructive commands, path-traversal and
    control-character rejection, `--dry-run`.
  - `PV-*` Predictability: deterministic output, distinct **documented** exit codes, `--timeout`,
    retry/rate-limit signaling.
  - Grades: A ≥90% agent-ready, B ≥70%, C ≥50%, D ≥30%, F <30%.
  - Two modes: passive (`--help` analysis only, zero side effects) and active probing
    (`--no-probe` to disable).
  - Its own exit codes: 0 all passed, 1 fail-severity check failed, 2 usage/runtime error.
  - **The README does not enumerate all 34 check IDs**; `cli-agent-lint checks` lists them at
    runtime.

- **Anthropic, "Writing effective tools for AI agents"**
  (<https://www.anthropic.com/engineering/writing-tools-for-agents>) — the transferable principles:
  implement "pagination, range selection, filtering, and/or truncation with sensible default
  parameter values" for any response that could consume lots of context; Claude Code restricts tool
  responses to **25,000 tokens by default**; make verbosity configurable via a `response_format`
  parameter (`concise` vs `detailed`); "prompt-engineer your error responses to clearly communicate
  specific and actionable improvements, rather than opaque error codes or tracebacks"; return
  "human-readable fields and simplified outputs" rather than raw technical IDs. Companion:
  <https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents> — "find the
  smallest set of high-signal tokens that maximize the likelihood of your desired outcome."

### 3.6 Where the agent norms *contradict* clig.dev (genuinely contested)

| Question | clig.dev (human-first) | Agent-first sources |
|---|---|---|
| Default output format | Human-readable is "paramount"; JSON only if `--json` passed (<https://clig.dev/>) | JSON when stdout is not a TTY (Arcjet, Agent Surface); JSON always (JoelClaw) |
| "Did you mean?" suggestions | Guess and offer (<https://clig.dev/>) | Disable; typos must be hard failures (<https://blog.arcjet.com/designing-a-cli-for-ai-agents/>) |
| Confirmations | Interactive prompt scaled to danger (<https://clig.dev/>) | Non-interactive protocol: exit 4 + JSON envelope + exact `--confirm` rerun command (Arcjet); or `--yes`/`--dry-run` (Cursor, cli-agent-lint) |
| Exit codes | 0/non-zero, map codes to "important failure modes" (<https://clig.dev/>) | Fine-grained documented taxonomy is mandatory (Agent Surface, Arcjet, cli-agent-lint `PV-*`) |
| Verbosity of success output | "Display output on success, but keep it brief" (<https://clig.dev/>) | Truncate/paginate/field-mask aggressively by default (Agent Surface, Anthropic) |

**Strong consensus across both camps:** stdout = data, stderr = messaging; honor `NO_COLOR` and TTY
detection; never colorize machine output; additive-only changes to the machine-readable contract;
`-`/stdin support; no secrets in flags or env.

### 3.7 CLI vs MCP (relevant framing, unsettled)

Multiple 2026 analyses argue CLI wins on context cost and MCP wins on structure/security:
- A GitHub MCP server "dumps ~55,000 tokens into context before the agent does anything useful,"
  while a CLI invocation "costs roughly what its command string costs"
  (<https://www.mindstudio.ai/blog/mcp-vs-cli-ai-agents-token-costs-when-to-use>,
  <https://jannikreinhard.com/2026/02/22/why-cli-tools-are-beating-mcp-for-ai-agents/>).
- Counterpoints: CLIs "communicate through unstructured text, so a model reading stdout must infer
  structure"; CLI "inherits whatever permissions the host session has"
  (<https://buildwithfern.com/post/cli-vs-mcp-which-api-interface-first>,
  <https://tyk.io/learning-center/mcp-vs-cli-for-ai-agents-enterprise-comparison-guide/>).
- Emerging third position: "Skills" abstract the transport — "the agent calls a Skill, the Skill
  routes to CLI or MCP underneath"
  (<https://levelup.gitconnected.com/mcp-vs-cli-stop-over-engineering-your-ai-agent-tooling-1860023c567b>).
- Arcjet's practical answer: build both from a shared domain model as "peer clients"
  (<https://blog.arcjet.com/designing-a-cli-for-ai-agents/>).

### 3.8 TOON as an alternative to JSON (contested, evidence trending negative)

`obr` depends on `toon_rust`, so this matters. TOON = Token-Oriented Object Notation
(<https://github.com/toon-format/toon>): YAML-style indentation for nesting plus CSV-style tabular
rows for uniform arrays; claimed 30–60% token savings; project's own benchmark claims 29.2
accuracy%/1K tokens vs 23.8 for compact JSON and 16.6 for pretty JSON.

Criticisms:
- Not standardized: "no RFC, no governing body, and no canonical specification"; the project itself
  says the format is "an idea in progress, with nothing set in stone"
  (<https://github.com/toon-format/toon>,
  <https://community.ibm.com/community/user/blogs/ranjeet-kumar/2025/11/20/json-vs-toon-token-oriented-object-notation-choosi>).
- Academic benchmark (Feb/Mar 2026), "Token-Oriented Object Notation vs JSON: A Benchmark of Plain
  and Constrained Decoding Generation" (<https://arxiv.org/abs/2603.03306>,
  <https://arxiv.org/html/2603.03306v1>): "plain JSON generation shows the best one-shot and final
  accuracy"; TOON's "only significant advantage is the lowest token usage as a trade-off for slightly
  decreased accuracy overall and significant degradation for some models." For deeply nested
  "company" structures TOON's **one-shot accuracy was 0%** (final 48.6% after repair cycles vs JSON's
  43.8%) — "inefficient for deep recursive hierarchies as-is." The paper frames the conclusion as "a
  critical distinction between syntax efficiency and inference efficiency."
- Efficiency is non-linear: TOON only pays off "beyond a specific point where cumulative syntax
  savings amortize the initial prompt overhead" (same paper).
- Sweet spot is uniform tabular records; "for deeply nested or non-uniform data, JSON may actually be
  more efficient" (<https://github.com/toon-format/toon>).
- Practitioners keep a JSON fallback path when TOON parsing fails
  (<https://dev.to/islamhafez0/toon-token-oriented-object-notation-a-complete-guide-for-llm-data-efficiency-1ng4>).

**Note on the caveat:** that benchmark measures *LLM generation of* TOON. A CLI *emitting* TOON for
an agent to *read* is the easier direction, and the token savings on uniform tabular output (like an
issue list) are real. But the format is unstandardized, so it should never be the only machine
format, and JSON must remain canonical.

---

## 4. Error-reporting UX in Rust

### 4.1 The stable consensus: thiserror for libraries, anyhow for applications

Widely repeated and essentially uncontested:
- "use thiserror for libraries and anyhow for applications"
  (<https://everbytes.dazzbytes.com/rust/mastering-rust-error-handling-when-to-use-thiserror-vs-anyhow>,
  <https://oneuptime.com/blog/post/2026-01-25-error-types-thiserror-anyhow-rust/view>,
  <https://www.carolinemorton.co.uk/blog/rust-error-handling-anyhow-thiserror/>).
- thiserror: derive `Display`/`Error` without hiding underlying types; "always use `#[source]` or
  `#[from]` to preserve the underlying cause—losing context makes debugging painful"; group errors or
  use `#[error(transparent)]` (same sources).
- anyhow: dynamic `anyhow::Error`, `.context()`/`.with_context()`; tradeoff is "all errors become the
  same type, so the caller can't easily distinguish between different error cases"
  (<https://dev.to/leapcell/rust-error-handling-compared-anyhow-vs-thiserror-vs-snafu-2003>).
- The hybrid pattern for a binary with an internal library: "thiserror for public errors with status
  codes usually, and anyhow internally"
  (<https://markaicode.com/rust-error-handling-2025-guide/>).
- The meta-point that several sources converge on: "the most important thing is consistency — agree
  on a system with your team, write it down, and stick to it, since that matters more than which
  specific crate you choose" (<https://markaicode.com/rust-error-handling-2025-guide/>).

### 4.2 miette

<https://lib.rs/crates/miette>: drop-in replacements for anyhow/eyre `Result`, `Report`, and a
`miette!` macro; "generic support for arbitrary `SourceCode`s for snippet data"; default handler with
"fancy graphical diagnostic output using ANSI/Unicode text". "you can derive a `Diagnostic` from any
`std::error::Error` type—thiserror is a great way to define them, and plays nicely with miette."
Explicitly library-safe: "fully compatible with library usage, so consumers who don't know about, or
don't want, miette features can safely use its error types."

**Judgement for `obr`:** miette's headline value is *source-span* diagnostics (compiler/parser
style). For an issue tracker whose errors are mostly "not found / invalid state / conflict," miette
buys pretty rendering and a `help` field but adds a rendering dependency; the span machinery is only
clearly worth it for JSONL parse errors, where pointing at the offending line/column of
`.beads/issues.jsonl` would be a genuine UX win. Not a consensus requirement.

### 4.3 Rust CLI Book on errors

<https://rust-cli.github.io/book/tutorial/errors.html>: `?` propagation and its error conversion;
raw errors are unhelpful (a bare `NotFound` omits the file name); `.map_err()` with a custom struct,
or preferably anyhow's `Context`/`with_context`, which preserves the original error and produces a
"Caused by:" chain. Recommended shape: clap + `anyhow::Result` in `main()` with `.with_context()`.

### 4.4 Exit codes — genuinely contested

**sysexits.h (BSD, 64–78).** `EX_OK`=0, `EX_USAGE`=64 ("The command was used incorrectly"),
`EX_DATAERR`=65, `EX_NOINPUT`=66, `EX_CONFIG`=78
(<https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/sysexits.3.html>).
Rationale: "the caller of the process can get a rough estimation about the failure class without
looking up the source code."

**The Rust CLI Book endorses it.** <https://rust-cli.github.io/book/in-depth/exit-code.html>: "Rust
sets an exit code of `101` when the process panicked"; there's no ecosystem-wide convention beyond
"many tools exit with `1`"; it points at the BSD sysexits list and recommends the `exitcode` crate,
demonstrating `std::process::exit(exitcode::OK / CONFIG / DATAERR)`. It does **not** discuss
returning `ExitCode` from `main()`.

**The critique.** A widely cited HN comment calls sysexits "a largely unusable, not-at-all standard,"
comparing it to "a strictly less usable and relevant version of HTTP categories 400 and 500 smashed
together with no rhyme or reason" (<https://news.ycombinator.com/item?id=29056429>). Practical
problems: not available on Windows, "not all Unices agree on what the standard should look like"
(<https://chrisdown.name/2013/11/03/exit-code-best-practises.html>).

**Divergent modern schemes.** Square's `exit` library deliberately avoids 64–78, using 80–99 for user
errors and 100–119 for software/system errors (`UsageError`=80, `Forbidden`=83, `InternalError`=100)
(<https://github.com/square/exit>). The Advanced Bash Scripting Guide proposes restricting
user-defined codes to 64–113 (<https://tldp.org/LDP/abs/html/exitcodes.html>).

**The dominant practice.** "CLI tools are generally expected to use proper exit codes: 0 for success,
1 for general errors, and 2 for usage errors"
(<https://pocketcmds.com/rules/clitools/clitools-exit-codes>). The agent-first sources build small
dense taxonomies starting at 0/1/2 and extending upward with *documented, tool-specific* meanings
(Arcjet 0–4, Agent Surface 0–5, Linear-style 0/1/4/5/6) rather than adopting sysexits — see §3.3–3.4.

**Rust plumbing.** `proc-exit` (<https://github.com/rust-cli/proc-exit>) is an "`i32` newtype for exit
codes" that "Includes both standard exit codes and signal-related exit codes," integrates with
`main`, `std::process`, and `std::io::Error`, and supports "exiting silently" when the message was
already reported. Alternatives it compares itself against: `sysexit` (enum-based, "makes certain
states unrepresentable," no `main` integration), `exit-code`/`exitcode` (bare `i32` constants),
`exitfailure`.

**Bottom line for a 2026 agent-facing CLI:** sysexits is *not* the winning convention. A small,
documented, stable, tool-specific taxonomy anchored at 0/1/2 is what both the pragmatic Unix camp and
the agent camp converge on. What is non-negotiable is that the codes are (a) distinct per failure
class, (b) **documented**, and (c) stable across releases (cli-agent-lint `PV-*`;
<https://agentsurface.dev/docs/error-handling/cli-errors>).

### 4.5 Gap: Rain's error-handling chapter was unreachable

`https://rust-cli-recommendations.sunshowers.io/error-handling-and-exit-codes.html` → **HTTP 404**.
`https://rust-cli-recommendations.sunshowers.io/exit-codes.html` → **HTTP 404**.
`https://rust-cli-recommendations.sunshowers.io/print.html` returned the chapter *titles* for "Error
handling and exit codes," "Signal handling," "Atomic writes," "Locking and TOCTOU races," "Dry runs
and the interpreter pattern," and "Logging" but **no body text** for them (likely lazy-loaded or
unwritten). I did not guess their contents. The chapter titles themselves are evidence that Rain
considers signal handling, atomic writes, and TOCTOU/locking to be first-class CLI concerns
(<https://rust-cli-recommendations.sunshowers.io/>) — directly relevant to a tool that writes a
SQLite DB and a JSONL file that git also touches.

### 4.6 The SIGPIPE / broken-pipe trap (Rust-specific, frequently missed)

Rust ignores SIGPIPE by default, so a Rust CLI piped into `head`/`less` can panic with "failed
printing to stdout: Broken pipe (os error 32)" — because unlike `writeln!`, `println!` does not
return errors and panics instead
(<https://github.com/rust-lang/rust/issues/46016>,
<https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/on-broken-pipe.html>). ripgrep hit
exactly this (<https://github.com/BurntSushi/ripgrep/issues/22>).

Options:
1. Check `ErrorKind::BrokenPipe` on writes and exit quietly — the pattern ripgrep, xsv, and eza use
   (<https://github.com/sxyazi/yazi/pull/2110>, <https://github.com/rust-lang/measureme/pull/243>).
2. `-Zon-broken-pipe` compiler flag; "if the flag is not used, libstd will behave in the manner it has
   since 2014 ... SIGPIPE will be set to SIG_IGN before fn main()"
   (<https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/on-broken-pipe.html>). Formerly
   the `unix_sigpipe` attribute (<https://github.com/rust-lang/rust/issues/97889>).
3. The `calm_io` crate, whose author notes this isn't a Rust bug — "Rust was in fact handling it
   correctly" — the panic is technically accurate but bad CLI UX
   (<https://myrrlyn.net/crates/calm_io>).

Since `obr` is already on nightly, option 2 (`-Zon-broken-pipe=kill`) is available at zero cost;
option 1 is the portable one.

### 4.7 Color/TTY plumbing: the modern crates

`anstream`/`anstyle` have displaced `termcolor` as the Rust CLI default
(<https://github.com/rust-cli/anstyle>, <https://epage.github.io/blog/2023/03/anstream-simplifying-terminal-styling/>):
- `AutoStream` "automatically selects the appropriate stream based on environment variables
  (NO_COLOR, CLICOLOR, CLICOLOR_FORCE)"
  (<https://shadow.github.io/docs/rust/anstream/struct.AutoStream.html>).
- Cargo migrated from termcolor to anstream to "centralize color schemes between Shell ... and clap
  (using anstyle)" (<https://github.com/rust-lang/cargo/pull/12751>,
  <https://github.com/rust-lang/cargo/issues/12627>).
- Rationale: termcolor "came at a huge cost to ergonomics"; anstream lets you `write!` styled text
  directly and its strip API "is significantly faster than strip-ansi-escapes"
  (<https://epage.github.io/blog/2023/03/anstream-simplifying-terminal-styling/>).
- `atty` is superseded by `std::io::IsTerminal` (stable since Rust 1.70)
  (<https://rust-cli.github.io/book/in-depth/machine-communication.html>).

`NO_COLOR` spec (<https://no-color.org/>): "Command-line software which adds ANSI color to its output
by default should check for a NO_COLOR environment variable that, when present and not an empty
string (regardless of its value), prevents the addition of ANSI color." Known ambiguity in the
community: whether it should also disable bold/italic, not just color
(<https://finance.biggo.com/news/202510090202_NO_COLOR_Standard_Implementation_Challenges>).
`CLICOLOR` is deprecated in favor of `FORCE_COLOR`/`NO_COLOR`, and "should treat FORCE_COLOR as an
alias for CLICOLOR_FORCE, enabling color whenever either is set, unless NO_COLOR is also set"
(<http://bixense.com/clicolors/>).

---

## 5. CLI testing practice

### 5.1 The tool taxonomy (strong consensus)

From trycmd's own docs (<https://docs.rs/trycmd>) and the Rust CLI Book
(<https://rust-cli.github.io/book/tutorial/testing.html>):

| Tool | When | Framing |
|---|---|---|
| `assert_cmd` (+ `assert_fs`, `predicates`) | "test cases follow a certain pattern but special attention is needed in how to verify the results" | tests "that are individual pets" |
| `trycmd` | "running a lot of blunt tests" with "limited test predicates"; test data can be pulled into docs (mdbook) | "Treat your tests like cattle, instead of pets" |
| `snapbox` | "when you want something like trycmd in one off cases or you need to customize trycmd's behavior"; "flexible enough to build your own test harness like trycmd" | the lower-level building block (<https://github.com/assert-rs/snapbox>) |
| `insta` (+ `insta-cmd`) | complex/structured output, low-friction review workflow | general-purpose snapshot testing |
| `cram` | language-agnostic end-to-end CLI snapshotting | external |

### 5.2 assert_cmd concrete practice

<https://alexwlchan.net/2025/testing-rust-cli-apps-with-assert-cmd/>:
- `Command::cargo_bin("name").unwrap()` → `.arg()`/`.args()` → `.assert()` → `.success()`/`.failure()`
  /`.code()`/`.stdout()`/`.stderr()`.
- Success tests assert exact stdout **and** `.stderr("")`.
- Most tests focus on errors: `.failure()`, specific `.code()`, empty stdout, expected stderr.
- Use `predicates::str::is_match` when output isn't exactly predictable (e.g. version strings).
- Pitfall: passing a `&str` into `.stderr()` inside a helper produces "expected_stderr escapes the
  function body here"; wrap in `predicate::eq(...)`.
- Philosophy: helpers only for tightly related scenarios; "some duplication in tests is acceptable if
  it improves readability"; the author warns against a variadic macro he wrote to avoid repetition.

### 5.3 trycmd concrete mechanics

<https://docs.rs/trycmd>:
- Two formats. Literate `.trycmd`/`.md`: fenced ` ```console `/` ```trycmd ` blocks; `$` starts a
  command, `>` continues it, `? <status>` denotes exit status (default `success`); parsed with shlex;
  first token maps to `bin.name`. Structured `.toml`: "precise control over current dir,
  stdin/stdout/stderr (including binary support)", with companion `.stdin`/`.stdout`/`.stderr` files
  and `.in/`/`.out/` directories.
- Normalization: "newlines and path separators" normalized before comparison.
- Elision syntax: `...` matches all lines to the next fixed line; `[..]` matches any characters within
  a line; `[EXE]` matches `.exe` on Windows only; `[ROOT]`, `[CWD]`; custom vars via
  `TestCases::insert_var`.
- Workflow: `TRYCMD=dump` writes `.stdout`/`.stderr` into `dump/` for review; `TRYCMD=overwrite`
  updates snapshots in place. "We will preserve these with TRYCMD=dump and will make a best-effort at
  preserving them with TRYCMD=overwrite" (re: elided placeholders).
- `.in/` becomes the working dir; `.out/` compares generated files and implicitly sets
  `fs.sandbox = true`.
- Debug with `cargo test -F trycmd/debug`.

### 5.4 insta

Workflow (<https://www.rustprojectprimer.com/testing/snapshot.html>, <https://insta.rs/docs/>):
`cargo insta test` writes `.snap.new` files → `cargo insta review` shows an interactive diff per
pending change → accepted snapshots are promoted to `.snap` and committed. Combined:
`cargo insta test --review`. Snapshots live in a `snapshots/` directory next to the test by default;
inline snapshots with `@"..."` syntax are supported and updated in-source by `cargo insta review`.
Serde-based macros (JSON/YAML/TOML/CSV/RON) require `Serialize`; `insta::assert_debug_snapshot!` for
debug output. `insta-cmd` bridges to processes: `assert_cmd_snapshot!(Command::new("echo").arg("hello"))`.
Insta documents **Redactions** and **Filters** features for nondeterministic data plus **Settings**
and **globbing** (<https://insta.rs/docs/>) — I could only reach the docs index, so I have not
verified redaction syntax or the `INSTA_UPDATE`/`--check` CI flags; treat those as
"exists, details unverified."

### 5.5 Property testing and fuzzing

- proptest is "a property testing library for Rust inspired by Hypothesis for Python"; it complements
  rather than replaces coverage-guided fuzzing
  (<https://rustprojectprimer.com/testing/fuzzing.html>).
- Structure-aware fuzzing is the current norm: derive `Arbitrary` so the fuzzer generates well-formed
  structured inputs rather than raw bytes — "this approach tends to be more effective than raw byte
  fuzzing for code that doesn't directly parse bytes, since the fuzzer doesn't waste time generating
  inputs that fail to deserialize"
  (<https://rust-fuzz.github.io/book/cargo-fuzz/structure-aware-fuzzing.html>,
  <https://rustprojectprimer.com/testing/fuzzing.html>). Pattern: `arbitrary = { version = "1",
  optional = true, features = ["derive"] }` in the main crate, feature-enabled from `fuzz/Cargo.toml`.
- Bridge fuzz generators into property tests with `proptest-arbitrary-interop`
  (<https://appsec.guide/docs/fuzzing/rust/techniques/writing-harnesses/>).
- Corpus discipline: seed with valid inputs, minimize periodically
  (<https://rustprojectprimer.com/testing/fuzzing.html>).
- `cargo fuzz` shows the `Debug` output of failing structured inputs (needs libfuzzer-sys ≥ 0.2.0) and
  suggests reproduce/minimize next steps
  (<https://github.com/rust-fuzz/cargo-fuzz/blob/main/CHANGELOG.md>).
- 2025 cargo-fuzz flags: `--disable-branch-folding`, `--strip-dead-code`, `--codegen-units` (default
  1 gives "best fuzzing throughput"), `--no-include-main-msvc` for Windows (same changelog).
- `fuzz_target!` supports an `init` block (LLVMFuzzerInitialize equivalent) for one-time setup, with a
  caveat that "reproducibility may be compromised if the system under test mutates the global state"
  (<https://appsec.guide/docs/fuzzing/rust/techniques/writing-harnesses/>).
- `arbitrary` "is useful only when starting from an empty corpus, which isn't an issue with cargo-fuzz
  since it uses libFuzzer internally" — AFL++ needs a seed (same source).
- Newer alternative for model-based/differential fuzzing: `mutatis`
  (<https://rust-fuzz.github.io/book/cargo-fuzz/structure-aware-fuzzing.html>).

### 5.6 Determinism as a testability requirement

For agent tooling, determinism is both a design property and a test property. cli-agent-lint's `PV-*`
category checks "deterministic output" directly (<https://github.com/Camil-H/cli-agent-lint>), and
Agent Surface makes determinism a four-part contract (§3.4). Golden/baseline diffing of tool traces
is becoming a CI gate in the agent-ops world — e.g. `whatbroke` diffs JSONL agent traces "offline,
deterministic, with no judge or API keys needed, and is CI-gateable via exit codes"
(<https://dev.to/thedailyagent/5-open-source-tools-for-testing-ai-agents-before-they-break-production-5d9c>).
Practical consequence: any timestamp, random ID, duration, or hash-ordered map in JSON output makes
snapshot tests flaky *and* makes agent behavior non-reproducible; sort deterministically and provide
redaction points.

---

## 6. MSRV vs nightly

### 6.1 What Cargo says

<https://doc.rust-lang.org/cargo/reference/rust-version.html>:
- Policy is a balancing act between "costs for maintainers in not using newer toolchain features,"
  "costs to users who'd benefit from newer features (like reduced build times)," and "availability of
  the package to users on older Rust versions."
- "choose a policy for what Rust versions to support and when it changes, so users can compare it with
  their own policy."
- Drift is allowed but risky: "the further rust-version drifts from your specified policy, the more
  likely users are to infer a policy you did not intend, leading to frustration at unmet expectations."
- Common policies: always-latest; a fixed re-verification schedule ("the first release of the year, or
  every 5 releases").

### 6.2 Real policies

- kube-rs: "let MSRV trail 2 stable versions behind the latest stable"; crucially, they "use the
  nightly toolchain for auto-formatting and documentation, but this is a contributor-only quirk—all
  crates always build with the stable toolchain" (<https://kube.rs/rust-version/>).
- Firefox: MSRV bumps to "the strict minimum required at that moment," and never to a version "that
  hasn't been used for Firefox Nightly for at least 14 days"
  (<https://firefox-source-docs.mozilla.org/writing-rust-code/update-policy.html>).
- heapless (2025 debate): one side argued "bumping MSRV is not a breaking change"; another that "most
  Rust devs just use the latest Rust (if not nightly), so an overly new MSRV is mainly a problem for
  Linux distributions"; compromise landed on "no more recent than a 6-month-old release"
  (<https://github.com/rust-embedded/heapless/pull/595>).

### 6.3 Tooling

- MSRV-aware resolver: RFC 3537 (<https://rust-lang.github.io/rfcs/3537-msrv-resolver.html>), call for
  testing (<https://github.com/rust-lang/cargo/issues/13873>). Available via
  `workspace.resolver = "3"` or `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback`. New flags:
  `--ignore-rust-version`, `--update-rust-version`. RFC rationale: MSRV arguments "can take years for
  changes to percolate," and that delay "can be reduced if a newer toolchain can be used for
  development without upgrading the MSRV."
- `cargo msrv` empirically walks backwards from the newest toolchain until compilation fails
  (<https://crates.io/crates/cargo-msrv>).
- RFC 2495 introduced the `rust-version` field (<https://rust-lang.github.io/rfcs/2495-min-rust-version.html>).

### 6.4 The nightly-requirement critique (relevant to `obr`)

- Arch packaging guidelines set `RUSTUP_TOOLCHAIN=stable` by default, "and of course this should be
  set to nightly in the event that's what the upstream project requires," but note "this workaround is
  not required if the upstream project has a rust-toolchain file or rust-toolchain.toml file in their
  sources" (<https://wiki.archlinux.org/title/Rust_package_guidelines>).
- FreeBSD ports actively patch tools that assume nightly, e.g. "only compute unstable paths on nightly
  toolchains"
  (<https://gitlab.com/FreeBSD/freebsd-ports/commit/cdf8144444aa33f481407a4d34643e6a7e1168bd>).
- Nix `rust-overlay` keeps only nightly/beta "not earlier than one year prior to the current year"
  (<https://github.com/oxalica/rust-overlay>) — a mild reproducibility hazard for pinned old nightlies.
- Upstream Rust itself restricts which components ship on stable vs nightly and has internal
  inconsistency about what "preview" means (<https://github.com/rust-lang/rust/pull/86568>).
- Official framing: "most Rust users use stable most of the time," nightly is for "a cutting-edge
  feature" (<https://doc.rust-lang.org/book/appendix-07-nightly-rust.html>).
- CI pinning: `dtolnay/rust-toolchain` supports exact specs like `nightly-2025-01-01` and time-offset
  forms for "a sliding window of compiler support"
  (<https://github.com/dtolnay/rust-toolchain>).

**Community position, summarized:** nightly for *ancillary* tasks (rustfmt, rustdoc, miri, fuzzing) is
uncontroversial and common. Requiring nightly *to build the shipped binary* is a real distribution
burden, and the mitigations expected of you are (a) a committed `rust-toolchain.toml` and (b) pinning
a specific nightly date rather than floating `nightly`. Declaring `rust-version` while shipping a
nightly-only build is exactly the "infer a policy you did not intend" hazard Cargo warns about
(<https://doc.rust-lang.org/cargo/reference/rust-version.html>).

---

## 7. `#![forbid(unsafe_code)]` and clippy pedantic/nursery

### 7.1 forbid(unsafe_code)

- Zero-tooling guarantee at the crate level; the community treats it as close to a norm — one internals
  thread argues it "may be the minimum to ask from the community for a healthy ecosystem" and that
  "crates violating this rule can get an advisory on RUSTSEC"
  (<https://internals.rust-lang.org/t/about-supply-chain-attacks/14038/6>).
- `cargo-geiger` is the standard auditor: it "scans a Rust project (including all of its dependencies)
  for any usage of unsafe code" and marks crates declaring `#![forbid(unsafe_code)]`
  (<https://terminaltrove.com/cargo-geiger/>). Recent behavior changes: "requires all entry points for
  a crate to declare `#[forbid(unsafe_code)]` for it to count as crate-wide"; a fast `--forbid-only`
  scan mode that only parses entry-point `.rs` files; build-relevant filtering via rustc `.d` files so
  a crate with unused unsafe files can still be green; 🔒 for fully-forbidden crates vs ❓ when an
  entry point lacks the declaration
  (<https://github.com/geiger-rs/cargo-geiger/blob/master/CHANGELOG.md>).
- **The honest limit** (important not to oversell): "it's commonly overestimated how much forbidding
  unsafe would help, since `std::process::Command`, `#[no_mangle]`, `#[link_section]`,
  `#[export_name]`, `extern \"C\"` linkage, and even proc-macros are all technically 'safe' yet can
  inject arbitrary capabilities. You can forbid all of that, but you lose a ton of Rust features"
  (<https://internals.rust-lang.org/t/pre-rfc-rust-safety-standard/23963?page=4>).
- Broader proposals ("Pre-RFC: Cargo Safety Rails", "Pre-RFC: Rust Safety Standard") remain
  discussions, not policy
  (<https://internals.rust-lang.org/t/pre-rfc-cargo-safety-rails/5535/53>,
  <https://internals.rust-lang.org/t/pre-rfc-rust-safety-standard/23963?page=4>).
- `cargo-safety` is an alternative AST-walking auditor (<https://github.com/davidbarsky/cargo-safety>).
- Related: edition 2024 added `deprecated_safe_2024`, currently allow-by-default, which "detects unsafe
  functions being used as safe functions"
  (<https://doc.rust-lang.org/beta/nightly-rustc/rustc_lint_defs/builtin/static.DEPRECATED_SAFE_2024.html>).

### 7.2 Clippy pedantic / nursery — where consensus is weakest

- Official categorization: `clippy::all` is "everything on by default (all categories except nursery,
  pedantic, and cargo)"; `clippy::pedantic` "contains lints which are rather strict and off by
  default"; `clippy::nursery` "contains new lints that aren't quite ready yet"
  (<https://github.com/rust-lang/rust-clippy>).
- Official warning: "`clippy::pedantic` contains some very aggressive lints prone to false positives."
  For `restriction`, the docs are emphatic that it "should, emphatically, not be enabled as a whole,
  since contained lints may lint against perfectly reasonable code, may not have an alternative
  suggestion, and may contradict other lints" (<https://github.com/rust-lang/rust-clippy>,
  <https://doc.rust-lang.org/clippy/configuration.html>).
- `deny` "emits an error when triggering for your code — an error causes Clippy to exit with an error
  code, making it most useful in CI/CD scripts" (<https://doc.rust-lang.org/clippy/configuration.html>).
- **Prevailing pattern in practice**: enable pedantic/nursery as **warnings** in the manifest and let
  CI fail on `-D warnings`, plus a maintained per-lint allow-list. swiftnav-rs did exactly this:
  "this sets the `clippy::pedantic` set of lints by default to be warnings, but remember that CI will
  fail on any warnings"; the author noted they'd "stumbled upon another extremely controversial topic
  in the Rust community" (<https://github.com/swift-nav/swiftnav-rs/pull/131>).
- The "enable broadly, then carve out exceptions" list pattern is common: typical carve-outs are
  `missing_errors_doc`, `must_use_candidate`, `module_name_repetitions` (pedantic) and
  `cognitive_complexity`, `option_if_let_else` (nursery) (<https://rtaw.co.uk/posts/clippy-lints/>).
- Generated code should be exempted from pedantic/nursery entirely
  (<https://github.com/slint-ui/slint/pull/7433>).
- Numbers for scale: ~445 lints are warn/deny by default; ~291 are allow by default
  (<https://rtaw.co.uk/posts/clippy-lints/>).

**Verdict:** `pedantic + nursery` at **deny** in the manifest (as `obr` does) is *above* the community
norm. It is defensible, but nursery specifically is upstream-declared "not quite ready," so denying it
in the manifest couples your build to Clippy's experimental lint churn on every toolchain bump — a
real maintenance tax on a nightly toolchain, where clippy moves fastest. The lower-risk convention
that gets the same CI enforcement: `warn` in `[lints.clippy]`, `-D warnings` in CI.

---

## 8. Release-profile tradeoffs (secondary, but `obr` has an unusual profile)

- `panic = "abort"`: "shrinks the binary and speeds up panic paths, but you lose the stack trace";
  "use panic = 'abort' for applications and 'unwind' for libraries," and note that "`catch_unwind`
  does not work with `panic = 'abort'`"
  (<https://www.stanza.dev/courses/rust-performance/profiles/rust-perf-panic-binary>). As of Rust
  1.92+ "you get proper backtraces even with abort on Linux, so you should only disable unwind tables
  if binary size is critical" (same source).
- `opt-level = "z"` is the *smallest*-binary setting, inherited from clang; it is explicitly a
  speed/size trade — "everyone wants their program to be super fast and super small ... it's usually
  not possible to have both"
  (<https://docs.rust-embedded.org/book/unsorted/speed-vs-size.html>).
- Common guidance splits by workload: "performance-critical CLI tools use `lto = 'fat'` with
  `codegen-units = 1`, whereas WASM binaries use `opt-level = 'z'` specifically for download size"
  (<https://leapcell.io/blog/rust-release-optimization>).
- `opt-level = 3` is not automatically best: "opt-level = 2 often matches speed while keeping the
  binary smaller and compiling faster"
  (<https://www.stanza.dev/courses/rust-performance/profiles/rust-perf-cargo-profiles>).
- Measure before tuning: "use cargo-bloat to find what is taking space before blindly applying
  settings" (<https://markaicode.com/binary-size-optimization-techniques/>).

---

## 9. Consensus strength summary

**Strong consensus (safe to treat as norms):**
- stdout = machine data; stderr = human messaging/logs/progress. (clig.dev, Rain, Rust CLI Book, Agent Surface, Gibil)
- `--json` must exist; colors must be off in machine output; honor `NO_COLOR` and TTY detection.
- Additive-only evolution of the machine-readable contract; version breaking changes explicitly. (Rain, clig.dev, Arcjet, Agent Surface)
- NDJSON for streaming/multi-record output, one object per line, each with a `type`. (Rain, Rust CLI Book/ripgrep, Agent Surface, JoelClaw)
- clap derive is the default parser choice for new Rust CLIs. (clap FAQ, Rain)
- Global options via `global = true` + `flatten`; `App` is a struct above the command enum. (Rain)
- thiserror for typed/library errors, anyhow for application glue; preserve sources. (near-universal)
- Non-zero exit on structured error; exit code and error body are complementary. (Agent Surface, Gibil, clig.dev)
- Non-interactive path for everything; `--yes`/`--force`/`--dry-run` on destructive commands. (Cursor, cli-agent-lint, Composio, clig.dev)
- Idempotency for retry safety. (Cursor, Composio, Gibil, Arcjet)
- `--help` per subcommand with real examples. (Cursor, Arcjet, clig.dev, cli-agent-lint `SD-*`)
- assert_cmd/trycmd/snapbox/insta each have a well-understood niche; use the right one.
- `#![forbid(unsafe_code)]` is a recognized, auditable signal (cargo-geiger).

**Contested / no consensus:**
- Whether JSON should be the default in non-TTY contexts (Arcjet/Agent Surface yes; clig.dev implies no) or the *only* format (JoelClaw).
- sysexits.h vs a small tool-specific taxonomy vs 0/1/2 only.
- "Did you mean?" suggestions: helpful (clig.dev) vs harmful (Arcjet).
- Interactive confirmation prompts vs non-interactive confirmation protocol.
- Clippy pedantic/nursery at `deny` vs `warn` + `-D warnings` in CI.
- MSRV policy: always-latest vs N-versions-behind vs 6-month floor.
- TOON vs JSON for agent-facing serialization (evidence currently favors JSON as canonical).
- CLI vs MCP as the primary agent surface (converging on "both, from a shared model").
- Whether `#![forbid(unsafe_code)]` meaningfully improves supply-chain safety.

---

## 10. Sources consulted (with reachability notes)

Reachable and read in depth:
- <https://clig.dev/>
- <https://github.com/cli-guidelines/cli-guidelines>
- <https://rust-cli-recommendations.sunshowers.io/> (intro), `/cli-parser.html`, `/handling-arguments.html`, `/machine-readable-output.html`, `/print.html` (partial)
- <https://docs.rs/clap/latest/clap/_faq/index.html>
- <https://docs.rs/clap/latest/clap/_derive/_tutorial/index.html>
- <https://docs.rs/clap_complete/latest/clap_complete/env/index.html>
- <https://docs.rs/clap-verbosity-flag>, <https://github.com/clap-rs/clap-verbosity-flag>
- <https://rust-cli.github.io/book/tutorial/errors.html>, `/in-depth/machine-communication.html`, `/in-depth/exit-code.html`
- <https://github.com/rust-cli/proc-exit>
- <https://blog.arcjet.com/designing-a-cli-for-ai-agents/>
- <https://agentsurface.dev/docs/cli-design> (+ `/machine-readable-output`, `/schema-introspection`, `/context-window-discipline`, `/docs/error-handling/cli-errors`)
- <https://github.com/cursor/plugins/blob/main/cli-for-agent/skills/cli-for-agents/SKILL.md>
- <https://github.com/Camil-H/cli-agent-lint>
- <https://github.com/ComposioHQ/awesome-agent-clis>
- <https://www.gibil.dev/blog/cli-json-pattern>
- <https://joelclaw.com/cli-design-for-ai-agents>
- <https://www.anthropic.com/engineering/writing-tools-for-agents>
- <https://docs.rs/trycmd>
- <https://alexwlchan.net/2025/testing-rust-cli-apps-with-assert-cmd/>
- <https://rust-fuzz.github.io/book/cargo-fuzz/structure-aware-fuzzing.html>
- <https://github.com/rust-fuzz/cargo-fuzz/blob/main/CHANGELOG.md>
- <https://no-color.org/>, <http://bixense.com/clicolors/>
- <https://doc.rust-lang.org/cargo/reference/rust-version.html>
- <https://arxiv.org/abs/2603.03306>

Partially reachable (index only; details unverified):
- <https://insta.rs/docs/> — index listed Redactions/Filters/Settings/Cargo Insta but no bodies.
- <https://rust-cli-recommendations.sunshowers.io/print.html> — chapter titles only for Error handling
  and exit codes, Signal handling, Atomic writes, Locking and TOCTOU races, Dry runs, Logging.

Unreachable (404):
- <https://rust-cli-recommendations.sunshowers.io/error-handling-and-exit-codes.html>
- <https://rust-cli-recommendations.sunshowers.io/exit-codes.html>

---

## 11. Grounding: what `obr` already does (read-only inspection)

Recorded so the recommendations in §12 are concrete. Paths are absolute.

- `/Users/johnw/src/obr/Cargo.toml:5` — `edition = "2024"`, `/Users/johnw/src/obr/Cargo.toml:6` —
  `rust-version = "1.85"`, but `/Users/johnw/src/obr/rust-toolchain.toml:2` — `channel = "nightly"`
  (floating, undated). Package name `beads_rust`, `[[bin]] name = "obr"`
  (`/Users/johnw/src/obr/Cargo.toml:2`, `:13`).
- Deps already aligned with norms: clap 4.5 derive + env, clap_complete with `unstable-dynamic`,
  thiserror 2.0 **and** anyhow 1.0, serde/serde_json, schemars (JSON Schema), tracing +
  tracing-subscriber with `json` feature (`/Users/johnw/src/obr/Cargo.toml:15-63`).
- Dev-deps already cover the norm stack: `assert_cmd`, `predicates`, `insta` (json+yaml features),
  `proptest`, `criterion`, `tempfile` (`/Users/johnw/src/obr/Cargo.toml:76-84`). **No `trycmd`/`snapbox`.**
- `fuzz/` directory exists (cargo-fuzz targets, per recent commit `5312cb5`).
- `/Users/johnw/src/obr/Cargo.toml:100` — `[lints.rust] unsafe_code = "forbid"`. ✅
- `/Users/johnw/src/obr/Cargo.toml:103-104` — clippy `pedantic` and `nursery` both at **`deny`**, with
  six per-lint allows at `:106-111` (`cast_precision_loss`, `doc_markdown`, `missing_const_for_fn`,
  `uninlined_format_args`, `useless_let_if_seq`, `format_push_string`) commented "Allow these lints to
  unblock CI."
- `/Users/johnw/src/obr/Cargo.toml:88-94` — release profile is `opt-level = "z"`, `lto = true`,
  `codegen-units = 1`, `panic = "abort"`, `strip = true`. Note tension with the project's own
  "SQLite for speed" positioning (see §8).
- Global flags exist and are correct: `--json`, `-v` (`ArgAction::Count`), `-q`, `--no-color` all
  declared `global = true` (`/Users/johnw/src/obr/src/cli/mod.rs:659-704`).
- **But** `--robot` is declared *per-subcommand* at least 11 times as a duplicate local flag
  ("Machine-readable output (alias for --json)"), e.g.
  `/Users/johnw/src/obr/src/cli/mod.rs:1859, 1871, 1936, 1977, 2005, 2021, 2104, 2195, 2278, 2298`.
  Dispatch honors it inconsistently: `Close`/`Reopen`/`Blocked` use `cli.json || args.robot`
  (`/Users/johnw/src/obr/src/main.rs:59, 62, 76`) while `List`, `Show`, `Search`, `Comments`, `Count`,
  `Lint`, `Ready`, `Sync`, `Dep`, `Epic`, `Label` are passed only `cli.json`
  (`/Users/johnw/src/obr/src/main.rs:49-79`). This violates clig.dev "Be consistent across
  subcommands" and Rain's `global = true` guidance.
- Structured error taxonomy already exists with grouped exit codes:
  `/Users/johnw/src/obr/src/error/structured.rs:32` (`enum ErrorCode`) and `:199` (`exit_code()`) —
  2 = database/init, 3 = issue/operational, 4 = validation, 5 = dependency, 6 = sync/JSONL, and
  includes a `PathTraversal` code (`:91`). This is already close to Agent Surface's recommended shape.
- `handle_error` (`/Users/johnw/src/obr/src/main.rs:304-325`) does the right thing: JSON structured
  error **to stderr** when `--json` **or** `!stdout().is_terminal()`, human+color otherwise (color
  gated on `stderr().is_terminal()`), then `std::process::exit(exit_code)`. Note it does **not** check
  `NO_COLOR` here (Agent Surface says TTY detection alone is insufficient), and it uses
  `to_string_pretty` for a machine stream.
- `CompleteEnv::with_factory(Cli::command).complete()` runs first in `main`
  (`/Users/johnw/src/obr/src/main.rs:17`) — correct placement per clap_complete docs, but it depends
  on the **unstable** `unstable-dynamic` feature.
- A `CLI_SCHEMA.json` exists at repo root, and `schemars` is a dependency — the raw material for the
  `describe --json` / `--help --json` introspection layer Agent Surface scores at level 2–3.
- `AGENTS.md`, `AGENT_FRIENDLINESS_REPORT.md`, `skills/`, and `ROBOT_MODE_EXAMPLES.jsonl` exist at repo
  root — the project is already thinking in the SKILL.md/agent-packaging direction.

---

## 12. Benchmark verdict and prioritized recommendations for `obr`

Rough grading against the agent-CLI rubrics (cli-agent-lint categories + Agent Surface 0–3 scales),
based on the §11 inspection only:

| Dimension | Assessment |
|---|---|
| stdout/stderr separation | Good — structured errors go to stderr, data to stdout |
| Exit-code taxonomy | Strong (2–6 by class) but **undocumented in help/README** as far as inspected |
| JSON output | Present via global `--json`; **default-in-non-TTY only for errors**, not for success output |
| Flag consistency | Weak — `--robot` duplicated per-subcommand and inconsistently honored |
| Schema introspection | Partial — `CLI_SCHEMA.json` exists but no `describe --json` / `--help --json` verified |
| Context discipline | Unknown/likely weak — no `--fields` observed |
| Testing | Good breadth (assert_cmd + insta + proptest + cargo-fuzz); missing trycmd/snapbox |
| Safety rails | `--dry-run`/`--yes` coverage not verified; `PathTraversal` error code is a good sign |
| Toolchain posture | Nightly floating + `rust-version = 1.85` is a contradictory signal |
| Lint posture | Above-norm strict (pedantic+nursery at `deny`) |

Prioritized, each tied to a source:

1. **Make `--robot` a single global alias for `--json`, not 11 local flags.** Rain: use
   `#[clap(global = true)]` and `flatten`
   (<https://rust-cli-recommendations.sunshowers.io/handling-arguments.html>); clig.dev: "Be
   consistent across subcommands" (<https://clig.dev/>). This also removes the `cli.json` vs
   `cli.json || args.robot` divergence in `/Users/johnw/src/obr/src/main.rs:49-79`.
2. **Document the exit-code table in `--help`, README, and the skill file, and freeze it.**
   cli-agent-lint's `PV-*` explicitly checks for "distinct **documented** exit codes"
   (<https://github.com/Camil-H/cli-agent-lint>); Agent Surface: exit codes "should never be omitted
   or always set to 1" and must be a stable contract
   (<https://agentsurface.dev/docs/error-handling/cli-errors>). `obr` has the taxonomy; it needs the
   published contract.
3. **Extend the non-TTY JSON default from errors to success output** (or decide explicitly not to, and
   say why). Today `handle_error` already switches on `!stdout().is_terminal()`
   (`/Users/johnw/src/obr/src/main.rs:310`) but success paths take `cli.json`. Arcjet: JSON default
   when stdout is not a TTY so "agents, scripts, and subprocess calls get structured output without
   remembering to pass `--output json`" (<https://blog.arcjet.com/designing-a-cli-for-ai-agents/>);
   Agent Surface says keep `--json` as an explicit override on top
   (<https://agentsurface.dev/docs/cli-design/machine-readable-output>). Caveat: this is the single
   most **contested** recommendation here (clig.dev implies human-default) and it is a breaking change
   for existing scripts — gate it behind a major version or an opt-in env var.
4. **Honor `NO_COLOR` (and `CLICOLOR_FORCE`/`FORCE_COLOR`) in addition to TTY detection.**
   `handle_error` checks only `stderr().is_terminal()`
   (`/Users/johnw/src/obr/src/main.rs:319`). clig.dev requires `NO_COLOR`/`TERM=dumb`/`--no-color`
   (<https://clig.dev/>); Agent Surface notes CI can have a TTY attached
   (<https://agentsurface.dev/docs/cli-design/machine-readable-output>). `anstream`'s `AutoStream` +
   `anstyle_query` does all of this for free and is what Cargo and clap use
   (<https://github.com/rust-cli/anstyle>, <https://github.com/rust-lang/cargo/pull/12751>).
5. **Emit compact JSON, not `to_string_pretty`, on machine streams.** Every pretty-printed byte is
   context budget (<https://www.anthropic.com/engineering/writing-tools-for-agents>;
   cli-agent-lint `TE-*`). `/Users/johnw/src/obr/src/main.rs:314-317` currently pretty-prints errors.
6. **Add a `describe --json` (or `--help --json`) introspection command generated from the existing
   schemars types**, with a top-level `version` field, and validate `CLI_SCHEMA.json` against it in CI
   so it cannot drift. Agent Surface scores this 2–3 and explicitly recommends "Auto-generate or
   validate CLI skill metadata from your CLI's describe output as part of your release pipeline"
   (<https://agentsurface.dev/docs/cli-design/schema-introspection>).
7. **Add `--fields` (dot notation) and enforce a smaller default field set on `list`/`ready`/`show`,
   with `--full` to opt back in; emit NDJSON when listing with `--all`.** Arcjet's `--fields` rationale
   (<https://blog.arcjet.com/designing-a-cli-for-ai-agents/>); Agent Surface's rubric
   (<https://agentsurface.dev/docs/cli-design/context-window-discipline>); Rain's ndjson guidance
   (<https://rust-cli-recommendations.sunshowers.io/machine-readable-output.html>). For an issue
   tracker whose output an agent reads on every turn, this is the highest-leverage token win.
8. **Guarantee JSON determinism**: optional fields present as `null` rather than omitted, empty
   arrays never `null`, stable field names, stable sort order
   (<https://agentsurface.dev/docs/cli-design/machine-readable-output>). This simultaneously makes
   insta snapshots non-flaky (§5.6).
9. **Publish a JSON output stability policy** — append-only within a major version, breaking changes
   only behind `--format-version 2`
   (<https://rust-cli-recommendations.sunshowers.io/machine-readable-output.html>), and treat the CLI
   surface as the semver public API
   (<https://rust-cli-recommendations.sunshowers.io/versioning.html>).
10. **Handle `BrokenPipe` explicitly** (or add `-Zon-broken-pipe=kill`, available since you're on
    nightly). `br list | head` panicking is a classic Rust CLI embarrassment
    (<https://github.com/BurntSushi/ripgrep/issues/22>,
    <https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/on-broken-pipe.html>). Note
    `panic = "abort"` in `/Users/johnw/src/obr/Cargo.toml:92` makes an unhandled broken-pipe panic
    abort with SIGABRT rather than exit 101 — worse, not better.
11. **Resolve the nightly/MSRV contradiction.** Either drop `rust-version = "1.85"` (it implies a
    stable-channel promise the build does not keep) or move to stable and confine nightly to
    fmt/miri/fuzz, kube-rs style (<https://kube.rs/rust-version/>). At minimum pin a dated nightly in
    `rust-toolchain.toml` so distro packagers and CI are reproducible
    (<https://wiki.archlinux.org/title/Rust_package_guidelines>,
    <https://github.com/dtolnay/rust-toolchain>). Cargo's own warning about drifting `rust-version`
    applies directly (<https://doc.rust-lang.org/cargo/reference/rust-version.html>).
12. **Consider downgrading clippy `nursery` (and possibly `pedantic`) from `deny` to `warn` in
    `[lints.clippy]` while keeping `-D warnings` in CI.** Upstream calls nursery "not quite ready" and
    pedantic "prone to false positives" (<https://github.com/rust-lang/rust-clippy>); the common
    pattern is warn-in-manifest + deny-in-CI (<https://github.com/swift-nav/swiftnav-rs/pull/131>).
    This decouples local developer builds from Clippy churn on nightly bumps without weakening the
    gate. Contested — this is a style choice, not a defect.
13. **Add `trycmd` or `snapbox` for bulk golden CLI tests** alongside the existing assert_cmd/insta
    setup, especially since trycmd's `.trycmd` markdown format lets README/skill examples double as
    tests and its `[..]`/`...`/`[ROOT]` normalization handles paths and IDs
    (<https://docs.rs/trycmd>). This directly protects the "frozen CLI contract" promise agents depend
    on (<https://blog.arcjet.com/designing-a-cli-for-ai-agents/>).
14. **Run `cli-agent-lint` against the built binary** and treat the letter grade as a CI signal; Arcjet
    reported it "found a number of things we'd overlooked"
    (<https://blog.arcjet.com/designing-a-cli-for-ai-agents/>,
    <https://github.com/Camil-H/cli-agent-lint>). Use `--no-probe` first given `obr` mutates a database.
15. **Keep JSON canonical; treat TOON as an opt-in secondary encoding for uniform tabular output
    only.** The Feb–Mar 2026 benchmark found "plain JSON generation shows the best one-shot and final
    accuracy" and 0% one-shot accuracy for TOON on deeply nested structures
    (<https://arxiv.org/abs/2603.03306>), and TOON has "no RFC, no governing body, and no canonical
    specification" (<https://github.com/toon-format/toon>). An issue *list* is TOON's best case; an
    issue *detail* with nested comments/deps is its worst.
16. **Decide the confirmation model deliberately for destructive commands** (`delete`, `sync` with
    collisions). Options: clig.dev's interactive prompt gated on `stdin.is_terminal()`
    (<https://clig.dev/>), Cursor's `--dry-run` + `--yes` (<https://github.com/cursor/plugins/blob/main/cli-for-agent/skills/cli-for-agents/SKILL.md>),
    or Arcjet's exit-4 + JSON confirmation envelope with an exact `--confirm` rerun command
    (<https://blog.arcjet.com/designing-a-cli-for-ai-agents/>). All three are defensible; picking one
    and documenting it is what matters.
17. **Document idempotency per command** (which are retry-safe: `update`, `label add/remove`, `close`
    on an already-closed issue; which are not: `create`, `comment add`) in `AGENTS.md`/skill file.
    Composio badges idempotency keys as an agent-readiness criterion
    (<https://github.com/ComposioHQ/awesome-agent-clis>); Cursor: "The same successful command run
    twice should be safe (no-op or explicit 'already done')"
    (<https://github.com/cursor/plugins/blob/main/cli-for-agent/skills/cli-for-agents/SKILL.md>).
18. **Add Examples sections to every subcommand's `--help`.** "Every `--help` includes **Examples**
    with real invocations ... Examples do more than prose for pattern-matching"
    (<https://github.com/cursor/plugins/blob/main/cli-for-agent/skills/cli-for-agents/SKILL.md>);
    clig.dev: "Lead with examples" (<https://clig.dev/>). With trycmd these examples become tests.
19. **Align the skill file with the Composio SKILL.md standard** — YAML frontmatter `name` ≤64 chars,
    `description` ≤1024 chars, body ≤5k tokens covering install/auth/key commands/output modes, with
    progressive disclosure (<https://github.com/ComposioHQ/awesome-agent-clis>).
20. **Reconsider `opt-level = "z"`** if SQLite-backed speed is a product claim; "z" is the
    smallest-binary setting and an explicit speed trade
    (<https://docs.rust-embedded.org/book/unsorted/speed-vs-size.html>), while
    "performance-critical CLI tools use `lto = 'fat'` with `codegen-units = 1`"
    (<https://leapcell.io/blog/rust-release-optimization>). Measure with `cargo-bloat`/benchmarks
    before changing (<https://markaicode.com/binary-size-optimization-techniques/>).
