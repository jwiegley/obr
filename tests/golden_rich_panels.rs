//! Golden snapshots for Rich-mode panel/table widths.
//!
//! The usual CLI test helpers force `NO_COLOR=1`, which makes `obr` select plain
//! output. These tests run `obr` under `script(1)` so stdout is a pseudo-terminal
//! and the Rich renderer observes the requested terminal width.

mod common;

use assert_cmd::Command;
use insta::assert_snapshot;
use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tempfile::TempDir;

/// The pty plumbing lives in one place; `tests/vocabulary_lint.rs` drives the
/// same terminal through it.
#[path = "common/rich_pty.rs"]
mod rich_pty;

use rich_pty::{clear_inherited_obr_env, obr_bin, pin_fixture_identity};

struct RichFixture {
    _temp_dir: TempDir,
    root: PathBuf,
    show_id: String,
}

fn obr_cmd() -> Command {
    Command::new(obr_bin())
}

fn run_setup_obr(root: &Path, args: &[&str]) -> String {
    let mut cmd = obr_cmd();
    cmd.current_dir(root);
    cmd.args(args);
    clear_inherited_obr_env(&mut cmd);
    cmd.env("HOME", root);
    cmd.env("NO_COLOR", "1");
    cmd.env("RUST_LOG", "error");
    cmd.env("RUST_BACKTRACE", "1");
    pin_fixture_identity(&mut cmd);

    let output = cmd.output().expect("run setup obr command");
    assert!(
        output.status.success(),
        "obr setup command failed: {:?}\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn extract_json_payload(stdout: &str) -> &str {
    let start = stdout
        .find('{')
        .or_else(|| stdout.find('['))
        .expect("JSON payload in stdout");
    stdout[start..].trim()
}

fn create_issue(
    root: &Path,
    title: &str,
    issue_type: &str,
    priority: &str,
    description: &str,
    labels: &str,
) -> String {
    let stdout = run_setup_obr(
        root,
        &[
            "create",
            title,
            "--type",
            issue_type,
            "--priority",
            priority,
            "--description",
            description,
            "--labels",
            labels,
            "--json",
        ],
    );
    let parsed: Value =
        serde_json::from_str(extract_json_payload(&stdout)).expect("create JSON output");
    parsed["id"].as_str().expect("created issue id").to_string()
}

fn init_fixture() -> RichFixture {
    let temp_dir =
        TempDir::new_in(common::cli::isolated_temp_root()).expect("create isolated temp dir");
    let root = temp_dir.path().to_path_buf();

    run_setup_obr(&root, &["init", "--prefix", "rich"]);

    let show_id = create_issue(
        &root,
        "Alpha layout regression with a medium length title",
        "bug",
        "1",
        "A deterministic issue used to freeze Rich-mode show panel wrapping and field alignment.",
        "ui,regression",
    );
    let blocked_id = create_issue(
        &root,
        "Beta table row exercises dependency columns",
        "feature",
        "2",
        "Second fixture issue with dependency metadata for list and stats rendering.",
        "backend,triage",
    );
    let closed_id = create_issue(
        &root,
        "Gamma closed work contributes status counts",
        "task",
        "3",
        "Closed fixture issue so the statistics panel contains mixed status data.",
        "done,metrics",
    );

    run_setup_obr(
        &root,
        &[
            "comments",
            "add",
            &show_id,
            "--author",
            "ubuntu",
            "A stable comment keeps the show panel exercising comment rendering.",
        ],
    );
    run_setup_obr(&root, &["dep", "add", &blocked_id, &show_id]);
    run_setup_obr(
        &root,
        &[
            "close",
            &closed_id,
            "--reason",
            "Completed for golden snapshot coverage",
        ],
    );

    RichFixture {
        _temp_dir: temp_dir,
        root,
        show_id,
    }
}

fn run_rich_obr(root: &Path, width: usize, args: &[&str]) -> String {
    normalize_rich_output(&rich_pty::rich_obr_ok(root, width, args))
}

fn issue_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\brich-[a-z0-9]{3,}\b").expect("issue id regex"))
}

fn timestamp_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z| UTC)?)?")
            .expect("timestamp regex")
    })
}

fn relative_time_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(?:just now|\d+(?:\.\d+)?(?:ns|us|µs|ms|s|m|h|d) ago)\b")
            .expect("relative time regex")
    })
}

fn replace_preserving_width(input: &str, regex: &Regex, placeholder: &str) -> String {
    regex
        .replace_all(input, |captures: &regex::Captures<'_>| {
            let matched_width = captures[0].chars().count();
            let placeholder_width = placeholder.chars().count();
            if placeholder_width >= matched_width {
                placeholder.to_string()
            } else {
                format!(
                    "{placeholder}{}",
                    " ".repeat(matched_width - placeholder_width)
                )
            }
        })
        .into_owned()
}

/// Golden-specific scrubbing, applied on top of the pty cleanup in
/// [`rich_pty::pty_text`]: freeze the values that legitimately differ between
/// runs (generated ids, clocks) while preserving column widths, since these
/// goldens exist to pin layout.
fn normalize_rich_output(without_ansi: &str) -> String {
    let without_ids = replace_preserving_width(without_ansi, issue_id_re(), "rich-ID");
    let without_timestamps = replace_preserving_width(&without_ids, timestamp_re(), "TIMESTAMP");
    let without_relative_times =
        replace_preserving_width(&without_timestamps, relative_time_re(), "TIME_AGO");
    without_relative_times
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

fn assert_rich_frame(output: &str, command: &str, width: usize) {
    assert!(
        output.contains('┌')
            || output.contains('┏')
            || output.contains('╭')
            || output.contains('╔'),
        "expected Rich frame characters for {command} at width {width}, got:\n{output}"
    );
}

/// Names that were true before the workspace rename and the D-SURFACE move.
/// For any workspace obr creates today they are lies, and the `obr init` panel
/// printed all three, hardcoded, because the panel only renders on a TTY and
/// nothing in the suite ever gave it one.
const STALE_INIT_LITERALS: &[&str] = &[".beads", "beads.db", "issues.org"];

fn assert_panel_says(output: &str, case: &str, expected: &[&str]) {
    for line in expected {
        assert!(
            output.contains(line),
            "{case}: init panel never said {line:?}:\n{output}"
        );
    }
}

/// `obr init` on a real pseudo-terminal must describe the workspace it really
/// created. Covers the default root surface, a `doc/` surface, and a legacy
/// `.beads` workspace adopted in place.
#[test]
fn rich_init_panel_names_the_real_artifacts() {
    // (a) default: `.obr/` + `obr.db`, surface at the project root.
    let plain = TempDir::new().expect("temp dir");
    let out = run_rich_obr(plain.path(), 100, &["init", "--prefix", "gate"]);
    assert_rich_frame(&out, "init", 100);
    assert_panel_says(
        &out,
        "default workspace",
        &[
            "[+] .obr/ directory",
            "[+] SQLite database (obr.db)",
            "[+] PLAN.org (empty export seed)",
            "  .obr/",
            "    |-- obr.db",
            "    `-- .gitignore",
            "  PLAN.org",
        ],
    );
    for stale in STALE_INIT_LITERALS {
        assert!(
            !out.contains(stale),
            "default workspace: panel still prints {stale:?}:\n{out}"
        );
    }
    assert!(plain.path().join(".obr/obr.db").is_file());
    assert!(plain.path().join("PLAN.org").is_file());
    assert!(!plain.path().join(".obr/issues.org").exists());

    // (b) a project that already has `doc/`: the surface resolves there.
    let doc = TempDir::new().expect("temp dir");
    std::fs::create_dir(doc.path().join("doc")).expect("create doc/");
    let out = run_rich_obr(doc.path(), 100, &["init", "--prefix", "gate"]);
    assert_panel_says(
        &out,
        "doc surface",
        &["[+] doc/PLAN.org (empty export seed)", "  doc/PLAN.org"],
    );
    for stale in STALE_INIT_LITERALS {
        assert!(
            !out.contains(stale),
            "doc surface: panel still prints {stale:?}:\n{out}"
        );
    }
    assert!(doc.path().join("doc/PLAN.org").is_file());
    assert!(!doc.path().join("PLAN.org").exists());

    // (c) a genuine pre-rename workspace, adopted in place: here the legacy
    // names are the truth and the panel is required to print them.
    let legacy = TempDir::new().expect("temp dir");
    run_setup_obr(legacy.path(), &["init", "--prefix", "gate"]);
    let legacy_dir = legacy.path().join(".beads");
    std::fs::rename(legacy.path().join(".obr"), &legacy_dir).expect("rename workspace");
    std::fs::rename(legacy_dir.join("obr.db"), legacy_dir.join("beads.db")).expect("rename db");
    std::fs::write(
        legacy_dir.join("metadata.json"),
        br#"{"jsonl_export":"PLAN.org"}"#,
    )
    .expect("un-record the database name");
    std::fs::remove_file(legacy.path().join("PLAN.org")).expect("drop the seeded surface");

    let out = run_rich_obr(legacy.path(), 100, &["init", "--prefix", "gate", "--force"]);
    assert_panel_says(
        &out,
        "legacy workspace",
        &[
            "[=] .beads/ directory",
            "[=] SQLite database (beads.db)",
            "[+] PLAN.org (empty export seed)",
            "  .beads/",
            "    |-- beads.db",
            "  PLAN.org",
        ],
    );
    assert!(
        !out.contains("issues.org"),
        "legacy workspace: the surface is seeded outside the dot dir:\n{out}"
    );
    assert!(legacy_dir.join("beads.db").is_file());
    assert!(legacy.path().join("PLAN.org").is_file());
    assert!(!legacy.path().join(".obr").exists());
}

#[test]
fn golden_list_rich_widths() {
    let fixture = init_fixture();

    let width_80 = run_rich_obr(&fixture.root, 80, &["list", "--limit", "3"]);
    assert_rich_frame(&width_80, "list", 80);
    assert_snapshot!("list_width_80", width_80);

    let width_120 = run_rich_obr(&fixture.root, 120, &["list", "--limit", "3"]);
    assert_rich_frame(&width_120, "list", 120);
    assert_snapshot!("list_width_120", width_120);
}

#[test]
fn golden_show_rich_widths() {
    let fixture = init_fixture();

    let width_80 = run_rich_obr(&fixture.root, 80, &["show", &fixture.show_id]);
    assert_rich_frame(&width_80, "show", 80);
    assert_snapshot!("show_width_80", width_80);

    let width_120 = run_rich_obr(&fixture.root, 120, &["show", &fixture.show_id]);
    assert_rich_frame(&width_120, "show", 120);
    assert_snapshot!("show_width_120", width_120);
}

#[test]
fn golden_stats_rich_widths() {
    let fixture = init_fixture();

    let width_80 = run_rich_obr(&fixture.root, 80, &["stats"]);
    assert_rich_frame(&width_80, "stats", 80);
    assert_snapshot!("stats_width_80", width_80);

    let width_120 = run_rich_obr(&fixture.root, 120, &["stats"]);
    assert_rich_frame(&width_120, "stats", 120);
    assert_snapshot!("stats_width_120", width_120);
}
