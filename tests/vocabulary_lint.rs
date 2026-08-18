//! A STALENESS gate for everything `obr` says out loud.
//!
//! # Why this file exists
//!
//! Goldens detect CHANGE, not staleness. Every rename pass in this fork
//! re-baselined the snapshots, so strings that were already wrong were
//! re-frozen as "expected" — and one golden went further and masked the
//! offending line behind a `.*$`. Meanwhile the Rich branch (gated across
//! dozens of files) is unreachable from a piped harness, so a whole output
//! mode was never executed at all: `obr init` told users it had created
//! `.beads/`, `beads.db` and `issues.org` while the filesystem held `.obr/`,
//! `obr.db` and `PLAN.org`.
//!
//! So this test asserts nothing about what obr *should* say. It asserts what
//! obr must never say again — the vocabulary of the pre-fork tool — across
//! every output mode the CLI supports, against workspaces it creates fresh.
//! An assertion of that shape cannot be re-baselined into uselessness.
//!
//! # What is checked
//!
//! * every subcommand's `--help`, walked from clap's own tree, in the default
//!   run — help is pure prose and the likeliest place staleness hides (before
//!   this file, 1 of 46 top-level subcommands had any assertion on its help);
//! * a lifecycle matrix (init → create → read → close → flush → delete →
//!   purge) in plain, `--json`, TOON and `--robot` modes;
//! * the same matrix on a real pseudo-terminal, which is the only way to
//!   reach the Rich renderers;
//! * a legacy `.beads` workspace, as a positive control that the deprecation
//!   warnings still NAME the legacy artifact — this file must never be
//!   satisfiable by deleting those.
//!
//! Both stdout and stderr are scanned. A run of the exhaustive variant
//! (`cargo test --test vocabulary_lint -- --ignored`) widens the command
//! matrix; the default set is the one the pre-commit gate runs.

#[path = "common/rich_pty.rs"]
mod rich_pty;

use assert_cmd::Command;
use clap::CommandFactory;
use regex::Regex;
use serde_json::Value;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// The forbidden vocabulary
// ---------------------------------------------------------------------------

/// One retired name, with the evidence that its pattern still works.
struct Forbidden {
    /// What the token was, in the terms a reader needs to fix the hit.
    what: &'static str,
    pattern: &'static str,
    /// A string the pattern MUST match. Without this a typo'd regex would
    /// match nothing and the whole gate would pass vacuously — the precise
    /// failure mode this file exists to prevent.
    stale_sample: &'static str,
    /// The modern spelling, which the pattern must NOT match. Without this a
    /// too-greedy pattern would fire on correct output and get "fixed" by
    /// widening the allowlist.
    current_sample: &'static str,
}

/// The vocabulary of the pre-fork tool. Nothing obr prints may use it.
const FORBIDDEN: &[Forbidden] = &[
    Forbidden {
        what: "the pre-rename workspace directory (now `.obr/`)",
        pattern: r"\.beads",
        stale_sample: "Location: /proj/.beads",
        current_sample: "Location: /proj/.obr",
    },
    Forbidden {
        what: "the pre-rename database filename (now `obr.db`)",
        pattern: r"beads\.db",
        stale_sample: "SQLite database (beads.db)",
        current_sample: "SQLite database (obr.db)",
    },
    Forbidden {
        what: "the pre-D-SURFACE export filename (now `PLAN.org`)",
        // Only a lie for a workspace that is NOT pinned to a JSONL export;
        // every workspace this file creates takes the default surface, and
        // the legacy fixture pins `PLAN.org` explicitly, so any hit here is
        // a hardcoded literal rather than a resolved path.
        pattern: r"issues\.jsonl",
        stale_sample: "wrote .obr/issues.jsonl",
        current_sample: "wrote PLAN.org",
    },
    Forbidden {
        what: "the pre-rename issue-id prefix (`bd-`)",
        pattern: r"\bbd-",
        stale_sample: r#"Unique ID (e.g., "bd-abc123")"#,
        current_sample: r#"Unique ID (e.g., "obr-abc123")"#,
    },
    Forbidden {
        what: "the pre-rename binary name (`br`, now `obr`)",
        pattern: r"\bbr(\s|$)",
        stale_sample: "run br list",
        current_sample: "run obr list",
    },
    Forbidden {
        what: "the pre-fork environment prefix `BEADS_`",
        pattern: r"\bBEADS_",
        stale_sample: "[env: BEADS_DIR=]",
        current_sample: "[env: OBR_DIR=]",
    },
    Forbidden {
        what: "the pre-fork environment prefix `BD_`",
        pattern: r"\bBD_",
        stale_sample: "[env: BD_DB=]",
        current_sample: "[env: OBR_DB=]",
    },
    Forbidden {
        what: "the pre-fork environment prefix `BR_`",
        pattern: r"\bBR_",
        stale_sample: "[env: BR_OUTPUT_FORMAT=]",
        current_sample: "[env: OBR_OUTPUT_FORMAT=]",
    },
];

fn compiled() -> &'static [(&'static Forbidden, Regex)] {
    static COMPILED: OnceLock<Vec<(&'static Forbidden, Regex)>> = OnceLock::new();
    COMPILED.get_or_init(|| {
        FORBIDDEN
            .iter()
            .map(|entry| {
                let regex = Regex::new(entry.pattern)
                    .unwrap_or_else(|e| panic!("forbidden pattern {:?}: {e}", entry.pattern));
                (entry, regex)
            })
            .collect()
    })
}

// ---------------------------------------------------------------------------
// The allowlist
// ---------------------------------------------------------------------------

/// Messages whose entire job is to NAME a legacy artifact.
///
/// Each entry is a substring that identifies such a message, paired with the
/// reason it is exempt. Nothing belongs here that merely *mentions* a legacy
/// name in passing: these must stay matched so that "fixing" the warning by
/// removing the legacy name from it turns the positive-control test
/// (`legacy_workspace_warnings_still_name_the_legacy_artifacts`) red.
const DELIBERATE_LEGACY_MESSAGES: &[(&str, &str)] = &[(
    "is a legacy name and support for it will be removed",
    // WHY: this is the deprecation warning for an adopted pre-rename
    // workspace. It has to print `.beads` / `beads.db` — those are the names
    // the operator must rename, and a warning that would not say them is
    // useless. It is emitted only when such a workspace really is in use.
    "legacy-name deprecation warning; must name the artifact to rename",
)];

/// Inventory of stale strings that are real defects but belong to code this
/// slice does not own. Same shape as the other gates in `ci/baseline/`:
/// tolerated, never grown, deleted as the debt is paid.
const DEBT_INVENTORY: &str = "ci/baseline/vocabulary-debt.txt";

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn debt_entries() -> &'static [String] {
    static ENTRIES: OnceLock<Vec<String>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let path = manifest_dir().join(DEBT_INVENTORY);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect()
    })
}

/// What makes a line containing legacy vocabulary acceptable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Absolution {
    DeliberateLegacyMessage,
    /// The workspace under test really is a pre-rename one, so its own
    /// resolved paths contain `.beads` truthfully.
    LegacyWorkspacePath,
    KnownDebt,
}

/// The workspace a scan is running against, which decides what may be
/// absolved.
#[derive(Clone, Copy, Default)]
struct Scope<'a> {
    /// Set only for a workspace this test deliberately created as legacy.
    legacy_root: Option<&'a str>,
}

fn absolve(line: &str, scope: Scope<'_>) -> Option<Absolution> {
    if DELIBERATE_LEGACY_MESSAGES
        .iter()
        .any(|(needle, _)| line.contains(needle))
    {
        return Some(Absolution::DeliberateLegacyMessage);
    }
    if let Some(root) = scope.legacy_root
        && line.contains(root)
    {
        return Some(Absolution::LegacyWorkspacePath);
    }
    if debt_entries().iter().any(|entry| line.contains(entry)) {
        return Some(Absolution::KnownDebt);
    }
    None
}

// ---------------------------------------------------------------------------
// Output modes
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Plain,
    Json,
    Toon,
    Robot,
    Rich,
}

impl Mode {
    const fn label(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Json => "--json",
            Self::Toon => "toon (OBR_OUTPUT_FORMAT)",
            Self::Robot => "--robot",
            Self::Rich => "rich (pty)",
        }
    }
}

/// The non-terminal modes every command supports.
const PIPED_MODES: &[Mode] = &[Mode::Plain, Mode::Json, Mode::Toon, Mode::Robot];

// ---------------------------------------------------------------------------
// Violations
// ---------------------------------------------------------------------------

struct Violation {
    command: String,
    mode: Mode,
    stream: &'static str,
    what: &'static str,
    pattern: &'static str,
    line_no: usize,
    line: String,
}

/// Keep the report readable when the offender is a 100 KB JSON schema line.
fn excerpt(line: &str, at: usize) -> String {
    const HALF: usize = 120;
    let start = line[..at]
        .char_indices()
        .rev()
        .nth(HALF)
        .map_or(0, |(i, _)| i);
    let end = line[at..]
        .char_indices()
        .nth(HALF)
        .map_or(line.len(), |(i, _)| at + i);
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(&line[start..end]);
    if end < line.len() {
        out.push('…');
    }
    out
}

fn report(violations: &[Violation]) -> String {
    let mut out = String::from("obr still speaks the pre-fork vocabulary.\n\n");
    for v in violations {
        let _ = writeln!(
            out,
            "  {} [{}] {}:{}\n    {}\n    -> {} (/{}/)",
            v.command,
            v.stream,
            v.mode.label(),
            v.line_no,
            v.line,
            v.what,
            v.pattern,
        );
    }
    let _ = writeln!(
        out,
        "\n{} violation(s). Derive the string from a resolved value \
         (config::WORKSPACE_DIR_NAME, DEFAULT_DB_FILENAME, SURFACE_FILENAME, \
         the resolved ConfigPaths) instead of a literal. A deliberate \
         legacy-discovery message belongs in DELIBERATE_LEGACY_MESSAGES in \
         this file, with the reason; nothing else does.",
        violations.len()
    );
    out
}

fn scan_stream(
    violations: &mut Vec<Violation>,
    command: &str,
    mode: Mode,
    stream: &'static str,
    text: &str,
    scope: Scope<'_>,
) {
    for (index, line) in text.lines().enumerate() {
        for (entry, regex) in compiled() {
            let Some(found) = regex.find(line) else {
                continue;
            };
            if absolve(line, scope).is_some() {
                continue;
            }
            violations.push(Violation {
                command: command.to_string(),
                mode,
                stream,
                what: entry.what,
                pattern: entry.pattern,
                line_no: index + 1,
                line: excerpt(line, found.start()),
            });
            // One report per line is enough to locate it.
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Running obr
// ---------------------------------------------------------------------------

struct Captured {
    success: bool,
    stdout: String,
    stderr: String,
}

impl Captured {
    fn is_empty(&self) -> bool {
        self.stdout.trim().is_empty() && self.stderr.trim().is_empty()
    }
}

fn base_cmd(root: &Path) -> Command {
    let mut cmd = Command::new(rich_pty::obr_bin());
    cmd.current_dir(root);
    rich_pty::clear_inherited_obr_env(&mut cmd);
    rich_pty::pin_fixture_identity(&mut cmd);
    cmd.env("HOME", root);
    cmd.env("RUST_LOG", "error");
    cmd.env("RUST_BACKTRACE", "1");
    cmd
}

fn run_piped(root: &Path, mode: Mode, args: &[String]) -> Captured {
    assert_ne!(mode, Mode::Rich, "Rich output needs a terminal");
    let mut cmd = base_cmd(root);
    cmd.env("NO_COLOR", "1");
    if mode == Mode::Json {
        // Global flag: it has to precede the subcommand.
        cmd.arg("--json");
    }
    cmd.args(args);
    // `--robot` is declared per subcommand, so the lifecycle still has to run
    // the steps that do not take it — they degrade to plain, which keeps the
    // workspace in the state the later steps need.
    // `robot_mode_is_reachable_from_the_matrix` keeps that from degenerating
    // into a second plain column.
    if mode == Mode::Robot && accepts_robot(args) {
        cmd.arg("--robot");
    }
    if mode == Mode::Toon {
        cmd.env("OBR_OUTPUT_FORMAT", "toon");
    }
    let output = cmd.output().expect("run obr");
    Captured {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Width wide enough that no panel row wraps mid-token; a wrapped `.beads`
/// would slip past a line-oriented scan.
const RICH_WIDTH: usize = 200;

fn run_rich(root: &Path, args: &[String]) -> Captured {
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let run = rich_pty::rich_obr(root, RICH_WIDTH, &borrowed);
    Captured {
        success: run.success,
        // stdout and stderr are the same device on a terminal.
        stdout: run.text,
        stderr: String::new(),
    }
}

fn run(root: &Path, mode: Mode, args: &[String]) -> Captured {
    if mode == Mode::Rich {
        run_rich(root, args)
    } else {
        run_piped(root, mode, args)
    }
}

/// Run a command purely to set up state; its output is not scanned.
fn setup(root: &Path, args: &[&str]) -> String {
    let mut cmd = base_cmd(root);
    cmd.env("NO_COLOR", "1");
    cmd.args(args);
    let output = cmd.output().expect("run obr setup command");
    assert!(
        output.status.success(),
        "setup command failed: {args:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

// ---------------------------------------------------------------------------
// The clap subcommand tree
// ---------------------------------------------------------------------------

/// Every subcommand path, taken from clap rather than from a hand-kept list
/// so a newly added command is covered the day it lands.
fn subcommand_paths() -> Vec<Vec<String>> {
    fn walk(cmd: &clap::Command, prefix: &[String], out: &mut Vec<Vec<String>>) {
        for sub in cmd.get_subcommands() {
            let name = sub.get_name();
            if name == "help" {
                continue;
            }
            let mut path = prefix.to_vec();
            path.push(name.to_string());
            out.push(path.clone());
            walk(sub, &path, out);
        }
    }

    let root = obr::cli::Cli::command();
    let mut out = Vec::new();
    walk(&root, &[], &mut out);
    out
}

/// The deepest clap node the leading tokens of `args` name.
fn command_node(args: &[String]) -> clap::Command {
    let mut node = obr::cli::Cli::command();
    for token in args {
        if token.starts_with('-') {
            break;
        }
        let matched = node
            .get_subcommands()
            .find(|sub| {
                sub.get_name() == token || sub.get_all_aliases().any(|alias| alias == token)
            })
            .cloned();
        match matched {
            Some(sub) => node = sub,
            None => break,
        }
    }
    node
}

/// Ask clap, not the help text: `--robot` is declared per subcommand and
/// several commands spell it `--robot-triage` instead.
fn accepts_robot(args: &[String]) -> bool {
    command_node(args)
        .get_arguments()
        .any(|arg| arg.get_long() == Some("robot"))
}

// ---------------------------------------------------------------------------
// The command matrix
// ---------------------------------------------------------------------------

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

/// One step of the lifecycle matrix.
struct Step {
    args: Vec<String>,
    /// `obr lint` exits non-zero when it has findings; that is a result, not
    /// a failure, and its output still has to be clean.
    may_exit_nonzero: bool,
}

impl Step {
    fn new(parts: &[&str]) -> Self {
        Self {
            args: argv(parts),
            may_exit_nonzero: false,
        }
    }

    fn tolerating_failure(parts: &[&str]) -> Self {
        Self {
            args: argv(parts),
            may_exit_nonzero: true,
        }
    }
}

/// Commands that only read. Extended by the exhaustive variant.
fn read_only_steps(issue: &str, exhaustive: bool) -> Vec<Step> {
    let mut steps = vec![
        Step::new(&["list"]),
        Step::new(&["ready"]),
        Step::new(&["show", issue]),
        Step::new(&["doctor"]),
        Step::new(&["doctor", "health"]),
        Step::new(&["where"]),
        Step::new(&["info"]),
        Step::new(&["sync", "--status"]),
        Step::new(&["history", "list"]),
        Step::new(&["config", "list"]),
        Step::new(&["version"]),
        Step::new(&["stats"]),
        Step::new(&["agents", "--check"]),
    ];
    if exhaustive {
        steps.extend([
            Step::new(&["blocked"]),
            Step::new(&["count"]),
            Step::new(&["stale"]),
            Step::new(&["orphans"]),
            Step::new(&["status"]),
            Step::new(&["search", "probe"]),
            Step::new(&["graph", issue]),
            Step::new(&["dep", "list", issue]),
            Step::new(&["gate", "list", issue]),
            Step::new(&["audit", "log", issue]),
            Step::new(&["audit", "summary"]),
            Step::new(&["comments", "list", issue]),
            Step::new(&["label", "list"]),
            Step::new(&["query", "list"]),
            Step::new(&["epic", "status"]),
            Step::new(&["capacity", "exemptions"]),
            Step::new(&["coordination", "status"]),
            Step::new(&["scheduler"]),
            Step::new(&["changelog"]),
            Step::new(&["vcs-status"]),
            Step::new(&["capabilities"]),
            Step::new(&["schema"]),
            Step::new(&["robot-docs", "guide"]),
            Step::tolerating_failure(&["lint"]),
        ]);
    }
    steps
}

/// The JSON document in `stdout`, ignoring anything printed ahead of it.
///
/// Takes the EARLIEST of `{` and `[`: `list --json` returns an object whose
/// first field is an array, so keying on `[` alone lands inside the envelope
/// and leaves the object's tail as trailing characters.
fn json_payload(stdout: &str) -> &str {
    let start = [stdout.find('{'), stdout.find('[')]
        .into_iter()
        .flatten()
        .min()
        .expect("a JSON payload in stdout");
    stdout[start..].trim()
}

/// Every issue id in the workspace, newest first, via a setup call so the
/// parsing never depends on the mode under test.
fn issue_ids(root: &Path) -> Vec<String> {
    let stdout = setup(root, &["list", "--json"]);
    let payload: Value =
        serde_json::from_str(json_payload(&stdout)).expect("list --json output parses");
    let issues = payload
        .get("issues")
        .and_then(Value::as_array)
        .or_else(|| payload.as_array())
        .expect("an issue array in list --json");
    issues
        .iter()
        .filter_map(|issue| issue.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn first_issue_id(root: &Path) -> String {
    issue_ids(root)
        .into_iter()
        .next()
        .expect("at least one issue")
}

/// Drive a whole workspace lifecycle in one mode and collect every violation.
fn scan_lifecycle(mode: Mode, exhaustive: bool) -> Vec<Violation> {
    let temp = TempDir::new().expect("temp dir");
    let root = temp.path();
    let mut violations = Vec::new();
    let scan = |violations: &mut Vec<Violation>, step: &Step| {
        let command = format!("obr {}", step.args.join(" "));
        let captured = run(root, mode, &step.args);
        if !step.may_exit_nonzero {
            assert!(
                captured.success,
                "{command} [{}] failed; a command that never ran proves nothing\
                 \nstdout:\n{}\nstderr:\n{}",
                mode.label(),
                captured.stdout,
                captured.stderr
            );
        }
        assert!(
            !captured.is_empty(),
            "{command} [{}] produced no output at all; the scan would pass \
             vacuously",
            mode.label()
        );
        scan_stream(
            violations,
            &command,
            mode,
            "stdout",
            &captured.stdout,
            Scope::default(),
        );
        scan_stream(
            violations,
            &command,
            mode,
            "stderr",
            &captured.stderr,
            Scope::default(),
        );
    };

    // The prefix must not itself be legacy vocabulary, or every generated id
    // would trip `\bbd-` and the gate would be measuring the fixture.
    scan(&mut violations, &Step::new(&["init", "--prefix", "vocab"]));
    scan(
        &mut violations,
        &Step::new(&[
            "create",
            "Alpha vocabulary probe",
            "--type",
            "bug",
            "--priority",
            "1",
            "--description",
            "A probe issue so read commands have something to render.",
        ]),
    );
    scan(
        &mut violations,
        &Step::new(&["create", "Beta vocabulary probe"]),
    );

    let ids = issue_ids(root);
    let issue = ids.first().expect("a probe issue").clone();
    let doomed = ids
        .iter()
        .find(|id| **id != issue)
        .expect("a second probe issue to delete")
        .clone();

    for step in read_only_steps(&issue, exhaustive) {
        scan(&mut violations, &step);
    }

    scan(
        &mut violations,
        &Step::new(&["close", &issue, "--reason", "vocabulary probe complete"]),
    );
    scan(&mut violations, &Step::new(&["sync", "--flush-only"]));
    // delete then `--hard` with no ids: the tombstone garbage-collector, whose
    // Rich branch used to print the count and swallow the ids.
    scan(&mut violations, &Step::new(&["delete", &doomed, "--force"]));
    scan(&mut violations, &Step::new(&["delete", "--hard"]));

    violations
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Anti-vacuity: a gate whose patterns match nothing is green forever.
#[test]
fn forbidden_patterns_match_the_vocabulary_they_name() {
    for (entry, regex) in compiled() {
        assert!(
            regex.is_match(entry.stale_sample),
            "/{}/ ({}) does not match its own stale sample {:?}; this pattern \
             would never fire and the gate would be decorative",
            entry.pattern,
            entry.what,
            entry.stale_sample
        );
        assert!(
            !regex.is_match(entry.current_sample),
            "/{}/ ({}) matches the CURRENT spelling {:?}; it would fire on \
             correct output and invite the allowlist to be widened",
            entry.pattern,
            entry.what,
            entry.current_sample
        );
    }
    assert!(
        !FORBIDDEN.is_empty(),
        "an empty forbidden list makes every other test in this file vacuous"
    );
}

/// Help is pure prose, is emitted by every command, and is where the rename
/// passes were least likely to look.
#[test]
fn every_subcommand_help_is_free_of_legacy_vocabulary() {
    let paths = subcommand_paths();
    assert!(
        paths.len() > 40,
        "expected the full subcommand tree from clap, got {} entries — the \
         walk is broken and the coverage is imaginary",
        paths.len()
    );

    let temp = TempDir::new().expect("temp dir");
    let mut violations = Vec::new();

    // The root help too: it lists every command's one-line description.
    for path in std::iter::once(Vec::new()).chain(paths) {
        let mut args = path.clone();
        args.push("--help".to_string());
        let command = format!("obr {}", args.join(" "));
        let captured = run_piped(temp.path(), Mode::Plain, &args);
        assert!(captured.success, "{command} failed");
        assert!(!captured.is_empty(), "{command} printed no help at all");
        scan_stream(
            &mut violations,
            &command,
            Mode::Plain,
            "stdout",
            &captured.stdout,
            Scope::default(),
        );
        scan_stream(
            &mut violations,
            &command,
            Mode::Plain,
            "stderr",
            &captured.stderr,
            Scope::default(),
        );
    }

    assert!(violations.is_empty(), "{}", report(&violations));
}

/// The lifecycle matrix in every non-terminal mode.
#[test]
fn command_matrix_is_free_of_legacy_vocabulary() {
    let mut violations = Vec::new();
    for mode in PIPED_MODES {
        violations.extend(scan_lifecycle(*mode, false));
    }
    assert!(violations.is_empty(), "{}", report(&violations));
}

/// The same matrix on a real terminal. This is the only mode that reaches the
/// Rich renderers, and it is the mode that shipped `.beads/` for four
/// releases.
#[test]
fn rich_command_matrix_is_free_of_legacy_vocabulary() {
    let violations = scan_lifecycle(Mode::Rich, false);
    assert!(violations.is_empty(), "{}", report(&violations));
}

/// Positive control for the allowlist.
///
/// In a workspace that really is pre-rename, `.beads` and `beads.db` are the
/// truth: the deprecation warning has to name them, and every resolved path
/// legitimately contains them. This test fails if those warnings are ever
/// swept along with the genuinely stale strings — so the gate above cannot be
/// satisfied by deleting them.
#[test]
fn legacy_workspace_warnings_still_name_the_legacy_artifacts() {
    let temp = TempDir::new().expect("temp dir");
    let root = temp.path();
    setup(root, &["init", "--prefix", "vocab"]);

    let legacy_dir = root.join(".beads");
    std::fs::rename(root.join(".obr"), &legacy_dir).expect("rename workspace");
    std::fs::rename(legacy_dir.join("obr.db"), legacy_dir.join("beads.db")).expect("rename db");
    // Pin the surface so the legacy fixture keeps the modern export name and
    // an `issues.jsonl` hit would still be a real defect.
    std::fs::write(
        legacy_dir.join("metadata.json"),
        br#"{"jsonl_export":"PLAN.org"}"#,
    )
    .expect("rewrite metadata");

    let captured = run_piped(root, Mode::Plain, &argv(&["where"]));
    assert!(captured.success, "obr where failed in a legacy workspace");
    let combined = format!("{}{}", captured.stdout, captured.stderr);

    for expected in [
        ".beads is a legacy name and support for it will be removed",
        "beads.db is a legacy name and support for it will be removed",
    ] {
        assert!(
            combined.contains(expected),
            "the legacy-discovery warning stopped naming the legacy artifact \
             ({expected:?}); an operator can no longer tell what to rename:\n{combined}"
        );
    }

    // And the scan must be quiet here: everything legacy in this output is
    // either that warning or the workspace's own resolved path.
    let legacy_root = root.to_string_lossy().into_owned();
    let scope = Scope {
        legacy_root: Some(&legacy_root),
    };
    let mut violations = Vec::new();
    scan_stream(
        &mut violations,
        "obr where",
        Mode::Plain,
        "stdout",
        &captured.stdout,
        scope,
    );
    scan_stream(
        &mut violations,
        "obr where",
        Mode::Plain,
        "stderr",
        &captured.stderr,
        scope,
    );
    assert!(violations.is_empty(), "{}", report(&violations));
}

/// The wide matrix: every read-only command in every mode, terminal included.
/// Not in the pre-commit set only because of runtime.
#[test]
#[ignore = "exhaustive: run with --ignored"]
fn exhaustive_command_matrix_is_free_of_legacy_vocabulary() {
    let mut violations = Vec::new();
    for mode in PIPED_MODES.iter().copied().chain([Mode::Rich]) {
        violations.extend(scan_lifecycle(mode, true));
    }
    assert!(violations.is_empty(), "{}", report(&violations));
}

/// `--robot` is a real second machine-readable mode; make sure the matrix is
/// actually exercising it somewhere rather than silently degrading to plain.
#[test]
fn robot_mode_is_reachable_from_the_matrix() {
    let reachable = read_only_steps("vocab-000", true)
        .iter()
        .filter(|step| accepts_robot(&step.args))
        .count();
    assert!(
        reachable > 0,
        "no command in the matrix accepts --robot; the Robot column of this \
         gate is testing plain output twice"
    );
}

/// Debt paid down should be debt deleted. Ignored rather than enforced so a
/// concurrent fix in another slice cannot turn this suite red before the
/// inventory line is removed.
#[test]
#[ignore = "maintenance: run with --ignored after paying down vocabulary debt"]
fn vocabulary_debt_inventory_has_no_paid_entries() {
    let mut everything = String::new();
    for mode in PIPED_MODES.iter().copied().chain([Mode::Rich]) {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path();
        setup(root, &["init", "--prefix", "vocab"]);
        setup(root, &["create", "Alpha vocabulary probe"]);
        for step in read_only_steps(&first_issue_id(root), true) {
            let captured = run(root, mode, &step.args);
            everything.push_str(&captured.stdout);
            everything.push_str(&captured.stderr);
        }
    }
    for path in subcommand_paths() {
        let mut args = path;
        args.push("--help".to_string());
        let temp = TempDir::new().expect("temp dir");
        everything.push_str(&run_piped(temp.path(), Mode::Plain, &args).stdout);
    }

    let paid: Vec<&String> = debt_entries()
        .iter()
        .filter(|entry| !everything.contains(entry.as_str()))
        .collect();
    assert!(
        paid.is_empty(),
        "these lines are no longer produced anywhere; delete them from {}:\n{:#?}",
        DEBT_INVENTORY,
        paid
    );
}

/// Guard the guard: the fixture path itself must not smuggle legacy
/// vocabulary into the scanned output.
#[test]
fn fixture_prefix_is_not_itself_legacy_vocabulary() {
    let temp = TempDir::new().expect("temp dir");
    setup(temp.path(), &["init", "--prefix", "vocab"]);
    setup(temp.path(), &["create", "Alpha vocabulary probe"]);
    let id = first_issue_id(temp.path());
    for (entry, regex) in compiled() {
        assert!(
            !regex.is_match(&id),
            "the fixture's own issue id {id:?} trips /{}/ ({}); this gate \
             would be measuring its own workspace",
            entry.pattern,
            entry.what
        );
    }
    let path: PathBuf = temp.path().to_path_buf();
    assert!(
        !path.to_string_lossy().contains(".beads"),
        "the fixture temp dir must not contain a legacy name"
    );
}
