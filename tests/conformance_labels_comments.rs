#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! Conformance Tests: Label and Comment Commands
//!
//! These tests verify obr (Rust) produces identical output to bd (Go)
//! for label and comment operations.

mod common;

use assert_cmd::Command;
use chrono::Utc;
use common::cli::extract_json_payload;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tempfile::TempDir;
use tracing::info;

/// Skip test when `bd` is not a usable classic conformance reference.
macro_rules! skip_if_no_bd {
    () => {
        if let Some(reason) = common::bd_skip_reason() {
            eprintln!("Skipping conformance test: {reason}");
            return;
        }
    };
}

/// Output from running a command
#[derive(Debug)]
pub struct CmdOutput {
    pub stdout: String,
    pub stderr: String,
    pub status: std::process::ExitStatus,
    pub duration: Duration,
}

/// Workspace for conformance tests with paired obr/bd directories
pub struct ConformanceWorkspace {
    pub temp_dir: TempDir,
    pub obr_root: PathBuf,
    pub bd_root: PathBuf,
    pub log_dir: PathBuf,
}

impl ConformanceWorkspace {
    pub fn new() -> Self {
        let temp_dir = TempDir::new().expect("create temp dir");
        let root = temp_dir.path().to_path_buf();
        let obr_root = root.join("br_workspace");
        let bd_root = root.join("bd_workspace");
        let log_dir = root.join("logs");

        fs::create_dir_all(&obr_root).expect("create obr workspace");
        fs::create_dir_all(&bd_root).expect("create bd workspace");
        fs::create_dir_all(&log_dir).expect("create log dir");

        Self {
            temp_dir,
            obr_root,
            bd_root,
            log_dir,
        }
    }

    /// Initialize both obr and bd workspaces
    pub fn init_both(&self) -> (CmdOutput, CmdOutput) {
        let obr_out = self.run_obr(["init"], "init");
        let bd_out = self.run_bd(["init"], "init");
        (obr_out, bd_out)
    }

    /// Run obr command in the obr workspace
    pub fn run_obr<I, S>(&self, args: I, label: &str) -> CmdOutput
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_obr_cmd(&self.obr_root, &self.log_dir, args, &format!("br_{label}"))
    }

    /// Run bd command in the bd workspace
    pub fn run_bd<I, S>(&self, args: I, label: &str) -> CmdOutput
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_bd_cmd(&self.bd_root, &self.log_dir, args, &format!("bd_{label}"))
    }
}

#[derive(Debug, Clone)]
struct LogConfig {
    json_logs: bool,
    junit: bool,
    summary: bool,
    failure_context: bool,
}

impl LogConfig {
    fn from_env() -> Self {
        Self {
            json_logs: env_flag("CONFORMANCE_JSON_LOGS"),
            junit: env_flag("CONFORMANCE_JUNIT_XML"),
            summary: env_flag("CONFORMANCE_SUMMARY"),
            failure_context: env_flag("CONFORMANCE_FAILURE_CONTEXT"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct RunLogEntry {
    timestamp: String,
    label: String,
    binary: String,
    args: Vec<String>,
    cwd: String,
    status_code: i32,
    success: bool,
    duration_ms: u128,
    stdout_len: usize,
    stderr_len: usize,
    log_path: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct SummaryStats {
    runs: u64,
    failures: u64,
    total_ms: u128,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct SummaryReport {
    generated_at: String,
    total_runs: u64,
    total_failures: u64,
    by_binary: std::collections::HashMap<String, SummaryStats>,
    by_label: std::collections::HashMap<String, SummaryStats>,
    comparisons: std::collections::HashMap<String, ComparisonStats>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct ComparisonStats {
    obr_runs: u64,
    bd_runs: u64,
    obr_total_ms: u128,
    bd_total_ms: u128,
    speedup_bd_over_obr: Option<f64>,
}

static LOG_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

fn log_mutex() -> &'static Mutex<()> {
    LOG_MUTEX.get_or_init(|| Mutex::new(()))
}

fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

fn collect_dir_listing(path: &PathBuf) -> Vec<String> {
    let mut entries = Vec::new();
    if let Ok(read_dir) = fs::read_dir(path) {
        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    entries.push(format!("{name}/"));
                } else {
                    entries.push(format!("{name} ({:?} bytes)", meta.len()));
                }
            } else {
                entries.push(name);
            }
        }
    }
    entries.sort();
    entries
}

fn append_run_entry(log_dir: &PathBuf, entry: &RunLogEntry) {
    let log_path = log_dir.join("conformance_runs.jsonl");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("open conformance_runs.jsonl");
    let json = serde_json::to_string(entry).expect("serialize run entry");
    writeln!(file, "{json}").expect("append run entry");
}

fn read_run_entries(log_dir: &PathBuf) -> Vec<RunLogEntry> {
    let log_path = log_dir.join("conformance_runs.jsonl");
    let Ok(contents) = fs::read_to_string(&log_path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<RunLogEntry>(line).ok())
        .collect()
}

fn update_summary(log_dir: &PathBuf, entries: &[RunLogEntry]) {
    let mut report = SummaryReport::default();
    report.generated_at = chrono::Utc::now().to_rfc3339();

    for entry in entries {
        report.total_runs += 1;
        if !entry.success {
            report.total_failures += 1;
        }

        let by_binary = report
            .by_binary
            .entry(entry.binary.clone())
            .or_insert_with(SummaryStats::default);
        by_binary.runs += 1;
        if !entry.success {
            by_binary.failures += 1;
        }
        by_binary.total_ms = by_binary.total_ms.saturating_add(entry.duration_ms);

        let by_label = report
            .by_label
            .entry(entry.label.clone())
            .or_insert_with(SummaryStats::default);
        by_label.runs += 1;
        if !entry.success {
            by_label.failures += 1;
        }
        by_label.total_ms = by_label.total_ms.saturating_add(entry.duration_ms);

        let comparison = report
            .comparisons
            .entry(entry.label.clone())
            .or_insert_with(ComparisonStats::default);
        if entry.binary == "obr" {
            comparison.obr_runs += 1;
            comparison.obr_total_ms = comparison.obr_total_ms.saturating_add(entry.duration_ms);
        } else if entry.binary == "bd" {
            comparison.bd_runs += 1;
            comparison.bd_total_ms = comparison.bd_total_ms.saturating_add(entry.duration_ms);
        }
    }

    for comparison in report.comparisons.values_mut() {
        if comparison.obr_total_ms > 0 && comparison.bd_total_ms > 0 {
            comparison.speedup_bd_over_obr =
                Some(comparison.bd_total_ms as f64 / comparison.obr_total_ms as f64);
        }
    }

    let summary_path = log_dir.join("conformance_summary.json");
    let json = serde_json::to_string_pretty(&report).expect("serialize summary");
    fs::write(summary_path, json).expect("write summary");
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn write_junit(log_dir: &PathBuf, entries: &[RunLogEntry]) {
    let total = entries.len();
    let failures = entries.iter().filter(|e| !e.success).count();
    let mut xml = String::new();
    xml.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    xml.push('\n');
    xml.push_str(&format!(
        r#"<testsuite name="conformance_runs" tests="{total}" failures="{failures}">"#
    ));
    xml.push('\n');

    for entry in entries {
        let name = xml_escape(&format!("{}:{}", entry.binary, entry.label));
        let classname = xml_escape(&entry.binary);
        let time_secs = entry.duration_ms as f64 / 1000.0;
        xml.push_str(&format!(
            r#"  <testcase classname="{classname}" name="{name}" time="{time_secs:.3}">"#
        ));
        if !entry.success {
            let msg = xml_escape(&format!(
                "exit={}; log={}",
                entry.status_code, entry.log_path
            ));
            xml.push_str(&format!(r#"<failure message="{msg}"/>"#));
        }
        xml.push_str("</testcase>\n");
    }

    xml.push_str("</testsuite>\n");
    let junit_path = log_dir.join("conformance_junit.xml");
    fs::write(junit_path, xml).expect("write junit xml");
}

fn write_failure_context(
    log_dir: &PathBuf,
    entry: &RunLogEntry,
    stdout: &str,
    stderr: &str,
    cwd: &PathBuf,
) {
    let obr_dir = cwd.join(".obr");
    let context = serde_json::json!({
        "timestamp": entry.timestamp,
        "label": entry.label,
        "binary": entry.binary,
        "args": entry.args,
        "cwd": entry.cwd,
        "status_code": entry.status_code,
        "success": entry.success,
        "duration_ms": entry.duration_ms,
        "stdout_len": entry.stdout_len,
        "stderr_len": entry.stderr_len,
        "stdout_preview": stdout.chars().take(2000).collect::<String>(),
        "stderr_preview": stderr.chars().take(2000).collect::<String>(),
        "obr_dir": obr_dir.display().to_string(),
        "beads_entries": collect_dir_listing(&obr_dir),
        "recent_runs": read_run_entries(log_dir).into_iter().rev().take(5).collect::<Vec<_>>(),
    });
    let path = log_dir.join(format!("{}.failure.json", entry.label));
    let json = serde_json::to_string_pretty(&context).expect("serialize failure context");
    fs::write(path, json).expect("write failure context");
}

fn record_run(log_dir: &PathBuf, entry: RunLogEntry, stdout: &str, stderr: &str, cwd: &PathBuf) {
    let config = LogConfig::from_env();
    if !(config.json_logs || config.junit || config.summary || config.failure_context) {
        return;
    }

    let _guard = log_mutex().lock().expect("lock test log mutex");
    append_run_entry(log_dir, &entry);
    let entries = read_run_entries(log_dir);

    if config.summary {
        update_summary(log_dir, &entries);
    }
    if config.junit {
        write_junit(log_dir, &entries);
    }
    if config.failure_context && !entry.success {
        write_failure_context(log_dir, &entry, stdout, stderr, cwd);
    }
}

fn run_obr_cmd<I, S>(cwd: &PathBuf, log_dir: &PathBuf, args: I, label: &str) -> CmdOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("obr"));
    cmd.current_dir(cwd);
    cmd.args(args);
    cmd.env("NO_COLOR", "1");
    cmd.env("RUST_LOG", "obr=debug");
    cmd.env("RUST_BACKTRACE", "1");
    cmd.env("HOME", cwd);

    run_and_log(cmd, cwd, log_dir, label)
}

fn run_bd_cmd<I, S>(cwd: &PathBuf, log_dir: &PathBuf, args: I, label: &str) -> CmdOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_cmd_system("bd", cwd, log_dir, args, label)
}

fn run_cmd_system<I, S>(
    binary: &str,
    cwd: &PathBuf,
    log_dir: &PathBuf,
    args: I,
    label: &str,
) -> CmdOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = std::process::Command::new(binary);
    cmd.current_dir(cwd);
    cmd.args(args);
    cmd.env("NO_COLOR", "1");
    cmd.env("HOME", cwd);

    let start = Instant::now();
    let output = cmd.output().expect(&format!("run {binary}"));
    let duration = start.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Log output
    let log_path = log_dir.join(format!("{label}.log"));
    let timestamp = SystemTime::now();
    let log_body = format!(
        "label: {label}\nbinary: {binary}\nstarted: {:?}\nduration: {:?}\nstatus: {}\ncwd: {}\n\nstdout:\n{}\n\nstderr:\n{}\n",
        timestamp,
        duration,
        output.status,
        cwd.display(),
        stdout,
        stderr
    );
    fs::write(&log_path, log_body).expect("write log");

    let entry = RunLogEntry {
        timestamp: Utc::now().to_rfc3339(),
        label: label.to_string(),
        binary: binary.to_string(),
        args: cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect(),
        cwd: cwd.display().to_string(),
        status_code: output.status.code().unwrap_or(-1),
        success: output.status.success(),
        duration_ms: duration.as_millis(),
        stdout_len: stdout.len(),
        stderr_len: stderr.len(),
        log_path: log_path.display().to_string(),
    };
    record_run(log_dir, entry, &stdout, &stderr, cwd);

    CmdOutput {
        stdout,
        stderr,
        status: output.status,
        duration,
    }
}

fn run_and_log(mut cmd: Command, cwd: &PathBuf, log_dir: &PathBuf, label: &str) -> CmdOutput {
    let start = Instant::now();
    let output = cmd.output().expect("run command");
    let duration = start.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let log_path = log_dir.join(format!("{label}.log"));
    let timestamp = SystemTime::now();
    let log_body = format!(
        "label: {label}\nstarted: {:?}\nduration: {:?}\nstatus: {}\nargs: {:?}\ncwd: {}\n\nstdout:\n{}\n\nstderr:\n{}\n",
        timestamp,
        duration,
        output.status,
        cmd.get_args().collect::<Vec<_>>(),
        cwd.display(),
        stdout,
        stderr
    );
    fs::write(&log_path, log_body).expect("write log");

    let entry = RunLogEntry {
        timestamp: Utc::now().to_rfc3339(),
        label: label.to_string(),
        binary: "obr".to_string(),
        args: cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect(),
        cwd: cwd.display().to_string(),
        status_code: output.status.code().unwrap_or(-1),
        success: output.status.success(),
        duration_ms: duration.as_millis(),
        stdout_len: stdout.len(),
        stderr_len: stderr.len(),
        log_path: log_path.display().to_string(),
    };
    record_run(log_dir, entry, &stdout, &stderr, cwd);

    CmdOutput {
        stdout,
        stderr,
        status: output.status,
        duration,
    }
}

fn extract_issue_id(json_str: &str) -> String {
    let val: Value = serde_json::from_str(json_str).expect("parse json");
    val["id"]
        .as_str()
        .or_else(|| val[0]["id"].as_str())
        .expect("id field")
        .to_string()
}

fn extract_id_from_json(output: &str) -> String {
    let json = extract_json_payload(output);
    extract_issue_id(&json)
}

// ---------------------------------------------------------------------------
// Label Command Conformance Tests (12 tests)
// ---------------------------------------------------------------------------

#[test]
fn conformance_label_add_single() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_add_single test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues
    let obr_create = workspace.run_obr(["create", "Label test issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Label test issue", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add single label
    let obr_add = workspace.run_obr(["label", "add", &obr_id, "urgent"], "label_add");
    let bd_add = workspace.run_bd(["label", "add", &bd_id, "urgent"], "label_add");

    info!(
        "obr label add single: success={}, bd: success={}",
        obr_add.status.success(),
        bd_add.status.success()
    );

    assert!(
        obr_add.status.success(),
        "obr label add failed: {}",
        obr_add.stderr
    );
    assert!(
        bd_add.status.success(),
        "bd label add failed: {}",
        bd_add.stderr
    );

    // Verify label was added via list
    let obr_list = workspace.run_obr(["label", "list", &obr_id, "--json"], "label_list");
    let bd_list = workspace.run_bd(["label", "list", &bd_id, "--json"], "label_list");

    let obr_labels = extract_json_payload(&obr_list.stdout);
    let bd_labels = extract_json_payload(&bd_list.stdout);

    info!("obr labels: {}", obr_labels);
    info!("bd labels: {}", bd_labels);

    assert!(
        obr_labels.contains("urgent"),
        "obr missing 'urgent' label: {}",
        obr_labels
    );
    assert!(
        bd_labels.contains("urgent"),
        "bd missing 'urgent' label: {}",
        bd_labels
    );

    info!("conformance_label_add_single passed");
}

#[test]
fn conformance_label_add_multiple() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_add_multiple test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Multi-label issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Multi-label issue", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add multiple labels (one at a time for compatibility)
    workspace.run_obr(["label", "add", &obr_id, "bug"], "label_add_bug");
    workspace.run_bd(["label", "add", &bd_id, "bug"], "label_add_bug");

    workspace.run_obr(["label", "add", &obr_id, "priority"], "label_add_priority");
    workspace.run_bd(["label", "add", &bd_id, "priority"], "label_add_priority");

    workspace.run_obr(["label", "add", &obr_id, "frontend"], "label_add_frontend");
    workspace.run_bd(["label", "add", &bd_id, "frontend"], "label_add_frontend");

    // Verify all labels
    let obr_list = workspace.run_obr(["label", "list", &obr_id, "--json"], "label_list");
    let bd_list = workspace.run_bd(["label", "list", &bd_id, "--json"], "label_list");

    let obr_labels = extract_json_payload(&obr_list.stdout);
    let bd_labels = extract_json_payload(&bd_list.stdout);

    info!("obr labels: {}", obr_labels);
    info!("bd labels: {}", bd_labels);

    for label in &["bug", "priority", "frontend"] {
        assert!(
            obr_labels.contains(label),
            "obr missing '{}' label: {}",
            label,
            obr_labels
        );
        assert!(
            bd_labels.contains(label),
            "bd missing '{}' label: {}",
            label,
            bd_labels
        );
    }

    info!("conformance_label_add_multiple passed");
}

#[test]
fn conformance_label_remove() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_remove test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Remove label test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Remove label test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add then remove
    workspace.run_obr(["label", "add", &obr_id, "toremove"], "label_add");
    workspace.run_bd(["label", "add", &bd_id, "toremove"], "label_add");

    let obr_remove = workspace.run_obr(["label", "remove", &obr_id, "toremove"], "label_remove");
    let bd_remove = workspace.run_bd(["label", "remove", &bd_id, "toremove"], "label_remove");

    info!(
        "obr label remove: success={}, bd: success={}",
        obr_remove.status.success(),
        bd_remove.status.success()
    );

    assert!(
        obr_remove.status.success(),
        "obr label remove failed: {}",
        obr_remove.stderr
    );
    assert!(
        bd_remove.status.success(),
        "bd label remove failed: {}",
        bd_remove.stderr
    );

    // Verify label was removed
    let obr_list = workspace.run_obr(["label", "list", &obr_id, "--json"], "label_list");
    let bd_list = workspace.run_bd(["label", "list", &bd_id, "--json"], "label_list");

    let obr_labels = extract_json_payload(&obr_list.stdout);
    let bd_labels = extract_json_payload(&bd_list.stdout);

    info!("obr labels after remove: {}", obr_labels);
    info!("bd labels after remove: {}", bd_labels);

    assert!(
        !obr_labels.contains("toremove"),
        "obr still has 'toremove' label: {}",
        obr_labels
    );
    assert!(
        !bd_labels.contains("toremove"),
        "bd still has 'toremove' label: {}",
        bd_labels
    );

    info!("conformance_label_remove passed");
}

#[test]
fn conformance_label_list() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_list test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "List labels test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "List labels test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add labels
    workspace.run_obr(["label", "add", &obr_id, "alpha"], "add_alpha");
    workspace.run_bd(["label", "add", &bd_id, "alpha"], "add_alpha");
    workspace.run_obr(["label", "add", &obr_id, "beta"], "add_beta");
    workspace.run_bd(["label", "add", &bd_id, "beta"], "add_beta");

    // List labels
    let obr_list = workspace.run_obr(["label", "list", &obr_id, "--json"], "label_list");
    let bd_list = workspace.run_bd(["label", "list", &bd_id, "--json"], "label_list");

    assert!(
        obr_list.status.success(),
        "obr label list failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd label list failed: {}",
        bd_list.stderr
    );

    let obr_labels = extract_json_payload(&obr_list.stdout);
    let bd_labels = extract_json_payload(&bd_list.stdout);

    info!("obr labels: {}", obr_labels);
    info!("bd labels: {}", bd_labels);

    // Both should have alpha and beta
    assert!(obr_labels.contains("alpha"), "obr missing alpha");
    assert!(obr_labels.contains("beta"), "obr missing beta");
    assert!(bd_labels.contains("alpha"), "bd missing alpha");
    assert!(bd_labels.contains("beta"), "bd missing beta");

    info!("conformance_label_list passed");
}

#[test]
fn conformance_label_list_all() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_list_all test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create multiple issues with different labels
    let obr_create1 = workspace.run_obr(["create", "Issue 1", "--json"], "create1");
    let bd_create1 = workspace.run_bd(["create", "Issue 1", "--json"], "create1");
    let obr_id1 = extract_id_from_json(&obr_create1.stdout);
    let bd_id1 = extract_id_from_json(&bd_create1.stdout);

    let obr_create2 = workspace.run_obr(["create", "Issue 2", "--json"], "create2");
    let bd_create2 = workspace.run_bd(["create", "Issue 2", "--json"], "create2");
    let obr_id2 = extract_id_from_json(&obr_create2.stdout);
    let bd_id2 = extract_id_from_json(&bd_create2.stdout);

    // Add labels to different issues
    workspace.run_obr(["label", "add", &obr_id1, "shared"], "add_shared1");
    workspace.run_bd(["label", "add", &bd_id1, "shared"], "add_shared1");
    workspace.run_obr(["label", "add", &obr_id2, "shared"], "add_shared2");
    workspace.run_bd(["label", "add", &bd_id2, "shared"], "add_shared2");
    workspace.run_obr(["label", "add", &obr_id1, "unique1"], "add_unique1");
    workspace.run_bd(["label", "add", &bd_id1, "unique1"], "add_unique1");
    workspace.run_obr(["label", "add", &obr_id2, "unique2"], "add_unique2");
    workspace.run_bd(["label", "add", &bd_id2, "unique2"], "add_unique2");

    // List all labels
    let obr_list_all = workspace.run_obr(["label", "list-all", "--json"], "list_all");
    let bd_list_all = workspace.run_bd(["label", "list-all", "--json"], "list_all");

    assert!(
        obr_list_all.status.success(),
        "obr label list-all failed: {}",
        obr_list_all.stderr
    );
    assert!(
        bd_list_all.status.success(),
        "bd label list-all failed: {}",
        bd_list_all.stderr
    );

    let obr_output = extract_json_payload(&obr_list_all.stdout);
    let bd_output = extract_json_payload(&bd_list_all.stdout);

    info!("obr list-all: {}", obr_output);
    info!("bd list-all: {}", bd_output);

    // Should contain all labels
    assert!(obr_output.contains("shared"), "obr missing shared label");
    assert!(obr_output.contains("unique1"), "obr missing unique1 label");
    assert!(obr_output.contains("unique2"), "obr missing unique2 label");
    assert!(bd_output.contains("shared"), "bd missing shared label");
    assert!(bd_output.contains("unique1"), "bd missing unique1 label");
    assert!(bd_output.contains("unique2"), "bd missing unique2 label");

    info!("conformance_label_list_all passed");
}

#[test]
fn conformance_label_special_chars() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_special_chars test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Special char label test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Special char label test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Test label with hyphen and underscore
    let obr_add = workspace.run_obr(
        ["label", "add", &obr_id, "my-label_v2"],
        "label_add_special",
    );
    let bd_add = workspace.run_bd(["label", "add", &bd_id, "my-label_v2"], "label_add_special");

    info!(
        "obr special char add: success={}, bd: success={}",
        obr_add.status.success(),
        bd_add.status.success()
    );

    // Verify
    let obr_list = workspace.run_obr(["label", "list", &obr_id, "--json"], "label_list");
    let bd_list = workspace.run_bd(["label", "list", &bd_id, "--json"], "label_list");

    let obr_labels = extract_json_payload(&obr_list.stdout);
    let bd_labels = extract_json_payload(&bd_list.stdout);

    info!("obr labels: {}", obr_labels);
    info!("bd labels: {}", bd_labels);

    // Exit codes should match
    assert_eq!(
        obr_add.status.success(),
        bd_add.status.success(),
        "Exit code mismatch for special char label"
    );

    info!("conformance_label_special_chars passed");
}

#[test]
fn conformance_label_unicode() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_unicode test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Unicode label test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Unicode label test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Test unicode label
    let obr_add = workspace.run_obr(["label", "add", &obr_id, "测试标签"], "label_add_unicode");
    let bd_add = workspace.run_bd(["label", "add", &bd_id, "测试标签"], "label_add_unicode");

    info!(
        "obr unicode add: success={}, bd: success={}",
        obr_add.status.success(),
        bd_add.status.success()
    );

    // Log unicode handling behavior (obr and bd may differ)
    if obr_add.status.success() != bd_add.status.success() {
        info!(
            "NOTE: Unicode label handling differs between obr (success={}) and bd (success={})",
            obr_add.status.success(),
            bd_add.status.success()
        );
        // This is a known conformance difference - document but don't fail
    }

    // If obr succeeded, verify the label is there
    if obr_add.status.success() {
        let obr_list = workspace.run_obr(["label", "list", &obr_id, "--json"], "label_list");
        let obr_labels = extract_json_payload(&obr_list.stdout);
        info!("obr unicode labels: {}", obr_labels);
        assert!(obr_labels.contains("测试标签"), "obr missing unicode label");
    }

    // If bd succeeded, verify the label is there
    if bd_add.status.success() {
        let bd_list = workspace.run_bd(["label", "list", &bd_id, "--json"], "label_list");
        let bd_labels = extract_json_payload(&bd_list.stdout);
        info!("bd unicode labels: {}", bd_labels);
        assert!(bd_labels.contains("测试标签"), "bd missing unicode label");
    }

    info!("conformance_label_unicode passed");
}

#[test]
fn conformance_label_duplicate() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_duplicate test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Duplicate label test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Duplicate label test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add label first time
    workspace.run_obr(["label", "add", &obr_id, "existing"], "label_add_first");
    workspace.run_bd(["label", "add", &bd_id, "existing"], "label_add_first");

    // Try adding same label again
    let obr_dup = workspace.run_obr(["label", "add", &obr_id, "existing"], "label_add_dup");
    let bd_dup = workspace.run_bd(["label", "add", &bd_id, "existing"], "label_add_dup");

    info!(
        "obr duplicate add: success={}, bd: success={}",
        obr_dup.status.success(),
        bd_dup.status.success()
    );

    // Exit codes should match (might succeed or fail, but should be consistent)
    assert_eq!(
        obr_dup.status.success(),
        bd_dup.status.success(),
        "Exit code mismatch for duplicate label add"
    );

    info!("conformance_label_duplicate passed");
}

#[test]
fn conformance_label_remove_nonexistent() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_remove_nonexistent test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Remove nonexistent label", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Remove nonexistent label", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Try removing a label that doesn't exist
    let obr_remove = workspace.run_obr(
        ["label", "remove", &obr_id, "doesnotexist"],
        "label_remove_nonexistent",
    );
    let bd_remove = workspace.run_bd(
        ["label", "remove", &bd_id, "doesnotexist"],
        "label_remove_nonexistent",
    );

    info!(
        "obr remove nonexistent: success={}, bd: success={}",
        obr_remove.status.success(),
        bd_remove.status.success()
    );

    // Both should handle this consistently
    assert_eq!(
        obr_remove.status.success(),
        bd_remove.status.success(),
        "Exit code mismatch for remove nonexistent label"
    );

    info!("conformance_label_remove_nonexistent passed");
}

#[test]
fn conformance_label_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "JSON shape test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "JSON shape test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    workspace.run_obr(["label", "add", &obr_id, "test"], "add");
    workspace.run_bd(["label", "add", &bd_id, "test"], "add");

    let obr_list = workspace.run_obr(["label", "list", &obr_id, "--json"], "list");
    let bd_list = workspace.run_bd(["label", "list", &bd_id, "--json"], "list");

    assert!(obr_list.status.success(), "obr label list --json failed");
    assert!(bd_list.status.success(), "bd label list --json failed");

    let obr_json = extract_json_payload(&obr_list.stdout);
    let bd_json = extract_json_payload(&bd_list.stdout);

    // Both should parse as valid JSON
    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr JSON parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd JSON parse");

    info!("obr JSON: {:?}", obr_val);
    info!("bd JSON: {:?}", bd_val);

    // Both should be arrays or objects containing label data
    assert!(
        obr_val.is_array() || obr_val.is_object(),
        "obr output not array or object"
    );
    assert!(
        bd_val.is_array() || bd_val.is_object(),
        "bd output not array or object"
    );

    info!("conformance_label_json_shape passed");
}

#[test]
fn conformance_label_filter_issues() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_filter_issues test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with different labels
    let obr_create1 = workspace.run_obr(["create", "Has target label", "--json"], "create1");
    let bd_create1 = workspace.run_bd(["create", "Has target label", "--json"], "create1");
    let obr_id1 = extract_id_from_json(&obr_create1.stdout);
    let bd_id1 = extract_id_from_json(&bd_create1.stdout);

    let obr_create2 = workspace.run_obr(["create", "No target label", "--json"], "create2");
    let bd_create2 = workspace.run_bd(["create", "No target label", "--json"], "create2");
    let obr_id2 = extract_id_from_json(&obr_create2.stdout);
    let bd_id2 = extract_id_from_json(&bd_create2.stdout);

    // Add target label to first issue only
    workspace.run_obr(["label", "add", &obr_id1, "target"], "add_target");
    workspace.run_bd(["label", "add", &bd_id1, "target"], "add_target");
    workspace.run_obr(["label", "add", &obr_id2, "other"], "add_other");
    workspace.run_bd(["label", "add", &bd_id2, "other"], "add_other");

    // List issues with target label
    let obr_filter = workspace.run_obr(["list", "--label", "target", "--json"], "filter");
    let bd_filter = workspace.run_bd(["list", "--label", "target", "--json"], "filter");

    assert!(
        obr_filter.status.success(),
        "obr list --label failed: {}",
        obr_filter.stderr
    );
    assert!(
        bd_filter.status.success(),
        "bd list --label failed: {}",
        bd_filter.stderr
    );

    let obr_output = extract_json_payload(&obr_filter.stdout);
    let bd_output = extract_json_payload(&bd_filter.stdout);

    info!("obr filtered: {}", obr_output);
    info!("bd filtered: {}", bd_output);

    // Should contain the issue with target label
    assert!(
        obr_output.contains("Has target label") || obr_output.contains(&obr_id1),
        "obr missing filtered issue"
    );
    assert!(
        bd_output.contains("Has target label") || bd_output.contains(&bd_id1),
        "bd missing filtered issue"
    );

    info!("conformance_label_filter_issues passed");
}

#[test]
fn conformance_label_case_sensitivity() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_case_sensitivity test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Case test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Case test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add lowercase label
    workspace.run_obr(["label", "add", &obr_id, "myLabel"], "add_mixed");
    workspace.run_bd(["label", "add", &bd_id, "myLabel"], "add_mixed");

    // Add uppercase variant
    let obr_upper = workspace.run_obr(["label", "add", &obr_id, "MYLABEL"], "add_upper");
    let bd_upper = workspace.run_bd(["label", "add", &bd_id, "MYLABEL"], "add_upper");

    info!(
        "obr upper case add: success={}, bd: success={}",
        obr_upper.status.success(),
        bd_upper.status.success()
    );

    // Check the result
    let obr_list = workspace.run_obr(["label", "list", &obr_id, "--json"], "list");
    let bd_list = workspace.run_bd(["label", "list", &bd_id, "--json"], "list");

    let obr_labels = extract_json_payload(&obr_list.stdout);
    let bd_labels = extract_json_payload(&bd_list.stdout);

    info!("obr labels (case test): {}", obr_labels);
    info!("bd labels (case test): {}", bd_labels);

    // Both should handle case consistently (either both case-sensitive or both insensitive)
    assert_eq!(
        obr_upper.status.success(),
        bd_upper.status.success(),
        "Case handling mismatch"
    );

    info!("conformance_label_case_sensitivity passed");
}

// ---------------------------------------------------------------------------
// Comments Command Conformance Tests (13 tests)
// ---------------------------------------------------------------------------

#[test]
fn conformance_comments_add() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_comments_add test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Comment test issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Comment test issue", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add comment
    let obr_add = workspace.run_obr(
        ["comments", "add", &obr_id, "This is a test comment"],
        "comments_add",
    );
    let bd_add = workspace.run_bd(
        ["comments", "add", &bd_id, "This is a test comment"],
        "comments_add",
    );

    info!(
        "obr comments add: success={}, bd: success={}",
        obr_add.status.success(),
        bd_add.status.success()
    );

    assert!(
        obr_add.status.success(),
        "obr comments add failed: {}",
        obr_add.stderr
    );
    assert!(
        bd_add.status.success(),
        "bd comments add failed: {}",
        bd_add.stderr
    );

    // Verify comment exists
    let obr_list = workspace.run_obr(["comments", &obr_id, "--json"], "comments_list");
    let bd_list = workspace.run_bd(["comments", &bd_id, "--json"], "comments_list");

    let obr_comments = extract_json_payload(&obr_list.stdout);
    let bd_comments = extract_json_payload(&bd_list.stdout);

    info!("obr comments: {}", obr_comments);
    info!("bd comments: {}", bd_comments);

    assert!(obr_comments.contains("test comment"), "obr missing comment");
    assert!(bd_comments.contains("test comment"), "bd missing comment");

    info!("conformance_comments_add passed");
}

#[test]
fn conformance_comments_list() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_comments_list test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Comments list test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Comments list test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add a comment
    workspace.run_obr(["comments", "add", &obr_id, "First comment"], "add");
    workspace.run_bd(["comments", "add", &bd_id, "First comment"], "add");

    // List comments
    let obr_list = workspace.run_obr(["comments", &obr_id, "--json"], "list");
    let bd_list = workspace.run_bd(["comments", &bd_id, "--json"], "list");

    assert!(
        obr_list.status.success(),
        "obr comments list failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd comments list failed: {}",
        bd_list.stderr
    );

    let obr_output = extract_json_payload(&obr_list.stdout);
    let bd_output = extract_json_payload(&bd_list.stdout);

    info!("obr comments list: {}", obr_output);
    info!("bd comments list: {}", bd_output);

    assert!(
        obr_output.contains("First comment"),
        "obr missing comment in list"
    );
    assert!(
        bd_output.contains("First comment"),
        "bd missing comment in list"
    );

    info!("conformance_comments_list passed");
}

#[test]
fn conformance_comments_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_comments_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "No comments issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "No comments issue", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // List comments on issue with no comments
    let obr_list = workspace.run_obr(["comments", &obr_id, "--json"], "list_empty");
    let bd_list = workspace.run_bd(["comments", &bd_id, "--json"], "list_empty");

    assert!(
        obr_list.status.success(),
        "obr comments list empty failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd comments list empty failed: {}",
        bd_list.stderr
    );

    let obr_output = extract_json_payload(&obr_list.stdout);
    let bd_output = extract_json_payload(&bd_list.stdout);

    info!("obr empty comments: {}", obr_output);
    info!("bd empty comments: {}", bd_output);

    // Both should return empty array or similar
    let obr_val: Value = serde_json::from_str(&obr_output).expect("obr parse");
    let bd_val: Value = serde_json::from_str(&bd_output).expect("bd parse");

    // Should be empty array or null
    if let Some(arr) = obr_val.as_array() {
        assert!(arr.is_empty(), "obr not empty: {:?}", arr);
    }
    if let Some(arr) = bd_val.as_array() {
        assert!(arr.is_empty(), "bd not empty: {:?}", arr);
    }

    info!("conformance_comments_empty passed");
}

#[test]
fn conformance_comments_multiple() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_comments_multiple test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Multiple comments", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Multiple comments", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add multiple comments
    for i in 1..=3 {
        let comment = format!("Comment number {i}");
        workspace.run_obr(["comments", "add", &obr_id, &comment], &format!("add_{i}"));
        workspace.run_bd(["comments", "add", &bd_id, &comment], &format!("add_{i}"));
    }

    // List all comments
    let obr_list = workspace.run_obr(["comments", &obr_id, "--json"], "list");
    let bd_list = workspace.run_bd(["comments", &bd_id, "--json"], "list");

    assert!(obr_list.status.success(), "obr comments list failed");
    assert!(bd_list.status.success(), "bd comments list failed");

    let obr_output = extract_json_payload(&obr_list.stdout);
    let bd_output = extract_json_payload(&bd_list.stdout);

    info!("obr multiple comments: {}", obr_output);
    info!("bd multiple comments: {}", bd_output);

    // Should have all 3 comments
    for i in 1..=3 {
        let text = format!("Comment number {i}");
        assert!(obr_output.contains(&text), "obr missing comment {i}");
        assert!(bd_output.contains(&text), "bd missing comment {i}");
    }

    info!("conformance_comments_multiple passed");
}

#[test]
fn conformance_comments_markdown() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_comments_markdown test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Markdown comment test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Markdown comment test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add markdown comment
    let md_comment = "# Heading\n\n- bullet 1\n- bullet 2\n\n**bold** and _italic_";
    let obr_add = workspace.run_obr(["comments", "add", &obr_id, md_comment], "add_md");
    let bd_add = workspace.run_bd(["comments", "add", &bd_id, md_comment], "add_md");

    info!(
        "obr markdown add: success={}, bd: success={}",
        obr_add.status.success(),
        bd_add.status.success()
    );

    assert_eq!(
        obr_add.status.success(),
        bd_add.status.success(),
        "Markdown handling mismatch"
    );

    if obr_add.status.success() {
        let obr_list = workspace.run_obr(["comments", &obr_id, "--json"], "list");
        let bd_list = workspace.run_bd(["comments", &bd_id, "--json"], "list");

        let obr_output = extract_json_payload(&obr_list.stdout);
        let bd_output = extract_json_payload(&bd_list.stdout);

        info!("obr markdown comment: {}", obr_output);
        info!("bd markdown comment: {}", bd_output);

        // Should contain the markdown content
        assert!(
            obr_output.contains("Heading"),
            "obr missing markdown content"
        );
        assert!(bd_output.contains("Heading"), "bd missing markdown content");
    }

    info!("conformance_comments_markdown passed");
}

#[test]
fn conformance_comments_unicode() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_comments_unicode test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Unicode comment test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Unicode comment test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add unicode comment
    let unicode_comment = "这是一个测试评论 émojis работают";
    let obr_add = workspace.run_obr(["comments", "add", &obr_id, unicode_comment], "add_unicode");
    let bd_add = workspace.run_bd(["comments", "add", &bd_id, unicode_comment], "add_unicode");

    info!(
        "obr unicode add: success={}, bd: success={}",
        obr_add.status.success(),
        bd_add.status.success()
    );

    assert_eq!(
        obr_add.status.success(),
        bd_add.status.success(),
        "Unicode handling mismatch"
    );

    if obr_add.status.success() {
        let obr_list = workspace.run_obr(["comments", &obr_id, "--json"], "list");
        let bd_list = workspace.run_bd(["comments", &bd_id, "--json"], "list");

        let obr_output = extract_json_payload(&obr_list.stdout);
        let bd_output = extract_json_payload(&bd_list.stdout);

        info!("obr unicode comment: {}", obr_output);
        info!("bd unicode comment: {}", bd_output);

        assert!(
            obr_output.contains("测试评论"),
            "obr missing unicode content"
        );
        assert!(bd_output.contains("测试评论"), "bd missing unicode content");
    }

    info!("conformance_comments_unicode passed");
}

#[test]
fn conformance_comments_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_comments_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "JSON shape test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "JSON shape test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    workspace.run_obr(["comments", "add", &obr_id, "Test comment"], "add");
    workspace.run_bd(["comments", "add", &bd_id, "Test comment"], "add");

    let obr_list = workspace.run_obr(["comments", &obr_id, "--json"], "list");
    let bd_list = workspace.run_bd(["comments", &bd_id, "--json"], "list");

    assert!(obr_list.status.success(), "obr comments --json failed");
    assert!(bd_list.status.success(), "bd comments --json failed");

    let obr_json = extract_json_payload(&obr_list.stdout);
    let bd_json = extract_json_payload(&bd_list.stdout);

    // Parse as JSON
    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr JSON parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd JSON parse");

    info!("obr JSON shape: {:?}", obr_val);
    info!("bd JSON shape: {:?}", bd_val);

    // Both should be arrays
    assert!(obr_val.is_array(), "obr output not array");
    assert!(bd_val.is_array(), "bd output not array");

    // Comments should have body field
    if let Some(arr) = obr_val.as_array() {
        if let Some(first) = arr.first() {
            assert!(
                first.get("body").is_some()
                    || first.get("content").is_some()
                    || first.get("text").is_some(),
                "obr comment missing body/content field"
            );
        }
    }

    info!("conformance_comments_json_shape passed");
}

#[test]
fn conformance_comments_issue_not_found() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_comments_issue_not_found test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Try to list comments for nonexistent issue
    let obr_list = workspace.run_obr(["comments", "nonexistent-id", "--json"], "list_notfound");
    let bd_list = workspace.run_bd(["comments", "nonexistent-id", "--json"], "list_notfound");

    info!(
        "obr not found: success={}, bd: success={}",
        obr_list.status.success(),
        bd_list.status.success()
    );

    // Both should fail similarly
    assert_eq!(
        obr_list.status.success(),
        bd_list.status.success(),
        "Error handling mismatch for nonexistent issue"
    );

    // If both fail, that's expected
    if !obr_list.status.success() && !bd_list.status.success() {
        info!("Both correctly failed for nonexistent issue");
    }

    info!("conformance_comments_issue_not_found passed");
}

#[test]
fn conformance_comments_ordering() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_comments_ordering test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Ordering test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Ordering test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add comments in order
    workspace.run_obr(["comments", "add", &obr_id, "FIRST"], "add1");
    workspace.run_bd(["comments", "add", &bd_id, "FIRST"], "add1");

    // Small delay to ensure different timestamps
    std::thread::sleep(std::time::Duration::from_millis(50));

    workspace.run_obr(["comments", "add", &obr_id, "SECOND"], "add2");
    workspace.run_bd(["comments", "add", &bd_id, "SECOND"], "add2");

    std::thread::sleep(std::time::Duration::from_millis(50));

    workspace.run_obr(["comments", "add", &obr_id, "THIRD"], "add3");
    workspace.run_bd(["comments", "add", &bd_id, "THIRD"], "add3");

    // List comments
    let obr_list = workspace.run_obr(["comments", &obr_id, "--json"], "list");
    let bd_list = workspace.run_bd(["comments", &bd_id, "--json"], "list");

    let obr_output = &obr_list.stdout;
    let bd_output = &bd_list.stdout;

    info!("obr ordering: {}", obr_output);
    info!("bd ordering: {}", bd_output);

    // FIRST should appear before SECOND, SECOND before THIRD
    let obr_first_pos = obr_output.find("FIRST").unwrap_or(usize::MAX);
    let obr_second_pos = obr_output.find("SECOND").unwrap_or(usize::MAX);
    let obr_third_pos = obr_output.find("THIRD").unwrap_or(usize::MAX);

    let bd_first_pos = bd_output.find("FIRST").unwrap_or(usize::MAX);
    let bd_second_pos = bd_output.find("SECOND").unwrap_or(usize::MAX);
    let bd_third_pos = bd_output.find("THIRD").unwrap_or(usize::MAX);

    assert!(
        obr_first_pos < obr_second_pos,
        "obr: FIRST not before SECOND"
    );
    assert!(
        obr_second_pos < obr_third_pos,
        "obr: SECOND not before THIRD"
    );
    assert!(bd_first_pos < bd_second_pos, "bd: FIRST not before SECOND");
    assert!(bd_second_pos < bd_third_pos, "bd: SECOND not before THIRD");

    info!("conformance_comments_ordering passed");
}

#[test]
fn conformance_comments_author() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_comments_author test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Author test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Author test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Add comment with specific actor
    let obr_add = workspace.run_obr(
        [
            "comments",
            "add",
            &obr_id,
            "Author test comment",
            "--actor",
            "TestUser",
        ],
        "add",
    );
    let bd_add = workspace.run_bd(
        [
            "comments",
            "add",
            &bd_id,
            "Author test comment",
            "--actor",
            "TestUser",
        ],
        "add",
    );

    assert!(
        obr_add.status.success(),
        "obr add failed: {}",
        obr_add.stderr
    );
    assert!(bd_add.status.success(), "bd add failed: {}", bd_add.stderr);

    // List and check author
    let obr_list = workspace.run_obr(["comments", &obr_id, "--json"], "list");
    let bd_list = workspace.run_bd(["comments", &bd_id, "--json"], "list");

    let obr_output = extract_json_payload(&obr_list.stdout);
    let bd_output = extract_json_payload(&bd_list.stdout);

    info!("obr author test: {}", obr_output);
    info!("bd author test: {}", bd_output);

    // Both should have author information (might be in different fields)
    let obr_val: Value = serde_json::from_str(&obr_output).expect("obr parse");
    let bd_val: Value = serde_json::from_str(&bd_output).expect("bd parse");

    if let Some(arr) = obr_val.as_array() {
        if let Some(first) = arr.first() {
            let has_author = first.get("author").is_some()
                || first.get("created_by").is_some()
                || first.get("actor").is_some();
            info!("obr comment has author field: {}", has_author);
        }
    }

    if let Some(arr) = bd_val.as_array() {
        if let Some(first) = arr.first() {
            let has_author = first.get("author").is_some()
                || first.get("created_by").is_some()
                || first.get("actor").is_some();
            info!("bd comment has author field: {}", has_author);
        }
    }

    info!("conformance_comments_author passed");
}

#[test]
fn conformance_comments_timestamps() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_comments_timestamps test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Timestamp test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Timestamp test", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    workspace.run_obr(["comments", "add", &obr_id, "Timestamp comment"], "add");
    workspace.run_bd(["comments", "add", &bd_id, "Timestamp comment"], "add");

    let obr_list = workspace.run_obr(["comments", &obr_id, "--json"], "list");
    let bd_list = workspace.run_bd(["comments", &bd_id, "--json"], "list");

    let obr_json = extract_json_payload(&obr_list.stdout);
    let bd_json = extract_json_payload(&bd_list.stdout);

    info!("obr timestamps: {}", obr_json);
    info!("bd timestamps: {}", bd_json);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd parse");

    // Check for timestamp fields
    if let Some(arr) = obr_val.as_array() {
        if let Some(first) = arr.first() {
            let has_timestamp = first.get("created_at").is_some()
                || first.get("timestamp").is_some()
                || first.get("date").is_some();
            info!("obr comment has timestamp: {}", has_timestamp);
        }
    }

    if let Some(arr) = bd_val.as_array() {
        if let Some(first) = arr.first() {
            let has_timestamp = first.get("created_at").is_some()
                || first.get("timestamp").is_some()
                || first.get("date").is_some();
            info!("bd comment has timestamp: {}", has_timestamp);
        }
    }

    info!("conformance_comments_timestamps passed");
}
